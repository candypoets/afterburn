# Mindmap keyboard shortcuts

Mindmap's map-mode actions are defined in
`crates/ui-egui/src/map_action_catalog.rs`: 39 bound actions, three palette-only actions and four
pointer gestures. `map_actions.rs`, `menus.rs` and `shortcuts_panel.rs` use that catalog for input,
menus, the Actions palette and platform-specific Help. The full FigCraft editor keeps its own
keymap. Map input never falls through to the drawing editor's tool, mutation or document-history
shortcuts. The control channel's `ui.key` uses the same map vocabulary; `ui.input` exercises actual
focus and overlay routing.

## Focus and cancellation

Text fields own their typing, selection, clipboard and local undo/redo. In the composer, Enter
inserts a newline, Shift+Enter saves a draft and Cmd/Ctrl+Enter sends only while the composer owns
focus. External overlays disable the underlying composer. Escape closes the active overlay or
cancels an interaction before a later Escape clears canvas selection. Map pointer gestures select
cards, open their detail pages and pan; they never reach editor move/resize/text tools. A click
selects a card, or, once a box inside a card is selected, the box beside it at the same depth.
Double-click goes one box deeper (a card with a detail page opens it instead) and Cmd/Ctrl+click
selects the innermost box under the pointer. F8 and
pointer marquees include readable locked cards and cards inside sections. A cancelled pointer gesture
waits for button release before another pan or selection can begin. Drafts remain attached to
their recorded card/page.

F6 moves between workspace navigation, page navigation, the canvas and the composer; hidden
regions are skipped. Tab traverses content cards on the canvas and controls elsewhere. Focus
mode hides the workspace pane and page header; the composer and status bar remain available.

## Navigation and copying

`map_navigation.rs` implements read-only search over current-map card names/text, page-tree-order
navigation, source-card/detail-page links, card traversal and readable clipboard content with
page, node and source context. Cards are recognized from visible content containers/text and
existing detail-page metadata. Conventional page bounds, headings, dividers and hidden content
are skipped. This is a structural heuristic rather than a new mandatory card schema.

Copy selection as image uses the existing PNG renderer. Export asks explicitly for Page or
Selection and a format before opening the destination dialog; opening preview writes nothing.

## Workspace and persistence

`map_shell.rs` opens the existing workspace/new-workspace/worktree UI. Closing a map returns to
Workspaces without ending the live session or its agents. Composer states are parked per node
address/session (or local document identity) and restored when returning. The input's cursor and
Undo/Redo history are parked with it, and a disconnected node retains its document's identity.
This preserves drafts
and pending send state during in-app switching; it is memory-only and does not promise recovery
after app termination. Replies for parked composers are reconciled when their map is shown again.

Save sends `session.save` to the authoritative node session, reporting Saving, Saved or an error.
Its target is captured before the asynchronous request, so a subsequent host/workspace switch
cannot redirect it. Save does not stage, commit or start Sync. Open Sync review only opens the
existing review panel. Save and worktree switching require the desktop node connection.

## Verification

`map_shortcut_tests.rs` drives actual egui key events. `map_surfaces_tests.rs` checks catalog,
menu, palette and Help consistency. Module tests in `map_navigation.rs`, `map_shell.rs` and
`a11y.rs` cover navigation, save replies, parked composers and focus. Export tests cover explicit
scope without selection mutation. Existing editor tests continue to run with map mode disabled.
