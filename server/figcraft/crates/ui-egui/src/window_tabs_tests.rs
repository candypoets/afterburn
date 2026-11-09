//! Native window tabs: each project window keeps its own Sync, switcher and page tree, a map can
//! move into a window, a tab bar drops the traffic-light band, and the keys open and cycle windows.

use egui::{Key, Modifiers, RawInput};
use figcraft_engine::Session;
use figcraft_engine::keymap::Platform;
use serde_json::json;

use crate::{FigApp, NoServices, TitleBar};

fn frame(ctx: &egui::Context, app: &mut FigApp) {
    ctx.run_ui(RawInput::default(), |ui| app.ui(ui)).textures_delta.clear();
}

/// Files A, B, C; A moved to a window of its own. Returns the app and that window's id.
fn two_windows(ctx: &egui::Context) -> (FigApp, u64) {
    let mut s = Session::empty();
    for n in ["A", "B", "C"] {
        s.execute("file.new", &json!({"name": n})).unwrap();
    }
    ctx.set_embed_viewports(false);
    let mut app = FigApp::new(s, Box::new(NoServices));
    frame(ctx, &mut app);
    let w = crate::control::handle(&mut app, ctx, "window.new", &json!({"index": 0})).unwrap()["window"].as_u64().unwrap();
    frame(ctx, &mut app);
    (app, w)
}

fn uid(app: &FigApp, name: &str) -> u64 {
    app.session.docs.iter().find(|d| d.doc.name == name).unwrap().uid
}

#[test]
fn a_map_moves_into_the_window_that_asked_for_it() {
    let ctx = egui::Context::default();
    let (mut app, w) = two_windows(&ctx);
    let (a, b) = (uid(&app, "A"), uid(&app, "B"));
    assert!(crate::windows::shown_maps(&app).contains(&(w, Some(a))));
    crate::windows::give(&mut app, b, w);
    frame(&ctx, &mut app);
    assert_eq!(app.windows.of(b), w, "B is the window's now");
    assert!(crate::windows::shown_maps(&app).contains(&(w, Some(b))), "and the window shows it");
    let main = crate::windows::shown_maps(&app).into_iter().find(|(id, _)| *id == 0).and_then(|(_, u)| u);
    assert_eq!(main, Some(uid(&app, "C")), "the main window keeps one of its own");
    // The main window can't be given a file: it shows every file no other window holds.
    crate::windows::give(&mut app, b, 0);
    assert_eq!(app.windows.of(b), w);
}

#[test]
fn each_window_has_its_own_switcher_and_sync() {
    let ctx = egui::Context::default();
    let (mut app, w) = two_windows(&ctx);
    crate::windows::as_window(&mut app, w, |app| {
        crate::switcher::open(app);
        app.sync.note = "window note".into();
    })
    .unwrap();
    assert!(!crate::switcher::is_open(&app), "the main window's switcher stays closed");
    assert!(app.sync.note.is_empty(), "the main window's Sync is its own");
    let (open, note) = crate::windows::as_window(&mut app, w, |app| (crate::switcher::is_open(app), app.sync.note.clone())).unwrap();
    assert!(open);
    assert_eq!(note, "window note");
}

#[test]
fn a_tab_bar_takes_the_place_of_the_traffic_light_band() {
    let ctx = egui::Context::default();
    let (mut app, w) = two_windows(&ctx);
    app.ui.title_bar = TitleBar::Inset;
    assert!(app.ui.mac_inset());
    assert!(crate::windows::set_native_tab_bar(&mut app, 0, true));
    assert!(!crate::windows::set_native_tab_bar(&mut app, 0, true), "no change, no repaint");
    assert!(!app.ui.mac_inset(), "AppKit's title bar and tabs sit above the content");
    assert!(crate::windows::set_native_tab_bar(&mut app, w, true));
    assert!(crate::windows::as_window(&mut app, w, |app| app.ui.native_tab_bar).unwrap());
    assert!(!crate::windows::set_native_tab_bar(&mut app, 99, true), "no such window");
    assert!(crate::windows::set_native_tab_bar(&mut app, 0, false));
    assert!(app.ui.mac_inset());
}

