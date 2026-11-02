//! Docked Sync participates in the same window layout and input routing as Pages.
use crate::layers_drag_tests::Ui;
use egui::{Event, Key, Modifiers, Pos2, Rect};
use figcraft_doc::{NodeKind, TextData};
use figcraft_engine::Session;
use figcraft_engine::figcraft_collab::message::ClientMsg;
use figcraft_engine::live::{Access, Event as LiveEvent, HostOptions, LiveLink, Presence, Role, SessionInfo};
use serde_json::json;

const WORKSPACE: &str = "sidebar-test";

/// Keep the workspace attached through update_active on every frame, without a live node.
struct Hosting;

impl LiveLink for Hosting {
    fn host(&mut self, _: u64, _: HostOptions) -> Result<SessionInfo, String> {
        let node = crate::node::SessionRef {
            // Port zero cannot address a live node if a UI refresh asks for a diff or agents.
            address: "127.0.0.1:0".into(),
            session: "sidebar-test-session".into(),
            workspace: WORKSPACE.into(),
            path: "/synthetic/sidebar-test".into(),
            branch: "main".into(),
        };
        Ok(SessionInfo {
            role: Role::Host,
            me: 1,
            name: "Sidebar tester".into(),
            color: "#14AE5C".into(),
            file_name: "Sidebar test".into(),
            pages: vec![],
            link: node.route(),
            url: String::new(),
            address: node.address,
            network: false,
            access: Access::Edit,
        })
    }

    fn join(&mut self, _: u64, _: &str, _: &str) -> Result<(), String> {
        Err("The sidebar fixture only hosts its synthetic document.".into())
    }
    fn leave(&mut self, _: u64) {}
    fn presence(&mut self, _: u64, _: &Presence) {}
    fn send(&mut self, _: u64, _: ClientMsg) {}
    fn poll(&mut self) -> Vec<(u64, LiveEvent)> {
        Vec::new()
    }
}

fn setup() -> Ui {
    let mut session = Session::new();
    let page = session.page().unwrap();
    let doc = session.doc_mut().unwrap();
    let cards: Vec<_> = [("First card", 0.0), ("Second card", 200.0)]
        .into_iter()
        .map(|(name, x)| {
            let card = doc.create(page, NodeKind::Frame, name, x, 0.0, 100.0, 80.0).unwrap();
            doc.create(card, NodeKind::Text(TextData::new(name)), "body", 8.0, 8.0, 84.0, 64.0).unwrap();
            card
        })
        .collect();
    assert_eq!(crate::map_navigation::cards(doc, page), cards, "the fixture has two readable cards, not empty decoration");
    session.live.install(Box::new(Hosting));
    session.execute("collab.host", &json!({})).unwrap();
    let mut u = Ui::new(session);
    u.app.onboarding.active = false;
    u.app.onboarding.decided = true;
    u.app.ui.home = false;
    u.app.ws.pane_open = true;
    u.frame(vec![]);
    u.app.sync.open = true;
    u.app.sync.note = "Keep this Sync note".into();
    u.app.sync.excluded.insert(42);
    for _ in 0..3 {
        u.frame(vec![]);
    }
    assert_eq!(u.app.active_workspace.as_deref(), Some(WORKSPACE), "the mock host survives workspace polling");
    assert!(crate::map_panes::sync_open(&u.app), "Sync must actually participate in the docked layout");
    assert_eq!(crate::map_navigation::cards(u.app.session.doc().unwrap(), page), cards);
    u
}

fn composer(u: &Ui) -> Rect {
    u.ctx.memory(|m| m.area_rect(egui::Id::new("map_composer"))).unwrap()
}

fn click(u: &mut Ui, at: Pos2) {
    u.frame(vec![Event::PointerMoved(at)]);
    u.button(at, true);
    u.button(at, false);
    u.frame(vec![]);
}

fn close_to(a: f32, b: f32) {
    assert!((a - b).abs() <= 1.0, "{a} != {b}");
}

