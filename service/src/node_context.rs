//! The context packet an agent receives with comments, new pages and Sync.
//!
//! It names the worktree, branch, node and session, then summarizes what the request points at:
//! the page and its subpages, the page's top-level cards, and each commented card with its path
//! and text. It never dumps raw document JSON; agents read details with `get_node`.

use figcraft_doc::{Document, NodeId, NodeKind};

/// The most characters of the whole packet.
pub const MAX_CONTEXT_CHARS: usize = 8_000;
/// The most characters of one card's text.
const MAX_CARD_TEXT: usize = 600;
/// The most top-level card names listed for a page.
const MAX_CARD_NAMES: usize = 40;
/// Plugin data key a subpage stores its parent page under (set by the client's page tree).
const PARENT_KEY: &str = "mindmap.parent_page";

/// What the caller knows about the request.
pub struct Request<'a> {
    pub worktree: &'a str,
    pub branch: &'a str,
    pub node: &'a str,
    pub session: &'a str,
    /// The page the request belongs to, when the client named one.
    pub page: Option<NodeId>,
    /// Cards the comments are attached to.
    pub commented: &'a [NodeId],
    /// Cards selected in the client when the prompt was sent: what the task is about.
    pub selected: &'a [NodeId],
    /// The page was created for this prompt.
    pub new_page: bool,
    /// The card a new page was made from.
    pub source: Option<NodeId>,
}

/// Render the packet. The page falls back to the page of the first commented card.
pub fn render(doc: &Document, request: &Request<'_>) -> String {
    let mut out = format!(
        "MINDMAP CONTEXT\nWorktree: {}\nBranch: {}\nNode: {}\nSession: {}\n\
         Read and edit the live session with `mindmap-node mcp --path <worktree>` (tools: inspect_document, get_node, \
         create_node, set_properties, run_command, screenshot, node_request) or `mindmap-node request '<JSON>'`. \
         Do not write .mindmap/map.figcraft: only the node saves it. Keep names stable and source claims. \
         Read skills/mindmap/SKILL.md for the node workflow.\n",
        one_line(request.worktree),
        one_line(request.branch),
        one_line(request.node),
        one_line(request.session),
    );
    let page = request
        .page
        .filter(|page| doc.pages.contains(page))
        .or_else(|| {
            request
                .selected
                .iter()
                .chain(request.commented)
                .find_map(|id| page_of(doc, *id))
        });
    if let Some(page) = page {
        let subpages = subpages(doc, page);
        out.push_str(&format!(
            "Scope: page {} and its subpages only. Other pages are out of scope.\n",
            page.0
        ));
        out.push_str(&format!("\nPage: {} (id {})\n", name(doc, page), page.0));
        if !subpages.is_empty() {
            let names: Vec<String> = subpages
                .iter()
                .map(|id| format!("{} (id {})", name(doc, *id), id.0))
                .collect();
            out.push_str(&format!("Subpages: {}\n", names.join(", ")));
        }
        if request.new_page {
            out.push_str("This page was just created for the prompt below.\n");
        }
        let cards: Vec<String> = doc
            .node(page)
            .map(|node| {
                node.children
                    .iter()
                    .take(MAX_CARD_NAMES)
                    .map(|id| name(doc, *id))
                    .collect()
            })
            .unwrap_or_default();
        if !cards.is_empty() {
            let more = doc
                .node(page)
                .map_or(0, |node| node.children.len().saturating_sub(MAX_CARD_NAMES));
            let tail = if more > 0 {
                format!(" … and {more} more")
            } else {
                String::new()
            };
            out.push_str(&format!("Top-level cards: {}{tail}\n", cards.join(", ")));
        }
    } else {
        out.push_str("Scope: the whole map; the request names no page.\n");
    }
    if let Some(source) = request.source.filter(|id| doc.node(*id).is_some()) {
        out.push_str("\nMade from this card:\n");
        out.push_str(&card(doc, source));
    }
    let selected: Vec<NodeId> = request
        .selected
        .iter()
        .copied()
        .filter(|id| doc.node(*id).is_some())
        .collect();
    if !selected.is_empty() {
        out.push_str("\nSelected cards (the task is about these):\n");
        for id in selected {
            out.push_str(&card(doc, id));
        }
    }
    let commented: Vec<NodeId> = request
        .commented
        .iter()
        .copied()
        .filter(|id| doc.node(*id).is_some())
        .collect();
    if !commented.is_empty() {
        out.push_str("\nCommented cards:\n");
        for id in commented {
            out.push_str(&card(doc, id));
        }
    }
    out.push_str("\nRead any card with get_node <id>; the full property diff comes from node_request {\"method\":\"diff\"} with pagination.\n");
    truncate_lines(out, MAX_CONTEXT_CHARS)
}