#[test]
fn next_window_tab_brings_the_next_window_forward() {
    let ctx = egui::Context::default();
    let (mut app, w) = two_windows(&ctx);
    app.windows.focused = 0;
    crate::window_tabs::step(&mut app, &ctx, 1);
    assert_eq!(app.windows.focused, w);
    crate::windows::as_window(&mut app, w, |app| crate::window_tabs::step(app, &ctx, 1)).unwrap();
    assert_eq!(app.windows.focused, 0, "wraps around to the main window");
}

#[test]
fn command_t_arms_the_switcher_until_it_closes() {
    let ctx = egui::Context::default();
    let mut app = FigApp::new(Session::empty(), Box::new(NoServices));
    frame(&ctx, &mut app);
    crate::window_tabs::request_new_tab(&mut app);
    assert!(app.ui.new_tab_armed && crate::switcher::is_open(&app));
    crate::window_tabs::frame(&mut app);
    assert!(app.ui.new_tab_armed, "still choosing");
    crate::switcher::close(&mut app);
    crate::window_tabs::frame(&mut app);
    assert!(!app.ui.new_tab_armed, "closing the switcher drops the request: the next choice switches in place");
}

#[test]
fn window_tab_keys_are_bound() {
    let id = |platform, key, mods| crate::map_actions::lookup(platform, key, None, mods).map(|a| a.id);
    assert_eq!(id(Platform::Mac, Key::T, Modifiers::MAC_CMD), Some("map.newWindowTab"));
    assert_eq!(id(Platform::Other, Key::T, Modifiers::COMMAND), Some("map.newWindowTab"));
    let shift_cmd = Modifiers { shift: true, ..Modifiers::MAC_CMD };
    assert_eq!(id(Platform::Mac, Key::CloseBracket, shift_cmd), Some("map.nextWindowTab"));
    assert_eq!(id(Platform::Mac, Key::OpenBracket, shift_cmd), Some("map.previousWindowTab"));
    assert_eq!(id(Platform::Mac, Key::Tab, Modifiers::CTRL), Some("map.nextWindowTab"));
    assert_eq!(id(Platform::Mac, Key::Tab, Modifiers { shift: true, ..Modifiers::CTRL }), Some("map.previousWindowTab"));
    assert_eq!(id(Platform::Other, Key::Tab, Modifiers::CTRL), Some("map.nextWindowTab"));
}

#[test]
fn window_tab_actions_need_a_second_window_to_cycle() {
    let ctx = egui::Context::default();
    let mut app = FigApp::new(Session::empty(), Box::new(NoServices));
    frame(&ctx, &mut app);
    let action = |id| crate::map_action_catalog::find(id).unwrap();
    assert!(crate::map_action_catalog::enabled(&app, action("map.newWindowTab")));
    assert!(!crate::map_action_catalog::enabled(&app, action("map.nextWindowTab")));
    let (app, _) = two_windows(&ctx);
    assert!(crate::map_action_catalog::enabled(&app, action("map.nextWindowTab")));
}

/// Hosts each file as a node map: workspace = the file's name, checkout `/synthetic/<name>`.
struct Projects;

const NODE: &str = "127.0.0.1:0";