#[test]
fn both_sidebars_bound_the_canvas_and_composer_and_toggle_independently() {
    crate::with_map_mode(|| {
        let mut u = setup();
        close_to(u.app.canvas.rect.left(), crate::map_panes::SIDEBAR_DEFAULT);
        close_to(u.app.canvas.rect.right(), 1440.0 - crate::map_panes::SYNC_DEFAULT);
        close_to(composer(&u).left(), u.app.canvas.rect.left());
        close_to(composer(&u).right(), u.app.canvas.rect.right());
        assert!(!crate::map_actions::blocking_overlay(&u.app, &u.ctx));
        // A click on the map is not a popup dismissal.
        click(&mut u, Pos2::new(600.0, 400.0));
        assert!(u.app.sync.open);
        let rect = u.app.sync.button_rect.unwrap();
        let toggle = Pos2::new(rect.right() - 14.0, rect.center().y);
        click(&mut u, toggle);
        assert!(!u.app.sync.open);
        assert!(u.app.ws.pane_open);
        close_to(u.app.canvas.rect.right(), 1440.0);
        close_to(composer(&u).right(), 1440.0);
        assert_eq!(u.app.sync.note, "Keep this Sync note");
        assert!(u.app.sync.excluded.contains(&42));
        let rect = u.app.sync.button_rect.unwrap();
        let toggle = Pos2::new(rect.right() - 14.0, rect.center().y);
        click(&mut u, toggle);
        assert!(u.app.sync.open);
        assert_eq!(u.app.sync.note, "Keep this Sync note");
        assert!(u.app.sync.excluded.contains(&42));
        u.app.ws.pane_open = false;
        u.frame(vec![]);
        u.frame(vec![]);
        close_to(u.app.canvas.rect.left(), 0.0);
        close_to(composer(&u).left(), 0.0);
        close_to(u.app.canvas.rect.right(), 1440.0 - crate::map_panes::SYNC_DEFAULT);
        assert!(u.app.sync.open);
    });
}

#[test]
fn dragging_the_right_inner_edge_grows_leftward_and_double_click_resets_it() {
    crate::with_map_mode(|| {
        let mut u = setup();
        let from = Pos2::new(1440.0 - crate::map_panes::SYNC_DEFAULT, 400.0);
        let to = from - egui::vec2(40.0, 0.0);
        u.drag_to(from, to);
        u.button(to, false);
        u.frame(vec![]);
        assert_eq!(u.app.session.prefs.map_sync_width, Some(crate::map_panes::SYNC_DEFAULT + 40.0));
        close_to(composer(&u).right(), 1440.0 - crate::map_panes::SYNC_DEFAULT - 40.0);
        // A distinct double click on the new divider restores the default.
        for _ in 0..30 {
            u.frame(vec![]);
        }
        click(&mut u, to);
        click(&mut u, to);
        u.frame(vec![]);
        assert_eq!(u.app.session.prefs.map_sync_width, Some(crate::map_panes::SYNC_DEFAULT));
        assert!(u.app.ws.pane_open);
    });
}

#[test]
fn a_docked_sync_does_not_disable_composer_keys_or_turn_escape_into_dismissal() {
    crate::with_map_mode(|| {
        let mut u = setup();
        u.app.compose.focus_input = true;
        u.frame(vec![]);
        u.frame(vec![Event::Text("A saved comment beside Sync".into())]);
        let key = |key| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        u.frame(vec![key(Key::Enter)]);
        assert_eq!(u.app.compose.drafts.len(), 1);
        assert_eq!(u.app.compose.drafts[0].text, "A saved comment beside Sync");
        assert!(u.app.compose.input.is_empty());
        assert!(u.app.sync.open);
        u.frame(vec![key(Key::Escape)]);
        assert!(u.app.sync.open, "Sync is a persistent sidebar, like Pages");
        assert_eq!(u.app.sync.note, "Keep this Sync note");
    });
}

#[test]
fn clicking_the_canvas_beside_sync_keeps_canvas_keyboard_shortcuts() {
    crate::with_map_mode(|| {
        let mut u = setup();
        let expected = crate::map_navigation::cards(u.app.session.doc().unwrap(), u.app.session.page().unwrap());
        assert_eq!(expected.len(), 2);
        click(&mut u, Pos2::new(700.0, 500.0));
        assert_eq!(u.app.ui.nav.region, crate::a11y::Region::Canvas);
        let modifiers = if crate::shortcuts::platform() == figcraft_engine::keymap::Platform::Mac {
            Modifiers::MAC_CMD | Modifiers::COMMAND
        } else {
            Modifiers::CTRL | Modifiers::COMMAND
        };
        u.frame_with(vec![Event::Key { key: Key::A, physical_key: None, pressed: true, repeat: false, modifiers }], modifiers);
        assert_eq!(u.app.session.selection(), expected, "Cmd/Ctrl+A selects both content cards while Sync is open");
        assert!(u.app.sync.open);
    });
}
