//! The map's title row, driven through egui headless: tabs follow the open page, ⌘W closes a tab
//! (never a page), ⇧⌘W closes the map, the cells fill the row, and the panes keep their widths.
use crate::layers_drag_tests::Ui;
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
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

fn click(u: &mut Ui, at: Pos2, button: PointerButton) {
    u.frame(vec![Event::PointerMoved(at)]);
    let ev = |pressed| Event::PointerButton { pos: at, button, pressed, modifiers: Modifiers::NONE };
    u.frame(vec![ev(true)]);
    u.frame(vec![ev(false)]);
}

/// A map with three pages; the first is open.
fn setup() -> (Ui, [NodeId; 3]) {
    let mut s = Session::new();
    let first = s.page().unwrap();
    s.execute("node.rename", &json!({"id": first.0, "name": "Overview"})).ok();
    let add = |s: &mut Session, name: &str| NodeId(s.execute("page.add", &json!({"name": name})).unwrap()["id"].as_u64().unwrap());
    let second = add(&mut s, "Components");
    let third = add(&mut s, "Fills");
    s.execute("page.select", &json!({"id": first.0})).unwrap();
    let mut u = Ui::new(s);
    u.app.onboarding.active = false;
    u.app.onboarding.decided = true;
    u.app.ui.home = false;
    u.frame(vec![]);
    u.frame(vec![]);
    (u, [first, second, third])
}

fn select(u: &mut Ui, page: NodeId) {
    u.app.run("page.select", json!({"id": page.0}));
    u.frame(vec![]);
}

fn page_count(u: &Ui) -> usize {
    u.app.session.doc().unwrap().pages.len()
}

#[test]
fn tabs_follow_the_selected_page() {
    crate::with_map_mode(|| {
        let (mut u, [a, b, c]) = setup();
        assert_eq!(crate::map_tabs::open(&u.app), vec![a]);
        select(&mut u, c);
        select(&mut u, b);
        assert_eq!(crate::map_tabs::open(&u.app), vec![a, c, b], "each page opened gets a tab, in order");
        select(&mut u, a);
        assert_eq!(crate::map_tabs::open(&u.app), vec![a, c, b], "an open page keeps its tab");
    });
}

#[test]
fn clicking_a_tab_opens_its_page_and_middle_click_closes_it() {
    crate::with_map_mode(|| {
        let (mut u, [a, b, _]) = setup();
        select(&mut u, b);
        let cell = |u: &Ui, page| u.app.page_tabs.cells.iter().find(|(id, _)| *id == Some(page)).unwrap().1;
        let at = cell(&u, a).center();
        click(&mut u, at, PointerButton::Primary);
        assert_eq!(u.app.session.page().unwrap(), a);
        let at = cell(&u, b).center();
        click(&mut u, at, PointerButton::Middle);
        assert_eq!(crate::map_tabs::open(&u.app), vec![a]);
        assert_eq!(page_count(&u), 3, "closing a tab never deletes its page");
    });
}

#[test]
fn the_close_button_closes_any_tab_never_its_page_the_last_one_too() {
    crate::with_map_mode(|| {
        let (mut u, [a, b, c]) = setup();
        select(&mut u, b);
        select(&mut u, c);
        let cell = |u: &Ui, page| u.app.page_tabs.cells.iter().find(|(id, _)| *id == Some(page)).unwrap().1;
        // The active tab's ×.
        let at = crate::map_tabs::close_rect(cell(&u, c)).center();
        click(&mut u, at, PointerButton::Primary);
        assert_eq!(crate::map_tabs::open(&u.app), vec![a, b]);
        assert_eq!(page_count(&u), 3, "closing a tab never deletes its page");
        // An inactive tab's ×.
        let at = crate::map_tabs::close_rect(cell(&u, a)).center();
        click(&mut u, at, PointerButton::Primary);
        assert_eq!(crate::map_tabs::open(&u.app), vec![b], "the inactive tab closed, not selected");
        // The last tab has its × too: no page is open after it.
        u.frame(vec![]);
        let at = crate::map_tabs::close_rect(cell(&u, b)).center();
        click(&mut u, at, PointerButton::Primary);
        assert!(crate::map_tabs::open(&u.app).is_empty());
        assert!(crate::map_tabs::is_empty(&u.app));
        assert_eq!(page_count(&u), 3, "closing the last tab never deletes its page");
    });
}

