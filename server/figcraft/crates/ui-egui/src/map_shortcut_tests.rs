//! Actual egui input routing in the map client, separate from catalog lookup tests.
use crate::layers_drag_tests::Ui;
use egui::{Event, Key, Modifiers};
use figcraft_doc::NodeId;
use figcraft_engine::{Session, keymap::Platform};
use serde_json::json;

fn cmd() -> Modifiers {
    if crate::shortcuts::platform() == Platform::Mac { Modifiers::MAC_CMD | Modifiers::COMMAND } else { Modifiers::CTRL | Modifiers::COMMAND }
}
fn press(u: &mut Ui, key: Key, modifiers: Modifiers) {
    let ev = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
    u.frame_with(vec![ev(true), ev(false)], modifiers);
}
fn setup() -> (Ui, NodeId, NodeId) {
    let mut s = Session::new();
    let mut card = |name, x| {
        let id = NodeId(
            s.execute("node.create", &json!({"type":"frame", "name":name,"x":x,"y":100,"width":200,"height":100})).unwrap()["id"].as_u64().unwrap(),
        );
        s.execute("text.create", &json!({"parent":id,"text":name,"x":10,"y":10})).unwrap();
        id
    };
    let a = card("Alpha card", 50);
    let b = card("Beta card", 350);
    s.set_selection(vec![a]);
    let mut u = Ui::new(s);
    u.app.onboarding.active = false;
    u.app.onboarding.decided = true;
    u.app.ui.home = false;
    u.frame(vec![]);
    u.frame(vec![]);
    (u, a, b)
}
#[test]
fn map_keys_never_mutate_document_or_enter_editor_tools() {
    crate::with_map_mode(|| {
        let (mut u, a, _) = setup();
        let before = serde_json::to_value(u.app.session.doc().unwrap()).unwrap();
        for (key, mods) in [
            (Key::Delete, Modifiers::NONE),
            (Key::R, Modifiers::NONE),
            (Key::ArrowRight, Modifiers::NONE),
            (Key::D, cmd()),
            (Key::Z, cmd()),
            (Key::Z, cmd() | Modifiers::SHIFT),
            (Key::Enter, Modifiers::NONE),
        ] {
            press(&mut u, key, mods);
        }
        assert_eq!(serde_json::to_value(u.app.session.doc().unwrap()).unwrap(), before);
        assert_eq!(u.app.session.selection(), vec![a]);
        assert!(u.app.session.ds().unwrap().text_edit.is_none());
        assert!(!crate::map_command_allowed("edit.undo"));
        assert!(!crate::map_command_allowed("file.save"));
        // FigCraft's comment threads are off; the composer's draft pins keep their toggle.
        assert!(!crate::map_command_allowed("comment.add"));
        assert!(!crate::map_command_allowed("comment.open"));
        assert!(!crate::map_command_allowed("tool.comment"));
        assert!(crate::map_command_allowed("view.comments"));
    });
}
#[test]
fn map_native_copy_event_follows_canvas_focus() {
    crate::with_map_mode(|| {
        let (mut u, _, _) = setup();
        let copied =
            |output: &egui::FullOutput| output.platform_output.commands.iter().any(|command| matches!(command, egui::OutputCommand::CopyText(_)));
        u.app.ui.nav.region = crate::a11y::Region::Toolbar;
        assert!(!copied(&u.frame_output(vec![Event::Copy])));
        u.app.ui.nav.region = crate::a11y::Region::Canvas;
        assert!(copied(&u.frame_output(vec![Event::Copy])));
    });
}
#[test]
fn map_escape_closes_one_overlay_before_clearing_selection_and_keeps_drafts() {
    crate::with_map_mode(|| {
        let (mut u, a, _) = setup();
        u.app.compose.drafts.push(crate::compose::Draft {
            text: "Keep me".into(),
            node: None,
            page: None,
            node_id: None,
            page_id: u.app.session.page().ok(),
            group: None,
        });
        u.app.ui.shortcuts_panel = true;
        press(&mut u, Key::Escape, Modifiers::NONE);
        assert!(!u.app.ui.shortcuts_panel);
        assert_eq!(u.app.session.selection(), vec![a]);
        press(&mut u, Key::Escape, Modifiers::NONE);
        assert!(u.app.session.selection().is_empty());
        assert_eq!(u.app.compose.drafts.len(), 1);
    });
}
#[test]
fn map_tab_selects_cards_and_command_a_excludes_decorations() {
    crate::with_map_mode(|| {
        let (mut u, a, b) = setup();
        press(&mut u, Key::Tab, Modifiers::NONE);
        assert_eq!(u.app.session.selection(), vec![b]);
        press(&mut u, Key::Tab, Modifiers::SHIFT);
        assert_eq!(u.app.session.selection(), vec![a]);
        press(&mut u, Key::A, cmd());
        assert_eq!(u.app.session.selection(), vec![a, b]);
    });
}
#[test]
fn map_search_is_read_only_and_escape_does_not_deselect() {
    crate::with_map_mode(|| {
        let (mut u, a, _) = setup();
        let before = serde_json::to_value(u.app.session.doc().unwrap()).unwrap();
        press(&mut u, Key::F, cmd());
        assert!(crate::map_navigation::search_open(&u.ctx));
        u.frame(vec![Event::Text("Alpha".into())]);
        press(&mut u, Key::Escape, Modifiers::NONE);
        assert!(!crate::map_navigation::search_open(&u.ctx));
        assert_eq!(u.app.session.selection(), vec![a]);
        assert_eq!(serde_json::to_value(u.app.session.doc().unwrap()).unwrap(), before);
    });
}
#[test]
fn map_focus_mode_hides_chrome_and_workspace_shortcuts_preserve_prompt() {
    crate::with_map_mode(|| {
        let (mut u, _, _) = setup();
        press(&mut u, Key::Backslash, cmd());
        assert!(u.app.session.ds().unwrap().view.ui_hidden);
        press(&mut u, Key::Backslash, cmd());
        assert!(!u.app.session.ds().unwrap().view.ui_hidden);
        u.app.compose.input = "Unsent".into();
        press(&mut u, Key::W, cmd() | Modifiers::SHIFT);
        assert!(u.app.ui.home);
        assert_eq!(u.app.compose.input, "Unsent");
        press(&mut u, Key::K, cmd());
        assert!(
            u.app.ui.palette.is_some(),
            "home={} overlay={} new={} machine={} onboarding={} popup={}",
            u.app.ui.home,
            crate::map_actions::overlay_open(&u.app, &u.ctx),
            u.app.new_workspace.open,
            u.app.machine_modal.open,
            u.app.onboarding.active,
            egui::Popup::is_any_open(&u.ctx)
        );
    });
}
#[test]
fn map_composer_typing_owns_shift_enter_and_enter_saves() {
    crate::with_map_mode(|| {
        let (mut u, a, _) = setup();
        u.app.compose.focus_input = true;
        u.frame(vec![]);
        u.frame(vec![Event::Text("A note".into())]);
        press(&mut u, Key::Enter, Modifiers::SHIFT);
        assert!(u.app.compose.input.contains('\n'), "Shift+Enter adds a line");
        assert!(u.app.compose.drafts.is_empty(), "a new line does not add a draft");
        assert_eq!(u.app.session.selection(), vec![a]);
        press(&mut u, Key::Enter, Modifiers::NONE);
        assert!(u.app.compose.input.is_empty(), "Enter saves even without a connected workspace");
        assert_eq!(u.app.compose.drafts.len(), 1);
        assert_eq!(u.app.compose.drafts[0].text, "A note");
        assert_eq!(u.app.compose.drafts[0].node_id, Some(a));
        assert_eq!(u.app.compose.drafts[0].page_id, u.app.session.page().ok());
        assert_eq!(u.app.session.selection(), vec![a], "the composer keeps the Enter");
    });
}

