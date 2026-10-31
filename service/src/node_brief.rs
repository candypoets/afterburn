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

/// `HOW TO SHOW IT`: an explanation is drawn, not written; a design is its own picture.
const HOW_TO_SHOW: &str = "HOW TO SHOW IT\n\
     Explaining: draw it. A diagram, chart, flow or before/after, with as few words as possible; \
     words only label and source (file:line).\n\
     Designing: the design is the picture. Don't add a diagram about it.\n";

/// `HOW TO SHOW IT` and `HOW TO FINISH`: the kinds of work and their rules live in the mindmap
/// skill; the prompt keeps the reply's first line and the exact record command. Init is one big
/// explanation.
pub fn comment_brief(request: &str, page: Option<u64>, init: bool) -> String {
    let page = page.map_or_else(String::new, |p| format!(",\"page\":{p}"));
    let record = |kind: &str| {
        format!("{{\"method\":\"record\",\"request\":\"{request}\",\"kind\":\"{kind}\"{page}}}")
    };
    let finish =
        "commit what it stages with the returned trailer as the last line, then node_request \
         {\"method\":\"sync.finish\",\"job\":\"<job>\",\"commit\":\"<sha>\"}. \
         If you cannot commit: {\"method\":\"sync.cancel\",\"job\":\"<job>\"}.";
    if init {
        return format!(
            "{HOW_TO_SHOW}\n\
             HOW TO FINISH INIT\n\
             Init is one big explanation: draw the code as it is, source every claim (file:line), change no code.\n\
             When the map is complete: node_request {}, {finish}\n",
            record("explain")
        );
    }
    format!(
        "{HOW_TO_SHOW}\n\
         HOW TO FINISH\n\
         Start your reply with \"Kind: explanation\", \"Kind: bug fix\", \"Kind: design edit\" or \
         \"Kind: bug fix + design edit\"; the skill says what each kind may change.\n\
         Explanation or fix, after your last map edit: node_request {} (or \"fix\"), {finish}\n",
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
    fn the_brief_asks_for_pictures_and_keeps_the_kind_line_and_record_command() {
        let text = comment_brief("r1", Some(7), false);
        assert!(text.starts_with("HOW TO SHOW IT\n"), "{text}");
        assert!(
            text.contains("Explaining: draw it.")
                && text.contains("Designing: the design is the picture."),
            "{text}"
        );
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
        assert!(
            text.len() < 800,
            "the rules live in the skill: {} chars",
            text.len()
        );
        let init = comment_brief("r2", None, true);
        assert!(
            init.contains(r#"{"method":"record","request":"r2","kind":"explain"}"#),
            "{init}"
        );
        assert!(init.contains("one big explanation"), "{init}");
        assert!(init.contains("HOW TO SHOW IT"), "{init}");
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
