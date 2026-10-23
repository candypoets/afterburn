//! Mindmap's versioned node transport. One persistent socket (and at most one SSH tunnel)
//! per machine carries requests, replies and collaboration events. No invite credentials.
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak, mpsc};
use std::time::Duration;

use figcraft_collab::message::{self, ClientMsg, Hello, ServerMsg};
use figcraft_engine::live::{Event, HostOptions, LiveLink, Presence, Resume, Role, SessionId, SessionInfo, Status};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const PROTOCOL: u64 = 1;
const MAX_FRAME: u64 = 16 * 1024 * 1024;
const RPC_TIMEOUT: Duration = Duration::from_secs(30);
type Reply = Result<Value, String>;
type Pending = BTreeMap<u64, mpsc::Sender<Reply>>;
type Subscribers = BTreeMap<String, Vec<mpsc::Sender<Reply>>>;
type Wake = Arc<dyn Fn() + Send + Sync>;
static WAKE: OnceLock<Mutex<Option<Wake>>> = OnceLock::new();
type Slot = Arc<Mutex<Option<Arc<Connection>>>>;
static POOL: OnceLock<Pool> = OnceLock::new();

#[derive(Default)]
struct Pool {
    slots: Mutex<BTreeMap<String, Slot>>,
    closed: AtomicBool,
}

impl Pool {
    fn connection(&self, address: &str) -> Result<Arc<Connection>, String> {
        let slot = {
            let mut slots = self.slots.lock().unwrap_or_else(|p| p.into_inner());
            if self.closed.load(Ordering::SeqCst) {
                return Err("The Mindmap node transport has shut down.".into());
            }
            slots.entry(address.into()).or_insert_with(|| Arc::new(Mutex::new(None))).clone()
        };
        let mut slot = slot.lock().unwrap_or_else(|p| p.into_inner());
        if self.closed.load(Ordering::SeqCst) {
            return Err("The Mindmap node transport has shut down.".into());
        }
        if let Some(connection) = slot.as_ref().filter(|c| !c.closed.load(Ordering::SeqCst)) {
            return Ok(connection.clone());
        }
        let connection = Connection::open(address)?;
        if self.closed.load(Ordering::SeqCst) {
            connection.fail("The Mindmap node transport has shut down.");
            return Err("The Mindmap node transport has shut down.".into());
        }
        *slot = Some(connection.clone());
        Ok(connection)
    }

    fn shutdown(&self) {
        let slots = {
            let mut slots = self.slots.lock().unwrap_or_else(|p| p.into_inner());
            self.closed.store(true, Ordering::SeqCst);
            std::mem::take(&mut *slots)
        };
        for (_, slot) in slots {
            // Wait for any in-flight connection setup before terminating its tunnel.
            if let Some(connection) = slot.lock().unwrap_or_else(|p| p.into_inner()).take() {
                connection.fail("The Mindmap node transport has shut down.");
            }
        }
    }
}

/// Close every machine connection and reap SSH tunnels after the native event loop exits.
/// This is process-wide cleanup, not a per-window or per-document operation.
pub fn shutdown() {
    POOL.get_or_init(Pool::default).shutdown();
}

/// The node-owned identity of a worktree's live design. This is not a sharing link.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SessionRef {
    pub address: String,
    pub session: String,
    pub workspace: String,
    pub path: String,
    pub branch: String,
}

impl SessionRef {
    /// Private engine routing marker; contains no authority or secret.
    pub fn route(&self) -> String {
        format!("mindmap-node://{}", serde_json::to_string(self).unwrap_or_default())
    }
    pub fn parse(route: &str) -> Result<Self, String> {
        let body = route.strip_prefix("mindmap-node://").ok_or("Open a workspace from a Mindmap node.")?;
        let target: Self = serde_json::from_str(body).map_err(|_| "Invalid node session reference.")?;
        if target.address.is_empty() || target.session.is_empty() {
            return Err("Incomplete node session reference.".into());
        }
        Ok(target)
    }
}

struct Connection {
    writer: Mutex<TcpStream>,
    pending: Mutex<Pending>,
    subscribers: Mutex<Subscribers>,
    next: AtomicU64,
    closed: AtomicBool,
    tunnel: Mutex<Option<Child>>,
    /// Coalesce state notifications while the UI sleeps; collaboration is never coalesced.
    events: Mutex<BTreeMap<String, Value>>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.writer.get_mut().unwrap_or_else(|p| p.into_inner()).shutdown(Shutdown::Both);
        if let Some(mut tunnel) = self.tunnel.get_mut().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = tunnel.kill();
            let _ = tunnel.wait();
        }
    }
}

