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
    assert_eq!(header_line(&state, &rows), "3 selected · 2 keep waiting · first Burn");
    assert_eq!(sync_label(3, 5, Some("codex")), "Burn 3 cards with Codex");
    assert_eq!(sync_label(5, 5, Some("codex")), "Burn 5 cards with Codex");
    assert_eq!(sync_label(1, 5, Some("codex")), "Burn 1 card with Codex");
    assert_eq!(sync_label(0, 0, Some("claude")), "Burn with Claude");
    assert_eq!(sync_label(0, 0, None), "Burn");
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
    height: f32,
    shapes: Vec<egui::epaint::ClippedShape>,
    remaining_keys: Vec<Key>,
    external_text: String,
    external_text_rect: Rect,
    canvas_button: Option<egui::Response>,
}

impl Frames {
    fn new() -> Frames {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, false, 0);
        Frames {
            ctx,
            time: 0.0,
            height: 900.0,
            shapes: Vec::new(),
            remaining_keys: Vec::new(),
            external_text: String::new(),
            external_text_rect: Rect::NOTHING,
            canvas_button: None,
        }
    }

    /// One frame of the pane with `events`, the canvas marks painted under it.
    fn run(&mut self, app: &mut FigApp, events: Vec<Event>) -> Vec<String> {
        self.time += 1.0 / 60.0;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, self.height))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut out = self.ctx.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            app.canvas.rect = Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0));
            canvas_marks(app, ui.painter());
            poll(app, &ctx);
            if app.sync.open {
                egui::Panel::right("sync_test_pane")
                    .exact_size(360.0)
                    .resizable(false)
                    .show_separator_line(false)
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| pane(app, ui));
            } else {
                let rect = Rect::from_min_size(pos2(1300.0, 0.0), vec2(140.0, 44.0));
                let mut header = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
                header_button(app, &mut header, &ctx);
            }
            self.remaining_keys = ctx.input(|input| {
                input
                    .events
                    .iter()
                    .filter_map(|event| match event {
                        Event::Key { key, pressed: true, .. } => Some(*key),
                        _ => None,
                    })
                    .collect()
            });
            // Render real canvas controls after Sync, as the composer is in the full app.
            let mut controls = ui.new_child(
                egui::UiBuilder::new().id_salt("canvas_test_controls").max_rect(Rect::from_min_size(pos2(100.0, 400.0), vec2(300.0, 100.0))),
            );
            let button = controls.button("Canvas test button");
            if button.clicked() {
                button.request_focus();
            }
            self.canvas_button = Some(button);
            self.external_text_rect =
                controls.add(egui::TextEdit::multiline(&mut self.external_text).id(crate::compose::input_id()).desired_rows(1)).rect;
        });
        out.textures_delta.clear();
        let text = texts(&out.shapes);
        self.shapes = out.shapes;
        text
    }
}

fn press(key: Key) -> Vec<Event> {
    vec![Event::PointerMoved(pos2(1200.0, 150.0)), Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]
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
        "2 selected · 1 keeps waiting · first Burn",
        "Sidebar pages · window",
        "new · 75 layers",
        "Rules",
        "1 layer changed",
        "Old onboarding note",
        "left out · keeps waiting",
        "include: ",
        "all",
        "none",
        "Code · first Burn",
        "Burn 2 cards",
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

fn click(frames: &mut Frames, app: &mut FigApp, pos: Pos2) {
    frames.run(app, vec![Event::PointerMoved(pos)]);
    for pressed in [true, false] {
        frames.run(
            app,
            vec![Event::PointerMoved(pos), Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE }],
        );
    }
}

#[test]
fn sidebar_toggle_works_during_sync_and_preserves_draft_and_exclusions() {
    let (mut app, ..) = map();
    let mut frames = Frames::new();
    open(&mut app, &mut frames);
    app.sync.note = "Keep this draft".into();
    app.sync.excluded.insert(77);
    app.sync.job = Some(Job { id: "running".into(), state: "in_flight".into(), ..Job::default() });
    // Both the label and the right-side icon toggle; neither cancels the job.
    for icon in [false, true] {
        frames.run(&mut app, Vec::new());
        let rect = app.sync.button_rect.unwrap();
        let pos = pos2(if icon { rect.right() - 14.0 } else { rect.left() + 15.0 }, rect.center().y);
        click(&mut frames, &mut app, pos);
        assert!(!app.sync.open);
        frames.run(&mut app, Vec::new());
        let rect = app.sync.button_rect.unwrap();
        let pos = pos2(if icon { rect.right() - 14.0 } else { rect.left() + 15.0 }, rect.center().y);
        click(&mut frames, &mut app, pos);
        assert!(app.sync.open);
        assert!(app.sync.in_flight());
        assert_eq!(app.sync.note, "Keep this draft");
        assert!(app.sync.excluded.contains(&77));
        assert!(app.sync.rx.is_none(), "reopening a running sync must not start a diff request");
    }
}

