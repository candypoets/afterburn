//! Map surfaces use one vocabulary, without toggling the process-wide map mode in parallel tests.
use std::collections::BTreeSet;

use egui::{Event, Key, Modifiers};
use figcraft_engine::keymap::{self, Platform};

use crate::map_action_catalog::{self as catalog, ACTIONS};

#[test]
fn map_registry_has_exact_agreed_ids_and_parseable_platform_bindings() {
    let expected: BTreeSet<_> = [
        "actions",
        "help",
        "search",
        "openWorkspace",
        "newWorkspace",
        "save",
        "close",
        "preferences",
        "focusMode",
        "fullscreen",
        "pageTree",
        "previousPage",
        "nextPage",
        "workspacePane",
        "comments",
        "cursors",
        "nextRegion",
        "previousRegion",
        "contextMenu",
        "zoomIn",
        "zoomOut",
        "zoom100",
        "zoomFit",
        "zoomSelection",
        "selectAll",
        "selectNone",
        "openDetail",
        "parentPage",
        "nextCard",
        "previousCard",
        "selectArea",
        "copy",
        "copyImage",
        "export",
        "undoTyping",
        "redoTyping",
        "newline",
        "saveDraft",
        "send",
        "switchWorktree",
        "focusComposer",
        "syncReview",
    ]
    .into_iter()
    .map(|id| format!("map.{id}"))
    .chain(["app.checkForUpdates".to_string()])
    .collect();
    let actual: BTreeSet<_> = ACTIONS.iter().map(|a| a.id.to_string()).collect();
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), ACTIONS.len(), "no duplicate IDs");
    for action in ACTIONS {
        assert!(!action.label.is_empty() && !action.group.is_empty() && !action.scope.is_empty());
        assert_eq!(catalog::find(action.id).unwrap().label, action.label);
        for platform in [Platform::Mac, Platform::Other] {
            let keys = action.keys(platform);
            assert!(keys.is_empty() || keymap::parse(keys).is_some(), "{}: {keys}", action.id);
        }
    }
    assert!(catalog::find("edit.delete").is_none());
}

#[test]
fn map_help_and_palette_have_exact_registry_parity_and_no_editor_actions() {
    let expected: BTreeSet<_> = ACTIONS.iter().map(|a| a.id).collect();
    let sections = crate::menus::map_command_sections("");
    let palette: BTreeSet<_> = sections.iter().flat_map(|(_, rows)| rows).map(|row| row.key.strip_prefix("command:").unwrap()).collect();
    assert_eq!(palette, expected);
    for platform in [Platform::Mac, Platform::Other] {
        let help = crate::shortcuts_panel::map_help_rows("", platform);
        assert_eq!(help.iter().filter_map(|row| row.id).collect::<BTreeSet<_>>(), expected);
        assert_eq!(help.iter().filter(|row| row.id.is_none()).count(), crate::map_action_catalog::GESTURES.len());
        for row in help.iter().filter(|row| row.id.is_some()) {
            let action = catalog::find(row.id.unwrap()).unwrap();
            assert_eq!((row.label, row.scope), (action.label, action.scope));
            let mut expected: Vec<_> = action.bindings(platform).map(|keys| keymap::display(keys, platform)).collect();
            if expected.is_empty() {
                expected.push("Actions palette".into());
            }
            assert_eq!(row.keys, expected.join(" or "));
        }
    }
    for query in ["rectangle", "prototype", "plugin:", "edit.delete", "object.group", "shape."] {
        assert!(crate::menus::map_command_sections(query).is_empty(), "{query}");
        assert!(crate::shortcuts_panel::map_help_rows(query, Platform::Mac).is_empty(), "{query}");
    }
}