impl figcraft_engine::live::LiveLink for Projects {
    fn host(&mut self, _: u64, options: figcraft_engine::live::HostOptions) -> Result<figcraft_engine::live::SessionInfo, String> {
        let name = options.doc.name.clone();
        let node = crate::node::SessionRef {
            address: NODE.into(),
            session: format!("session-{name}"),
            workspace: name.clone(),
            path: format!("/synthetic/{name}"),
            branch: if name == "code" { "feature/sync-pane".into() } else { "main".into() },
        };
        Ok(figcraft_engine::live::SessionInfo {
            role: figcraft_engine::live::Role::Host,
            me: 1,
            name: "Tabs tester".into(),
            color: "#14AE5C".into(),
            file_name: name,
            pages: vec![],
            link: node.route(),
            url: String::new(),
            address: node.address,
            network: false,
            access: figcraft_engine::live::Access::Edit,
        })
    }
    fn join(&mut self, _: u64, _: &str, _: &str) -> Result<(), String> {
        Err("the fixture only hosts".into())
    }
    fn leave(&mut self, _: u64) {}
    fn presence(&mut self, _: u64, _: &figcraft_engine::live::Presence) {}
    fn send(&mut self, _: u64, _: figcraft_engine::figcraft_collab::message::ClientMsg) {}
    fn poll(&mut self) -> Vec<(u64, figcraft_engine::live::Event)> {
        Vec::new()
    }
}

/// Three projects open as node maps in the main window, `afterburn` shown.
fn projects(ctx: &egui::Context) -> FigApp {
    let mut s = Session::empty();
    s.live.install(Box::new(Projects));
    for n in ["afterburn", "accounting", "code"] {
        s.execute("file.new", &json!({"name": n})).unwrap();
        s.execute("collab.host", &json!({})).unwrap();
    }
    ctx.set_embed_viewports(false);
    let mut app = FigApp::new(s, Box::new(NoServices));
    app.ui.home = false;
    let first = app.session.docs.iter().position(|d| d.doc.name == "afterburn").unwrap();
    app.session.active = first;
    frame(ctx, &mut app);
    app
}

fn pending(workspace: &str, into: crate::window_tabs::Into, main_active: Option<u64>) -> crate::window_tabs::Pending {
    crate::window_tabs::Pending {
        address: NODE.into(),
        workspace: workspace.into(),
        into,
        origin: 0,
        origin_active: main_active,
        origin_home: false,
        main_active,
        main_home: false,
    }
}

#[test]
fn projects_are_titled_workspace_and_branch() {
    let ctx = egui::Context::default();
    let app = projects(&ctx);
    let code = crate::window_tabs::project_of(&app, uid(&app, "code")).unwrap();
    assert_eq!(code.window_title(), "code — feature/sync-pane");
    assert_eq!(code.tab_title(), "code · feature/sync-pane");
    assert_eq!(crate::window_tabs::find_open(&app, NODE, "accounting"), Some(uid(&app, "accounting")));
    assert_eq!(crate::window_tabs::find_open(&app, "elsewhere:1", "accounting"), None, "another machine's workspace is another project");
}

#[test]
fn an_arrived_map_opens_in_a_new_window_and_the_main_window_keeps_its_own() {
    let ctx = egui::Context::default();
    let mut app = projects(&ctx);
    let (afterburn, accounting) = (uid(&app, "afterburn"), uid(&app, "accounting"));
    // The join made `accounting` active in the main window, as a node welcome does.
    app.session.active = app.session.docs.iter().position(|d| d.uid == accounting).unwrap();
    app.window_tabs.pending.push(pending("accounting", crate::window_tabs::Into::NewWindow, Some(afterburn)));
    // In map mode, where a new window starts on the workspace browser.
    crate::with_map_mode(|| crate::window_tabs::poll(&mut app, &ctx));
    assert!(app.window_tabs.pending.is_empty(), "it landed");
    let w = app.windows.of(accounting);
    assert_ne!(w, 0, "accounting has a window of its own");
    assert!(crate::windows::shown_maps(&app).contains(&(w, Some(accounting))), "showing its map, not the browser");
    assert_eq!(app.session.ds().unwrap().uid, afterburn, "the main window still shows afterburn");
    frame(&ctx, &mut app);
    let titles = crate::window_tabs::native_titles(&app);
    assert_eq!(
        titles,
        vec![("afterburn — main".to_string(), "afterburn · main".to_string()), ("accounting — main".to_string(), "accounting · main".to_string())]
    );
    assert_eq!(crate::with_map_mode(|| app.title()), "afterburn — main", "the window is titled by its project");

    // Opening it again brings its window forward; it is never joined twice.
    app.windows.focused = 0;
    assert_eq!(crate::window_tabs::open_workspace(&mut app, &ctx, NODE, "accounting"), crate::window_tabs::Plan::Focus(w));
    assert_eq!(app.windows.focused, w);
    assert!(app.ws.busy.is_none(), "no Open was asked of the node");
    assert_eq!(crate::window_tabs::open_workspace(&mut app, &ctx, NODE, "afterburn"), crate::window_tabs::Plan::Focus(0));

    // `code`, open but hidden behind afterburn in the main window, moves to a window of its own.
    let code = uid(&app, "code");
    assert_eq!(crate::window_tabs::open_workspace(&mut app, &ctx, NODE, "code"), crate::window_tabs::Plan::Move(code));
    assert_ne!(app.windows.of(code), 0);
    assert_ne!(app.windows.of(code), w);
    assert_eq!(app.windows.list.len(), 2);
}

