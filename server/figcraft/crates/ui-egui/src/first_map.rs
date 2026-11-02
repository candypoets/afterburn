//! The first map's on-map guidance, from the onboarding design's First map card.
//!
//! When Init opens a workspace's map, an agent starts drawing it and the screen stays out of the
//! way: one banner says which agent is drawing and counts the cards drawn so far (nothing knows
//! the total), and the Sync button explains itself once in a tooltip. The composer's hint line
//! already teaches the keys. The banner leaves when the agent stops working; the tooltip is shown
//! once ever and remembered in the Mindmap config folder.

use std::time::Duration;

use egui::{Align2, Color32, CornerRadius, Id, Margin, Pos2, Rect, Stroke, vec2};

use crate::FigApp;
use crate::theme::semibold;
use crate::workspaces::{Busy, Target};

const PANEL: Color32 = Color32::WHITE;
const INK: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x1A);
const MUTED: Color32 = Color32::from_rgb(0x6E, 0x6C, 0x68);
const ACCENT: Color32 = Color32::from_rgb(0xB5, 0x45, 0x2F);
const TIP: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x1A);

/// The banner's width, as drawn.
const BANNER_W: f32 = 520.0;
/// How long the Sync tooltip stays before it leaves on its own.
const TIP_SECS: f64 = 8.0;
/// A drawing agent the node never reports as working still lets the banner go after this long.
const BANNER_MAX_SECS: f64 = 20.0 * 60.0;
/// The file that remembers the Sync tooltip was shown.
#[cfg(not(target_arch = "wasm32"))]
const TIP_FILE: &str = "first-map.json";

/// An Init that is opening a map: which workspace, and the agent kind that will draw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub workspace: String,
    pub kind: String,
}

/// The map an agent is drawing for the first time.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawing {
    pub workspace: String,
    pub kind: String,
    /// The checkout the agent reads, when the node listed it.
    pub path: Option<String>,
    /// When the map opened (egui time).
    pub since: f64,
    /// The agent has been seen working; its next idle state ends the banner.
    pub seen_working: bool,
}

#[derive(Default)]
pub struct State {
    pub pending: Option<Pending>,
    pub drawing: Option<Drawing>,
    /// When the Sync tooltip started showing; `None` when it is not showing.
    tip_since: Option<f64>,
    /// The tooltip has had its one showing (read from disk once).
    tip_done: Option<bool>,
}

/// What the banner says under its title.
pub fn detail_line(path: Option<&str>, cards: usize) -> String {
    let cards = match cards {
        1 => "1 card so far".to_string(),
        n => format!("{n} cards so far"),
    };
    match path.map(str::trim).filter(|path| !path.is_empty()) {
        Some(path) => format!("reading {path}  ·  {cards}  ·  they appear as they're drawn"),
        None => format!("{cards}  ·  they appear as they're drawn"),
    }
}

/// Whether the drawing agent is done: it was seen working and now is not, or it took too long.
pub fn finished(drawing: &Drawing, working: Option<bool>, now: f64) -> bool {
    (drawing.seen_working && working == Some(false)) || now - drawing.since > BANNER_MAX_SECS
}

/// Watch Init: remember the workspace it opens, then start the banner once its map is in front.
pub fn observe(app: &mut FigApp, ctx: &egui::Context) {
    if let Some(Busy::Open { workspace, init: true, target }) = &app.ws.busy {
        let kind = match target {
            Some(Target::Existing { kind, .. } | Target::New { kind }) => kind.trim().to_string(),
            None => String::new(),
        };
        app.first_map.pending = Some(Pending { workspace: workspace.clone(), kind });
    }
    let now = ctx.input(|i| i.time);
    let opened = app.first_map.pending.as_ref().is_some_and(|pending| app.active_workspace.as_deref() == Some(pending.workspace.as_str()))
        && !app.ui.home
        && app.ws.busy.is_none();
    if opened && let Some(pending) = app.first_map.pending.take() {
        let path = app.ws.rows.iter().find(|row| row.id == pending.workspace).and_then(|row| row.path.clone());
        app.first_map.drawing = Some(Drawing { workspace: pending.workspace, kind: pending.kind, path, since: now, seen_working: false });
        if !tip_done(&mut app.first_map) {
            app.first_map.tip_since = Some(now);
        }
    }
    // Another workspace in front ends the banner: it belongs to the map being drawn.
    if app.first_map.drawing.as_ref().is_some_and(|drawing| app.active_workspace.as_deref() != Some(drawing.workspace.as_str())) {
        app.first_map.drawing = None;
    }
    let working = drawing_kind(app).map(|kind| {
        app.compose
            .targets
            .iter()
            .filter(|info| !info.new_agent && info.target.kind().trim().eq_ignore_ascii_case(&kind))
            .any(|info| info.status.trim() == "working")
    });
    if let Some(drawing) = app.first_map.drawing.as_mut() {
        if working == Some(true) {
            drawing.seen_working = true;
        }
        if finished(drawing, working, now) {
            app.first_map.drawing = None;
        }
    }
}

fn drawing_kind(app: &FigApp) -> Option<String> {
    let drawing = app.first_map.drawing.as_ref()?;
    // A New target names the kind; an Init with no choice reports whatever agent the workspace has.
    if !drawing.kind.is_empty() {
        return Some(drawing.kind.to_lowercase());
    }
    app.compose.targets.iter().find(|info| !info.new_agent).map(|info| info.target.kind().trim().to_lowercase())
}

