//! Git-backed design baselines, immutable sync inputs and branch merge planning.
//!
//! Transport and agent lifecycle belong to the caller. In particular, an idle agent is not
//! evidence of completion: only `verify_completed` / `find_completed` establish that. The caller
//! must gate document saves, install `frozen_document` in the worktree, then `prepare_commit`,
//! and keep that gate until verification (or explicit abort), restoring the latest live design.
//! Sync helpers never write the live design file or make commits on the agent's behalf. The
//! explicit merge helpers run Git's ordinary merge lifecycle behind the same document save gate.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use figcraft_collab::{merge, Change, Entry, Replica};
use figcraft_doc::{Document, NodeId};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{Error, Result};

pub const DESIGN_PATH: &str = ".mindmap/map.figcraft";
const TRAILER: &str = "Mindmap-Sync";
/// What a record says about the map it commits. `sync`: the code now implements it. `explain`:
/// it describes the code as it already is. `fix`: a bug fix, committed with its code.
pub const KINDS: [&str; 3] = ["sync", "explain", "fix"];
/// Request start copies older than this are removed when a new one is kept.
const REQUEST_TTL_SECS: u64 = 30 * 24 * 3600;
static NEXT_FREEZE: AtomicU64 = AtomicU64::new(0);

/// A reachable sync commit. `page: None` means an explicit `all` trailer, never an unknown scope.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Baseline {
    pub commit: String,
    pub at: String,
    pub page: Option<u64>,
    pub trailer: String,
    /// `sync`, `explain` or `fix` (see [`KINDS`]). Markers written before kinds existed are syncs.
    #[serde(default = "sync_kind")]
    pub kind: String,
}

fn sync_kind() -> String {
    "sync".to_owned()
}

impl Baseline {
    pub fn last_json(&self) -> Value {
        json!({"exists": true, "commit": self.commit, "at": self.at, "page": self.page, "kind": self.kind})
    }
}

/// The kind a marker records: its last word when that names one, else `sync`. The scope stays the
/// first word, so older readers keep finding the page.
pub fn marker_kind(marker: &str) -> &'static str {
    match marker.split_whitespace().last() {
        Some("explain") => "explain",
        Some("fix") => "fix",
        _ => "sync",
    }
}

/// Durable job input; serialize this record with the caller's job metadata. No live document is
/// held here. The read-only snapshot belongs to this worktree's private Git directory.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrozenSync {
    pub id: String,
    pub repository: PathBuf,
    pub snapshot: PathBuf,
    pub page: Option<u64>,
    pub start_head: Option<String>,
    pub start_ref: Option<String>,
    pub baseline: Option<Baseline>,
    pub blob: String,
    pub trailer: String,
    pub started: u64,
}

fn git_command(path: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(path).env("GIT_TERMINAL_PROMPT", "0");
    // The node's worktree argument owns the operation, not an agent's inherited Git environment.
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    ] {
        cmd.env_remove(key);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

fn git_output(path: &Path, args: &[&str]) -> Result<Output> {
    git_command(path)
        .args(args)
        .output()
        .map_err(|_| Error::new("could not execute git"))
}

fn git(path: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = git_output(path, args)?;
    if !out.status.success() {
        // Do not echo stderr, command arguments or repository configuration (possibly credentials).
        return Err(Error::new(format!(
            "git {} failed",
            args.first().unwrap_or(&"operation")
        )));
    }
    Ok(out.stdout)
}

fn git_text(path: &Path, args: &[&str]) -> Result<String> {
    String::from_utf8(git(path, args)?)
        .map(|s| s.trim().to_owned())
        .map_err(|_| Error::new("git returned non-UTF-8 metadata"))
}

fn git_input(path: &Path, args: &[&str], input: &[u8]) -> Result<String> {
    let mut child = git_command(path).args(args).stdin(Stdio::piped()).spawn()?;
    let write = match child.stdin.take() {
        Some(mut pipe) => pipe.write_all(input),
        None => return Err(Error::new("git input pipe unavailable")),
    };
    let out = child.wait_with_output()?;
    write?;
    if !out.status.success() {
        return Err(Error::new("git object operation failed"));
    }
    String::from_utf8(out.stdout)
        .map(|s| s.trim().to_owned())
        .map_err(|_| Error::new("git returned non-UTF-8 object id"))
}

fn repository(path: &Path) -> Result<PathBuf> {
    let root = git_text(path, &["rev-parse", "--show-toplevel"])?;
    Ok(fs::canonicalize(root)?)
}

fn head(path: &Path) -> Result<Option<String>> {
    let out = git_output(path, &["rev-parse", "--verify", "HEAD"])?;
    if out.status.success() {
        return Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_owned()));
    }
    // Only an unborn symbolic branch is a legitimate absent HEAD.
    let symbolic = git_output(path, &["symbolic-ref", "-q", "HEAD"])?;
    if symbolic.status.success() {
        let name = String::from_utf8_lossy(&symbolic.stdout).trim().to_owned();
        if git_output(path, &["show-ref", "--verify", "--quiet", &name])?
            .status
            .code()
            == Some(1)
        {
            return Ok(None);
        }
    }
    Err(Error::new("cannot resolve repository HEAD"))
}

