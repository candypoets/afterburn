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
    action!("map.actions", "Actions palette", "Workspaces + app", "Cmd+K", "Ctrl+K", "App: search and run Mindmap actions."),
    action!("map.help", "Keyboard shortcuts", "Workspaces + app", "Ctrl+Shift+?", "Ctrl+Shift+?", "App: show shortcuts and their scope."),
    action!("map.search", "Search this map", "Workspaces + app", "Cmd+F", "Ctrl+F", "Map: find card text and names; open a result."),
    action!(
        "map.openWorkspace",
        "Open workspace / worktree",
        "Workspaces + app",
        "Cmd+O",
        "Ctrl+O",
        "Workspace: choose a project and worktree before opening."
    ),
    action!("map.newWorkspace", "New workspace", "Workspaces + app", "Cmd+N", "Ctrl+N", "Workspace: open workspace creation."),
    action!("map.save", "Save map", "Workspaces + app", "Cmd+S", "Ctrl+S", "Map: ask the node to persist; show saving, saved or error."),
    action!("map.close", "Close map", "Workspaces + app", "Cmd+W", "Ctrl+W", "Map: return to workspaces; keep agents and drafts alive."),
    action!("map.preferences", "Preferences", "Workspaces + app", "Cmd+,", "Ctrl+,", "App: open settings."),
    action!("map.focusMode", "Focus mode", "Workspaces + app", "Cmd+\\", "Ctrl+\\", "App: toggle workspace chrome around the canvas."),
    action!("map.fullscreen", "Full screen", "Workspaces + app", "Ctrl+Cmd+F", "F11", "App: toggle the native window's fullscreen state."),
    action!("map.pageTree", "Page tree", "Pages + navigation", "Shift+Cmd+P", "Ctrl+Shift+P", "Map: open the tree; search and choose a page."),
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
];

pub fn find(id: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|action| action.id == id)
}

impl Action {
    pub fn keys(&self, platform: figcraft_engine::keymap::Platform) -> &'static str {
        if platform == figcraft_engine::keymap::Platform::Mac { self.mac } else { self.other }
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
    ACTIONS
        .iter()
        .filter(|action| {
            [action.id, action.label, action.group, action.scope, action.keys(platform)].iter().any(|text| text.to_lowercase().contains(&query))
        })
        .collect()
}

/// Shared menu/palette availability. Field-owned actions remain in Help and in search results,
/// but cannot be activated by a surface that has taken focus away from their field.
pub fn enabled(app: &crate::FigApp, action: &Action) -> bool {
    if !action.palette_enabled() {
        return false;
    }
    match action.id {
        "map.actions" | "map.help" | "map.openWorkspace" | "map.newWorkspace" | "map.preferences" | "map.fullscreen" | "map.focusMode"
        | "map.workspacePane" | "map.nextRegion" | "map.previousRegion" => true,
        // Navigation resolves links (including subpages linked through their source card),
        // and safely declines an absent or ambiguous detail page.
        "map.copy" | "map.copyImage" | "map.zoomSelection" | "map.openDetail" => !app.ui.home && !app.session.selection().is_empty(),
        _ => !app.ui.home && app.session.ds().is_ok(),
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
