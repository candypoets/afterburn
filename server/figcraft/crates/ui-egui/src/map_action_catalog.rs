//! The map client's action vocabulary. Keyboard routing, menus, palette and Help share these IDs.
//! Bindings use `figcraft_engine::keymap::parse` syntax; an empty binding means palette-only.

#[derive(Debug)]
pub struct Action {
    pub id: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub mac: &'static str,
    pub other: &'static str,
    pub scope: &'static str,
}

macro_rules! action {
    ($id:literal, $label:literal, $group:literal, $mac:literal, $other:literal, $scope:literal) => {
        Action { id: $id, label: $label, group: $group, mac: $mac, other: $other, scope: $scope }
    };
}

pub const ACTIONS: &[Action] = &[
    action!("map.actions", "Actions palette", "Workspaces + app", "Shift+Cmd+P", "Ctrl+Shift+P", "App: search and run Mindmap actions."),
    action!("map.help", "Keyboard shortcuts", "Workspaces + app", "Ctrl+Shift+?", "Ctrl+Shift+?", "App: show shortcuts and their scope."),
    action!("map.search", "Search this map", "Workspaces + app", "Cmd+F", "Ctrl+F", "Map: find card text and names; open a result."),
    action!(
        "map.openWorkspace",
        "All workspaces",
        "Workspaces + app",
        "Shift+Cmd+O",
        "Ctrl+Shift+O",
        "Workspace: browse every workspace on every machine; Init the ones without a map."
    ),
    action!("map.newWorkspace", "New workspace", "Workspaces + app", "Cmd+N", "Ctrl+N", "Workspace: open workspace creation."),
    action!("map.save", "Save map", "Workspaces + app", "Cmd+S", "Ctrl+S", "Map: ask the node to persist; show saving, saved or error."),
    action!(
        "map.reconnect",
        "Reconnect to node",
        "Workspaces + app",
        "",
        "",
        "Map: retry the current node connection; keep comments and pending edits."
    ),
    action!(
        "map.closeTab",
        "Close tab",
        "Pages + navigation",
        "Cmd+W",
        "Ctrl+W",
        "Map: close the active page tab and open its neighbour; the page stays, and so does the last tab."
    ),
    action!("map.close", "Close map", "Workspaces + app", "Shift+Cmd+W", "Ctrl+Shift+W", "Map: return to workspaces; keep agents and drafts alive."),
    action!("map.preferences", "Preferences", "Workspaces + app", "Cmd+,", "Ctrl+,", "App: open settings."),
    action!("map.focusMode", "Focus mode", "Workspaces + app", "Cmd+\\", "Ctrl+\\", "App: toggle workspace chrome around the canvas."),
    action!("map.fullscreen", "Full screen", "Workspaces + app", "Ctrl+Cmd+F", "F11", "App: toggle the native window's fullscreen state."),
    action!("map.pageTree", "Page tree", "Pages + navigation", "Cmd+P", "Ctrl+P", "Map: show the page tree in the sidebar; choose a page."),
    action!("map.previousPage", "Previous page", "Pages + navigation", "PageUp", "PageUp", "Canvas: previous page in visible tree order."),
    action!("map.nextPage", "Next page", "Pages + navigation", "PageDown", "PageDown", "Canvas: next page in visible tree order."),
    action!("map.workspacePane", "Workspace pane", "Pages + navigation", "Shift+Cmd+\\", "Ctrl+Shift+\\", "App: show or hide workspace navigation."),
    action!("map.comments", "Comments", "Pages + navigation", "Shift+C", "Shift+C", "Canvas: show or hide comments on the map."),
    action!("map.cursors", "Collaborator cursors", "Pages + navigation", "Alt+Cmd+\\", "Ctrl+Alt+\\", "Map: show or hide other people's pointers."),
    action!("map.nextRegion", "Focus next region", "Pages + navigation", "F6", "F6", "App: workspace, page navigation, canvas, composer."),
    action!(
        "map.previousRegion",
        "Focus previous region",
        "Pages + navigation",
        "Shift+F6",
        "Shift+F6",
        "App: move through focus regions in reverse."
    ),
    action!(
        "map.contextMenu",
        "Selection actions",
        "Pages + navigation",
        "Shift+F10",
        "Shift+F10",
        "Canvas: open the selected card's Mindmap context menu."
    ),
    action!("map.zoomIn", "Zoom in", "Canvas + zoom", "Shift+Cmd+=", "Ctrl+Shift+=", "Canvas: increase zoom."),
    action!("map.zoomOut", "Zoom out", "Canvas + zoom", "Cmd+-", "Ctrl+-", "Canvas: decrease zoom."),
    action!("map.zoom100", "Actual size", "Canvas + zoom", "Cmd+0", "Ctrl+0", "Canvas: return to 100% zoom."),
    action!("map.zoomFit", "Fit page", "Canvas + zoom", "Shift+1", "Shift+1", "Canvas: fit the page content in view."),
    action!("map.zoomSelection", "Fit selection", "Canvas + zoom", "Shift+2", "Shift+2", "Selection: bring selected cards into view."),
    action!(
        "map.selectAll",
        "Select all cards",
        "Cards + selection",
        "Cmd+A",
        "Ctrl+A",
        "Canvas: select content cards on this page, excluding decoration."
    ),
    action!(
        "map.selectNone",
        "Select none",
        "Cards + selection",
        "Escape",
        "Escape",
        "Canvas: clear selection after the active popup or interaction; keep drafts."
    ),
    action!(
        "map.openDetail",
        "Open detail page",
        "Cards + selection",
        "Enter",
        "Enter",
        "Selection: open the selected card's subpage, when it has one."
    ),
    action!("map.parentPage", "Parent page", "Cards + selection", "Shift+Enter", "Shift+Enter", "Canvas: return from a detail page to its parent."),
    action!(
        "map.nextCard",
        "Next card",
        "Cards + selection",
        "Tab",
        "Tab",
        "Canvas: select the next visible content card; outside the canvas, Tab moves between controls."
    ),
    action!("map.previousCard", "Previous card", "Cards + selection", "Shift+Tab", "Shift+Tab", "Canvas: select the previous visible content card."),
    action!("map.selectArea", "Select an area", "Cards + selection", "F8", "F8", "Canvas: arrows move the box; Shift+arrows resize; Enter selects."),
    action!(
        "map.delete",
        "Delete",
        "Cards + selection",
        "Cmd+Backspace",
        "Ctrl+Backspace",
        "Selection: delete the selected cards, arrows and labels at once, with a card's detail page; ⌘Z brings them back."
    ),
    action!("map.undo", "Undo", "Cards + selection", "Cmd+Z", "Ctrl+Z", "Map: take back your own last edit (never an agent's)."),
    action!("map.redo", "Redo", "Cards + selection", "Shift+Cmd+Z", "Ctrl+Shift+Z", "Map: redo the edit you took back."),
    action!(
        "map.copy",
        "Copy card content",
        "Copying + text fields",
        "Cmd+C",
        "Ctrl+C",
        "Selection: copy readable card text with source context; text fields copy their own text."
    ),
    action!(
        "map.copyImage",
        "Copy selection as image",
        "Copying + text fields",
        "Shift+Cmd+C",
        "Ctrl+Shift+C",
        "Selection: copy a PNG of the current selection."
    ),
    action!(
        "map.export",
        "Export page or selection",
        "Copying + text fields",
        "Shift+Cmd+E",
        "Ctrl+Shift+E",
        "Page or selection: choose the export scope and format."
    ),
    action!(
        "map.undoTyping",
        "Undo typing",
        "Copying + text fields",
        "Cmd+Z",
        "Ctrl+Z",
        "Text field only: undo the focused field's local editing history."
    ),
    action!(
        "map.redoTyping",
        "Redo typing",
        "Copying + text fields",
        "Shift+Cmd+Z",
        "Ctrl+Shift+Z",
        "Text field only: redo the focused field's local editing history."
    ),
    action!("map.newline", "New line", "Comments + composer", "Enter", "Enter", "Composer only: keep writing in the composer."),
    action!(
        "map.saveDraft",
        "Save comment draft",
        "Comments + composer",
        "Shift+Enter",
        "Shift+Enter",
        "Composer only: attach this draft to the selected card or current page."
    ),
    action!(
        "map.send",
        "Send to agent",
        "Comments + composer",
        "Cmd+Enter",
        "Ctrl+Enter",
        "Composer only: send the prompt and included drafts to the selected agent."
    ),
    action!("map.switchWorktree", "Switch worktree", "Workspaces + app", "", "", "Workspace: choose another worktree from the actions palette."),
    action!("map.focusComposer", "Focus composer", "Comments + composer", "", "", "Composer: focus the prompt field without sending anything."),
    action!("map.syncReview", "Open Sync review", "Workspaces + app", "", "", "Workspace: review Sync; opening this view never starts Sync."),
    action!(
        "app.checkForUpdates",
        "Check for Updates…",
        "Workspaces + app",
        "",
        "",
        "App: look for a new Mindmap release; install the app and its node together."
    ),
    action!(
        "map.switchWorkspace",
        "Switch workspace",
        "Workspaces + app",
        "Ctrl+Cmd+O",
        "Ctrl+Alt+O",
        "Workspace: search the workspaces and worktrees of every machine; open one."
    ),
    action!(
        "map.workspace1",
        "Open workspace 1",
        "Workspaces + app",
        "Ctrl+Cmd+1",
        "Ctrl+Alt+1",
        "Workspace: open the first workspace of the Workspace menu."
    ),
    action!(
        "map.workspace2",
        "Open workspace 2",
        "Workspaces + app",
        "Ctrl+Cmd+2",
        "Ctrl+Alt+2",
        "Workspace: open the second workspace of the Workspace menu."
    ),
    action!(
        "map.workspace3",
        "Open workspace 3",
        "Workspaces + app",
        "Ctrl+Cmd+3",
        "Ctrl+Alt+3",
        "Workspace: open the third workspace of the Workspace menu."
    ),
    action!(
        "map.workspace4",
        "Open workspace 4",
        "Workspaces + app",
        "Ctrl+Cmd+4",
        "Ctrl+Alt+4",
        "Workspace: open the fourth workspace of the Workspace menu."
    ),
    action!(
        "map.workspace5",
        "Open workspace 5",
        "Workspaces + app",
        "Ctrl+Cmd+5",
        "Ctrl+Alt+5",
        "Workspace: open the fifth workspace of the Workspace menu."
    ),
    action!(
        "map.workspace6",
        "Open workspace 6",
        "Workspaces + app",
        "Ctrl+Cmd+6",
        "Ctrl+Alt+6",
        "Workspace: open the sixth workspace of the Workspace menu."
    ),
    action!(
        "map.workspace7",
        "Open workspace 7",
        "Workspaces + app",
        "Ctrl+Cmd+7",
        "Ctrl+Alt+7",
        "Workspace: open the seventh workspace of the Workspace menu."
    ),
    action!(
        "map.workspace8",
        "Open workspace 8",
        "Workspaces + app",
        "Ctrl+Cmd+8",
        "Ctrl+Alt+8",
        "Workspace: open the eighth workspace of the Workspace menu."
    ),
    action!(
        "map.workspace9",
        "Open workspace 9",
        "Workspaces + app",
        "Ctrl+Cmd+9",
        "Ctrl+Alt+9",
        "Workspace: open the ninth workspace of the Workspace menu."
    ),
];