fn branch_ref(path: &Path) -> Result<Option<String>> {
    let out = git_output(path, &["symbolic-ref", "-q", "HEAD"])?;
    if out.status.success() {
        Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_owned()))
    } else if out.status.code() == Some(1) {
        Ok(None)
    } else {
        Err(Error::new("cannot resolve repository branch"))
    }
}

fn resolve_commit(path: &Path, rev: &str) -> Result<String> {
    git_text(
        path,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{rev}^{{commit}}"),
        ],
    )
}

fn is_ancestor(path: &Path, older: &str, newer: &str) -> Result<bool> {
    match git_output(path, &["merge-base", "--is-ancestor", older, newer])?
        .status
        .code()
    {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(Error::new("cannot inspect commit ancestry")),
    }
}

/// Parse only explicit scopes. Legacy `123`, `page:123` and `page=123` are accepted, along with
/// `all`. Arbitrary trailer prose must never silently mean that every page was synchronized.
fn trailer_scope(value: &str) -> Option<Option<u64>> {
    let scope = value.split_whitespace().next()?;
    if scope == "all" {
        return Some(None);
    }
    let number = scope
        .strip_prefix("page=")
        .or_else(|| scope.strip_prefix("page:"))
        .unwrap_or(scope);
    number.parse::<u64>().ok().filter(|p| *p > 0).map(Some)
}

fn commit_markers(path: &Path, commit: &str) -> Result<(String, Vec<String>)> {
    // Git's trailer parser excludes lookalike lines in the commit body.
    let text = git_text(
        path,
        &[
            "show",
            "-s",
            "--format=%cI%n%(trailers:only,key=Mindmap-Sync,valueonly,unfold=true,separator=%x1f)",
            commit,
        ],
    )?;
    let (at, rest) = text.split_once('\n').unwrap_or((&text, ""));
    Ok((
        at.to_owned(),
        rest.split('\x1f')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .collect(),
    ))
}

fn design_blob(path: &Path, commit: &str) -> Result<Option<String>> {
    let out = git_text(path, &["ls-tree", commit, "--", DESIGN_PATH])?;
    if out.is_empty() {
        return Ok(None);
    }
    let fields: Vec<&str> = out.split_whitespace().collect();
    if fields.get(1) != Some(&"blob")
        || !matches!(fields.first(), Some(&"100644") | Some(&"100755"))
    {
        return Err(Error::new("committed design must be a regular Git blob"));
    }
    Ok(fields.get(2).map(|s| (*s).to_owned()))
}

/// Load the committed native file directly from Git, without checking it out or reading the
/// potentially newer live worktree file. Format repairs are rejected rather than hidden.
pub fn load_design(path: &Path, revision: &str) -> Result<Document> {
    decode_design(&load_design_bytes(path, revision)?)
}

/// Exact committed archive, including ZIP metadata/history. Use with the document save gate
/// before `begin_merge`, so Git sees HEAD's actual bytes rather than a re-encoded equivalent.
pub fn load_design_bytes(path: &Path, revision: &str) -> Result<Vec<u8>> {
    let commit = resolve_commit(path, revision)?;
    let blob = design_blob(path, &commit)?
        .ok_or_else(|| Error::new("commit has no .mindmap/map.figcraft"))?;
    git(path, &["cat-file", "blob", &blob])
}

fn decode_design(bytes: &[u8]) -> Result<Document> {
    let (doc, repairs) = figcraft_format::load_bytes(bytes)
        .map_err(|e| Error::new(format!("decode design: {e}")))?;
    if !repairs.is_empty() {
        return Err(Error::new(
            "design requires repairs; refusing an inexact baseline",
        ));
    }
    Ok(doc)
}

/// Last reachable, topologically ordered record (a Sync, an explanation or a fix) with a trailer
/// for exactly this page, or `all`. This is the design baseline: the map as last recorded.
/// A whole-map request only accepts `all`; a page-only commit cannot baseline unrelated pages.
pub fn baseline(path: &Path, page: Option<u64>) -> Result<Option<Baseline>> {
    find_baseline(path, page, false)
}

/// Last reachable Sync for this page (or `all`): the code baseline. Explanations commit no code
/// and fixes are not design work, so neither moves it.
pub fn sync_baseline(path: &Path, page: Option<u64>) -> Result<Option<Baseline>> {
    find_baseline(path, page, true)
}

fn find_baseline(path: &Path, page: Option<u64>, sync_only: bool) -> Result<Option<Baseline>> {
    validate_page(page)?;
    let root = repository(path)?;
    let Some(tip) = head(&root)? else {
        return Ok(None);
    };
    let candidates = git_text(
        &root,
        &[
            "log",
            "--topo-order",
            "--format=%H",
            "--regexp-ignore-case",
            "--grep=^Mindmap-Sync:",
            &tip,
        ],
    )?;
    for commit in candidates.lines() {
        let (at, markers) = commit_markers(&root, commit)?;
        for marker in markers {
            let Some(scope) = trailer_scope(&marker) else {
                continue;
            };
            if scope.is_some() && scope != page {
                continue;
            }
            let kind = marker_kind(&marker);
            if sync_only && kind != "sync" {
                continue;
            }
            if design_blob(&root, commit)?.is_none() {
                continue;
            }
            return Ok(Some(Baseline {
                commit: commit.to_owned(),
                at,
                page: scope,
                trailer: format!("{TRAILER}: {marker}"),
                kind: kind.to_owned(),
            }));
        }
    }
    Ok(None)
}

