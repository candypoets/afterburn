//! The title row's tabs: the open pages of the map, any depth, from the window's left above the
//! page tree (the Page tabs design). The tree, a link or search opens a page by selecting it; the
//! page then gets a tab. A tab shows its × while hovered; it, ⌘W for the active one, or a middle-click
//! closes a tab (its neighbour's page opens). Closing the last tab leaves no page open: the strip
//! keeps `+ Page` and the canvas is empty until a page is opened. Closing a tab never deletes a
//! page. Pages are renamed in the tree, not here.
//!
//! Each open map (its file, so each worktree) keeps its own tabs for the session. A tab whose page
//! was deleted goes away. When the tabs outgrow the row they shrink to a minimum, then the strip
//! scrolls.

use std::collections::HashMap;

use egui::{Align2, Color32, FontId, Id, Pos2, Rect, Sense, Stroke, Ui, pos2};
use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::FigApp;

/// The label's left inset in a cell; the design leaves more room after the label.
pub const PAD: f32 = 10.0;
/// The room after the label: the × and its margins.
const TAIL: f32 = 26.0;
/// The narrowest a tab shrinks to before the strip scrolls.
pub const MIN_CELL: f32 = 56.0;
/// The separators between cells.
pub const SEPARATOR: f32 = 1.0;
/// The active tab's underline.
pub const UNDERLINE: f32 = 2.0;

const INACTIVE: Color32 = Color32::from_rgb(0xF1, 0xF0, 0xED);
const ACTIVE: Color32 = Color32::WHITE;
/// The title row behind the cells.
pub const ROW: Color32 = Color32::from_rgb(0xF7, 0xF6, 0xF3);
const HOVER: Color32 = Color32::from_rgb(0xEA, 0xE9, 0xE5);
const MUTED: Color32 = Color32::from_rgb(0x6E, 0x6C, 0x68);
const INK: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x1A);
const ACCENT: Color32 = Color32::from_rgb(0xB5, 0x45, 0x2F);
const RULE: Color32 = Color32::from_rgb(0xE3, 0xE2, 0xDF);
/// The × button's hover fill.
const CLOSE_HOVER: Color32 = Color32::from_rgb(0xDF, 0xDE, 0xD9);
/// The × button: a 12 pt square in the cell's tail, its centre this far from the right edge.
const CLOSE: f32 = 12.0;
const CLOSE_INSET: f32 = 12.0;
/// Half the ×'s arm, corner to centre.
const CROSS: f32 = 2.5;

/// The tab label's type: a JetBrains Mono–like monospace.
pub fn font() -> FontId {
    FontId::monospace(9.5)
}

/// The tabs of every map opened this session.
#[derive(Default)]
pub struct State {
    /// The open pages by map (its file, else its session id), in strip order.
    maps: HashMap<String, Vec<NodeId>>,
    /// How far the strip is scrolled (points).
    scroll: f32,
    /// The page the strip last scrolled into view.
    shown: Option<NodeId>,
    /// The cells as last drawn: each tab's page and rect, then `+ Page` (`None`).
    pub(crate) cells: Vec<(Option<NodeId>, Rect)>,
    /// Maps whose last tab was closed, with the page open then: they show no page until one is
    /// opened (a `page.select`, or another page becoming current).
    empty: HashMap<String, Option<NodeId>>,
}

/// The open map's key: its file, or its session id when it has none.
fn key(app: &FigApp) -> Option<String> {
    app.session.ds().ok().map(|ds| ds.path.clone().unwrap_or_else(|| format!("doc:{}", ds.uid)))
}

/// Keep the open map's tabs in step: deleted pages lose their tab and the current page has one,
/// unless every tab was closed and that page is still the current one.
pub fn follow(app: &mut FigApp) {
    let Some(key) = key(app) else { return };
    let Ok(doc) = app.session.doc() else { return };
    let current = app.session.page().ok().filter(|page| doc.pages.contains(page));
    let state = &mut app.page_tabs;
    if state.empty.get(&key).is_some_and(|at| *at != current) {
        state.empty.remove(&key);
    }
    let empty = state.empty.contains_key(&key);
    let tabs = state.maps.entry(key).or_default();
    tabs.retain(|id| doc.pages.contains(id));
    if let Some(page) = current
        && !empty
        && !tabs.contains(&page)
    {
        tabs.push(page);
    }
}

