//! Worktree/branch document authority for the node's multiplexed transport.
//!
//! Callers serialize access to this manager and deliver every returned `Outgoing` using the
//! connection's joined peer id. Git and agents never write the live design: they use these
//! methods. `tick` polls disk and debounces saves; inactive branches are retained in Git's
//! worktree-local cache and cannot write the currently checked-out branch's file.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use figcraft_collab::merge::{self, Side};
use figcraft_collab::{
    Change, ClientMsg, Hello, Limits, Outgoing, Presence, Resume, Server, ServerMsg, SessionState,
};
use figcraft_doc::{Document, NodeId};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{Error, Result};

pub const DESIGN_PATH: &str = ".mindmap/map.figcraft";
const SAVE_DELAY: Duration = Duration::from_millis(500);
const CACHE_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMetadata {
    pub id: String,
    pub worktree: PathBuf,
    pub branch: String,
    pub seq: u64,
    pub dirty: bool,
    pub active: bool,
    pub sync_prepared: bool,
    pub merge_ready: bool,
}

/// In a merge, `branch` is the live authority; `main` is the incoming disk/Git version.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergePick {
    pub key: String,
    pub side: Side,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeConflict {
    pub key: String,
    pub branch: Vec<Change>,
    pub main: Vec<Change>,
}

#[derive(Clone, Debug)]
pub struct MergeOutcome {
    pub outgoing: Vec<Outgoing>,
    pub conflicts: Vec<MergeConflict>,
}

#[derive(Clone, Debug)]
pub struct SessionUpdate {
    pub session: String,
    pub outgoing: Vec<Outgoing>,
    pub error: Option<String>,
    pub metadata: SessionMetadata,
}

#[derive(Clone)]
struct Connected {
    hello: Hello,
    presence: Option<Presence>,
    ack: Option<u64>,
}

#[derive(Clone)]
struct LiveSession {
    id: String,
    worktree: PathBuf,
    branch: String,
    cache: PathBuf,
    server: Server,
    peers: BTreeMap<u32, Connected>,
    spotlight: Option<u32>,
    /// Exact bytes last read or saved. Also the three-way merge base for outside changes.
    disk: Option<Vec<u8>>,
    /// The missing-file baseline is stable, including its initial page id.
    initial: Document,
    dirty: bool,
    edited: Instant,
    /// Durable save/reload gate. Contains the precise design being committed.
    frozen: Option<Vec<u8>>,
    merge_ready: bool,
}

#[derive(Serialize, Deserialize)]
struct Cache {
    version: u32,
    worktree: PathBuf,
    branch: String,
    authority: SessionState,
    disk: Option<Vec<u8>>,
    initial: Vec<u8>,
    dirty: bool,
    frozen: Option<Vec<u8>>,
    #[serde(default)]
    merge_ready: bool,
}

#[derive(Default)]
pub struct NodeDocuments {
    sessions: BTreeMap<String, LiveSession>,
}