fn validate_page(page: Option<u64>) -> Result<()> {
    if page == Some(0) {
        Err(Error::new("page id must be positive"))
    } else {
        Ok(())
    }
}

/// Include the selected page and its page-tree descendants; cycles cannot loop. Selection is
/// calculated in both versions, so deleted/moved pages and nodes remain visible in drift.
fn scope_nodes(doc: &Document, page: Option<u64>) -> BTreeSet<NodeId> {
    let Some(page) = page else {
        return doc.nodes().map(|n| n.id).collect();
    };
    let mut pages = BTreeSet::from([NodeId(page)]);
    loop {
        let before = pages.len();
        for id in &doc.pages {
            let parent = doc
                .node(*id)
                .and_then(|n| n.plugin_data.get("mindmap.parent_page"))
                .and_then(|s| s.parse::<u64>().ok());
            if parent.is_some_and(|p| pages.contains(&NodeId(p))) {
                pages.insert(*id);
            }
        }
        if pages.len() == before {
            break;
        }
    }
    pages
        .into_iter()
        .flat_map(|p| {
            if doc.contains(p) {
                doc.subtree(p)
            } else {
                Vec::new()
            }
        })
        .collect()
}

fn scoped_change(
    change: &Change,
    old: &Document,
    live: &Document,
    ids: &BTreeSet<NodeId>,
    page: Option<u64>,
) -> bool {
    if page.is_none() {
        return true;
    }
    if let Some(id) = change.node() {
        return ids.contains(&id);
    }
    match change {
        Change::Put {
            entry: Entry::DevSnapshot(id),
            ..
        }
        | Change::Remove {
            entry: Entry::DevSnapshot(id),
        } => ids.contains(id),
        Change::Put {
            entry: Entry::Comment(id),
            ..
        }
        | Change::Remove {
            entry: Entry::Comment(id),
        } => old
            .comments
            .iter()
            .chain(&live.comments)
            .any(|c| c.id == *id && ids.contains(&c.page)),
        // Shared styles, variables, image assets, etc. may affect the selected page. They are
        // explicitly exposed as global changes, without ever advancing another page's baseline.
        _ => true,
    }
}

fn card(doc: &Document, id: NodeId) -> Value {
    let node = doc.node(id);
    let page = doc.page_of(id).and_then(|p| doc.node(p));
    json!({"id": id.0, "name": node.map(|n| n.name.as_str()).unwrap_or(""), "page": page.map(|n| n.name.as_str()).unwrap_or("")})
}

/// A card with its parent's id, so readers can keep only the top-most of new or removed layers.
fn with_parent(mut card: Value, doc: &Document, id: NodeId) -> Value {
    card["parent"] = json!(doc.node(id).and_then(|n| n.parent).map(|p| p.0));
    card
}

/// The design waiting since the last record: FigCraft's changes from the recorded map to `live`,
/// scoped to the page, and the cards added, changed and removed.
pub struct Drift {
    pub last: Option<Baseline>,
    pub changes: Vec<Change>,
    pub added: Vec<Value>,
    pub changed: Vec<Value>,
    pub removed: Vec<Value>,
}

impl Drift {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
            && self.added.is_empty()
            && self.changed.is_empty()
            && self.removed.is_empty()
    }
}

pub fn design_drift(path: &Path, live: &Document, page: Option<u64>) -> Result<Drift> {
    validate_page(page)?;
    let root = repository(path)?;
    let last = baseline(&root, page)?;
    let old = match &last {
        Some(last) => load_design(&root, &last.commit)?,
        None => Document::without_pages(live.name.clone()),
    };
    if page.is_some_and(|p| !old.pages.contains(&NodeId(p)) && !live.pages.contains(&NodeId(p))) {
        return Err(Error::new(
            "selected page does not exist in live design or baseline",
        ));
    }
    let before = scope_nodes(&old, page);
    let after = scope_nodes(live, page);
    let ids = before.union(&after).copied().collect();
    let changes: Vec<Change> = figcraft_collab::diff(&Replica::new(old.clone()), live)
        .map_err(|e| Error::new(format!("design diff: {e}")))?
        .changes
        .into_iter()
        .filter(|c| scoped_change(c, &old, live, &ids, page))
        .collect();
    let added: Vec<Value> = after
        .difference(&before)
        .map(|id| with_parent(card(live, *id), live, *id))
        .collect();
    let removed: Vec<Value> = before
        .difference(&after)
        .map(|id| with_parent(card(&old, *id), &old, *id))
        .collect();
    let mut fields: BTreeMap<NodeId, BTreeSet<String>> = BTreeMap::new();
    for change in &changes {
        if let Change::Set { id, prop, .. } = change {
            if before.contains(id) && after.contains(id) {
                let property = serde_json::to_value(prop)?
                    .as_str()
                    .unwrap_or("")
                    .to_owned();
                fields.entry(*id).or_default().insert(property);
            }
        }
    }
    let changed: Vec<Value> = fields.into_iter().map(|(id, fields)| {
        json!({"id": id.0, "name": live.node(id).map(|n| n.name.as_str()).unwrap_or(""), "fields": fields})
    }).collect();
    Ok(Drift {
        last,
        changes,
        added,
        changed,
        removed,
    })
}