#[test]
fn shift_cmd_p_toggles_the_actions_and_cmd_p_goes_to_a_page() {
    crate::with_map_mode(|| {
        let (mut u, _, _) = setup();
        press(&mut u, Key::P, cmd() | Modifiers::SHIFT);
        assert_eq!(u.app.ui.palette.as_deref(), Some(">"), "the same box, with > typed");
        assert!(crate::map_palette::listing_actions(&u.app));
        assert!(!u.app.page_tree.open);
        press(&mut u, Key::P, cmd() | Modifiers::SHIFT);
        assert!(u.app.ui.palette.is_none(), "the same chord closes it");
        press(&mut u, Key::P, cmd());
        assert_eq!(u.app.ui.palette.as_deref(), Some(""), "Cmd+P opens the box on the pages");
        assert_eq!(crate::map_palette::scope(&u.app), Some(&crate::map_palette::Scope::Pages));
        press(&mut u, Key::P, cmd() | Modifiers::SHIFT);
        assert_eq!(u.app.ui.palette.as_deref(), Some(">"), "Shift+Cmd+P switches the open box to the actions");
        press(&mut u, Key::P, cmd());
        assert_eq!(u.app.ui.palette.as_deref(), Some(""), "and Cmd+P back to the pages");
        press(&mut u, Key::P, cmd());
        assert!(u.app.ui.palette.is_none(), "the same chord closes it");
    });
}

