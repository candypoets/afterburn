//! The native menu bar's model: the design's menus in order, every item a known command id, only
//! the bindings a human can use in the map, and ⌘ key equivalents alone.
use std::collections::HashSet;

use figcraft_engine::Session;
use figcraft_engine::keymap::{KEYS, Platform, parse};

use crate::layers_drag_tests::Ui;
use crate::map_action_catalog::{ACTIONS, find};
use crate::native_menu::{
    ADD_MACHINE, CHECK_FOR_UPDATES, DOCS, Entry, Menu, Native, Role, WORKSPACE_KEYS, enabled, key_equivalent, menus, run, workspace_items,
};

fn items(menu: &Menu) -> Vec<(&'static str, &'static str)> {
    menu.entries.iter().filter_map(|e| if let Entry::Item { id, label } = e { Some((*id, *label)) } else { None }).collect()
}

fn all_ids() -> Vec<&'static str> {
    menus().iter().flat_map(|m| items(m).into_iter().map(|(id, _)| id)).collect()
}

fn menu(title: &str) -> Menu {
    menus().into_iter().find(|m| m.title == title).unwrap()
}

#[test]
fn the_bar_is_app_file_workspace_edit_view_window_help() {
    let bar = menus();
    let titles: Vec<_> = bar.iter().map(|m| m.title).collect();
    assert_eq!(titles, ["Mindmap", "File", "Workspace", "Edit", "View", "Window", "Help"]);
    let roles: Vec<_> = bar.iter().map(|m| m.role).collect();
    assert_eq!(roles, [Role::App, Role::Plain, Role::Plain, Role::Plain, Role::Plain, Role::Window, Role::Help]);
}

#[test]
fn app_window_and_help_menus_follow_the_design() {
    assert_eq!(
        menu("Mindmap").entries,
        [
            Entry::Native(Native::About),
            Entry::Item { id: CHECK_FOR_UPDATES, label: "Check for Updates…" },
            Entry::Separator,
            Entry::Item { id: "map.preferences", label: "Settings…" },
            Entry::Separator,
            Entry::Native(Native::Hide),
            Entry::Native(Native::Quit),
        ]
    );
    assert_eq!(
        menu("Window").entries,
        [
            Entry::Native(Native::Minimize),
            Entry::Native(Native::Zoom),
            Entry::Separator,
            Entry::Item { id: "map.previousWindowTab", label: "Show previous window tab" },
            Entry::Item { id: "map.nextWindowTab", label: "Show next window tab" },
            Entry::Separator,
            Entry::Native(Native::BringAllToFront),
        ]
    );
    assert_eq!(key_equivalent("map.nextWindowTab"), Some("Shift+Cmd+]"));
    assert_eq!(key_equivalent("map.newWindowTab"), Some("Cmd+T"));
    assert_eq!(
        menu("Help").entries,
        [Entry::Item { id: "map.help", label: "Keyboard shortcuts" }, Entry::Item { id: DOCS, label: "Docs" }, Entry::Native(Native::About)]
    );
    let file: Vec<_> = items(&menu("File")).into_iter().map(|(_, label)| label).collect();
    assert_eq!(file, ["New page…", "New workspace…", "New window tab…", "Close tab", "Close map", "Burn…", "Save", "Export…"]);
    assert_eq!(key_equivalent("map.newPage"), Some("Cmd+N"));
    assert_eq!(key_equivalent("map.newWorkspace"), Some("Shift+Cmd+N"));
}

#[test]
fn the_workspace_menu_lists_workspaces_then_switches_and_creates() {
    assert_eq!(
        menu("Workspace").entries,
        [
            Entry::Workspaces,
            Entry::Separator,
            Entry::Item { id: "map.switchWorkspace", label: "Switch workspace…" },
            Entry::Worktrees,
            Entry::Separator,
            Entry::Item { id: "map.openWorkspace", label: "All workspaces…" },
            Entry::Item { id: ADD_MACHINE, label: "Add machine…" },
            Entry::Separator,
            Entry::Item { id: "map.agents", label: "Agents…" },
            Entry::Item { id: "map.newAgent", label: "New agent…" },
        ]
    );
    assert_eq!(key_equivalent("map.switchWorkspace"), Some("Ctrl+Cmd+O"));
    assert_eq!(key_equivalent("map.openWorkspace"), Some("Shift+Cmd+O"));
    for (i, id) in WORKSPACE_KEYS.iter().enumerate() {
        assert_eq!(key_equivalent(id), Some(format!("Ctrl+Cmd+{}", i + 1)).as_deref());
    }
    // No workspace listed: one disabled line.
    let u = Ui::new(Session::new());
    let items = workspace_items(&u.app);
    assert_eq!(items.len(), 1);
    assert!(!items[0].enabled && !enabled(&u.app, &items[0].id));
}