#[test]
fn command_w_closes_the_active_tab_never_the_page() {
    crate::with_map_mode(|| {
        let (mut u, [a, b, c]) = setup();
        select(&mut u, b);
        select(&mut u, c);
        select(&mut u, b);
        press(&mut u, Key::W, cmd());
        assert_eq!(crate::map_tabs::open(&u.app), vec![a, c]);
        assert_eq!(u.app.session.page().unwrap(), c, "the next tab's page opens");
        assert_eq!(page_count(&u), 3);
        assert!(u.app.page_delete.is_none(), "no delete is even asked");
        assert!(!u.app.ui.home, "⌘W no longer closes the map");
        press(&mut u, Key::W, cmd());
        assert_eq!(crate::map_tabs::open(&u.app), vec![a], "the last tab before it opens");
        assert_eq!(u.app.session.page().unwrap(), a);
        // The last tab closes too, and the map stays open with no page.
        press(&mut u, Key::W, cmd());
        assert!(crate::map_tabs::open(&u.app).is_empty());
        assert!(crate::map_tabs::is_empty(&u.app));
        assert_eq!(page_count(&u), 3);
        assert!(!u.app.ui.home);
    });
}

#[test]
fn shift_command_w_closes_the_map() {
    crate::with_map_mode(|| {
        let (mut u, _) = setup();
        press(&mut u, Key::W, cmd() | Modifiers::SHIFT);
        assert!(u.app.ui.home);
    });
}

#[test]
fn close_others_and_deleted_pages_drop_their_tabs() {
    crate::with_map_mode(|| {
        let (mut u, [a, b, c]) = setup();
        select(&mut u, b);
        select(&mut u, c);
        crate::map_tabs::close_others(&mut u.app, b);
        u.frame(vec![]);
        assert_eq!(crate::map_tabs::open(&u.app), vec![b]);
        assert_eq!(u.app.session.page().unwrap(), b);
        select(&mut u, a);
        select(&mut u, c);
        u.app.session.execute("page.delete", &json!({"id": a.0})).unwrap();
        u.frame(vec![]);
        assert_eq!(crate::map_tabs::open(&u.app), vec![b, c]);
    });
}

#[test]
fn tab_cells_fill_the_title_row_with_centred_labels() {
    crate::with_map_mode(|| {
        let (mut u, [_, b, _]) = setup();
        select(&mut u, b);
        let cells = u.app.page_tabs.cells.clone();
        assert_eq!(cells.len(), 3, "two tabs, then + Page");
        assert_eq!(cells.last().unwrap().0, None);
        for (_, cell) in &cells {
            assert_eq!(cell.top(), 0.0);
            assert_eq!(cell.height(), 44.0, "full header height");
            assert_eq!(crate::map_tabs::label_pos(*cell).y, cell.center().y);
            assert_eq!(crate::map_tabs::label_pos(*cell).x, cell.left() + crate::map_tabs::PAD);
        }
        // Flat cells side by side, a 1 pt separator between them, from the sidebar toggle.
        for pair in cells.windows(2) {
            assert_eq!(pair[1].1.left() - pair[0].1.right(), crate::map_tabs::SEPARATOR);
        }
        assert!(cells[0].1.left() < 60.0, "the strip starts at the window's left, after the toggle");
    });
}

#[test]
fn many_tabs_shrink_then_scroll() {
    crate::with_map_mode(|| {
        let (mut u, _) = setup();
        let pages: Vec<NodeId> = (0..40)
            .map(|i| {
                NodeId(u.app.session.execute("page.add", &json!({"name": format!("A rather long page name {i}")})).unwrap()["id"].as_u64().unwrap())
            })
            .collect();
        for page in &pages {
            select(&mut u, *page);
        }
        let cells = u.app.page_tabs.cells.clone();
        assert!(cells.iter().filter(|(id, _)| id.is_some()).all(|(_, r)| (r.width() - crate::map_tabs::MIN_CELL).abs() < 0.5));
        // The active (last) tab was scrolled into view.
        let active = cells.iter().find(|(id, _)| *id == pages.last().copied()).unwrap().1;
        assert!(active.right() <= 1440.0 && active.left() >= 0.0);
        assert!(crate::map_tabs::inspect(&u.app)["scroll"].as_f64().unwrap() > 0.0);
    });
}