#[test]
fn map_palette_can_open_while_composer_owns_text_focus() {
    crate::with_map_mode(|| {
        let (mut u, _, _) = setup();
        u.app.compose.focus_input = true;
        u.frame(vec![]);
        u.frame(vec![Event::Text("Keep writing".into())]);
        press(&mut u, Key::K, cmd());
        assert!(u.app.ui.palette.is_some());
        assert_eq!(u.app.compose.input, "Keep writing");
        press(&mut u, Key::Escape, Modifiers::NONE);
        assert!(u.app.ui.palette.is_none());
        assert_eq!(u.app.compose.input, "Keep writing");
    });
}

#[test]
fn map_area_selection_cancel_and_f6_keep_card_context() {
    crate::with_map_mode(|| {
        let (mut u, a, _) = setup();
        press(&mut u, Key::F8, Modifiers::NONE);
        let original = u.app.ui.key_box.unwrap();
        press(&mut u, Key::ArrowRight, Modifiers::SHIFT);
        assert!(u.app.ui.key_box.unwrap()[2] > original[2]);
        press(&mut u, Key::Escape, Modifiers::NONE);
        assert!(u.app.ui.key_box.is_none());
        assert_eq!(u.app.session.selection(), vec![a]);
        press(&mut u, Key::F6, Modifiers::NONE);
        assert_eq!(u.app.ui.nav.region, crate::a11y::Region::Properties);
        assert!(u.ctx.memory(|m| m.has_focus(crate::compose::input_id())));
        u.frame(vec![Event::Text("Field text".into())]);
        press(&mut u, Key::A, cmd());
        assert_eq!(u.app.session.selection(), vec![a]);
        press(&mut u, Key::Escape, Modifiers::NONE);
        assert_eq!(u.app.ui.nav.region, crate::a11y::Region::Canvas);
        assert_eq!(u.app.session.selection(), vec![a]);
        assert_eq!(u.app.compose.input, "Field text");
    });
}

