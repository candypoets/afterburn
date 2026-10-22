//! Sync: map snapshots, design and code diffs, prompts, and in-flight jobs.
//!
//! A sync is two honest diffs. The client sends the map as it is now; the service compares it by
//! node id with the snapshot recorded by the last successful sync, and runs `git` against the
//! commit that sync recorded. The prompt carries both lists. The job remembers the head commit and
//! the snapshot from the start of the sync; only a finished agent writes `last_sync`.
//!
//! This module never edits or locks the document. The map stays editable while a sync is in
//! flight, and a failed job leaves the last successful snapshot untouched.

use std::collections::{BTreeMap, BTreeSet};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::agent::AgentOutcome;
use crate::error::{Error, Result};
use crate::records::LastSync;

/// The most pages one map snapshot may carry.
pub const MAX_MAP_PAGES: usize = 500;

/// The most nodes one map snapshot may carry.
pub const MAX_MAP_NODES: usize = 5_000;

/// The most changed files a code diff returns.
pub const MAX_GIT_FILES: usize = 100;

/// The most commits a code diff returns.
pub const MAX_GIT_COMMITS: usize = 100;

/// The most characters of sync prompt submitted to an agent.
pub const MAX_PROMPT_CHARS: usize = 20_000;

/// The most characters of the optional user note kept in the prompt.
pub const MAX_PROMPT_NOTE_CHARS: usize = 2_000;

/// The most characters one prompt line keeps before it is shortened.
pub const MAX_PROMPT_LINE_CHARS: usize = 240;

/// Space held back for an "… and N more" line in a prompt section.
const OMISSION_RESERVE: usize = 48;

/// How long an in-flight job may run before it is reported as failed.
pub const JOB_TIMEOUT_SECS: u64 = 30 * 60;

/// One page in the client-supplied map snapshot.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MapPage {
    /// Positive page id.
    pub id: u64,
    /// Page name (`Mindmap`).
    pub name: String,
}

/// One node in the client-supplied map snapshot.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MapNode {
    /// Positive node id.
    pub id: u64,
    /// Card label.
    pub name: String,
    /// Parent node id, when the card hangs below another card.
    #[serde(default)]
    pub parent: Option<u64>,
    /// Page the node lives on.
    pub page: u64,
    /// Node kind (`frame`, `text`, …).
    pub kind: String,
    /// Card body text.
    pub text: String,
}

/// The map as the client sees it: pages plus nodes, stored verbatim in `last_sync`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct MapSnapshot {
    /// Pages in the map.
    #[serde(default)]
    pub pages: Vec<MapPage>,
    /// Nodes in the map.
    #[serde(default)]
    pub nodes: Vec<MapNode>,
}

/// One added or removed card in the design diff.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct MapCard {
    /// Node id.
    pub id: u64,
    /// Card label in the snapshot that owns the card (current for added, previous for removed).
    pub name: String,
    /// Page label in that snapshot.
    pub page: String,
}

/// One changed card in the design diff.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct MapChange {
    /// Node id.
    pub id: u64,
    /// Current card label.
    pub name: String,
    /// Changed fields, in `name`, `text`, `kind`, `parent`, `page` order.
    pub fields: Vec<String>,
}

/// The design half of a diff: cards added, removed, and changed by id.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct MapDiff {
    /// Nodes only in the current snapshot.
    pub added: Vec<MapCard>,
    /// Nodes only in the last-synced snapshot.
    pub removed: Vec<MapCard>,
    /// Nodes in both snapshots whose fields differ.
    pub changed: Vec<MapChange>,
}

/// One changed file from `git diff --numstat`.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct FileDelta {
    /// Repository-relative path.
    pub path: String,
    /// Added lines (0 for binary files).
    pub added: u64,
    /// Removed lines (0 for binary files).
    pub removed: u64,
}

/// One commit from `git log`.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Commit {
    /// Abbreviated hash (`%h`).
    pub hash: String,
    /// Subject line (`%s`).
    pub subject: String,
}

/// The code half of a diff: head, previous commit, files, and commits.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct CodeDiff {
    /// `git rev-parse HEAD`, when it worked.
    pub head: Option<String>,
    /// Commit recorded by the last successful sync, when it had one.
    pub from: Option<String>,
    /// Changed files, capped at [`MAX_GIT_FILES`].
    pub files: Vec<FileDelta>,
    /// Commits since the last sync, capped at [`MAX_GIT_COMMITS`].
    pub commits: Vec<Commit>,
    /// Why the code side is empty (no path, not a repository, bad commit, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The state of one sync job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// The agent was prompted and has not finished.
    InFlight,
    /// The agent finished; `last_sync` was written.
    Done,
    /// The agent disappeared, errored, or timed out.
    Failed,
}

