//! What an agent is told around a comment, Init or Sync: the state of the map (what is recorded,
//! what waits), how to decide what kind of work a comment is, and how to finish each kind.
//!
//! A comment is an explanation (record the map, no code), a bug fix (record the map with the
//! code) or a design edit (change only the map; it waits for Sync). A record commits the last
//! recorded map plus only that request's edits, so a Sync receives exactly the design edits that
//! still wait. The context packet (`node_context`) says where; this says what to do.

use serde_json::Value;

use crate::node_git::{Baseline, Drift};

/// The most cards listed per group (added, changed, removed) before `… and N more`.
const MAX_LISTED: usize = 12;

/// Where the request sits: its id and what is recorded and waiting in its scope.
pub struct State<'a> {
    pub request: &'a str,
    pub page: Option<u64>,
    /// The design waiting in scope, or `None` when the node could not compare (a new page).
    pub drift: Option<&'a Drift>,
}

/// `REQUEST`: the request id, the scope's last record and the design edits waiting in it.
pub fn request_section(state: &State<'_>) -> String {
    let mut out = format!(
        "REQUEST\nRequest: {} (Mindmap kept the map as it was when this was sent)\n",
        state.request
    );
    let scope = scope_name(state.page);
    match state.drift {
        None => out.push_str(&format!(
            "{scope} has no record yet: it describes nothing recorded, so Sync would treat every card as new design.\n"
        )),
        Some(drift) => match &drift.last {
            None => out.push_str(&format!(
                "{scope} has never been recorded: Sync would treat every card as new design. Recording an explanation marks it all as describing today's code; do that only if it does.\n"
            )),
            Some(last) => {
                out.push_str(&format!("Last record of {}: {}\n", lower(&scope), record_line(last)));
                if drift.is_empty() {
                    out.push_str("Nothing waits for Sync here: every card describes the recorded state.\n");
                } else {
                    out.push_str(
                        "Waiting for Sync (design edits since that record; proposals, not what the code does today):\n",
                    );
                    out.push_str(&cards(drift));
                }
            }
        },
    }
    out
}

/// `HOW TO HANDLE THIS REQUEST`: decide the kind of work, then finish it the way that kind is
/// finished. Init is one big explanation.
pub fn comment_brief(request: &str, page: Option<u64>, init: bool) -> String {
    let page = page.map_or_else(String::new, |p| format!(",\"page\":{p}"));
    let record = |kind: &str| {
        format!("{{\"method\":\"record\",\"request\":\"{request}\",\"kind\":\"{kind}\"{page}}}")
    };
    if init {
        return format!(
            "HOW TO FINISH INIT\n\
             Init describes the code as it is: it is one big explanation. Draw the map from the code; source every claim (file:line). Change no code.\n\
             When the map is complete, record it so Sync does not treat the existing code as new design:\n\
             1. node_request {}\n\
             2. Commit only the staged .mindmap/map.figcraft, with the returned trailer as the last line of the message.\n\
             3. node_request {{\"method\":\"sync.finish\",\"job\":\"<job>\",\"commit\":\"<sha>\"}}\n",
            record("explain")
        );
    }
    format!(
        "HOW TO HANDLE THIS REQUEST\n\
         Decide what kind of work it is before changing anything, and start your reply with one line: \
         \"Kind: explanation\", \"Kind: bug fix\", \"Kind: design edit\" or \"Kind: bug fix + design edit\".\n\
         - Explanation: a question, or the map should describe how the app works today. Read the code; change only the map, \
         only as asked, and source every claim (file:line). Never change code. Then record it as \"explain\".\n\
         - Bug fix: the app is broken today. Fix and test the code, and correct any card that described the bug wrongly. \
         Then record it as \"fix\".\n\
         - Design edit: the request proposes behaviour the code does not have. Change only the map and label new or changed \
         cards as proposals. Do not change code, record or commit: the edit waits for Sync.\n\
         - Unsure: it is a design edit (it waits instead of disappearing). Mixed: do and record the fix, and leave the \
         proposal as a waiting design edit.\n\
         \n\
         RECORD (explanation or bug fix only), after your last map edit for this request:\n\
         1. node_request {} (or \"fix\"). Mindmap builds the map to commit: the last recorded map plus only this \
         request's edits; design edits that were waiting stay out. It stages that map and returns a job and a trailer. \
         If it lists cards kept waiting (your edit overlaps a waiting edit), say so in your reply.\n\
         2. Commit: an explanation commits only the staged .mindmap/map.figcraft; a fix commits its code with it. The \
         returned trailer is the last line of the message.\n\
         3. node_request {{\"method\":\"sync.finish\",\"job\":\"<job>\",\"commit\":\"<sha>\"}}\n\
         If you cannot commit, call {{\"method\":\"sync.cancel\",\"job\":\"<job>\"}}: an open record blocks Sync.\n",
        record("explain")
    )
}