/// One card: name, id, type, path from the page, and its text.
fn card(doc: &Document, id: NodeId) -> String {
    let Some(node) = doc.node(id) else {
        return String::new();
    };
    let mut out = format!(
        "- {} (id {}, {})\n  Path: {}\n",
        one_line(&node.name),
        id.0,
        node.kind.type_name(),
        path(doc, id).join(" > ")
    );
    let text = text_of(doc, id);
    if !text.is_empty() {
        out.push_str(&format!(
            "  Text: {}\n",
            shorten(&one_line(&text), MAX_CARD_TEXT)
        ));
    }
    out
}

/// Names from the page down to `id`.
fn path(doc: &Document, id: NodeId) -> Vec<String> {
    let mut names = Vec::new();
    let mut current = Some(id);
    let mut steps = 0;
    while let Some(node_id) = current {
        let Some(node) = doc.node(node_id) else { break };
        names.push(one_line(&node.name));
        current = node.parent;
        steps += 1;
        if steps > 64 {
            break;
        }
    }
    names.reverse();
    names
}

/// The page that holds `id`, if any.
fn page_of(doc: &Document, id: NodeId) -> Option<NodeId> {
    let mut current = Some(id);
    let mut steps = 0;
    while let Some(node_id) = current {
        if doc.pages.contains(&node_id) {
            return Some(node_id);
        }
        current = doc.node(node_id)?.parent;
        steps += 1;
        if steps > 64 {
            return None;
        }
    }
    None
}

/// Every page below `page` in the page tree, in document order.
fn subpages(doc: &Document, page: NodeId) -> Vec<NodeId> {
    let parent_of = |id: NodeId| -> Option<NodeId> {
        doc.node(id)?
            .plugin_data
            .get(PARENT_KEY)?
            .parse::<u64>()
            .ok()
            .map(NodeId)
    };
    let mut found: Vec<NodeId> = vec![page];
    loop {
        let before = found.len();
        for id in &doc.pages {
            if !found.contains(id) && parent_of(*id).is_some_and(|parent| found.contains(&parent)) {
                found.push(*id);
            }
        }
        if found.len() == before {
            break;
        }
    }
    found.retain(|id| *id != page);
    found.sort_by_key(|id| doc.pages.iter().position(|p| p == id));
    found
}

/// Every text layer below `id`, joined.
fn text_of(doc: &Document, id: NodeId) -> String {
    let mut parts = Vec::new();
    let mut stack = vec![id];
    let mut seen = 0usize;
    while let Some(node_id) = stack.pop() {
        seen += 1;
        if seen > 2_000 {
            break;
        }
        let Some(node) = doc.node(node_id) else {
            continue;
        };
        if let NodeKind::Text(text) = &node.kind {
            let part = text.characters.trim();
            if !part.is_empty() {
                parts.push(part.to_string());
            }
        }
        for child in node.children.iter().rev() {
            stack.push(*child);
        }
    }
    parts.join(" · ")
}