impl NodeDocuments {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&mut self, path: impl AsRef<Path>) -> Result<SessionMetadata> {
        let (worktree, branch, cache_dir) = identity(path.as_ref())?;
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        worktree.hash(&mut hash);
        branch.hash(&mut hash);
        let id = format!("{:016x}", hash.finish());
        if let Some(s) = self.sessions.get(&id) {
            if s.worktree != worktree || s.branch != branch {
                return Err(Error::new("document session identity collision"));
            }
            return Ok(s.metadata());
        }
        let cache = cache_dir.join(format!("{id}.json"));
        let mut s = match read_optional(&cache)? {
            Some(bytes) => {
                let saved: Cache = serde_json::from_slice(&bytes)?;
                if saved.version != CACHE_VERSION
                    || saved.worktree != worktree
                    || saved.branch != branch
                {
                    return Err(Error::new("document cache identity/version mismatch"));
                }
                LiveSession {
                    id: id.clone(),
                    worktree,
                    branch,
                    cache,
                    server: Server::restore(saved.authority, "", Limits::default())
                        .map_err(collab_error)?,
                    peers: BTreeMap::new(),
                    spotlight: None,
                    disk: saved.disk,
                    initial: decode(&saved.initial)?,
                    dirty: saved.dirty,
                    edited: Instant::now(),
                    frozen: saved.frozen,
                    merge_ready: saved.merge_ready,
                }
            }
            None => {
                let disk = read_optional(&worktree.join(DESIGN_PATH))?;
                let doc = match &disk {
                    Some(bytes) => decode(bytes)?,
                    None => Document::new("Mindmap"),
                };
                LiveSession {
                    id: id.clone(),
                    worktree,
                    branch,
                    cache,
                    server: Server::new(doc.clone(), "", Limits::default()),
                    peers: BTreeMap::new(),
                    spotlight: None,
                    dirty: disk.is_none(),
                    disk,
                    initial: doc,
                    edited: Instant::now(),
                    frozen: None,
                    merge_ready: false,
                }
            }
        };
        // Conflicts are surfaced by tick/resolve_disk. Opening must still permit inspection of
        // the preserved live document and recovery of an interrupted Sync.
        if s.frozen.is_none() {
            match s.refresh(&[]) {
                Ok(_) => {}
                Err(_) => { /* Keep the cache intact; tick reports the exact error. */ }
            }
        }
        s.persist()?;
        let metadata = s.metadata();
        self.sessions.insert(id, s);
        Ok(metadata)
    }

    pub fn metadata(&self, session: &str) -> Result<SessionMetadata> {
        Ok(self.get(session)?.metadata())
    }

    pub fn sessions(&self) -> Vec<SessionMetadata> {
        self.sessions.values().map(LiveSession::metadata).collect()
    }

    pub fn document(&self, session: &str) -> Result<Document> {
        Ok(self.get(session)?.server.document().clone())
    }

    pub fn inspect(&self, session: &str, node: Option<NodeId>, depth: usize) -> Result<Value> {
        let doc = self.get(session)?.server.document();
        let depth = depth.min(128);
        match node {
            Some(id) if !doc.contains(id) => Err(Error::new(format!("unknown node {id}"))),
            Some(id) => Ok(figcraft_engine::inspect::node_json(doc, id, depth)),
            None => Ok(json!({"name": doc.name, "pages": doc.pages.iter().map(|id|
                figcraft_engine::inspect::node_json(doc, *id, depth)).collect::<Vec<_>>()})),
        }
    }

    pub fn join(&mut self, session: &str, mut hello: Hello) -> Result<(u32, Vec<Outgoing>)> {
        let mut s = self.get(session)?.clone();
        // Node sessions have no owner link or secret. Never retain caller-supplied tokens.
        if !hello.token.is_empty() {
            return Err(Error::new("node sessions do not use tokens"));
        }
        hello.file = session.to_string();
        let (peer, mut outgoing) = s.server.join(hello.clone()).map_err(refused)?;
        // Core Welcome lists peers; also hydrate cursors already present before this join.
        for (id, connected) in &s.peers {
            if *id != peer {
                if let Some(presence) = &connected.presence {
                    outgoing.push(Outgoing::To(
                        peer,
                        ServerMsg::Presence {
                            client_id: *id,
                            presence: presence.clone(),
                        },
                    ));
                }
            }
        }
        s.peers.insert(
            peer,
            Connected {
                hello,
                presence: None,
                ack: None,
            },
        );
        self.commit(s)?;
        Ok((peer, outgoing))
    }

    pub fn receive(&mut self, session: &str, peer: u32, msg: ClientMsg) -> Result<Vec<Outgoing>> {
        // Cursors and acknowledgements are ephemeral. Do not serialize the document cache
        // (or copy its saved ZIP) for every pointer movement.
        if !matches!(&msg, ClientMsg::Changes { .. }) {
            let s = self
                .sessions
                .get_mut(session)
                .ok_or_else(|| unknown(session))?;
            s.track_presence(peer, &msg);
            return Ok(s.server.receive(peer, msg));
        }
        let mut s = self.get(session)?.clone();
        let before = s.server.seq();
        let outgoing = s.server.receive(peer, msg);
        if s.server.seq() != before {
            s.changed();
        }
        // Persist issued/processed batch ids even for rejections, so reconnect remains correct.
        self.commit(s)?;
        Ok(outgoing)
    }

    pub fn leave(&mut self, session: &str, peer: u32) -> Result<Vec<Outgoing>> {
        let s = self
            .sessions
            .get_mut(session)
            .ok_or_else(|| unknown(session))?;
        s.peers.remove(&peer);
        if s.spotlight == Some(peer) {
            s.spotlight = None;
        }
        Ok(s.server.leave(peer))
    }

    /// Each RPC executes on an isolated engine copy of the latest authority. Use command.batch
    /// for selection/clipboard state across steps. The resulting property diff is one batch.
    pub fn execute(
        &mut self,
        session: &str,
        command: &str,
        params: &Value,
    ) -> Result<(Value, Vec<Outgoing>)> {
        validate_command(command, params, 0)?;
        let mut s = self.get(session)?.clone();
        let author = s.internal_peer()?;
        let mut doc = s.server.document().clone();
        doc.set_client_id(author);
        let mut engine = figcraft_engine::Session::with_document(doc);
        let result = engine.execute(command, params).map_err(collab_error)?;
        let changes =
            figcraft_collab::diff(s.server.replica(), engine.doc().map_err(collab_error)?)
                .map_err(collab_error)?
                .changes;
        let outgoing = s.submit(author, changes)?;
        s.server.leave(author);
        self.commit(s)?;
        Ok((result, outgoing))
    }

    /// Trusted full-document import. Retains IDs, session identity, peers and ordering; clients
    /// receive a normal Changes batch. Do not expose this as a client-authored Changes bypass.
    pub fn replace(&mut self, session: &str, document: Document) -> Result<Vec<Outgoing>> {
        let mut s = self.get(session)?.clone();
        let outgoing = s.apply_document(&document)?;
        self.commit(s)?;
        Ok(outgoing)
    }

    /// Resolve the current disk change against the last observed disk baseline. Unresolved
    /// conflicts leave both disk and live authority untouched, and prevent autosaving.
    pub fn resolve_disk(&mut self, session: &str, picks: &[MergePick]) -> Result<MergeOutcome> {
        let mut s = self.get(session)?.clone();
        s.ensure_active()?;
        if s.frozen.is_some() {
            return Err(Error::new("Sync has frozen disk reloads"));
        }
        let outcome = s.refresh(picks)?;
        if outcome.conflicts.is_empty() {
            self.commit(s)?;
        }
        Ok(outcome)
    }

    /// Persist the live cache and gate FIRST, then write the exact frozen snapshot. Edits keep
    /// flowing through the authority while the gate blocks autosave/reload (including restart).
    /// The caller keeps this gate through Git commit verification; finish also serves abort.
    /// The bytes are Git's frozen blob: the worktree must match it exactly, including ZIP metadata.
    pub fn prepare_sync_bytes(&mut self, session: &str, bytes: &[u8]) -> Result<Vec<Outgoing>> {
        decode(bytes)?;
        let mut s = self.get(session)?.clone();
        s.ensure_active()?;
        if s.frozen.is_some() {
            return Err(Error::new("a Sync is already prepared for this session"));
        }
        if read_optional(&s.worktree.join(DESIGN_PATH))? != s.disk {
            return Err(Error::new(
                "design changed on disk before Sync; resolve_disk and retry",
            ));
        }
        s.frozen = Some(bytes.to_vec());
        s.merge_ready = false;
        self.commit(s)?;
        let s = self.get(session)?;
        // If writing fails the durable gate remains, so callers can safely finish/abort it.
        s.write_design(bytes)?;
        Ok(Vec::new())
    }

    /// The immutable prepared blob. Use this for staging, never the mutable worktree file.
    pub fn prepared_bytes(&self, session: &str) -> Result<Vec<u8>> {
        self.get(session)?
            .frozen
            .clone()
            .ok_or_else(|| Error::new("no prepared document snapshot"))
    }

    pub fn finish_sync(&mut self, session: &str) -> Result<Vec<Outgoing>> {
        self.finish_sync_with_picks(session, &[])
    }

    /// Explicit recovery if an outside writer changed the frozen file during a Sync.
    pub fn finish_sync_with_picks(
        &mut self,
        session: &str,
        picks: &[MergePick],
    ) -> Result<Vec<Outgoing>> {
        let mut s = self.get(session)?.clone();
        s.ensure_active()?;
        let Some(frozen) = s.frozen.clone() else {
            return self.save(session);
        };
        let actual = read_optional(&s.worktree.join(DESIGN_PATH))?;
        let mut outgoing = Vec::new();
        // A failed prepare may not have written its frozen snapshot. Preserve that original
        // baseline too; otherwise an abort would mistake old disk for new edits.
        if actual != Some(frozen.clone()) && actual != s.disk {
            let incoming = actual.as_ref().ok_or_else(|| {
                Error::new("design removed during Sync; restore it before finishing")
            })?;
            let outcome = s.merge_document(&decode(&frozen)?, &decode(incoming)?, picks)?;
            require_resolved(&outcome)?;
            outgoing.extend(outcome.outgoing);
        }
        let bytes = encode(s.server.document())?;
        s.write_design(&bytes)?;
        s.disk = Some(bytes);
        s.frozen = None;
        s.merge_ready = false;
        s.dirty = false;
        self.commit(s)?;
        Ok(outgoing)
    }

    /// Git may temporarily leave a binary conflict in map.figcraft. While its gate is held,
    /// merge the explicit committed documents with live edits, never that transient disk file.
    /// `base` is HEAD before Git merge; `incoming` is the resolved committed design merge.
    /// The accepted result becomes the frozen commit target; later live edits remain drift.
    pub fn prepare_merge(
        &mut self,
        session: &str,
        base: &Document,
        incoming: &Document,
        picks: &[MergePick],
    ) -> Result<MergeOutcome> {
        let mut s = self.get(session)?.clone();
        s.ensure_active()?;
        if s.frozen.is_none() {
            return Err(Error::new(
                "prepare_merge requires a prepared save/reload gate",
            ));
        }
        let outcome = s.merge_document(base, incoming, picks)?;
        if !outcome.conflicts.is_empty() {
            s.merge_ready = false;
            self.commit(s)?;
            return Ok(outcome);
        }
        let bytes = encode(s.server.document())?;
        // The original gate is already durable. On a write/cache failure it still protects
        // the old authority, and finish_sync can merge the new disk against its original base.
        s.write_design(&bytes)?;
        s.frozen = Some(bytes);
        s.merge_ready = true;
        self.commit(s)?;
        Ok(outcome)
    }

    /// Controlled cancellation after Git restored HEAD. That restore is not an outside edit:
    /// do not merge it against the prepared merge target and thereby undo accepted live work.
    /// The transport must only call this for an explicit abort/cancel, not commit success.
    pub fn abort_prepared(&mut self, session: &str) -> Result<Vec<Outgoing>> {
        let mut s = self.get(session)?.clone();
        s.ensure_active()?;
        if s.frozen.is_none() {
            return Err(Error::new("no prepared save/reload gate to abort"));
        }
        let bytes = encode(s.server.document())?;
        s.write_design(&bytes)?;
        s.disk = Some(bytes);
        s.frozen = None;
        s.merge_ready = false;
        s.dirty = false;
        self.commit(s)?;
        Ok(Vec::new())
    }

    pub fn save(&mut self, session: &str) -> Result<Vec<Outgoing>> {
        let mut s = self.get(session)?.clone();
        s.ensure_active()?;
        if s.frozen.is_some() {
            return Err(Error::new(
                "Sync has frozen saves until commit verification",
            ));
        }
        let outcome = s.refresh(&[])?;
        require_resolved(&outcome)?;
        s.save_live()?;
        self.commit(s)?;
        Ok(outcome.outgoing)
    }

    /// Run regularly (e.g. every 100 ms). Errors/conflicts are session-local: a damaged file or
    /// inactive branch must not prevent another worktree's saves or broadcasts.
    pub fn tick(&mut self) -> Vec<SessionUpdate> {
        let ids: Vec<_> = self.sessions.keys().cloned().collect();
        let mut updates = Vec::new();
        for id in ids {
            let result = (|| -> Result<Vec<Outgoing>> {
                let mut s = self.get(&id)?.clone();
                if !s.active() || s.frozen.is_some() {
                    return Ok(Vec::new());
                }
                let old_disk = s.disk.clone();
                let outcome = s.refresh(&[])?;
                require_resolved(&outcome)?;
                let save = s.dirty && s.edited.elapsed() >= SAVE_DELAY;
                if save {
                    s.save_live()?;
                }
                if save || s.disk != old_disk || !outcome.outgoing.is_empty() {
                    self.commit(s)?;
                }
                Ok(outcome.outgoing)
            })();
            if let Some(s) = self.sessions.get(&id) {
                let (outgoing, error) = match result {
                    Ok(outgoing) => (outgoing, None),
                    Err(error) => (Vec::new(), Some(error.to_string())),
                };
                updates.push(SessionUpdate {
                    session: id,
                    outgoing,
                    error,
                    metadata: s.metadata(),
                });
            }
        }
        updates
    }

    fn get(&self, id: &str) -> Result<&LiveSession> {
        self.sessions.get(id).ok_or_else(|| unknown(id))
    }

    fn commit(&mut self, s: LiveSession) -> Result<()> {
        s.persist()?;
        self.sessions.insert(s.id.clone(), s);
        Ok(())
    }
}

