//! Full-height Pages and Sync sidebars. Requested widths live in preferences; fitted widths are
//! shared by the panels, canvas and composer without overwriting those preferences.

use crate::FigApp;

/// The sidebar's widths: Herdr's sidebar is 268 pt wide by default.
pub const SIDEBAR: std::ops::RangeInclusive<f32> = 220.0..=480.0;
pub const SIDEBAR_DEFAULT: f32 = 268.0;
pub const SYNC_DEFAULT: f32 = 360.0;
/// What the canvas keeps while Sync is open (or the whole window, if it is smaller).
pub const CANVAS_MIN: f32 = 240.0;

fn within(w: Option<f32>, range: &std::ops::RangeInclusive<f32>, default: f32) -> f32 {
    w.filter(|w| w.is_finite()).unwrap_or(default).clamp(*range.start(), *range.end())
}

/// The sidebar width asked for (the preference, within its limits).
pub fn sidebar_width(app: &FigApp) -> f32 {
    within(app.session.prefs.map_sidebar_width, &SIDEBAR, SIDEBAR_DEFAULT)
}

/// The Sync width asked for, independently of whether either sidebar is open.
pub fn sync_width(app: &FigApp) -> f32 {
    within(app.session.prefs.map_sync_width, &SIDEBAR, SYNC_DEFAULT)
}

/// Whether the chrome around the canvas shows (not in focus mode).
fn chrome(app: &FigApp) -> bool {
    app.session.ds().is_ok_and(|ds| !ds.view.ui_hidden)
}

/// Whether the left sidebar is open beside the canvas.
pub fn sidebar_open(app: &FigApp) -> bool {
    app.ws.pane_open && chrome(app)
}

/// Sync can remain open independently of Pages, but needs a workspace and visible chrome.
pub fn sync_open(app: &FigApp) -> bool {
    app.sync.open && app.active_workspace.is_some() && chrome(app)
}

/// The width the sidebar draws at in a window `window` wide, asked for `left` (`None`: closed,
/// 0). The canvas keeps [`CANVAS_MIN`], the sidebar never goes below its minimum.
pub fn fit(window: f32, left: Option<f32>) -> f32 {
    let window = if window.is_finite() { window.max(0.0) } else { 0.0 };
    left.map_or(0.0, |l| within(Some(l), &SIDEBAR, SIDEBAR_DEFAULT).min((window - CANVAS_MIN).max(*SIDEBAR.start())))
}

/// Fit both requested widths together. Pages alone retains its existing minimum-width behavior.
/// With both open, preserve the canvas, sharing spare space in proportion to each panel's width
/// above its minimum. If even the minima do not fit, split the remaining space equally.
fn fit_widths(window: f32, left: Option<f32>, right: Option<f32>) -> (f32, f32) {
    let left = left.map(|w| within(Some(w), &SIDEBAR, SIDEBAR_DEFAULT));
    let right = right.map(|w| within(Some(w), &SIDEBAR, SYNC_DEFAULT));
    let window = if window.is_finite() { window.max(0.0) } else { 0.0 };
    let available = (window - CANVAS_MIN).max(0.0);
    let (Some(left), Some(right)) = (left, right) else {
        return (fit(window, left), right.map_or(0.0, |r| r.min(available)));
    };
    if left + right <= available {
        return (left, right);
    }
    let minimum = *SIDEBAR.start();
    if available <= 2.0 * minimum {
        return (available / 2.0, available / 2.0);
    }
    let extra = left + right - 2.0 * minimum;
    let left = minimum + (available - 2.0 * minimum) * ((left - minimum) / extra);
    (left, available - left)
}

/// Fitted (Pages, Sync) widths for the full window, with zero for a closed panel.
pub fn widths(app: &FigApp, window: f32) -> (f32, f32) {
    fit_widths(window, sidebar_open(app).then(|| sidebar_width(app)), sync_open(app).then(|| sync_width(app)))
}

/// What the left sidebar draws at in this window, coordinated with Sync; 0 when closed.
pub fn width(app: &FigApp, window: f32) -> f32 {
    widths(app, window).0
}

