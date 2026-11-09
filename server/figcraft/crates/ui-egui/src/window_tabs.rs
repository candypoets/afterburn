//! Native window tabs (design: `Native window tabs` on the Afterburn page): several projects (a
//! workspace or a worktree, each a node map) open at once, each in a window of its own with its
//! own sidebar, page tabs, canvas and Sync. macOS groups the windows as native window tabs (the
//! desktop app's `mac_tabs`); elsewhere they are separate windows.
//!
//! A project window is a [`crate::windows`] window holding the project's document. Opening one:
//! choosing a workspace row ([`open_row`]), ⌘T then a workspace ([`request_new_tab`]), or the control
//! channel's `window.openWorkspace`. A project already open is never joined twice: its window comes
//! to the front instead (a node connection has one peer per session).
//!
//! The workspace browser and its Open request are shared by every window, so an Open is followed
//! here ([`Pending`]): when its map arrives (in whichever window's frame), it moves to the window
//! that asked for it, a new one for a plain choice, and the main window keeps what it showed.
//!
//! Titles: the window is `<workspace> — <branch>`, its tab `<workspace> · <branch>` ([`titles`]).

use std::collections::HashMap;

use egui::Context;

use crate::FigApp;
use crate::switcher::Row;
#[cfg(not(target_arch = "wasm32"))]
use crate::workspaces::Busy;

/// Where an Open's map goes once it arrives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Into {
    /// A new window (a new tab on macOS).
    NewWindow,
    /// Window `id`, in place of its project (a switch made in a window other than the main one).
    Window(u64),
}

/// An Open on its way: the project it opens and where its map goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub address: String,
    pub workspace: String,
    pub into: Into,
    /// The window it was asked from, the map it showed and whether it showed the workspace browser.
    pub origin: u64,
    pub origin_active: Option<u64>,
    pub origin_home: bool,
    /// The main window's map then and whether it showed the browser: a join that lands in the main
    /// window's frame puts both back.
    pub main_active: Option<u64>,
    pub main_home: bool,
}

/// The windows' projects: Opens on their way, and the projects last known per open map (a lost
/// connection forgets its session, the window keeps its title).
#[derive(Default)]
pub struct State {
    pub pending: Vec<Pending>,
    known: HashMap<u64, Project>,
}

/// One project: where its node is, its workspace id, and how its window and tab are titled.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Project {
    pub address: String,
    pub workspace: String,
    pub name: String,
    pub branch: String,
}

impl Project {
    pub fn window_title(&self) -> String {
        titles(&self.name, &self.branch).0
    }

    pub fn tab_title(&self) -> String {
        titles(&self.name, &self.branch).1
    }
}

/// The window's title (`afterburn — main`) and its tab's (`afterburn · main`); a project without a
/// branch is titled with its name alone.
pub fn titles(name: &str, branch: &str) -> (String, String) {
    let (name, branch) = (name.trim(), branch.trim());
    let name = if name.is_empty() { "Afterburn" } else { name };
    if branch.is_empty() { (name.to_string(), name.to_string()) } else { (format!("{name} — {branch}"), format!("{name} · {branch}")) }
}

/// What opening a project in a new window does, given where it is open now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Not open: ask the node for it, then give it a window.
    Join,
    /// Open and shown in window `id`: bring that window to the front.
    Focus(u64),
    /// Open but not shown (an earlier map of a window that switched since): its own window.
    Move(u64),
}

/// The plan for a project open as map `open` (if it is), held by window `owner`, which shows it
/// or not.
pub fn plan(open: Option<u64>, owner: u64, shown: bool) -> Plan {
    match open {
        None => Plan::Join,
        Some(_) if shown => Plan::Focus(owner),
        Some(uid) => Plan::Move(uid),
    }
}

/// The project of open map `uid`: its live session's, else the last one known.
pub fn project_of(app: &FigApp, uid: u64) -> Option<Project> {
    live_project(app, uid).or_else(|| app.window_tabs.known.get(&uid).cloned())
}