/// Second bindings that run an action without being its primary shortcut: ⌘K, the palette's
/// earlier key, keeps opening it; ⌘O, once Open workspace, opens the workspace switcher. Help shows them after the primary binding.
pub const ALIASES: &[(&str, &str, &str)] = &[("map.actions", "Cmd+K", "Ctrl+K"), ("map.switchWorkspace", "Cmd+O", "Ctrl+O")];

pub fn find(id: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|action| action.id == id)
}

impl Action {
    pub fn keys(&self, platform: figcraft_engine::keymap::Platform) -> &'static str {
        if platform == figcraft_engine::keymap::Platform::Mac { self.mac } else { self.other }
    }

    /// This action's alias binding on `platform`, if it has one.
    pub fn alias(&self, platform: figcraft_engine::keymap::Platform) -> Option<&'static str> {
        ALIASES
            .iter()
            .find(|(id, ..)| *id == self.id)
            .map(|(_, mac, other)| if platform == figcraft_engine::keymap::Platform::Mac { *mac } else { *other })
            .filter(|keys| !keys.is_empty())
    }

    /// Every binding that runs this action on `platform`: the primary one first, then its alias.
    pub fn bindings(&self, platform: figcraft_engine::keymap::Platform) -> impl Iterator<Item = &'static str> {
        std::iter::once(self.keys(platform)).filter(|keys| !keys.is_empty()).chain(self.alias(platform))
    }

    /// Whether a key press runs this action (its primary binding or its alias).
    pub fn bound_to(&self, platform: figcraft_engine::keymap::Platform, key: &str, mods: figcraft_engine::keymap::Mods) -> bool {
        self.bindings(platform).any(|binding| figcraft_engine::keymap::matches(binding, platform, key, mods))
    }

    /// The bindings as Help writes them: "⇧⌘P or ⌘K"; "Actions palette" for palette-only actions.
    pub fn help_keys(&self, platform: figcraft_engine::keymap::Platform) -> String {
        let shown: Vec<String> = self.bindings(platform).map(|keys| figcraft_engine::keymap::display(keys, platform)).collect();
        if shown.is_empty() { "Actions palette".to_string() } else { shown.join(" or ") }
    }

    /// Text operations must stay with their owning field. A palette search must never submit a
    /// prompt, alter a draft or fall through to document undo/redo.
    pub fn palette_enabled(&self) -> bool {
        !matches!(self.id, "map.undoTyping" | "map.redoTyping" | "map.newline" | "map.saveDraft" | "map.send")
    }
}