/// Where the canvas column starts, from the window's left edge: what the docked composer and the
/// first-map banner span from.
pub fn canvas_left(app: &FigApp, ctx: &egui::Context) -> f32 {
    width(app, ctx.content_rect().width())
}

/// The canvas column's inset from the window's right edge (not an absolute coordinate).
pub fn canvas_right(app: &FigApp, ctx: &egui::Context) -> f32 {
    widths(app, ctx.content_rect().width()).1
}

/// Ask for a sidebar width (kept within its limits; saved with the preferences).
pub fn set_sidebar_width(app: &mut FigApp, w: f32) {
    app.session.prefs.map_sidebar_width = Some(within(Some(w), &SIDEBAR, SIDEBAR_DEFAULT));
}

/// Remember the requested Sync width, even when the current window needs a smaller fitted width.
pub fn set_sync_width(app: &mut FigApp, w: f32) {
    app.session.prefs.map_sync_width = Some(within(Some(w), &SIDEBAR, SYNC_DEFAULT));
}

/// A pane's fitted inner edge at `x`, down `rows`. Pass the saved requested width from
/// [`sidebar_width`], not the fitted width. Dragging adds pointer displacement to the request at
/// drag start; double-click restores `default`. Returns a new requested width when it changes.
/// Constrained panels move monotonically, but may move less than the pointer while fitting.
pub fn splitter(ui: &mut egui::Ui, name: &str, x: f32, rows: egui::Rangef, requested_width: f32, default: f32) -> Option<f32> {
    splitter_edge(ui, name, x, rows, requested_width, default, 1.0)
}

/// The right pane's inner edge: dragging left grows it; double-click restores `default`.
/// Pass the fitted edge coordinate `x` and the saved requested width from [`sync_width`].
pub fn splitter_right(ui: &mut egui::Ui, name: &str, x: f32, rows: egui::Rangef, requested_width: f32, default: f32) -> Option<f32> {
    splitter_edge(ui, name, x, rows, requested_width, default, -1.0)
}

