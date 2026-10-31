//! Page/subpage hierarchy stored as document metadata.
//!
//! A page with `mindmap.parent_page` is a subpage. A subpage created from a card also stores
//! `mindmap.source_node`; if that card disappears, the subpage and its descendants are removed.
//! The tree is docked in the map's sidebar under the workspace switcher (design: `Sidebar pages ·
//! window`): every page, indented by level, the open one marked; a click opens a page. ⌘P (the
//! `map.pageTree` action) reveals the sidebar.

use std::collections::HashSet;

use egui::{Align2, Color32, CornerRadius, FontId, Id, Rect, Sense, Stroke, Ui, pos2, vec2};
use figcraft_doc::{Document, NodeId};

use crate::FigApp;
use crate::theme::{Tokens, regular};
use crate::workspaces::Skin;

pub const PARENT_KEY: &str = "mindmap.parent_page";
/// Pages nest at most this deep (the root counts as the first level).
pub const MAX_LEVELS: usize = 5;
pub const SOURCE_KEY: &str = "mindmap.source_node";

/// One row of the docked tree, and its pitch.
const ROW_H: f32 = 30.0;
/// Each level sits this much further in.
const INDENT: f32 = 16.0;

/// The tree's state plus the orphan-cleanup clock.
#[derive(Default)]
pub struct State {
    /// Asked to show the tree (⌘P): the sidebar opens on the next frame.
    pub open: bool,
    pub just_opened: bool,
    /// Pages whose children are folded away in the docked tree.
    pub collapsed: HashSet<NodeId>,
    last_cleanup: f64,
}

fn meta_id(doc: &Document, page: NodeId, key: &str) -> Option<NodeId> {
    doc.node(page)?.plugin_data.get(key)?.parse::<u64>().ok().and_then(|id| doc.node(NodeId(id)).map(|_| NodeId(id)))
}

/// The parent page, when the page is a subpage still attached to a live parent.
pub fn parent(doc: &Document, page: NodeId) -> Option<NodeId> {
    meta_id(doc, page, PARENT_KEY)
}

/// The component that owns a subpage.
pub fn source(doc: &Document, page: NodeId) -> Option<NodeId> {
    meta_id(doc, page, SOURCE_KEY)
}

/// Root pages, in document order.
pub fn roots(doc: &Document) -> Vec<NodeId> {
    doc.pages.iter().copied().filter(|page| parent(doc, *page).is_none()).collect()
}

/// Direct children of `page`, in document order.
pub fn children(doc: &Document, page: NodeId) -> Vec<NodeId> {
    doc.pages.iter().copied().filter(|child| parent(doc, *child) == Some(page)).collect()
}

/// The root of a page, following parent metadata.
pub fn root(doc: &Document, page: NodeId) -> NodeId {
    let mut current = page;
    let mut seen = Vec::new();
    while let Some(parent) = parent(doc, current) {
        if seen.contains(&parent) || parent == current {
            break;
        }
        seen.push(current);
        current = parent;
    }
    current
}

/// Root-to-page path, for the breadcrumb.
pub fn path(doc: &Document, page: NodeId) -> Vec<NodeId> {
    let mut out = vec![page];
    let mut current = page;
    while let Some(parent) = parent(doc, current) {
        if out.contains(&parent) {
            break;
        }
        out.push(parent);
        current = parent;
    }
    out.reverse();
    out
}

/// Depth from the root; a root is 0.
pub fn depth(doc: &Document, page: NodeId) -> usize {
    path(doc, page).len().saturating_sub(1)
}

/// All descendants of `page` (not including it), deepest first.
/// Every page below `page`, deepest first.
pub(crate) fn descendants(doc: &Document, page: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![page];
    let mut seen = Vec::new();
    while let Some(current) = stack.pop() {
        if seen.contains(&current) {
            continue;
        }
        seen.push(current);
        for child in children(doc, current) {
            out.push(child);
            stack.push(child);
        }
    }
    out.sort_by_key(|child| std::cmp::Reverse(depth(doc, *child)));
    out
}

fn page_name(doc: &Document, page: NodeId) -> String {
    doc.node(page).map(|node| node.name.clone()).unwrap_or_else(|| "Untitled".to_string())
}

/// Store the hierarchy metadata on a new subpage.
pub fn set_subpage(app: &mut FigApp, page: u64, parent_page: u64, source_node: u64) -> Result<(), String> {
    let params = serde_json::json!({
        "id": page,
        "pluginData": {
            PARENT_KEY: parent_page.to_string(),
            SOURCE_KEY: source_node.to_string(),
        },
    });
    app.session.execute("node.set", &params).map(|_| ()).map_err(|error| error.to_string())
}