/// Complete property changes plus the old UI's `{last,map:{added,changed,removed},code:{...}}`.
/// `changes` contains FigCraft's actual serde Change values, including non-text properties and
/// document entries. No prompt-size limit or silent truncation is applied. The design compares
/// with the last record of any kind; the code with the last Sync (`last_sync`).
pub fn diff(path: &Path, live: &Document, page: Option<u64>) -> Result<Value> {
    let root = repository(path)?;
    let drift = design_drift(&root, live, page)?;
    let last_sync = sync_baseline(&root, page)?;
    let global_count = drift.changes.iter().filter(|c| c.node().is_none()).count();
    Ok(json!({
        "last": drift.last.as_ref().map(Baseline::last_json).unwrap_or_else(|| json!({"exists": false})),
        "last_sync": last_sync.as_ref().map(Baseline::last_json).unwrap_or_else(|| json!({"exists": false})),
        "map": {"added": drift.added, "changed": drift.changed, "removed": drift.removed},
        "changes": drift.changes, "global_changes": global_count, "page": page,
        "code": code_diff(&root, last_sync.as_ref().map(|b| b.commit.as_str()))?,
    }))
}

/// The code committed since the last Sync, as `{head, from, files, commits}`.
pub fn code_since_sync(path: &Path, page: Option<u64>) -> Result<Value> {
    let root = repository(path)?;
    let last_sync = sync_baseline(&root, page)?;
    code_diff(&root, last_sync.as_ref().map(|b| b.commit.as_str()))
}

/// Page each of `changes`, `code.files`, and `code.commits` independently at the requested offset.
/// Summary cards stay complete for legacy UI consumers. Each list has a total and next offset;
/// callers needing stable multi-request pagination should use an immutable frozen document.
pub fn diff_paginated(
    path: &Path,
    live: &Document,
    page: Option<u64>,
    offset: usize,
    limit: usize,
) -> Result<Value> {
    if limit == 0 || limit > 1000 {
        return Err(Error::new("diff limit must be between 1 and 1000"));
    }
    let mut out = diff(path, live, page)?;
    let mut pagination = serde_json::Map::new();
    for (label, pointer) in [
        ("changes", "/changes"),
        ("files", "/code/files"),
        ("commits", "/code/commits"),
    ] {
        if let Some(Value::Array(values)) = out.pointer_mut(pointer) {
            let total = values.len();
            let end = offset.saturating_add(limit).min(total);
            let selected = values
                .get(offset.min(total)..end)
                .unwrap_or_default()
                .to_vec();
            *values = selected;
            pagination.insert(label.to_owned(), json!({"offset": offset, "limit": limit, "total": total, "next": if end < total { Some(end) } else { None }}));
        }
    }
    out["pagination"] = Value::Object(pagination);
    Ok(out)
}

fn code_diff(path: &Path, from: Option<&str>) -> Result<Value> {
    let tip = head(path)?;
    let empty_tree;
    let base = match from {
        Some(from) => from,
        None => {
            empty_tree = git_input(path, &["hash-object", "-w", "-t", "tree", "--stdin"], &[])?;
            &empty_tree
        }
    };
    // Binary-safe names, literal pathspec, no rename folding and no external diff helpers.
    let raw = git(
        path,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--numstat",
            "-z",
            "--no-renames",
            base,
            "--",
            ".",
            ":(exclude,literal).mindmap/map.figcraft",
        ],
    )?;
    let mut files = Vec::new();
    for row in raw.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        let mut columns = row.splitn(3, |b| *b == b'\t');
        let added = columns.next().unwrap_or_default();
        let removed = columns.next().unwrap_or_default();
        let name = columns
            .next()
            .ok_or_else(|| Error::new("invalid git numstat record"))?;
        let number = |s: &[u8]| {
            std::str::from_utf8(s)
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0)
        };
        files.push(json!({"path": String::from_utf8_lossy(name), "added": number(added), "removed": number(removed), "binary": added == b"-" || removed == b"-"}));
    }
    // New agent files are drift too; git diff alone silently omits them.
    for name in git(path, &["ls-files", "--others", "--exclude-standard", "-z"])?
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
    {
        if name == DESIGN_PATH.as_bytes() {
            continue;
        }
        files.push(json!({"path": String::from_utf8_lossy(name), "added": 0, "removed": 0, "untracked": true}));
    }
    let mut commits = Vec::new();
    if let Some(tip) = &tip {
        let range = from
            .map(|from| format!("{from}..{tip}"))
            .unwrap_or_else(|| tip.clone());
        let raw = git(path, &["log", "--format=%h%x00%s", "-z", &range])?;
        let mut fields = raw.split(|b| *b == 0);
        while let (Some(hash), Some(subject)) = (fields.next(), fields.next()) {
            if !hash.is_empty() {
                commits.push(json!({"hash": String::from_utf8_lossy(hash), "subject": String::from_utf8_lossy(subject)}));
            }
        }
    }
    Ok(json!({"head": tip, "from": from, "files": files, "commits": commits, "scope": []}))
}

/// Freeze the complete native document. The trailer alone restricts what is acknowledged: a
/// page sync does not claim that other pages in the same archive were implemented.
pub fn freeze(path: &Path, live: &Document, page: Option<u64>) -> Result<FrozenSync> {
    freeze_as(path, live, page, "sync")
}

