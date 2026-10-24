//! The map client's macOS window chrome: an empty unified toolbar on each window.
//!
//! With a full-size content view and a hidden title bar, AppKit draws the traffic lights tight in
//! the corner. A unified toolbar moves them to where native apps have them (20 pt from the left,
//! centred in a 52 pt band); the content reserves that band (`figcraft_ui_egui::MAC_BAND`).

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSToolbar, NSWindowToolbarStyle};

/// Give every window that has none the empty unified toolbar. Cheap enough to call each frame:
/// windows that already have one are left alone, and new windows get theirs on the next frame.
pub fn unify_toolbars() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    for window in app.windows().iter() {
        if window.toolbar().is_some() {
            continue;
        }
        let toolbar = NSToolbar::new(mtm);
        toolbar.setShowsBaselineSeparator(false);
        window.setToolbar(Some(&toolbar));
        window.setToolbarStyle(NSWindowToolbarStyle::Unified);
    }
}
