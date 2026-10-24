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
        assert_eq!(help.iter().filter(|row| row.id.is_none()).count(), 2);
        for row in help.iter().filter(|row| row.id.is_some()) {
            let action = catalog::find(row.id.unwrap()).unwrap();
            assert_eq!((row.label, row.scope), (action.label, action.scope));
            let expected = if action.keys(platform).is_empty() { "Actions palette".into() } else { keymap::display(action.keys(platform), platform) };
            assert_eq!(row.keys, expected);
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
    assert_eq!(palette_only, BTreeSet::from(["map.switchWorktree", "map.focusComposer", "map.syncReview"]));
    let mac = crate::shortcuts_panel::map_help_rows("map.actions", Platform::Mac);
    let other = crate::shortcuts_panel::map_help_rows("map.actions", Platform::Other);
    assert_eq!(mac[0].keys, keymap::display("Cmd+K", Platform::Mac));
    assert_eq!(other[0].keys, "Ctrl+K");
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