/// Everything a Sync needs to start without searching: what to build, what is context, and how
/// to finish.
pub struct SyncPrompt<'a> {
    pub workspace: &'a str,
    pub job: &'a str,
    pub page: Option<u64>,
    pub page_name: Option<&'a str>,
    pub context: &'a str,
    pub drift: &'a Drift,
    pub last_sync: Option<&'a Baseline>,
    /// `{files, commits}` since the last Sync.
    pub code: &'a Value,
    pub trailer: &'a str,
    pub note: &'a str,
}

pub fn sync_prompt(sync: &SyncPrompt<'_>) -> String {
    let scope = match (sync.page, sync.page_name) {
        (Some(id), Some(name)) => format!("page {name} (id {id})"),
        (Some(id), None) => format!("page {id}"),
        _ => "the whole map".to_owned(),
    };
    let mut out = format!("SYNC · {} · {scope} · job {}\n\n", sync.workspace, sync.job);
    out.push_str(sync.context.trim());
    out.push_str("\n\nWHAT TO BUILD\n");
    match &sync.drift.last {
        Some(last) => out.push_str(&format!(
            "Design baseline: {}. Cards not listed below already describe the code.\n",
            record_line(last)
        )),
        None => out.push_str(
            "No record yet: every card counts as added. Much of it may already exist in the code: check before building, and build only what is missing.\n",
        ),
    }
    if sync.drift.is_empty() {
        out.push_str("Nothing is waiting: tell the user, and call sync.cancel for this job.\n");
    } else {
        out.push_str("Design changes waiting since then (implement these):\n");
        out.push_str(&cards(sync.drift));
    }
    let page = sync
        .page
        .map_or_else(String::new, |p| format!(",\"page\":{p}"));
    out.push_str(&code_lines(sync.code, sync.last_sync));
    out.push_str(&format!(
        "Full detail: node_request {{\"method\":\"diff\"{page},\"offset\":0,\"limit\":200}} (follow pagination) and git from the last Sync.\n"
    ));
    out.push_str(
        "\nHOW TO WORK\n\
         Implement only the waiting design changes. If one turns out to describe what the code already does, change no code for it and say so.\n\
         Large work: plan it and split it across sub-agents in your own harness; finish all authorized work.\n",
    );
    out.push_str(&format!(
        "\nFINISH\n\
         1. node_request {{\"method\":\"sync.prepare\",\"job\":\"{job}\"}}\n\
         2. Commit code and .mindmap/map.figcraft together, with this exact trailer as the last line of the message:\n\
         {trailer}\n\
         3. node_request {{\"method\":\"sync.finish\",\"job\":\"{job}\",\"commit\":\"<sha>\"}}\n\
         Agent idle alone cannot finish this sync: only the verified commit does. Design edits made after it started wait for the next one. Do not commit node cache or private links.\n",
        job = sync.job,
        trailer = sync.trailer,
    ));
    let note = sync.note.trim();
    if !note.is_empty() {
        out.push_str("\nNOTE FROM THE USER\n");
        out.push_str(note);
        out.push('\n');
    }
    out
}

fn scope_name(page: Option<u64>) -> String {
    match page {
        Some(_) => "This page".to_owned(),
        None => "The map".to_owned(),
    }
}

fn lower(scope: &str) -> String {
    let mut chars = scope.chars();
    chars
        .next()
        .map(|c| c.to_lowercase().chain(chars).collect())
        .unwrap_or_default()
}