/// Delete `page`, its descendants and any card links pointing at them. Deepest first.
pub fn delete_tree(app: &mut FigApp, page: NodeId) -> Result<usize, String> {
    let (pages, links) = {
        let doc = app.session.doc().map_err(|error| error.to_string())?;
        let mut pages = descendants(doc, page);
        pages.push(page);
        let ids: Vec<u64> = pages.iter().map(|id| id.0).collect();
        let mut links = Vec::new();
        for node in doc.nodes() {
            if node.plugin_data.get(crate::new_page::DETAIL_PAGE_KEY).and_then(|value| value.parse::<u64>().ok()).is_some_and(|id| ids.contains(&id))
            {
                links.push(node.id);
            }
        }
        (pages, links)
    };
    for node in links {
        let _ = app.session.execute("node.set", &serde_json::json!({"id": node, "pluginData": {crate::new_page::DETAIL_PAGE_KEY: null}}));
    }
    let mut deleted = 0;
    for page in pages {
        match app.session.execute("page.delete", &serde_json::json!({"id": page})) {
            Ok(_) => deleted += 1,
            Err(error) => return Err(format!("could not delete {page}: {error}")),
        }
    }
    Ok(deleted)
}

/// Remove subpages whose source card or parent page disappeared. Runs at most every two seconds.
pub fn maintain(app: &mut FigApp, ctx: &egui::Context) {
    if app.page_tree.last_cleanup > 0.0 && ctx.input(|input| input.time) - app.page_tree.last_cleanup < 2.0 {
        return;
    }
    app.page_tree.last_cleanup = ctx.input(|input| input.time);
    let orphans: Vec<NodeId> = match app.session.doc() {
        Ok(doc) => doc
            .pages
            .iter()
            .copied()
            .filter(|page| {
                let has_parent = doc.node(*page).is_some_and(|node| node.plugin_data.contains_key(PARENT_KEY));
                if !has_parent {
                    return false;
                }
                let parent_gone = parent(doc, *page).is_none();
                let source_gone = doc
                    .node(*page)
                    .and_then(|node| node.plugin_data.get(SOURCE_KEY))
                    .and_then(|value| value.parse::<u64>().ok())
                    .is_some_and(|id| doc.node(NodeId(id)).is_none());
                parent_gone || source_gone
            })
            .collect(),
        Err(_) => return,
    };
    for page in orphans {
        let name = app.session.doc().ok().map(|doc| page_name(doc, page)).unwrap_or_default();
        if delete_tree(app, page).is_ok() {
            app.toast(format!("Removed subpage {name}; its component is gone"));
        }
    }
}

/// ⌘P (`map.pageTree`): the tree lives in the sidebar, so asking for it shows the sidebar.
pub fn show(app: &mut FigApp, _ctx: &egui::Context) {
    if std::mem::take(&mut app.page_tree.open) {
        app.page_tree.just_opened = false;
        app.ws.pane_open = true;
    }
}

/// The docked tree's rows, top to bottom: each root, then its pages depth first in document
/// order, with their level; a folded page's descendants are left out.
pub fn rows(doc: &Document, collapsed: &HashSet<NodeId>) -> Vec<(NodeId, usize)> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut stack: Vec<(NodeId, usize)> = roots(doc).into_iter().rev().map(|page| (page, 0)).collect();
    while let Some((page, level)) = stack.pop() {
        if !seen.insert(page) {
            continue;
        }
        out.push((page, level));
        if !collapsed.contains(&page) {
            stack.extend(children(doc, page).into_iter().rev().map(|child| (child, level + 1)));
        }
    }
    out
}

/// One docked row as drawn.
struct Line {
    page: NodeId,
    level: usize,
    name: String,
    kids: usize,
    folded: bool,
}

/// What a click in the tree asked for.
enum Ask {
    Open(NodeId),
    Fold(NodeId),
    Delete(NodeId),
    NewPage,
}