impl Connection {
    fn open(address: &str) -> Result<Arc<Self>, String> {
        let host = crate::node_hosts::resolve(address);
        let (stream, mut tunnel) = crate::node_hosts::connect(&host)?;
        let prepared = (|| -> std::io::Result<TcpStream> {
            stream.set_nodelay(true)?;
            stream.set_read_timeout(None)?;
            stream.set_write_timeout(Some(Duration::from_secs(10)))?;
            stream.try_clone()
        })();
        let reader = match prepared {
            Ok(reader) => reader,
            Err(error) => {
                if let Some(mut child) = tunnel.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                return Err(format!("Could not configure node connection: {error}"));
            }
        };
        let connection = Arc::new(Self {
            writer: Mutex::new(stream),
            pending: Mutex::new(BTreeMap::new()),
            subscribers: Mutex::new(BTreeMap::new()),
            next: AtomicU64::new(1),
            closed: AtomicBool::new(false),
            tunnel: Mutex::new(tunnel),
            events: Mutex::new(BTreeMap::new()),
        });
        let weak = Arc::downgrade(&connection);
        std::thread::Builder::new().name("mindmap-node-reader".into()).spawn(move || read_frames(reader, weak)).map_err(|e| e.to_string())?;
        let hello = connection.request(json!({"method":"hello"}), Duration::from_secs(10))?;
        if hello.get("protocol").and_then(Value::as_u64) != Some(PROTOCOL) || hello.get("ok") != Some(&Value::Bool(true)) {
            connection.fail("Incompatible Mindmap node protocol; update the app and node together.");
            return Err("Incompatible Mindmap node protocol; update the app and node together.".into());
        }
        Ok(connection)
    }

    fn fail(&self, reason: &str) {
        // Serialize cleanup so shutdown also waits for a reader's concurrent failure cleanup.
        let mut tunnel = self.tunnel.lock().unwrap_or_else(|p| p.into_inner());
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(mut tunnel) = tunnel.take() {
            let _ = tunnel.kill();
            let _ = tunnel.wait();
        }
        let _ = self.writer.lock().unwrap_or_else(|p| p.into_inner()).shutdown(Shutdown::Both);
        let pending = std::mem::take(&mut *self.pending.lock().unwrap_or_else(|p| p.into_inner()));
        for (_, tx) in pending {
            let _ = tx.send(Err(reason.into()));
        }
        let subscribers = std::mem::take(&mut *self.subscribers.lock().unwrap_or_else(|p| p.into_inner()));
        for (_, listeners) in subscribers {
            for tx in listeners {
                let _ = tx.send(Err(reason.into()));
            }
        }
    }

    fn request(&self, mut request: Value, timeout: Duration) -> Reply {
        if !request.is_object() {
            return Err("A node request must be a JSON object.".into());
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        request["id"] = json!(id);
        request["version"] = json!(PROTOCOL);
        let mut bytes = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_FRAME {
            return Err("Node request exceeds the frame limit.".into());
        }
        bytes.push(b'\n');
        let (tx, rx) = mpsc::channel();
        {
            let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
            if self.closed.load(Ordering::SeqCst) {
                return Err("The node connection is closed; reopen the workspace.".into());
            }
            pending.insert(id, tx);
        }
        let written = self.writer.lock().unwrap_or_else(|p| p.into_inner()).write_all(&bytes);
        if written.is_err() {
            self.fail("The node connection was lost. Reopen the workspace to reconnect.");
        }
        let reply =
            rx.recv_timeout(timeout).unwrap_or_else(|_| Err("The node request timed out; its outcome is unknown. Refresh before retrying.".into()));
        self.pending.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
        reply
    }

    fn subscribe(&self, session: &str) -> mpsc::Receiver<Reply> {
        let (tx, rx) = mpsc::channel();
        let mut subscribers = self.subscribers.lock().unwrap_or_else(|p| p.into_inner());
        if self.closed.load(Ordering::SeqCst) {
            let _ = tx.send(Err("The node connection is closed.".into()));
        } else {
            subscribers.entry(session.into()).or_default().push(tx);
        }
        rx
    }
}

fn read_frames(stream: TcpStream, weak: Weak<Connection>) {
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        let read = reader.by_ref().take(MAX_FRAME + 1).read_line(&mut line);
        let Some(connection) = weak.upgrade() else { return };
        if !matches!(read, Ok(n) if n > 0) {
            connection.fail("The node connection was lost. Reopen the workspace to reconnect.");
            return;
        }
        if line.len() as u64 > MAX_FRAME || !line.ends_with('\n') {
            connection.fail("The node sent an oversized or incomplete frame.");
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            connection.fail("The node sent invalid JSON.");
            return;
        };
        if value.get("version").and_then(Value::as_u64) != Some(PROTOCOL) {
            connection.fail("Incompatible Mindmap node protocol; update the app and node together.");
            return;
        }
        if value.get("event").is_some() {
            if value["event"] != "collab" {
                let key = format!(
                    "{}:{}",
                    value["event"].as_str().unwrap_or(""),
                    value["job"]["id"].as_str().or_else(|| value["session"].as_str()).unwrap_or("")
                );
                let mut events = connection.events.lock().unwrap_or_else(|p| p.into_inner());
                if events.len() < 512 || events.contains_key(&key) {
                    events.insert(key, value.clone());
                }
                drop(events);
                if let Some(wake) = WAKE.get().and_then(|slot| slot.lock().ok().and_then(|w| w.clone())) {
                    wake();
                }
            }
            if let Some(session) = value.get("session").and_then(Value::as_str) {
                let mut subscribers = connection.subscribers.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(listeners) = subscribers.get_mut(session) {
                    listeners.retain(|tx| tx.send(Ok(value.clone())).is_ok());
                }
            }
        } else if let Some(id) = value.get("id").and_then(Value::as_u64) {
            if let Some(tx) = connection.pending.lock().unwrap_or_else(|p| p.into_inner()).remove(&id) {
                let _ = tx.send(Ok(value));
            }
        } else {
            connection.fail("The node sent a reply without a request id.");
            return;
        }
    }
}