#[test]
fn canvas_click_and_escape_leave_sidebar_open_but_escape_closes_its_picker() {
    let (mut app, ..) = map();
    let mut frames = Frames::new();
    open(&mut app, &mut frames);
    click(&mut frames, &mut app, pos2(200.0, 200.0));
    assert!(app.sync.open);
    frames.run(&mut app, press(Key::Escape));
    assert!(app.sync.open);
    app.sync.menu_open = true;
    frames.run(&mut app, press(Key::Escape));
    assert!(!app.sync.menu_open);
    assert!(app.sync.open);
}

#[test]
fn closed_sidebar_still_receives_job_status_and_diff_replies() {
    let (mut app, ..) = map();
    let mut frames = Frames::new();
    open(&mut app, &mut frames);
    app.sync.open = false;
    app.sync.note = "Draft while waiting".into();
    app.sync.excluded.insert(77);
    app.sync.job = Some(Job { id: "running".into(), state: "in_flight".into(), ..Job::default() });
    let (job_tx, job_rx) = mpsc::channel();
    app.sync.job_rx = Some(job_rx);
    job_tx
        .send(JobMsg::Status(Ok(StatusReply {
            ok: true,
            job: Some(Job { id: "running".into(), state: "failed".into(), reason: Some("Agent stopped".into()), ..Job::default() }),
            ..StatusReply::default()
        })))
        .unwrap();
    let (diff_tx, diff_rx) = mpsc::channel();
    app.sync.rx = Some(diff_rx);
    app.sync.busy = true;
    diff_tx.send(Ok(DiffReply { ok: true, ..DiffReply::default() })).unwrap();
    frames.run(&mut app, Vec::new());
    assert!(!app.sync.open && !app.sync.busy && !app.sync.in_flight());
    assert!(app.sync.job.as_ref().unwrap().failed());
    assert!(app.sync.diff.as_ref().unwrap().ok);
    assert!(app.sync.rx.is_none() && app.sync.job_rx.is_none());
    assert_eq!(app.sync.note, "Draft while waiting");
    assert!(app.sync.excluded.contains(&77));
}

#[test]
fn the_header_says_burning_while_a_burn_runs_even_with_the_sidebar_closed() {
    let (mut app, ..) = map();
    let mut frames = Frames::new();
    open(&mut app, &mut frames);
    app.sync.open = false;
    let idle = frames.run(&mut app, Vec::new());
    assert!(idle.iter().any(|text| text.starts_with("Burn")) && !idle.iter().any(|text| text == "Burning…"), "{idle:?}");
    app.sync.job = Some(Job { id: "running".into(), state: "in_flight".into(), ..Job::default() });
    let shown = frames.run(&mut app, Vec::new());
    assert!(!app.sync.open);
    assert!(shown.iter().any(|text| text == "Burning…"), "the closed toggle says Burning…: {shown:?}");
    // Open, the sidebar's header and its action say it too.
    app.sync.open = true;
    frames.run(&mut app, Vec::new());
    let shown = frames.run(&mut app, Vec::new());
    assert!(shown.iter().filter(|text| *text == "Burning…").count() >= 2, "{shown:?}");
}

#[test]
fn the_button_looks_idle_waiting_burning_and_disabled() {
    let colors = crate::map_colors::get();
    let idle = button_look(false, true, None);
    assert_eq!((idle.label.as_str(), idle.fill), ("Burn", None));
    assert_eq!(button_look(false, true, Some(0)), idle, "nothing waiting is idle");
    let waiting = button_look(false, true, Some(7));
    assert_eq!((waiting.label.as_str(), waiting.ink, waiting.fill), ("Burn 7", colors.map_ink, Some(colors.selected)));
    let burning = button_look(true, true, Some(7));
    assert_eq!((burning.label.as_str(), burning.fill), ("Burning…", None));
    assert_eq!(button_look(true, false, None).label, "Burning…", "a Burn in flight says so even with no map open");
    let disabled = button_look(false, false, Some(7));
    assert_eq!((disabled.label.as_str(), disabled.ink), ("Burn", colors.faint));
}