#[cfg(not(target_arch = "wasm32"))]
fn live_project(app: &FigApp, uid: u64) -> Option<Project> {
    let target = crate::node::SessionRef::parse(&app.session.live_of(uid).info.as_ref()?.link).ok()?;
    let name = workspace_name(app, &target.workspace).unwrap_or_else(|| last_segment(&target.path).unwrap_or(&target.workspace).to_string());
    let branch = if target.branch.trim().is_empty() { last_segment(&target.path).unwrap_or_default().to_string() } else { target.branch.clone() };
    Some(Project { address: target.address, workspace: target.workspace, name, branch })
}

#[cfg(target_arch = "wasm32")]
fn live_project(_app: &FigApp, _uid: u64) -> Option<Project> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
/// A path's last part (`/src/accounting` → `accounting`).
fn last_segment(path: &str) -> Option<&str> {
    path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().filter(|s| !s.is_empty())
}

/// The name the sidebar gives workspace `id`: its repository's (a worktree shares its base
/// checkout's), from any machine's listing.
#[cfg(not(target_arch = "wasm32"))]
fn workspace_name(app: &FigApp, id: &str) -> Option<String> {
    let lists = std::iter::once(&app.ws.rows).chain(app.ws.lists.values().map(|l| &l.rows));
    for rows in lists {
        if rows.iter().any(|w| w.id == id) {
            let groups = crate::workspaces::grouped(rows);
            let base = groups.iter().find(|(_, members)| members.iter().any(|m| m.id == id)).map(|(base, _)| base);
            return base.map(|b| b.name().to_string());
        }
    }
    None
}

/// The open map of project `workspace` on the node at `address`.
pub fn find_open(app: &FigApp, address: &str, workspace: &str) -> Option<u64> {
    app.session.docs.iter().map(|d| d.uid).find(|uid| live_project(app, *uid).is_some_and(|p| p.address == address && p.workspace == workspace))
}

/// The title of the window showing map `uid` (none: the workspace browser or a plain file).
pub fn window_title(app: &FigApp, uid: Option<u64>) -> Option<String> {
    uid.and_then(|uid| project_of(app, uid)).map(|p| p.window_title())
}

/// Every window's title and its tab's: the main window first, then the others (for the
/// platform's native tabs).
pub fn native_titles(app: &FigApp) -> Vec<(String, String)> {
    crate::windows::shown_maps(app)
        .into_iter()
        .filter_map(|(_, uid)| uid.and_then(|uid| project_of(app, uid)))
        .map(|p| (p.window_title(), p.tab_title()))
        .collect()
}

/// macOS shows a tab bar on the windows titled `tabbed` (their native title bar and tabs take the
/// top of the window, so the content keeps no band for the traffic lights).
pub fn set_tab_bars(app: &mut FigApp, ctx: &Context, tabbed: &[String]) {
    let shown = crate::windows::shown_maps(app);
    for (id, uid) in shown {
        let title = window_title(app, uid).unwrap_or_else(|| "Afterburn".to_string());
        let on = tabbed.contains(&title);
        if crate::windows::set_native_tab_bar(app, id, on) {
            ctx.request_repaint_of(crate::windows::viewport(id));
        }
    }
}

/// Choosing a workspace row (click or ↵): open it in a new window (tab). A row without a map can't open a window
/// of its own yet: it goes to the browser for Init, as a plain choice does.
pub fn open_row(app: &mut FigApp, ctx: &Context, row: &Row) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if row.map != crate::workspaces::MapState::Map || ctx.embed_viewports() {
            crate::switcher::switch_here(app, ctx, row);
            return;
        }
        crate::switcher::close(app);
        app.ui.new_tab_armed = false;
        if !row.in_use {
            let host = crate::node_hosts::hosts(&app.ws.hosts)
                .iter()
                .find(|host| host.address == row.address)
                .cloned()
                .unwrap_or_else(|| crate::node_hosts::resolve(&row.address));
            crate::workspaces::switch_to(app, &host, Some(&row.id));
            if crate::workspaces::in_use_key(app).as_deref() != Some(row.address.as_str()) {
                app.toast("Another workspace is still opening.");
                return;
            }
        }
        open_workspace(app, ctx, &crate::workspaces::service_addr(), &row.id);
    }
    #[cfg(target_arch = "wasm32")]
    crate::switcher::switch_here(app, ctx, row);
}

