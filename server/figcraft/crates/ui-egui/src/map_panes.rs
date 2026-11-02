//! The map's left sidebar beside the canvas: it runs the window's full height (as Herdr's does) and
//! is as wide as its inner edge was dragged (kept in the preferences), within limits, and less in a
//! window too small for it and the canvas.

use crate::FigApp;

/// The sidebar's widths: Herdr's sidebar is 268 pt wide by default.
pub const SIDEBAR: std::ops::RangeInclusive<f32> = 220.0..=480.0;
pub const SIDEBAR_DEFAULT: f32 = 268.0;
/// What the canvas keeps however the sidebar is dragged.
pub const CANVAS_MIN: f32 = 240.0;

fn within(w: Option<f32>, range: &std::ops::RangeInclusive<f32>, default: f32) -> f32 {
    w.filter(|w| w.is_finite()).unwrap_or(default).clamp(*range.start(), *range.end())
}

/// The sidebar width asked for (the preference, within its limits).
pub fn sidebar_width(app: &FigApp) -> f32 {
    within(app.session.prefs.map_sidebar_width, &SIDEBAR, SIDEBAR_DEFAULT)
}

/// Whether the chrome around the canvas shows (not in focus mode).
fn chrome(app: &FigApp) -> bool {
    app.session.ds().is_ok_and(|ds| !ds.view.ui_hidden)
}

/// Whether the left sidebar is open beside the canvas.
pub fn sidebar_open(app: &FigApp) -> bool {
    app.ws.pane_open && chrome(app)
}

/// The width the sidebar draws at in a window `window` wide, asked for `left` (`None`: closed,
/// 0). The canvas keeps [`CANVAS_MIN`], the sidebar never goes below its minimum.
pub fn fit(window: f32, left: Option<f32>) -> f32 {
    let window = if window.is_finite() { window.max(0.0) } else { 0.0 };
    left.map_or(0.0, |l| l.min((window - CANVAS_MIN).max(*SIDEBAR.start())))
}

/// What the sidebar draws at in this window, 0 when it is closed.
pub fn width(app: &FigApp, window: f32) -> f32 {
    fit(window, sidebar_open(app).then(|| sidebar_width(app)))
}

/// Where the canvas column starts, from the window's left edge: what the docked composer and the
/// first-map banner span from.
pub fn canvas_left(app: &FigApp, ctx: &egui::Context) -> f32 {
    width(app, ctx.content_rect().width())
}

/// Ask for a sidebar width (kept within its limits; saved with the preferences).
pub fn set_sidebar_width(app: &mut FigApp, w: f32) {
    app.session.prefs.map_sidebar_width = Some(within(Some(w), &SIDEBAR, SIDEBAR_DEFAULT));
}

/// A pane's inner edge at `x`, down `rows`: dragging it asks for `width` plus the move; a
/// double-click puts back `default`. Returns the width asked for, when it changed.
pub fn splitter(ui: &mut egui::Ui, name: &str, x: f32, rows: egui::Rangef, width: f32, default: f32) -> Option<f32> {
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
    let w = width + (at.x - grabbed - x);
    (w.is_finite() && w != width).then_some(w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sidebar_keeps_the_canvas_and_its_minimum() {
        assert_eq!(fit(1440.0, Some(300.0)), 300.0);
        assert_eq!(fit(1440.0, None), 0.0);
        // A small window: the sidebar gives way to the canvas.
        assert_eq!(fit(600.0, Some(450.0)), 600.0 - CANVAS_MIN);
        // Never below its minimum, even when the canvas would want more.
        assert_eq!(fit(300.0, Some(300.0)), *SIDEBAR.start());
    }

    #[test]
    fn widths_asked_stay_within_limits() {
        assert_eq!(within(None, &SIDEBAR, SIDEBAR_DEFAULT), SIDEBAR_DEFAULT);
        assert_eq!(within(Some(f32::NAN), &SIDEBAR, SIDEBAR_DEFAULT), SIDEBAR_DEFAULT);
        assert_eq!(within(Some(10.0), &SIDEBAR, SIDEBAR_DEFAULT), *SIDEBAR.start());
        assert_eq!(within(Some(9000.0), &SIDEBAR, SIDEBAR_DEFAULT), *SIDEBAR.end());
    }
}