#[test]
fn a_finished_burn_names_its_commit_or_its_cards() {
    assert_eq!(burned_toast(Some("a1f9c2e5d0b7"), 3), "Burned a1f9c2e");
    assert_eq!(burned_toast(Some("  "), 3), "Burned · 3 cards");
    assert_eq!(burned_toast(None, 1), "Burned · 1 card");
    assert_eq!(burned_toast(None, 0), "Burned");
    let job: Job = serde_json::from_value(json!({"id": "j", "state": "done", "commit": "a1f9c2e5d0"})).unwrap();
    assert_eq!(job.commit.as_deref(), Some("a1f9c2e5d0"));
    let job: Job = serde_json::from_value(json!({"id": "j", "state": "done", "commit": null})).unwrap();
    assert_eq!(job.commit, None);
}

#[test]
fn a_done_status_toasts_burned_with_the_commit_and_a_failure_says_burn_failed() {
    let (mut app, ..) = map();
    let mut frames = Frames::new();
    open(&mut app, &mut frames);
    for (job, toast) in [
        (Job { id: "j".into(), state: "done".into(), commit: Some("a1f9c2e5d0".into()), ..Job::default() }, "Burned a1f9c2e"),
        (Job { id: "j".into(), state: "failed".into(), reason: Some("agent stopped".into()), ..Job::default() }, "Burn failed: agent stopped"),
    ] {
        app.sync.job = Some(Job { id: "j".into(), state: "in_flight".into(), ..Job::default() });
        let (tx, rx) = mpsc::channel();
        app.sync.job_rx = Some(rx);
        tx.send(JobMsg::Status(Ok(StatusReply { ok: true, job: Some(job), ..StatusReply::default() }))).unwrap();
        frames.run(&mut app, Vec::new());
        assert_eq!(app.ui.toast.as_ref().map(|(text, _)| text.as_str()), Some(toast));
    }
}

fn command_enter(pos: Pos2) -> Vec<Event> {
    vec![Event::PointerMoved(pos), Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND }]
}

#[test]
fn sidebar_keys_require_hover_or_its_note_focus_and_leave_composer_keys_alone() {
    let (mut app, ..) = map();
    let mut frames = Frames::new();
    open(&mut app, &mut frames);
    let canvas = pos2(200.0, 200.0);
    let sidebar = pos2(1200.0, 150.0);
    frames.run(&mut app, command_enter(canvas));
    assert!(frames.remaining_keys.contains(&Key::Enter));
    frames.run(
        &mut app,
        vec![
            Event::PointerMoved(canvas),
            Event::Key { key: Key::CloseBracket, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE },
        ],
    );
    assert!(frames.remaining_keys.contains(&Key::CloseBracket));
    assert!(app.sync.current.is_none());
    // A text input outside Sync retains Cmd+Enter even with the pointer over the sidebar.
    let composer = crate::compose::input_id();
    let text_pos = frames.external_text_rect.center();
    click(&mut frames, &mut app, text_pos);
    assert!(frames.ctx.text_edit_focused());
    assert!(frames.ctx.memory(|memory| memory.has_focus(composer)));
    frames.run(&mut app, command_enter(sidebar));
    assert!(frames.remaining_keys.contains(&Key::Enter));
    assert!(app.sync.current.is_none());
    frames.run(&mut app, press(Key::CloseBracket));
    assert!(frames.remaining_keys.contains(&Key::CloseBracket));
    assert!(app.sync.current.is_none());
    frames.ctx.memory_mut(|memory| memory.surrender_focus(composer));
    frames.run(&mut app, command_enter(sidebar));
    assert!(!frames.remaining_keys.contains(&Key::Enter));
    frames.ctx.memory_mut(|memory| memory.request_focus(Id::new(NOTE_ID)));
    frames.run(&mut app, command_enter(canvas));
    assert!(!frames.remaining_keys.contains(&Key::Enter));
    assert!(!app.sync.starting, "without an agent, the shortcut cannot dispatch Sync");
}

#[test]
fn focused_canvas_button_does_not_block_hovered_sync_shortcuts() {
    let (mut app, _, _, first, _) = map();
    let mut frames = Frames::new();
    open(&mut app, &mut frames);
    let button = frames.canvas_button.as_ref().unwrap().clone();
    click(&mut frames, &mut app, button.rect.center());
    assert!(frames.ctx.memory(|memory| memory.has_focus(button.id)));
    assert!(frames.ctx.egui_wants_keyboard_input());
    assert!(!frames.ctx.text_edit_focused());
    frames.run(&mut app, press(Key::CloseBracket));
    assert_eq!(app.sync.current, Some(first));
    assert!(!frames.remaining_keys.contains(&Key::CloseBracket));
    frames.run(&mut app, command_enter(pos2(1200.0, 150.0)));
    assert!(!frames.remaining_keys.contains(&Key::Enter));
    assert!(!app.sync.starting, "shortcut still respects Sync action eligibility");
}

