//! Versioned, multiplexed node transport. All collaboration and job traffic shares this socket.
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, Arc, Mutex, MutexGuard,
};
use std::thread;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::node_documents::NodeDocuments;
use crate::node_git::{self, FrozenSync};
use crate::node_herdr::{HerdrEvent, HerdrMonitor, NodeHerdr};
use figcraft_collab::{
    message::{ClientMsg, Hello, ServerMsg},
    server::Outgoing,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const PROTOCOL: u64 = 1;
pub const MAX_FRAME: usize = 16 * 1024 * 1024;
const MAX_CONNECTIONS: usize = 128;

struct Connection {
    tx: mpsc::SyncSender<Value>,
    socket: TcpStream,
    peers: HashMap<String, u32>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Job {
    frozen: FrozenSync,
    session: String,
    workspace: String,
    agent: Value,
    state: String,
    seen_active: bool,
    prepared: bool,
    reason: Option<String>,
    commit: Option<String>,
}

pub struct Node {
    // Covers authority application, event enqueue and the corresponding reply. Dropping the
    // document mutex alone must not allow another thread to broadcast a later sequence first.
    document_order: Mutex<()>,
    docs: Mutex<NodeDocuments>,
    herdr: NodeHerdr,
    monitor: Mutex<HerdrMonitor>,
    connections: Mutex<HashMap<u64, Connection>>,
    jobs: Mutex<BTreeMap<String, Job>>,
    workspace_sessions: Mutex<HashMap<String, String>>,
    session_events: Mutex<HashMap<String, Value>>,
    next_connection: AtomicU64,
    address: String,
}

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| Error::new("node state lock poisoned"))
}

pub fn serve(address: &str) -> Result<()> {
    let listener = TcpListener::bind(address)?;
    let bound = listener.local_addr()?;
    if !bound.ip().is_loopback() {
        return Err(Error::new(
            "node must bind loopback; remote access uses SSH forwarding",
        ));
    }
    let herdr = NodeHerdr::new(&crate::config::Config::discover()?)?;
    let monitor = herdr.monitor()?;
    let node = Arc::new(Node {
        document_order: Mutex::new(()),
        docs: Mutex::new(NodeDocuments::new()),
        herdr,
        monitor: Mutex::new(monitor),
        connections: Mutex::new(HashMap::new()),
        jobs: Mutex::new(BTreeMap::new()),
        workspace_sessions: Mutex::new(HashMap::new()),
        session_events: Mutex::new(HashMap::new()),
        next_connection: AtomicU64::new(1),
        address: bound.to_string(),
    });
    let background = node.clone();
    thread::Builder::new()
        .name("mindmap-node-maintenance".into())
        .spawn(move || loop {
            if let Err(error) = background.maintain() {
                eprintln!("mindmap-node: {error}");
            }
            thread::sleep(Duration::from_millis(250));
        })?;
    println!("mindmap-node protocol {PROTOCOL} on {bound}");
    for incoming in listener.incoming() {
        let stream = incoming?;
        if lock(&node.connections)?.len() >= MAX_CONNECTIONS {
            continue;
        }
        let connection = node.next_connection.fetch_add(1, Ordering::Relaxed);
        let shared = node.clone();
        thread::Builder::new()
            .name(format!("mindmap-node-client-{connection}"))
            .spawn(move || {
                if let Err(error) = handle_connection(stream, connection, &shared) {
                    eprintln!("mindmap-node connection {connection}: {error}");
                }
                let _ = shared.disconnect(connection);
            })?;
    }
    Ok(())
}

fn handle_connection(stream: TcpStream, id: u64, node: &Arc<Node>) -> Result<()> {
    stream.set_nodelay(true)?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;
    let mut writer = stream.try_clone()?;
    let (tx, rx) = mpsc::sync_channel::<Value>(256);
    lock(&node.connections)?.insert(
        id,
        Connection {
            tx: tx.clone(),
            socket: stream.try_clone()?,
            peers: HashMap::new(),
        },
    );
    thread::Builder::new()
        .name(format!("mindmap-node-writer-{id}"))
        .spawn(move || {
            for value in rx {
                let bytes = match serde_json::to_vec(&value) {
                    Ok(bytes) if bytes.len() <= MAX_FRAME => bytes,
                    _ => break,
                };
                if writer
                    .write_all(&bytes)
                    .and_then(|_| writer.write_all(b"\n"))
                    .is_err()
                {
                    break;
                }
            }
            let _ = writer.shutdown(std::net::Shutdown::Both);
        })?;
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        let count = (&mut reader)
            .take(MAX_FRAME as u64 + 1)
            .read_line(&mut line)?;
        if count == 0 {
            return Ok(());
        }
        if count > MAX_FRAME {
            return Err(Error::new("request exceeds 16 MiB limit"));
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => {
                node.reply(
                    id,
                    json!({"version":PROTOCOL,"id":null,"ok":false,"error":"invalid JSON"}),
                );
                continue;
            }
        };
        // Mutations are processed in wire order. Slow host/agent calls run separately so that
        // the same connection can continue carrying live edits while an agent starts.
        let slow = matches!(
            request["method"].as_str(),
            Some("init" | "send" | "sync" | "host.ensure" | "workspaces" | "worktree.create")
        );
        if slow {
            let shared = node.clone();
            thread::Builder::new()
                .name(format!("mindmap-node-job-{id}"))
                .spawn(move || shared.dispatch(id, request))?;
        } else {
            node.dispatch(id, request);
        }
    }
}

