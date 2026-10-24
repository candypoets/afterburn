//! Herdr 0.9 socket subscription adapter. No terminal contents enter node events.

use std::collections::BTreeSet;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

use super::{AgentStatus, NodeHerdr};
use crate::error::{Error, Result};

const MAX_FRAME: usize = 8 * 1024 * 1024;
const READ_TICK: Duration = Duration::from_millis(250);
const RESYNC_INTERVAL: Duration = Duration::from_secs(30);

/// Parent translates these into node events and uses the statuses for Sync job completion.
/// A resync is a current snapshot, not proof that a job completed while disconnected.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HerdrEvent {
    Connected,
    Disconnected { reason: String },
    Resync { agents: Vec<AgentStatus> },
    AgentStatus { agent: AgentStatus },
    InventoryChanged,
}

/// Keep this guard alive while reading `events`; drop cancels the worker without stopping Herdr.
pub struct HerdrMonitor {
    pub events: mpsc::Receiver<HerdrEvent>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl HerdrMonitor {
    pub(super) fn start(node: NodeHerdr) -> Result<Self> {
        let (tx, events) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("mindmap-herdr-events".into())
            .spawn(move || {
                let mut backoff = Duration::from_millis(250);
                let mut disconnected = false;
                while !stopping.load(Ordering::Relaxed) {
                    let established = Arc::new(AtomicBool::new(false));
                    match stream_events(&node, &tx, &stopping, &established) {
                        Ok(()) => {
                            backoff = Duration::from_millis(250);
                            disconnected = false;
                        }
                        Err(error) => {
                            if established.load(Ordering::Relaxed) {
                                disconnected = false;
                                backoff = Duration::from_millis(250);
                            }
                            if !disconnected
                                && !stopping.load(Ordering::Relaxed)
                                && tx
                                    .send(HerdrEvent::Disconnected {
                                        reason: error.to_string(),
                                    })
                                    .is_err()
                            {
                                break;
                            }
                            disconnected = true;
                            let deadline = Instant::now() + backoff;
                            while Instant::now() < deadline && !stopping.load(Ordering::Relaxed) {
                                thread::sleep(Duration::from_millis(50));
                            }
                            backoff = (backoff * 2).min(Duration::from_secs(10));
                        }
                    }
                }
            })?;
        Ok(Self {
            events,
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for HerdrMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Do not make node shutdown wait on a peer stuck midway through its snapshot response.
        // Each socket operation has a finite deadline; the worker exits on its next stop check.
        if let Some(worker) = self.worker.take() {
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

fn send(tx: &mpsc::Sender<HerdrEvent>, event: HerdrEvent) -> Result<()> {
    tx.send(event)
        .map_err(|_| Error::new("Herdr event receiver closed"))
}

fn pane_ids(snapshot: &Value) -> BTreeSet<String> {
    snapshot["panes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|pane| pane["pane_id"].as_str().map(str::to_owned))
        .collect()
}

fn subscriptions(panes: &BTreeSet<String>) -> Vec<Value> {
    let mut subscriptions: Vec<Value> = [
        "workspace.created",
        "workspace.updated",
        "workspace.closed",
        "workspace.renamed",
        "workspace.focused",
        "worktree.created",
        "worktree.opened",
        "worktree.removed",
        "tab.created",
        "tab.closed",
        "tab.renamed",
        "tab.focused",
        "pane.created",
        "pane.closed",
        "pane.moved",
        "pane.exited",
        "pane.agent_detected",
    ]
    .into_iter()
    .map(|kind| json!({"type": kind}))
    .collect();
    subscriptions.extend(
        panes
            .iter()
            .map(|pane| json!({"type": "pane.agent_status_changed", "pane_id": pane})),
    );
    subscriptions
}

fn stream_events(
    node: &NodeHerdr,
    tx: &mpsc::Sender<HerdrEvent>,
    stop: &AtomicBool,
    established: &AtomicBool,
) -> Result<()> {
    // Herdr requires a pane_id for each status subscription. Learn the current pane set, then
    // subscribe + await ack BEFORE the authoritative resync. Events buffer in the socket while
    // that second snapshot runs. A changed pane set causes a fresh subscription immediately.
    let initial = node.snapshot()?;
    let panes = pane_ids(&initial);
    let mut stream = connect(&node.socket, READ_TICK)?;
    write_request(
        &mut stream,
        "subscribe",
        "events.subscribe",
        json!({"subscriptions": subscriptions(&panes)}),
    )?;
    let mut reader = Frames::default();
    let ack = reader.until(&mut stream, Instant::now() + Duration::from_secs(10))?;
    let result = response(ack, "subscribe")?;
    if result["type"] != "subscription_started" {
        return Err(Error::new(
            "Herdr did not acknowledge its event subscription",
        ));
    }
    established.store(true, Ordering::Relaxed);
    let snapshot = node.snapshot()?;
    send(tx, HerdrEvent::Connected)?;
    resync(tx, &snapshot)?;
    if pane_ids(&snapshot) != panes {
        return Ok(());
    }
    let mut last_resync = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        if let Some(frame) = reader.next(&mut stream)? {
            let name = frame["event"]
                .as_str()
                .ok_or_else(|| Error::new("invalid Herdr event envelope"))?;
            match name {
                "pane.agent_status_changed" | "pane_agent_status_changed" => {
                    let agent = status_from(&frame["data"])
                        .ok_or_else(|| Error::new("invalid Herdr agent status event"))?;
                    send(tx, HerdrEvent::AgentStatus { agent })?;
                }
                _ => {
                    send(tx, HerdrEvent::InventoryChanged)?;
                    if matches!(
                        name,
                        "pane_exited" | "pane.exited" | "pane_closed" | "pane.closed"
                    ) {
                        if let Some(mut agent) = status_from(&frame["data"]) {
                            agent.status = "exited".into();
                            send(tx, HerdrEvent::AgentStatus { agent })?;
                        }
                    }
                    if matches!(name, "pane_agent_detected" | "pane.agent_detected")
                        && frame["data"]["released"].as_bool() == Some(true)
                    {
                        if let Some(mut agent) = status_from(&frame["data"]) {
                            agent.status = frame["data"]["final_status"]
                                .as_str()
                                .unwrap_or("exited")
                                .into();
                            send(tx, HerdrEvent::AgentStatus { agent })?;
                        }
                    }
                    // Per-pane subscriptions cannot be amended in place. Rebuild on topology
                    // changes; all other events remain on this long-lived connection.
                    if matches!(
                        name,
                        "pane_created"
                            | "pane.created"
                            | "pane_closed"
                            | "pane.closed"
                            | "pane_moved"
                            | "pane.moved"
                    ) {
                        return Ok(());
                    }
                    if matches!(
                        name,
                        "pane_agent_detected"
                            | "pane.agent_detected"
                            | "pane_exited"
                            | "pane.exited"
                    ) {
                        resync(tx, &node.snapshot()?)?;
                    }
                }
            }
        }
        // The socket carries real status transitions. This infrequent snapshot is a liveness
        // check and lost-event reconciliation, never the sole status source.
        if last_resync.elapsed() >= RESYNC_INTERVAL {
            let snapshot = node.snapshot()?;
            resync(tx, &snapshot)?;
            if pane_ids(&snapshot) != panes {
                return Ok(());
            }
            last_resync = Instant::now();
        }
    }
    Ok(())
}

fn status_from(value: &Value) -> Option<AgentStatus> {
    Some(AgentStatus {
        workspace_id: value["workspace_id"].as_str()?.into(),
        pane_id: value["pane_id"].as_str()?.into(),
        status: value["agent_status"].as_str().unwrap_or("unknown").into(),
        kind: value["agent"].as_str().map(str::to_owned),
        name: value["name"].as_str().map(str::to_owned),
        tab_id: value["tab_id"].as_str().map(str::to_owned),
    })
}

fn resync(tx: &mpsc::Sender<HerdrEvent>, snapshot: &Value) -> Result<()> {
    let agents = snapshot["agents"]
        .as_array()
        .ok_or_else(|| Error::new("Herdr snapshot omitted agent inventory"))?
        .iter()
        .map(|agent| {
            status_from(agent).ok_or_else(|| Error::new("invalid agent in Herdr snapshot"))
        })
        .collect::<Result<Vec<_>>>()?;
    send(tx, HerdrEvent::Resync { agents })
}

fn connect(path: &Path, timeout: Duration) -> Result<UnixStream> {
    let stream =
        UnixStream::connect(path).map_err(|_| Error::new("shared Herdr socket is unavailable"))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    Ok(stream)
}

fn write_request(stream: &mut UnixStream, id: &str, method: &str, params: Value) -> Result<()> {
    let mut request = serde_json::to_vec(&json!({"id": id, "method": method, "params": params}))?;
    request.push(b'\n');
    stream
        .write_all(&request)
        .map_err(|_| Error::new("could not write to Herdr socket"))
}

pub(crate) fn rpc(path: &Path, method: &str, params: Value, timeout: Duration) -> Result<Value> {
    let mut stream = connect(path, READ_TICK.min(timeout))?;
    write_request(&mut stream, "node", method, params)?;
    response(
        Frames::default().until(&mut stream, Instant::now() + timeout)?,
        "node",
    )
}

fn response(value: Value, id: &str) -> Result<Value> {
    if value["id"].as_str() != Some(id) {
        return Err(Error::new("Herdr response id mismatch"));
    }
    if let Some(error) = value.get("error") {
        // Error messages may echo arbitrary request content. Emit only the machine error code.
        let code = error["code"]
            .as_str()
            .filter(|s| s.len() <= 80 && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'))
            .unwrap_or("request_failed");
        return Err(Error::new(format!("Herdr request failed ({code})")));
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| Error::new("Herdr response omitted result"))
}

/// Retains partial lines across read timeouts and caps allocation. BufRead::read_line with a
/// fresh String per timeout would silently discard split JSON frames.
#[derive(Default)]
struct Frames {
    pending: Vec<u8>,
}

impl Frames {
    fn next(&mut self, stream: &mut UnixStream) -> Result<Option<Value>> {
        loop {
            if let Some(newline) = self.pending.iter().position(|b| *b == b'\n') {
                if newline > MAX_FRAME {
                    return Err(Error::new("Herdr frame exceeds size limit"));
                }
                let line: Vec<_> = self.pending.drain(..=newline).collect();
                return serde_json::from_slice(&line)
                    .map(Some)
                    .map_err(|_| Error::new("invalid JSON from Herdr socket"));
            }
            if self.pending.len() > MAX_FRAME {
                return Err(Error::new("Herdr frame exceeds size limit"));
            }
            let mut buffer = [0u8; 8192];
            match stream.read(&mut buffer) {
                Ok(0) => return Err(Error::new("Herdr event socket disconnected")),
                Ok(n) => {
                    self.pending.extend_from_slice(&buffer[..n]);
                    // Yield incomplete frames so the caller can check its total deadline and
                    // cancellation even when a peer trickles bytes without a read timeout.
                    if !self.pending.contains(&b'\n') {
                        return Ok(None);
                    }
                }
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    return Ok(None)
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(_) => return Err(Error::new("could not read Herdr socket")),
            }
        }
    }

    fn until(&mut self, stream: &mut UnixStream, deadline: Instant) -> Result<Value> {
        while Instant::now() < deadline {
            if let Some(frame) = self.next(stream)? {
                return Ok(frame);
            }
        }
        Err(Error::new("Herdr socket request timed out"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_contract_uses_per_pane_status_and_no_output() {
        let result = subscriptions(
            &["w1:p1".to_owned(), "w2:p3".to_owned()]
                .into_iter()
                .collect(),
        );
        let statuses: Vec<_> = result
            .iter()
            .filter(|s| s["type"] == "pane.agent_status_changed")
            .collect();
        assert_eq!(statuses.len(), 2);
        assert_eq!(statuses[0]["pane_id"], "w1:p1");
        assert!(!result
            .iter()
            .any(|s| s["type"].as_str().unwrap_or("").contains("output")));
    }

    #[test]
    fn split_frames_survive_timeout_and_multiple_lines() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        client
            .set_read_timeout(Some(Duration::from_millis(10)))
            .unwrap();
        let mut frames = Frames::default();
        server.write_all(b"{\"event\":").unwrap();
        assert!(frames.next(&mut client).unwrap().is_none());
        server
            .write_all(b"\"test\"}\n{\"event\":\"second\"}\n")
            .unwrap();
        assert_eq!(frames.next(&mut client).unwrap().unwrap()["event"], "test");
        assert_eq!(
            frames.next(&mut client).unwrap().unwrap()["event"],
            "second"
        );
    }

    #[test]
    fn response_errors_do_not_echo_arbitrary_contents() {
        let error = response(
            json!({"id":"node","error":{"code":"bad_request","message":"secret sentinel"}}),
            "node",
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "Herdr request failed (bad_request)");
        assert!(response(json!({"id":"other","result":{}}), "node").is_err());
    }
}
