//! Afterburn's keyboard dispatch. The catalog is shared with menus, palette and Help;
//! the inherited drawing-editor keymap is never a fallback in map mode.

use egui::{Context, Event, Key, Modifiers};
use figcraft_engine::keymap::Platform;
use serde_json::json;

use crate::{
    FigApp,
    map_action_catalog::{ACTIONS, Action},
};

pub fn lookup(platform: Platform, key: Key, physical: Option<Key>, modifiers: Modifiers) -> Option<&'static Action> {
    let mut mods = crate::shortcuts::mods(modifiers);
    if mods.shift
        && physical.is_some_and(|p| p != key)
        && matches!(key, Key::Slash | Key::Backslash | Key::Minus | Key::Equals | Key::Comma | Key::Quote)
    {
        mods.shift = false;
    }
    ACTIONS.iter().find(|a| a.bound_to(platform, key.name(), mods))
}

/// Text-field actions are handled by the field, never replayed as document commands.
pub fn field_action(id: &str) -> bool {
    matches!(id, "map.undoTyping" | "map.redoTyping" | "map.newline" | "map.saveDraft" | "map.send")
}

fn global_action(id: &str) -> bool {
    matches!(
        id,
        "map.actions"
            | "map.help"
            | "map.preferences"
            | "map.fullscreen"
            | "map.openWorkspace"
            | "map.newWorkspace"
            | "map.newPage"
            | "map.save"
            | "map.close"
            | "map.closeTab"
            | "map.focusMode"
            | "map.workspacePane"
            | "map.pageTree"
            | "map.goToPage"
            | "map.newAgent"
            | "map.search"
            | "map.switchWorkspace"
            | "map.newWindowTab"
            | "map.nextWindowTab"
            | "map.previousWindowTab"
    ) || crate::map_action_catalog::workspace_number(id).is_some()
}

pub fn run(app: &mut FigApp, ctx: &Context, id: &str) -> bool {
    let Some(action) = crate::map_action_catalog::find(id) else { return false };
    if field_action(id) {
        return true;
    }
    if !crate::map_action_catalog::enabled(app, action) {
        return true;
    }
    if crate::map_shell::run(app, ctx, id) || crate::map_navigation::run(app, ctx, id) {
        return true;
    }
    match id {
        "map.actions" => crate::map_palette::toggle_actions(app),
        "map.goToPage" => crate::map_palette::toggle_pages(app),
        "map.agents" => crate::map_palette::open(app, crate::map_palette::Scope::Agents),
        "map.newAgent" => {
            if let Ok(ds) = app.session.ds_mut() {
                ds.view.ui_hidden = false;
            }
            app.compose.open_menu();
        }
        "map.help" => app.ui.shortcuts_panel = !app.ui.shortcuts_panel,
        "map.preferences" => crate::map_palette::toggle(app, crate::map_palette::Scope::Settings),
        "map.fullscreen" => {
            app.run("view.fullscreen", json!({}));
        }
        "map.focusMode" => {
            app.run("view.toggleUi", json!({}));
        }
        "map.workspacePane" => app.ws.pane_open = !app.ws.pane_open,
        // Closes the tab, never the page; closing the last one leaves no page open.
        "map.closeTab" => {
            crate::map_tabs::close_active(app);
        }
        "map.switchWorkspace" => crate::switcher::toggle(app),
        "map.newWindowTab" => crate::window_tabs::request_new_tab(app),
        "map.nextWindowTab" => crate::window_tabs::step(app, ctx, 1),
        "map.previousWindowTab" => crate::window_tabs::step(app, ctx, -1),
        "map.delete" => {
            if let Some(said) = crate::map_delete::delete_selection(app) {
                app.toast(said);
            }
        }
        "map.newSubpage" => {
            crate::new_page::start_subpage(app);
        }
        // ⌘N: the New page dialog, as the page tree's + opens it.
        "map.newPage" => {
            let total = app.session.doc().map_or(0, |doc| doc.pages.len());
            app.new_page.start(total + 1);
        }
        "map.colorTheme" => crate::map_palette::open_themes(app),
        "map.nextColorTheme" => crate::map_palette::next_theme(app),
        "map.followSystemAppearance" => crate::map_palette::follow_system(app),
        "map.undo" => crate::map_delete::undo(app, false),
        "map.redo" => crate::map_delete::undo(app, true),
        id if crate::map_action_catalog::workspace_number(id).is_some() => {
            crate::switcher::open_nth(app, ctx, crate::map_action_catalog::workspace_number(id).unwrap_or(0));
        }
        "map.comments" => {
            app.run("view.comments", json!({}));
        }
        "map.cursors" => {
            app.run("view.multiplayerCursors", json!({}));
        }
        "map.nextRegion" => app.ui.nav.step(1),
        "map.previousRegion" => app.ui.nav.step(-1),
        "map.contextMenu" => crate::a11y::context_menu(app, ctx),
        "map.selectArea" => app.start_key_box(),
        "map.copyImage" => crate::menus::invoke_legacy(app, "edit.copyAsPng"),
        "map.export" => app.ui.export_preview = true,
        "app.checkForUpdates" => crate::update::open(app, ctx),
        id => {
            let engine = match id {
                "map.zoomIn" => "view.zoomIn",
                "map.zoomOut" => "view.zoomOut",
                "map.zoom100" => "view.zoom100",
                "map.zoomFit" => "view.zoomFit",
                "map.zoomSelection" => "view.zoomSelection",
                _ => return false,
            };
            app.run(engine, json!({}));
        }
    }
    ctx.request_repaint();
    true
}

