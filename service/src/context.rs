//! The context packet sent with comments and new-page prompts.
//!
//! A comment is not enough: the agent must know the workspace, the live document, the page, the
//! selected card, its path, text and children, and how to edit the map. This module joins the
//! document read-only through the FigCraft CLI, reads the page/node JSON, and renders a bounded
//! plain-text header. Failures degrade to the static workspace/document context; they never block
//! the comment from being delivered.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::config::Config;
use crate::error::Result;
use crate::mcp::FigcraftSession;

/// The most context characters sent before the comments.
pub const MAX_CONTEXT_CHARS: usize = 8_000;
/// The most characters of one card body included.
const MAX_CARD_TEXT: usize = 900;
/// How long to wait for the collaboration session to become live.
const JOIN_TIMEOUT: Duration = Duration::from_secs(5);

/// What the caller knows before reading the document.
pub struct Request<'a> {
    /// Herdr workspace label.
    pub label: &'a str,
    /// Herdr workspace id.
    pub id: &'a str,
    /// Checkout path.
    pub path: Option<&'a str>,
    /// FigCraft file id.
    pub file: Option<&'a str>,
    /// Owner edit link, used only to join; never rendered.
    pub edit: Option<&'a str>,
    /// Page the comments/prompt belong to.
    pub page_id: Option<u64>,
    /// The page was created for this prompt.
    pub new_page: bool,
    /// The component page a new page was generated from.
    pub source_page_id: Option<u64>,
    /// The component node a new page was generated from.
    pub source_node_id: Option<u64>,
    /// Nodes the comments are attached to.
    pub node_ids: &'a [u64],
}

/// Build the context header. It never includes the edit link or its token.
pub fn build(config: &Config, request: &Request<'_>) -> String {
    let mut out = String::new();
    out.push_str("MINDMAP CONTEXT\n\n");
    out.push_str(&format!(
        "Workspace: {} ({})\n",
        inline(request.label),
        inline(request.id)
    ));
    if let Some(path) = request.path.filter(|path| !path.trim().is_empty()) {
        out.push_str(&format!("Repository: {}\n", inline(path)));
    }
    if let Some(file) = request.file.filter(|file| !file.trim().is_empty()) {
        out.push_str(&format!("Document: {}\n", inline(file)));
    }
    if let Some(edit) = request.edit.filter(|edit| !edit.trim().is_empty()) {
        out.push_str(&format!(
            "Edit link (owner access; do not share, commit or paste): {}\n",
            inline(edit)
        ));
    }
    out.push_str(&format!(
        "FigCraft CLI: {}\n",
        config.figcraft_cli.display()
    ));
    out.push_str("The live FigCraft document is the source of truth. The map is read by a human; the human does not edit it.\n");
    out.push_str(&format!(
        "Card conventions: read {}; use the FigCraft MCP tools on the live document. The renderer command is not required; edit the live page directly.\n",
        config.skill_file.display()
    ));

    match inspect(config, request) {
        Ok(details) => {
            out.push_str(&details);
        }
        Err(error) => {
            out.push_str("Document context: unavailable (");
            out.push_str(&one_line(&error.to_string()));
            out.push_str("). Use the workspace and the page name above.\n");
        }
    }
    out.push_str("\nHOW TO WORK\n");
    out.push_str("Edit the live document, not the client UI or a static copy. Work on the named page; keep card names stable; source every claim. To edit: run the FigCraft CLI with `mcp`, `collab.join` the edit link above, `get_node` the page and card ids to read their current JSON, then use `run_command` to change or add the cards. Inspect before changing, and do not touch unrelated pages.\n");
    truncate(out, MAX_CONTEXT_CHARS)
}