fn connection(address: &str) -> Result<Arc<Connection>, String> {
    POOL.get_or_init(Pool::default).connection(address)
}

/// Drain state notifications without connecting or waiting for network I/O.
pub fn take_events() -> Vec<(String, Value)> {
    let Some(pool) = POOL.get() else { return Vec::new() };
    let Ok(pool) = pool.slots.try_lock() else { return Vec::new() };
    let mut events = Vec::new();
    for (address, slot) in pool.iter() {
        let Ok(slot) = slot.try_lock() else { continue };
        let Some(connection) = slot.as_ref() else { continue };
        let pending = std::mem::take(&mut *connection.events.lock().unwrap_or_else(|p| p.into_inner()));
        events.extend(pending.into_values().map(|event| (address.clone(), event)));
    }
    events
}

/// Synchronous RPC for background workers. Never retries a mutation after an ambiguous failure.
pub fn rpc(address: &str, request: Value, timeout: Duration) -> Reply {
    connection(address)?.request(request, timeout)
}

pub fn ask(address: &str, request: &str, timeout: Duration) -> Result<String, String> {
    let request = serde_json::from_str(request).map_err(|_| "Invalid node request.".to_string())?;
    rpc(address, request, timeout).map(|reply| reply.to_string())
}

pub fn command(target: &SessionRef, command: &str, params: Value) -> Reply {
    rpc(&target.address, json!({"method":"session.command","session":target.session,"command":command,"params":params}), RPC_TIMEOUT)
}

fn accepted(reply: Value) -> Reply {
    if reply.get("ok") == Some(&Value::Bool(true)) {
        Ok(reply)
    } else {
        Err(reply.get("error").and_then(Value::as_str).unwrap_or("The node refused the request.").to_string())
    }
}

/// Engine live transport used only by Mindmap mode. All methods return immediately to the UI;
/// one ordered worker per session multiplexes its messages through the machine connection.
pub struct NodeLiveLink {
    wake: Arc<dyn Fn() + Send + Sync>,
    tx: mpsc::Sender<(SessionId, Event)>,
    rx: mpsc::Receiver<(SessionId, Event)>,
    sessions: BTreeMap<SessionId, mpsc::Sender<Option<ClientMsg>>>,
}

impl NodeLiveLink {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, rx) = mpsc::channel();
        *WAKE.get_or_init(|| Mutex::new(None)).lock().unwrap_or_else(|p| p.into_inner()) = Some(wake.clone());
        Self { wake, tx, rx, sessions: BTreeMap::new() }
    }
}
impl LiveLink for NodeLiveLink {
    fn can_host(&self) -> bool {
        false
    }
    fn host(&mut self, _: SessionId, _: HostOptions) -> Result<SessionInfo, String> {
        Err("Mindmap nodes host workspaces; open a workspace first.".into())
    }
    fn join(&mut self, id: SessionId, route: &str, name: &str) -> Result<(), String> {
        self.join_resuming(id, route, name, None)
    }
    fn join_resuming(&mut self, id: SessionId, route: &str, name: &str, resume: Option<Resume>) -> Result<(), String> {
        let target = SessionRef::parse(route)?;
        self.leave(id);
        let (tx, rx) = mpsc::channel();
        let events = self.tx.clone();
        let wake = self.wake.clone();
        let name = name.to_string();
        std::thread::Builder::new()
            .name("mindmap-node-session".into())
            .spawn(move || {
                let emit = |event| {
                    let _ = events.send((id, event));
                    wake();
                };
                if let Err(reason) = session_worker(&target, &name, resume, rx, &emit) {
                    emit(Event::Status(Status::Failed { reason }));
                }
            })
            .map_err(|e| e.to_string())?;
        self.sessions.insert(id, tx);
        Ok(())
    }
    fn leave(&mut self, id: SessionId) {
        if let Some(tx) = self.sessions.remove(&id) {
            let _ = tx.send(None);
        }
    }
    fn presence(&mut self, id: SessionId, presence: &Presence) {
        self.send(id, ClientMsg::Presence(presence.clone()));
    }
    fn send(&mut self, id: SessionId, message: ClientMsg) {
        if let Some(tx) = self.sessions.get(&id) {
            let _ = tx.send(Some(message));
        }
    }
    fn poll(&mut self) -> Vec<(SessionId, Event)> {
        self.rx.try_iter().collect()
    }
}
impl Drop for NodeLiveLink {
    fn drop(&mut self) {
        for (_, tx) in std::mem::take(&mut self.sessions) {
            let _ = tx.send(None);
        }
    }
}