/// [`freeze`] for any record kind: the trailer ends with `explain` or `fix` for those.
pub fn freeze_as(
    path: &Path,
    live: &Document,
    page: Option<u64>,
    kind: &str,
) -> Result<FrozenSync> {
    validate_page(page)?;
    if !KINDS.contains(&kind) {
        return Err(Error::new("record kind must be sync, explain or fix"));
    }
    let root = repository(path)?;
    let last = baseline(&root, page)?;
    if let Some(page) = page {
        let existed = match &last {
            Some(last) => load_design(&root, &last.commit)?
                .pages
                .contains(&NodeId(page)),
            None => false,
        };
        if !live.pages.contains(&NodeId(page)) && !existed {
            return Err(Error::new("cannot freeze an unknown page"));
        }
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::new("clock precedes Unix epoch"))?;
    let id = format!(
        "{:x}-{:x}-{:x}",
        now.as_nanos(),
        std::process::id(),
        NEXT_FREEZE.fetch_add(1, Ordering::Relaxed)
    );
    let cache =
        PathBuf::from(git_text(&root, &["rev-parse", "--absolute-git-dir"])?).join("mindmap-sync");
    fs::create_dir_all(&cache)?;
    let snapshot = cache.join(format!("{id}.figcraft"));
    let bytes = figcraft_format::save_bytes(live, None)
        .map_err(|e| Error::new(format!("freeze design: {e}")))?;
    // Ensure a saved/loaded frozen document retains exactly the same native encoding.
    decode_design(&bytes)?;
    let blob = git_input(&root, &["hash-object", "--stdin"], &bytes)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&snapshot)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    let mut permissions = file.metadata()?.permissions();
    permissions.set_readonly(true);
    file.set_permissions(permissions)?;
    let scope = page
        .map(|p| format!("page={p}"))
        .unwrap_or_else(|| "all".to_owned());
    Ok(FrozenSync {
        start_head: head(&root)?,
        start_ref: branch_ref(&root)?,
        repository: root,
        snapshot,
        page,
        baseline: last,
        blob,
        trailer: if kind == "sync" {
            format!("{TRAILER}: {scope} job={id}")
        } else {
            format!("{TRAILER}: {scope} job={id} {kind}")
        },
        id,
        started: now.as_secs(),
    })
}

fn check_job(path: &Path, frozen: &FrozenSync) -> Result<PathBuf> {
    let root = repository(path)?;
    if root != frozen.repository {
        return Err(Error::new("sync job belongs to another worktree"));
    }
    if branch_ref(&root)? != frozen.start_ref {
        return Err(Error::new("branch changed during sync"));
    }
    Ok(root)
}

fn frozen_bytes(frozen: &FrozenSync) -> Result<Vec<u8>> {
    let bytes = fs::read(&frozen.snapshot)?;
    if git_input(&frozen.repository, &["hash-object", "--stdin"], &bytes)? != frozen.blob {
        return Err(Error::new("frozen design integrity check failed"));
    }
    Ok(bytes)
}

pub fn frozen_document(frozen: &FrozenSync) -> Result<Document> {
    decode_design(&frozen_bytes(frozen)?)
}

/// Stage the frozen Git blob, leaving code staging and commit creation to the agent. The caller
/// must have gated document saves and installed this snapshot in the worktree beforehand, so an
/// agent's later `git add -A` cannot accidentally stage newer live edits instead.
pub fn prepare_commit(path: &Path, frozen: &FrozenSync) -> Result<Value> {
    let root = check_job(path, frozen)?;
    if let Some(start) = &frozen.start_head {
        let tip = head(&root)?.ok_or_else(|| Error::new("HEAD disappeared during sync"))?;
        if !is_ancestor(&root, start, &tip)? {
            return Err(Error::new("sync starting commit is no longer an ancestor"));
        }
    }
    let bytes = frozen_bytes(frozen)?;
    let blob = git_input(&root, &["hash-object", "-w", "--stdin"], &bytes)?;
    git(
        &root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("100644,{blob},{DESIGN_PATH}"),
        ],
    )?;
    Ok(
        json!({"job": frozen.id, "blob": blob, "path": DESIGN_PATH, "trailer": frozen.trailer, "start_head": frozen.start_head}),
    )
}

/// Verify explicit completion against Git, never agent status. The commit must be newer than
/// the job, reachable from current HEAD, descend from its starting HEAD, and carry exactly this
/// job's trailer and exact frozen native blob. Extra sync trailers are rejected to avoid an
/// accidental claim that unrelated pages were implemented.
pub fn verify_completed(path: &Path, frozen: &FrozenSync, revision: &str) -> Result<Baseline> {
    let root = check_job(path, frozen)?;
    let commit = resolve_commit(&root, revision)?;
    let tip = head(&root)?.ok_or_else(|| Error::new("sync has no commit"))?;
    if !is_ancestor(&root, &commit, &tip)? {
        return Err(Error::new("sync commit is not reachable from HEAD"));
    }
    if let Some(start) = &frozen.start_head {
        if start == &commit || !is_ancestor(&root, start, &commit)? {
            return Err(Error::new(
                "sync needs a new commit descending from its starting HEAD",
            ));
        }
    }
    let (at, markers) = commit_markers(&root, &commit)?;
    let expected = frozen
        .trailer
        .strip_prefix("Mindmap-Sync: ")
        .ok_or_else(|| Error::new("invalid frozen trailer"))?;
    if markers.len() != 1
        || markers.first().map(String::as_str) != Some(expected)
        || trailer_scope(expected) != Some(frozen.page)
    {
        return Err(Error::new(
            "commit does not carry exactly this sync job's page trailer",
        ));
    }
    frozen_bytes(frozen)?;
    if design_blob(&root, &commit)?.as_ref() != Some(&frozen.blob) {
        return Err(Error::new(
            "commit does not contain the exact frozen design",
        ));
    }
    Ok(Baseline {
        commit,
        at,
        page: frozen.page,
        kind: marker_kind(expected).to_owned(),
        trailer: frozen.trailer.clone(),
    })
}

