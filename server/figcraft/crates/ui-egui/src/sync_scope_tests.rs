//! Sync's page scope: a page sync snapshots the page and its subpages, with their parents.

use egui::{Pos2, RawInput, Rect, vec2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::sync::{CodeDiff, DiffReply, Snapshot};
use crate::{FigApp, NoServices};

fn id(value: &serde_json::Value) -> u64 {
    figcraft_engine::params::id_of(&value["id"]).unwrap().0
}

/// The first page with a card, a second root page with a subpage below it, each with a card.
fn tree() -> (Session, u64, u64, u64) {
    let mut session = Session::new();
    let first = session.page().unwrap().0;
    session.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100, "name": "Card Overview"})).unwrap();
    let backend = id(&session.execute("page.add", &json!({"name": "Backend"})).unwrap());
    session.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100, "name": "Card Server"})).unwrap();
    let diff = id(&session.execute("page.add", &json!({"name": "Diff engine"})).unwrap());
    session.execute("node.set", &json!({"id": diff, "pluginData": {crate::page_tree::PARENT_KEY: backend.to_string()}})).unwrap();
    session.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100, "name": "Card Numstat"})).unwrap();
    (session, first, backend, diff)
}

fn names(snapshot: &Snapshot) -> Vec<&str> {
    snapshot.nodes.iter().map(|node| node.name.as_str()).collect()
}

#[test]
fn a_page_snapshot_takes_its_subpages_and_their_parents() {
    let (session, first, backend, diff) = tree();
    let doc = session.doc().unwrap();

    let page = Snapshot::of_pages(doc, Some(NodeId(backend)));
    let pages: Vec<(u64, Option<u64>)> = page.pages.iter().map(|page| (page.id, page.parent)).collect();
    assert_eq!(pages, vec![(backend, None), (diff, Some(backend))]);
    assert_eq!(names(&page), vec!["Card Server", "Card Numstat"]);

    let subpage = Snapshot::of_pages(doc, Some(NodeId(diff)));
    assert_eq!(names(&subpage), vec!["Card Numstat"]);

    let all = Snapshot::of(doc);
    assert_eq!(all.pages.len(), 3);
    assert_eq!(names(&all), vec!["Card Overview", "Card Server", "Card Numstat"]);
    assert_eq!(all.pages[0].id, first);
}

#[test]
fn a_root_page_is_sent_without_a_parent_field() {
    let (session, _, backend, diff) = tree();
    let snapshot = Snapshot::of_pages(session.doc().unwrap(), Some(NodeId(backend)));
    let value = serde_json::to_value(&snapshot).unwrap();
    assert!(value["pages"][0].get("parent").is_none(), "{value}");
    assert_eq!(value["pages"][1]["parent"], json!(backend));
    assert_eq!(value["pages"][1]["id"], json!(diff));
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
fn the_widget_names_the_page_scope_and_the_code_it_covers() {
    let (mut session, _, backend, _) = tree();
    session.execute("page.select", &json!({"id": backend})).unwrap();
    let mut app = FigApp::new(session, Box::new(NoServices));
    app.active_workspace = Some("wX".to_string());

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
    // The first frame binds the state to the workspace; then the widget shows a page diff.
    run(&mut app);
    app.sync.open = true;
    app.sync.diff = Some(DiffReply {
        ok: true,
        code: CodeDiff { scope: vec!["service/src".to_string(), "skills/mindmap".to_string()], ..CodeDiff::default() },
        ..DiffReply::default()
    });
    run(&mut app);
    let shown = run(&mut app);
    for needle in ["This page", "All pages", "Backend + 1 subpage", "Code · first Burn"] {
        assert!(shown.iter().any(|text| text == needle), "{needle:?} not in {shown:?}");
    }
    // The code folds to one line; unfolded, it names the paths it is limited to.
    assert!(!shown.iter().any(|text| text == "in service/src, skills/mindmap"), "{shown:?}");
    app.sync.code_open = true;
    run(&mut app);
    let shown = run(&mut app);
    assert!(shown.iter().any(|text| text == "in service/src, skills/mindmap"), "{shown:?}");
}