fn session_worker(
    target: &SessionRef,
    name: &str,
    resume: Option<Resume>,
    outgoing: mpsc::Receiver<Option<ClientMsg>>,
    emit: &dyn Fn(Event),
) -> Result<(), String> {
    let connection = connection(&target.address)?;
    let incoming = connection.subscribe(&target.session);
    let hello = Hello {
        protocol: message::PROTOCOL,
        file: target.session.clone(),
        token: String::new(),
        name: name.into(),
        color: String::new(),
        client_version: message::client_version(),
        resume,
    };
    let joined = accepted(connection.request(json!({"method":"session.join","session":target.session,"hello":hello}), RPC_TIMEOUT)?)?;
    let messages: Vec<ServerMsg> = serde_json::from_value(joined.get("messages").cloned().unwrap_or(Value::Null))
        .map_err(|_| "The node returned an invalid session welcome.")?;
    if !messages.iter().any(|m| matches!(m, ServerMsg::Welcome { .. })) {
        return Err("The node did not send a session welcome.".into());
    }
    let mut info = None;
    let mut peers = Vec::new();
    for message in messages {
        deliver(message, target, name, &mut info, &mut peers, emit)?;
    }
    loop {
        for event in incoming.try_iter() {
            let event = event?;
            if event.get("event").and_then(Value::as_str) == Some("collab") {
                let message = serde_json::from_value(event["message"].clone()).map_err(|_| "The node sent an invalid collaboration message.")?;
                deliver(message, target, name, &mut info, &mut peers, emit)?;
            }
        }
        match outgoing.recv_timeout(Duration::from_millis(10)) {
            Ok(Some(message)) => {
                accepted(connection.request(json!({"method":"session.send","session":target.session,"message":message}), RPC_TIMEOUT)?)?;
            }
            Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                accepted(connection.request(json!({"method":"session.leave","session":target.session}), RPC_TIMEOUT)?)?;
                return Ok(());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn deliver(
    message: ServerMsg,
    target: &SessionRef,
    name: &str,
    info: &mut Option<SessionInfo>,
    peers: &mut Vec<message::Peer>,
    emit: &dyn Fn(Event),
) -> Result<(), String> {
    match message {
        ServerMsg::Welcome { client_id, peers: ref welcomed_peers, access, .. } => {
            let joined = SessionInfo {
                role: Role::Guest,
                me: client_id,
                name: name.into(),
                color: String::new(),
                file_name: String::new(),
                pages: Vec::new(),
                link: target.route(),
                url: String::new(),
                address: target.address.clone(),
                network: false,
                access,
            };
            *info = Some(joined.clone());
            *peers = welcomed_peers.clone();
            emit(Event::Welcome { info: joined, peers: peers.clone() });
            emit(Event::Sync(message));
        }
        ServerMsg::PeerJoined(peer) => {
            if let Some(info) = info.as_mut().filter(|i| i.me == peer.client_id) {
                info.name = peer.name;
                info.color = peer.color;
                emit(Event::Welcome { info: info.clone(), peers: peers.clone() });
            } else {
                peers.retain(|p| p.client_id != peer.client_id);
                peers.push(peer.clone());
                emit(Event::PeerJoined(peer));
            }
        }
        ServerMsg::PeerLeft { client_id } => {
            peers.retain(|p| p.client_id != client_id);
            emit(Event::PeerLeft(client_id));
        }
        ServerMsg::Presence { client_id, presence } => emit(Event::Presence(client_id, presence)),
        ServerMsg::Signal { client_id, signal } => emit(Event::Signal(client_id, signal)),
        ServerMsg::Changes(_) | ServerMsg::Reject { .. } => emit(Event::Sync(message)),
        ServerMsg::Refused { reason } => return Err(reason),
        ServerMsg::Hosting(hosting) => emit(Event::Hosting(*hosting)),
        ServerMsg::Reported { error } => emit(Event::Reported(error)),
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests;