/// Draw the banner over the top of the canvas, and the Sync tooltip under the button.
pub fn show(app: &mut FigApp, ctx: &egui::Context, header_h: f32) {
    if let Some(drawing) = app.first_map.drawing.clone() {
        let cards = page_cards(app);
        let kind = drawing_kind(app).unwrap_or_else(|| "The agent".to_string());
        // Centred over the canvas column, beside the sidebar.
        let pane = crate::map_panes::canvas_left(app, ctx);
        let width = BANNER_W.min((ctx.content_rect().width() - pane - 48.0).max(240.0));
        egui::Area::new(Id::new("first_map_banner"))
            .anchor(Align2::CENTER_TOP, vec2(pane / 2.0, header_h + 20.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, ACCENT))
                    .corner_radius(CornerRadius::same(8))
                    .shadow(egui::epaint::Shadow { offset: [0, 3], blur: 10, spread: 0, color: Color32::from_black_alpha(16) })
                    .inner_margin(Margin { left: 15, right: 15, top: 10, bottom: 10 })
                    .show(ui, |ui| {
                        ui.set_width(width - 30.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            ui.add(egui::Spinner::new().size(12.0).color(ACCENT));
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ui.label(egui::RichText::new(format!("{kind} is drawing the map")).font(semibold(12.0)).color(INK));
                                let detail = detail_line(drawing.path.as_deref(), cards);
                                ui.add(egui::Label::new(egui::RichText::new(detail).font(egui::FontId::monospace(9.0)).color(MUTED)).truncate());
                            });
                        });
                    });
            });
        ctx.request_repaint_after(Duration::from_millis(500));
    }
    tooltip(app, ctx);
}

/// `Turn map changes into code`, under the Sync button, once.
fn tooltip(app: &mut FigApp, ctx: &egui::Context) {
    let Some(since) = app.first_map.tip_since else { return };
    let now = ctx.input(|i| i.time);
    let dismissed = ctx.input(|i| i.pointer.any_pressed()) && now - since > 0.3;
    if now - since > TIP_SECS || dismissed {
        app.first_map.tip_since = None;
        remember_tip(&mut app.first_map);
        return;
    }
    let Some(button) = app.sync.button_rect else { return };
    let text = "Turn map changes into code";
    egui::Area::new(Id::new("first_map_sync_tip"))
        .pivot(Align2::RIGHT_TOP)
        .fixed_pos(tip_pos(button))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::NONE.fill(TIP).corner_radius(CornerRadius::same(6)).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                ui.label(egui::RichText::new(text).font(semibold(10.5)).color(Color32::WHITE));
            });
        });
    ctx.request_repaint_after(Duration::from_millis(250));
}

/// The tooltip's top-right corner: 6 pt under the button, flush with its right edge.
fn tip_pos(button: Rect) -> Pos2 {
    egui::pos2(button.max.x, button.max.y + 6.0)
}

/// The cards drawn on the open page so far: its top-level layers.
fn page_cards(app: &FigApp) -> usize {
    let Ok(doc) = app.session.doc() else { return 0 };
    let Ok(page) = app.session.page() else { return 0 };
    doc.node(page).map(|node| node.children.len()).unwrap_or(0)
}

fn tip_done(state: &mut State) -> bool {
    if let Some(done) = state.tip_done {
        return done;
    }
    let done = read_tip_done();
    state.tip_done = Some(done);
    done
}

fn remember_tip(state: &mut State) {
    state.tip_done = Some(true);
    write_tip_done();
}

#[cfg(not(target_arch = "wasm32"))]
fn tip_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config").join("mindmap").join(TIP_FILE))
}

#[cfg(not(target_arch = "wasm32"))]
fn read_tip_done() -> bool {
    tip_path()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|value| value["syncTip"].as_bool() == Some(true))
}

#[cfg(target_arch = "wasm32")]
fn read_tip_done() -> bool {
    false
}

/// Best effort: a failed write only means the tooltip shows once more.
#[cfg(not(target_arch = "wasm32"))]
fn write_tip_done() {
    let Some(path) = tip_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    if std::fs::write(&temp, br#"{"syncTip":true}"#).is_ok() {
        let _ = std::fs::rename(temp, path);
    }
}

#[cfg(target_arch = "wasm32")]
fn write_tip_done() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawing(seen_working: bool) -> Drawing {
        Drawing { workspace: "w1".into(), kind: "kimi".into(), path: Some("/code/acme".into()), since: 0.0, seen_working }
    }

    #[test]
    fn the_banner_counts_cards_without_a_total() {
        assert_eq!(detail_line(Some("/code/acme"), 4), "reading /code/acme  ·  4 cards so far  ·  they appear as they're drawn");
        assert_eq!(detail_line(None, 1), "1 card so far  ·  they appear as they're drawn");
        assert_eq!(detail_line(Some("  "), 0), "0 cards so far  ·  they appear as they're drawn");
    }

    #[test]
    fn the_banner_waits_for_the_agent_to_work_then_stop() {
        // Not yet seen working: an idle report is the agent still starting.
        assert!(!finished(&drawing(false), Some(false), 10.0));
        assert!(!finished(&drawing(true), Some(true), 10.0));
        assert!(finished(&drawing(true), Some(false), 10.0));
        // An unknown status keeps it until the cap.
        assert!(!finished(&drawing(false), None, 10.0));
        assert!(finished(&drawing(false), None, BANNER_MAX_SECS + 1.0));
    }

    #[test]
    fn the_tooltip_hangs_under_the_button_flush_right() {
        let button = Rect::from_min_max(egui::pos2(880.0, 8.0), egui::pos2(932.0, 34.0));
        assert_eq!(tip_pos(button), egui::pos2(932.0, 40.0));
    }
}