/// Open project `workspace` (on the machine in use, `address`) in a new window, or bring its window
/// to the front when it is open. Returns what it did.
#[cfg(not(target_arch = "wasm32"))]
pub fn open_workspace(app: &mut FigApp, ctx: &Context, address: &str, workspace: &str) -> Plan {
    let open = find_open(app, address, workspace);
    let owner = open.map_or(0, |uid| app.windows.of(uid));
    let shown = open.is_some_and(|uid| crate::windows::shown_maps(app).contains(&(owner, Some(uid))));
    let planned = plan(open, owner, shown);
    match planned {
        Plan::Focus(id) => {
            if let Err(e) = crate::windows::focus(app, ctx, id) {
                app.toast(e);
            }
        }
        Plan::Move(uid) => move_to_new_window(app, ctx, uid),
        Plan::Join => {
            if app.ws.busy.is_some() {
                app.toast("Another workspace is still opening.");
                return planned;
            }
            let entry = pending(app, address, workspace, Into::NewWindow);
            app.ws.act(ctx, workspace, false);
            if matches!(&app.ws.busy, Some(Busy::Open { workspace: w, .. }) if w == workspace) {
                app.window_tabs.pending.retain(|p| p.workspace != workspace);
                app.window_tabs.pending.push(entry);
            }
        }
    }
    planned
}

