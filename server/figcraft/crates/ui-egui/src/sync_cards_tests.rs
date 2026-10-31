//! The Sync pane's cards: the node's grouping as rows, leaving cards out, stepping with `[` / `]`,
//! the canvas flying to a card, and the pane and the canvas marks as drawn.

use egui::{Event, Key, Modifiers, Pos2, RawInput, Rect, vec2};
use figcraft_engine::Session;
use serde_json::json;

use super::*;
use crate::{FigApp, NoServices};

fn card(id: u64, name: &str, state: &str, kind: &str, page: u64) -> WaitingCard {
    WaitingCard {
        id,
        name: name.into(),
        page: "Mindmap".into(),
        page_id: Some(page),
        state: state.into(),
        kind: kind.into(),
        layers: 75,
        changes: Vec::new(),
    }
}

fn layer(id: u64, name: &str, state: &str, parent: u64, fields: &[&str]) -> LayerChange {
    LayerChange { id, name: name.into(), state: state.into(), parent: Some(parent), fields: fields.iter().map(|f| f.to_string()).collect() }
}

/// The design's list: two new cards, an edited one, a deleted one and two captions.
fn waiting() -> Vec<WaitingCard> {
    let mut tabs = card(3, "Widget Page tabs states", "changed", "frame", 1);
    tabs.changes = vec![
        layer(30, "tabs", "changed", 3, &["corners", "fills"]),
        layer(31, "tab Mindmap", "changed", 30, &["height", "layout", "childLayout"]),
        layer(32, "separator", "added", 30, &[]),
        layer(33, "separator", "added", 30, &[]),
        layer(34, "l", "added", 32, &[]),
        layer(35, "old pill", "removed", 3, &[]),
    ];
    vec![
        card(1, "Sidebar pages · window", "added", "frame", 1),
        card(2, "Sidebar pages · switcher", "added", "frame", 1),
        tabs,
        card(4, "Old onboarding note", "removed", "frame", 1),
        card(5, "Legend", "added", "text", 1),
        card(6, "Caption", "changed", "text", 1),
    ]
}

#[test]
fn layers_become_cards_and_loose_captions_fold_into_one_row() {
    let rows = rows(&waiting());
    let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(
        names,
        ["Sidebar pages · window", "Sidebar pages · switcher", "Widget Page tabs states", "Old onboarding note", "2 captions and legends"]
    );
    let subs: Vec<&str> = rows.iter().map(|row| row.sub.as_str()).collect();
    assert_eq!(subs, ["new · 75 layers", "new · 75 layers", "6 layers changed", "deleted · nowhere to go", "1 new · 1 changed"]);
    let marks: Vec<&str> = rows.iter().map(|row| row.mark.glyph()).collect();
    assert_eq!(marks, ["+", "+", "~", "−", "~"]);
    // The edited card lists its layers as a person reads them: fields in words, new layers counted.
    let details: Vec<(&str, &str)> = rows[2].details.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    assert_eq!(details, [("tabs", "corners · fills"), ("tab Mindmap", "height · layout"), ("+ 2 separators", "new"), ("− old pill", "removed")]);
    // A deleted card has nowhere to go; the folded row stands for both captions.
    assert!(!rows[3].navigable() && rows[3].page.is_none());
    assert!(rows[2].navigable());
    assert_eq!(rows[4].cards, [5, 6]);
    assert_eq!(chip_text(&rows[2], 2, 5), "~ 3 / 5 · tabs · tab Mindmap · 2 separators");
    assert_eq!(chip_text(&rows[0], 0, 5), "+ 1 / 5 · new · 75 layers");
}

#[test]
fn a_node_without_grouping_still_lists_its_top_most_layers() {
    let map: MapDiff = serde_json::from_value(json!({
        "added": [{"id": 1, "name": "New card"}, {"id": 2, "name": "inner", "parent": 1}],
        "changed": [{"id": 3, "name": "Status", "fields": ["fills"]}],
        "removed": [],
    }))
    .unwrap();
    let names: Vec<String> = map.rows().into_iter().map(|row| row.name).collect();
    assert_eq!(names, ["New card", "Status"]);
    let grouped: MapDiff = serde_json::from_value(json!({"cards": [{"id": 9, "name": "Rules", "state": "changed", "kind": "frame",
        "page_id": 4, "layers": 3, "changes": [{"id": 10, "name": "title", "state": "changed", "fields": ["kind"]}]}]}))
    .unwrap();
    let rows = grouped.rows();
    assert_eq!((rows[0].sub.as_str(), rows[0].page), ("1 layer changed", Some(4)));
    assert_eq!(rows[0].details, [("title".to_string(), "content".to_string())]);
}