/// Every tab of the open map is closed: no page shows.
pub fn is_empty(app: &FigApp) -> bool {
    key(app).is_some_and(|key| app.page_tabs.empty.contains_key(&key))
}

/// A page was opened (`page.select` from the tree, a link, search or a tab): it gets its tab back
/// even when it is the page that was current when the last tab closed.
pub fn opened(app: &mut FigApp) {
    if let Some(key) = key(app) {
        app.page_tabs.empty.remove(&key);
    }
    follow(app);
}

/// The open map's tabs, in strip order.
pub fn open(app: &FigApp) -> Vec<NodeId> {
    key(app).and_then(|key| app.page_tabs.maps.get(&key).cloned()).unwrap_or_default()
}

fn select(app: &mut FigApp, page: NodeId) {
    if app.session.page().ok() != Some(page) {
        app.run("page.select", json!({"id": page.0}));
    }
}

/// Close `page`'s tab; when it was the active one its neighbour's page opens (the next tab, else
/// the one before). Closing the last tab leaves no page open (and nothing selected, so no
/// shortcut acts on a page out of sight). The page itself is never touched. Returns whether a tab
/// closed.
pub fn close(app: &mut FigApp, page: NodeId) -> bool {
    follow(app);
    let Some(key) = key(app) else { return false };
    let current = app.session.page().ok();
    let Some(tabs) = app.page_tabs.maps.get_mut(&key) else { return false };
    let Some(at) = tabs.iter().position(|id| *id == page) else { return false };
    if tabs.len() == 1 {
        tabs.clear();
        app.page_tabs.empty.insert(key, current);
        app.run("select.none", json!({}));
        return true;
    }
    tabs.remove(at);
    let next = tabs.get(at).or_else(|| tabs.get(at.saturating_sub(1))).copied();
    if app.session.page().ok() == Some(page)
        && let Some(next) = next
    {
        select(app, next);
    }
    true
}

/// ⌘W: close the active tab.
pub fn close_active(app: &mut FigApp) -> bool {
    match app.session.page() {
        Ok(page) => close(app, page),
        Err(_) => false,
    }
}

/// Close every tab but `keep`'s, and open it.
pub fn close_others(app: &mut FigApp, keep: NodeId) {
    follow(app);
    let Some(key) = key(app) else { return };
    if let Some(tabs) = app.page_tabs.maps.get_mut(&key) {
        tabs.retain(|id| *id == keep);
    }
    select(app, keep);
}

/// The cells' widths in `room` points: as wide as their labels when they fit, else the widest
/// shrink to one shared width, never below [`MIN_CELL`] (the strip scrolls past that).
pub fn layout(natural: &[f32], room: f32) -> Vec<f32> {
    let total: f32 = natural.iter().sum();
    if total <= room || natural.is_empty() {
        return natural.to_vec();
    }
    let mut sorted = natural.to_vec();
    sorted.sort_by(f32::total_cmp);
    let mut rest = room.max(0.0);
    let mut cap = MIN_CELL;
    for (i, w) in sorted.iter().enumerate() {
        let share = rest / (sorted.len() - i) as f32;
        if *w > share {
            cap = share;
            break;
        }
        rest -= w;
    }
    let cap = cap.max(MIN_CELL);
    natural.iter().map(|w| w.min(cap).max(MIN_CELL.min(*w))).collect()
}

/// Where a cell's label starts: its inset, on the cell's centre line.
pub fn label_pos(cell: Rect) -> Pos2 {
    pos2(cell.left() + PAD, cell.center().y)
}

/// The active cell's underline: the accent along its bottom edge.
pub fn underline(cell: Rect) -> Rect {
    Rect::from_min_max(pos2(cell.left(), cell.bottom() - UNDERLINE), cell.right_bottom())
}

/// A tab's × button, in its tail on the centre line.
pub fn close_rect(cell: Rect) -> Rect {
    Rect::from_center_size(pos2(cell.right() - CLOSE_INSET, cell.center().y), egui::vec2(CLOSE, CLOSE))
}