#[test]
fn overlays_keep_sync_shortcuts_for_their_own_controls_even_with_note_focus() {
    let (mut app, ..) = map();
    let mut frames = Frames::new();
    open(&mut app, &mut frames);
    for overlay in 0..4 {
        app.ui.preferences = overlay == 0;
        app.ui.palette = (overlay == 1).then(String::new);
        app.compose.menu_open = overlay == 2;
        app.sync.menu_open = overlay == 3;
        frames.run(&mut app, press(Key::CloseBracket));
        assert!(frames.remaining_keys.contains(&Key::CloseBracket), "overlay {overlay}");
        assert!(app.sync.current.is_none());
        // Sync's popup may dismiss itself when its owner isn't focused: restore its flag
        // before exercising the next shortcut, just as the overlay owner does on opening.
        app.sync.menu_open = overlay == 3;
        frames.ctx.memory_mut(|memory| memory.request_focus(Id::new(NOTE_ID)));
        frames.run(&mut app, command_enter(pos2(1200.0, 150.0)));
        assert!(frames.remaining_keys.contains(&Key::Enter), "overlay {overlay}");
        assert!(!app.sync.starting);
        frames.ctx.memory_mut(|memory| memory.surrender_focus(Id::new(NOTE_ID)));
    }
}

fn text_rect(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Rect {
    fn find(shape: &egui::Shape, needle: &str) -> Option<Rect> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == needle => Some(Rect::from_min_size(text.pos, text.galley.size())),
            egui::Shape::Vec(shapes) => shapes.iter().rev().find_map(|shape| find(shape, needle)),
            _ => None,
        }
    }
    // The empty-list action and header both say Burn; the footer is painted last.
    shapes.iter().rev().find_map(|shape| find(&shape.shape, needle)).unwrap_or_else(|| panic!("missing text {needle:?}"))
}

#[test]
fn docked_shell_and_footer_fill_the_window_with_short_and_long_lists() {
    let (mut app, ..) = map();
    let mut frames = Frames::new();
    open(&mut app, &mut frames);
    for height in [900.0, 520.0] {
        frames.height = height;
        for count in [0, 80] {
            app.sync.diff = Some(DiffReply {
                ok: true,
                map: MapDiff {
                    cards: (0..count).map(|i| card(1000 + i, &format!("Waiting card {i}"), "added", "frame", 1)).collect(),
                    ..MapDiff::default()
                },
                ..DiffReply::default()
            });
            for _ in 0..3 {
                frames.run(&mut app, Vec::new());
            }
            // Frame backgrounds can be nested under Shape::Vec (for example with shadows).
            fn collect_rects(shape: &egui::Shape, rects: &mut Vec<egui::epaint::RectShape>) {
                match shape {
                    egui::Shape::Rect(rect) => rects.push(rect.clone()),
                    egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect_rects(shape, rects)),
                    _ => {}
                }
            }
            let mut rects = Vec::new();
            for clipped in &frames.shapes {
                collect_rects(&clipped.shape, &mut rects);
            }
            let expected = Rect::from_min_max(pos2(1080.0, 0.0), pos2(1440.0, height));
            let shell = rects.iter().find(|rect| rect.fill == crate::map_colors::get().panel && rect.rect == expected).unwrap_or_else(|| {
                let white_rects: Vec<_> = rects.iter().filter(|rect| rect.fill == crate::map_colors::get().panel).map(|rect| (rect.rect, rect.corner_radius)).collect();
                panic!("missing flat full-height Sync shell: expected={expected:?}, cards={count}, open={}, viewport={:?}, header={:?}, white_rects={white_rects:?}",
                    app.sync.open, frames.ctx.viewport_rect(), app.sync.button_rect);
            });
            assert_eq!(shell.corner_radius, CornerRadius::ZERO);
            assert_eq!(shell.rect.top(), 0.0);
            assert_eq!(shell.rect.right(), 1440.0);
            let label = sync_label(count as usize, count as usize, None);
            let action = text_rect(&frames.shapes, &label);
            assert!(action.bottom() < height && action.top() > height - 100.0, "footer action {action:?} in {height}");
            let header = app.sync.button_rect.unwrap();
            assert!(header.bottom() <= crate::MAC_BAND);
            assert!(header.left() >= shell.rect.left());
            // Scroll the long list: its content moves, while the footer stays put.
            if count > 0 {
                frames.run(
                    &mut app,
                    vec![
                        Event::PointerMoved(pos2(1200.0, 300.0)),
                        Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: vec2(0.0, -500.0),
                            phase: egui::TouchPhase::Move,
                            modifiers: Modifiers::NONE,
                        },
                    ],
                );
                assert_eq!(text_rect(&frames.shapes, &label), action);
            }
        }
    }
}
