//! Map shell actions shared by menus and shortcuts.
//!
//! Integration: store `State::default()` in `FigApp::map_shell`, dispatch `run`, and call
//! `poll` after live/workspace polling, before drawing the composer (including on Home).
//! The parent owns chrome visibility using `session.ds().view.ui_hidden`.
//!
//! Close means navigation, not file.close/session.leave: documents and agents stay alive.
//! Composer states are parked by node address/session (document uid for non-node maps).
//! They are memory-only; an inactive composer's send reply is reconciled when it returns.

use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::FigApp;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ComposerKey {
    #[cfg(not(target_arch = "wasm32"))]
    Node(String, String),
    Document(u64),
}

/// Last explicit node-save result. The label identifies the saved session even after a switch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SaveStatus {
    #[default]
    Idle,
    Saving(String),
    Saved(String),
    Error(String),
}

struct PendingSave {
    label: String,
    rx: Receiver<Result<(), String>>,
}

/// egui stores the input's cursor and undo stack outside compose::State. Park both together.
#[derive(Default)]
struct ParkedComposer {
    compose: crate::compose::State,
    text_edit: egui::text_edit::TextEditState,
}

/// Shell state; no document snapshots or file-dialog state are kept here.
#[derive(Default)]
pub struct State {
    pub save: SaveStatus,
    pending_save: Option<PendingSave>,
    composer: Option<ComposerKey>,
    composers: BTreeMap<ComposerKey, ParkedComposer>,
    /// A failed live connection clears SessionInfo, but the document still owns its drafts.
    #[cfg(not(target_arch = "wasm32"))]
    node_keys: BTreeMap<u64, ComposerKey>,
}

fn composer_key(app: &mut FigApp) -> Option<ComposerKey> {
    let uid = app.session.ds().ok()?.uid;
    // Retain the node identity during a disconnect; connectivity must not move its drafts.
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(target) = app.session.live().info.as_ref().and_then(|info| crate::node::SessionRef::parse(&info.link).ok()) {
            let key = ComposerKey::Node(target.address, target.session);
            app.map_shell.node_keys.insert(uid, key.clone());
            return Some(key);
        }
        if let Some(key) = app.map_shell.node_keys.get(&uid) {
            return Some(key.clone());
        }
    }
    Some(ComposerKey::Document(uid))
}

fn switch_composer(app: &mut FigApp, ctx: &egui::Context) {
    let next = composer_key(app);
    if app.map_shell.composer == next {
        return;
    }
    let input_id = crate::compose::input_id();
    if let Some(previous) = app.map_shell.composer.take() {
        let parked =
            ParkedComposer { compose: std::mem::take(&mut app.compose), text_edit: egui::TextEdit::load_state(ctx, input_id).unwrap_or_default() };
        app.map_shell.composers.insert(previous, parked);
        let saved = next.as_ref().and_then(|key| app.map_shell.composers.remove(key)).unwrap_or_default();
        app.compose = saved.compose;
        // Replace the context's whole state, not its shared undoer in place: a cloned
        // TextEditState shares its undoer through an Arc with the parked copy.
        saved.text_edit.store(ctx, input_id);
    } else if let Some(key) = next.as_ref()
        && let Some(saved) = app.map_shell.composers.remove(key)
    {
        app.compose = saved.compose;
        saved.text_edit.store(ctx, input_id);
    }
    // The composer normally clears input when selection changes. Returning to a parked map
    // must preserve its input even if a remote update changed its selection while away.
    app.compose.last_selection = app.session.selection();
    app.compose.last_page = app.session.page().ok();
    app.map_shell.composer = next;
}

/// Poll on every frame, including Workspaces, after workspace joins and before compose::show.
pub fn poll(app: &mut FigApp, ctx: &egui::Context) {
    switch_composer(app, ctx);
    let Some(pending) = app.map_shell.pending_save.as_ref() else { return };
    let result = match pending.rx.try_recv() {
        Ok(result) => result,
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => Err("The save worker stopped before reporting a result; the save outcome is unknown.".into()),
    };
    let label = pending.label.clone();
    app.map_shell.pending_save = None;
    match result {
        Ok(()) => {
            app.map_shell.save = SaveStatus::Saved(label.clone());
            app.toast(format!("Saved {label}."));
        }
        Err(error) => save_error(app, format!("{label}: {error}")),
    }
    ctx.request_repaint();
}

