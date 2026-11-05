//! The map client's native window tabs on macOS (design: `Native window tabs`): each open project
//! is a window, and the windows form one tab group, so the system gives them its tab bar, ⌃Tab /
//! ⌃⇧Tab, Window ▸ Show Next Tab, Move Tab to New Window and Merge All Windows.
//!
//! egui creates each window already shown, too late for AppKit's automatic tabbing, so a window
//! seen for the first time is added to the group explicitly; one the human drags out of the group
//! later stays on its own. Each tab is titled `<workspace> · <branch>`; its window (in the title bar
//! above the tabs) `<workspace> — <branch>` (`figcraft_ui_egui::window_tabs`).
//!
//! A window showing a tab bar wears the native title bar: AppKit puts the title and the tab bar
//! above the content, the traffic lights in the title bar, and the content drops its own band for
//! them (`UiState::native_tab_bar`). Alone again, it gets the inset look back (`mac_chrome`).
//!
//! In full screen the tab bar would lie over the map's own header, so it hides and comes back only
//! while the pointer is at the top of the screen (with the menu bar), as a full-screen title bar
//! does; leaving full screen gives it back.
//!
//! Only safe AppKit calls (no `unsafe`). AppKit's `+` button in the tab bar needs a
//! `newWindowForTab:` responder, i.e. an Objective-C class of our own (`unsafe`): ⌘T opens a tab
//! instead.

use std::collections::HashSet;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSWindow, NSWindowOrderingMode, NSWindowStyleMask, NSWindowTabbingMode, NSWindowTitleVisibility};
use objc2_foundation::NSString;

/// The tab group every project window joins.
const TABBING_ID: &str = "dev.mindmap.client.project";

/// How many frames a window may take to appear before the group stops looking for it.
const PATIENCE: u32 = 20;

/// Full screen: the band at the top of the screen (points) that brings the hidden tab bar back.
const EDGE: f64 = 3.0;

/// Full screen: how far down (points) the revealed tab bar stays while the pointer is in it (the
/// menu bar, the title bar and the tab bar).
const REVEALED: f64 = 90.0;

/// What the windows look like after [`MacTabs::sync`].
pub struct Synced {
    /// The titles of the windows showing a tab bar.
    pub tabbed: Vec<String>,
    /// Every project window was found and grouped (else: look again shortly).
    pub settled: bool,
    /// A full-screen window's tab bar follows the pointer (look again shortly).
    pub watching: bool,
}

/// The windows already grouped and the ones wearing the native title bar, by window number.
#[derive(Default)]
pub struct MacTabs {
    joined: HashSet<isize>,
    native: HashSet<isize>,
    /// Full-screen windows whose tab bar this hid (shown again when they leave full screen).
    hidden: HashSet<isize>,
    waiting: u32,
}

/// The app's own windows (not panels or menus), the background tabs of a group included.
fn app_windows(mtm: MainThreadMarker) -> Vec<Retained<NSWindow>> {
    let app = NSApplication::sharedApplication(mtm);
    app.windows()
        .iter()
        .filter(|w| w.styleMask().contains(NSWindowStyleMask::Titled) && w.canBecomeMainWindow() && (w.isVisible() || w.tabbedWindows().is_some()))
        .collect()
}