#[test]
fn leaving_cards_out_counts_them_and_sends_only_listed_ids() {
    let mut state = State {
        diff: Some(DiffReply { ok: true, map: MapDiff { cards: waiting(), ..MapDiff::default() }, ..DiffReply::default() }),
        ..State::default()
    };
    let rows = state.rows();
    assert_eq!(state.included(&rows), 5);
    assert!(state.has_work());
    state.toggle(&rows[4]);
    state.toggle(&rows[1]);
    // A card the diff no longer lists is not sent.
    state.excluded.insert(99);
    assert_eq!(state.included(&rows), 3);
    assert_eq!(state.excluded_ids(&rows), [2, 5, 6]);
    assert!(rows[4].left_out(&state.excluded));
    assert_eq!(header_line(&state, &rows), "3 in this Sync · 2 keep waiting · first sync");
    assert_eq!(sync_label(3, 5, Some("codex")), "Sync 3 of 5 cards with codex");
    assert_eq!(sync_label(5, 5, Some("codex")), "Sync 5 cards with codex");
    assert_eq!(sync_label(0, 0, None), "Sync");
    state.toggle(&rows[1]);
    assert_eq!(state.excluded_ids(&rows), [5, 6]);
    for row in &rows {
        if !row.left_out(&state.excluded) {
            state.toggle(row);
        }
    }
    assert_eq!(state.included(&rows), 0);
    assert!(!state.has_work(), "nothing included and no note: nothing to sync");
    state.note = "only this".into();
    assert!(state.has_work());
}

#[test]
fn brackets_step_through_the_rows_and_stop_at_the_ends() {
    let rows = rows(&waiting());
    let mut state = State::default();
    assert_eq!(state.step(&rows, 1), Some(0));
    assert_eq!(state.step(&rows, -1), Some(0));
    assert_eq!(state.step(&rows, 1), Some(1));
    for _ in 0..10 {
        state.step(&rows, 1);
    }
    assert_eq!(state.current, Some(rows[4].key()));
    let mut fresh = State::default();
    assert_eq!(fresh.step(&rows, -1), Some(4), "`[` from nothing starts at the last row");
    assert_eq!(fresh.step(&[], 1), None);
    assert_eq!(fresh.current, None);
}

fn id(value: &serde_json::Value) -> u64 {
    figcraft_engine::params::id_of(&value["id"]).unwrap().0
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

/// A map with a card on its first page and one on a second page, and a pane listing both.
fn map() -> (FigApp, u64, u64, u64, u64) {
    let mut session = Session::new();
    let first = session.page().unwrap().0;
    let window =
        id(&session.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "Sidebar pages · window"})).unwrap());
    let second = id(&session.execute("page.add", &json!({"name": "Backend"})).unwrap());
    let rules = id(&session.execute("frame.create", &json!({"x": 2000, "y": 1500, "width": 300, "height": 120, "name": "Rules"})).unwrap());
    session.execute("page.select", &json!({"id": first})).unwrap();
    let mut app = FigApp::new(session, Box::new(NoServices));
    app.active_workspace = Some("wX".to_string());
    app.sync.all_pages = true;
    let mut rules_card = card(rules, "Rules", "changed", "frame", second);
    rules_card.changes = vec![layer(rules, "Rules", "changed", second, &["fills"])];
    let cards =
        vec![card(window, "Sidebar pages · window", "added", "frame", first), rules_card, card(77, "Old onboarding note", "removed", "frame", first)];
    app.sync.diff = Some(DiffReply { ok: true, map: MapDiff { cards, ..MapDiff::default() }, ..DiffReply::default() });
    (app, first, second, window, rules)
}

/// The first frame binds the pane to the workspace (and forgets any diff): open it on the map's.
fn open(app: &mut FigApp, frames: &mut Frames) {
    let diff = app.sync.diff.take();
    frames.run(app, Vec::new());
    app.sync.diff = diff;
    app.sync.open = true;
    frames.run(app, Vec::new());
}