/// The docked tree in `rect`: `PAGES  11`, the rows, and `+ Page` under them.
pub fn docked(app: &mut FigApp, ui: &mut Ui, rect: Rect) {
    let tokens = Tokens::get(ui.ctx());
    let s = Skin::of(&tokens);
    // The open row's fill, as designed (the dark skin's selection otherwise).
    let current_fill = if tokens.dark { s.selected } else { Color32::from_rgb(0xEF, 0xEE, 0xEA) };
    let (lines, total) = match app.session.doc() {
        Ok(doc) => {
            let lines: Vec<Line> = rows(doc, &app.page_tree.collapsed)
                .into_iter()
                .map(|(page, level)| Line {
                    page,
                    level,
                    name: page_name(doc, page),
                    kids: children(doc, page).len(),
                    folded: app.page_tree.collapsed.contains(&page),
                })
                .collect();
            (lines, doc.pages.len())
        }
        Err(_) => return,
    };
    let current = app.session.page().ok();
    ui.painter().text(pos2(rect.min.x + 14.0, rect.min.y), Align2::LEFT_TOP, format!("PAGES  {total}"), FontId::monospace(9.0), s.muted);

    let list = Rect::from_min_max(pos2(rect.min.x, rect.min.y + 18.0), rect.max);
    let mut ask = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(list).id_salt("page_tree_docked"), |ui| {
        ui.set_clip_rect(list.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical().id_salt("page_tree_docked_scroll").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for line in &lines {
                if let Some(a) = row(ui, &s, line, current == Some(line.page), current_fill) {
                    ask = Some(a);
                }
            }
            ui.add_space(14.0);
            let (outer, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
            let plus = Rect::from_min_max(pos2(outer.min.x + 6.0, outer.min.y), pos2(outer.min.x + 90.0, outer.max.y));
            let resp = ui.interact(plus, Id::new("page_tree_new_page"), Sense::click());
            crate::a11y::button(&resp, "New page");
            if resp.hovered() {
                ui.painter().rect_filled(plus, CornerRadius::same(4), s.soft);
            }
            let ink = if resp.hovered() { s.text } else { s.muted };
            ui.painter().text(pos2(plus.min.x + 8.0, plus.center().y), Align2::LEFT_CENTER, "+ Page", regular(12.0), ink);
            if resp.on_hover_text("New page").clicked() {
                ask = Some(Ask::NewPage);
            }
            ui.add_space(12.0);
        });
    });

    match ask {
        Some(Ask::Open(page)) => {
            app.run("page.select", serde_json::json!({"id": page.0}));
        }
        Some(Ask::Fold(page)) => {
            if !app.page_tree.collapsed.remove(&page) {
                app.page_tree.collapsed.insert(page);
            }
        }
        Some(Ask::Delete(page)) => app.page_delete = Some(page),
        Some(Ask::NewPage) => app.new_page.start(total + 1),
        None => {}
    }
}

/// One page: the fold chevron for a page with children, its name indented by level, and its child
/// count at the right (× to delete while hovered). The open page wears the fill and the accent bar.
fn row(ui: &mut Ui, s: &Skin, line: &Line, is_current: bool, current_fill: Color32) -> Option<Ask> {
    let (outer, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let rect = Rect::from_min_max(pos2(outer.min.x + 6.0, outer.min.y), pos2(outer.max.x - 6.0, outer.max.y));
    crate::a11y::button(&response, &line.name);
    let x = rect.min.x + 22.0 + line.level as f32 * INDENT;
    let fold = (line.kids > 0).then(|| {
        let r = Rect::from_center_size(pos2(x - 11.0, rect.center().y), vec2(16.0, ROW_H));
        ui.interact(r, Id::new(("page_tree_fold", line.page.0)), Sense::click())
    });
    let delete = ui.interact(
        Rect::from_center_size(pos2(rect.max.x - 14.0, rect.center().y), vec2(20.0, ROW_H - 6.0)),
        Id::new(("page_tree_delete", line.page.0)),
        Sense::click(),
    );
    let hot = response.hovered() || delete.hovered() || fold.as_ref().is_some_and(|f| f.hovered());
    if is_current {
        ui.painter().rect_filled(rect, CornerRadius { nw: 0, sw: 0, ne: 5, se: 5 }, current_fill);
        ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(2.0, rect.height())), CornerRadius::ZERO, s.accent);
    } else if hot {
        ui.painter().rect_filled(rect, CornerRadius::same(5), s.soft);
    }
    if let Some(fold) = &fold {
        let ink = if fold.hovered() { s.text } else { s.muted };
        chevron(ui, pos2(x - 11.0, rect.center().y), !line.folded, ink);
    }
    let room = (rect.max.x - 34.0 - x).max(30.0);
    let name = crate::widgets::elide_font(ui, &line.name, room, regular(12.0));
    ui.painter().text(pos2(x, rect.center().y), Align2::LEFT_CENTER, name, regular(12.0), s.text);
    // The right column: how many children, or the depth cap on a page at the last level.
    let trail = pos2(rect.max.x - 14.0, rect.center().y);
    if hot {
        let ink = if delete.hovered() { s.danger } else { s.faint };
        ui.painter().text(trail, Align2::CENTER_CENTER, "×", regular(12.0), ink);
    } else if line.kids > 0 {
        ui.painter().text(trail, Align2::CENTER_CENTER, line.kids.to_string(), FontId::monospace(9.0), s.faint);
    } else if line.level + 1 >= MAX_LEVELS {
        ui.painter().text(trail, Align2::CENTER_CENTER, format!("L{}", line.level + 1), FontId::monospace(9.0), s.faint);
    }
    let delete = delete.on_hover_text(format!("Delete {}…", line.name));
    let mut ask = None;
    if delete.clicked() {
        ask = Some(Ask::Delete(line.page));
    } else if fold.as_ref().is_some_and(|f| f.clicked()) {
        ask = Some(Ask::Fold(line.page));
    } else if response.clicked() {
        ask = Some(Ask::Open(line.page));
    }
    response.context_menu(|ui| {
        if ui.button("Open").clicked() {
            ask = Some(Ask::Open(line.page));
            ui.close();
        }
        if ui.button("Delete page…").clicked() {
            ask = Some(Ask::Delete(line.page));
            ui.close();
        }
    });
    ask
}