/// Discover explicit verified completion after an agent event or restart. A matching but
/// incorrect marker returns an error; the caller must not turn it into success on idle.
pub fn find_completed(path: &Path, frozen: &FrozenSync) -> Result<Option<Baseline>> {
    let root = check_job(path, frozen)?;
    let Some(tip) = head(&root)? else {
        return Ok(None);
    };
    let range = frozen
        .start_head
        .as_ref()
        .map(|start| format!("{start}..{tip}"))
        .unwrap_or(tip);
    let candidates = git_text(
        &root,
        &[
            "log",
            "--topo-order",
            "--format=%H",
            "--fixed-strings",
            &format!("--grep=job={}", frozen.id),
            &range,
        ],
    )?;
    for commit in candidates.lines() {
        let (_, markers) = commit_markers(&root, commit)?;
        if markers.iter().any(|m| {
            m.split_whitespace()
                .any(|part| part == format!("job={}", frozen.id))
        }) {
            return verify_completed(&root, frozen, commit).map(Some);
        }
    }
    Ok(None)
}

fn requests_dir(root: &Path) -> Result<PathBuf> {
    Ok(
        PathBuf::from(git_text(root, &["rev-parse", "--absolute-git-dir"])?)
            .join("mindmap-sync")
            .join("requests"),
    )
}

fn valid_request(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// Keep the map as it is when a request (a comment or Init) is sent: the request's starting
/// point. A later [`record_document`] tells this request's edits from edits made before it. The
/// copy lives in the worktree's private Git directory; copies older than a month are removed.
pub fn save_request(path: &Path, live: &Document) -> Result<String> {
    let root = repository(path)?;
    let dir = requests_dir(&root)?;
    fs::create_dir_all(&dir)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::new("clock precedes Unix epoch"))?;
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| now.checked_sub(t.duration_since(UNIX_EPOCH).ok()?))
                .is_some_and(|age| age.as_secs() > REQUEST_TTL_SECS);
            if old {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    let id = format!(
        "{:x}-{:x}",
        now.as_nanos(),
        NEXT_FREEZE.fetch_add(1, Ordering::Relaxed)
    );
    let bytes = figcraft_format::save_bytes(live, None)
        .map_err(|e| Error::new(format!("keep request start: {e}")))?;
    fs::write(dir.join(format!("{id}.figcraft")), bytes)?;
    Ok(id)
}

/// The map as it was when request `id` was sent.
pub fn load_request(path: &Path, id: &str) -> Result<Document> {
    if !valid_request(id) {
        return Err(Error::new("unknown request id"));
    }
    let root = repository(path)?;
    let bytes = fs::read(requests_dir(&root)?.join(format!("{id}.figcraft")))
        .map_err(|_| Error::new("unknown request id: its starting map is gone"))?;
    decode_design(&bytes)
}

/// What a record committed and what it left waiting.
pub struct Recorded {
    pub document: Document,
    /// Changes this request made to the map, all recorded but `waiting`.
    pub edits: usize,
    /// Cards (or document entries) the request changed where an older waiting edit changed the
    /// same thing: they are not recorded and keep waiting for Sync.
    pub waiting: Vec<String>,
    /// Whether an earlier record exists for this scope. Without one, the whole scope as the
    /// request found it is recorded too.
    pub had_record: bool,
}