impl Node {
    fn dispatch(&self, connection: u64, request: Value) {
        let slow = matches!(
            request["method"].as_str(),
            Some("init" | "send" | "sync" | "host.ensure" | "workspaces" | "worktree.create")
        );
        let _order = if slow {
            None
        } else {
            match lock(&self.document_order) {
                Ok(guard) => Some(guard),
                Err(error) => {
                    self.reply(connection,json!({"version":PROTOCOL,"id":request["id"],"ok":false,"error":error.to_string()}));
                    return;
                }
            }
        };
        let result = if request.get("version").and_then(Value::as_u64) != Some(PROTOCOL) {
            Err(Error::new(format!(
                "incompatible node protocol; expected version {PROTOCOL}"
            )))
        } else {
            self.handle(connection, &request)
        };
        let mut reply = match result {
            Ok(mut value) => {
                value["ok"] = json!(true);
                value
            }
            Err(error) => json!({"ok":false,"error":error.to_string()}),
        };
        reply["version"] = json!(PROTOCOL);
        reply["id"] = request.get("id").cloned().unwrap_or(Value::Null);
        self.reply(connection, reply);
    }

    fn reply(&self, id: u64, value: Value) {
        if let Ok(connections) = self.connections.lock() {
            if let Some(connection) = connections.get(&id) {
                if connection.tx.try_send(value).is_err() {
                    let _ = connection.socket.shutdown(std::net::Shutdown::Both);
                }
            }
        }
    }