/// One flat cell, full height: inactive ones on the soft fill in muted text, the active one white
/// in ink with the underline.
fn paint_cell(ui: &Ui, cell: Rect, clip: Rect, label: &str, active: bool, hovered: bool, closable: bool) {
    let painter = ui.painter().with_clip_rect(clip);
    let fill = if active {
        ACTIVE
    } else if hovered {
        HOVER
    } else {
        INACTIVE
    };
    painter.rect_filled(cell, egui::CornerRadius::ZERO, fill);
    let text = if active || hovered { INK } else { MUTED };
    // The label stops before the × when the cell shows one.
    let end = if closable { close_rect(cell).left() - 4.0 } else { cell.right() - 6.0 };
    let label_clip = Rect::from_min_max(cell.min, pos2(end, cell.bottom())).intersect(clip);
    ui.painter().with_clip_rect(label_clip).text(label_pos(cell), Align2::LEFT_CENTER, label, font(), text);
    if active {
        painter.rect_filled(underline(cell), egui::CornerRadius::ZERO, ACCENT);
    }
}

enum Act {
    Select(NodeId),
    Close(NodeId),
    CloseOthers(NodeId),
    NewPage,
}

/// The tab strip in `rect` (the title row from the sidebar toggle to the Sync button).
pub fn strip(app: &mut FigApp, ui: &mut Ui, rect: Rect) {
    follow(app);
    let current = app.session.page().ok();
    let tabs: Vec<(NodeId, String)> = match app.session.doc() {
        Ok(doc) => open(app).into_iter().filter_map(|id| doc.node(id).map(|node| (id, node.name.clone()))).collect(),
        Err(_) => Vec::new(),
    };
    let page_count = app.session.doc().map(|doc| doc.pages.len()).unwrap_or(0);
    let measure = |text: &str| ui.painter().layout_no_wrap(text.to_string(), font(), INK).size().x;
    let natural: Vec<f32> = tabs.iter().map(|(_, name)| (PAD + measure(name) + TAIL).ceil()).collect();
    let plus_w = (PAD + measure("+ Page") + TAIL).ceil();
    let separators = SEPARATOR * tabs.len() as f32;
    let widths = layout(&natural, rect.width() - plus_w - separators);
    let content: f32 = widths.iter().sum::<f32>() + separators + plus_w;
    let overflow = (content - rect.width()).max(0.0);

    // Scrolling: the wheel over the strip, and the active tab into view when it changes.
    let state = &mut app.page_tabs;
    if overflow > 0.0 {
        if ui.rect_contains_pointer(rect) {
            let delta = ui.input(|i| i.smooth_scroll_delta);
            state.scroll -= if delta.x != 0.0 { delta.x } else { delta.y };
        }
        if state.shown != current {
            let mut x = 0.0;
            for ((id, _), w) in tabs.iter().zip(&widths) {
                if Some(*id) == current {
                    if x < state.scroll {
                        state.scroll = x;
                    } else if x + w > state.scroll + rect.width() {
                        state.scroll = x + w - rect.width();
                    }
                }
                x += w + SEPARATOR;
            }
        }
    }
    state.shown = current;
    state.scroll = state.scroll.clamp(0.0, overflow);
    let scroll = state.scroll;

    let mut act = None;
    let mut cells = Vec::with_capacity(tabs.len() + 1);
    let mut x = rect.left() - scroll;
    let rule = Stroke::new(SEPARATOR, RULE);
    for ((id, name), w) in tabs.iter().zip(&widths) {
        let cell = Rect::from_min_max(pos2(x, rect.top()), pos2(x + w, rect.bottom()));
        let hit = cell.intersect(rect);
        let resp = ui.interact(hit, Id::new(("map_tab", id.0)), Sense::click());
        let active = current == Some(*id);
        let over = ui.rect_contains_pointer(hit);
        let several = tabs.len() > 1;
        // The ×: on the hovered tab only, the last one too.
        paint_cell(ui, cell, rect, name, active, over && !active, over);
        if over {
            let button = close_rect(cell);
            let close = ui.interact(button.intersect(rect), Id::new(("map_tab_close", id.0)), Sense::click());
            let painter = ui.painter().with_clip_rect(rect);
            if close.hovered() {
                painter.rect_filled(button, egui::CornerRadius::same(3), CLOSE_HOVER);
            }
            let ink = if close.hovered() { INK } else { MUTED };
            let c = button.center();
            let stroke = Stroke::new(1.0, ink);
            painter.line_segment([pos2(c.x - CROSS, c.y - CROSS), pos2(c.x + CROSS, c.y + CROSS)], stroke);
            painter.line_segment([pos2(c.x + CROSS, c.y - CROSS), pos2(c.x - CROSS, c.y + CROSS)], stroke);
            crate::a11y::button(&close, &format!("Close tab {name}"));
            if close.on_hover_text("Close tab").clicked() {
                act = Some(Act::Close(*id));
            }
        }
        ui.painter().with_clip_rect(rect).vline(cell.right() + SEPARATOR / 2.0, rect.y_range(), rule);
        crate::a11y::button(&resp, &format!("Page tab {name}"));
        if resp.middle_clicked() {
            act = Some(Act::Close(*id));
        } else if resp.clicked() && act.is_none() {
            act = Some(Act::Select(*id));
        }
        resp.context_menu(|ui| {
            if ui.button("Close tab").clicked() {
                act = Some(Act::Close(*id));
                ui.close();
            }
            if ui.add_enabled(several, egui::Button::new("Close other tabs")).clicked() {
                act = Some(Act::CloseOthers(*id));
                ui.close();
            }
        });
        if w + 0.5 < natural.get(cells.len()).copied().unwrap_or(0.0) {
            resp.on_hover_text(name);
        }
        cells.push((Some(*id), cell));
        x = cell.right() + SEPARATOR;
    }
    let plus = Rect::from_min_max(pos2(x, rect.top()), pos2(x + plus_w, rect.bottom()));
    let resp = ui.interact(plus.intersect(rect), Id::new("map_tab_new_page"), Sense::click());
    paint_cell(ui, plus, rect, "+ Page", false, resp.hovered(), false);
    crate::a11y::button(&resp, "New page");
    if resp.on_hover_text("New page").clicked() {
        act = Some(Act::NewPage);
    }
    cells.push((None, plus));
    if tabs.is_empty() {
        // Every tab closed: say how to open a page again.
        let at = pos2(plus.right() + 12.0, rect.center().y);
        ui.painter().with_clip_rect(rect).text(at, Align2::LEFT_CENTER, "No page open · pick one in the tree", font(), MUTED);
    }
    app.page_tabs.cells = cells;

    match act {
        Some(Act::Select(id)) => select(app, id),
        Some(Act::Close(id)) => {
            close(app, id);
        }
        Some(Act::CloseOthers(id)) => close_others(app, id),
        Some(Act::NewPage) => app.new_page.start(page_count + 1),
        None => {}
    }
    if act.is_some() {
        ui.ctx().request_repaint();
    }
}