#[cfg(not(target_arch = "wasm32"))]
/// An Open of `workspace` asked from the window drawing now.
fn pending(app: &FigApp, address: &str, workspace: &str, into: Into) -> Pending {
    let main_active = if app.windows.current == 0 { app.session.ds().ok().map(|d| d.uid) } else { crate::windows::main_active(app) };
    Pending {
        address: address.to_string(),
        workspace: workspace.to_string(),
        into,
        origin: app.windows.current,
        origin_active: app.session.ds().ok().map(|d| d.uid),
        origin_home: app.ui.home,
        main_active,
        main_home: if app.windows.current == 0 { app.ui.home } else { crate::windows::main_home(app) },
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Give open map `uid` a new window of its own.
fn move_to_new_window(app: &mut FigApp, ctx: &Context, uid: u64) {
    let Some(index) = app.session.docs.iter().position(|d| d.uid == uid) else { return };
    match crate::windows::open(app, ctx, Some(index), None, None) {
        Ok(id) => {
            // The new window shows the map, not the browser a map-mode window starts on; the main
            // window looks again soon (macOS tabs the new window with the others then).
            crate::windows::set_home(app, id, false);
            ctx.request_repaint_after_for(std::time::Duration::from_millis(120), egui::ViewportId::ROOT);
        }
        Err(e) => {
            // One window here (the browser, a screen reader): the map opens in place.
            app.session.active = index;
            app.ui.home = false;
            app.toast(e);
        }
    }
}

/// ⌘T: choose a workspace to open in a new window tab (the workspace switcher, armed so its next
/// choice opens a window).
pub fn request_new_tab(app: &mut FigApp) {
    app.ui.new_tab_armed = true;
    crate::switcher::open(app);
}

/// The switcher's choice after ⌘T: `true` when it opened in a new window (the caller is done).
/// The hook at the top of [`crate::switcher::switch_here`].
pub fn take_armed(app: &mut FigApp, ctx: &Context, row: &Row) -> bool {
    if !std::mem::take(&mut app.ui.new_tab_armed) {
        return false;
    }
    open_row(app, ctx, row);
    true
}

/// The platform's own tab switching (macOS: the key window's tab group selects its next or previous
/// tab, in the order the human arranged them); `false` when the windows aren't tabbed.
static NATIVE_STEP: std::sync::OnceLock<fn(i32) -> bool> = std::sync::OnceLock::new();

/// The desktop app's native tab switching ([`NATIVE_STEP`]), once at startup.
pub fn set_native_step(step: fn(i32) -> bool) {
    let _ = NATIVE_STEP.set(step);
}

/// The window `by` places after `current` among `ids`, wrapping around.
pub fn next_window(ids: &[u64], current: u64, by: i32) -> Option<u64> {
    let n = i64::try_from(ids.len()).ok().filter(|n| *n > 0)?;
    let at = ids.iter().position(|id| *id == current).and_then(|i| i64::try_from(i).ok()).unwrap_or(0);
    let next = (at + i64::from(by)).rem_euclid(n);
    usize::try_from(next).ok().and_then(|i| ids.get(i)).copied()
}

/// ⇧⌘] / ⇧⌘[ (⌃Tab elsewhere): the next or previous project window comes to the front.
pub fn step(app: &mut FigApp, ctx: &Context, by: i32) {
    if NATIVE_STEP.get().is_some_and(|native| native(by)) {
        return;
    }
    let ids: Vec<u64> = std::iter::once(0).chain(app.windows.list.iter().map(|w| w.id)).collect();
    if let Some(next) = next_window(&ids, app.windows.current, by).filter(|next| *next != app.windows.current)
        && let Err(e) = crate::windows::focus(app, ctx, next)
    {
        app.toast(e);
    }
}

/// Each frame of a window: ⌘T's request lapses once nothing is open to choose from.
pub(crate) fn frame(app: &mut FigApp) {
    if app.ui.new_tab_armed && !crate::switcher::is_open(app) {
        app.ui.new_tab_armed = false;
    }
}

/// After window `id` drew: an Open it started (a switch in place) brings its map to it, not to the
/// main window.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn after_window_frame(app: &mut FigApp, id: u64, before: (Option<&Busy>, Option<u64>, bool), main: (Option<u64>, bool)) {
    let (busy_before, active_before, home_before) = before;
    let Some(Busy::Open { workspace, .. }) = app.ws.busy.clone() else { return };
    if busy_before == app.ws.busy.as_ref() || app.window_tabs.pending.iter().any(|p| p.workspace == workspace) {
        return;
    }
    app.window_tabs.pending.push(Pending {
        address: crate::workspaces::service_addr(),
        workspace,
        into: Into::Window(id),
        origin: id,
        origin_active: active_before,
        origin_home: home_before,
        main_active: main.0,
        main_home: main.1,
    });
}

/// The main window's frame, after the node's events: Opens whose map arrived give it to its window;
/// failed ones are dropped (the browser shows why).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn poll(app: &mut FigApp, ctx: &Context) {
    // Titles survive a lost connection.
    let uids: Vec<u64> = app.session.docs.iter().map(|d| d.uid).collect();
    for uid in &uids {
        if let Some(p) = live_project(app, *uid) {
            app.window_tabs.known.insert(*uid, p);
        }
    }
    app.window_tabs.known.retain(|uid, _| uids.contains(uid));
    if app.window_tabs.pending.is_empty() {
        return;
    }
    let settled = app.ws.busy.is_none() && app.ws.join.is_none();
    for p in std::mem::take(&mut app.window_tabs.pending) {
        let arrived = find_open(app, &p.address, &p.workspace).filter(|uid| app.session.live_of(*uid).status == figcraft_engine::live::Status::Live);
        match arrived {
            Some(uid) if settled => land(app, ctx, &p, uid),
            None if settled => {
                if p.into == Into::NewWindow
                    && let Some(e) = app.ws.error.clone()
                {
                    app.toast(format!("Couldn't open the workspace in a new window: {e}"));
                }
            }
            _ => app.window_tabs.pending.push(p),
        }
    }
}

