//! ⌘⌫ on the map (design: `Delete with Cmd+Backspace` on the Mindmap page): the human deletes
//! what is selected, at once, with no dialog and no agent. A card's detail page (and its subpages)
//! goes with it, in the same undo step, so ⌘Z brings both back. The deletion is a design edit like
//! any other: it waits for Sync.
//!
//! Map mode refuses the editor's `edit.delete` and `edit.undo` ([`crate::map_command_allowed`]),
//! so these actions run them on the session directly, only from here.

use figcraft_doc::{Document, NodeId, NodeKind};
use serde_json::json;

use crate::FigApp;
use crate::new_page::DETAIL_PAGE_KEY;
use crate::page_tree;

/// The pages that go with `ids`: every page a deleted layer links as its detail page, or that
/// names one as its source, with their subpages; deepest first.
pub fn detail_pages(doc: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let mut gone: Vec<NodeId> = Vec::new();
    for id in ids {
        collect(doc, *id, &mut gone);
    }
    let mut pages: Vec<NodeId> = Vec::new();
    for page in doc.pages.iter().copied() {
        let linked = gone.iter().any(|id| {
            doc.node(*id)
                .and_then(|node| node.plugin_data.get(DETAIL_PAGE_KEY))
                .and_then(|value| value.parse::<u64>().ok())
                .is_some_and(|linked| linked == page.0)
        });
        let sourced = page_tree::source(doc, page).is_some_and(|source| gone.contains(&source));
        if (linked || sourced) && !pages.contains(&page) {
            pages.push(page);
        }
    }
    let mut out = Vec::new();
    for page in pages {
        for sub in page_tree::descendants(doc, page).into_iter().rev().chain([page]) {
            if !out.contains(&sub) {
                out.push(sub);
            }
        }
    }
    out
}

fn collect(doc: &Document, id: NodeId, out: &mut Vec<NodeId>) {
    let Some(node) = doc.node(id) else { return };
    out.push(id);
    for child in node.children.clone() {
        collect(doc, child, out);
    }
}

/// What the toast says was deleted: "2 cards, 1 arrow, 1 label".
pub fn summary(doc: &Document, ids: &[NodeId], pages: usize) -> String {
    let (mut cards, mut arrows, mut labels, mut other) = (0, 0, 0, 0);
    for id in ids {
        match doc.node(*id).map(|node| &node.kind) {
            Some(NodeKind::Text(_)) => labels += 1,
            Some(NodeKind::Vector { .. } | NodeKind::Line) => arrows += 1,
            Some(NodeKind::Frame | NodeKind::Group | NodeKind::Section { .. } | NodeKind::Instance { .. } | NodeKind::Component { .. }) => cards += 1,
            Some(_) => other += 1,
            None => {}
        }
    }
    let mut parts = Vec::new();
    for (n, one, many) in
        [(cards, "card", "cards"), (arrows, "arrow", "arrows"), (labels, "label", "labels"), (other, "layer", "layers"), (pages, "page", "pages")]
    {
        if n > 0 {
            parts.push(format!("{n} {}", if n == 1 { one } else { many }));
        }
    }
    parts.join(", ")
}

/// Delete the selection and the pages that go with it, as one undo step. Returns what the toast
/// says, or `None` when nothing is selected.
pub fn delete_selection(app: &mut FigApp) -> Option<String> {
    let ids = app.session.selection();
    if ids.is_empty() {
        return None;
    }
    let (pages, said) = {
        let doc = app.session.doc().ok()?;
        let pages = detail_pages(doc, &ids);
        let said = summary(doc, &ids, pages.len());
        (pages, said)
    };
    let mut commands = vec![json!({"command": "edit.delete", "params": {"ids": ids.iter().map(|id| id.0).collect::<Vec<_>>()}})];
    for page in &pages {
        commands.push(json!({"command": "page.delete", "params": {"id": page.0}}));
    }
    match app.session.execute("command.batch", &json!({"label": "Delete", "commands": commands})) {
        Ok(_) => {
            app.session.set_selection(Vec::new());
            Some(format!("Deleted {said}  ·  ⌘Z to undo"))
        }
        Err(error) => Some(format!("Could not delete: {error}")),
    }
}

/// ⌘Z / ⇧⌘Z on the map: take back (or redo) the human's own last edit. In a live map that is only
/// their own steps, never an agent's or another person's.
pub fn undo(app: &mut FigApp, redo: bool) {
    let _ = app.session.execute(if redo { "edit.redo" } else { "edit.undo" }, &json!({}));
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_engine::Session;

    fn frame(s: &mut Session, parent: u64, name: &str) -> NodeId {
        NodeId(
            s.execute("node.create", &json!({"type": "FRAME", "parent": parent, "name": name, "x": 0, "y": 0, "width": 100, "height": 80})).unwrap()
                ["id"]
                .as_u64()
                .unwrap(),
        )
    }

    #[test]
    fn a_card_takes_its_detail_page_and_undo_brings_both_back() {
        let mut app = FigApp::new(Session::new(), Box::new(crate::NoServices));
        let page = app.session.page().unwrap();
        let card = frame(&mut app.session, page.0, "Card Canvas");
        let keep = frame(&mut app.session, page.0, "Card Other");
        let detail = NodeId(app.session.execute("page.add", &json!({"name": "Canvas — detail"})).unwrap()["id"].as_u64().unwrap());
        app.session.execute("page.select", &json!({"id": page.0})).unwrap();
        page_tree::set_subpage(&mut app, detail.0, page.0, card.0).unwrap();
        app.session.execute("node.set", &json!({"id": card.0, "pluginData": {DETAIL_PAGE_KEY: detail.0.to_string()}})).unwrap();
        app.session.set_selection(vec![card]);

        let said = delete_selection(&mut app).unwrap();
        assert!(said.starts_with("Deleted 1 card, 1 page"), "{said}");
        let doc = app.session.doc().unwrap();
        assert!(doc.node(card).is_none() && !doc.pages.contains(&detail));
        assert!(doc.node(keep).is_some());

        undo(&mut app, false);
        let doc = app.session.doc().unwrap();
        assert!(doc.node(card).is_some() && doc.pages.contains(&detail), "one step brings both back");
        undo(&mut app, true);
        assert!(app.session.doc().unwrap().node(card).is_none());
    }

    #[test]
    fn nothing_selected_deletes_nothing() {
        let mut app = FigApp::new(Session::new(), Box::new(crate::NoServices));
        assert!(delete_selection(&mut app).is_none());
    }
}