#[test]
fn palette_only_actions_and_platform_help_are_explicit() {
    let palette_only: BTreeSet<_> = ACTIONS.iter().filter(|a| a.mac.is_empty() && a.other.is_empty()).map(|a| a.id).collect();
    assert_eq!(palette_only, BTreeSet::from(["map.switchWorktree", "map.focusComposer", "map.syncReview", "app.checkForUpdates"]));
    let mac = crate::shortcuts_panel::map_help_rows("map.actions", Platform::Mac);
    let other = crate::shortcuts_panel::map_help_rows("map.actions", Platform::Other);
    assert_eq!(mac[0].keys, format!("{} or {}", keymap::display("Shift+Cmd+P", Platform::Mac), keymap::display("Cmd+K", Platform::Mac)));
    assert_eq!(other[0].keys, "Ctrl+Shift+P or Ctrl+K");
    let tree = crate::shortcuts_panel::map_help_rows("map.pageTree", Platform::Other);
    assert_eq!(tree[0].keys, "Ctrl+P");
    assert_eq!(catalog::find("map.fullscreen").unwrap().other, "F11");
    assert_eq!(catalog::find("map.contextMenu").unwrap().mac, "Shift+F10");
    assert_eq!(catalog::find("map.selectArea").unwrap().mac, "F8");
    assert!(crate::shortcuts_panel::map_help_rows("map.selectArea", Platform::Mac)[0].scope.contains("Shift+arrows"));
}

#[test]
fn zoom_in_displays_plus_and_preserves_shift_equals_lookup() {
    let action = catalog::find("map.zoomIn").unwrap();
    for (platform, old_binding, displayed, modifiers) in [
        (Platform::Mac, "Cmd+Plus", "⌘+", Modifiers { shift: true, command: true, mac_cmd: true, ..Modifiers::NONE }),
        (Platform::Other, "Ctrl+Plus", "Ctrl++", Modifiers { shift: true, command: true, ctrl: true, ..Modifiers::NONE }),
    ] {
        assert_eq!(keymap::display(action.keys(platform), platform), displayed);
        assert_eq!(crate::shortcuts_panel::map_help_rows("map.zoomIn", platform)[0].keys, displayed);
        assert_eq!(keymap::parse(action.keys(platform)), keymap::parse(old_binding), "binding semantics stay identical");
        assert_eq!(crate::map_actions::lookup(platform, Key::Equals, Some(Key::Equals), modifiers).map(|action| action.id), Some("map.zoomIn"));
        assert_eq!(crate::map_actions::lookup(platform, Key::Plus, Some(Key::Equals), modifiers).map(|action| action.id), Some("map.zoomIn"));
    }
}

#[test]
fn help_search_includes_gestures_and_distinguishes_canvas_and_field_scopes() {
    for (platform, zoom) in [(Platform::Mac, "Cmd + scroll"), (Platform::Other, "Ctrl + scroll")] {
        let pan = crate::shortcuts_panel::map_help_rows("Pan canvas", platform);
        assert_eq!(pan.len(), 1);
        assert_eq!(pan[0].keys, "Space + drag");
        let rows = crate::shortcuts_panel::map_help_rows("Zoom around pointer", platform);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].keys, zoom);
        assert!(rows[0].scope.contains("pinch"));
        assert!(
            crate::shortcuts_panel::map_help_rows("Text field only", platform)
                .iter()
                .all(|row| matches!(row.id, Some("map.undoTyping" | "map.redoTyping")))
        );
    }
    assert!(catalog::find("map.openDetail").unwrap().scope.starts_with("Selection:"));
    assert!(catalog::find("map.newline").unwrap().scope.starts_with("Composer only:"));
    assert_eq!(catalog::find("map.openDetail").unwrap().mac, catalog::find("map.newline").unwrap().mac);
}

#[test]
fn field_actions_and_editor_commands_cannot_activate_from_map_palette() {
    let mut app = crate::FigApp::new(figcraft_engine::Session::new(), Box::new(crate::NoServices));
    let ctx = egui::Context::default();
    app.compose.input = "Unsent prompt".to_string();
    app.ui.palette = Some("send".to_string());
    let selection = app.session.selection();
    for id in ["map.undoTyping", "map.redoTyping", "map.newline", "map.saveDraft", "map.send", "edit.undo", "edit.delete", "plugin:example/run"] {
        assert!(!crate::menus::activate_map_action(&mut app, &ctx, id), "{id}");
        assert_eq!(app.compose.input, "Unsent prompt");
        assert_eq!(app.ui.palette.as_deref(), Some("send"));
        assert_eq!(app.session.selection(), selection);
        assert!(app.session.prefs.recent_actions.is_empty());
    }
    for id in ["map.copy", "map.copyImage", "map.zoomSelection", "map.openDetail"] {
        assert!(!catalog::enabled(&app, catalog::find(id).unwrap()), "selection required: {id}");
    }
}