/// `abc1234 (an explanation, 2026-10-26)`.
fn record_line(last: &Baseline) -> String {
    let kind = match last.kind.as_str() {
        "explain" => "an explanation",
        "fix" => "a bug fix",
        _ => "a Sync",
    };
    let short: String = last.commit.chars().take(7).collect();
    let day: String = last.at.chars().take(10).collect();
    format!("{short} ({kind}, {day})")
}

/// `  Added: A (id 1), B (id 2)` / `  Changed: C (id 3): text, fills` / `  Removed: …`.
fn cards(drift: &Drift) -> String {
    let mut out = String::new();
    let name = |card: &Value| {
        let name = card["name"].as_str().unwrap_or("").trim();
        let name = if name.is_empty() { "unnamed" } else { name };
        format!("{name} (id {})", card["id"])
    };
    for (label, list, with_fields) in [
        ("Added", &drift.added, false),
        ("Changed", &drift.changed, true),
        ("Removed", &drift.removed, false),
    ] {
        // A new or removed card lists once: the layers inside it come with it.
        let ids: std::collections::BTreeSet<u64> =
            list.iter().filter_map(|c| c["id"].as_u64()).collect();
        let list: Vec<&Value> = list
            .iter()
            .filter(|c| with_fields || c["parent"].as_u64().is_none_or(|p| !ids.contains(&p)))
            .collect();
        if list.is_empty() {
            continue;
        }
        let shown: Vec<String> = list
            .iter()
            .copied()
            .take(MAX_LISTED)
            .map(|card| {
                let fields: Vec<&str> = card["fields"]
                    .as_array()
                    .map(|f| f.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                if with_fields && !fields.is_empty() {
                    format!("{}: {}", name(card), fields.join(", "))
                } else {
                    name(card)
                }
            })
            .collect();
        let more = list.len().saturating_sub(MAX_LISTED);
        let tail = if more > 0 {
            format!(" … and {more} more")
        } else {
            String::new()
        };
        out.push_str(&format!("  {label}: {}{tail}\n", shown.join("; ")));
    }
    let global = drift.changes.iter().filter(|c| c.node().is_none()).count();
    if global > 0 {
        out.push_str(&format!(
            "  Plus {global} document-level change{} (styles, variables, comments).\n",
            if global == 1 { "" } else { "s" }
        ));
    }
    out
}

/// `Code since the last Sync abc1234 (context only, already done): …`.
fn code_lines(code: &Value, last_sync: Option<&Baseline>) -> String {
    let commits = code["commits"].as_array().map_or(&[][..], Vec::as_slice);
    let files = code["files"].as_array().map_or(0, Vec::len);
    let since = last_sync.map_or_else(
        || "No Sync yet: the whole repository is new code".to_owned(),
        |b| {
            format!(
                "Code since the last Sync {}",
                b.commit.chars().take(7).collect::<String>()
            )
        },
    );
    if last_sync.is_none() {
        return format!("{since} ({files} files). Read it as context, not as work.\n");
    }
    if commits.is_empty() && files == 0 {
        return format!("{since}: nothing.\n");
    }
    let mut out = format!(
        "{since} (context only, already done): {} commit{}, {files} file{} changed.\n",
        commits.len(),
        if commits.len() == 1 { "" } else { "s" },
        if files == 1 { "" } else { "s" }
    );
    for commit in commits.iter().take(MAX_LISTED) {
        out.push_str(&format!(
            "  {} {}\n",
            commit["hash"].as_str().unwrap_or(""),
            commit["subject"].as_str().unwrap_or("")
        ));
    }
    if commits.len() > MAX_LISTED {
        out.push_str(&format!("  … and {} more\n", commits.len() - MAX_LISTED));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn last(kind: &str) -> Baseline {
        Baseline {
            commit: "0123456789abcdef".into(),
            at: "2026-10-26T10:00:00+01:00".into(),
            page: Some(7),
            trailer: "Mindmap-Sync: page=7 job=x".into(),
            kind: kind.into(),
        }
    }

    fn drift(last: Option<Baseline>, added: &[&str]) -> Drift {
        Drift {
            last,
            changes: Vec::new(),
            added: added
                .iter()
                .enumerate()
                .map(|(i, n)| json!({"id": i + 10, "name": n}))
                .collect(),
            changed: vec![json!({"id": 3, "name": "Status", "fields": ["characters", "fills"]})],
            removed: Vec::new(),
        }
    }

    #[test]
    fn a_comment_names_its_request_what_is_recorded_and_what_waits() {
        let d = drift(Some(last("explain")), &["Retry"]);
        let text = request_section(&State {
            request: "r1",
            page: Some(7),
            drift: Some(&d),
        });
        assert!(text.contains("Request: r1"), "{text}");
        assert!(
            text.contains("Last record of this page: 0123456 (an explanation, 2026-10-26)"),
            "{text}"
        );
        assert!(text.contains("Added: Retry (id 10)"), "{text}");
        assert!(
            text.contains("Changed: Status (id 3): characters, fills"),
            "{text}"
        );
        let never = drift(None, &["A"]);
        let text = request_section(&State {
            request: "r1",
            page: Some(7),
            drift: Some(&never),
        });
        assert!(text.contains("never been recorded"), "{text}");
        assert!(
            !text.contains("Added"),
            "a never-recorded page is not listed card by card: {text}"
        );
    }

    #[test]
    fn the_brief_lets_the_agent_choose_and_finish_each_kind() {
        let text = comment_brief("r1", Some(7), false);
        for kind in ["Kind: explanation", "Kind: bug fix", "Kind: design edit"] {
            assert!(text.contains(kind), "{text}");
        }
        assert!(
            text.contains(r#"{"method":"record","request":"r1","kind":"explain","page":7}"#),
            "{text}"
        );
        assert!(
            text.contains("sync.finish") && text.contains("sync.cancel"),
            "{text}"
        );
        let init = comment_brief("r2", None, true);
        assert!(
            init.contains(r#"{"method":"record","request":"r2","kind":"explain"}"#),
            "{init}"
        );
        assert!(init.contains("one big explanation"), "{init}");
    }

    #[test]
    fn a_sync_lists_the_work_the_context_code_and_the_finish() {
        let d = drift(Some(last("fix")), &["Retry"]);
        let code = json!({"files": [{"path": "a.rs"}], "commits": [{"hash": "abc1234", "subject": "Fix status"}]});
        let sync = last("sync");
        let text = sync_prompt(&SyncPrompt {
            workspace: "mindmap",
            job: "j1",
            page: Some(7),
            page_name: Some("Overview"),
            context: "MINDMAP CONTEXT",
            drift: &d,
            last_sync: Some(&sync),
            code: &code,
            trailer: "Mindmap-Sync: page=7 job=j1",
            note: "ship it",
        });
        assert!(
            text.starts_with("SYNC · mindmap · page Overview (id 7) · job j1"),
            "{text}"
        );
        assert!(
            text.contains("Design baseline: 0123456 (a bug fix, 2026-10-26)"),
            "{text}"
        );
        assert!(text.contains("Added: Retry (id 10)"), "{text}");
        assert!(
            text.contains("(context only, already done): 1 commit, 1 file changed."),
            "{text}"
        );
        assert!(text.contains("  abc1234 Fix status"), "{text}");
        assert!(text.contains("\nMindmap-Sync: page=7 job=j1\n"), "{text}");
        assert!(
            text.trim_end().ends_with("NOTE FROM THE USER\nship it"),
            "{text}"
        );
        let empty = Drift {
            last: Some(last("sync")),
            changes: Vec::new(),
            added: Vec::new(),
            changed: Vec::new(),
            removed: Vec::new(),
        };
        let text = sync_prompt(&SyncPrompt {
            drift: &empty,
            note: "",
            ..SyncPrompt {
                workspace: "w",
                job: "j",
                page: None,
                page_name: None,
                context: "",
                drift: &d,
                last_sync: None,
                code: &json!({}),
                trailer: "t",
                note: "",
            }
        });
        assert!(text.contains("Nothing is waiting"), "{text}");
        assert!(!text.contains("NOTE FROM THE USER"), "{text}");
    }
}