pub fn overlay_open(app: &FigApp, ctx: &Context) -> bool {
    blocking_overlay(app, ctx) || app.compose.menu_open
}

/// Overlays outside the composer suspend its keyboard and send controls.
pub fn blocking_overlay(app: &FigApp, ctx: &Context) -> bool {
    app.ui.palette.is_some()
        || app.ui.context_menu.is_some()
        || app.ui.main_menu
        || app.ui.preferences
        || app.ui.about
        || app.ui.shortcuts_panel
        || app.ui.export_preview
        || app.ui.confirm_close
        || app.ui.menu_input.is_some()
        || app.ui.find.is_some()
        || app.ui.color_profile.is_some()
        || app.ui.import_report.is_some()
        || app.ui.missing_fonts.is_some()
        || app.page_tree.open
        || app.agents_modal.open
        || app.sync.menu_open
        || app.new_page.open
        || app.page_delete.is_some()
        || app.machine_modal.open
        || app.new_workspace.open
        || app.update.open
        || app.compose.confirm.is_some()
        || crate::map_navigation::search_open(ctx)
        || egui::Popup::is_any_open(ctx)
}

/// Consume Escape before independently drawn overlays can all observe it in one frame.
pub fn begin(app: &mut FigApp, ctx: &Context, typing: bool) {
    crate::shortcuts::note_platform(ctx);
    // App chords precede TextEdit (Ctrl+W, for example, otherwise deletes a word there).
    // Field-local clipboard and history chords are deliberately absent from this set.
    for event in ctx.input(|i| i.events.clone()) {
        if let Event::Key { key, physical_key, pressed: true, modifiers, repeat: false } = event
            && let Some(action) = lookup(crate::shortcuts::platform(), key, physical_key, modifiers)
            && global_action(action.id)
            && (!overlay_open(app, ctx) || (app.ui.palette.is_some() && crate::map_palette::box_action(action.id)))
        {
            ctx.input_mut(|i| i.consume_key(modifiers, key));
            run(app, ctx, action.id);
        }
    }
    if !ctx.input(|i| i.key_pressed(Key::Escape)) {
        return;
    }
    let mut closed = true;
    if egui::Popup::is_any_open(ctx) {
        egui::Popup::close_all(ctx);
    } else if app.ui.palette.is_some() {
        crate::menus::set_palette(&mut app.ui, false);
    } else if app.ui.context_menu.take().is_some()
        || std::mem::take(&mut app.ui.main_menu)
        || app.compose.confirm.take().is_some()
        || close_agents(app)
        || std::mem::take(&mut app.compose.menu_open)
        || std::mem::take(&mut app.sync.menu_open)
        || crate::map_navigation::close_search(ctx)
    {
    } else if app.ui.preferences
        || app.ui.about
        || app.ui.shortcuts_panel
        || app.ui.export_preview
        || app.ui.confirm_close
        || app.ui.menu_input.is_some()
        || app.ui.find.is_some()
        || app.ui.color_profile.is_some()
        || app.ui.import_report.is_some()
        || app.ui.missing_fonts.is_some()
    {
        crate::dialogs::keys(app, ctx);
    } else if app.new_workspace.open {
        if app.new_workspace.busy.is_none() {
            app.new_workspace.open = false;
        }
    } else if app.machine_modal.open {
        app.machine_modal.open = false;
    } else if app.update.open {
        app.update.close();
    } else if app.new_page.open {
        if !app.new_page.busy {
            app.new_page.open = false;
        }
    } else if app.page_delete.take().is_some()
        // Esc in the tree's name field keeps the old name.
        || app.page_tree.renaming.take().is_some()
        || close_agents(app)
        || std::mem::take(&mut app.page_tree.open)
        || app.ui.key_box.take().is_some()
    {
    } else if app.compose.editing.is_some() {
        app.compose.stop_editing();
    } else if crate::canvas::cancel_map_interaction(app) {
    } else {
        closed = false;
    }
    if !closed && typing {
        app.ui.nav.go(crate::a11y::Region::Canvas);
    }
    if closed || typing {
        ctx.input_mut(|i| {
            i.consume_key(Modifiers::NONE, Key::Escape);
        });
    }
}

