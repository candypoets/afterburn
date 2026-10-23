//! Page/subpage hierarchy stored as document metadata.
//!
//! A page with `mindmap.parent_page` is a subpage. A subpage created from a card also stores
//! `mindmap.source_node`; if that card disappears, the subpage and its descendants are removed.
//! The header shows only root pages; the current path is a read-only breadcrumb. The centered
//! tree popover opens from an already-focused root tab, a right-click on a tab, or Cmd/Ctrl+Shift+O.

use std::time::Duration;

use egui::{Align2, Color32, CornerRadius, FontId, Id, Margin, RichText, Stroke, vec2};
use figcraft_doc::{Document, NodeId};

use crate::FigApp;

pub const PARENT_KEY: &str = "mindmap.parent_page";
pub const SOURCE_KEY: &str = "mindmap.source_node";

const PANEL: Color32 = Color32::WHITE;
const SOFT: Color32 = Color32::from_rgb(0xF8, 0xF6, 0xF3);
const BORDER: Color32 = Color32::from_rgb(0xE3, 0xE2, 0xDF);
const INK: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x1A);
const MUTED: Color32 = Color32::from_rgb(0x6E, 0x6C, 0x68);
const FAINT: Color32 = Color32::from_rgb(0x9A, 0x97, 0x91);
const ACCENT: Color32 = Color32::from_rgb(0xB5, 0x45, 0x2F);
const DANGER: Color32 = Color32::from_rgb(0x9E, 0x3B, 0x2C);

/// Popover state plus the orphan-cleanup clock.
#[derive(Default)]
pub struct State {
    pub open: bool,
    pub just_opened: bool,
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

/// The centered page tree popover and its shortcut.
pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::P)) {
        app.page_tree.open = true;
        app.page_tree.just_opened = true;
    }
    if !app.page_tree.open {
        return;
    }
    let (roots, pages) = match app.session.doc() {
        Ok(doc) => (roots(doc), doc.pages.clone()),
        Err(_) => return,
    };
    let current = app.session.page().ok();
    let current_root = current.and_then(|page| app.session.doc().ok().map(|doc| root(doc, page)));
    let subpages = pages.len().saturating_sub(roots.len());
    let mut close = false;
    let mut pick = None;
    let mut remove = None;

    let area =
        egui::Area::new(Id::new("page_tree_modal")).anchor(Align2::CENTER_CENTER, vec2(0.0, -18.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::NONE
                .fill(PANEL)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(CornerRadius::same(10))
                .shadow(egui::epaint::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha(46) })
                .inner_margin(Margin::symmetric(18, 14))
                .show(ui, |ui| {
                    ui.set_width(420.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("PAGES").font(FontId::monospace(9.5)).color(ACCENT));
                        let plural = if subpages == 1 { "" } else { "s" };
                        ui.label(
                            RichText::new(format!("{subpages} subpage{plural} · {} roots", roots.len())).font(FontId::monospace(8.8)).color(MUTED),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(egui::Button::new(RichText::new("×").size(16.0).color(MUTED)).frame(false)).clicked() {
                                close = true;
                            }
                        });
                    });
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical().id_salt("page_tree_scroll").max_height(430.0).auto_shrink([false, true]).show(ui, |ui| {
                        for root in &roots {
                            row(app, ui, *root, 0, current, current_root, &mut pick, &mut remove);
                            let mut stack: Vec<(NodeId, usize)> = match app.session.doc() {
                                Ok(doc) => children(doc, *root).into_iter().map(|child| (child, 1)).collect(),
                                Err(_) => Vec::new(),
                            };
                            while let Some((page, level)) = stack.pop() {
                                row(app, ui, page, level, current, current_root, &mut pick, &mut remove);
                                let mut kids: Vec<(NodeId, usize)> = match app.session.doc() {
                                    Ok(doc) => children(doc, page).into_iter().map(|child| (child, level + 1)).collect(),
                                    Err(_) => Vec::new(),
                                };
                                kids.reverse();
                                stack.extend(kids);
                            }
                        }
                    });
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Right-click a main tab, click the focused main tab, or ⌘⇧P / Ctrl+Shift+P. Subpages are created from a card.")
                            .font(FontId::monospace(8.2))
                            .color(FAINT),
                    );
                });
        });

    if let Some(page) = pick {
        app.run("page.select", serde_json::json!({"id": page.0}));
        app.page_tree.open = false;
    }
    if let Some(page) = remove {
        app.page_delete = Some(page);
        app.page_tree.open = false;
    }
    let pressed = ctx.input(|input| input.pointer.any_pressed());
    let pointer = ctx.input(|input| input.pointer.interact_pos());
    let inside = pointer.is_some_and(|pos| area.response.rect.contains(pos));
    let just_opened = app.page_tree.just_opened;
    app.page_tree.just_opened = false;
    if (pressed && !inside && !just_opened) || close || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        app.page_tree.open = false;
    }
}

fn row(
    app: &FigApp,
    ui: &mut egui::Ui,
    page: NodeId,
    level: usize,
    current: Option<NodeId>,
    current_root: Option<NodeId>,
    pick: &mut Option<NodeId>,
    remove: &mut Option<NodeId>,
) {
    let doc = match app.session.doc() {
        Ok(doc) => doc,
        Err(_) => return,
    };
    let name = page_name(doc, page);
    let kids = children(doc, page).len();
    let is_current = current == Some(page);
    let is_root = current_root == Some(page);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), egui::Sense::click());
    let bg = if is_current || response.hovered() { SOFT } else { PANEL };
    ui.painter().rect_filled(rect, CornerRadius::same(5), bg);
    if is_current {
        ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, vec2(2.0, rect.height())), CornerRadius::ZERO, ACCENT);
    }
    let x = rect.left() + 10.0 + level as f32 * 16.0;
    if kids > 0 {
        ui.painter().text(egui::pos2(x, rect.center().y), Align2::LEFT_CENTER, "⌄", FontId::monospace(9.0), FAINT);
    }
    let label_x = x + if kids > 0 { 14.0 } else { 4.0 };
    ui.painter().text(
        egui::pos2(label_x, rect.center().y),
        Align2::LEFT_CENTER,
        &name,
        FontId::proportional(11.5),
        if is_current || is_root { INK } else { MUTED },
    );
    if is_root && level == 0 {
        ui.painter().text(egui::pos2(rect.right() - 34.0, rect.center().y), Align2::RIGHT_CENTER, "main", FontId::monospace(8.0), FAINT);
    }
    let close = ui.interact(
        egui::Rect::from_center_size(egui::pos2(rect.right() - 12.0, rect.center().y), vec2(18.0, 22.0)),
        ui.id().with(("page_tree_delete", page.0)),
        egui::Sense::click(),
    );
    if response.hovered() || close.hovered() || is_current {
        ui.painter().text(close.rect.center(), Align2::CENTER_CENTER, "×", FontId::proportional(11.0), if close.hovered() { DANGER } else { FAINT });
    }
    if close.clicked() {
        *remove = Some(page);
    } else if response.clicked() {
        *pick = Some(page);
    }
    let _ = Duration::from_secs(0);
}
