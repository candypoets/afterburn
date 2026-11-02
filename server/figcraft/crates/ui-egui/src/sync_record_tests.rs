//! An open record in the Sync panel: what it is, whose it is, how long it waited, and Cancel.

use egui::{Pos2, RawInput, Rect, vec2};
use figcraft_engine::Session;
use serde_json::json;

use crate::compose::TargetInfo;
use crate::sync::{DiffReply, Job, job_owner, open_title, waited};
use crate::workspaces::Target;
use crate::{FigApp, NoServices};

fn record(work: &str, pane: Option<&str>) -> Job {
    serde_json::from_value(json!({
        "id": "j1", "state": "in_flight", "kind": work, "started": "1000",
        "agent": {"kind": "external", "pane": pane},
    }))
    .unwrap()
}

fn claude(pane: &str) -> TargetInfo {
    TargetInfo {
        target: Target::Existing { pane: pane.into(), kind: "claude".into(), status: "working".into() },
        label: "small brain".into(),
        title: None,
        status: "working".into(),
        focused: false,
        new_agent: false,
    }
}

#[test]
fn an_open_record_says_what_it_is_whose_it_is_and_how_long_it_waited() {
    let job = record("explain", Some("wX:p2"));
    assert!(job.is_record());
    assert_eq!(open_title(&job), "Explanation record open");
    assert_eq!(open_title(&record("fix", None)), "Bug fix record open");
    assert_eq!(open_title(&record("sync", None)), "Sync in flight");
    assert_eq!(job_owner(&job, &[claude("wX:p2")]), "small brain · claude");
    assert_eq!(job_owner(&job, &[]), "pane wX:p2");
    assert_eq!(job_owner(&record("explain", None), &[]), "an agent outside Herdr");
    assert_eq!(waited("1000", Some(1030)), "opened just now");
    assert_eq!(waited("1000", Some(1000 + 12 * 60)), "waited 12 min");
    assert_eq!(waited("1000", Some(1000 + 125 * 60)), "waited 2 h 5 min");
    assert_eq!(waited("", Some(5)), "open");
}

#[test]
fn the_diff_reply_carries_the_open_job() {
    let reply: DiffReply = serde_json::from_value(json!({"ok": true, "open": {"id": "j1", "state": "in_flight", "kind": "fix"}})).unwrap();
    assert_eq!(reply.open.unwrap().work, "fix");
    let none: DiffReply = serde_json::from_value(json!({"ok": true, "open": null})).unwrap();
    assert!(none.open.is_none());
}

fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

#[test]
fn the_panel_shows_the_open_record_with_cancel_and_holds_sync() {
    let mut app = FigApp::new(Session::new(), Box::new(NoServices));
    app.active_workspace = Some("wX".to_string());
    app.compose.targets = vec![claude("wX:p2")];
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx, false, 0);
    let mut time = 0.0f64;
    let mut run = |app: &mut FigApp| {
        time += 1.0 / 60.0;
        let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), time: Some(time), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            crate::sync::poll(app, &ctx);
            if app.sync.open {
                egui::Panel::right("sync_test_pane")
                    .exact_size(360.0)
                    .resizable(false)
                    .show_separator_line(false)
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| crate::sync::pane(app, ui));
            }
        });
        out.textures_delta.clear();
        texts(&out.shapes)
    };
    run(&mut app);
    app.sync.open = true;
    app.sync.diff = Some(DiffReply { ok: true, open: Some(record("explain", Some("wX:p2"))), ..DiffReply::default() });
    run(&mut app);
    let shown = run(&mut app);
    for needle in ["Explanation record open", "Cancel record", "commits nothing", "Sync waits until the open record is committed or cancelled."] {
        assert!(shown.iter().any(|text| text == needle), "{needle:?} not in {shown:?}");
    }
    assert!(shown.iter().any(|text| text.starts_with("by small brain · claude · waited")), "{shown:?}");
    assert!(app.sync.blocker().is_some());
}