/// A sync job as exposed to the client.
#[derive(Clone, Debug, Serialize)]
pub struct SyncJobView {
    /// Job id returned by `sync` and passed to `sync_status`.
    pub id: String,
    /// Current state.
    pub state: JobState,
    /// Start time, `YYYY-MM-DDTHH:MM:SSZ`.
    pub started: String,
    /// The agent that accepted the prompt.
    pub agent: AgentOutcome,
    /// Failure reason, when the state is `failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The in-memory job record. It lives only in the running service.
#[derive(Clone, Debug)]
pub struct SyncJob {
    /// Job id.
    pub id: String,
    /// Herdr workspace id.
    pub workspace: String,
    /// Start time.
    pub started: String,
    /// Start time, seconds since the Unix epoch, for the timeout.
    pub started_secs: u64,
    /// Repository head captured at sync start.
    pub head: Option<String>,
    /// The map snapshot captured at sync start.
    pub map_snapshot: MapSnapshot,
    /// Pane hosting the prompted agent.
    pub pane: String,
    /// The agent outcome that accepted the prompt.
    pub agent: AgentOutcome,
    /// Current job state.
    pub state: JobState,
    /// Whether Herdr has ever reported the agent as `working` or `blocked`.
    pub seen_active: bool,
    /// Failure reason, when failed.
    pub reason: Option<String>,
}

impl SyncJob {
    /// The serializable view returned by `sync` and `sync_status`.
    pub fn view(&self) -> SyncJobView {
        SyncJobView {
            id: self.id.clone(),
            state: self.state,
            started: self.started.clone(),
            agent: self.agent.clone(),
            reason: self.reason.clone(),
        }
    }
}

/// The next state implied by one Herdr observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transition {
    /// State the job should take.
    pub state: JobState,
    /// Updated `seen_active` flag.
    pub seen_active: bool,
    /// Failure reason for `failed`.
    pub reason: Option<String>,
}

/// Parse and validate the `map` request field.
///
/// The snapshot is an object with a `pages` list and a `nodes` list. Page and node ids are
/// positive integers, page ids are unique and every node names a page that exists, node ids are
/// unique, and `parent` is optional (a positive integer or null when present). `name`, `kind`,
/// and `text` are strings. At most [`MAX_MAP_PAGES`] pages and [`MAX_MAP_NODES`] nodes are
/// accepted; larger snapshots are rejected rather than truncated.
pub fn parse_snapshot(raw: &Value) -> Result<MapSnapshot> {
    let object = raw
        .as_object()
        .ok_or_else(|| Error::new(format!("\"map\" must be an object, not {raw}")))?;
    let pages_raw = object
        .get("pages")
        .ok_or_else(|| Error::new("\"map\" has no \"pages\" list"))?;
    let pages_values = pages_raw
        .as_array()
        .ok_or_else(|| Error::new(format!("\"map.pages\" must be a list, not {pages_raw}")))?;
    if pages_values.len() > MAX_MAP_PAGES {
        return Err(Error::new(format!(
            "map.pages has too many pages ({}; at most {MAX_MAP_PAGES})",
            pages_values.len()
        )));
    }
    let nodes_raw = object
        .get("nodes")
        .ok_or_else(|| Error::new("\"map\" has no \"nodes\" list"))?;
    let nodes_values = nodes_raw
        .as_array()
        .ok_or_else(|| Error::new(format!("\"map.nodes\" must be a list, not {nodes_raw}")))?;
    if nodes_values.len() > MAX_MAP_NODES {
        return Err(Error::new(format!(
            "map.nodes has too many nodes ({}; at most {MAX_MAP_NODES})",
            nodes_values.len()
        )));
    }

    let mut pages = Vec::with_capacity(pages_values.len());
    let mut page_ids = BTreeSet::new();
    for (index, item) in pages_values.iter().enumerate() {
        let path = format!("map.pages[{index}]");
        let page = item
            .as_object()
            .ok_or_else(|| Error::new(format!("{path} must be an object, not {item}")))?;
        let id = positive_id(page.get("id"), &format!("{path}.id"))?;
        if !page_ids.insert(id) {
            return Err(Error::new(format!("{path}.id {id} is duplicated")));
        }
        pages.push(MapPage {
            id,
            name: string_field(page, "name", &path)?,
        });
    }

    let mut nodes = Vec::with_capacity(nodes_values.len());
    let mut node_ids = BTreeSet::new();
    for (index, item) in nodes_values.iter().enumerate() {
        let path = format!("map.nodes[{index}]");
        let node = item
            .as_object()
            .ok_or_else(|| Error::new(format!("{path} must be an object, not {item}")))?;
        let id = positive_id(node.get("id"), &format!("{path}.id"))?;
        if !node_ids.insert(id) {
            return Err(Error::new(format!("{path}.id {id} is duplicated")));
        }
        let page = positive_id(node.get("page"), &format!("{path}.page"))?;
        if !page_ids.contains(&page) {
            return Err(Error::new(format!(
                "{path}.page {page} is not one of the map's page ids"
            )));
        }
        let parent = match node.get("parent") {
            None | Some(Value::Null) => None,
            Some(value) => Some(positive_id(Some(value), &format!("{path}.parent"))?),
        };
        nodes.push(MapNode {
            id,
            name: string_field(node, "name", &path)?,
            parent,
            page,
            kind: string_field(node, "kind", &path)?,
            text: string_field(node, "text", &path)?,
        });
    }

    Ok(MapSnapshot { pages, nodes })
}

fn positive_id(value: Option<&Value>, path: &str) -> Result<u64> {
    match value {
        Some(value) => match value.as_u64() {
            Some(id) if id > 0 => Ok(id),
            _ => Err(Error::new(format!(
                "{path} must be a positive integer, not {value}"
            ))),
        },
        None => Err(Error::new(format!("{path} is missing"))),
    }
}