struct Frames {
    ctx: egui::Context,
    time: f64,
}

impl Frames {
    fn new() -> Frames {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, false, 0);
        Frames { ctx, time: 0.0 }
    }

    /// One frame of the pane with `events`, the canvas marks painted under it.
    fn run(&mut self, app: &mut FigApp, events: Vec<Event>) -> Vec<String> {
        self.time += 1.0 / 60.0;
        let input =
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), time: Some(self.time), events, ..Default::default() };
        let mut out = self.ctx.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            app.canvas.rect = Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0));
            canvas_marks(app, ui.painter());
            show(app, &ctx);
        });
        out.textures_delta.clear();
        texts(&out.shapes)
    }
}

fn press(key: Key) -> Vec<Event> {
    vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]
}

#[test]
fn the_pane_lists_cards_with_their_checkboxes_the_folded_code_and_the_sticky_action() {
    let (mut app, ..) = map();
    let mut frames = Frames::new();
    open(&mut app, &mut frames);
    app.sync.excluded.insert(77);
    frames.run(&mut app, Vec::new());
    let shown = frames.run(&mut app, Vec::new());
    for needle in [
        "3 cards waiting",
        "2 in this Sync · 1 keeps waiting · first sync",
        "Sidebar pages · window",
        "new · 75 layers",
        "Rules",
        "1 layer changed",
        "Old onboarding note",
        "left out · keeps waiting",
        "include: ",
        "all",
        "none",
        "Code · first sync",
        "Sync 2 of 3 cards",
        "No agent is available in this workspace yet.",
        "– / 3",
        "[ ]  step",
    ] {
        assert!(shown.iter().any(|text| text == needle), "{needle:?} not in {shown:?}");
    }
    // While the pane is open the waiting card on this page wears its badge; the other page's does not.
    assert_eq!(shown.iter().filter(|text| *text == "+").count(), 2, "row mark and canvas badge: {shown:?}");
    assert!(!shown.iter().any(|text| text == "~ 2 / 3 · Rules"), "{shown:?}");
}

#[test]
fn a_bracket_steps_to_a_card_and_the_canvas_opens_its_page_and_flies_to_it() {
    let (mut app, first, second, window, rules) = map();
    let mut frames = Frames::new();
    open(&mut app, &mut frames);
    frames.run(&mut app, press(Key::CloseBracket));
    assert_eq!(app.sync.current, Some(window));
    assert_eq!(app.session.page().unwrap().0, first);
    let shown = frames.run(&mut app, press(Key::CloseBracket));
    assert_eq!(app.sync.current, Some(rules));
    assert_eq!(app.session.page().unwrap().0, second, "the card's page opens first");
    // The view is centred on the card.
    let ds = app.session.ds().unwrap();
    assert!((ds.view.cx - 2150.0).abs() < 1.0, "{} {}", ds.view.cx, ds.view.cy);
    assert!(ds.view.cy > 1560.0 && ds.view.cy < 1620.0, "{}", ds.view.cy);
    assert!(shown.iter().any(|text| text == "2 / 3"), "{shown:?}");
    let shown = frames.run(&mut app, Vec::new());
    assert!(shown.iter().any(|text| text == "~ 2 / 3 · Rules"), "the current card's chip: {shown:?}");
    assert!(shown.iter().any(|text| text == "fills"), "the current row lists its layers: {shown:?}");
    // Onto the deleted card: the row is current, the canvas stays where it was.
    let before = (app.session.ds().unwrap().view.cx, app.session.page().unwrap().0);
    frames.run(&mut app, press(Key::CloseBracket));
    assert_eq!(app.sync.current, Some(77));
    assert_eq!((app.session.ds().unwrap().view.cx, app.session.page().unwrap().0), before);
    frames.run(&mut app, press(Key::OpenBracket));
    assert_eq!(app.sync.current, Some(rules));
}

#[test]
fn brackets_type_in_the_note_instead_of_stepping() {
    let (mut app, ..) = map();
    let mut frames = Frames::new();
    open(&mut app, &mut frames);
    frames.ctx.memory_mut(|memory| memory.request_focus(Id::new(NOTE_ID)));
    frames.run(&mut app, Vec::new());
    frames.run(&mut app, press(Key::CloseBracket));
    assert_eq!(app.sync.current, None);
}
