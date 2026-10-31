//! The map client's native macOS menu bar (design: `Widget Menu bar`), built once at startup from
//! [`figcraft_ui_egui::native_menu`], which holds the menus, items, ids and shortcuts (and their
//! tests). Replaces winit's default bar.
//!
//! A click (or a ⌘ key equivalent: AppKit takes those keys before the window sees them) queues the
//! item's command id; [`MacMenu::poll`] drains the queue each frame and runs it as the in-app menu
//! does. While a text field has the keyboard the key equivalents come off, so every key reaches
//! egui instead: one path per key, never both. Enabled state follows the registry each frame
//! (only changed items are touched).
//!
//! The Workspace menu's listed workspaces and its Worktree submenu follow the workspace listing:
//! each frame [`MacMenu::poll`] asks the model for them and rebuilds those items only when they
//! changed (a check moving is set in place).
//!
//! NSMenu items need an Objective-C target class (`unsafe`, which the workspace forbids); muda
//! wraps that behind a safe API, as `fmv-macos-events` does for Apple events.

use std::sync::{Arc, Mutex, PoisonError};

use figcraft_ui_egui::FigApp;
use figcraft_ui_egui::native_menu::{self, DynItem, Entry, Native, Role};
use muda::accelerator::Accelerator;
use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

struct Item {
    item: MenuItem,
    id: &'static str,
    keys: Option<Accelerator>,
    enabled: bool,
}

/// A listed workspace or worktree, as last built.
struct Dyn {
    item: CheckMenuItem,
    data: DynItem,
    keys: Option<Accelerator>,
}

/// Items rebuilt from the listing: where they go and what is there now.
struct Slot {
    menu: Submenu,
    /// The position of the first item in `menu`.
    at: usize,
    shown: Vec<Dyn>,
}

impl Slot {
    /// Bring the slot's items up to `want`: rebuilt when the list changed, else checks set in place.
    fn sync(&mut self, want: Vec<DynItem>, typing: bool) {
        if self.shown.len() == want.len() && self.shown.iter().zip(&want).all(|(d, w)| d.data == *w) {
            // AppKit flips a check on click; put it back where the listing says.
            for d in &self.shown {
                if d.item.is_checked() != d.data.checked {
                    d.item.set_checked(d.data.checked);
                }
            }
            return;
        }
        for d in self.shown.drain(..) {
            if let Err(e) = self.menu.remove(&d.item) {
                log::warn!("menu bar: {e}");
            }
        }
        for (i, data) in want.into_iter().enumerate() {
            let keys = data.keys.and_then(|k| k.parse::<Accelerator>().ok());
            let item = CheckMenuItem::with_id(data.id.clone(), &data.label, data.enabled, data.checked, if typing { None } else { keys });
            if let Err(e) = self.menu.insert(&item, self.at + i) {
                log::warn!("menu bar: {e}");
            }
            self.shown.push(Dyn { item, data, keys });
        }
    }

    /// Key equivalents off while a text field has the keyboard, back on after.
    fn set_typing(&self, typing: bool) {
        for d in &self.shown {
            if let Some(keys) = d.keys
                && let Err(e) = d.item.set_accelerator((!typing).then_some(keys))
            {
                log::warn!("menu bar: {e}");
            }
        }
    }
}

/// The installed bar; hold it for the app's life.
pub struct MacMenu {
    _bar: Menu,
    items: Vec<Item>,
    workspaces: Option<Slot>,
    worktrees: Option<Slot>,
    typing: bool,
    clicked: Arc<Mutex<Vec<String>>>,
}

fn native(n: Native) -> PredefinedMenuItem {
    let label = Some(n.label());
    match n {
        Native::About => PredefinedMenuItem::about(label, None),
        Native::Hide => PredefinedMenuItem::hide(label),
        Native::Quit => PredefinedMenuItem::quit(label),
        Native::Minimize => PredefinedMenuItem::minimize(label),
        Native::Zoom => PredefinedMenuItem::zoom(label),
        Native::BringAllToFront => PredefinedMenuItem::bring_all_to_front(label),
    }
}

