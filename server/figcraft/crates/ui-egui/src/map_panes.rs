//! The map as IDE panes: the left sidebar and the right comments pane around the canvas. Each is
//! as wide as its inner edge was dragged (kept in the preferences), within limits, and less in a
//! window too small for both and the canvas. The comments pane folds to a rail from its header.

use crate::FigApp;

/// The left sidebar's widths: the design draws 300 pt.
pub const SIDEBAR: std::ops::RangeInclusive<f32> = 220.0..=520.0;
pub const SIDEBAR_DEFAULT: f32 = 300.0;
/// The comments pane's widths.
pub const COMMENTS: std::ops::RangeInclusive<f32> = 220.0..=480.0;
pub const COMMENTS_DEFAULT: f32 = 260.0;
/// The folded comments pane: a rail with the button that unfolds it.
pub const RAIL: f32 = 28.0;
/// What the canvas keeps however the panes are dragged.
pub const CANVAS_MIN: f32 = 240.0;

fn within(w: Option<f32>, range: &std::ops::RangeInclusive<f32>, default: f32) -> f32 {
    w.filter(|w| w.is_finite()).unwrap_or(default).clamp(*range.start(), *range.end())
}

/// The sidebar width asked for (the preference, within its limits).
pub fn sidebar_width(app: &FigApp) -> f32 {
    within(app.session.prefs.map_sidebar_width, &SIDEBAR, SIDEBAR_DEFAULT)
}

/// The comments pane width asked for (the preference, within its limits).
pub fn comments_width(app: &FigApp) -> f32 {
    within(app.session.prefs.map_comments_width, &COMMENTS, COMMENTS_DEFAULT)
}

/// Whether the chrome around the canvas shows (not in focus mode).
fn chrome(app: &FigApp) -> bool {
    app.session.ds().is_ok_and(|ds| !ds.view.ui_hidden)
}

/// Whether the left sidebar is open beside the canvas.
pub fn sidebar_open(app: &FigApp) -> bool {
    app.ws.pane_open && chrome(app)
}

/// Whether the comments pane is beside the canvas: comments are shown (View ▸ Comments, Shift+C,
/// which also shows their marks on the canvas) and the chrome is.
pub fn comments_open(app: &FigApp) -> bool {
    chrome(app) && app.session.ds().is_ok_and(|ds| ds.view.comments)
}

/// The widths the panes draw at in a window `window` wide: `left` and `right` are the widths asked
/// for (`None`: the pane is closed; `right` may be the rail). The canvas keeps [`CANVAS_MIN`];
/// the sidebar gives way first, then the comments pane, never below their minimum.
pub fn fit(window: f32, left: Option<f32>, right: Option<f32>) -> (f32, f32) {
    let window = if window.is_finite() { window.max(0.0) } else { 0.0 };
    let right_w = right.unwrap_or(0.0);
    let left_w = left.map_or(0.0, |l| l.min((window - right_w - CANVAS_MIN).max(*SIDEBAR.start())));
    let right_min = if right_w <= RAIL { right_w } else { *COMMENTS.start() };
    let right_w = right.map_or(0.0, |r| r.min((window - left_w - CANVAS_MIN).max(right_min)));
    (left_w, right_w)
}

/// What the panes draw at in this window: (sidebar, comments pane), 0 for a closed one.
pub fn widths(app: &FigApp, window: f32) -> (f32, f32) {
    let left = sidebar_open(app).then(|| sidebar_width(app));
    let right = comments_open(app).then(|| if app.session.prefs.map_comments_collapsed { RAIL } else { comments_width(app) });
    fit(window, left, right)
}

/// The canvas column between the panes, as x offsets from the window's left and right edges:
/// what the docked composer spans.
pub fn canvas_insets(app: &FigApp, ctx: &egui::Context) -> (f32, f32) {
    widths(app, ctx.content_rect().width())
}

/// Ask for a sidebar width (kept within its limits; saved with the preferences).
pub fn set_sidebar_width(app: &mut FigApp, w: f32) {
    app.session.prefs.map_sidebar_width = Some(within(Some(w), &SIDEBAR, SIDEBAR_DEFAULT));
}

/// Ask for a comments pane width (kept within its limits; saved with the preferences).
pub fn set_comments_width(app: &mut FigApp, w: f32) {
    app.session.prefs.map_comments_width = Some(within(Some(w), &COMMENTS, COMMENTS_DEFAULT));
}

/// A pane's inner edge at `x`, down `rows`: dragging it asks for `width` plus the move (`grows`
/// +1 for the left pane, −1 for the right one); a double-click puts back `default`. Returns the
/// width asked for, when it changed.
pub fn splitter(ui: &mut egui::Ui, name: &str, x: f32, rows: egui::Rangef, width: f32, grows: f32, default: f32) -> Option<f32> {
    let grab = ui.style().interaction.resize_grab_radius_side;
    let rect = egui::Rect::from_x_y_ranges(x - grab..=x + grab, rows);
    let id = egui::Id::new(("map_splitter", name));
    let resp = ui.interact(rect, id, egui::Sense::click_and_drag());
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        ui.painter().vline(x, rows, egui::Stroke::new(2.0, egui::Color32::from_rgb(0xB5, 0x45, 0x2F)));
    }
    if resp.double_clicked() {
        return Some(default);
    }
    // The edge stays under the pointer where it was grabbed.
    let at = resp.interact_pointer_pos().filter(|_| resp.dragged())?;
    if resp.drag_started() {
        let pressed = ui.input(|i| i.pointer.press_origin()).unwrap_or(at);
        ui.data_mut(|d| d.insert_temp(id, pressed.x - x));
    }
    let grabbed: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(0.0);
    let w = width + (at.x - grabbed - x) * grows;
    (w.is_finite() && w != width).then_some(w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panes_keep_the_canvas_and_their_minimums() {
        assert_eq!(fit(1440.0, Some(300.0), Some(260.0)), (300.0, 260.0));
        assert_eq!(fit(1440.0, None, None), (0.0, 0.0));
        // A small window: the sidebar gives way first.
        let (l, r) = fit(800.0, Some(500.0), Some(260.0));
        assert_eq!(r, 260.0);
        assert_eq!(l, 800.0 - 260.0 - CANVAS_MIN);
        // Never below their minimum, even when the canvas would want more.
        assert_eq!(fit(300.0, Some(300.0), Some(300.0)), (*SIDEBAR.start(), *COMMENTS.start()));
        // The rail stays a rail.
        assert_eq!(fit(300.0, None, Some(RAIL)).1, RAIL);
    }

    #[test]
    fn widths_asked_stay_within_limits() {
        assert_eq!(within(None, &SIDEBAR, SIDEBAR_DEFAULT), SIDEBAR_DEFAULT);
        assert_eq!(within(Some(f32::NAN), &SIDEBAR, SIDEBAR_DEFAULT), SIDEBAR_DEFAULT);
        assert_eq!(within(Some(10.0), &SIDEBAR, SIDEBAR_DEFAULT), *SIDEBAR.start());
        assert_eq!(within(Some(9000.0), &COMMENTS, COMMENTS_DEFAULT), *COMMENTS.end());
    }
}