fn splitter_edge(ui: &mut egui::Ui, name: &str, x: f32, rows: egui::Rangef, requested_width: f32, default: f32, direction: f32) -> Option<f32> {
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
    // Keep the original request and pointer position: fitting moves x by less than the pointer
    // in a constrained window. Feeding the fitted width back into the request would jump or
    // reverse the divider; accumulating displacement each frame would drift with a still pointer.
    let at = resp.interact_pointer_pos().filter(|_| resp.dragged())?;
    let drag_id = id.with("requested_width_at_press");
    if resp.drag_started() {
        let pressed = ui.input(|i| i.pointer.press_origin()).unwrap_or(at);
        ui.data_mut(|d| d.insert_temp(drag_id, (pressed.x, within(Some(requested_width), &SIDEBAR, default))));
    }
    let (pressed_x, original_width): (f32, f32) = ui.data(|d| d.get_temp(drag_id))?;
    let w = original_width + direction * (at.x - pressed_x);
    if !w.is_finite() {
        return None;
    }
    let w = within(Some(w), &SIDEBAR, default);
    (w != requested_width).then_some(w)
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

    #[test]
    fn panels_fit_together_and_keep_the_single_panel_behavior() {
        assert_eq!(fit_widths(1440.0, Some(268.0), Some(360.0)), (268.0, 360.0));
        assert_eq!(fit_widths(1440.0, None, None), (0.0, 0.0));
        assert_eq!(fit_widths(1440.0, None, Some(360.0)), (0.0, 360.0));
        assert_eq!(fit_widths(1440.0, Some(268.0), None), (268.0, 0.0));
        assert_eq!(fit_widths(830.0, Some(320.0), Some(420.0)), (270.0, 320.0));
        assert_eq!(fit_widths(680.0, Some(268.0), Some(360.0)), (220.0, 220.0));
        assert_eq!(fit_widths(600.0, Some(268.0), Some(360.0)), (180.0, 180.0));
        assert_eq!(fit_widths(300.0, Some(268.0), Some(360.0)), (30.0, 30.0));
        assert_eq!(fit_widths(200.0, Some(268.0), Some(360.0)), (0.0, 0.0));
        assert_eq!(fit_widths(300.0, None, Some(360.0)), (0.0, 60.0));
        assert_eq!(fit_widths(200.0, None, Some(360.0)), (0.0, 0.0));
        for window in [0.0, 200.0, 300.0, 600.0, 1440.0] {
            assert_eq!(fit_widths(window, Some(450.0), None), (fit(window, Some(450.0)), 0.0));
        }
    }

    #[test]
    fn invalid_geometry_is_finite_and_two_panels_never_consume_the_canvas() {
        let requests = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -10.0, 0.0, 220.0, 268.0, 360.0, 480.0, f32::MAX];
        for window in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, 240.0, 300.0, 679.0, 680.0, 681.0, 800.0, 1440.0] {
            for left in requests {
                for right in requests {
                    let (l, r) = fit_widths(window, Some(left), Some(right));
                    assert!(l.is_finite() && r.is_finite());
                    assert!(l >= 0.0 && r >= 0.0);
                    assert!(l <= within(Some(left), &SIDEBAR, SIDEBAR_DEFAULT) + 0.001);
                    assert!(r <= within(Some(right), &SIDEBAR, SYNC_DEFAULT) + 0.001);
                    let available = if window.is_finite() { (window - CANVAS_MIN).max(0.0) } else { 0.0 };
                    assert!(l + r <= available + 0.001);
                    let (closed, only_right) = fit_widths(window, None, Some(right));
                    assert_eq!(closed, 0.0);
                    assert!(only_right.is_finite() && only_right >= 0.0 && only_right <= available);
                    let (mirrored_r, mirrored_l) = fit_widths(window, Some(r.max(220.0)), Some(l.max(220.0)));
                    assert!((mirrored_l - l).abs() < 0.001 && (mirrored_r - r).abs() < 0.001);
                }
            }
        }
        assert_eq!(fit_widths(1440.0, Some(f32::NAN), Some(f32::NAN)), (SIDEBAR_DEFAULT, SYNC_DEFAULT));
        assert_eq!(fit_widths(1440.0, None, Some(f32::NAN)), (0.0, SYNC_DEFAULT));
        assert!(fit(f32::NAN, Some(f32::NAN)).is_finite());
    }

    fn app() -> FigApp {
        FigApp::new(figcraft_engine::Session::new(), Box::new(crate::NoServices))
    }

    #[test]
    fn sync_visibility_is_independent_of_pages_and_requires_workspace_and_chrome() {
        let mut app = app();
        app.ws.pane_open = true;
        app.sync.open = true;
        assert!(!sync_open(&app));
        assert_eq!(widths(&app, 1440.0), (SIDEBAR_DEFAULT, 0.0));
        app.active_workspace = Some("geometry-test".into());
        assert!(sync_open(&app));
        assert_eq!(widths(&app, 1440.0), (SIDEBAR_DEFAULT, SYNC_DEFAULT));
        app.ws.pane_open = false;
        assert!(sync_open(&app));
        assert_eq!(widths(&app, 1440.0), (0.0, SYNC_DEFAULT));
        app.session.ds_mut().unwrap().view.ui_hidden = true;
        assert!(!sync_open(&app));
        assert_eq!(widths(&app, 1440.0), (0.0, 0.0));
        app.session.ds_mut().unwrap().view.ui_hidden = false;
        app.sync.open = false;
        assert_eq!(widths(&app, 1440.0), (0.0, 0.0));
    }

    #[test]
    fn sync_width_is_remembered_across_fit_close_and_preference_round_trip() {
        let mut app = app();
        app.active_workspace = Some("geometry-test".into());
        app.ws.pane_open = true;
        app.sync.open = true;
        assert_eq!(sync_width(&app), SYNC_DEFAULT);
        set_sidebar_width(&mut app, 300.0);
        set_sync_width(&mut app, 420.0);
        assert_eq!(widths(&app, 600.0), (180.0, 180.0));
        assert_eq!(width(&app, 600.0), 180.0);
        assert_eq!(sync_width(&app), 420.0);
        app.sync.open = false;
        assert_eq!(widths(&app, 1440.0), (300.0, 0.0));
        app.session.prefs = serde_json::from_str(&serde_json::to_string(&app.session.prefs).unwrap()).unwrap();
        assert_eq!(app.session.prefs.map_sync_width, Some(420.0));
        app.sync.open = true;
        assert_eq!(widths(&app, 1440.0), (300.0, 420.0));
        let ctx = egui::Context::default();
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0))), ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| {
            assert_eq!(canvas_left(&app, ui.ctx()), 300.0);
            assert_eq!(canvas_right(&app, ui.ctx()), 420.0);
        });
        output.textures_delta.clear();
        for (requested, expected) in [(10.0, 220.0), (9000.0, 480.0), (f32::NAN, SYNC_DEFAULT), (f32::INFINITY, SYNC_DEFAULT)] {
            set_sync_width(&mut app, requested);
            assert_eq!(app.session.prefs.map_sync_width, Some(expected));
            assert_eq!(sync_width(&app), expected);
        }
    }

    #[test]
    fn old_preferences_default_to_no_saved_sync_width() {
        let old: figcraft_engine::Prefs = serde_json::from_str(r#"{"map_sidebar_width":312}"#).unwrap();
        assert_eq!(old.map_sidebar_width, Some(312.0));
        assert_eq!(old.map_sync_width, None);
        assert_eq!(figcraft_engine::Prefs::default().map_sync_width, None);
        assert!(serde_json::to_value(old).unwrap().get("map_sync_width").is_none());
    }

    fn right_splitter_frame(ctx: &egui::Context, width: &mut f32, time: &mut f64, events: Vec<egui::Event>) {
        *time += 0.05;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0))),
            time: Some(*time),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            if let Some(w) = splitter_right(ui, "sync-test", 1000.0 - *width, egui::Rangef::new(0.0, 700.0), *width, SYNC_DEFAULT) {
                *width = within(Some(w), &SIDEBAR, SYNC_DEFAULT);
            }
        });
        output.textures_delta.clear();
    }

    fn pointer_button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
    }

    #[test]
    fn right_splitter_drag_mirrors_left_and_keeps_the_grab_offset() {
        let ctx = egui::Context::default();
        let mut width = SYNC_DEFAULT;
        let mut time = 0.0;
        let from = egui::pos2(1000.0 - width + 2.0, 300.0);
        right_splitter_frame(&ctx, &mut width, &mut time, vec![]);
        right_splitter_frame(&ctx, &mut width, &mut time, vec![egui::Event::PointerMoved(from)]);
        right_splitter_frame(&ctx, &mut width, &mut time, vec![pointer_button(from, true)]);
        for dx in [-40.0, -60.0, -20.0] {
            let at = from + egui::vec2(dx, 0.0);
            right_splitter_frame(&ctx, &mut width, &mut time, vec![egui::Event::PointerMoved(at)]);
            assert_eq!(width, SYNC_DEFAULT - dx);
        }
        right_splitter_frame(&ctx, &mut width, &mut time, vec![pointer_button(from - egui::vec2(20.0, 0.0), false)]);
        assert_eq!(width, SYNC_DEFAULT + 20.0);
    }

    #[test]
    fn right_splitter_double_click_resets_to_design_default() {
        let ctx = egui::Context::default();
        let mut width = 420.0;
        let mut time = 0.0;
        let at = egui::pos2(1000.0 - width, 300.0);
        right_splitter_frame(&ctx, &mut width, &mut time, vec![]);
        right_splitter_frame(&ctx, &mut width, &mut time, vec![egui::Event::PointerMoved(at)]);
        for _ in 0..2 {
            right_splitter_frame(&ctx, &mut width, &mut time, vec![pointer_button(at, true)]);
            right_splitter_frame(&ctx, &mut width, &mut time, vec![pointer_button(at, false)]);
        }
        assert_eq!(width, SYNC_DEFAULT);
    }

    fn constrained_splitter_frame(ctx: &egui::Context, app: &mut FigApp, window: f32, right: bool, time: &mut f64, events: Vec<egui::Event>) {
        *time += 0.05;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(window, 700.0))),
            time: Some(*time),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let (left_w, right_w) = widths(app, window);
            let rows = egui::Rangef::new(0.0, 700.0);
            if right {
                if let Some(w) = splitter_right(ui, "constrained-sync", window - right_w, rows, sync_width(app), SYNC_DEFAULT) {
                    set_sync_width(app, w);
                }
            } else if let Some(w) = splitter(ui, "constrained-pages", left_w, rows, sidebar_width(app), SIDEBAR_DEFAULT) {
                set_sidebar_width(app, w);
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn constrained_drags_preserve_requests_and_reach_both_limits_without_drifting() {
        for window in [600.0, 800.0] {
            for right in [false, true] {
                let ctx = egui::Context::default();
                let mut app = app();
                app.active_workspace = Some("constrained-drag-test".into());
                app.ws.pane_open = true;
                app.sync.open = true;
                let requested = |app: &FigApp| if right { sync_width(app) } else { sidebar_width(app) };
                let fitted = |app: &FigApp| {
                    let (left_w, right_w) = widths(app, window);
                    if right { right_w } else { left_w }
                };
                let initial_request = requested(&app);
                let initial_fit = fitted(&app);
                let edge = if right { window - initial_fit } else { initial_fit };
                let from = egui::pos2(edge + 2.0, 300.0);
                let mut time = 0.0;
                constrained_splitter_frame(&ctx, &mut app, window, right, &mut time, vec![]);
                constrained_splitter_frame(&ctx, &mut app, window, right, &mut time, vec![egui::Event::PointerMoved(from)]);
                constrained_splitter_frame(&ctx, &mut app, window, right, &mut time, vec![pointer_button(from, true)]);
                assert_eq!(requested(&app), initial_request);
                assert_eq!(fitted(&app), initial_fit);
                // A first 20 pt growth must never shrink the divider. Subsequent movement reaches
                // both preference limits, reverses direction, then returns to the original size.
                let mut previous_request = initial_request;
                let mut previous_fit = initial_fit;
                for growth in [20.0, 60.0, 300.0, -300.0, 0.0, 40.0] {
                    let dx = if right { -growth } else { growth };
                    let at = from + egui::vec2(dx, 0.0);
                    constrained_splitter_frame(&ctx, &mut app, window, right, &mut time, vec![egui::Event::PointerMoved(at)]);
                    let expected = (initial_request + growth).clamp(*SIDEBAR.start(), *SIDEBAR.end());
                    assert_eq!(requested(&app), expected);
                    let actual_fit = fitted(&app);
                    if expected > previous_request {
                        assert!(actual_fit >= previous_fit);
                    } else {
                        assert!(actual_fit <= previous_fit);
                    }
                    // Holding the pointer still must not keep adding the outstanding distance
                    // between the pointer and the fitted edge.
                    for _ in 0..3 {
                        constrained_splitter_frame(&ctx, &mut app, window, right, &mut time, vec![]);
                        assert_eq!(requested(&app), expected);
                        assert_eq!(fitted(&app), actual_fit);
                    }
                    if growth == 0.0 {
                        assert_eq!(actual_fit, initial_fit);
                    }
                    previous_request = expected;
                    previous_fit = actual_fit;
                }
                let end = from + egui::vec2(if right { -40.0 } else { 40.0 }, 0.0);
                constrained_splitter_frame(&ctx, &mut app, window, right, &mut time, vec![pointer_button(end, false)]);
                // Returning to a wide window restores exactly the request, not the constrained fit.
                let wide = widths(&app, 1440.0);
                assert_eq!(if right { wide.1 } else { wide.0 }, initial_request + 40.0);
                // A new drag must take its baseline from the new request, not stale drag state.
                let edge = if right { window - fitted(&app) } else { fitted(&app) };
                let from = egui::pos2(edge + 2.0, 300.0);
                constrained_splitter_frame(&ctx, &mut app, window, right, &mut time, vec![egui::Event::PointerMoved(from)]);
                constrained_splitter_frame(&ctx, &mut app, window, right, &mut time, vec![pointer_button(from, true)]);
                let at = from + egui::vec2(if right { -20.0 } else { 20.0 }, 0.0);
                constrained_splitter_frame(&ctx, &mut app, window, right, &mut time, vec![egui::Event::PointerMoved(at)]);
                assert_eq!(requested(&app), initial_request + 60.0);
            }
        }
    }
}
