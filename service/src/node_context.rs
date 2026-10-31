//! The context packet an agent receives with comments, new pages and Sync.
//!
//! It names the worktree, branch, node and session, the page in scope, and any selected card that
//! no comment names; each comment names its own card ([`card_line`]). It lists no other cards and
//! never dumps raw document JSON: agents read the page and its cards with `get_node`.

use figcraft_doc::{Document, NodeId, NodeKind};

/// The most characters of the whole packet.
pub const MAX_CONTEXT_CHARS: usize = 8_000;
/// The most characters of one card's text.
const MAX_CARD_TEXT: usize = 600;

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
        "MINDMAP CONTEXT\nWorktree: {} · Branch: {} · Node: {} · Session: {}\n\
         Use the mindmap skill (skills/mindmap/SKILL.md): it lists the tools and how each kind of work finishes. \
         Only the node writes .mindmap/map.figcraft. Keep names stable and source claims.\n",
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
        out.push_str(&format!(
            "Scope: page {} (id {}) and its subpages only.\n",
            name(doc, page),
            page.0
        ));
        if request.new_page {
            out.push_str("This page was just created for the prompt below.\n");
        }
    } else {
        out.push_str("Scope: the whole map; the request names no page.\n");
    }
    if let Some(source) = request.source.filter(|id| doc.node(*id).is_some()) {
        out.push_str("\nMade from this card:\n");
        out.push_str(&card(doc, source));
    }
    // A commented card is named with its comment; a selected one without a comment here, once.
    let mut seen: Vec<NodeId> = request.commented.to_vec();
    for id in request.selected {
        if seen.contains(id) {
            continue;
        }
        seen.push(*id);
        if let Some(line) = card_line(doc, *id) {
            out.push_str(&format!("Selected: {line}\n"));
        }
    }
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

/// `Card (id 7) · Parent › Card`: the card's name and id, and its path below the page when it is
/// nested. The agent reads its content with `get_node`.
pub fn card_line(doc: &Document, id: NodeId) -> Option<String> {
    let node = doc.node(id)?;
    let mut line = format!("{} (id {})", one_line(&node.name), id.0);
    let names = path(doc, id);
    let below_page = names.get(1..).unwrap_or_default();
    if below_page.len() > 1 {
        line.push_str(&format!(" · {}", below_page.join(" › ")));
    }
    Some(line)
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
                &json!({"id": remote.0, "pluginData": {"mindmap.parent_page": backend.0.to_string()}}),
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
    fn a_comment_scopes_its_page_and_leaves_its_card_to_the_comment() {
        let (doc, backend, card, _) = doc();
        let text = render(&doc, &request(None, &[card]));
        assert!(
            text.contains(&format!(
                "Scope: page backend code (id {}) and its subpages only.\n",
                backend.0
            )),
            "{text}"
        );
        for gone in [
            "Page:",
            "Subpages:",
            "Cards:",
            "Commented cards",
            "Proposed system",
        ] {
            assert!(!text.contains(gone), "{gone:?} is gone: {text}");
        }
        assert!(
            !text.contains("Card Overview"),
            "another page leaked in: {text}"
        );
        assert!(!text.contains("absoluteBounds"), "no raw JSON: {text}");
    }

    #[test]
    fn a_page_brings_its_name_not_a_list_of_its_cards() {
        let (doc, backend, _, _) = doc();
        let text = render(&doc, &request(Some(backend), &[]));
        assert!(
            text.contains(&format!("Scope: page backend code (id {})", backend.0)),
            "{text}"
        );
        assert!(!text.contains("Cards:"), "{text}");
        assert!(
            !text.contains("Proposed system"),
            "only selected or commented cards: {text}"
        );
        assert!(
            !text.contains("tools:"),
            "the skill lists the tools: {text}"
        );
    }

    #[test]
    fn a_selected_card_is_named_once_and_a_commented_one_by_its_comment() {
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
            text.contains(&format!("Scope: page backend code (id {})", backend.0)),
            "{text}"
        );
        assert!(
            text.contains(&format!("Selected: Proposed system (id {})\n", card.0)),
            "{text}"
        );
        let both = render(
            &doc,
            &Request {
                selected: &selected,
                ..request(None, &selected)
            },
        );
        assert!(!both.contains("Selected:"), "the comment names it: {both}");
    }

    #[test]
    fn a_card_line_gives_name_id_and_path_below_the_page() {
        let (doc, _, card, _) = doc();
        assert_eq!(
            card_line(&doc, card),
            Some(format!("Proposed system (id {})", card.0))
        );
        let title = doc.node(card).unwrap().children[0];
        assert_eq!(
            card_line(&doc, title),
            Some(format!("title (id {}) · Proposed system › title", title.0))
        );
        assert_eq!(card_line(&doc, NodeId(999_999)), None);
    }

    #[test]
    fn a_named_page_wins_and_no_page_says_so() {
        let (doc, backend, _, _) = doc();
        assert!(render(&doc, &request(Some(backend), &[])).contains("Scope: page backend code"));
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