    fn broadcast(&self, value: Value) {
        let ids = self
            .connections
            .lock()
            .map(|c| c.keys().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        for id in ids {
            self.reply(id, value.clone());
        }
    }

    fn deliver(&self, session: &str, outgoing: Vec<Outgoing>) {
        let recipients = self
            .connections
            .lock()
            .map(|connections| {
                connections
                    .iter()
                    .filter_map(|(id, c)| c.peers.get(session).map(|peer| (*id, *peer)))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for message in outgoing {
            for (id, peer) in &recipients {
                let body = match &message {
                    Outgoing::To(to, m) if to == peer => Some(m),
                    Outgoing::All(m) => Some(m),
                    Outgoing::Others(from, m) if from != peer => Some(m),
                    _ => None,
                };
                if let Some(message) = body {
                    self.reply(*id, json!({"version":PROTOCOL,"event":"collab","session":session,"message":message}));
                }
            }
        }
    }

    fn disconnect(&self, connection: u64) -> Result<()> {
        let _order = lock(&self.document_order)?;
        let peers = lock(&self.connections)?
            .remove(&connection)
            .map(|c| c.peers)
            .unwrap_or_default();
        for (session, peer) in peers {
            let outgoing = lock(&self.docs)?.leave(&session, peer)?;
            self.deliver(&session, outgoing);
        }
        Ok(())
    }

    fn session(&self, req: &Value) -> Result<String> {
        if let Some(session) = req["session"].as_str() {
            lock(&self.docs)?.metadata(session)?;
            return Ok(session.into());
        }
        let opened = self.open(req)?;
        Ok(required(&opened, "session")?.into())
    }

    fn open(&self, req: &Value) -> Result<Value> {
        let (workspace, label, path) = if let Some(path) = req["path"].as_str() {
            (
                req["workspace"].as_str().unwrap_or("").to_string(),
                Path::new(path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Mindmap")
                    .to_string(),
                PathBuf::from(path),
            )
        } else {
            let workspace = self.herdr.find_workspace(required(req, "workspace")?)?;
            (
                workspace.id,
                workspace.label,
                PathBuf::from(
                    workspace
                        .path
                        .ok_or_else(|| Error::new("workspace has no checkout path"))?,
                ),
            )
        };
        let metadata = lock(&self.docs)?.open(&path)?;
        if !workspace.is_empty() {
            lock(&self.workspace_sessions)?.insert(workspace.clone(), metadata.id.clone());
        }
        self.restore_jobs(&metadata.id, &metadata.worktree)?;
        Ok(
            json!({"session":metadata.id,"path":metadata.worktree,"branch":metadata.branch,"name":format!("Mindmap · {label}"),"workspace":workspace,"label":label}),
        )
    }

    fn handle(&self, connection: u64, req: &Value) -> Result<Value> {
        match required(req, "method")? {
            "hello" | "health" => Ok(
                json!({"service":"mindmap-node","protocol":PROTOCOL,"version_name":env!("CARGO_PKG_VERSION"),"capabilities":["collaboration","git-sync","merge","herdr-events"]}),
            ),
            "host.status" => Ok(serde_json::to_value(self.herdr.status()?)?),
            "host.ensure" => Ok(serde_json::to_value(
                self.herdr
                    .ensure(req["install_herdr"].as_bool().unwrap_or(false))?,
            )?),
            "workspaces" => Ok(
                json!({"workspaces":self.herdr.workspaces()?,"clis":crate::agent::installed_kinds()}),
            ),
            "open" | "session.open" => self.open(req),
            "session.list" => Ok(json!({"sessions":lock(&self.docs)?.sessions()})),
            "session.join" => {
                let session = self.session(req)?;
                let hello: Hello = serde_json::from_value(req["hello"].clone())?;
                let mut docs = lock(&self.docs)?;
                let (peer, outgoing) = docs.join(&session, hello)?;
                let previous = lock(&self.connections)?
                    .get_mut(&connection)
                    .and_then(|c| c.peers.insert(session.clone(), peer));
                let old = match previous {
                    Some(previous) if previous != peer => docs.leave(&session, previous)?,
                    _ => Vec::new(),
                };
                drop(docs);
                self.deliver(&session, old);
                // The welcome belongs in the join response; other recipients receive peer events.
                let messages: Vec<ServerMsg> = outgoing
                    .iter()
                    .filter_map(|m| match m {
                        Outgoing::To(id, m) if *id == peer => Some(m.clone()),
                        _ => None,
                    })
                    .collect();
                self.deliver(
                    &session,
                    outgoing
                        .into_iter()
                        .filter(|m| !matches!(m,Outgoing::To(id,_) if *id == peer))
                        .collect(),
                );
                Ok(json!({"peer":peer,"messages":messages}))
            }
            "session.send" => {
                let session = self.session(req)?;
                let peer = lock(&self.connections)?
                    .get(&connection)
                    .and_then(|c| c.peers.get(&session))
                    .copied()
                    .ok_or_else(|| Error::new("join this session before sending"))?;
                let message: ClientMsg = serde_json::from_value(req["message"].clone())?;
                let outgoing = lock(&self.docs)?.receive(&session, peer, message)?;
                self.deliver(&session, outgoing);
                Ok(json!({}))
            }
            "session.leave" => {
                let session = required(req, "session")?;
                let peer = lock(&self.connections)?
                    .get_mut(&connection)
                    .and_then(|c| c.peers.remove(session));
                if let Some(peer) = peer {
                    let outgoing = lock(&self.docs)?.leave(session, peer)?;
                    self.deliver(session, outgoing);
                }
                Ok(json!({}))
            }
            "session.inspect" => {
                let session = self.session(req)?;
                let node = req["node"].as_u64().map(figcraft_doc::NodeId);
                let depth = req["depth"].as_u64().unwrap_or(6).min(24) as usize;
                Ok(json!({"document":lock(&self.docs)?.inspect(&session,node,depth)?}))
            }
            "session.render" => {
                let session = self.session(req)?;
                let ids: Vec<u64> = req["ids"]
                    .as_array()
                    .map(|ids| ids.iter().filter_map(Value::as_u64).take(64).collect())
                    .unwrap_or_default();
                let page = req["page"].as_u64();
                let scale = req["scale"].as_f64().unwrap_or(1.0);
                let document = lock(&self.docs)?.document(&session)?;
                crate::node_documents::render(document, &ids, page, scale)
            }
            "session.commands" => {
                let filter = req["filter"].as_str().unwrap_or("");
                let commands: Vec<Value> = figcraft_engine::all_commands().iter().filter(|c|c.id.contains(filter)).map(|c|json!({"id":c.id,"label":c.label,"params":c.params,"history":c.history})).collect();
                Ok(json!({"commands":commands}))
            }
            "session.command" => {
                let session = self.session(req)?;
                let command = required(req, "command")?;
                let params = req
                    .get("params")
                    .filter(|v| v.is_object())
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let (result, outgoing) = lock(&self.docs)?.execute(&session, command, &params)?;
                self.deliver(&session, outgoing);
                Ok(json!({"result":result}))
            }
            "session.import" => {
                let session = self.session(req)?;
                let path = Path::new(required(req, "source")?);
                if std::fs::metadata(path)?.len() > 64 * 1024 * 1024 {
                    return Err(Error::new("design import exceeds 64 MiB"));
                }
                let (document, warnings) =
                    figcraft_format::load_file(path).map_err(|e| Error::new(e.to_string()))?;
                let outgoing = lock(&self.docs)?.replace(&session, document)?;
                self.deliver(&session, outgoing);
                let outgoing = lock(&self.docs)?.save(&session)?;
                self.deliver(&session, outgoing);
                Ok(json!({"session":session,"warnings":warnings}))
            }
            "session.save" => {
                let session = self.session(req)?;
                let outgoing = lock(&self.docs)?.save(&session)?;
                self.deliver(&session, outgoing);
                Ok(json!({}))
            }
            "session.resolve" => {
                let session = self.session(req)?;
                let picks: Vec<crate::node_documents::MergePick> =
                    serde_json::from_value(req.get("picks").cloned().unwrap_or_else(|| json!([])))?;
                let outcome = lock(&self.docs)?.resolve_disk(&session, &picks)?;
                self.deliver(&session, outcome.outgoing);
                Ok(json!({"conflicts":outcome.conflicts}))
            }
            "diff" => {
                let session = self.session(req)?;
                let docs = lock(&self.docs)?;
                let metadata = docs.metadata(&session)?;
                let document = docs.document(&session)?;
                drop(docs);
                node_git::diff_paginated(
                    &metadata.worktree,
                    &document,
                    page(req)?,
                    req["offset"].as_u64().unwrap_or(0) as usize,
                    req["limit"].as_u64().unwrap_or(200).min(1000) as usize,
                )
            }
            "init" | "send" => self.send(req),
            "sync" => self.start_sync(req),
            // Explicitly capture an already-running external agent's task, without prompting
            // a second agent. It has the same immutable input and commit proof as UI Sync.
            "sync.capture" => self.capture_sync(req),
            "sync_status" => self.sync_status(required(req, "job")?),
            "sync.prepare" => self.prepare_sync(required(req, "job")?),
            "sync.finish" => self.finish_sync(required(req, "job")?, req["commit"].as_str()),
            "sync.cancel" => {
                let picks: Vec<crate::node_documents::MergePick> =
                    serde_json::from_value(req.get("picks").cloned().unwrap_or_else(|| json!([])))?;
                self.cancel_sync(required(req, "job")?, &picks)
            }
            "worktree.create" => self
                .herdr
                .create_worktree(&serde_json::from_value(req.clone())?),
            "merge.plan" => self.merge(req, false),
            "merge.apply" => self.merge(req, true),
            "merge.finish" => self.finish_merge(req),
            "merge.abort" => self.abort_merge(req),
            method => Err(Error::new(format!("unknown node method {method:?}"))),
        }
    }

    fn context(&self, session: &str, req: &Value, commented: &[u64]) -> Result<String> {
        let docs = lock(&self.docs)?;
        let metadata = docs.metadata(session)?;
        let document = docs.document(session)?;
        drop(docs);
        // Sync names `page`; comments and new pages send `page_id` (and the comments their own).
        let scope = match page(req)? {
            Some(page) => Some(page),
            None => req["page_id"].as_u64().filter(|id| *id > 0),
        };
        let commented: Vec<figcraft_doc::NodeId> = commented
            .iter()
            .copied()
            .map(figcraft_doc::NodeId)
            .collect();
        let worktree = metadata.worktree.display().to_string();
        Ok(crate::node_context::render(
            &document,
            &crate::node_context::Request {
                worktree: &worktree,
                branch: &metadata.branch,
                node: &self.address,
                session,
                page: scope.map(figcraft_doc::NodeId),
                commented: &commented,
                new_page: req["new_page"].as_bool().unwrap_or(false),
                source: req["source_node_id"].as_u64().map(figcraft_doc::NodeId),
            },
        ))
    }

    fn send(&self, req: &Value) -> Result<Value> {
        let workspace = self.herdr.find_workspace(required(req, "workspace")?)?;
        let target = crate::agent::parse_target(req["target"].as_str())?;
        let comments = crate::comments::parse_comments(req.get("comments"))?;
        let initial = required(req, "method")? == "init";
        let prompt=req["prompt"].as_str().unwrap_or(if initial {"Create the project map in the live session. Read the code, plan and implement the requested map; source every claim."} else {""});
        let session = self.session(req)?;
        self.validate_workspace(&workspace, &session)?;
        let commented: Vec<u64> = comments
            .iter()
            .filter_map(|comment| comment.node_id)
            .collect();
        let mut context_req = req.clone();
        if context_req["page_id"].is_null() {
            if let Some(page) = comments.iter().find_map(|comment| comment.page_id) {
                context_req["page_id"] = json!(page);
            }
        }
        let context = self.context(&session, &context_req, &commented)?;
        let prompt = crate::comments::format_prompt(
            &workspace.label,
            workspace.path.as_deref(),
            &comments,
            prompt,
            Some(&context),
        )?;
        let agent = self.herdr.prompt(&workspace, &target, &prompt)?;
        if !agent.prompted || matches!(agent.state, crate::agent::AgentState::Error) {
            return Err(Error::new(
                agent
                    .reason
                    .as_deref()
                    .unwrap_or("agent did not accept the prompt"),
            ));
        }
        let mut result = self.open(&json!({"workspace":workspace.id}))?;
        result["agent"] = serde_json::to_value(agent)?;
        result["sent"] = json!(comments.len());
        Ok(result)
    }

    fn start_sync(&self, req: &Value) -> Result<Value> {
        let workspace = self.herdr.find_workspace(required(req, "workspace")?)?;
        let target = crate::agent::parse_target(req["target"].as_str())?;
        self.herdr.preflight(&workspace, &target)?;
        let session = self.session(req)?;
        self.validate_workspace(&workspace, &session)?;
        let docs = lock(&self.docs)?;
        let metadata = docs.metadata(&session)?;
        let document = docs.document(&session)?;
        drop(docs);
        let mut jobs = lock(&self.jobs)?;
        if jobs
            .values()
            .any(|j| j.session == session && j.state == "in_flight")
        {
            return Err(Error::new("a sync is already running for this session"));
        }
        let frozen = node_git::freeze(&metadata.worktree, &document, page(req)?)?;
        let id = frozen.id.clone();
        let since = frozen
            .baseline
            .as_ref()
            .map(|b| b.commit.as_str())
            .unwrap_or("first sync");
        let prompt=format!("Sync {} ({}) since {}.\nLarge work: plan it and split it across sub-agents in your own harness; finish all authorized work.\n{}\nJob: {}\nRead complete design drift with node diff and code drift with git. At completion call node sync.prepare with job {}, then commit code and .mindmap/map.figcraft together with this exact trailer:\n{}\nCall sync.finish with job {} and commit SHA after committing. Agent idle alone cannot finish this sync. Changes made after its start remain drift. Do not commit node cache or private links.\nNOTE: {}",workspace.label,metadata.worktree.display(),since,self.context(&session,req,&[])?,id,id,frozen.trailer,id,req["prompt"].as_str().unwrap_or(""));
        let job = Job {
            frozen,
            session: session.clone(),
            workspace: workspace.id.clone(),
            agent: json!({}),
            state: "in_flight".into(),
            seen_active: false,
            prepared: false,
            reason: None,
            commit: None,
        };
        self.persist_job(&job)?;
        jobs.insert(id.clone(), job);
        drop(jobs);
        let agent = match self.herdr.prompt(&workspace, &target, &prompt) {
            Ok(agent)
                if agent.prompted && !matches!(agent.state, crate::agent::AgentState::Error) =>
            {
                agent
            }
            result => {
                let error = match result {
                    Err(e) => e,
                    Ok(a) => Error::new(
                        a.reason
                            .as_deref()
                            .unwrap_or("agent did not accept the prompt"),
                    ),
                };
                let _order = lock(&self.document_order)?;
                let _ = self.cancel_sync(&id, &[]);
                return Err(error);
            }
        };
        let mut jobs = lock(&self.jobs)?;
        let job = jobs
            .get_mut(&id)
            .ok_or_else(|| Error::new("sync disappeared"))?;
        job.agent = serde_json::to_value(agent)?;
        self.persist_job(job)?;
        let result = json!({"job":job_view(job),"agent":job.agent});
        self.broadcast(json!({"version":PROTOCOL,"event":"job","job":job_view(job)}));
        Ok(result)
    }

    fn capture_sync(&self, req: &Value) -> Result<Value> {
        let session = self.session(req)?;
        let docs = lock(&self.docs)?;
        let metadata = docs.metadata(&session)?;
        if !metadata.active {
            return Err(Error::new(
                "branch changed; reopen the worktree before Sync",
            ));
        }
        let document = docs.document(&session)?;
        drop(docs);
        let mut jobs = lock(&self.jobs)?;
        if jobs
            .values()
            .any(|j| j.session == session && j.state == "in_flight")
        {
            return Err(Error::new("a sync is already running for this session"));
        }
        let frozen = node_git::freeze(&metadata.worktree, &document, page(req)?)?;
        let job = Job {
            frozen,
            session,
            workspace: req["workspace"].as_str().unwrap_or("").into(),
            agent: json!({"kind":"external","state":"existing","prompted":false}),
            state: "in_flight".into(),
            seen_active: true,
            prepared: false,
            reason: None,
            commit: None,
        };
        self.persist_job(&job)?;
        let result = json!({"job":job_view(&job),"trailer":job.frozen.trailer});
        jobs.insert(job.frozen.id.clone(), job);
        Ok(result)
    }

    fn prepare_sync(&self, id: &str) -> Result<Value> {
        let mut jobs = lock(&self.jobs)?;
        let job = jobs
            .get_mut(id)
            .ok_or_else(|| Error::new("unknown sync job"))?;
        if job.state != "in_flight" {
            return Err(Error::new("sync is no longer running"));
        }
        let _ = node_git::frozen_document(&job.frozen)?; // verifies the immutable snapshot hash
        let bytes = std::fs::read(&job.frozen.snapshot)?;
        let outgoing = if job.prepared {
            Vec::new()
        } else {
            lock(&self.docs)?.prepare_sync_bytes(&job.session, &bytes)?
        };
        self.deliver(&job.session, outgoing);
        job.prepared = true;
        self.persist_job(job)?;
        node_git::prepare_commit(&job.frozen.repository, &job.frozen)
    }

    fn finish_sync(&self, id: &str, commit: Option<&str>) -> Result<Value> {
        let mut jobs = lock(&self.jobs)?;
        let job = jobs
            .get_mut(id)
            .ok_or_else(|| Error::new("unknown sync job"))?;
        if job.state == "done" {
            return Ok(json!({"job":job_view(job)}));
        }
        if job.state != "in_flight" {
            return Err(Error::new("sync is not running"));
        }
        let baseline = match commit {
            Some(commit) => {
                node_git::verify_completed(&job.frozen.repository, &job.frozen, commit)?
            }
            None => {
                node_git::find_completed(&job.frozen.repository, &job.frozen)?.ok_or_else(|| {
                    Error::new("no verified commit contains this job's frozen design and trailer")
                })?
            }
        };
        let outgoing = lock(&self.docs)?.finish_sync(&job.session)?;
        self.deliver(&job.session, outgoing);
        job.state = "done".into();
        job.commit = Some(baseline.commit.clone());
        job.prepared = false;
        self.persist_job(job)?;
        let result = json!({"job":job_view(job),"last":baseline.last_json()});
        self.broadcast(json!({"version":PROTOCOL,"event":"job","job":job_view(job),"last":baseline.last_json()}));
        Ok(result)
    }

    fn cancel_sync(&self, id: &str, picks: &[crate::node_documents::MergePick]) -> Result<Value> {
        let mut jobs = lock(&self.jobs)?;
        let job = jobs
            .get_mut(id)
            .ok_or_else(|| Error::new("unknown sync job"))?;
        if job.state == "in_flight" || job.prepared {
            let outgoing = lock(&self.docs)?.finish_sync_with_picks(&job.session, picks)?;
            self.deliver(&job.session, outgoing);
            job.state = "failed".into();
            job.reason = Some("sync cancelled".into());
            job.prepared = false;
            self.persist_job(job)?;
        }
        Ok(json!({"job":job_view(job)}))
    }

    fn sync_status(&self, id: &str) -> Result<Value> {
        let job = lock(&self.jobs)?
            .get(id)
            .cloned()
            .ok_or_else(|| Error::new("unknown sync job"))?;
        if job.state == "in_flight"
            && node_git::find_completed(&job.frozen.repository, &job.frozen)?.is_some()
        {
            return self.finish_sync(id, None);
        }
        let baseline = node_git::baseline(&job.frozen.repository, job.frozen.page)?;
        Ok(
            json!({"job":job_view(&job),"last":baseline.map(|b|b.last_json()).unwrap_or_else(||json!({"exists":false}))}),
        )
    }

    fn persist_job(&self, job: &Job) -> Result<()> {
        let parent = job
            .frozen
            .snapshot
            .parent()
            .ok_or_else(|| Error::new("frozen snapshot lacks parent"))?;
        let path = parent.join(format!("{}.job.json", job.frozen.id));
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, serde_json::to_vec(job)?)?;
        std::fs::rename(temporary, path)?;
        Ok(())
    }

    fn restore_jobs(&self, session: &str, path: &Path) -> Result<()> {
        // Frozen metadata is stored under the worktree's own Git directory, never projects.json.
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["rev-parse", "--absolute-git-dir"])
            .output()?;
        if !output.status.success() {
            return Ok(());
        }
        let root =
            PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()).join("mindmap-sync");
        fn visit(path: &Path, out: &mut Vec<PathBuf>, depth: usize) {
            if depth == 0 {
                return;
            }
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_dir() {
                        visit(&p, out, depth - 1)
                    } else if p
                        .file_name()
                        .and_then(|s| s.to_str())
                        .is_some_and(|s| s.ends_with(".job.json"))
                    {
                        out.push(p)
                    }
                }
            }
        }
        let mut paths = Vec::new();
        visit(&root, &mut paths, 4);
        let mut jobs = lock(&self.jobs)?;
        for p in paths {
            if let Ok(bytes) = std::fs::read(p) {
                if let Ok(mut job) = serde_json::from_slice::<Job>(&bytes) {
                    // Session IDs encode canonical worktree AND branch. Never rebind an inactive
                    // branch's job to the branch currently checked out after a restart.
                    if job.frozen.repository == path && job.session == session {
                        job.prepared = lock(&self.docs)?.metadata(session)?.sync_prepared;
                        jobs.entry(job.frozen.id.clone()).or_insert(job);
                    }
                }
            }
        }
        Ok(())
    }

    fn merge(&self, req: &Value, apply: bool) -> Result<Value> {
        let session = self.session(req)?;
        let metadata = lock(&self.docs)?.metadata(&session)?;
        let incoming = required(req, "incoming")?;
        let plan = node_git::plan_merge(&metadata.worktree, "HEAD", incoming)?;
        if !apply {
            return Ok(plan.json());
        }
        self.no_active_sync(&session)?;
        let picks: BTreeMap<String, figcraft_collab::merge::Side> =
            serde_json::from_value(req.get("picks").cloned().unwrap_or_else(|| json!({})))?;
        let document = node_git::resolve_merge(&plan, &picks)?;
        let base = node_git::load_design(&metadata.worktree, &plan.branch)?;
        let pending = node_git::pending_merge(&metadata.worktree)?;
        let state = if metadata.sync_prepared {
            if pending != vec![plan.main.clone()] {
                return Err(Error::new(
                    "prepared design does not belong to this Git merge",
                ));
            }
            json!({"state":"conflicts"})
        } else {
            let bytes = node_git::load_design_bytes(&metadata.worktree, &plan.branch)?;
            let outgoing = lock(&self.docs)?.prepare_sync_bytes(&session, &bytes)?;
            self.deliver(&session, outgoing);
            match node_git::begin_merge(&metadata.worktree, &plan.main) {
                Ok(value) => value,
                Err(error) => {
                    // A failed merge may still have left Git conflict state for inspection.
                    if node_git::pending_merge(&metadata.worktree)?.is_empty() {
                        let outgoing = lock(&self.docs)?.abort_prepared(&session)?;
                        self.deliver(&session, outgoing);
                    }
                    return Err(error);
                }
            }
        };
        if state["state"] == "up_to_date" {
            let outgoing = lock(&self.docs)?.abort_prepared(&session)?;
            self.deliver(&session, outgoing);
            return Ok(state);
        }
        // Git branch conflicts and conflicts with uncommitted live edits are separate plans.
        let picks: Vec<crate::node_documents::MergePick> =
            serde_json::from_value(req.get("live_picks").cloned().unwrap_or_else(|| json!([])))?;
        let outcome = lock(&self.docs)?.prepare_merge(&session, &base, &document, &picks)?;
        self.deliver(&session, outcome.outgoing);
        if !outcome.conflicts.is_empty() {
            return Ok(
                json!({"session":session,"state":"design_conflicts","conflicts":outcome.conflicts,"committed":false}),
            );
        }
        node_git::stage_design(&metadata.worktree)?;
        match node_git::commit_merge(
            &metadata.worktree,
            "Merge branch design and code through Mindmap",
        ) {
            Ok(commit) => {
                let outgoing = lock(&self.docs)?.finish_sync(&session)?;
                self.deliver(&session, outgoing);
                Ok(json!({"session":session,"state":"done","committed":true,"commit":commit}))
            }
            Err(error) => Ok(
                json!({"session":session,"state":"pending_commit","committed":false,"reason":error.to_string(),"git":state}),
            ),
        }
    }

    fn no_active_sync(&self, session: &str) -> Result<()> {
        if lock(&self.jobs)?
            .values()
            .any(|j| j.session == session && j.state == "in_flight")
        {
            return Err(Error::new(
                "finish or cancel the active Sync before merging",
            ));
        }
        Ok(())
    }

    fn finish_merge(&self, req: &Value) -> Result<Value> {
        let session = self.session(req)?;
        self.no_active_sync(&session)?;
        let metadata = lock(&self.docs)?.metadata(&session)?;
        if !metadata.sync_prepared || !metadata.merge_ready {
            return Err(Error::new(
                "resolve every design conflict before finishing the merge",
            ));
        }
        let frozen = lock(&self.docs)?.prepared_bytes(&session)?;
        node_git::stage_bytes(&metadata.worktree, &frozen)?;
        let commit = node_git::commit_merge(
            &metadata.worktree,
            req["message"]
                .as_str()
                .unwrap_or("Merge branch design and code through Mindmap"),
        )?;
        let outgoing = lock(&self.docs)?.finish_sync(&session)?;
        self.deliver(&session, outgoing);
        Ok(json!({"session":session,"state":"done","committed":true,"commit":commit}))
    }

    fn abort_merge(&self, req: &Value) -> Result<Value> {
        let session = self.session(req)?;
        self.no_active_sync(&session)?;
        let metadata = lock(&self.docs)?.metadata(&session)?;
        node_git::abort_merge(&metadata.worktree)?;
        let outgoing = lock(&self.docs)?.abort_prepared(&session)?;
        self.deliver(&session, outgoing);
        Ok(json!({"session":session,"state":"aborted"}))
    }

    fn maintain(&self) -> Result<()> {
        let _order = lock(&self.document_order)?;
        let updates = lock(&self.docs)?.tick();
        for update in updates {
            self.deliver(&update.session, update.outgoing);
            let event = json!({"version":PROTOCOL,"event":"session","session":update.session,"metadata":update.metadata,"error":update.error});
            let changed = lock(&self.session_events)?
                .insert(update.session, event.clone())
                .as_ref()
                != Some(&event);
            if changed {
                self.broadcast(event);
            }
        }
        // Herdr's event stream supplies status changes. Reconnected streams also emit a fresh
        // snapshot, so a sleeping client never determines a job's lifetime.
        let events = lock(&self.monitor)?.events.try_iter().collect::<Vec<_>>();
        for event in events {
            self.broadcast(json!({"version":PROTOCOL,"event":"herdr","data":event}));
            // A completed commit wins over a subsequent agent exit. Verify before classifying
            // lifecycle failures, including a reconnect snapshot with a missing agent.
            let ids = lock(&self.jobs)?
                .values()
                .filter(|j| j.state == "in_flight")
                .map(|j| j.frozen.id.clone())
                .collect::<Vec<_>>();
            for id in ids {
                let _ = self.sync_status(&id);
            }
            let statuses = match &event {
                HerdrEvent::AgentStatus { agent } => vec![agent.clone()],
                HerdrEvent::Resync { agents } => agents.clone(),
                _ => Vec::new(),
            };
            if !statuses.is_empty() || matches!(event, HerdrEvent::Resync { .. }) {
                let mut jobs = lock(&self.jobs)?;
                for job in jobs.values_mut().filter(|j| j.state == "in_flight") {
                    let pane = job.agent["pane"].as_str().unwrap_or("");
                    if let Some(status) = statuses
                        .iter()
                        .find(|s| s.pane_id == pane && s.workspace_id == job.workspace)
                    {
                        if matches!(status.status.as_str(), "working" | "blocked") {
                            job.seen_active = true;
                        }
                        if job.seen_active && matches!(status.status.as_str(), "idle" | "done") {
                            job.reason = Some(
                                "Agent stopped; waiting for a verified code-and-design commit"
                                    .into(),
                            );
                        }
                        if matches!(
                            status.status.as_str(),
                            "error" | "failed" | "crashed" | "exited" | "stopped"
                        ) {
                            job.reason = Some(format!("agent {}", status.status));
                            match lock(&self.docs)?.finish_sync(&job.session) {
                                Ok(outgoing)=>{self.deliver(&job.session,outgoing);job.prepared=false;job.state="failed".into();},
                                Err(error)=>job.reason=Some(format!("Agent ended; resolve design recovery and cancel this job: {error}")),
                            }
                        }
                        self.persist_job(job)?;
                    } else if matches!(event, HerdrEvent::Resync { .. }) && !pane.is_empty() {
                        job.reason =
                            Some("agent no longer exists in the shared Herdr session".into());
                        match lock(&self.docs)?.finish_sync(&job.session) {
                            Ok(outgoing) => {
                                self.deliver(&job.session, outgoing);
                                job.prepared = false;
                                job.state = "failed".into();
                            }
                            Err(error) => {
                                job.reason = Some(format!(
                                "Agent ended; resolve design recovery and cancel this job: {error}"
                            ))
                            }
                        }
                        self.persist_job(job)?;
                    }
                }
            }
            let ids = lock(&self.jobs)?
                .values()
                .filter(|j| j.state == "in_flight")
                .map(|j| j.frozen.id.clone())
                .collect::<Vec<_>>();
            for id in ids {
                let _ = self.sync_status(&id);
            }
        }
        Ok(())
    }

    fn validate_workspace(
        &self,
        workspace: &crate::workspace::WorkspaceView,
        session: &str,
    ) -> Result<()> {
        let metadata = lock(&self.docs)?.metadata(session)?;
        if !metadata.active {
            return Err(Error::new(
                "branch changed; reopen the worktree before prompting an agent",
            ));
        }
        let path = Path::new(
            workspace
                .path
                .as_deref()
                .ok_or_else(|| Error::new("workspace has no checkout path"))?,
        )
        .canonicalize()?;
        if path != metadata.worktree {
            return Err(Error::new(
                "agent workspace and design session belong to different worktrees",
            ));
        }
        Ok(())
    }
}

fn required<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| Error::new(format!("{key} needs a nonempty string")))
}
fn page(value: &Value) -> Result<Option<u64>> {
    match value.get("page") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .filter(|n| *n > 0)
            .map(Some)
            .ok_or_else(|| Error::new("page must be a positive id")),
    }
}
fn job_view(job: &Job) -> Value {
    json!({"id":job.frozen.id,"state":job.state,"started":job.frozen.started.to_string(),"agent":job.agent,"reason":job.reason,"commit":job.commit,"prepared":job.prepared})
}