impl LiveSession {
    fn track_presence(&mut self, peer: u32, msg: &ClientMsg) {
        if let Some(connected) = self.peers.get_mut(&peer) {
            match msg {
                ClientMsg::Presence(p) => connected.presence = Some(p.clone()),
                ClientMsg::Ack { seq } => connected.ack = Some(*seq),
                ClientMsg::Signal(signal) => {
                    if let Some(signal) = signal.clone().clean() {
                        if signal.kind == figcraft_collab::Signal::SPOTLIGHT {
                            if signal.on() {
                                self.spotlight = Some(peer);
                            } else if self.spotlight == Some(peer) {
                                self.spotlight = None;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn metadata(&self) -> SessionMetadata {
        SessionMetadata {
            id: self.id.clone(),
            worktree: self.worktree.clone(),
            branch: self.branch.clone(),
            seq: self.server.seq(),
            dirty: self.dirty,
            active: self.active(),
            sync_prepared: self.frozen.is_some(),
            merge_ready: self.merge_ready,
        }
    }

    fn active(&self) -> bool {
        current_branch(&self.worktree).is_ok_and(|b| b == self.branch)
    }

    fn ensure_active(&self) -> Result<()> {
        if self.active() {
            Ok(())
        } else {
            Err(Error::new(
                "worktree changed branch; open its current branch session before writing",
            ))
        }
    }

    fn changed(&mut self) {
        self.dirty = true;
        self.edited = Instant::now();
    }

    fn persist(&self) -> Result<()> {
        let saved = Cache {
            version: CACHE_VERSION,
            worktree: self.worktree.clone(),
            branch: self.branch.clone(),
            authority: self.server.state().map_err(collab_error)?,
            disk: self.disk.clone(),
            initial: encode(&self.initial)?,
            dirty: self.dirty,
            frozen: self.frozen.clone(),
            merge_ready: self.merge_ready,
        };
        if let Some(parent) = self.cache.parent() {
            std::fs::create_dir_all(parent)?;
        }
        atomic_write(&self.cache, &serde_json::to_vec(&saved)?)
    }

    fn write_design(&self, bytes: &[u8]) -> Result<()> {
        let path = self.worktree.join(DESIGN_PATH);
        self.ensure_active()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        self.ensure_active()?;
        atomic_write(&path, bytes)
    }

    fn save_live(&mut self) -> Result<()> {
        let bytes = encode(self.server.document())?;
        // Refuse to clobber an outside writer that raced with refresh/serialization.
        if read_optional(&self.worktree.join(DESIGN_PATH))? != self.disk {
            return Err(Error::new(
                "design changed on disk during save; retry to merge it",
            ));
        }
        self.write_design(&bytes)?;
        self.disk = Some(bytes);
        self.dirty = false;
        Ok(())
    }

    fn refresh(&mut self, picks: &[MergePick]) -> Result<MergeOutcome> {
        if self.frozen.is_some() {
            return Ok(empty_outcome());
        }
        self.ensure_active()?;
        let actual = read_optional(&self.worktree.join(DESIGN_PATH))?;
        if actual == self.disk {
            return Ok(empty_outcome());
        }
        let incoming = actual.as_ref().ok_or_else(|| {
            Error::new("design was removed on disk; refusing to overwrite live edits")
        })?;
        let base = match &self.disk {
            Some(bytes) => decode(bytes)?,
            None => self.initial.clone(),
        };
        let incoming_doc = decode(incoming)?;
        let outcome = self.merge_document(&base, &incoming_doc, picks)?;
        if outcome.conflicts.is_empty() {
            self.disk = actual;
            self.dirty = !figcraft_collab::diff(
                &figcraft_collab::Replica::new(incoming_doc),
                self.server.document(),
            )
            .map_err(collab_error)?
            .changes
            .is_empty();
        }
        Ok(outcome)
    }

    fn merge_document(
        &mut self,
        base: &Document,
        incoming: &Document,
        picks: &[MergePick],
    ) -> Result<MergeOutcome> {
        let plan = merge::plan(base, self.server.document(), incoming).map_err(collab_error)?;
        let mut choices = BTreeMap::new();
        for pick in picks {
            if !plan.conflicts.iter().any(|c| c.key.to_string() == pick.key) {
                return Err(Error::new(format!("unknown merge conflict {}", pick.key)));
            }
            if choices.insert(pick.key.clone(), pick.side).is_some() {
                return Err(Error::new(format!(
                    "duplicate merge resolution {}",
                    pick.key
                )));
            }
        }
        let choose = |key: &merge::Key| choices.get(&key.to_string()).copied();
        let conflicts = plan
            .unresolved(&choose)
            .iter()
            .map(|c| MergeConflict {
                key: c.key.to_string(),
                branch: c.branch.iter().map(|i| plan.branch[*i].clone()).collect(),
                main: c.main.iter().map(|i| plan.main[*i].clone()).collect(),
            })
            .collect::<Vec<_>>();
        if !conflicts.is_empty() {
            return Ok(MergeOutcome {
                outgoing: Vec::new(),
                conflicts,
            });
        }
        let (mut merged, _) = plan
            .merge(self.server.document(), Side::Main, &choose)
            .map_err(collab_error)?;
        figcraft_engine::normalize(&mut merged, self.server.document());
        Ok(MergeOutcome {
            outgoing: self.apply_document(&merged)?,
            conflicts: Vec::new(),
        })
    }

    fn internal_peer(&mut self) -> Result<u32> {
        self.server
            .join(Hello {
                protocol: figcraft_collab::message::PROTOCOL,
                file: self.id.clone(),
                token: String::new(),
                name: "Mindmap node".into(),
                color: "#6677cc".into(),
                client_version: "mindmap-node/1".into(),
                resume: None,
            })
            .map(|(id, _)| id)
            .map_err(refused)
    }

    fn submit(&mut self, author: u32, changes: Vec<Change>) -> Result<Vec<Outgoing>> {
        if changes.is_empty() {
            return Ok(Vec::new());
        }
        let outgoing = self.server.receive(
            author,
            ClientMsg::Changes {
                base_seq: self.server.seq(),
                batch_id: 1,
                changes,
            },
        );
        for out in &outgoing {
            if let Outgoing::To(
                _,
                ServerMsg::Reject { reason, .. } | ServerMsg::Refused { reason },
            ) = out
            {
                return Err(Error::new(reason.clone()));
            }
        }
        self.changed();
        Ok(outgoing)
    }

    fn apply_document(&mut self, document: &Document) -> Result<Vec<Outgoing>> {
        let changes = figcraft_collab::diff(self.server.replica(), document)
            .map_err(collab_error)?
            .changes;
        if changes.is_empty() {
            return Ok(Vec::new());
        }
        // Imported Git IDs may belong to earlier/other authorities. Reserve them as restorable
        // objects on a *candidate* core state, then let Server validate the complete diff as one
        // ordered batch. No remapping (Git merge bases and agent node references stay stable).
        // Restoring preserves the session id, log and sequence. Rehydrate connected peers below.
        let mut state = self.server.state().map_err(collab_error)?;
        for change in &changes {
            match change {
                Change::Create { id, .. } if !self.server.document().contains(*id) => {
                    state.deleted.insert(*id);
                }
                Change::Put { entry, .. } => {
                    state.removed.insert(entry.clone());
                }
                _ => {}
            }
        }
        state.next_client = state
            .next_client
            .max(document.highest_id_space().saturating_add(1));
        let session_id = state.session;
        self.server = Server::restore(state, "", Limits::default()).map_err(collab_error)?;
        for (id, connected) in &self.peers {
            let mut hello = connected.hello.clone();
            hello.resume = Some(Resume {
                session: session_id,
                client_id: *id,
                seq: self.server.seq(),
            });
            let (resumed, _) = self.server.join(hello).map_err(refused)?;
            if resumed != *id {
                return Err(Error::new("could not restore authority peer"));
            }
            if let Some(presence) = &connected.presence {
                self.server
                    .receive(*id, ClientMsg::Presence(presence.clone()));
            }
            if let Some(seq) = connected.ack {
                self.server.receive(*id, ClientMsg::Ack { seq });
            }
        }
        if let Some(peer) = self.spotlight {
            self.server.receive(
                peer,
                ClientMsg::Signal(figcraft_collab::Signal::spotlight(true)),
            );
        }
        let author = self.internal_peer()?;
        let outgoing = self.submit(author, changes)?;
        self.server.leave(author);
        Ok(outgoing)
    }
}

fn empty_outcome() -> MergeOutcome {
    MergeOutcome {
        outgoing: Vec::new(),
        conflicts: Vec::new(),
    }
}
fn unknown(id: &str) -> Error {
    Error::new(format!("unknown document session {id}"))
}
fn collab_error(error: impl std::fmt::Display) -> Error {
    Error::new(error.to_string())
}
fn refused(msg: ServerMsg) -> Error {
    match msg {
        ServerMsg::Refused { reason } => Error::new(reason),
        _ => Error::new("collaboration join refused"),
    }
}
fn encode(doc: &Document) -> Result<Vec<u8>> {
    figcraft_format::save_bytes(doc, None).map_err(collab_error)
}
fn decode(bytes: &[u8]) -> Result<Document> {
    figcraft_format::load_bytes(bytes)
        .map(|(doc, _)| doc)
        .map_err(collab_error)
}
fn require_resolved(outcome: &MergeOutcome) -> Result<()> {
    if outcome.conflicts.is_empty() {
        Ok(())
    } else {
        Err(Error::new(format!(
            "design merge conflicts: {}",
            serde_json::to_string(&outcome.conflicts)?
        )))
    }
}
fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Staging file + fsync + atomic rename from FigCraft; then fsync the containing directory.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    figcraft_format::write_file_atomic(path, bytes)?;
    std::fs::File::open(path)?.sync_all()?;
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn git(path: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .output()?;
    if !output.status.success() {
        // Git stderr can contain remote URLs; report the operation without credential-bearing output.
        return Err(Error::new(format!(
            "git {} failed in worktree",
            args.first().copied().unwrap_or("command")
        )));
    }
    Ok(String::from_utf8(output.stdout)
        .map_err(collab_error)?
        .trim_end_matches(['\r', '\n'])
        .to_string())
}
fn current_branch(path: &Path) -> Result<String> {
    match git(path, &["symbolic-ref", "--quiet", "--short", "HEAD"]) {
        Ok(branch) => Ok(branch),
        Err(_) => Ok(format!(
            "detached:{}",
            git(path, &["rev-parse", "--verify", "HEAD"])?
        )),
    }
}
fn identity(path: &Path) -> Result<(PathBuf, String, PathBuf)> {
    let path = path.canonicalize()?;
    let dir = if path.is_file() {
        path.parent()
            .ok_or_else(|| Error::new("missing worktree parent"))?
    } else {
        &path
    };
    let worktree = PathBuf::from(git(dir, &["rev-parse", "--show-toplevel"])?).canonicalize()?;
    let branch = current_branch(&worktree)?;
    let git_dir = PathBuf::from(git(&worktree, &["rev-parse", "--absolute-git-dir"])?);
    Ok((worktree, branch, git_dir.join("mindmap-node")))
}

/// Keep command execution in-memory, including nested batches. File/network/plugin commands
/// belong to other node APIs; an RPC must never let engine file.save bypass this authority.
fn validate_command(command: &str, params: &Value, depth: usize) -> Result<()> {
    if depth > 32 {
        return Err(Error::new("command batch nesting exceeds 32"));
    }
    if command == "command.batch" {
        let commands = params
            .get("commands")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::new("batch requires commands"))?;
        for step in commands {
            let id = step
                .get("command")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::new("batch step requires command"))?;
            validate_command(id, step.get("params").unwrap_or(&Value::Null), depth + 1)?;
        }
        return Ok(());
    }
    let family = command.split('.').next().unwrap_or("");
    let allowed = matches!(
        family,
        "node"
            | "text"
            | "shape"
            | "frame"
            | "page"
            | "select"
            | "object"
            | "layout"
            | "arrange"
            | "style"
            | "variable"
            | "component"
            | "instance"
            | "comment"
            | "vector"
            | "section"
            | "boolean"
            | "constraint"
            | "stroke"
            | "fill"
            | "effect"
            | "grid"
            | "guide"
            | "image"
    ) || matches!(
        command,
        "edit.delete"
            | "edit.duplicate"
            | "edit.copy"
            | "edit.cut"
            | "edit.paste"
            | "edit.copyProperties"
            | "edit.pasteProperties"
            | "edit.pasteHere"
    );
    if !allowed
        || matches!(
            command,
            "image.place" | "variable.import" | "variable.export" | "component.playground"
        )
        || command.ends_with(".export")
        || command.ends_with(".import")
    {
        return Err(Error::new(format!(
            "command {command} is not a node document command"
        )));
    }
    Ok(())
}

/// Render layers (or a page) of a copy of the live design as a PNG. Nothing is written to disk
/// and no session is edited. Returns FigCraft's export result (`data` in base64).
pub fn render(doc: Document, ids: &[u64], page: Option<u64>, scale: f64) -> Result<Value> {
    let mut engine = figcraft_engine::Session::with_document(doc);
    if let Some(page) = page {
        engine
            .execute("page.select", &json!({"id": page}))
            .map_err(collab_error)?;
    }
    let scale = if scale.is_finite() {
        scale.clamp(0.05, 4.0)
    } else {
        1.0
    };
    let mut params = json!({"format": "png", "scale": scale});
    if !ids.is_empty() {
        params["ids"] = json!(ids);
    }
    engine.execute("file.export", &params).map_err(collab_error)
}

#[cfg(test)]
#[path = "node_documents/tests.rs"]
mod tests;