#[test]
fn dragged_sidebar_width_persists_in_the_preferences() {
    crate::with_map_mode(|| {
        let (mut u, _) = setup();
        u.app.ws.pane_open = true;
        u.frame(vec![]);
        let left = crate::map_panes::width(&u.app, 1440.0);
        assert_eq!(left, crate::map_panes::SIDEBAR_DEFAULT);
        // Drag the sidebar's inner edge 40 pt right.
        let from = Pos2::new(left, 400.0);
        u.frame(vec![Event::PointerMoved(from)]);
        u.frame(vec![Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
        for step in 1..=4 {
            u.frame(vec![Event::PointerMoved(from + egui::vec2(40.0 * step as f32 / 4.0, 0.0))]);
        }
        let to = from + egui::vec2(40.0, 0.0);
        u.frame(vec![Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
        u.frame(vec![]);
        let prefs = &u.app.session.prefs;
        assert_eq!(prefs.map_sidebar_width, Some(left + 40.0));
        // Saved and read back.
        let back: figcraft_engine::Prefs = serde_json::from_value(serde_json::to_value(prefs).unwrap()).unwrap();
        assert_eq!(back.map_sidebar_width, Some(left + 40.0));
        assert_eq!(crate::map_panes::width(&u.app, 1440.0), left + 40.0);
    });
}

#[test]
fn after_the_last_tab_closes_opening_any_page_brings_a_tab_back() {
    crate::with_map_mode(|| {
        let (mut u, [a, b, _]) = setup();
        u.app.session.execute("select.set", &json!({"ids": []})).ok();
        crate::map_tabs::close(&mut u.app, a);
        u.frame(vec![]);
        u.frame(vec![]);
        assert!(crate::map_tabs::is_empty(&u.app), "the current page doesn't sneak its tab back");
        assert!(u.app.session.selection().is_empty(), "nothing out of sight stays selected");
        // The tree opens the very page that was showing: it gets its tab again.
        select(&mut u, a);
        assert_eq!(crate::map_tabs::open(&u.app), vec![a]);
        assert!(!crate::map_tabs::is_empty(&u.app));
        crate::map_tabs::close(&mut u.app, a);
        select(&mut u, b);
        assert_eq!(crate::map_tabs::open(&u.app), vec![b], "another page opens with its tab");
    });
}

fn double_click(u: &mut Ui, at: Pos2) {
    // Half a second apart from any earlier clicks, so these two read as one double-click.
    for _ in 0..30 {
        u.frame(vec![]);
    }
    u.frame(vec![Event::PointerMoved(at)]);
    for _ in 0..2 {
        let ev = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        u.frame(vec![ev(true)]);
        u.frame(vec![ev(false)]);
    }
    u.frame(vec![]);
    u.frame(vec![]);
}

fn type_text(u: &mut Ui, text: &str) {
    u.frame(vec![Event::Text(text.to_string())]);
}

fn page_name(u: &Ui, page: NodeId) -> String {
    u.app.session.doc().unwrap().node(page).unwrap().name.clone()
}

#[test]
fn a_tab_double_click_never_renames_the_tree_does_and_esc_keeps_the_name() {
    crate::with_map_mode(|| {
        let (mut u, [a, b, _]) = setup();
        u.app.ws.pane_open = true;
        select(&mut u, b);
        u.frame(vec![]);
        let tab = |u: &Ui, page| u.app.page_tabs.cells.iter().find(|(id, _)| *id == Some(page)).unwrap().1;
        let before = page_name(&u, b);
        let at = tab(&u, b).center();
        double_click(&mut u, at);
        assert!(u.app.page_tree.renaming.is_none(), "a tab is never a name field");
        // The tree's row: a double-click turns its name into a field, all selected.
        let row = |u: &Ui, page| u.app.page_tree.drawn.iter().find(|(id, _)| *id == page).unwrap().1;
        let at = row(&u, b).center();
        double_click(&mut u, at);
        assert_eq!(u.app.page_tree.renaming.as_ref().map(|(id, _)| *id), Some(b), "the row became a field");
        type_text(&mut u, "Widgets");
        press(&mut u, Key::Enter, Modifiers::NONE);
        u.frame(vec![]);
        assert_eq!(page_name(&u, b), "Widgets", "Enter renames the page");
        assert_ne!(before, "Widgets");
        assert!(u.app.page_tree.renaming.is_none());
        assert_eq!(crate::map_tabs::open(&u.app), vec![a, b], "its tab stays where it was");
        // Esc keeps the name.
        let at = row(&u, b).center();
        double_click(&mut u, at);
        type_text(&mut u, "Nope");
        press(&mut u, Key::Escape, Modifiers::NONE);
        u.frame(vec![]);
        assert_eq!(page_name(&u, b), "Widgets");
        assert!(u.app.page_tree.renaming.is_none());
        // An empty name keeps the old one too.
        let before = page_name(&u, a);
        crate::page_tree::start_rename(&mut u.app, a);
        u.frame(vec![]);
        u.frame(vec![]);
        press(&mut u, Key::Backspace, Modifiers::NONE);
        press(&mut u, Key::Enter, Modifiers::NONE);
        u.frame(vec![]);
        assert_eq!(page_name(&u, a), before);
    });
}

#[test]
fn the_close_button_shows_only_on_the_hovered_tab() {
    crate::with_map_mode(|| {
        let (mut u, [a, b, _]) = setup();
        select(&mut u, b);
        let cell = |u: &Ui, page| u.app.page_tabs.cells.iter().find(|(id, _)| *id == Some(page)).unwrap().1;
        // Away from the strip, a click where the active tab's × would be just selects it.
        u.frame(vec![Event::PointerMoved(egui::pos2(600.0, 400.0))]);
        let at = crate::map_tabs::close_rect(cell(&u, b)).center();
        let ev = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        // Press without hovering first: the × isn't there yet.
        u.frame(vec![ev(true)]);
        u.frame(vec![ev(false)]);
        assert_eq!(crate::map_tabs::open(&u.app), vec![a, b], "no × until the tab is hovered");
    });
}