#[test]
fn context_actions_and_menu_aliases_resolve_only_to_map_ids() {
    for id in crate::menus::MAP_CONTEXT_ACTIONS {
        assert!(catalog::find(id).is_some());
    }
    for (old, new) in [
        ("view.contextMenu", "map.contextMenu"),
        ("edit.copy", "map.copy"),
        ("file.open", "map.openWorkspace"),
        ("view.prevPage", "map.previousPage"),
    ] {
        assert_eq!(crate::menus::map_action_id(old), Some(new));
    }
    for id in ["edit.undo", "edit.redo", "edit.paste", "edit.cut", "tool.rectangle", "object.group", "plugin.import"] {
        assert!(crate::menus::map_action_id(id).is_none(), "{id}");
    }
}

fn key(key: Key) -> Vec<Event> {
    vec![
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE },
        Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE },
    ]
}

fn palette_frame(app: &mut crate::FigApp, ctx: &egui::Context, events: Vec<Event>) {
    let mut output = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0))), events, ..Default::default() },
        |ui| crate::menus::map_palette(app, ui.ctx()),
    );
    output.textures_delta.clear();
}

#[test]
fn enter_on_field_action_keeps_palette_open_and_escape_closes_it() {
    let mut app = crate::FigApp::new(figcraft_engine::Session::new(), Box::new(crate::NoServices));
    let ctx = egui::Context::default();
    app.compose.input = "Do not send this".to_string();
    crate::menus::set_palette(&mut app.ui, true);
    app.ui.palette = Some("map.send".to_string());
    palette_frame(&mut app, &ctx, vec![]);
    palette_frame(&mut app, &ctx, key(Key::Enter));
    assert_eq!(app.ui.palette.as_deref(), Some("map.send"));
    assert_eq!(app.compose.input, "Do not send this");
    assert!(app.session.prefs.recent_actions.is_empty());
    palette_frame(&mut app, &ctx, key(Key::Escape));
    assert!(app.ui.palette.is_none());
}

#[test]
fn keyboard_context_menu_opens_and_escape_preserves_selection_and_prompt() {
    let mut app = crate::FigApp::new(figcraft_engine::Session::new(), Box::new(crate::NoServices));
    let ctx = egui::Context::default();
    app.compose.input = "Keep draft".to_string();
    let selection = app.session.selection();
    let pos = egui::pos2(100.0, 100.0);
    app.ui.context_menu = Some((pos, selection.clone()));
    let mut output = ctx
        .run_ui(egui::RawInput { events: key(Key::ArrowDown), ..Default::default() }, |ui| crate::menus::map_context_menu(&mut app, ui.ctx(), pos));
    output.textures_delta.clear();
    assert!(app.ui.context_menu.is_some());
    let mut output =
        ctx.run_ui(egui::RawInput { events: key(Key::Escape), ..Default::default() }, |ui| crate::menus::map_context_menu(&mut app, ui.ctx(), pos));
    output.textures_delta.clear();
    assert!(app.ui.context_menu.is_none());
    assert_eq!(app.session.selection(), selection);
    assert_eq!(app.compose.input, "Keep draft");
}

fn map_app() -> crate::FigApp {
    let mut app = crate::FigApp::new(figcraft_engine::Session::new(), Box::new(crate::NoServices));
    app.ui.home = false;
    app
}

fn ids(rows: &[&catalog::Action]) -> Vec<&'static str> {
    rows.iter().map(|action| action.id).collect()
}