fn string_field(object: &Map<String, Value>, field: &str, path: &str) -> Result<String> {
    match object.get(field) {
        Some(Value::String(value)) => Ok(value.clone()),
        Some(other) => Err(Error::new(format!(
            "{path}.{field} must be a string, not {other}"
        ))),
        None => Err(Error::new(format!("{path}.{field} is missing"))),
    }
}

/// Compare a current snapshot with the last-synced one, by node id.
///
/// With no previous snapshot every node is added and nothing is removed or changed: that is the
/// honest first-sync answer. Page labels in added rows come from the current snapshot and in
/// removed rows from the previous one, so a removed card keeps the page name it had.
pub fn diff_maps(current: &MapSnapshot, previous: Option<&MapSnapshot>) -> MapDiff {
    let mut diff = MapDiff::default();
    let current_pages = page_names(&current.pages);
    let current_nodes = nodes_by_id(current);
    let Some(previous) = previous else {
        for node in current_nodes.values() {
            diff.added.push(card(node, &current_pages));
        }
        return diff;
    };

    let previous_pages = page_names(&previous.pages);
    let previous_nodes = nodes_by_id(previous);
    for node in current_nodes.values() {
        match previous_nodes.get(&node.id) {
            None => diff.added.push(card(node, &current_pages)),
            Some(old) => {
                let fields = changed_fields(node, old);
                if !fields.is_empty() {
                    diff.changed.push(MapChange {
                        id: node.id,
                        name: node.name.clone(),
                        fields,
                    });
                }
            }
        }
    }
    for node in previous_nodes.values() {
        if !current_nodes.contains_key(&node.id) {
            diff.removed.push(card(node, &previous_pages));
        }
    }
    diff
}

fn nodes_by_id(snapshot: &MapSnapshot) -> BTreeMap<u64, &MapNode> {
    snapshot.nodes.iter().map(|node| (node.id, node)).collect()
}

fn page_names(pages: &[MapPage]) -> BTreeMap<u64, String> {
    pages
        .iter()
        .map(|page| (page.id, page.name.clone()))
        .collect()
}

fn card(node: &MapNode, pages: &BTreeMap<u64, String>) -> MapCard {
    MapCard {
        id: node.id,
        name: node.name.clone(),
        page: page_label(pages, node.page),
    }
}

fn page_label(pages: &BTreeMap<u64, String>, id: u64) -> String {
    pages.get(&id).cloned().unwrap_or_else(|| format!("#{id}"))
}

fn changed_fields(node: &MapNode, old: &MapNode) -> Vec<String> {
    let mut fields = Vec::new();
    if node.name != old.name {
        fields.push("name".to_string());
    }
    if node.text != old.text {
        fields.push("text".to_string());
    }
    if node.kind != old.kind {
        fields.push("kind".to_string());
    }
    if node.parent != old.parent {
        fields.push("parent".to_string());
    }
    if node.page != old.page {
        fields.push("page".to_string());
    }
    fields
}

/// The `last` object both `diff` and `sync_status` return.
pub fn last_json(last: Option<&LastSync>) -> Value {
    match last {
        Some(last) => serde_json::json!({
            "exists": true,
            "commit": last.commit,
            "at": last.at,
        }),
        None => serde_json::json!({ "exists": false }),
    }
}

/// Build the code half of a diff.
///
/// `last_commit` is `None` for a first sync (only `head` is filled in), `Some(commit)` for a normal
/// diff, and `Some("")` when the last sync recorded no commit (for example a workspace that was
/// not a git repository at the time). A missing path, a non-repository, an unknown commit, or a
/// failed git command yields empty `files`/`commits` and a `note`, never an error: the design half
/// of the diff is still useful.
pub fn code_diff(path: Option<&str>, last_commit: Option<&str>) -> CodeDiff {
    let mut diff = CodeDiff::default();
    let last = last_commit
        .map(str::trim)
        .filter(|commit| !commit.is_empty());
    diff.from = last.map(str::to_owned);

    let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) else {
        diff.note = Some("workspace has no checkout path".to_string());
        return diff;
    };

    let head = match git(path, &["rev-parse", "HEAD"]) {
        Ok(stdout) => {
            let head = stdout.trim();
            if head.is_empty() {
                diff.note = Some(format!("git rev-parse HEAD returned nothing in {path}"));
                return diff;
            }
            head.to_string()
        }
        Err(reason) => {
            diff.note = Some(format!("cannot read HEAD in {path}: {reason}"));
            return diff;
        }
    };
    diff.head = Some(head.clone());

    if last_commit.is_some() && last.is_none() {
        diff.note = Some("last sync recorded no commit; no code diff".to_string());
        return diff;
    }
    let Some(last) = last else {
        return diff;
    };
    if last == head {
        return diff;
    }

    let range = format!("{last}..HEAD");
    match git(path, &["diff", "--numstat", &range]) {
        Ok(stdout) => diff.files = parse_numstat(&stdout),
        Err(reason) => {
            diff.note = Some(format!("git diff {range} failed: {reason}"));
            return diff;
        }
    }
    match git(path, &["log", "--format=%h%x09%s", &range]) {
        Ok(stdout) => diff.commits = parse_log(&stdout),
        Err(reason) => diff.note = Some(format!("git log {range} failed: {reason}")),
    }
    diff
}