fn inspect(config: &Config, request: &Request<'_>) -> Result<String> {
    let edit = request
        .edit
        .filter(|edit| !edit.trim().is_empty())
        .ok_or_else(|| crate::error::Error::new("no edit link recorded"))?;
    let mut session = FigcraftSession::start(&config.figcraft_cli)?;
    session.run_command("collab.join", json!({"link": edit}))?;
    let started = Instant::now();
    while started.elapsed() < JOIN_TIMEOUT {
        let live = session
            .run_command("collab.peers", json!({}))
            .ok()
            .and_then(|peers| {
                peers
                    .get("status")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .is_some_and(|status| status == "live");
        if live {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let pages = session.run_command("page.list", json!({}))?;
    let page_id = request
        .page_id
        .or_else(|| current_page(&pages))
        .ok_or_else(|| crate::error::Error::new("the document has no page"))?;
    let page = session.call_tool("get_node", json!({"id": page_id, "depth": 3}))?;
    if page.is_null() {
        return Err(crate::error::Error::new(format!("no page {page_id}")));
    }
    let mut out = String::new();
    let page_name = page
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("untitled");
    out.push_str(&format!("\nPage: {} (id {page_id})\n", inline(page_name)));
    if request.new_page {
        out.push_str("This page was just created for the prompt below.\n");
    }
    let page_cards = card_names(&page);
    if !page_cards.is_empty() {
        out.push_str(&format!("Top-level cards: {page_cards}\n"));
    }
    if let (Some(source_page_id), Some(source_node_id)) =
        (request.source_page_id, request.source_node_id)
    {
        if let Ok(source_page) =
            session.call_tool("get_node", json!({"id": source_page_id, "depth": 3}))
        {
            if let Some((node, path)) = find(&source_page, source_node_id, &mut Vec::new()) {
                out.push_str(&format!(
                    "\nSource page: {} (id {source_page_id})\n",
                    label(&source_page)
                ));
                out.push_str(&format!(
                    "Source component: {} (id {source_node_id})\n",
                    label(node)
                ));
                out.push_str(&format!("Source path: {}\n", path.join(" > ")));
                out.push_str(&format!(
                    "Source type: {}\n",
                    node.get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                ));
                let text = body(node);
                if !text.is_empty() {
                    out.push_str(&format!("Source text: {text}\n"));
                }
                let children = children(node);
                if !children.is_empty() {
                    out.push_str(&format!("Source children: {children}\n"));
                }
                out.push_str("The new page should explain this component in detail.\n");
            }
        }
    }
    let mut seen = Vec::new();
    for id in request.node_ids {
        if *id == 0 || seen.contains(id) {
            continue;
        }
        seen.push(*id);
        match find(&page, *id, &mut Vec::new()) {
            Some((node, path)) => {
                out.push_str(&format!("\nCommented node: {} (id {})\n", label(node), id));
                out.push_str(&format!("Path: {}\n", path.join(" > ")));
                out.push_str(&format!(
                    "Type: {}\n",
                    node.get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                ));
                let text = body(node);
                if !text.is_empty() {
                    out.push_str(&format!("Text: {}\n", text));
                }
                let children = children(node);
                if !children.is_empty() {
                    out.push_str(&format!("Children: {children}\n"));
                }
            }
            None => out.push_str(&format!(
                "\nCommented node: id {id} (not found in the page)\n"
            )),
        }
    }
    if request.node_ids.is_empty() && !request.new_page {
        out.push_str("Selection: the whole page.\n");
    }
    Ok(out)
}

fn current_page(pages: &Value) -> Option<u64> {
    pages
        .as_array()?
        .iter()
        .find(|page| {
            page.get("current")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .and_then(|page| page.get("id").and_then(Value::as_u64))
}

fn find<'a>(node: &'a Value, id: u64, path: &mut Vec<String>) -> Option<(&'a Value, Vec<String>)> {
    path.push(label(node));
    if node.get("id").and_then(Value::as_u64) == Some(id) {
        return Some((node, path.clone()));
    }
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            if let Some(found) = find(child, id, path) {
                return Some(found);
            }
        }
    }
    path.pop();
    None
}

fn label(node: &Value) -> String {
    node.get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("unnamed")
        .to_string()
}

fn card_names(page: &Value) -> String {
    let Some(children) = page.get("children").and_then(Value::as_array) else {
        return String::new();
    };
    let names: Vec<String> = children
        .iter()
        .filter(|node| {
            matches!(
                node.get("type").and_then(Value::as_str),
                Some("FRAME" | "COMPONENT" | "COMPONENT_SET" | "SECTION" | "GROUP" | "INSTANCE")
            )
        })
        .map(label)
        .filter(|name| name != "Page bounds")
        .take(20)
        .collect();
    names.join(", ")
}

fn children(node: &Value) -> String {
    let Some(children) = node.get("children").and_then(Value::as_array) else {
        return String::new();
    };
    children
        .iter()
        .map(|child| {
            format!(
                "{} ({})",
                label(child),
                child.get("type").and_then(Value::as_str).unwrap_or("?")
            )
        })
        .take(16)
        .collect::<Vec<_>>()
        .join(", ")
}

fn body(node: &Value) -> String {
    let mut out = String::new();
    collect_text(node, &mut out, 3);
    one_line(&out)
}

fn collect_text(node: &Value, out: &mut String, depth: usize) {
    if depth == 0 || out.len() >= MAX_CARD_TEXT {
        return;
    }
    if let Some(text) = node.get("characters").and_then(Value::as_str) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(text);
    }
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            collect_text(child, out, depth - 1);
            if out.len() >= MAX_CARD_TEXT {
                break;
            }
        }
    }
}

