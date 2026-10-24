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
/// Pages nest at most this deep (the root counts as the first level).
pub const MAX_LEVELS: usize = 5;
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
    if !crate::map_mode() && ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::P)) {
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
    let levels = app.session.doc().map(|doc| pages.iter().map(|page| depth(doc, *page) + 1).max().unwrap_or(1)).unwrap_or(1);
    let mut close = false;
    let mut pick = None;
    let mut remove = None;

    let area =
        egui::Area::new(Id::new("page_tree_modal")).anchor(Align2::CENTER_CENTER, vec2(0.0, -18.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::NONE
                .fill(PANEL)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(CornerRadius::same(10))
                .shadow(egui::epaint::Shadow { offset: [0, 12], blur: 30, spread: 0, color: Color32::from_black_alpha(36) })
                .inner_margin(Margin { left: 12, right: 12, top: 18, bottom: 13 })
                .show(ui, |ui| {
                    ui.set_width(436.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    // PAGES, then `8 subpages · 5 levels` under it; the × at the top right.
                    let head = ui.max_rect();
                    let close_rect = egui::Rect::from_min_size(egui::pos2(head.right() - 22.0, head.top() - 8.0), vec2(22.0, 22.0));
                    let close_resp = ui.interact(close_rect, Id::new("page_tree_close"), egui::Sense::click());
                    ui.painter().text(
                        close_rect.center(),
                        Align2::CENTER_CENTER,
                        "×",
                        crate::theme::regular(15.0),
                        if close_resp.hovered() { INK } else { MUTED },
                    );
                    if close_resp.clicked() {
                        close = true;
                    }
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.label(RichText::new("PAGES").font(FontId::monospace(9.0)).color(ACCENT));
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        let plural = if subpages == 1 { "" } else { "s" };
                        let level_plural = if levels == 1 { "" } else { "s" };
                        ui.label(
                            RichText::new(format!("{subpages} subpage{plural} · {levels} level{level_plural}"))
                                .font(FontId::monospace(8.5))
                                .color(MUTED),
                        );
                    });
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical().id_salt("page_tree_scroll").max_height(430.0).auto_shrink([false, true]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
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
                    ui.add_space(14.0);
                    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), egui::Sense::hover());
                    ui.painter().line_segment([line.left_center() + vec2(8.0, 0.0), line.right_center() - vec2(4.0, 0.0)], Stroke::new(1.0, BORDER));
                    ui.add_space(9.0);
                    let (foot, _) = ui.allocate_exact_size(vec2(ui.available_width(), 11.0), egui::Sense::hover());
                    let font = FontId::monospace(8.5);
                    ui.painter().text(foot.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, "Subpages come from a card", font.clone(), FAINT);
                    ui.painter().text(foot.right_center() - vec2(8.0, 0.0), Align2::RIGHT_CENTER, format!("{MAX_LEVELS} levels max"), font, FAINT);
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
    let _ = current_root;
    // 24 pt rows on a 26 pt pitch; 14 pt of indent per level; the chevron marks a page with children.
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), egui::Sense::click());
    if is_current {
        ui.painter().rect_filled(rect, CornerRadius::same(5), Color32::from_rgb(0xF1, 0xF0, 0xED));
        ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, vec2(2.0, rect.height())), CornerRadius::ZERO, ACCENT);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(5), SOFT);
    }
    let x = rect.left() + 8.0 + level as f32 * 14.0;
    if kids > 0 {
        ui.painter().text(egui::pos2(x, rect.center().y), Align2::LEFT_CENTER, "⌄", crate::theme::regular(11.0), MUTED);
    }
    ui.painter().text(egui::pos2(x + 13.0, rect.center().y), Align2::LEFT_CENTER, &name, FontId::monospace(10.5), INK);
    // The right column: how many children, or the depth cap on a page at the last level.
    let trail = if kids > 0 {
        kids.to_string()
    } else if level + 1 >= MAX_LEVELS {
        format!("L{}", level + 1)
    } else {
        String::new()
    };
    let close = ui.interact(
        egui::Rect::from_center_size(egui::pos2(rect.right() - 10.0, rect.center().y), vec2(16.0, 20.0)),
        ui.id().with(("page_tree_delete", page.0)),
        egui::Sense::click(),
    );
    let hot = response.hovered() || close.hovered();
    if !trail.is_empty() {
        ui.painter().text(egui::pos2(rect.left() + 398.0, rect.center().y), Align2::LEFT_CENTER, trail, FontId::monospace(9.0), FAINT);
    }
    if hot {
        ui.painter().text(close.rect.center(), Align2::CENTER_CENTER, "×", crate::theme::regular(11.0), if close.hovered() { DANGER } else { FAINT });
    }
    if close.clicked() {
        *remove = Some(page);
    } else if response.clicked() {
        *pick = Some(page);
    }
    let _ = Duration::from_secs(0);
}