/// Stable catalog order, searched by label, ID, group, binding and scope on the current platform.
pub fn search(query: &str, platform: figcraft_engine::keymap::Platform) -> Vec<&'static Action> {
    let query = query.trim().to_lowercase();
    ACTIONS.iter().filter(|action| rank(action, &query, platform).is_some()).collect()
}

/// The palette's search, best first (labels starting with the query, then a label word starting
/// with it, then labels containing it), catalog order within each rank. The ID, group, keys and
/// scope only answer a query that no label matches, so "work" lists the workspace actions, not
/// every action in "Workspaces + app".
pub fn ranked(query: &str, platform: figcraft_engine::keymap::Platform) -> Vec<&'static Action> {
    let query = query.trim().to_lowercase();
    let mut hits: Vec<_> = ACTIONS.iter().filter_map(|action| rank(action, &query, platform).map(|r| (r, action))).collect();
    if hits.iter().any(|(r, _)| *r < 3) {
        hits.retain(|(r, _)| *r < 3);
    }
    hits.sort_by_key(|(r, _)| *r);
    hits.into_iter().map(|(_, action)| action).collect()
}

/// How well `action` matches a lower-cased, trimmed query (lower is better); `None` when it
/// doesn't.
fn rank(action: &Action, query: &str, platform: figcraft_engine::keymap::Platform) -> Option<u8> {
    let label = action.label.to_lowercase();
    if label.starts_with(query) {
        return Some(0);
    }
    if label.split(|c: char| !c.is_alphanumeric()).any(|word| word.starts_with(query)) {
        return Some(1);
    }
    if label.contains(query) {
        return Some(2);
    }
    let alias = action.alias(platform).unwrap_or_default();
    [action.id, action.group, action.scope, action.keys(platform), alias].iter().any(|text| text.to_lowercase().contains(query)).then_some(3)
}