#[test]
fn every_item_id_resolves() {
    for id in all_ids() {
        // Check for Updates and Docs wait for their catalog actions (disabled until then).
        assert!(find(id).is_some() || [ADD_MACHINE, CHECK_FOR_UPDATES, DOCS].contains(&id), "{id} is no command");
    }
    let ids = all_ids();
    assert_eq!(ids.iter().collect::<HashSet<_>>().len(), ids.len(), "an id appears twice");
}

#[test]
fn the_bar_offers_exactly_the_live_bindings_of_the_shortcuts_page() {
    let ids: HashSet<_> = all_ids().into_iter().collect();
    // Every binding a human can use from the map (the shortcuts page minus the field-only keys).
    // ⌃⌘1…⌃⌘9 ride on the listed workspaces, which the Workspace menu builds as they change.
    for action in ACTIONS.iter().filter(|a| !a.keys(Platform::Mac).is_empty() && !WORKSPACE_KEYS.contains(&a.id)) {
        assert_eq!(ids.contains(action.id), action.palette_enabled(), "{} ({})", action.id, action.label);
    }
    // The drawing editor's bindings (tools, text edit, history…) are agent-only here.
    for binding in KEYS {
        assert!(!ids.contains(binding.id), "{} is an editor binding", binding.id);
    }
    for id in ["map.undoTyping", "map.redoTyping", "map.newline", "map.saveDraft", "map.send"] {
        assert!(!ids.contains(id), "{id} belongs to its field");
    }
}

#[test]
fn only_command_shortcuts_become_key_equivalents() {
    assert_eq!(key_equivalent("map.save"), Some("Cmd+S"));
    assert_eq!(key_equivalent("map.newPage"), Some("Cmd+N"));
    assert_eq!(key_equivalent("map.newWorkspace"), Some("Shift+Cmd+N"));
    assert_eq!(key_equivalent("map.preferences"), Some("Cmd+,"));
    assert_eq!(key_equivalent("map.fullscreen"), Some("Ctrl+Cmd+F"));
    assert_eq!(key_equivalent("map.actions"), Some("Shift+Cmd+P"));
    for id in ["map.selectNone", "map.openDetail", "map.nextCard", "map.nextRegion", "map.comments", "map.zoomFit", "map.help", ADD_MACHINE, DOCS] {
        assert_eq!(key_equivalent(id), None, "{id}");
    }
    let mut seen = HashSet::new();
    for id in all_ids().into_iter().chain(WORKSPACE_KEYS) {
        if let Some(keys) = key_equivalent(id) {
            let (mods, _) = parse(keys).unwrap();
            assert!(mods.cmd, "{id}: {keys}");
            assert!(seen.insert(keys), "{keys} is on two items");
        }
    }
}

#[test]
fn separators_only_divide_items() {
    for m in menus() {
        let seps: Vec<bool> = m.entries.iter().map(|e| *e == Entry::Separator).collect();
        assert!(!seps.first().unwrap() && !seps.last().unwrap(), "{}", m.title);
        assert!(!seps.windows(2).any(|w| w[0] && w[1]), "{}", m.title);
    }
}

#[test]
fn enabled_state_comes_from_the_registry() {
    let mut u = Ui::new(Session::new());
    u.app.ui.home = true;
    let app = &u.app;
    assert!(enabled(app, "map.newWorkspace"));
    assert!(enabled(app, "map.preferences"));
    assert!(enabled(app, ADD_MACHINE));
    assert!(!enabled(app, "map.save"), "no map open");
    assert!(!enabled(app, "map.copy"));
    assert!(!enabled(app, DOCS));
    assert_eq!(enabled(app, CHECK_FOR_UPDATES), find(CHECK_FOR_UPDATES).is_some_and(|a| crate::map_action_catalog::enabled(app, a)));
    for id in all_ids() {
        if let Some(action) = find(id) {
            assert_eq!(enabled(app, id), crate::map_action_catalog::enabled(app, action), "{id}");
        }
    }
}

#[test]
fn items_run_through_the_in_app_menu() {
    crate::with_map_mode(|| {
        let mut u = Ui::new(Session::new());
        u.app.ui.home = true;
        assert!(!u.app.ui.shortcuts_panel);
        run(&mut u.app, "map.help");
        assert!(u.app.ui.shortcuts_panel, "Help › Keyboard shortcuts opens the shortcuts page");
        run(&mut u.app, ADD_MACHINE);
        assert!(u.app.machine_modal.open);
        // A disabled or unknown item does nothing.
        run(&mut u.app, DOCS);
        run(&mut u.app, "map.save");
    });
}