/// The tabs as `ui.inspect` reports them.
pub fn inspect(app: &FigApp) -> Value {
    json!({
        "open": open(app).iter().map(|id| id.0).collect::<Vec<_>>(),
        "empty": is_empty(app),
        "active": app.session.page().ok().map(|id| id.0),
        "scroll": app.page_tabs.scroll,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_keep_their_width_until_they_must_shrink() {
        assert_eq!(layout(&[80.0, 100.0], 400.0), vec![80.0, 100.0]);
        // Too wide: the widest give way first, to one shared width.
        assert_eq!(layout(&[60.0, 200.0, 200.0], 300.0), vec![60.0, 120.0, 120.0]);
        // Never under the minimum; past that the strip scrolls.
        assert_eq!(layout(&[200.0, 200.0, 200.0], 30.0), vec![MIN_CELL; 3]);
        // A label shorter than the minimum keeps its width.
        assert_eq!(layout(&[40.0, 400.0], 50.0), vec![40.0, MIN_CELL]);
    }

    #[test]
    fn label_sits_on_the_centre_line_and_underline_on_the_bottom_edge() {
        let cell = Rect::from_min_max(pos2(100.0, 0.0), pos2(180.0, 44.0));
        assert_eq!(label_pos(cell), pos2(110.0, 22.0));
        let line = underline(cell);
        assert_eq!(line.bottom(), cell.bottom());
        assert_eq!(line.height(), UNDERLINE);
        assert_eq!(line.x_range(), cell.x_range());
    }
}