fn name(doc: &Document, id: NodeId) -> String {
    doc.node(id)
        .map(|node| one_line(&node.name))
        .unwrap_or_else(|| format!("#{}", id.0))
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn shorten(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let mut out: String = value.chars().take(limit.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Keep whole lines up to `limit` characters, then say how much was left out.
fn truncate_lines(text: String, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text;
    }
    let mut out = String::new();
    let mut used = 0;
    let mut left = 0;
    for line in text.lines() {
        let cost = line.chars().count() + 1;
        if left == 0 && used + cost <= limit.saturating_sub(80) {
            out.push_str(line);
            out.push('\n');
            used += cost;
        } else {
            left += 1;
        }
    }
    out.push_str(&format!(
        "… {left} more lines left out; read them with get_node.\n"
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_engine::Session;
    use serde_json::json;

    fn id(value: &serde_json::Value) -> NodeId {
        figcraft_engine::params::id_of(&value["id"]).unwrap()
    }

    /// Page "Mindmap" first, then "backend code" with a card and a subpage "Remote setup".
    fn doc() -> (Document, NodeId, NodeId, NodeId) {
        let mut session = Session::new();
        let first = session.page().unwrap();
        session
            .execute(
                "frame.create",
                &json!({"name": "Card Overview", "width": 100, "height": 100}),
            )
            .unwrap();
        let backend = id(&session
            .execute("page.add", &json!({"name": "backend code"}))
            .unwrap());
        let card = id(&session
            .execute(
                "frame.create",
                &json!({"name": "Proposed system", "width": 300, "height": 200}),
            )
            .unwrap());
        session
            .execute("node.create", &json!({"type": "text", "parent": card.0, "name": "title", "text": "The app talks to nodes"}))
            .unwrap();
        let remote = id(&session
            .execute("page.add", &json!({"name": "Remote setup"}))
            .unwrap());
        session
            .execute(
                "node.set",
                &json!({"id": remote.0, "pluginData": {PARENT_KEY: backend.0.to_string()}}),
            )
            .unwrap();
        let _ = first;
        (session.doc().unwrap().clone(), backend, card, remote)
    }

    fn request<'a>(page: Option<NodeId>, commented: &'a [NodeId]) -> Request<'a> {
        Request {
            worktree: "/repo",
            branch: "main",
            node: "127.0.0.1:47822",
            session: "s1",
            page,
            commented,
            selected: &[],
            new_page: false,
            source: None,
        }
    }

    #[test]
    fn a_comment_names_its_page_card_path_and_text() {
        let (doc, backend, card, remote) = doc();
        let text = render(&doc, &request(None, &[card]));
        assert!(
            text.contains(&format!("Scope: page {} and its subpages only.", backend.0)),
            "{text}"
        );
        assert!(
            text.contains(&format!("Page: backend code (id {})", backend.0)),
            "{text}"
        );
        assert!(
            text.contains(&format!("Subpages: Remote setup (id {})", remote.0)),
            "{text}"
        );
        assert!(text.contains("Top-level cards: Proposed system"), "{text}");
        assert!(
            text.contains(&format!("- Proposed system (id {}", card.0)),
            "{text}"
        );
        assert!(
            text.contains("Path: backend code > Proposed system"),
            "{text}"
        );
        assert!(text.contains("Text: The app talks to nodes"), "{text}");
        assert!(
            !text.contains("Card Overview"),
            "another page leaked in: {text}"
        );
        assert!(!text.contains("absoluteBounds"), "no raw JSON: {text}");
    }

    #[test]
    fn a_prompt_names_the_selected_card_and_its_page() {
        let (doc, backend, card, _) = doc();
        let selected = [card];
        let text = render(
            &doc,
            &Request {
                selected: &selected,
                ..request(None, &[])
            },
        );
        assert!(
            text.contains(&format!("Scope: page {} and its subpages only.", backend.0)),
            "{text}"
        );
        assert!(
            text.contains(&format!(
                "Selected cards (the task is about these):\n- Proposed system (id {}",
                card.0
            )),
            "{text}"
        );
        assert!(!text.contains("Commented cards"), "{text}");
    }

    #[test]
    fn a_named_page_wins_and_no_page_says_so() {
        let (doc, backend, _, _) = doc();
        assert!(render(&doc, &request(Some(backend), &[])).contains("Page: backend code"));
        assert!(render(&doc, &request(None, &[])).contains("Scope: the whole map"));
        // An unknown page id falls back instead of naming a page that does not exist.
        assert!(render(&doc, &request(Some(NodeId(999_999)), &[])).contains("Scope: the whole map"));
    }

    #[test]
    fn long_packets_keep_whole_lines() {
        let text = (0..400)
            .map(|i| format!("line {i} {}", "x".repeat(40)))
            .collect::<Vec<_>>()
            .join("\n");
        let cut = truncate_lines(text, 2_000);
        assert!(cut.chars().count() <= 2_000);
        assert!(cut.ends_with("read them with get_node.\n"));
        assert!(cut
            .lines()
            .all(|line| line.starts_with("line ") || line.starts_with('…')));
    }
}