/// True means this module recognizes the id, even if the action is currently unavailable.
/// The caller decides whether map-mode bindings are enabled; tests need no global map mode.
pub fn run(app: &mut FigApp, ctx: &egui::Context, id: &str) -> bool {
    if !matches!(
        id,
        "map.openWorkspace"
            | "map.newWorkspace"
            | "map.save"
            | "map.reconnect"
            | "map.close"
            | "map.switchWorktree"
            | "map.focusComposer"
            | "map.syncReview"
            | "map.focusMode"
    ) {
        return false;
    }
    switch_composer(app, ctx);
    match id {
        "map.openWorkspace" | "map.close" => workspace_home(app),
        "map.newWorkspace" => {
            // This dialog is drawn by the existing Workspaces screen/pane.
            workspace_home(app);
            if !app.new_workspace.open {
                crate::new_workspace::open(app);
            }
        }
        "map.save" => save(app, ctx),
        "map.reconnect" => {
            #[cfg(not(target_arch = "wasm32"))]
            crate::workspaces::reconnect(app);
            #[cfg(target_arch = "wasm32")]
            app.toast("Node connections require the desktop app.");
        }
        "map.switchWorktree" => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                reveal_chrome(app);
                crate::switcher::open_worktrees(app);
            }
            #[cfg(target_arch = "wasm32")]
            app.toast("Worktree switching requires a desktop node connection.");
        }
        "map.focusComposer" if map_visible(app) => {
            reveal_chrome(app);
            app.compose.focus_input = true;
        }
        "map.syncReview" if map_visible(app) => {
            reveal_chrome(app);
            // Reads the existing review diff; never dispatches sync/start/send.
            crate::sync::open_panel(app, ctx);
        }
        "map.focusMode" if map_visible(app) => {
            app.run("view.toggleUi", serde_json::json!({}));
        }
        _ => app.toast("Open a workspace map first."),
    }
    ctx.request_repaint();
    true
}

fn map_visible(app: &FigApp) -> bool {
    !app.ui.home && app.session.ds().is_ok()
}

fn reveal_chrome(app: &mut FigApp) {
    if let Ok(ds) = app.session.ds_mut() {
        ds.view.ui_hidden = false;
    }
}

fn workspace_home(app: &mut FigApp) {
    // Stop a pending UI transition from taking us straight back to a map. This only drops the
    // waiting UI request; it does not stop an agent or leave a live engine session.
    app.ws.cancel_open();
    app.status.switching = None;
    crate::switcher::close(app);
    app.ws.worktree_init = false;
    app.ui.home = true;
}

fn save_error(app: &mut FigApp, reason: String) {
    let message = format!("Could not save map: {reason}");
    app.map_shell.save = SaveStatus::Error(message.clone());
    app.ui.last_error = Some(message.clone());
    app.toast(message);
}

#[cfg(not(target_arch = "wasm32"))]
fn save_request(target: &crate::node::SessionRef) -> serde_json::Value {
    serde_json::json!({"method": "session.save", "session": target.session})
}

#[cfg(not(target_arch = "wasm32"))]
fn saved_reply(reply: serde_json::Value) -> Result<(), String> {
    if reply.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err(reply.get("error").and_then(serde_json::Value::as_str).unwrap_or("The node did not confirm the save.").to_string())
    }
}