impl MacTabs {
    /// Group the windows, title their tabs (`projects`: each window's title and its tab's) and give
    /// the tabbed ones the native title bar. Cheap enough for every frame: only changes touch AppKit.
    pub fn sync(&mut self, projects: &[(String, String)]) -> Synced {
        let Some(mtm) = MainThreadMarker::new() else { return Synced { tabbed: Vec::new(), settled: true, watching: false } };
        let windows = app_windows(mtm);
        let id = NSString::from_str(TABBING_ID);
        let mut anchor = NSApplication::sharedApplication(mtm)
            .keyWindow()
            .filter(|k| self.joined.contains(&k.windowNumber()))
            .or_else(|| windows.iter().find(|w| self.joined.contains(&w.windowNumber())).cloned());
        let mut tabbed = Vec::new();
        let mut watching = false;
        for w in &windows {
            let n = w.windowNumber();
            if w.tabbingIdentifier().to_string() != TABBING_ID {
                w.setTabbingIdentifier(&id);
                w.setTabbingMode(NSWindowTabbingMode::Preferred);
            }
            if !self.joined.contains(&n) {
                if let Some(a) = anchor.as_ref().filter(|a| a.windowNumber() != n)
                    && w.tabbedWindows().is_none()
                {
                    a.addTabbedWindow_ordered(w, NSWindowOrderingMode::Above);
                    w.makeKeyAndOrderFront(None);
                } else if anchor.is_none() {
                    // The first window: the others join its group.
                    anchor = Some(w.clone());
                }
                self.joined.insert(n);
            }
            let title = w.title().to_string();
            if let Some((_, tab)) = projects.iter().find(|(t, _)| *t == title) {
                let t = w.tab();
                if t.title().to_string() != *tab {
                    t.setTitle(Some(&NSString::from_str(tab)));
                }
            }
            let mask = w.styleMask();
            let bar = !mask.contains(NSWindowStyleMask::FullScreen) && w.tabGroup().is_some_and(|g| g.isTabBarVisible());
            if bar && !self.native.contains(&n) {
                native_chrome(w);
                self.native.insert(n);
            } else if !bar && self.native.contains(&n) && !mask.contains(NSWindowStyleMask::FullScreen) {
                inset_chrome(w);
                self.native.remove(&n);
            }
            if bar {
                tabbed.push(title);
            }
            watching |= self.full_screen_bar(w, mask.contains(NSWindowStyleMask::FullScreen));
        }
        let numbers: HashSet<isize> = windows.iter().map(|w| w.windowNumber()).collect();
        self.joined.retain(|n| numbers.contains(n));
        self.native.retain(|n| numbers.contains(n));
        self.hidden.retain(|n| numbers.contains(n));
        let titles: Vec<String> = windows.iter().map(|w| w.title().to_string()).collect();
        let found = projects.iter().all(|(t, _)| titles.contains(t));
        self.waiting = if found { 0 } else { self.waiting.saturating_add(1) };
        Synced { tabbed, settled: found || self.waiting > PATIENCE, watching }
    }

    /// Full screen: the tab bar shows only while the pointer is at the top of the key window's
    /// screen. Out of full screen, a bar this hid comes back. `true` while it follows the pointer.
    fn full_screen_bar(&mut self, w: &NSWindow, full: bool) -> bool {
        let n = w.windowNumber();
        let Some(group) = w.tabGroup() else { return false };
        let shown = group.isTabBarVisible();
        if !full {
            if self.hidden.remove(&n) && !shown && w.tabbedWindows().is_some_and(|t| t.count() > 1) {
                w.toggleTabBar(None);
            }
            return false;
        }
        if !w.isKeyWindow() || !w.tabbedWindows().is_some_and(|t| t.count() > 1) {
            return false;
        }
        let top = w.frame().size.height;
        let y = w.mouseLocationOutsideOfEventStream().y;
        let band = if shown { REVEALED } else { EDGE };
        let reveal = y.is_finite() && y >= top - band && y <= top + 1.0;
        if reveal != shown {
            w.toggleTabBar(None);
        }
        self.hidden.insert(n);
        true
    }
}

/// The title bar AppKit draws above a tab bar: the title shown, the traffic lights in it, the
/// content below it and the tabs.
fn native_chrome(w: &NSWindow) {
    w.setToolbar(None);
    w.setTitleVisibility(NSWindowTitleVisibility::Visible);
    w.setTitlebarAppearsTransparent(false);
    w.setStyleMask(w.styleMask() & !NSWindowStyleMask::FullSizeContentView);
}

/// The map client's own look again: the content under a transparent title bar (`mac_chrome` gives
/// it back its toolbar).
fn inset_chrome(w: &NSWindow) {
    w.setStyleMask(w.styleMask() | NSWindowStyleMask::FullSizeContentView);
    w.setTitlebarAppearsTransparent(true);
    w.setTitleVisibility(NSWindowTitleVisibility::Hidden);
}

/// ⇧⌘] / ⇧⌘[: the key window's group selects its next (`by` > 0) or previous tab, in the order
/// the human arranged them. `false` when the key window isn't tabbed.
pub fn step(by: i32) -> bool {
    let Some(mtm) = MainThreadMarker::new() else { return false };
    let Some(w) = NSApplication::sharedApplication(mtm).keyWindow() else { return false };
    if w.tabbedWindows().is_none() {
        return false;
    }
    if by >= 0 {
        w.selectNextTab(None);
    } else {
        w.selectPreviousTab(None);
    }
    true
}