/// Open `p`'s map, `uid`, where it was asked for, and put the main window (and the window it was
/// asked from) back as they were.
#[cfg(not(target_arch = "wasm32"))]
fn land(app: &mut FigApp, ctx: &Context, p: &Pending, uid: u64) {
    let owner = app.windows.of(uid);
    match &p.into {
        // The map may have landed in the main window or in the window that asked (its frame took
        // the join in): either way it moves on, and that window shows what it showed.
        Into::NewWindow if owner == 0 || owner == p.origin || !crate::windows::shown_maps(app).contains(&(owner, Some(uid))) => {
            move_to_new_window(app, ctx, uid);
            if p.origin != 0 {
                let (home, active) = (p.origin_home, p.origin_active);
                let _ = crate::windows::as_window(app, p.origin, |app| {
                    app.ui.home = home;
                    if let Some(i) = active.and_then(|a| app.session.docs.iter().position(|d| d.uid == a && app.shows(a))) {
                        app.session.active = i;
                    }
                });
            }
        }
        Into::NewWindow => {
            // Already shown in a window of its own (opened twice): that window comes forward.
            let _ = crate::windows::focus(app, ctx, owner);
        }
        Into::Window(id) if *id != 0 && app.windows.exists(*id) => {
            // Shown in another window already (the main one showed it before the join, or another
            // window shows it): that window comes forward, and this one keeps its map.
            let elsewhere = if owner == 0 {
                p.main_active == Some(uid) && !p.main_home
            } else {
                owner != *id && crate::windows::shown_maps(app).contains(&(owner, Some(uid)))
            };
            if elsewhere {
                let home = p.origin_home;
                let _ = crate::windows::as_window(app, *id, |app| {
                    app.ui.home = home;
                    app.status.switching = None;
                });
                let _ = crate::windows::focus(app, ctx, owner);
                return;
            }
            crate::windows::give(app, uid, *id);
            crate::windows::set_home(app, *id, false);
            ctx.request_repaint_of(crate::windows::viewport(*id));
        }
        Into::Window(_) => return,
    }
    // The main window keeps the map it showed (the join made the new one active here).
    if app.windows.of(uid) != 0 {
        app.ui.home = p.main_home;
        if let Some(i) = p.main_active.and_then(|m| app.session.docs.iter().position(|d| d.uid == m && app.windows.of(m) == 0)) {
            app.session.active = i;
        }
        crate::workspaces::update_active(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_follow_the_design() {
        assert_eq!(titles("afterburn", "main"), ("afterburn — main".to_string(), "afterburn · main".to_string()));
        assert_eq!(titles("code", "feature/sync-pane").1, "code · feature/sync-pane");
        assert_eq!(titles(" accounting ", ""), ("accounting".to_string(), "accounting".to_string()));
        assert_eq!(titles("", "").0, "Afterburn");
    }

    #[test]
    fn an_open_project_is_never_joined_twice() {
        // Not open: join it.
        assert_eq!(plan(None, 0, false), Plan::Join);
        // Shown in window 3: that window comes forward.
        assert_eq!(plan(Some(9), 3, true), Plan::Focus(3));
        assert_eq!(plan(Some(9), 0, true), Plan::Focus(0));
        // Open but hidden behind another map of its window: it moves to a window of its own.
        assert_eq!(plan(Some(9), 0, false), Plan::Move(9));
    }

    #[test]
    fn next_and_previous_window_wrap_around() {
        let ids = [0, 4, 7];
        assert_eq!(next_window(&ids, 0, 1), Some(4));
        assert_eq!(next_window(&ids, 7, 1), Some(0));
        assert_eq!(next_window(&ids, 0, -1), Some(7));
        assert_eq!(next_window(&ids, 4, -1), Some(0));
        // A window that closed meanwhile counts as the first.
        assert_eq!(next_window(&ids, 99, 1), Some(4));
        assert_eq!(next_window(&[], 0, 1), None);
    }

    #[test]
    fn last_segment_names_a_checkout() {
        assert_eq!(last_segment("/src/accounting/"), Some("accounting"));
        assert_eq!(last_segment("C:\\src\\code"), Some("code"));
        assert_eq!(last_segment(""), None);
    }
}
