//! The map client's native menu bar (design: `Widget Menu bar` and `Menu bar states` on the
//! Mindmap page): the menus, their items, separators and shortcuts, without any platform code.
//! macOS builds it with AppKit (`apps/figcraft/src/mac_menu.rs`); this model is what tests check.
//!
//! Each item is a command id from the map catalog ([`crate::map_action_catalog`]), the registry
//! the keyboard shortcuts page, the in-app menu and the palette read: its label, shortcut and
//! enabled state come from there. The bar offers only the bindings a human can use in the map
//! (the shortcuts page's list); the field-only ones (typing undo/redo, the composer's keys) and
//! the drawing editor's bindings (tools, text edit, history) are not offered. The app's own
//! workspace actions get File entries; the app menu's Hide and Quit and the Window menu are
//! AppKit's.
//!
//! Key equivalents: only shortcuts with ⌘ become native key equivalents. AppKit takes those keys
//! before the window sees them, so they run once, through the menu; while a text field has the
//! keyboard the bar drops them and every key stays with egui, which runs the same actions itself
//! (and gives ⌘C, ⌘A to the field). Shortcuts without ⌘ (Escape, Tab, Enter, F6, PageUp, ⇧C…)
//! act on egui's focus and popups, so they stay with egui alone and the bar doesn't show them;
//! the keyboard shortcuts page lists them.

use figcraft_engine::keymap::{Platform, parse};

use crate::FigApp;
use crate::map_action_catalog::{self, Action};

/// File › Add machine…: the Workspaces sidebar's command, not a keyboard action.
pub const ADD_MACHINE: &str = "map.addMachine";
/// App menu › Check for Updates…; runs once the map catalog knows it, disabled until then.
pub const CHECK_FOR_UPDATES: &str = "app.checkForUpdates";
/// Help › Docs; disabled until the map catalog has a docs action.
pub const DOCS: &str = "help.docs";

/// What a native menu item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    /// A command id run through [`run`]; `label` is the menu's title.
    Item {
        id: &'static str,
        label: &'static str,
    },
    /// One of AppKit's own items.
    Native(Native),
    Separator,
}

/// AppKit's standard items: they act through its own selectors, never through the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Native {
    /// The standard About panel (the bundle's name, version and icon).
    About,
    Hide,
    Quit,
    Minimize,
    Zoom,
    BringAllToFront,
}

impl Native {
    /// The item's title, as the design writes it.
    pub fn label(self) -> &'static str {
        match self {
            Native::About => "About Mindmap",
            Native::Hide => "Hide Mindmap",
            Native::Quit => "Quit Mindmap",
            Native::Minimize => "Minimize",
            Native::Zoom => "Zoom",
            Native::BringAllToFront => "Bring All to Front",
        }
    }
}

/// The special role AppKit gives a menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The application menu (titled with the app's name by AppKit).
    App,
    Plain,
    /// `NSApp.windowsMenu`: AppKit lists the open windows under its items.
    Window,
    /// `NSApp.helpMenu`: AppKit adds its search field.
    Help,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Menu {
    pub title: &'static str,
    pub role: Role,
    pub entries: Vec<Entry>,
}

use Entry::Separator as Sep;

/// An item labelled as the catalog labels it (the in-app menu and the palette).
fn action(id: &'static str) -> Entry {
    Entry::Item { id, label: map_action_catalog::find(id).map_or(id, |a| a.label) }
}

/// An item with the design's own title.
fn titled(id: &'static str, label: &'static str) -> Entry {
    Entry::Item { id, label }
}

/// The bar, left to right, as the design lists it.
pub fn menus() -> Vec<Menu> {
    vec![
        Menu {
            title: "Mindmap",
            role: Role::App,
            entries: vec![
                Entry::Native(Native::About),
                titled(CHECK_FOR_UPDATES, "Check for Updates…"),
                Sep,
                titled("map.preferences", "Settings…"),
                Sep,
                Entry::Native(Native::Hide),
                Entry::Native(Native::Quit),
            ],
        },
        Menu {
            title: "File",
            role: Role::Plain,
            entries: vec![
                titled("map.newWorkspace", "New workspace…"),
                titled("map.openWorkspace", "Open workspace…"),
                titled("map.close", "Close map"),
                Sep,
                titled("map.syncReview", "Sync…"),
                titled(ADD_MACHINE, "Add machine…"),
                Sep,
                titled("map.save", "Save"),
                titled("map.export", "Export…"),
            ],
        },
        Menu {
            title: "Edit",
            role: Role::Plain,
            entries: vec![
                action("map.copy"),
                action("map.copyImage"),
                Sep,
                action("map.search"),
                Sep,
                action("map.selectAll"),
                action("map.selectNone"),
                Sep,
                action("map.openDetail"),
                action("map.parentPage"),
                action("map.nextCard"),
                action("map.previousCard"),
                action("map.selectArea"),
            ],
        },
        Menu {
            title: "View",
            role: Role::Plain,
            entries: vec![
                action("map.focusMode"),
                action("map.fullscreen"),
                Sep,
                action("map.comments"),
                action("map.cursors"),
                Sep,
                action("map.workspacePane"),
                Sep,
                action("map.nextRegion"),
                action("map.previousRegion"),
                action("map.contextMenu"),
                action("map.pageTree"),
                action("map.previousPage"),
                action("map.nextPage"),
                Sep,
                action("map.zoomIn"),
                action("map.zoomOut"),
                action("map.zoom100"),
                action("map.zoomFit"),
                action("map.zoomSelection"),
                Sep,
                titled("map.actions", "Actions…"),
            ],
        },
        Menu {
            title: "Window",
            role: Role::Window,
            entries: vec![Entry::Native(Native::Minimize), Entry::Native(Native::Zoom), Entry::Native(Native::BringAllToFront)],
        },
        Menu {
            title: "Help",
            role: Role::Help,
            entries: vec![titled("map.help", "Keyboard shortcuts"), titled(DOCS, "Docs"), Entry::Native(Native::About)],
        },
    ]
}

/// The catalog action behind an item id, if the catalog knows it.
fn catalog(id: &str) -> Option<&'static Action> {
    map_action_catalog::find(id)
}

/// The shortcut the native item carries as its key equivalent (`figcraft_engine::keymap` syntax,
/// macOS keys): the action's primary binding when it holds ⌘, else none (see the module notes).
pub fn key_equivalent(id: &str) -> Option<&'static str> {
    let keys = catalog(id)?.keys(Platform::Mac);
    parse(keys).filter(|(mods, _)| mods.cmd).map(|_| keys)
}

/// Whether the item can run now: the registry's answer ([`map_action_catalog::enabled`]); Add
/// machine always can; an id the catalog doesn't know (yet) can't.
pub fn enabled(app: &FigApp, id: &str) -> bool {
    if id == ADD_MACHINE {
        return true;
    }
    catalog(id).is_some_and(|action| map_action_catalog::enabled(app, action))
}

/// Run a clicked (or key-equivalent) item as the in-app menu does ([`crate::menus::invoke`]).
pub fn run(app: &mut FigApp, id: &str) {
    if id == ADD_MACHINE {
        // The modal draws on the Workspaces screen and in the workspace pane beside a map.
        if !app.ui.home {
            app.ws.pane_open = true;
        }
        crate::machine_modal::open(app);
        return;
    }
    if enabled(app, id) {
        crate::menus::invoke(app, id);
    }
}
