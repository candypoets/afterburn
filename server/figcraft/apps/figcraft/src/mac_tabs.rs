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
//! In full screen the title bar and tabs would lie over the map's own header, so the content takes
//! the whole screen and they show only while the pointer is at the top of the screen.
//!
//! Only safe AppKit calls (no `unsafe`). AppKit's `+` button in the tab bar needs a
//! `newWindowForTab:` responder, i.e. an Objective-C class of our own (`unsafe`): ⌘T opens a tab
//! instead.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSApplication, NSWindow, NSWindowOrderingMode, NSWindowStyleMask, NSWindowTabbingMode,
    NSWindowTitleVisibility,
};
use objc2_foundation::NSString;

/// The tab group every project window joins.
const TABBING_ID: &str = "dev.mindmap.client.project";

/// Full screen: the band at the top of the screen (points) that brings the hidden tab bar back.
const EDGE: f64 = 4.0;

/// Full screen: how far down (points) the shown title bar and tabs stay while the pointer is in
/// them.
const REVEALED: f64 = 80.0;

/// Full screen: how long (seconds) the title bar and tabs take to fade in, and out.
const FADE_IN: f64 = 0.16;
const FADE_OUT: f64 = 0.24;

/// Full screen: after this, a fade is over (a band AppKit showed again meanwhile is hidden again).
const FADE_SETTLED: Duration = Duration::from_millis(400);

/// How many frames a window may take to appear before the group stops looking for it.
const PATIENCE: u32 = 20;

/// What the windows look like after [`MacTabs::sync`].
pub struct Synced {
    /// The titles of the windows showing a tab bar.
    pub tabbed: Vec<String>,
    /// Every project window was found and grouped (else: look again shortly).
    pub settled: bool,
    /// Full screen: the title bar and tabs follow the pointer (look again shortly).
    pub watching: bool,
}

/// The windows already grouped and the ones wearing the native title bar, by window number.
#[derive(Default)]
pub struct MacTabs {
    joined: HashSet<isize>,
    native: HashSet<isize>,
    waiting: u32,
    /// Full-screen windows given a full-size content view here (the native title bar takes it back
    /// when they leave full screen).
    full_inset: HashSet<isize>,
    /// Full screen: the title bar and tabs are see-through until the pointer reaches the top.
    band_hidden: bool,
    /// When the title bar and tabs last started to fade.
    faded: Option<Instant>,
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
        }
        let watching = self.full_screen_bar(mtm, &windows);
        let numbers: HashSet<isize> = windows.iter().map(|w| w.windowNumber()).collect();
        self.joined.retain(|n| numbers.contains(n));
        self.native.retain(|n| numbers.contains(n));
        self.full_inset.retain(|n| numbers.contains(n));
        let titles: Vec<String> = windows.iter().map(|w| w.title().to_string()).collect();
        let found = projects.iter().all(|(t, _)| titles.contains(t));
        self.waiting = if found { 0 } else { self.waiting.saturating_add(1) };
        Synced { tabbed, settled: found || self.waiting > PATIENCE, watching }
    }

    /// Full screen with several tabs: the content reaches the top of the screen and the band AppKit
    /// draws above it (the title bar and the tab bar, a window of its own) shows only while the
    /// pointer is at the top. AppKit refuses to hide a tab bar holding several tabs, and the window
    /// delegate's auto-hiding options need `unsafe`, so the band is made see-through and lets
    /// clicks pass instead. Out of full screen everything is as it was. `true` while the band
    /// follows the pointer.
    fn full_screen_bar(&mut self, mtm: MainThreadMarker, windows: &[Retained<NSWindow>]) -> bool {
        let key = windows
            .iter()
            .find(|w| w.isKeyWindow() && w.styleMask().contains(NSWindowStyleMask::FullScreen) && w.tabbedWindows().is_some_and(|t| t.count() > 1));
        // Back out of full screen: the native title bar keeps the content below it.
        for w in windows {
            let n = w.windowNumber();
            if !w.styleMask().contains(NSWindowStyleMask::FullScreen) && self.full_inset.remove(&n) && self.native.contains(&n) {
                native_chrome(w);
            }
        }
        let bands: Vec<Retained<NSWindow>> = NSApplication::sharedApplication(mtm)
            .windows()
            .iter()
            .filter(|w| {
                let class = w.class().name().to_string_lossy();
                class.contains("FullScreen") && (class.contains("Toolbar") || class.contains("Titlebar"))
            })
            .collect();
        let Some(w) = key else {
            if std::mem::take(&mut self.band_hidden) {
                for band in &bands {
                    fade(band, true);
                }
            }
            return false;
        };
        let n = w.windowNumber();
        if !w.styleMask().contains(NSWindowStyleMask::FullSizeContentView) {
            let mask = w.styleMask() | NSWindowStyleMask::FullSizeContentView;
            if objc2::exception::catch(std::panic::AssertUnwindSafe(|| w.setStyleMask(mask))).is_ok() {
                self.full_inset.insert(n);
            }
        }
        let top = w.frame().size.height;
        let y = w.mouseLocationOutsideOfEventStream().y;
        let band = if self.band_hidden { EDGE } else { REVEALED };
        let reveal = y.is_finite() && y >= top - band;
        // AppKit may show the band again on its own: once a fade is over, put it back.
        let settled = self.faded.is_none_or(|at| at.elapsed() > FADE_SETTLED);
        if reveal == self.band_hidden || (settled && bands.iter().any(|b| (b.alphaValue() > 0.5) != reveal)) {
            for b in &bands {
                fade(b, reveal);
            }
            self.band_hidden = !reveal;
            self.faded = Some(Instant::now());
        }
        if std::env::var_os("MINDMAP_DEBUG_TABS").is_some() {
            let names: Vec<String> = bands.iter().map(|b| b.class().name().to_string_lossy().into_owned()).collect();
            eprintln!("mac_tabs: y={y} top={top} reveal={reveal} bands={names:?}");
        }
        true
    }
}

/// Full screen: fade the title bar and tabs in (`show`) or out; hidden, clicks pass through them.
fn fade(band: &NSWindow, show: bool) {
    band.setIgnoresMouseEvents(!show);
    NSAnimationContext::beginGrouping();
    NSAnimationContext::currentContext().setDuration(if show { FADE_IN } else { FADE_OUT });
    band.animator().setAlphaValue(if show { 1.0 } else { 0.0 });
    NSAnimationContext::endGrouping();
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