#[test]
fn a_switch_made_in_another_window_lands_there() {
    let ctx = egui::Context::default();
    let mut app = projects(&ctx);
    let (afterburn, code) = (uid(&app, "afterburn"), uid(&app, "code"));
    let index = app.session.docs.iter().position(|d| d.doc.name == "accounting").unwrap();
    let w = crate::windows::open(&mut app, &ctx, Some(index), None, None).unwrap();
    frame(&ctx, &mut app);
    // Window w switched to `code`; its map arrived in the main window's frame.
    app.session.active = app.session.docs.iter().position(|d| d.uid == code).unwrap();
    let mut p = pending("code", crate::window_tabs::Into::Window(w), Some(afterburn));
    p.origin = w;
    app.window_tabs.pending.push(p);
    crate::window_tabs::poll(&mut app, &ctx);
    assert_eq!(app.windows.of(code), w);
    assert!(crate::windows::shown_maps(&app).contains(&(w, Some(code))));
    assert_eq!(app.session.ds().unwrap().uid, afterburn);
}

#[test]
fn an_open_still_on_its_way_waits() {
    let ctx = egui::Context::default();
    let mut app = projects(&ctx);
    app.ws.busy = Some(crate::workspaces::Busy::Open { workspace: "elsewhere".into(), init: false, target: None });
    app.window_tabs.pending.push(pending("elsewhere", crate::window_tabs::Into::NewWindow, None));
    crate::window_tabs::poll(&mut app, &ctx);
    assert_eq!(app.window_tabs.pending.len(), 1);
    // The node refused: the Open ends without a map, and nothing opens.
    app.ws.busy = None;
    crate::window_tabs::poll(&mut app, &ctx);
    assert!(app.window_tabs.pending.is_empty());
    assert!(app.windows.list.is_empty());
}

#[test]
fn switching_to_a_project_another_window_shows_brings_that_window_forward() {
    let ctx = egui::Context::default();
    let mut app = projects(&ctx);
    let (accounting, code) = (uid(&app, "accounting"), uid(&app, "code"));
    let at = |app: &FigApp, u| app.session.docs.iter().position(|d| d.uid == u).unwrap();
    let i = at(&app, accounting);
    let w = crate::windows::open(&mut app, &ctx, Some(i), None, None).unwrap();
    let i = at(&app, code);
    let x = crate::windows::open(&mut app, &ctx, Some(i), None, None).unwrap();
    frame(&ctx, &mut app);
    app.windows.focused = w;
    let mut p = pending("code", crate::window_tabs::Into::Window(w), Some(uid(&app, "afterburn")));
    p.origin = w;
    app.window_tabs.pending.push(p);
    crate::window_tabs::poll(&mut app, &ctx);
    assert_eq!(app.windows.focused, x, "code's window comes forward");
    assert_eq!(app.windows.of(code), x, "and keeps it: a tab never closes because another one switched");
    assert!(crate::windows::shown_maps(&app).contains(&(w, Some(accounting))));
}