/// The fold mark: ⌄ when open, › when folded.
fn chevron(ui: &Ui, c: egui::Pos2, open: bool, color: Color32) {
    let stroke = Stroke::new(1.1, color);
    let points = if open {
        [pos2(c.x - 3.0, c.y - 1.5), pos2(c.x, c.y + 1.5), pos2(c.x + 3.0, c.y - 1.5)]
    } else {
        [pos2(c.x - 1.5, c.y - 3.0), pos2(c.x + 1.5, c.y), pos2(c.x - 1.5, c.y + 3.0)]
    };
    ui.painter().line_segment([points[0], points[1]], stroke);
    ui.painter().line_segment([points[1], points[2]], stroke);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(doc: &mut Document, parent: NodeId, name: &str) -> NodeId {
        let page = doc.add_page(name);
        doc.node_mut(page).unwrap().plugin_data.insert(PARENT_KEY.into(), parent.0.to_string());
        page
    }

    #[test]
    fn the_docked_rows_go_depth_first_in_document_order() {
        let mut doc = Document::new("Map");
        let overview = doc.pages[0];
        let canvas = sub(&mut doc, overview, "Canvas and layers");
        let components = sub(&mut doc, overview, "Components");
        let paint = sub(&mut doc, canvas, "Paint");
        let checkout = doc.add_page("Checkout");
        let listed = rows(&doc, &HashSet::new());
        assert_eq!(listed, [(overview, 0), (canvas, 1), (paint, 2), (components, 1), (checkout, 0)]);
        let folded = rows(&doc, &HashSet::from([canvas]));
        assert_eq!(folded, [(overview, 0), (canvas, 1), (components, 1), (checkout, 0)]);
    }

    #[test]
    fn clicking_a_docked_page_opens_it() {
        crate::with_map_mode(|| {
            let mut session = figcraft_engine::Session::new();
            let second = session.execute("page.add", &serde_json::json!({"name": "Second"})).unwrap()["id"].as_u64().unwrap();
            let first = session.doc().unwrap().pages[0];
            session.execute("page.select", &serde_json::json!({"id": first.0})).unwrap();
            let mut u = crate::layers_drag_tests::Ui::new(session);
            u.app.onboarding.active = false;
            u.app.onboarding.decided = true;
            u.app.ui.home = false;
            u.app.ws.pane_open = true;
            u.frame(vec![]);
            assert_eq!(u.app.session.page().ok(), Some(first));
            // Walk down the sidebar's rows until one opens the second page.
            let mut opened = false;
            // Below the switcher head, which opens the switcher.
            for y in (124..400).step_by(10) {
                let p = egui::pos2(120.0, y as f32);
                u.button(p, true);
                u.button(p, false);
                if u.app.session.page().ok() == Some(NodeId(second)) {
                    opened = true;
                    break;
                }
            }
            assert!(opened, "a row of the docked tree opens its page");
            assert!(!u.app.new_page.open);
        });
    }
}