fn save(app: &mut FigApp, ctx: &egui::Context) {
    if let Some(pending) = app.map_shell.pending_save.as_ref() {
        app.toast(format!("Saving {}…", pending.label));
        return;
    }
    if !map_visible(app) {
        save_error(app, "Open a workspace map first.".into());
        return;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Resolve once, before spawning: changing the host picker or active map cannot reroute
        // the request. Never use FigApp::save, file.save, or the selected workspace as fallback.
        let target = match crate::workspaces::active_node(app) {
            Ok(target) => target,
            Err(error) => {
                save_error(app, error);
                return;
            }
        };
        let request = save_request(&target);
        let label = format!("{} ({})", if target.workspace.is_empty() { &target.session } else { &target.workspace }, target.address);
        let (tx, rx) = std::sync::mpsc::channel();
        let wake = ctx.clone();
        let spawned = std::thread::Builder::new().name("mindmap-save".into()).spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::node::rpc(&target.address, request, std::time::Duration::from_secs(30)).and_then(saved_reply)
            }))
            .unwrap_or_else(|_| Err("The save request failed unexpectedly; its outcome is unknown.".into()));
            let _ = tx.send(result);
            wake.request_repaint();
        });
        match spawned {
            Ok(_) => {
                app.map_shell.save = SaveStatus::Saving(label.clone());
                app.toast(format!("Saving {label}…"));
                app.map_shell.pending_save = Some(PendingSave { label, rx });
            }
            Err(error) => save_error(app, format!("Could not start the save worker: {error}")),
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = ctx;
        save_error(app, "Saving a node map requires the desktop app.".into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app() -> FigApp {
        FigApp::new(figcraft_engine::Session::new(), Box::new(crate::NoServices))
    }

    fn draft(text: &str) -> crate::compose::Draft {
        crate::compose::Draft { text: text.into(), node: None, page: None, node_id: None, page_id: None, group: None }
    }

    #[test]
    fn workspace_navigation_keeps_document_input_and_drafts() {
        let mut app = app();
        let ctx = egui::Context::default();
        let uid = app.session.ds().unwrap().uid;
        let revision = app.session.ds().unwrap().revision;
        app.compose.input = "unfinished edit".into();
        app.compose.drafts.push(draft("keep me"));
        app.compose.editing = Some(0);
        app.active_workspace = Some("agent-workspace".into());
        for id in ["map.close", "map.openWorkspace"] {
            app.ui.home = false;
            assert!(run(&mut app, &ctx, id));
            assert!(app.ui.home);
            assert_eq!(app.session.ds().unwrap().uid, uid);
            assert_eq!(app.session.ds().unwrap().revision, revision);
            assert_eq!(app.compose.input, "unfinished edit");
            assert_eq!(app.compose.drafts, [draft("keep me")]);
            assert_eq!(app.compose.editing, Some(0));
            assert_eq!(app.active_workspace.as_deref(), Some("agent-workspace"));
        }
    }

    #[test]
    fn switching_maps_parks_and_restores_the_entire_composer() {
        let mut app = app();
        let ctx = egui::Context::default();
        let first = app.session.active;
        app.compose.input = "first input".into();
        app.compose.drafts.push(draft("first draft"));
        app.compose.editing = Some(0);
        app.compose.sending.push(draft("pending send"));
        app.compose.busy = true;
        let (tx, rx) = std::sync::mpsc::channel();
        app.compose.rx = Some(rx);
        poll(&mut app, &ctx);
        app.session.execute("file.new", &json!({"name": "Second"})).unwrap();
        let second = app.session.active;
        poll(&mut app, &ctx);
        assert!(app.compose.input.is_empty());
        assert!(app.compose.drafts.is_empty());
        assert!(!app.compose.busy);
        assert!(app.compose.rx.is_none());
        app.compose.input = "second input".into();
        app.compose.drafts.push(draft("second draft"));
        tx.send(Err("synthetic send failure".into())).unwrap();
        app.session.active = first;
        poll(&mut app, &ctx);
        assert_eq!(app.compose.input, "first input");
        assert_eq!(app.compose.drafts, [draft("first draft")]);
        assert_eq!(app.compose.editing, Some(0));
        assert_eq!(app.compose.sending, [draft("pending send")]);
        assert!(app.compose.busy);
        assert_eq!(app.compose.rx.as_ref().unwrap().try_recv().unwrap().unwrap_err(), "synthetic send failure");
        assert_eq!(app.compose.last_page, app.session.page().ok());
        app.session.active = second;
        poll(&mut app, &ctx);
        assert_eq!(app.compose.input, "second input");
        assert_eq!(app.compose.drafts, [draft("second draft")]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn failed_node_connection_keeps_offline_edits_when_the_workspace_reopens() {
        use figcraft_engine::live::{Event, Status};

        // This harness's link only queues synthetic events; no socket or real node is used.
        let (mut h, _) = crate::live_tests::Harness::new();
        let node = crate::node::SessionRef { address: "synthetic:1".into(), session: "session-a".into(), ..Default::default() };
        h.joined();
        h.app.session.live_poll();
        h.app.session.live_mut().unwrap().info.as_mut().unwrap().link = node.route();
        let original_uid = h.app.session.ds().unwrap().uid;
        h.app.compose.input = "online input".into();
        h.app.compose.drafts.push(draft("online draft"));
        poll(&mut h.app, &h.ctx);

        // Exercise the engine's real failure handler: it clears SessionInfo and doc_uid.
        h.send(vec![Event::Status(Status::Failed { reason: "synthetic disconnect".into() })]);
        h.app.session.live_poll();
        assert!(h.app.session.live().info.is_none());
        assert!(matches!(h.app.session.live().status, Status::Failed { .. }));
        poll(&mut h.app, &h.ctx);
        assert_eq!(h.app.compose.input, "online input");
        assert_eq!(h.app.compose.drafts, [draft("online draft")]);
        h.app.compose.input = "edited while offline".into();
        h.app.compose.drafts.push(draft("offline draft"));

        run(&mut h.app, &h.ctx, "map.close");
        h.app.session.execute("file.new", &json!({"name": "Other workspace"})).unwrap();
        poll(&mut h.app, &h.ctx);
        assert!(h.app.compose.input.is_empty());
        assert!(h.app.compose.drafts.is_empty());

        // A failed node is reopened into a new local document, with the same node identity.
        h.app.session.execute("file.new", &json!({"name": "Reopened"})).unwrap();
        h.joined();
        h.app.session.live_poll();
        h.app.session.live_mut().unwrap().info.as_mut().unwrap().link = node.route();
        assert_ne!(h.app.session.ds().unwrap().uid, original_uid);
        poll(&mut h.app, &h.ctx);
        assert_eq!(h.app.compose.input, "edited while offline");
        assert_eq!(h.app.compose.drafts, [draft("online draft"), draft("offline draft")]);

        // Returning to the original disconnected tab uses the same stable key too.
        h.app.session.active = h.app.session.docs.iter().position(|ds| ds.uid == original_uid).unwrap();
        poll(&mut h.app, &h.ctx);
        assert_eq!(h.app.compose.input, "edited while offline");
        assert_eq!(h.app.compose.drafts, [draft("online draft"), draft("offline draft")]);
    }

    #[test]
    fn composer_undo_redo_and_cursor_are_isolated_and_restored_per_workspace() {
        use egui::{Event, Key, Modifiers, RawInput};

        fn frame(app: &mut FigApp, ctx: &egui::Context, time: &mut f64, events: Vec<Event>) {
            *time += 2.0; // Let egui commit stable text to its undo history between edits.
            let input = RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0))),
                time: Some(*time),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                poll(app, ui.ctx());
                crate::compose::show(app, ui.ctx());
            });
            output.textures_delta.clear();
        }

        fn history(redo: bool) -> Vec<Event> {
            vec![Event::Key {
                key: Key::Z,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers { shift: redo, ..Modifiers::COMMAND },
            }]
        }

        let mut app = app();
        app.ui.home = false;
        app.compose.focus_input = true;
        let ctx = egui::Context::default();
        let mut time = 0.0;
        let first = app.session.active;
        frame(&mut app, &ctx, &mut time, vec![]);
        frame(&mut app, &ctx, &mut time, vec![Event::Text("alpha".into())]);
        frame(&mut app, &ctx, &mut time, vec![]);
        frame(&mut app, &ctx, &mut time, vec![Event::Text(" one".into())]);
        frame(&mut app, &ctx, &mut time, vec![]);
        assert_eq!(app.compose.input, "alpha one");
        let first_cursor = egui::TextEdit::load_state(&ctx, crate::compose::input_id()).unwrap().cursor.char_range();

        app.session.execute("file.new", &json!({"name": "Second"})).unwrap();
        let second = app.session.active;
        frame(&mut app, &ctx, &mut time, vec![]);
        frame(&mut app, &ctx, &mut time, history(false));
        assert!(app.compose.input.is_empty(), "Undo must not bring the first workspace's prompt into the second");
        frame(&mut app, &ctx, &mut time, history(true));
        assert!(app.compose.input.is_empty());
        frame(&mut app, &ctx, &mut time, vec![Event::Text("bravo".into())]);
        frame(&mut app, &ctx, &mut time, vec![]);
        assert_eq!(app.compose.input, "bravo");
        frame(&mut app, &ctx, &mut time, history(false));
        assert!(app.compose.input.is_empty());

        app.session.active = first;
        poll(&mut app, &ctx);
        assert_eq!(egui::TextEdit::load_state(&ctx, crate::compose::input_id()).unwrap().cursor.char_range(), first_cursor);
        frame(&mut app, &ctx, &mut time, history(false));
        assert_eq!(app.compose.input, "alpha");
        frame(&mut app, &ctx, &mut time, history(true));
        assert_eq!(app.compose.input, "alpha one");

        app.session.active = second;
        // The second workspace's pending redo survives editing the first workspace's history.
        frame(&mut app, &ctx, &mut time, history(true));
        assert_eq!(app.compose.input, "bravo");
        frame(&mut app, &ctx, &mut time, history(false));
        assert!(app.compose.input.is_empty());
    }

    #[test]
    fn focus_actions_use_existing_composer_and_view_state() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.ui.home = false;
        assert!(run(&mut app, &ctx, "map.focusMode"));
        assert!(app.session.ds().unwrap().view.ui_hidden);
        assert!(run(&mut app, &ctx, "map.focusComposer"));
        assert!(app.compose.focus_input);
        assert!(!app.session.ds().unwrap().view.ui_hidden);
        assert!(run(&mut app, &ctx, "map.focusMode"));
        assert!(run(&mut app, &ctx, "map.focusMode"));
        assert!(!app.session.ds().unwrap().view.ui_hidden);
        assert!(!run(&mut app, &ctx, "map.notAShellAction"));
    }

    #[test]
    fn creation_opens_the_existing_form_without_resetting_an_open_form() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.compose.drafts.push(draft("retained"));
        assert!(run(&mut app, &ctx, "map.newWorkspace"));
        assert!(app.ui.home);
        assert!(app.new_workspace.open);
        app.new_workspace.folder = "/synthetic/folder".into();
        assert!(run(&mut app, &ctx, "map.newWorkspace"));
        assert_eq!(app.new_workspace.folder, "/synthetic/folder");
        assert_eq!(app.compose.drafts, [draft("retained")]);
        assert!(!app.new_workspace.creating);
    }

    #[test]
    fn sync_review_only_opens_the_existing_panel() {
        let mut app = app();
        app.ui.home = false;
        app.session.ds_mut().unwrap().view.ui_hidden = true;
        assert!(run(&mut app, &egui::Context::default(), "map.syncReview"));
        assert!(app.sync.open);
        assert!(!app.session.ds().unwrap().view.ui_hidden);
        assert!(app.sync.job.is_none());
        assert!(app.sync.job_rx.is_none());
        assert!(app.session.journal.is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn worktree_action_opens_the_palette_on_the_worktrees_without_opening_a_workspace() {
        let mut app = app();
        assert!(run(&mut app, &egui::Context::default(), "map.switchWorktree"));
        assert!(matches!(crate::map_palette::scope(&app), Some(crate::map_palette::Scope::Worktree { current: true, .. })));
        assert_eq!(app.ui.palette.as_deref(), Some(""));
        assert!(app.status.switching.is_none());
        assert!(app.ws.busy.is_none());
    }

    #[test]
    fn save_without_a_connected_node_fails_without_file_dialogs() {
        let mut app = app();
        app.ui.home = false;
        let revision = app.session.ds().unwrap().revision;
        assert!(run(&mut app, &egui::Context::default(), "map.save"));
        assert!(matches!(app.map_shell.save, SaveStatus::Error(_)));
        assert!(app.map_shell.pending_save.is_none());
        assert!(app.session.ui_requests.is_empty());
        assert_eq!(app.session.ds().unwrap().revision, revision);
    }

    #[test]
    fn save_completion_is_polled_on_home_and_stays_with_its_original_target() {
        let mut app = app();
        let ctx = egui::Context::default();
        let (tx, rx) = std::sync::mpsc::channel();
        app.map_shell.save = SaveStatus::Saving("original".into());
        app.map_shell.pending_save = Some(PendingSave { label: "original".into(), rx });
        run(&mut app, &ctx, "map.save");
        poll(&mut app, &ctx);
        assert!(matches!(app.map_shell.save, SaveStatus::Saving(_)));
        app.ui.home = true;
        app.active_workspace = Some("other".into());
        tx.send(Ok(())).unwrap();
        poll(&mut app, &ctx);
        assert_eq!(app.map_shell.save, SaveStatus::Saved("original".into()));
        assert!(app.map_shell.pending_save.is_none());
    }

    #[test]
    fn failed_and_disconnected_save_workers_report_errors() {
        for failure in [Some("disk conflict"), None] {
            let mut app = app();
            let (tx, rx) = std::sync::mpsc::channel();
            app.map_shell.pending_save = Some(PendingSave { label: "original".into(), rx });
            if let Some(reason) = failure {
                tx.send(Err(reason.into())).unwrap();
            }
            drop(tx);
            poll(&mut app, &egui::Context::default());
            assert!(matches!(app.map_shell.save, SaveStatus::Error(_)));
            assert!(app.ui.last_error.as_deref().unwrap().contains(failure.unwrap_or("outcome is unknown")));
            assert!(app.map_shell.pending_save.is_none());
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn save_protocol_only_requests_the_authoritative_session_save() {
        let target = crate::node::SessionRef { address: "synthetic:1".into(), session: "session-a".into(), ..Default::default() };
        assert_eq!(save_request(&target), json!({"method": "session.save", "session": "session-a"}));
        assert!(saved_reply(json!({"ok": true})).is_ok());
        assert_eq!(saved_reply(json!({"ok": false, "error": "conflict"})), Err("conflict".into()));
        assert!(saved_reply(json!({})).is_err());
    }
}