fn git(path: &str, args: &[&str]) -> std::result::Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("-c")
        .arg("core.quotepath=false")
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("could not run git: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr.trim();
        return Err(if message.is_empty() {
            format!("git exited with {:?}", output.status)
        } else {
            truncate_chars(message, 300)
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_numstat(stdout: &str) -> Vec<FileDelta> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let added = parts.next()?;
            let removed = parts.next()?;
            let path = parts.next()?.trim_end();
            if path.is_empty() {
                return None;
            }
            Some(FileDelta {
                path: path.to_string(),
                added: line_count(added),
                removed: line_count(removed),
            })
        })
        .take(MAX_GIT_FILES)
        .collect()
}

fn line_count(value: &str) -> u64 {
    value.parse().unwrap_or(0)
}

fn parse_log(stdout: &str) -> Vec<Commit> {
    stdout
        .lines()
        .filter_map(|line| {
            let (hash, subject) = line.split_once('\t')?;
            if hash.is_empty() {
                return None;
            }
            Some(Commit {
                hash: hash.to_string(),
                subject: subject.to_string(),
            })
        })
        .take(MAX_GIT_COMMITS)
        .collect()
}

/// Render the prompt `sync` submits to an agent.
///
/// ```text
/// Sync <label> (<path>) against the code.
///
/// DESIGN CHANGES SINCE <last|first> sync:
/// + <card> (<page>)
/// ~ <card> (name, text)
/// - <card> (<page>)
///
/// CODE SINCE <commit>:
/// <n files>, <n commits>
///   <path> (+a/−r)
///   <hash> <subject>
///
/// NOTE: <prompt>
/// ```
///
/// Empty sections are omitted (first sync has no code section). Long prompts stay under
/// [`MAX_PROMPT_CHARS`]: each line is shortened, a section reports `… and N more` when it runs out
/// of room, and the optional note is shortened to [`MAX_PROMPT_NOTE_CHARS`] and always kept at the
/// end when present.
pub fn format_prompt(
    label: &str,
    path: Option<&str>,
    map: &MapDiff,
    code: &CodeDiff,
    has_last: bool,
    note: &str,
) -> String {
    let label = truncate_chars(&one_line(label), 200);
    let mut header = format!("Sync {label}");
    if let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) {
        header.push_str(&format!(" ({})", truncate_chars(&one_line(path), 300)));
    }
    header.push_str(" against the code.\n");

    let note = note.trim();
    let note_block = if note.is_empty() {
        String::new()
    } else {
        format!("NOTE: {}\n", truncate_chars(note, MAX_PROMPT_NOTE_CHARS))
    };
    let reserved = header.chars().count() + note_block.chars().count() + 4;
    let limit = MAX_PROMPT_CHARS.saturating_sub(reserved);
    let mut body = String::new();
    let mut used = 0usize;

    let title = if has_last {
        "DESIGN CHANGES SINCE last sync:"
    } else {
        "DESIGN CHANGES SINCE first sync:"
    };
    let mut design = Vec::new();
    for card in &map.added {
        design.push(format!(
            "+ {} ({})",
            one_line(&card.name),
            one_line(&card.page)
        ));
    }
    for change in &map.changed {
        design.push(format!(
            "~ {} ({})",
            one_line(&change.name),
            change.fields.join(", ")
        ));
    }
    for card in &map.removed {
        design.push(format!(
            "- {} ({})",
            one_line(&card.name),
            one_line(&card.page)
        ));
    }
    push_section(&mut body, &mut used, limit, title, &design);

    let mut code_lines = Vec::new();
    if !code.files.is_empty() || !code.commits.is_empty() {
        code_lines.push(format!(
            "{} files, {} commits",
            code.files.len(),
            code.commits.len()
        ));
        for file in &code.files {
            code_lines.push(format!(
                "  {} (+{}{}{})",
                one_line(&file.path),
                file.added,
                '\u{2212}',
                file.removed
            ));
        }
        for commit in &code.commits {
            code_lines.push(format!(
                "  {} {}",
                one_line(&commit.hash),
                one_line(&commit.subject)
            ));
        }
        let from = truncate_chars(&one_line(code.from.as_deref().unwrap_or("unknown")), 200);
        push_section(
            &mut body,
            &mut used,
            limit,
            &format!("CODE SINCE {from}:"),
            &code_lines,
        );
    }

    let mut out = header;
    out.push_str(&body);
    if !note_block.is_empty() {
        out.push('\n');
        out.push_str(&note_block);
    }
    let out = out.trim_end().to_string();
    debug_assert!(out.chars().count() <= MAX_PROMPT_CHARS);
    out
}

fn push_section(out: &mut String, used: &mut usize, limit: usize, title: &str, entries: &[String]) {
    if entries.is_empty() {
        return;
    }
    let title = truncate_chars(title, MAX_PROMPT_LINE_CHARS);
    let title_cost = title.chars().count() + 2;
    if *used + title_cost + 1 > limit {
        return;
    }
    out.push('\n');
    out.push_str(&title);
    out.push('\n');
    *used += title_cost;

    let mut written = 0usize;
    for entry in entries {
        let line = truncate_chars(entry, MAX_PROMPT_LINE_CHARS);
        let cost = line.chars().count() + 1;
        if *used + cost + OMISSION_RESERVE > limit {
            break;
        }
        out.push_str(&line);
        out.push('\n');
        *used += cost;
        written += 1;
    }
    let skipped = entries.len() - written;
    if skipped > 0 {
        let more = format!("  … and {skipped} more");
        let cost = more.chars().count() + 1;
        if *used + cost <= limit {
            out.push_str(&more);
            out.push('\n');
            *used += cost;
        }
    }
}