fn inline(value: &str) -> String {
    one_line(value)
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate(mut text: String, limit: usize) -> String {
    if text.chars().count() > limit {
        text = text.chars().take(limit).collect();
        text.push_str("\n[context truncated]");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_context_names_the_workspace_and_document() {
        let config = Config {
            data_dir: "/tmp".into(),
            legacy_records: "/tmp/legacy.json".into(),
            empty_template: "/tmp/empty.figcraft".into(),
            key_file: "/tmp/server.key".into(),
            figcraft_cli: "/tmp/figcraft-cli".into(),
            herdr_bin: "herdr".into(),
            public_server: "host:1".into(),
            local_server: "ws://127.0.0.1:1/".into(),
            bind: "127.0.0.1:1".into(),
            skill_file: "/repo/skills/mindmap/SKILL.md".into(),
        };
        let request = Request {
            label: "mindmap",
            id: "wX",
            path: Some("/root/code/mindmap"),
            file: Some("abc"),
            edit: None,
            page_id: Some(7),
            new_page: true,
            source_page_id: None,
            source_node_id: None,
            node_ids: &[],
        };
        let text = build(&config, &request);
        assert!(text.contains("Workspace: mindmap (wX)"));
        assert!(text.contains("Repository: /root/code/mindmap"));
        assert!(text.contains("Document: abc"));
        assert!(text.contains("Document context: unavailable"));
        assert!(!text.contains("figcraft://"));
    }

    #[test]
    fn a_context_is_bounded_and_keeps_the_instruction() {
        let text = truncate("x".repeat(MAX_CONTEXT_CHARS + 100), MAX_CONTEXT_CHARS);
        assert!(text.ends_with("[context truncated]"));
        assert!(text.chars().count() <= MAX_CONTEXT_CHARS + 20);
    }

    #[test]
    fn node_text_and_path_are_read() {
        let page = json!({
            "id": 1, "name": "Page", "type": "PAGE", "children": [
                {"id": 2, "name": "Card", "type": "FRAME", "children": [
                    {"id": 3, "name": "body", "type": "TEXT", "characters": "hello world"}
                ]}
            ]
        });
        let (node, path) = find(&page, 3, &mut Vec::new()).unwrap();
        assert_eq!(path, vec!["Page", "Card", "body"]);
        assert_eq!(body(node), "hello world");
        assert_eq!(card_names(&page), "Card");
    }
}