/// Shared menu/palette availability. Field-owned actions remain in Help and in search results,
/// but cannot be activated by a surface that has taken focus away from their field.
pub fn enabled(app: &crate::FigApp, action: &Action) -> bool {
    disabled_reason(app, action).is_none()
}

/// Why `action` cannot run now, as the palette says it beside the dimmed row; `None` when it can.
pub fn disabled_reason(app: &crate::FigApp, action: &Action) -> Option<&'static str> {
    if !action.palette_enabled() {
        return Some(if action.scope.starts_with("Composer only:") { "Only in the composer" } else { "Only in a text field" });
    }
    let no_map = app.ui.home || app.session.ds().is_err();
    match action.id {
        "map.actions"
        | "map.help"
        | "map.openWorkspace"
        | "map.newWorkspace"
        | "map.preferences"
        | "map.fullscreen"
        | "map.focusMode"
        | "map.workspacePane"
        | "map.switchWorkspace"
        | "map.nextRegion"
        | "map.previousRegion"
        | "app.checkForUpdates" => None,
        // Navigation resolves links (including subpages linked through their source card),
        // and safely declines an absent or ambiguous detail page.
        "map.copy" | "map.copyImage" | "map.zoomSelection" | "map.openDetail" | "map.delete" => {
            if no_map {
                Some("Open a map first")
            } else if app.session.selection().is_empty() {
                Some("Select a card first")
            } else {
                None
            }
        }
        id if workspace_number(id).is_some() => {
            workspace_number(id).and_then(|n| crate::switcher::nth(app, n)).is_none().then_some("No workspace listed there")
        }
        _ => no_map.then_some("Open a map first"),
    }
}