/// Keep a label, name, or path on one line.
fn one_line(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.trim().chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if character.is_control() => out.push(' '),
            character => out.push(character),
        }
    }
    out
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    if limit == 0 {
        return String::new();
    }
    let mut shortened: String = value.chars().take(limit.saturating_sub(1)).collect();
    shortened.push('…');
    shortened
}

/// Decide the next state from one Herdr observation.
///
/// `status` is `None` when the pane no longer holds an agent, `Some("working")`, `Some("blocked")`,
/// `Some("idle")`, `Some("done")`, `Some("error")`, and so on otherwise. `seen_active` must be
/// true before `idle`/`done` finishes the job: an agent that has not started working yet is not a
/// completed sync.
pub fn transition(seen_active: bool, status: Option<&str>, age_secs: u64) -> Transition {
    if age_secs >= JOB_TIMEOUT_SECS {
        return Transition {
            state: JobState::Failed,
            seen_active,
            reason: Some(format!(
                "sync timed out after {} minutes",
                JOB_TIMEOUT_SECS / 60
            )),
        };
    }
    let Some(status) = status.map(str::trim).filter(|status| !status.is_empty()) else {
        return Transition {
            state: JobState::Failed,
            seen_active,
            reason: Some("agent is gone from its pane".to_string()),
        };
    };
    let status = status.to_ascii_lowercase();
    match status.as_str() {
        "working" | "blocked" => Transition {
            state: JobState::InFlight,
            seen_active: true,
            reason: None,
        },
        "idle" | "done" => {
            if seen_active {
                Transition {
                    state: JobState::Done,
                    seen_active: true,
                    reason: None,
                }
            } else {
                Transition {
                    state: JobState::InFlight,
                    seen_active: false,
                    reason: None,
                }
            }
        }
        "error" | "errored" | "failed" | "exited" | "stopped" | "crashed" | "gone" => Transition {
            state: JobState::Failed,
            seen_active,
            reason: Some(format!("agent reported status {status:?}")),
        },
        _ => Transition {
            state: JobState::InFlight,
            seen_active,
            reason: None,
        },
    }
}