#[test]
fn map_escape_ends_space_pan_before_deselecting_and_waits_for_release() {
    crate::with_map_mode(|| {
        let (mut u, a, _) = setup();
        let start = egui::pos2(u.app.canvas.rect.center().x, u.app.canvas.rect.top() + 180.0);
        let space = |pressed| Event::Key { key: Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
        u.frame(vec![Event::PointerMoved(start), space(true)]);
        u.frame(vec![Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
        let moved = start + egui::vec2(60.0, 20.0);
        u.frame(vec![Event::PointerMoved(moved)]);
        press(&mut u, Key::Escape, Modifiers::NONE);
        assert_eq!(u.app.session.selection(), vec![a]);
        let before = u.app.session.ds().unwrap().view.clone();
        u.frame(vec![Event::PointerMoved(moved + egui::vec2(50.0, 0.0))]);
        let after = &u.app.session.ds().unwrap().view;
        assert_eq!((after.cx, after.cy), (before.cx, before.cy));
        u.frame(vec![
            space(false),
            Event::PointerButton { pos: moved, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
        ]);
        press(&mut u, Key::Escape, Modifiers::NONE);
        assert!(u.app.session.selection().is_empty());
    });
}

#[test]
fn map_focus_mode_keeps_the_windowed_mac_traffic_light_band_clear() {
    crate::with_map_mode(|| {
        let (mut u, _, _) = setup();
        u.app.ui.title_bar = crate::TitleBar::Inset;
        press(&mut u, Key::Backslash, cmd());
        assert!(u.app.session.ds().unwrap().view.ui_hidden);
        assert_eq!(u.app.canvas.rect.top(), crate::MAC_BAND);
    });
}

fn pointer(u: &mut Ui, pos: egui::Pos2, pressed: bool, modifiers: Modifiers) {
    u.frame_with(vec![Event::PointerMoved(pos), Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers }], modifiers);
}

fn drag(u: &mut Ui, start: egui::Pos2, end: egui::Pos2, modifiers: Modifiers) {
    pointer(u, start, true, modifiers);
    u.frame_with(vec![Event::PointerMoved(end)], modifiers);
    pointer(u, end, false, modifiers);
}

fn map_view(u: &mut Ui) {
    let view = &mut u.app.session.ds_mut().unwrap().view;
    view.zoom = 1.0;
    view.cx = 300.0;
    view.cy = 150.0;
    u.frame(vec![]);
}

fn x_of(u: &Ui, id: NodeId) -> f64 {
    u.app.session.doc().unwrap().node(id).unwrap().transform.as_coeffs()[4]
}

#[test]
fn map_pointer_drag_moves_the_selected_cards_and_cmd_z_puts_them_back() {
    crate::with_map_mode(|| {
        let (mut u, a, b) = setup();
        map_view(&mut u);
        let before = serde_json::to_value(u.app.session.doc().unwrap()).unwrap();
        // A press on the text inside a card, or on its corner, drags the whole card: no resize.
        for (x, y) in [(70.0, 115.0), (249.0, 199.0)] {
            u.app.session.set_selection(vec![a]);
            u.frame(vec![]);
            let start = u.app.to_screen(x, y);
            drag(&mut u, start, start + egui::vec2(45.0, 25.0), Modifiers::NONE);
            let n = u.app.session.doc().unwrap().node(a).unwrap().clone();
            assert_eq!((n.transform.as_coeffs()[4], n.transform.as_coeffs()[5], n.width, n.height), (95.0, 125.0, 200.0, 100.0));
            assert_eq!(u.app.session.selection(), vec![a]);
            press(&mut u, Key::Z, cmd());
            assert_eq!(serde_json::to_value(u.app.session.doc().unwrap()).unwrap(), before, "one ⌘Z undoes the whole drag");
        }
        press(&mut u, Key::Z, cmd() | Modifiers::SHIFT);
        assert_eq!(x_of(&u, a), 95.0, "⇧⌘Z moves it again");
        press(&mut u, Key::Z, cmd());

        // Both selected cards move together; the other stays where it is when only one is.
        u.app.session.set_selection(vec![a, b]);
        u.frame(vec![]);
        let start = u.app.to_screen(400.0, 150.0);
        drag(&mut u, start, start + egui::vec2(-30.0, 0.0), Modifiers::NONE);
        assert_eq!((x_of(&u, a), x_of(&u, b)), (20.0, 320.0));
        press(&mut u, Key::Z, cmd());
        assert_eq!(serde_json::to_value(u.app.session.doc().unwrap()).unwrap(), before);

        // Esc during the drag puts the card back and records nothing.
        u.app.session.set_selection(vec![a]);
        u.frame(vec![]);
        let depth = u.app.session.undo_depth();
        let start = u.app.to_screen(150.0, 160.0);
        pointer(&mut u, start, true, Modifiers::NONE);
        u.frame(vec![Event::PointerMoved(start + egui::vec2(60.0, 0.0))]);
        assert_eq!(x_of(&u, a), 110.0, "the card follows the pointer");
        press(&mut u, Key::Escape, Modifiers::NONE);
        pointer(&mut u, start + egui::vec2(60.0, 0.0), false, Modifiers::NONE);
        assert_eq!(serde_json::to_value(u.app.session.doc().unwrap()).unwrap(), before);
        assert_eq!(u.app.session.undo_depth(), depth);

        // A box inside a card, a locked card and the gap between cards never move.
        let text = u.app.session.doc().unwrap().node(a).unwrap().children[0];
        u.app.session.set_selection(vec![text]);
        u.frame(vec![]);
        let start = u.app.to_screen(70.0, 115.0);
        drag(&mut u, start, start + egui::vec2(45.0, 25.0), Modifiers::NONE);
        u.app.session.doc_mut().unwrap().node_mut(b).unwrap().locked = true;
        let before = serde_json::to_value(u.app.session.doc().unwrap()).unwrap();
        u.app.session.set_selection(vec![b]);
        u.frame(vec![]);
        let start = u.app.to_screen(450.0, 150.0);
        drag(&mut u, start, start + egui::vec2(45.0, 25.0), Modifiers::NONE);
        u.app.session.set_selection(vec![a, b]);
        u.frame(vec![]);
        let gap = u.app.to_screen(300.0, 150.0);
        drag(&mut u, gap, gap + egui::vec2(45.0, 0.0), Modifiers::NONE);
        assert_eq!(serde_json::to_value(u.app.session.doc().unwrap()).unwrap(), before);
        let ds = u.app.session.ds().unwrap();
        assert!(ds.text_edit.is_none() && ds.vector_edit.is_none());
        assert!(!u.app.session.tool.busy());
    });
}

#[test]
fn map_pointer_double_click_never_enters_text_editing_and_opens_linked_detail() {
    crate::with_map_mode(|| {
        let (mut u, a, _) = setup();
        map_view(&mut u);
        let before = serde_json::to_value(u.app.session.doc().unwrap()).unwrap();
        let text = u.app.to_screen(70.0, 115.0);
        for _ in 0..2 {
            pointer(&mut u, text, true, Modifiers::NONE);
            pointer(&mut u, text, false, Modifiers::NONE);
        }
        assert!(u.app.session.ds().unwrap().text_edit.is_none());
        assert!(u.app.session.ds().unwrap().vector_edit.is_none());
        assert_eq!(serde_json::to_value(u.app.session.doc().unwrap()).unwrap(), before);

        let page = u.app.session.page().unwrap();
        let doc = u.app.session.doc_mut().unwrap();
        let detail = doc.add_page("Linked detail");
        doc.node_mut(detail).unwrap().plugin_data.insert(crate::page_tree::PARENT_KEY.into(), page.0.to_string());
        doc.node_mut(a).unwrap().plugin_data.insert(crate::new_page::DETAIL_PAGE_KEY.into(), detail.0.to_string());
        let before = serde_json::to_value(u.app.session.doc().unwrap()).unwrap();
        // Reset click timing before the independent linked-card double-click.
        for _ in 0..40 {
            u.frame(vec![]);
        }
        let card = u.app.to_screen(150.0, 160.0);
        for _ in 0..2 {
            pointer(&mut u, card, true, Modifiers::NONE);
            pointer(&mut u, card, false, Modifiers::NONE);
        }
        assert_eq!(u.app.session.page().unwrap(), detail);
        assert_eq!(serde_json::to_value(u.app.session.doc().unwrap()).unwrap(), before);
        // The existing single-click subpage badge still owns its pointer hit area.
        u.app.run("page.select", json!({"id": page}));
        map_view(&mut u);
        let badge = u.app.to_screen(230.0, 110.0);
        u.frame(vec![Event::PointerMoved(badge)]);
        pointer(&mut u, badge, true, Modifiers::NONE);
        pointer(&mut u, badge, false, Modifiers::NONE);
        assert_eq!(u.app.session.page().unwrap(), detail);
        assert_eq!(serde_json::to_value(u.app.session.doc().unwrap()).unwrap(), before);
    });
}

#[test]
fn map_f8_enter_selects_locked_and_section_cards_and_excludes_decoration() {
    crate::with_map_mode(|| {
        let (mut u, a, b) = setup();
        let page = u.app.session.page().unwrap();
        let doc = u.app.session.doc_mut().unwrap();
        doc.node_mut(a).unwrap().locked = true;
        let section = doc.create(page, figcraft_doc::NodeKind::Section { contents_hidden: false }, "Section", 0.0, 0.0, 600.0, 300.0).unwrap();
        doc.reparent(b, section, None).unwrap();
        let decoration = doc.create(page, figcraft_doc::NodeKind::Rectangle, "Page bounds", 0.0, 0.0, 600.0, 300.0).unwrap();
        let before = serde_json::to_value(u.app.session.doc().unwrap()).unwrap();
        press(&mut u, Key::F8, Modifiers::NONE);
        assert!(u.app.ui.key_box.is_some());
        u.app.ui.key_box = Some([-10.0, -10.0, 610.0, 310.0]);
        press(&mut u, Key::Enter, Modifiers::NONE);
        assert!(u.app.ui.key_box.is_none());
        assert_eq!(u.app.session.selection(), vec![a, b]);
        assert!(!u.app.session.selection().contains(&section));
        assert!(!u.app.session.selection().contains(&decoration));
        assert_eq!(serde_json::to_value(u.app.session.doc().unwrap()).unwrap(), before);
        // Reversed box coordinates normalize; Shift+Enter toggles whole cards, not text leaves.
        press(&mut u, Key::F8, Modifiers::NONE);
        u.app.ui.key_box = Some([251.0, 201.0, 49.0, 99.0]);
        press(&mut u, Key::Enter, Modifiers::SHIFT);
        assert_eq!(u.app.session.selection(), vec![b]);
    });
}

#[test]
fn map_pointer_marquee_selects_locked_cards_and_escape_restores_context() {
    crate::with_map_mode(|| {
        let (mut u, a, b) = setup();
        u.app.session.doc_mut().unwrap().node_mut(a).unwrap().locked = true;
        map_view(&mut u);
        let start = u.app.to_screen(30.0, 80.0);
        let end = u.app.to_screen(570.0, 220.0);
        drag(&mut u, start, end, Modifiers::NONE);
        assert_eq!(u.app.session.selection(), vec![a, b]);
        // A cancelled marquee restores the selection captured at pointer-down.
        pointer(&mut u, start, true, Modifiers::NONE);
        let partial = u.app.to_screen(260.0, 220.0);
        u.frame(vec![Event::PointerMoved(partial)]);
        assert_eq!(u.app.session.selection(), vec![a]);
        press(&mut u, Key::Escape, Modifiers::NONE);
        assert_eq!(u.app.session.selection(), vec![a, b]);
        pointer(&mut u, partial, false, Modifiers::NONE);
        assert_eq!(u.app.session.selection(), vec![a, b]);
    });
}

#[test]
fn map_middle_and_space_drag_pan_without_editing_cards() {
    crate::with_map_mode(|| {
        let (mut u, a, _) = setup();
        map_view(&mut u);
        let document = serde_json::to_value(u.app.session.doc().unwrap()).unwrap();
        for button in [egui::PointerButton::Middle, egui::PointerButton::Primary] {
            let start = egui::pos2(u.app.canvas.rect.center().x, u.app.canvas.rect.top() + 180.0);
            let end = start + egui::vec2(60.0, 30.0);
            let view = u.app.session.ds().unwrap().view.clone();
            let mut events = vec![Event::PointerMoved(start)];
            if button == egui::PointerButton::Primary {
                events.push(Event::Key { key: Key::Space, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
            }
            u.frame(events);
            u.frame(vec![Event::PointerButton { pos: start, button, pressed: true, modifiers: Modifiers::NONE }]);
            u.frame(vec![Event::PointerMoved(end)]);
            u.frame(vec![
                Event::PointerButton { pos: end, button, pressed: false, modifiers: Modifiers::NONE },
                Event::Key { key: Key::Space, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE },
            ]);
            let after = &u.app.session.ds().unwrap().view;
            assert_ne!((after.cx, after.cy), (view.cx, view.cy));
            assert_eq!(u.app.session.selection(), vec![a]);
            assert_eq!(serde_json::to_value(u.app.session.doc().unwrap()).unwrap(), document);
        }
    });
}

#[test]
fn map_pointer_selection_focuses_the_composer_and_keyboard_selection_does_not() {
    crate::with_map_mode(|| {
        let (mut u, a, b) = setup();
        map_view(&mut u);
        let composer = |u: &Ui| u.ctx.memory(|m| m.has_focus(crate::compose::input_id()));
        assert!(!composer(&u));
        // A click on a card: the comment field takes the keyboard at once.
        let card = u.app.to_screen(450.0, 160.0);
        pointer(&mut u, card, true, Modifiers::NONE);
        pointer(&mut u, card, false, Modifiers::NONE);
        u.frame(vec![]);
        assert_eq!(u.app.session.selection(), vec![b]);
        assert!(composer(&u), "pointer selection focuses the composer");
        u.frame(vec![Event::Text("About Beta".into())]);
        assert_eq!(u.app.compose.input, "About Beta");
        // The first Esc gives the keys back to the canvas and keeps the selection and the text.
        press(&mut u, Key::Escape, Modifiers::NONE);
        assert!(!composer(&u));
        assert_eq!(u.app.ui.nav.region, crate::a11y::Region::Canvas);
        assert_eq!(u.app.session.selection(), vec![b]);
        // Keyboard selection leaves the keys on the canvas.
        press(&mut u, Key::Tab, Modifiers::SHIFT);
        u.frame(vec![]);
        assert_eq!(u.app.session.selection(), vec![a]);
        assert!(!composer(&u), "keyboard selection must not move focus");
        // A click on empty canvas selects nothing and leaves focus alone.
        let empty = u.app.to_screen(300.0, 400.0);
        pointer(&mut u, empty, true, Modifiers::NONE);
        pointer(&mut u, empty, false, Modifiers::NONE);
        u.frame(vec![]);
        assert!(u.app.session.selection().is_empty());
        assert!(!composer(&u));
    });
}

#[test]
fn map_recipient_chip_opens_the_agent_menu_on_a_click() {
    crate::with_map_mode(|| {
        let (mut u, _, _) = setup();
        u.frame(vec![]);
        u.frame(vec![]);
        let chip = u.ctx.read_response(egui::Id::new("map_composer_target_chip")).unwrap().rect.center();
        u.frame(vec![Event::PointerMoved(chip)]);
        pointer(&mut u, chip, true, Modifiers::NONE);
        pointer(&mut u, chip, false, Modifiers::NONE);
        assert!(u.app.compose.menu_open);
        press(&mut u, Key::Escape, Modifiers::NONE);
        assert!(!u.app.compose.menu_open);
    });
}

#[test]
fn command_backspace_deletes_the_selection_and_command_z_brings_it_back() {
    crate::with_map_mode(|| {
        let (mut u, a, b) = setup();
        u.app.ui.nav.region = crate::a11y::Region::Canvas;
        press(&mut u, Key::Backspace, cmd());
        assert!(u.app.session.doc().unwrap().node(a).is_none(), "⌘⌫ deletes the selected card");
        assert!(u.app.session.doc().unwrap().node(b).is_some());
        press(&mut u, Key::Z, cmd());
        assert!(u.app.session.doc().unwrap().node(a).is_some(), "⌘Z brings it back");
        // A plain Delete or Backspace still does nothing.
        u.app.session.set_selection(vec![a]);
        press(&mut u, Key::Backspace, Modifiers::NONE);
        press(&mut u, Key::Delete, Modifiers::NONE);
        assert!(u.app.session.doc().unwrap().node(a).is_some());
    });
}

#[test]
fn cmd_n_opens_new_page_and_shift_cmd_n_lists_workspaces_to_open_in_a_new_tab() {
    crate::with_map_mode(|| {
        let (mut u, _, _) = setup();
        press(&mut u, Key::N, cmd());
        assert!(u.app.new_page.open, "⌘N opens the New page dialog");
        assert!(!u.app.new_workspace.open);
        u.app.new_page.open = false;
        u.frame(vec![]);

        press(&mut u, Key::N, cmd() | Modifiers::SHIFT);
        assert!(crate::switcher::is_open(&u.app), "⇧⌘N lists the existing workspaces");
        assert!(u.app.ui.new_tab_armed, "the one chosen opens in a new window tab");
        assert!(!u.app.new_workspace.open, "creating one is the list's last row");
        assert!(!u.app.ui.home, "this window keeps its map");
        assert!(!u.app.new_page.open);

        // The list's "New workspace…" opens the form over the map, for a new window tab.
        crate::map_shell::new_workspace_form(&mut u.app);
        crate::switcher::close(&mut u.app);
        u.app.ws.pane_open = false;
        u.frame(vec![]);
        assert!(u.app.new_workspace.open);
        assert!(u.app.new_workspace.new_window, "the new workspace opens in a new window tab");
        assert_eq!(u.ctx.memory(|m| m.top_modal_layer()).map(|layer| layer.id), Some(egui::Id::new("new_workspace_modal")), "the modal is drawn");
        assert!(!u.app.ui.home);
    });
}

#[test]
fn the_new_workspace_form_on_the_workspaces_screen_opens_there() {
    crate::with_map_mode(|| {
        let (mut u, _, _) = setup();
        u.app.ui.home = true;
        u.frame(vec![]);
        crate::map_shell::new_workspace_form(&mut u.app);
        assert!(u.app.new_workspace.open);
        assert!(!u.app.new_workspace.new_window, "from the Workspaces screen the workspace opens in place");
    });
}

#[test]
fn the_sidebar_starts_with_the_pages_and_has_no_switcher_or_footer() {
    crate::with_map_mode(|| {
        let (mut u, _, _) = setup();
        u.app.ws.pane_open = true;
        u.frame(vec![]);
        u.frame(vec![]);
        assert!(u.ctx.read_response(egui::Id::new("page_tree_new_page")).is_some(), "the page tree is drawn");
        assert!(u.ctx.read_response(egui::Id::new("workspace_switcher_head")).is_none(), "no workspace switcher");
        for i in 0..2 {
            assert!(u.ctx.read_response(egui::Id::new(("sidebar_footer", i))).is_none(), "no Settings or Add machine footer");
        }
        // Their actions stay in the palette and the menus.
        for id in ["map.switchWorkspace", "map.preferences"] {
            assert!(crate::map_action_catalog::find(id).is_some_and(|a| crate::map_action_catalog::enabled(&u.app, a)), "{id}");
        }
        assert!(crate::native_menu::enabled(&u.app, crate::native_menu::ADD_MACHINE));
    });
}

/// Afterburn has no FigCraft comment threads: a map that already holds one keeps it stored, and
/// the viewer neither opens it nor lets it be changed.
#[test]
fn map_mode_never_shows_a_stored_comment_thread() {
    crate::with_map_mode(|| {
        let (mut u, _, _) = setup();
        let c = u.app.session.execute("comment.add", &json!({"x": 150, "y": 150, "text": "Hello", "author": "Sam"})).unwrap()["id"].as_u64().unwrap();
        assert!(u.app.session.ds().unwrap().view.comments);
        assert!(u.app.run("comment.open", json!({"id": c})).is_none());
        assert!(u.app.run("comment.reply", json!({"id": c, "text": "no"})).is_none());
        // Even a thread opened behind the gate is closed by the next frame, and its pin takes
        // no click.
        u.app.ui.comments.show(c);
        u.frame(vec![]);
        assert_eq!(u.app.ui.comments.open, None);
        let pin = u.app.to_screen(150.0, 150.0) + egui::vec2(14.0, -14.0);
        u.button(pin, true);
        u.button(pin, false);
        u.frame(vec![]);
        assert_eq!(u.app.ui.comments.open, None);
        // The thread is still in the document, untouched.
        assert!(u.app.session.doc().unwrap().comment(c).unwrap().replies.is_empty());
    });
}