impl MacMenu {
    /// Build the bar and make it the app's main menu. Call on the main thread after launch (the
    /// app creator), so it replaces winit's default bar; clicks wake `ctx`.
    pub fn install(ctx: &egui::Context) -> Self {
        let bar = Menu::new();
        let mut items = Vec::new();
        let mut windows = None;
        let mut help = None;
        let mut workspaces = None;
        let mut worktrees = None;
        for menu in native_menu::menus() {
            let submenu = Submenu::new(menu.title, true);
            // How many items `submenu` holds so far: where a listed slot starts.
            let mut count = 0;
            for entry in menu.entries {
                let added = match entry {
                    Entry::Item { id, label } => {
                        let keys = native_menu::key_equivalent(id).and_then(|k| k.parse::<Accelerator>().ok());
                        // Disabled until the first poll reads the registry.
                        let item = MenuItem::with_id(id, label, false, keys);
                        let added = submenu.append(&item);
                        items.push(Item { item, id, keys, enabled: false });
                        added
                    }
                    Entry::Native(n) => submenu.append(&native(n)),
                    Entry::Separator => submenu.append(&PredefinedMenuItem::separator()),
                    // Filled by the first poll; nothing takes a place in the menu until then.
                    Entry::Workspaces => {
                        workspaces = Some(Slot { menu: submenu.clone(), at: count, shown: Vec::new() });
                        continue;
                    }
                    Entry::Worktrees => {
                        let sub = Submenu::new("Worktree", true);
                        worktrees = Some(Slot { menu: sub.clone(), at: 0, shown: Vec::new() });
                        submenu.append(&sub)
                    }
                };
                match added {
                    Ok(()) => count += 1,
                    Err(e) => log::warn!("menu bar: {e}"),
                }
            }
            if let Err(e) = bar.append(&submenu) {
                log::warn!("menu bar: {e}");
            }
            match menu.role {
                Role::Window => windows = Some(submenu),
                Role::Help => help = Some(submenu),
                Role::App | Role::Plain => {}
            }
        }
        bar.init_for_nsapp();
        // After the bar is installed: muda finds these menus through it.
        if let Some(menu) = windows {
            menu.set_as_windows_menu_for_nsapp();
        }
        if let Some(menu) = help {
            menu.set_as_help_menu_for_nsapp();
        }
        let clicked: Arc<Mutex<Vec<String>>> = Arc::default();
        let queue = Arc::clone(&clicked);
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            queue.lock().unwrap_or_else(PoisonError::into_inner).push(event.id().0.clone());
            ctx.request_repaint();
        }));
        Self { _bar: bar, items, workspaces, worktrees, typing: false, clicked }
    }

    /// Run the items chosen since the last frame, then bring the listed workspaces, enabled state
    /// and key equivalents up to date.
    pub fn poll(&mut self, fig: &mut FigApp, ctx: &egui::Context) {
        let clicked = std::mem::take(&mut *self.clicked.lock().unwrap_or_else(PoisonError::into_inner));
        for id in clicked {
            native_menu::run(fig, &id);
        }
        for item in &mut self.items {
            let on = native_menu::enabled(fig, item.id);
            if on != item.enabled {
                item.item.set_enabled(on);
                item.enabled = on;
            }
        }
        let typing = ctx.egui_wants_keyboard_input();
        if let Some(slot) = &mut self.workspaces {
            slot.sync(native_menu::workspace_items(fig), self.typing);
        }
        if let Some(slot) = &mut self.worktrees {
            slot.sync(native_menu::worktree_items(fig), self.typing);
        }
        if typing != self.typing {
            self.typing = typing;
            for item in &self.items {
                if let Some(keys) = item.keys
                    && let Err(e) = item.item.set_accelerator((!typing).then_some(keys))
                {
                    log::warn!("menu bar: {e}");
                }
            }
            for slot in [&self.workspaces, &self.worktrees].into_iter().flatten() {
                slot.set_typing(typing);
            }
        }
    }
}