/// A unique job id for this process. The service does not persist jobs, so process restarts forget
/// them; the timestamp keeps ids from colliding with a running older process.
pub fn new_job_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("sync-{millis:x}-{sequence:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn page(id: u64, name: &str) -> MapPage {
        MapPage {
            id,
            name: name.into(),
        }
    }

    fn node(
        id: u64,
        name: &str,
        text: &str,
        kind: &str,
        parent: Option<u64>,
        page: u64,
    ) -> MapNode {
        MapNode {
            id,
            name: name.into(),
            parent,
            page,
            kind: kind.into(),
            text: text.into(),
        }
    }

    fn snapshot(pages: Vec<MapPage>, nodes: Vec<MapNode>) -> MapSnapshot {
        MapSnapshot { pages, nodes }
    }

    fn small_map() -> MapSnapshot {
        snapshot(
            vec![page(1, "Mindmap")],
            vec![
                node(36, "heading", "", "frame", None, 1),
                node(37, "title", "The workspace client", "text", Some(36), 1),
            ],
        )
    }

    fn error_text(result: Result<MapSnapshot>) -> String {
        result
            .err()
            .map(|error| error.message().to_string())
            .unwrap_or_default()
    }

    #[test]
    fn parses_a_valid_snapshot() {
        let raw = json!({
            "pages": [{"id": 1, "name": "Mindmap"}],
            "nodes": [
                {"id": 36, "name": "heading", "parent": null, "page": 1, "kind": "frame", "text": ""},
                {"id": 37, "name": "title", "parent": 36, "page": 1, "kind": "text", "text": "The workspace client"}
            ],
            "ignored": true
        });
        assert_eq!(parse_snapshot(&raw).unwrap_or_default(), small_map());
        let node_without_parent = json!({
            "pages": [{"id": 1, "name": "Mindmap"}],
            "nodes": [{"id": 1, "name": "a", "page": 1, "kind": "text", "text": "b"}]
        });
        assert_eq!(
            parse_snapshot(&node_without_parent)
                .unwrap_or_default()
                .nodes[0]
                .parent,
            None
        );
    }

    #[test]
    fn rejects_malformed_snapshots() {
        let valid_node = json!({"id": 1, "name": "a", "page": 1, "kind": "text", "text": "b"});
        let valid_page = json!({"id": 1, "name": "Mindmap"});
        let cases = [
            (json!([]), "\"map\" must be an object"),
            (json!({"nodes": []}), "no \"pages\""),
            (json!({"pages": {}}), "map.pages"),
            (json!({"pages": [], "nodes": {}}), "map.nodes"),
            (
                json!({"pages": [], "nodes": [valid_node]}),
                "is not one of the map's page ids",
            ),
            (
                json!({"pages": [valid_page], "nodes": [{"id": 0, "name": "a", "page": 1, "kind": "text", "text": "b"}]}),
                "positive integer",
            ),
            (
                json!({"pages": [valid_page], "nodes": [{"id": -2, "name": "a", "page": 1, "kind": "text", "text": "b"}]}),
                "positive integer",
            ),
            (
                json!({"pages": [valid_page], "nodes": [{"id": "2", "name": "a", "page": 1, "kind": "text", "text": "b"}]}),
                "positive integer",
            ),
            (
                json!({"pages": [{"id": 0, "name": "p"}], "nodes": []}),
                "positive integer",
            ),
            (
                json!({"pages": [valid_page, valid_page], "nodes": []}),
                "duplicated",
            ),
            (
                json!({"pages": [valid_page], "nodes": [valid_node, valid_node]}),
                "duplicated",
            ),
            (
                json!({"pages": [valid_page], "nodes": [{"id": 1, "name": 3, "page": 1, "kind": "text", "text": "b"}]}),
                "name must be a string",
            ),
            (
                json!({"pages": [valid_page], "nodes": [{"id": 1, "name": "a", "page": 1, "kind": 4, "text": "b"}]}),
                "kind must be a string",
            ),
            (
                json!({"pages": [valid_page], "nodes": [{"id": 1, "name": "a", "page": 1, "kind": "text", "text": false}]}),
                "text must be a string",
            ),
            (
                json!({"pages": [valid_page], "nodes": [{"id": 1, "name": "a", "parent": "1", "page": 1, "kind": "text", "text": "b"}]}),
                "parent must be a positive integer",
            ),
            (
                json!({"pages": [valid_page], "nodes": [{"id": 1, "name": "a", "parent": 0, "page": 1, "kind": "text", "text": "b"}]}),
                "parent must be a positive integer",
            ),
            (
                json!({"pages": [valid_page], "nodes": [{"id": 1, "page": 1, "kind": "text", "text": "b"}]}),
                "name is missing",
            ),
        ];
        for (raw, needle) in cases {
            let error = error_text(parse_snapshot(&raw));
            assert!(error.contains(needle), "expected {needle:?} in {error:?}");
        }
    }

    #[test]
    fn rejects_oversized_snapshots() {
        let pages: Vec<Value> = (1..=MAX_MAP_PAGES + 1)
            .map(|id| json!({"id": id, "name": "p"}))
            .collect();
        let error = error_text(parse_snapshot(&json!({"pages": pages, "nodes": []})));
        assert!(error.contains("too many pages"), "{error}");

        let nodes: Vec<Value> = (1..=MAX_MAP_NODES + 1)
            .map(|id| json!({"id": id, "name": "a", "page": 1, "kind": "text", "text": "b"}))
            .collect();
        let error = error_text(parse_snapshot(
            &json!({"pages": [{"id": 1, "name": "p"}], "nodes": nodes}),
        ));
        assert!(error.contains("too many nodes"), "{error}");
    }

    #[test]
    fn first_sync_adds_every_node() {
        let diff = diff_maps(&small_map(), None);
        assert_eq!(
            diff.added,
            vec![
                MapCard {
                    id: 36,
                    name: "heading".into(),
                    page: "Mindmap".into(),
                },
                MapCard {
                    id: 37,
                    name: "title".into(),
                    page: "Mindmap".into(),
                },
            ]
        );
        assert!(diff.removed.is_empty());
        assert!(diff.changed.is_empty());
    }

    #[test]
    fn diff_reports_added_removed_and_changed() {
        let previous = snapshot(
            vec![page(1, "Mindmap")],
            vec![
                node(3, "Components", "old text", "frame", None, 1),
                node(7, "Old card", "same", "text", None, 1),
                node(8, "Stable", "same", "text", None, 1),
            ],
        );
        let current = snapshot(
            vec![page(1, "Mindmap")],
            vec![
                node(3, "Components", "new text", "frame", None, 1),
                node(8, "Stable", "same", "text", None, 1),
                node(9, "New card", "", "text", None, 1),
            ],
        );
        let diff = diff_maps(&current, Some(&previous));
        assert_eq!(
            diff.added,
            vec![MapCard {
                id: 9,
                name: "New card".into(),
                page: "Mindmap".into(),
            }]
        );
        assert_eq!(
            diff.removed,
            vec![MapCard {
                id: 7,
                name: "Old card".into(),
                page: "Mindmap".into(),
            }]
        );
        assert_eq!(
            diff.changed,
            vec![MapChange {
                id: 3,
                name: "Components".into(),
                fields: vec!["text".into()],
            }]
        );
    }

    #[test]
    fn diff_names_every_changed_field_in_order() {
        let previous = snapshot(
            vec![page(1, "Mindmap")],
            vec![node(3, "Components", "old", "frame", None, 1)],
        );
        let current = snapshot(
            vec![page(1, "Mindmap")],
            vec![node(3, "Components", "new", "text", Some(2), 2)],
        );
        let diff = diff_maps(&current, Some(&previous));
        assert_eq!(
            diff.changed[0].fields,
            vec!["text", "kind", "parent", "page"]
        );

        let renamed = snapshot(
            vec![page(1, "Mindmap")],
            vec![node(3, "Parts", "old", "frame", None, 1)],
        );
        assert_eq!(
            diff_maps(&renamed, Some(&previous)).changed[0].fields,
            vec!["name"]
        );
    }

    #[test]
    fn removed_cards_keep_the_previous_page_label() {
        let previous = snapshot(
            vec![page(1, "Old page")],
            vec![node(7, "Old card", "", "text", None, 1)],
        );
        let current = snapshot(vec![page(1, "New page")], Vec::new());
        let diff = diff_maps(&current, Some(&previous));
        assert_eq!(diff.removed[0].page, "Old page");
    }

    #[test]
    fn a_first_sync_prompt_has_no_code_section() {
        let map = diff_maps(&small_map(), None);
        let prompt = format_prompt(
            "mindmap",
            Some("/root/code/mindmap"),
            &map,
            &CodeDiff::default(),
            false,
            "",
        );
        assert_eq!(
            prompt,
            "Sync mindmap (/root/code/mindmap) against the code.\n\
             \n\
             DESIGN CHANGES SINCE first sync:\n\
             + heading (Mindmap)\n\
             + title (Mindmap)"
        );
    }

    #[test]
    fn formats_the_full_sync_prompt() {
        let map = MapDiff {
            added: vec![MapCard {
                id: 9,
                name: "New card".into(),
                page: "Mindmap".into(),
            }],
            removed: vec![MapCard {
                id: 7,
                name: "Old card".into(),
                page: "Mindmap".into(),
            }],
            changed: vec![MapChange {
                id: 3,
                name: "Components".into(),
                fields: vec!["name".into(), "text".into()],
            }],
        };
        let code = CodeDiff {
            head: Some("head1234".into()),
            from: Some("abc1234".into()),
            files: vec![FileDelta {
                path: "crates/x".into(),
                added: 10,
                removed: 2,
            }],
            commits: vec![Commit {
                hash: "abc1234".into(),
                subject: "Fix the map".into(),
            }],
            note: None,
        };
        assert_eq!(
            format_prompt(
                "mindmap",
                Some("/root/code/mindmap"),
                &map,
                &code,
                true,
                " optional note "
            ),
            "Sync mindmap (/root/code/mindmap) against the code.\n\
             \n\
             DESIGN CHANGES SINCE last sync:\n\
             + New card (Mindmap)\n\
             ~ Components (name, text)\n\
             - Old card (Mindmap)\n\
             \n\
             CODE SINCE abc1234:\n\
             1 files, 1 commits\n\
             \x20\x20crates/x (+10−2)\n\
             \x20\x20abc1234 Fix the map\n\
             \n\
             NOTE: optional note"
        );
    }

    #[test]
    fn prompt_omits_empty_sections() {
        let prompt = format_prompt(
            "mindmap",
            None,
            &MapDiff::default(),
            &CodeDiff::default(),
            true,
            "Only this note.",
        );
        assert_eq!(
            prompt,
            "Sync mindmap against the code.\n\nNOTE: Only this note."
        );
        assert_eq!(
            format_prompt(
                "mindmap",
                None,
                &MapDiff::default(),
                &CodeDiff::default(),
                false,
                ""
            ),
            "Sync mindmap against the code."
        );
    }

    #[test]
    fn prompt_caps_long_lists_and_keeps_the_note() {
        let mut map = MapDiff::default();
        for id in 1..=MAX_MAP_NODES as u64 {
            map.added.push(MapCard {
                id,
                name: "a card with a fairly long name ".repeat(8),
                page: "Mindmap".into(),
            });
        }
        let mut code = CodeDiff::default();
        for index in 0..MAX_GIT_FILES {
            code.files.push(FileDelta {
                path: format!("crates/some-long-directory/file-{index}.rs"),
                added: index as u64,
                removed: 2,
            });
        }
        let note = "keep this note ".repeat(2_000);
        let prompt = format_prompt("mindmap", Some("/repo"), &map, &code, true, &note);
        assert!(
            prompt.chars().count() <= MAX_PROMPT_CHARS,
            "prompt has {} chars",
            prompt.chars().count()
        );
        assert!(prompt.contains("… and"), "prompt lost its omission marker");
        assert!(prompt.contains("NOTE: keep this note"));
        assert!(prompt.contains("CODE SINCE unknown:"));
    }

    #[test]
    fn prompt_escapes_control_characters_in_names() {
        let map = MapDiff {
            added: vec![MapCard {
                id: 1,
                name: "line\nname".into(),
                page: "page\tone".into(),
            }],
            removed: Vec::new(),
            changed: Vec::new(),
        };
        let prompt = format_prompt("mindmap", None, &map, &CodeDiff::default(), false, "");
        assert!(prompt.contains("+ line\\nname (page\\tone)"), "{prompt}");
        assert_eq!(prompt.lines().count(), 4);
    }

    #[test]
    fn transitions_track_the_agent_lifecycle() {
        assert_eq!(
            transition(false, Some("working"), 0),
            Transition {
                state: JobState::InFlight,
                seen_active: true,
                reason: None,
            }
        );
        assert!(
            transition(false, Some("blocked"), 0).seen_active,
            "blocked means the agent is alive"
        );
        assert_eq!(
            transition(false, Some("idle"), 0).state,
            JobState::InFlight,
            "idle before the prompt is observed is not a finished sync"
        );
        assert_eq!(transition(true, Some("idle"), 0).state, JobState::Done);
        assert_eq!(transition(true, Some("done"), 0).state, JobState::Done);
        assert_eq!(
            transition(false, Some("unknown"), 0).state,
            JobState::InFlight
        );
    }

    #[test]
    fn transitions_fail_on_a_gone_or_errored_agent() {
        let gone = transition(true, None, 0);
        assert_eq!(gone.state, JobState::Failed);
        assert!(
            gone.reason.unwrap_or_default().contains("gone"),
            "reason should say the agent is gone"
        );

        let errored = transition(true, Some("error"), 0);
        assert_eq!(errored.state, JobState::Failed);
        assert!(errored.reason.unwrap_or_default().contains("\"error\""));
    }

    #[test]
    fn an_old_job_times_out() {
        let timed_out = transition(true, Some("working"), JOB_TIMEOUT_SECS);
        assert_eq!(timed_out.state, JobState::Failed);
        assert!(timed_out.reason.unwrap_or_default().contains("timed out"));
        assert_eq!(
            transition(false, Some("working"), JOB_TIMEOUT_SECS - 1).state,
            JobState::InFlight
        );
    }

    #[test]
    fn parses_git_numstat_and_log_output() {
        assert_eq!(
            parse_numstat("10\t2\tcrates/x\n-\t-\tassets/logo.png\n"),
            vec![
                FileDelta {
                    path: "crates/x".into(),
                    added: 10,
                    removed: 2,
                },
                FileDelta {
                    path: "assets/logo.png".into(),
                    added: 0,
                    removed: 0,
                },
            ]
        );
        assert_eq!(
            parse_log("abc1234\tFix the map\n9f8e7d6\tAdd a page\n"),
            vec![
                Commit {
                    hash: "abc1234".into(),
                    subject: "Fix the map".into(),
                },
                Commit {
                    hash: "9f8e7d6".into(),
                    subject: "Add a page".into(),
                },
            ]
        );
    }

    #[test]
    fn code_diff_without_a_path_is_a_note_not_an_error() {
        let diff = code_diff(None, Some("abc"));
        assert!(diff.head.is_none());
        assert_eq!(diff.from.as_deref(), Some("abc"));
        assert!(diff.files.is_empty());
        assert!(diff.commits.is_empty());
        assert!(diff.note.unwrap_or_default().contains("no checkout path"));
    }

    #[test]
    fn code_diff_outside_a_repository_is_a_note_not_an_error() {
        let dir = unique_dir("not-a-repo");
        std::fs::create_dir_all(&dir).unwrap_or(());
        let diff = code_diff(dir.to_str(), None);
        assert!(diff.head.is_none(), "expected no head in {dir:?}");
        assert!(diff.files.is_empty());
        assert!(diff.commits.is_empty());
        assert!(diff.note.is_some(), "expected a note");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn code_diff_reports_real_git_history() {
        let dir = unique_dir("git-repo");
        if !git_ok(&dir, &["init", "-q"]) {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let _ = git_ok(&dir, &["config", "user.email", "test@example.invalid"]);
        let _ = git_ok(&dir, &["config", "user.name", "Mindmap Test"]);
        std::fs::write(dir.join("file.txt"), "one\n").unwrap_or(());
        let _ = git_ok(&dir, &["add", "file.txt"]);
        let _ = git_ok(&dir, &["commit", "-q", "-m", "first"]);
        let first = git_stdout(&dir, &["rev-parse", "HEAD"]);
        std::fs::write(dir.join("file.txt"), "one\ntwo\n").unwrap_or(());
        let _ = git_ok(&dir, &["add", "file.txt"]);
        let _ = git_ok(&dir, &["commit", "-q", "-m", "second"]);
        let head = git_stdout(&dir, &["rev-parse", "HEAD"]);

        let diff = code_diff(dir.to_str(), Some(&first));
        assert_eq!(diff.head.as_deref(), Some(head.as_str()));
        assert_eq!(diff.from.as_deref(), Some(first.as_str()));
        assert_eq!(diff.files.len(), 1);
        assert_eq!(diff.files[0].path, "file.txt");
        assert_eq!(diff.files[0].added, 1);
        assert_eq!(diff.files[0].removed, 0);
        assert_eq!(diff.commits.len(), 1);
        assert_eq!(diff.commits[0].subject, "second");

        let first_sync = code_diff(dir.to_str(), None);
        assert_eq!(first_sync.head.as_deref(), Some(head.as_str()));
        assert!(first_sync.files.is_empty());
        assert!(first_sync.commits.is_empty());
        assert!(first_sync.note.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn git_ok(dir: &std::path::Path, args: &[&str]) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    fn git_stdout(dir: &std::path::Path, args: &[&str]) -> String {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .unwrap_or_default()
    }

    fn unique_dir(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "mindmap-sync-test-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ))
    }

    #[test]
    fn job_ids_are_unique() {
        assert_ne!(new_job_id(), new_job_id());
    }

    #[test]
    fn last_json_reports_existence_and_metadata() {
        let last = LastSync {
            commit: "abc".into(),
            at: "2026-01-01T00:00:00Z".into(),
            map: small_map(),
        };
        assert_eq!(
            last_json(Some(&last)),
            json!({"exists": true, "commit": "abc", "at": "2026-01-01T00:00:00Z"})
        );
        assert_eq!(last_json(None), json!({"exists": false}));
    }
}