pub fn handle(app: &mut FigApp, ctx: &Context, typing: bool) {
    if overlay_open(app, ctx) {
        return;
    }
    let typing = typing || crate::a11y::field_focused(ctx);
    for event in ctx.input(|i| i.events.clone()) {
        match event {
            Event::Copy if !typing && app.ui.nav.region == crate::a11y::Region::Canvas => {
                run(app, ctx, "map.copy");
            }
            Event::Key { key, physical_key, pressed: true, modifiers, repeat } => {
                if !typing && app.ui.nav.region == crate::a11y::Region::Canvas && app.ui.key_box.is_some() {
                    if key == Key::Enter {
                        if let Some([x0, y0, x1, y1]) = app.ui.key_box.take()
                            && let (Ok(doc), Ok(page)) = (app.session.doc(), app.session.page())
                        {
                            let bounds = figcraft_geom::Rect::from_points((x0, y0), (x1, y1));
                            let selected = crate::map_navigation::box_selection(doc, page, bounds, &app.session.selection(), modifiers.shift);
                            app.session.set_selection(selected);
                        }
                        ctx.input_mut(|input| input.consume_key(modifiers, key));
                        continue;
                    }
                    if crate::shortcuts::key_box(app, key, modifiers) {
                        ctx.input_mut(|input| input.consume_key(modifiers, key));
                        continue;
                    }
                }
                let Some(action) = lookup(crate::shortcuts::platform(), key, physical_key, modifiers) else { continue };
                let global = global_action(action.id);
                let canvas = action.scope.starts_with("Canvas:") || action.scope.starts_with("Selection:");
                if (typing && !global)
                    || (canvas && app.ui.nav.region != crate::a11y::Region::Canvas)
                    || field_action(action.id)
                    || (repeat && !matches!(action.id, "map.zoomIn" | "map.zoomOut"))
                {
                    continue;
                }
                ctx.input_mut(|i| {
                    i.consume_key(modifiers, key);
                });
                run(app, ctx, action.id);
            }
            _ => {}
        }
    }
}

/// Esc closes the Agents modal; the composer and the Sync pane stop treating it as their menu.
fn close_agents(app: &mut FigApp) -> bool {
    let was = std::mem::take(&mut app.agents_modal.open);
    if was {
        app.compose.menu_open = false;
        app.sync.menu_open = false;
    }
    was
}