#[test]
fn shift_cmd_p_opens_the_palette_cmd_k_still_does_and_cmd_p_opens_the_page_tree() {
    let mac = Modifiers { command: true, mac_cmd: true, ..Modifiers::NONE };
    let other = Modifiers { command: true, ctrl: true, ..Modifiers::NONE };
    for (platform, cmd) in [(Platform::Mac, mac), (Platform::Other, other)] {
        let shift = Modifiers { shift: true, ..cmd };
        let lookup = |key, modifiers| crate::map_actions::lookup(platform, key, Some(key), modifiers).map(|action| action.id);
        assert_eq!(lookup(Key::P, shift), Some("map.actions"));
        assert_eq!(lookup(Key::K, cmd), Some("map.actions"), "⌘K stays an alias");
        assert_eq!(lookup(Key::P, cmd), Some("map.pageTree"));
        let palette = catalog::find("map.actions").unwrap();
        assert_eq!(palette.bindings(platform).count(), 2);
        assert_eq!(catalog::find("map.pageTree").unwrap().bindings(platform).count(), 1);
    }
    // No other action shares a binding with the palette's alias.
    for action in ACTIONS.iter().filter(|action| action.id != "map.actions") {
        for platform in [Platform::Mac, Platform::Other] {
            assert!(
                !action
                    .bindings(platform)
                    .any(|keys| keymap::parse(keys) == keymap::parse(if platform == Platform::Mac { "Cmd+K" } else { "Ctrl+K" }))
            );
        }
    }
}

#[test]
fn recently_used_actions_come_first_once_and_the_palette_records_runs() {
    let mut app = map_app();
    let ctx = egui::Context::default();
    let all = crate::map_palette::sections(&app, "");
    assert_ne!(all[0].0, Some(crate::map_palette::RECENTLY_USED), "nothing used yet");
    app.session.prefs.recent_actions = vec!["command:map.zoomFit".into(), "command:edit.copy".into(), "command:map.pageTree".into()];
    let sections = crate::map_palette::sections(&app, "");
    assert_eq!(sections[0].0, Some(crate::map_palette::RECENTLY_USED));
    assert_eq!(ids(&sections[0].1), ["map.zoomFit", "map.pageTree"], "editor commands never show in the map palette");
    let rows: Vec<_> = sections.iter().flat_map(|(_, rows)| rows.iter().map(|action| action.id)).collect();
    assert_eq!(rows.len(), ACTIONS.len(), "every action exactly once");
    assert_eq!(rows.iter().collect::<BTreeSet<_>>().len(), ACTIONS.len());
    let navigation = sections.iter().find(|(group, _)| *group == Some("Pages + navigation")).unwrap();
    assert!(!ids(&navigation.1).contains(&"map.pageTree"), "a recent action leaves its group");
    assert!(crate::menus::activate_map_action(&mut app, &ctx, "map.workspacePane"));
    assert_eq!(app.session.prefs.recent_actions[0], "command:map.workspacePane");
    assert_eq!(crate::map_palette::sections(&app, "")[0].1[0].id, "map.workspacePane");
    assert!(crate::map_palette::sections(&app, "fit").iter().all(|(group, _)| group.is_none()), "a search is one ranked list");
}

#[test]
fn search_ranks_label_matches_first_and_counts_against_the_catalog() {
    let hits = catalog::ranked("work", Platform::Mac);
    assert_eq!(hits[0].id, "map.workspacePane");
    assert!(hits.iter().any(|action| action.id == "map.switchWorktree"));
    assert!(hits.iter().all(|action| action.label.to_lowercase().contains("work")), "labels answer before groups");
    assert!(hits.len() < catalog::search("work", Platform::Mac).len());
    assert_eq!(catalog::ranked("navigation", Platform::Mac).len(), ACTIONS.iter().filter(|a| a.group == "Pages + navigation").count());
    assert_eq!(crate::map_palette::count_label("", 0), format!("{} actions", ACTIONS.len()));
    assert_eq!(crate::map_palette::count_label("work", 5), format!("5 of {}", ACTIONS.len()));
}