/// `n` for `map.workspace<n>` (⌃⌘1…⌃⌘9: the Workspace menu's `n`th workspace).
pub fn workspace_number(id: &str) -> Option<usize> {
    id.strip_prefix("map.workspace").and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=9).contains(n))
}

/// A toggle's current state as the palette shows it at the right of its row ("Shown"/"Hidden",
/// "On"/"Off"); `None` for actions that aren't toggles, or whose state needs an open map.
pub fn toggle_state(app: &crate::FigApp, action: &Action) -> Option<&'static str> {
    let shown = |on: bool| if on { "Shown" } else { "Hidden" };
    let on = |on: bool| if on { "On" } else { "Off" };
    let view = app.session.ds().ok().filter(|_| !app.ui.home).map(|ds| &ds.view);
    match action.id {
        "map.workspacePane" => Some(shown(app.ws.pane_open)),
        "map.fullscreen" => Some(on(app.ui.fullscreen)),
        "map.focusMode" => view.map(|v| on(v.ui_hidden)),
        "map.comments" => view.map(|v| shown(v.comments)),
        "map.cursors" => view.map(|v| shown(v.multiplayer_cursors)),
        _ => None,
    }
}

/// Pointer gestures are Help entries, not dispatchable actions or palette commands.
pub struct Gesture {
    pub label: &'static str,
    pub mac: &'static str,
    pub other: &'static str,
    pub scope: &'static str,
}

pub const GESTURES: &[Gesture] = &[
    Gesture { label: "Pan canvas", mac: "Space + drag", other: "Space + drag", scope: "Canvas: move the view without moving cards." },
    Gesture {
        label: "Zoom around pointer",
        mac: "Cmd + scroll",
        other: "Ctrl + scroll",
        scope: "Canvas: zoom around the pointer; trackpad pinch works here too.",
    },
    Gesture {
        label: "Select a box inside a card",
        mac: "Double-click",
        other: "Double-click",
        scope: "Canvas: one box deeper each time; a card with a detail page opens it instead.",
    },
    Gesture { label: "Select the innermost box", mac: "Cmd + click", other: "Ctrl + click", scope: "Canvas: the deepest box under the pointer." },
];