/// The map to commit for a request: the last recorded map plus only this request's edits.
///
/// `start` is the map when the request was sent, `live` the map now. The design edits waiting
/// in scope at `start` (its difference from the last record) are taken back out, then the
/// request's own edits (`start` to `live`) are merged on top with FigCraft's three-way merge.
/// Where both touched the same thing the waiting edit wins, so nothing waiting is recorded.
/// Outside the scope the map stays as it is live: the trailer claims only the scope. Without an
/// earlier record for the scope (or when the page is newer than it), the request's starting map
/// counts as recorded: this is how a first explanation or Init declares the map current.
pub fn record_document(
    path: &Path,
    start: &Document,
    live: &Document,
    page: Option<u64>,
) -> Result<Recorded> {
    validate_page(page)?;
    let root = repository(path)?;
    let last = baseline(&root, page)?;
    let recorded = match &last {
        Some(last) => Some(load_design(&root, &last.commit)?)
            .filter(|old| page.is_none_or(|p| old.pages.contains(&NodeId(p)))),
        None => None,
    };
    let had_record = recorded.is_some();
    let undone = match &recorded {
        Some(old) => {
            let before = scope_nodes(start, page);
            let after = scope_nodes(old, page);
            let ids = before.union(&after).copied().collect();
            let mut replica = Replica::new(start.clone());
            let back: Vec<Change> = figcraft_collab::diff(&replica, old)
                .map_err(|e| Error::new(format!("record diff: {e}")))?
                .changes
                .into_iter()
                .filter(|c| scoped_change(c, start, old, &ids, page))
                .collect();
            replica
                .apply(&back, figcraft_collab::replica::Mode::Strict)
                .map_err(|e| Error::new(format!("take waiting edits out: {e}")))?;
            replica.document().clone()
        }
        None => start.clone(),
    };
    let plan =
        merge::plan(start, live, &undone).map_err(|e| Error::new(format!("plan record: {e}")))?;
    let waiting: Vec<String> = plan
        .conflicts
        .iter()
        .map(|c| match &c.key {
            merge::Key::Layer(id) | merge::Key::Node(id, _) => live
                .node(*id)
                .or_else(|| start.node(*id))
                .map(|n| format!("{} (id {})", n.name, id.0))
                .unwrap_or_else(|| c.key.to_string()),
            _ => c.key.to_string(),
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let (document, _) = plan
        .merge(&undone, merge::Side::Branch, &|_| Some(merge::Side::Main))
        .map_err(|e| Error::new(format!("record merge: {e}")))?;
    Ok(Recorded {
        document,
        edits: plan.branch.len(),
        waiting,
        had_record,
    })
}

/// Immutable plan bound to three resolved commit ids. Apply its resulting document through the
/// collaboration session; this never installs a Git merge driver or mutates a branch.
#[derive(Clone, Debug)]
pub struct MergePlan {
    pub base: String,
    pub branch: String,
    pub main: String,
    plan: merge::Plan,
    main_document: Document,
}

impl MergePlan {
    pub fn json(&self) -> Value {
        let conflicts: Vec<Value> = self.plan.conflicts.iter().map(|c| json!({
            "key": c.key.to_string(), "deleted_by": c.deleted_by,
            "branch": c.branch.iter().filter_map(|i| self.plan.branch.get(*i)).collect::<Vec<_>>(),
            "main": c.main.iter().filter_map(|i| self.plan.main.get(*i)).collect::<Vec<_>>(),
        })).collect();
        json!({"base": self.base, "branch": self.branch, "main": self.main,
            "branch_changes": self.plan.branch, "main_changes": self.plan.main, "conflicts": conflicts})
    }
}

pub fn plan_merge(path: &Path, branch: &str, main: &str) -> Result<MergePlan> {
    let root = repository(path)?;
    let branch = resolve_commit(&root, branch)?;
    let main = resolve_commit(&root, main)?;
    let bases = git_text(&root, &["merge-base", "--all", &branch, &main])?;
    let bases: Vec<&str> = bases.lines().collect();
    if bases.len() != 1 {
        return Err(Error::new(
            "design merge requires one unambiguous Git merge base",
        ));
    }
    let base = bases[0].to_owned();
    let base_document = load_design(&root, &base)?;
    let branch_document = load_design(&root, &branch)?;
    let main_document = load_design(&root, &main)?;
    let plan = merge::plan(&base_document, &branch_document, &main_document)
        .map_err(|e| Error::new(format!("plan design merge: {e}")))?;
    Ok(MergePlan {
        base,
        branch,
        main,
        plan,
        main_document,
    })
}

/// Conflict keys are exactly the strings in `MergePlan::json().conflicts[*].key`; picks are
/// FigCraft's `branch` / `main`. Missing and unknown selections are errors, never implicit wins.
pub fn resolve_merge(
    plan: &MergePlan,
    selections: &BTreeMap<String, merge::Side>,
) -> Result<Document> {
    let keys: BTreeSet<String> = plan
        .plan
        .conflicts
        .iter()
        .map(|c| c.key.to_string())
        .collect();
    if selections.keys().any(|k| !keys.contains(k)) {
        return Err(Error::new("unknown design merge conflict selection"));
    }
    plan.plan
        .merge(&plan.main_document, merge::Side::Branch, &|key| {
            selections.get(&key.to_string()).copied()
        })
        .map(|(doc, _)| doc)
        .map_err(|e| Error::new(format!("resolve design merge: {e}")))
}

fn merge_heads(path: &Path) -> Result<Vec<String>> {
    let merge_file = git_text(path, &["rev-parse", "--git-path", "MERGE_HEAD"])?;
    let file = Path::new(&merge_file);
    let file = if file.is_absolute() {
        file.to_owned()
    } else {
        path.join(file)
    };
    match fs::read_to_string(file) {
        Ok(text) => text
            .lines()
            .map(|line| resolve_commit(path, line))
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

/// Incoming commit ids of the current Git merge; empty when no merge is pending. The caller
/// uses these immutable ids to bind a resumed document gate to its original merge plan.
pub fn pending_merge(path: &Path) -> Result<Vec<String>> {
    merge_heads(&repository(path)?)
}

/// Abort only Git's pending merge, using its ordinary recovery mechanism. Never resets or
/// cleans the worktree. After success the caller restores authority with `abort_prepared`,
/// rather than interpreting Git's restored HEAD file as an external design edit.
pub fn abort_merge(path: &Path) -> Result<()> {
    let root = repository(path)?;
    if merge_heads(&root)?.is_empty() {
        return Err(Error::new("there is no Git merge to abort"));
    }
    // --cacheinfo deliberately changes only the index blob, leaving its filesystem stat
    // cache empty. Refresh metadata before Git's merge-abort safety check; otherwise even
    // identical prepared bytes can look dirty. No path arguments: this must never stage an
    // external edit. Real dirty files return 1 and are still protected by merge --abort.
    match git_output(&root, &["update-index", "--unmerged", "--refresh"])?
        .status
        .code()
    {
        Some(0 | 1) => {}
        _ => return Err(Error::new("cannot refresh Git index before merge abort")),
    }
    git(&root, &["merge", "--abort"])?;
    if !merge_heads(&root)?.is_empty() {
        return Err(Error::new("Git merge state remains after abort"));
    }
    Ok(())
}

fn unmerged_paths(path: &Path) -> Result<Vec<String>> {
    Ok(
        git(path, &["diff", "--name-only", "--diff-filter=U", "-z"])?
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect(),
    )
}

/// Start a real Git merge, leaving code conflicts for explicit resolution. The caller first
/// gates document saves and caches live edits, then restores HEAD's design to disk. A clean
/// tracked tree and index are required: pre-existing code work must never become merge output.
/// This command may change the on-disk design; the caller must not reload it into the session.
pub fn begin_merge(path: &Path, incoming: &str) -> Result<Value> {
    let root = repository(path)?;
    if !merge_heads(&root)?.is_empty() {
        return Err(Error::new("a Git merge is already in progress"));
    }
    let before = head(&root)?.ok_or_else(|| Error::new("cannot merge into an unborn branch"))?;
    let incoming = resolve_commit(&root, incoming)?;
    for args in [
        &["diff", "--quiet", "--no-ext-diff", "--no-textconv"][..],
        &[
            "diff",
            "--cached",
            "--quiet",
            "--no-ext-diff",
            "--no-textconv",
        ][..],
    ] {
        if !git_output(&root, args)?.status.success() {
            return Err(Error::new("merge requires a clean tracked worktree and index; preserve live design behind the document save gate first"));
        }
    }
    let output = git_output(
        &root,
        &["merge", "--no-commit", "--no-ff", "--no-edit", &incoming],
    )?;
    let heads = merge_heads(&root)?;
    let conflicts = unmerged_paths(&root)?;
    if !output.status.success() && (heads.is_empty() || conflicts.is_empty()) {
        return Err(Error::new(
            "Git could not start the merge; worktree and index were left for inspection",
        ));
    }
    let code_conflicts: Vec<&str> = conflicts
        .iter()
        .map(String::as_str)
        .filter(|p| *p != DESIGN_PATH)
        .collect();
    let state = if heads.is_empty() {
        "up_to_date"
    } else if conflicts.is_empty() {
        "ready"
    } else {
        "conflicts"
    };
    Ok(
        json!({"state": state, "committed": false, "head": before, "incoming": incoming,
        "merge_head": heads, "conflicts": conflicts, "code_conflicts": code_conflicts}),
    )
}

/// Stage only the session-resolved design, preserving all other index entries.
pub fn stage_design(path: &Path) -> Result<()> {
    let root = repository(path)?;
    let bytes = fs::read(root.join(DESIGN_PATH))?;
    stage_bytes(&root, &bytes)
}

/// Stage exactly the document gate's prepared bytes, regardless of later external worktree
/// edits. Only the design index entry changes; code remains explicitly staged by its author.
pub fn stage_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let root = repository(path)?;
    decode_design(bytes)?;
    let blob = git_input(&root, &["hash-object", "-w", "--stdin"], bytes)?;
    git(
        &root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("100644,{blob},{DESIGN_PATH}"),
        ],
    )?;
    Ok(())
}

/// Commit an already prepared merge. Never stages code, never resolves code conflicts, and
/// never creates a misleading ordinary commit if Git's merge state has disappeared.
pub fn commit_merge(path: &Path, message: &str) -> Result<String> {
    let root = repository(path)?;
    let incoming = merge_heads(&root)?;
    if incoming.is_empty() {
        return Err(Error::new("there is no Git merge to commit"));
    }
    if !unmerged_paths(&root)?.is_empty() {
        return Err(Error::new(
            "resolve and stage every Git conflict before committing the merge",
        ));
    }
    if message.trim().is_empty() {
        return Err(Error::new("merge commit message is empty"));
    }
    // A merge isn't a synchronization acknowledgment, even if its two parents had sync markers.
    if message
        .lines()
        .any(|line| line.to_ascii_lowercase().starts_with("mindmap-sync:"))
    {
        return Err(Error::new(
            "merge commits must not claim a new Mindmap-Sync baseline",
        ));
    }
    let before = head(&root)?.ok_or_else(|| Error::new("merge HEAD disappeared"))?;
    git_input(&root, &["commit", "--file=-"], message.as_bytes())?;
    let commit = head(&root)?.ok_or_else(|| Error::new("merge commit has no HEAD"))?;
    let parents = git_text(&root, &["show", "-s", "--format=%P", &commit])?;
    let parents: BTreeSet<&str> = parents.split_whitespace().collect();
    if commit == before
        || !parents.contains(before.as_str())
        || incoming.iter().any(|id| !parents.contains(id.as_str()))
    {
        return Err(Error::new("Git did not produce the expected merge commit"));
    }
    Ok(commit)
}

#[cfg(test)]
mod tests;