#[test]
fn unavailable_actions_stay_listed_with_a_reason_and_cannot_run() {
    let mut app = map_app();
    let ctx = egui::Context::default();
    let find = |id| catalog::find(id).unwrap();
    assert_eq!(catalog::disabled_reason(&app, find("map.zoomSelection")), Some("Select a card first"));
    assert_eq!(catalog::disabled_reason(&app, find("map.copyImage")), Some("Select a card first"));
    assert_eq!(catalog::disabled_reason(&app, find("map.zoomFit")), None);
    assert_eq!(catalog::disabled_reason(&app, find("map.send")), Some("Only in the composer"));
    assert_eq!(catalog::disabled_reason(&app, find("map.undoTyping")), Some("Only in a text field"));
    assert_eq!(catalog::disabled_reason(&app, find("map.workspacePane")), None);
    for action in ACTIONS {
        assert_eq!(catalog::enabled(&app, action), catalog::disabled_reason(&app, action).is_none(), "{}", action.id);
    }
    assert!(ids(&crate::map_palette::sections(&app, "fit selection")[0].1).contains(&"map.zoomSelection"), "still listed");
    crate::menus::set_palette(&mut app.ui, true);
    app.ui.palette = Some("fit selection".into());
    palette_frame(&mut app, &ctx, vec![]);
    palette_frame(&mut app, &ctx, key(Key::Enter));
    assert_eq!(app.ui.palette.as_deref(), Some("fit selection"), "Enter on a dimmed row does nothing");
    assert!(app.session.prefs.recent_actions.is_empty());
    app.ui.home = true;
    assert_eq!(catalog::disabled_reason(&app, find("map.zoomFit")), Some("Open a map first"));
    assert_eq!(catalog::disabled_reason(&app, find("map.copy")), Some("Open a map first"));
    assert_eq!(catalog::disabled_reason(&app, find("map.help")), None);
}

#[test]
fn toggles_show_their_current_state() {
    let mut app = map_app();
    let find = |id| catalog::find(id).unwrap();
    app.ws.pane_open = true;
    assert_eq!(catalog::toggle_state(&app, find("map.workspacePane")), Some("Shown"));
    app.ws.pane_open = false;
    assert_eq!(catalog::toggle_state(&app, find("map.workspacePane")), Some("Hidden"));
    app.session.ds_mut().unwrap().view.comments = true;
    app.session.ds_mut().unwrap().view.multiplayer_cursors = false;
    assert_eq!(catalog::toggle_state(&app, find("map.comments")), Some("Shown"));
    assert_eq!(catalog::toggle_state(&app, find("map.cursors")), Some("Hidden"));
    assert_eq!(catalog::toggle_state(&app, find("map.focusMode")), Some("Off"));
    assert_eq!(catalog::toggle_state(&app, find("map.fullscreen")), Some("Off"));
    assert_eq!(catalog::toggle_state(&app, find("map.zoomFit")), None);
    app.ui.home = true;
    assert_eq!(catalog::toggle_state(&app, find("map.comments")), None, "map state needs an open map");
}

#[test]
fn no_match_offers_to_search_the_map_with_the_query() {
    let mut app = map_app();
    let ctx = egui::Context::default();
    assert!(crate::map_palette::sections(&app, "retry button").is_empty());
    assert_eq!(crate::map_palette::search_map_label(" retry button "), "Search this map for “retry button”");
    assert_eq!(crate::map_palette::no_match_label("retry button"), "No action named “retry button”.   ↵ search the map");
    assert_eq!(crate::map_palette::count_label("retry button", 0), format!("0 of {}", ACTIONS.len()));
    crate::menus::set_palette(&mut app.ui, true);
    app.ui.palette = Some("retry button".into());
    palette_frame(&mut app, &ctx, vec![]);
    palette_frame(&mut app, &ctx, key(Key::Enter));
    assert!(app.ui.palette.is_none());
    assert!(crate::map_navigation::search_open(&ctx), "map search opens with the query");
    assert!(app.session.prefs.recent_actions.is_empty(), "searching is not a recent action");
    crate::map_navigation::close_search(&ctx);
    app.ui.home = true;
    assert!(!crate::map_palette::search_map(&mut app, &ctx, "retry button"), "no map, no search");
    assert!(!crate::map_navigation::search_open(&ctx));
}
