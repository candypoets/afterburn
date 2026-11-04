//! The workspace switcher's data: every workspace the app knows, grouped by machine, and the
//! worktrees of each. The palette shows it ([`crate::map_palette`], design: `Palette Switch
//! workspace`, `Palette Pick worktree`, `Palette Switch worktree` on the Mindmap page): ⌃⌘O, a
//! click on the sidebar's head or on the status bar's worktree open its Workspace scope; Switch
//! worktree opens the open workspace's worktrees. The Workspace menu lists the same rows
//! ([`menu_rows`], [`worktree_rows`]).
//!
//! The rows come from the same listings the Workspaces screen uses: the machine in use answers
//! through `workspaces::State::rows`, the other machines through the listings the browser already
//! made of them. Choosing a workspace with a map switches the map in place; one without a map
//! opens the browser on it, where Init chooses its agent.
//!
//! A switch in flight or failed is the status bar's [`crate::status_bar::State`]; the palette
//! shows it in place of the rows while its Workspace or Worktree scope is open.

use crate::FigApp;
use crate::map_palette::Scope;
use crate::status_bar::Switch;
use crate::workspaces::{self, MapState, Workspace};

/// One machine and the workspaces listed for it.
#[derive(Clone, Debug, Default)]
pub struct Machine {
    pub name: String,
    pub address: String,
    /// Reached over SSH (its group is titled with the machine's name).
    pub ssh: bool,
    /// The machine in use: its rows open without switching machines.
    pub in_use: bool,
    pub rows: Vec<Workspace>,
}

/// One workspace or worktree row (of the palette and of the Workspace menu).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    /// The machine's address (`ssh:<target>` for an SSH host).
    pub address: String,
    pub machine: String,
    pub ssh: bool,
    pub in_use: bool,
    /// The workspace id to open.
    pub id: String,
    /// The workspace's name, or a nested worktree's branch.
    pub name: String,
    pub map: MapState,
    /// A worktree nested under its workspace.
    pub nested: bool,
    /// The open map is here: the workspace that holds it, or the worktree itself.
    pub open: bool,
    /// The checkout's folder.
    pub path: Option<String>,
    /// The checkout's branch (or its folder's name).
    pub branch: String,
    /// How many agents run there.
    pub agents: u32,
    /// How many worktrees the workspace has (1 for a plain checkout; 0 on a nested row).
    pub worktrees: usize,
}

/// One machine's rows under its eyebrow.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Group {
    pub title: String,
    pub rows: Vec<Row>,
}

/// What a row says at its right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    /// ✓ OPEN: the open workspace.
    Open,
    /// ✓ alone: the open worktree under it.
    Here,
    Map,
    Empty,
    Init,
}

impl Badge {
    pub fn text(self) -> &'static str {
        match self {
            Badge::Open => "OPEN",
            Badge::Here => "",
            Badge::Map => "MAP",
            Badge::Empty => "EMPTY",
            Badge::Init => "INIT",
        }
    }
}

impl Row {
    pub fn badge(&self) -> Badge {
        match (self.open, self.nested, self.map) {
            (true, false, _) => Badge::Open,
            (true, true, _) => Badge::Here,
            (false, _, MapState::Map) => Badge::Map,
            (false, _, MapState::Empty) => Badge::Empty,
            (false, _, MapState::None) => Badge::Init,
        }
    }

    /// The Workspace menu's title: the name, and the machine for a remote one (`alzette · befree`).
    pub fn menu_label(&self) -> String {
        if self.ssh { format!("{} · {}", self.name, self.machine) } else { self.name.clone() }
    }

    /// A stable key for the keyboard cursor.
    pub fn key(&self) -> String {
        format!("{}\t{}\t{}", self.address, self.id, self.nested)
    }

    /// The palette's line under a workspace: its branch and its agents (`main · 3 agents`).
    pub fn detail(&self) -> String {
        let mut parts = Vec::new();
        if !self.branch.trim().is_empty() && self.branch != self.name {
            parts.push(self.branch.clone());
        }
        match self.agents {
            0 => {}
            1 => parts.push("1 agent".to_string()),
            n => parts.push(format!("{n} agents")),
        }
        parts.join(" · ")
    }
}

/// Whether `text` contains the lower-cased `query`.
fn hit(text: &str, query: &str) -> bool {
    text.to_lowercase().contains(query)
}

/// The row of workspace `w` on machine `m`.
fn row_of(m: &Machine, w: &Workspace, name: String, nested: bool, open: bool) -> Row {
    Row {
        address: m.address.clone(),
        machine: m.name.clone(),
        ssh: m.ssh,
        in_use: m.in_use,
        id: w.id.clone(),
        name,
        map: w.map_state(),
        nested,
        open,
        path: w.path.clone(),
        branch: workspaces::branch_label(w),
        agents: w.agents.max(w.agents_detail.len() as u32),
        worktrees: 0,
    }
}

/// A machine's eyebrow: this computer, or the remote machine's name.
fn title_of(m: &Machine) -> String {
    if m.ssh { m.name.to_uppercase() } else { "THIS COMPUTER".to_string() }
}

/// The switcher's groups: this computer first, then the SSH machines; in each, a row per
/// workspace (a repository's checkouts share one), the open workspace's worktrees nested under it.
/// A search keeps the rows whose name, path or branch contains it, and finds the worktrees of
/// every workspace, not only the open one.
pub fn build(machines: &[Machine], active: Option<&str>, query: &str) -> Vec<Group> {
    let query = query.trim().to_lowercase();
    let mut groups = Vec::new();
    for m in machines {
        let mut rows = Vec::new();
        for (base, members) in workspaces::grouped(&m.rows) {
            let holds = m.in_use && active.is_some_and(|id| members.iter().any(|w| w.id == id));
            let worktrees: Vec<Row> = if members.len() > 1 {
                workspaces::worktrees_of(&m.rows, &base)
                    .iter()
                    .map(|w| row_of(m, w, workspaces::branch_label(w), true, m.in_use && active == Some(w.id.as_str())))
                    .collect()
            } else {
                Vec::new()
            };
            let mut base_row = row_of(m, &base, base.name().to_string(), false, holds);
            base_row.worktrees = members.len().max(1);
            if query.is_empty() {
                rows.push(base_row);
                if holds {
                    rows.extend(worktrees);
                }
                continue;
            }
            let base_hit = hit(&base_row.name, &query) || base.path.as_deref().is_some_and(|path| hit(path, &query));
            let found: Vec<Row> = worktrees.into_iter().filter(|w| (base_hit && holds) || hit(&w.name, &query)).collect();
            if base_hit || !found.is_empty() {
                rows.push(base_row);
                rows.extend(found);
            }
        }
        if rows.is_empty() && !(m.in_use && query.is_empty()) {
            continue;
        }
        groups.push(Group { title: title_of(m), rows });
    }
    groups
}

/// The palette's Workspace scope: a row per workspace (never its worktrees: a workspace with
/// several opens them as a step of their own), grouped by machine. A search keeps the workspaces
/// whose name, folder or branch contains it, or one of whose worktrees' does.
pub fn workspace_groups(machines: &[Machine], active: Option<&str>, query: &str) -> Vec<Group> {
    let query = query.trim().to_lowercase();
    let mut groups = Vec::new();
    for m in machines {
        let mut rows = Vec::new();
        for (base, members) in workspaces::grouped(&m.rows) {
            let holds = m.in_use && active.is_some_and(|id| members.iter().any(|w| w.id == id));
            let mut row = row_of(m, &base, base.name().to_string(), false, holds);
            row.worktrees = members.len().max(1);
            if holds && let Some(open) = members.iter().find(|w| active == Some(w.id.as_str())) {
                // The line under the open workspace names the worktree that is open.
                row.branch = workspaces::branch_label(open);
                row.agents = open.agents.max(open.agents_detail.len() as u32);
            }
            let matches = query.is_empty()
                || members.iter().any(|w| {
                    hit(w.name(), &query) || w.path.as_deref().is_some_and(|path| hit(path, &query)) || hit(&workspaces::branch_label(w), &query)
                });
            if matches {
                rows.push(row);
            }
        }
        if rows.is_empty() && !(m.in_use && query.is_empty()) {
            continue;
        }
        groups.push(Group { title: title_of(m), rows });
    }
    groups
}

/// The worktrees of the workspace that holds `id` on the machine at `address`, the main checkout
/// first; a search keeps the ones whose branch or folder contains it.
pub fn worktrees_in(machines: &[Machine], address: &str, id: &str, active: Option<&str>, query: &str) -> Vec<Row> {
    let query = query.trim().to_lowercase();
    let Some(m) = machines.iter().find(|m| m.address == address) else { return Vec::new() };
    let Some(base) = workspaces::grouped(&m.rows).into_iter().map(|(base, _)| base).find(|base| {
        let all = workspaces::worktrees_of(&m.rows, base);
        all.iter().any(|w| w.id == id)
    }) else {
        return Vec::new();
    };
    workspaces::worktrees_of(&m.rows, &base)
        .iter()
        .map(|w| row_of(m, w, workspaces::branch_label(w), true, m.in_use && active == Some(w.id.as_str())))
        .filter(|row| query.is_empty() || hit(&row.name, &query) || row.path.as_deref().is_some_and(|path| hit(path, &query)))
        .collect()
}

/// The rows in the order the keyboard walks them.
pub fn flat(groups: &[Group]) -> Vec<&Row> {
    groups.iter().flat_map(|g| g.rows.iter()).collect()
}

/// The machines the app has listings for: this computer first, then the SSH machines.
#[cfg(not(target_arch = "wasm32"))]
pub fn machines(app: &FigApp) -> Vec<Machine> {
    let in_use = workspaces::in_use_key(app);
    let hosts: Vec<crate::node_hosts::Host> = crate::node_hosts::hosts(&app.ws.hosts)
        .iter()
        .enumerate()
        .filter(|(i, host)| *i == 0 || host.ssh.is_some())
        .map(|(_, host)| host.clone())
        .collect();
    let mut out = Vec::new();
    let in_listed = in_use.as_deref().is_some_and(|key| hosts.iter().any(|host| host.address == key));
    if !in_listed {
        // The hosts aren't known yet: the machine in use alone.
        let host = app.ws.hosts.selected.clone().unwrap_or_else(|| crate::node_hosts::resolve(&workspaces::service_addr()));
        out.push(Machine { name: host.name, address: host.address, ssh: host.ssh.is_some(), in_use: true, rows: app.ws.rows.clone() });
    }
    for host in hosts {
        let is_in_use = in_use.as_deref() == Some(host.address.as_str());
        let rows = if is_in_use {
            app.ws.rows.clone()
        } else {
            match app.ws.lists.get(&host.address) {
                Some(list) if list.loaded => list.rows.clone(),
                _ => continue,
            }
        };
        out.push(Machine { name: host.name, address: host.address, ssh: host.ssh.is_some(), in_use: is_in_use, rows });
    }
    out
}

/// The browser build lists one machine: the node it was served from.
#[cfg(target_arch = "wasm32")]
pub fn machines(app: &FigApp) -> Vec<Machine> {
    vec![Machine { name: "This computer".into(), address: workspaces::service_addr(), ssh: false, in_use: true, rows: app.ws.rows.clone() }]
}

/// The switcher's groups for `query`, as the app stands.
pub fn groups(app: &FigApp, query: &str) -> Vec<Group> {
    build(&machines(app), app.active_workspace.as_deref(), query)
}

/// The Workspace menu's workspaces (and ⌃⌘1…⌃⌘9's order): every workspace row, worktrees aside.
pub fn menu_rows(app: &FigApp) -> Vec<Row> {
    groups(app, "").into_iter().flat_map(|g| g.rows).filter(|row| !row.nested).collect()
}

/// The open workspace's worktrees, for Workspace › Worktree: its checkouts, the open one checked.
pub fn worktree_rows(app: &FigApp) -> Vec<Row> {
    let Some(active) = app.active_workspace.as_deref() else { return Vec::new() };
    let machines = machines(app);
    let Some(m) = machines.iter().find(|m| m.in_use) else { return Vec::new() };
    worktrees_in(&machines, &m.address, active, Some(active), "")
}

/// The `n`th workspace of the Workspace menu (1-based), for ⌃⌘`n`.
pub fn nth(app: &FigApp, n: usize) -> Option<Row> {
    n.checked_sub(1).and_then(|i| menu_rows(app).into_iter().nth(i))
}

/// Whether the palette shows the switcher (its Workspace or Worktree scope).
pub fn is_open(app: &FigApp) -> bool {
    matches!(crate::map_palette::scope(app), Some(Scope::Workspace | Scope::Worktree { .. }))
}

/// Open the palette on the workspaces, with an empty search.
pub fn open(app: &mut FigApp) {
    app.status.failed = None;
    crate::map_palette::open(app, Scope::Workspace);
}

/// ⌃⌘O: open the palette on the workspaces, or close it when it shows them.
pub fn toggle(app: &mut FigApp) {
    if is_open(app) { close(app) } else { open(app) }
}

/// Open the palette on the open workspace's worktrees (Switch worktree).
pub fn open_worktrees(app: &mut FigApp) {
    app.status.failed = None;
    let machines = machines(app);
    let address = machines.iter().find(|m| m.in_use).map(|m| m.address.clone()).unwrap_or_default();
    let id = app.active_workspace.clone().unwrap_or_default();
    let name = workspace_groups(&machines, Some(&id), "")
        .into_iter()
        .flat_map(|g| g.rows)
        .find(|row| row.open)
        .map(|row| row.name)
        .unwrap_or_else(|| "This workspace".to_string());
    crate::map_palette::open(app, Scope::Worktree { address, id, name, current: true });
}

/// Close the switcher: the palette when it shows it, and a failed switch's message. A switch in
/// flight keeps going (Cancel switch is the way to stop it).
pub fn close(app: &mut FigApp) {
    app.status.failed = None;
    if is_open(app) {
        crate::map_palette::close(app);
    }
}

/// The open workspace's line, for "Still viewing …".
pub(crate) fn current_name(app: &FigApp) -> String {
    groups(app, "").into_iter().flat_map(|g| g.rows).rfind(|row| row.open).map(|row| row.name).unwrap_or_else(|| "this workspace".to_string())
}

/// Open `row`: with a map, switch the map in place (the palette shows the switch until it is
/// done); without one, open the browser on it for Init. The open row just closes the switcher.
pub fn choose(app: &mut FigApp, ctx: &egui::Context, row: &Row) {
    if crate::window_tabs::take_armed(app, ctx, row) {
        return;
    }
    if row.open {
        close(app);
        return;
    }
    #[cfg(not(target_arch = "wasm32"))]
    if !row.in_use {
        let host = crate::node_hosts::hosts(&app.ws.hosts)
            .iter()
            .find(|host| host.address == row.address)
            .cloned()
            .unwrap_or_else(|| crate::node_hosts::resolve(&row.address));
        workspaces::switch_to(app, &host, Some(&row.id));
        if workspaces::in_use_key(app).as_deref() != Some(row.address.as_str()) {
            app.toast("Another workspace is still opening.");
            return;
        }
    }
    if row.map == MapState::Map {
        let from = current_name(app);
        app.status.switching = Some(Switch { id: row.id.clone(), branch: row.name.clone(), from });
        app.status.failed = None;
        app.ws.act(ctx, &row.id, false);
    } else {
        close(app);
        app.ws.select(&row.id);
        app.ws.worktree_init = row.nested;
        app.ui.home = true;
    }
}

/// ⌘↵ in the workspace or worktree list: open it in a new native window tab.
pub fn choose_in_new_window(app: &mut FigApp, ctx: &egui::Context, row: &Row) {
    crate::window_tabs::open_row(app, ctx, row);
}

/// Try the failed switch again.
pub fn retry(app: &mut FigApp, ctx: &egui::Context) {
    if let Some((switch, _)) = app.status.failed.take() {
        app.status.switching = Some(switch.clone());
        app.ws.act(ctx, &switch.id, false);
    }
}

/// Stop the switch in flight: the map on screen stays.
pub fn cancel(app: &mut FigApp) {
    app.ws.cancel_open();
    app.status.switching = None;
    close(app);
}

/// The Workspace menu's choice by machine and workspace id (the rows may have moved since the menu
/// was built, so it is found again).
pub fn choose_id(app: &mut FigApp, ctx: &egui::Context, address: &str, id: &str) {
    let found = groups(app, "")
        .into_iter()
        .flat_map(|g| g.rows)
        .find(|row| row.address == address && row.id == id)
        .or_else(|| worktree_rows(app).into_iter().find(|row| row.address == address && row.id == id));
    match found {
        Some(row) => choose(app, ctx, &row),
        None => app.toast("That workspace is no longer listed."),
    }
}

/// ⌃⌘`n`: open the Workspace menu's `n`th workspace.
pub fn open_nth(app: &mut FigApp, ctx: &egui::Context, n: usize) {
    if let Some(row) = nth(app, n) {
        choose(app, ctx, &row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(id: &str, label: &str, repo: &str, branch: &str, map: &str, worktree: bool) -> Workspace {
        Workspace {
            id: id.into(),
            label: label.into(),
            repo: Some(repo.into()),
            path: Some(format!("/root/code/{id}")),
            branch: Some(branch.into()),
            map: map.into(),
            worktree,
            ..Workspace::default()
        }
    }

    fn machines() -> Vec<Machine> {
        vec![
            Machine {
                name: "This computer".into(),
                address: "127.0.0.1:47822".into(),
                ssh: false,
                in_use: true,
                rows: vec![
                    ws("w1", "mindmap", "/root/code/mindmap", "main", "map", false),
                    ws("w2", "checkout", "/root/code/mindmap", "feature/checkout", "map", true),
                    ws("w3", "accounting", "/root/code/accounting", "main", "map", false),
                    ws("w4", "code", "/root/code/code", "main", "none", false),
                ],
            },
            Machine {
                name: "befree".into(),
                address: "ssh:befree".into(),
                ssh: true,
                in_use: false,
                rows: vec![ws("r1", "alzette", "/srv/alzette", "main", "map", false), ws("r2", "symphony", "/srv/symphony", "main", "none", false)],
            },
        ]
    }

    fn names(groups: &[Group]) -> Vec<(String, Badge)> {
        flat(groups).into_iter().map(|row| (row.name.clone(), row.badge())).collect()
    }

    #[test]
    fn rows_group_by_machine_with_the_open_workspaces_worktrees_nested() {
        let groups = build(&machines(), Some("w1"), "");
        let titles: Vec<&str> = groups.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(titles, ["THIS COMPUTER", "BEFREE"]);
        assert_eq!(
            names(&groups),
            [
                ("accounting".to_string(), Badge::Map),
                ("code".to_string(), Badge::Init),
                ("mindmap".to_string(), Badge::Open),
                ("main".to_string(), Badge::Here),
                ("feature/checkout".to_string(), Badge::Map),
                ("alzette".to_string(), Badge::Map),
                ("symphony".to_string(), Badge::Init),
            ]
        );
        assert_eq!(Badge::Open.text(), "OPEN");
        assert_eq!(Badge::Init.text(), "INIT");
    }

    #[test]
    fn a_closed_workspace_keeps_its_worktrees_folded_until_searched() {
        let groups = build(&machines(), Some("w3"), "");
        assert!(flat(&groups).iter().all(|row| row.name != "feature/checkout"));
        let found = build(&machines(), Some("w3"), "CHECKOUT");
        assert_eq!(names(&found), [("mindmap".to_string(), Badge::Map), ("feature/checkout".to_string(), Badge::Map)]);
        assert!(build(&machines(), None, "billing").is_empty());
    }

    #[test]
    fn the_open_worktree_marks_its_workspace_open() {
        let groups = build(&machines(), Some("w2"), "");
        let rows = flat(&groups);
        let open: Vec<(&str, bool)> = rows.iter().filter(|row| row.open).map(|row| (row.name.as_str(), row.nested)).collect();
        assert_eq!(open, [("mindmap", false), ("feature/checkout", true)]);
    }

    #[test]
    fn remote_rows_name_their_machine_in_the_menu() {
        let groups = build(&machines(), None, "");
        let labels: Vec<String> = flat(&groups).iter().filter(|row| !row.nested).map(|row| row.menu_label()).collect();
        assert_eq!(labels, ["accounting", "code", "mindmap", "alzette · befree", "symphony · befree"]);
    }

    #[test]
    fn an_empty_machine_in_use_still_shows_its_group() {
        let mut m = machines();
        m[0].rows.clear();
        let groups = build(&m, None, "");
        assert_eq!(groups[0].title, "THIS COMPUTER");
        assert!(groups[0].rows.is_empty());
        m[1].rows.clear();
        assert_eq!(build(&m, None, "").len(), 1, "an empty remote listing has no group");
    }

    use crate::layers_drag_tests::Ui;
    use egui::{Event, Key, Modifiers};
    use figcraft_engine::Session;
    use figcraft_engine::keymap::Platform;

    fn press(u: &mut Ui, key: Key, modifiers: Modifiers) {
        let ev = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
        u.frame_with(vec![ev(true), ev(false)], modifiers);
    }

    /// ⌃⌘ on a Mac; Ctrl+Alt elsewhere.
    fn ctrl_cmd() -> Modifiers {
        if crate::shortcuts::platform() == Platform::Mac {
            Modifiers::CTRL | Modifiers::MAC_CMD | Modifiers::COMMAND
        } else {
            Modifiers::CTRL | Modifiers::ALT | Modifiers::COMMAND
        }
    }

    fn cmd() -> Modifiers {
        if crate::shortcuts::platform() == Platform::Mac { Modifiers::MAC_CMD | Modifiers::COMMAND } else { Modifiers::CTRL | Modifiers::COMMAND }
    }

    fn map_ui() -> Ui {
        let mut u = Ui::new(Session::new());
        u.app.onboarding.active = false;
        u.app.onboarding.decided = true;
        u.app.ui.home = false;
        u.app.ws.rows = machines().remove(0).rows;
        u.app.active_workspace = Some("w1".into());
        u.frame(vec![]);
        u
    }

    #[test]
    fn ctrl_cmd_o_opens_the_palette_on_the_workspaces_and_escape_closes_it() {
        crate::with_map_mode(|| {
            let mut u = map_ui();
            press(&mut u, Key::O, ctrl_cmd());
            assert!(is_open(&u.app));
            assert_eq!(crate::map_palette::scope(&u.app), Some(&Scope::Workspace));
            press(&mut u, Key::Escape, Modifiers::NONE);
            assert!(!is_open(&u.app));
            assert!(u.app.ui.palette.is_none());
            // ⌘O, Open workspace's old key, opens it too.
            press(&mut u, Key::O, cmd());
            assert!(is_open(&u.app));
            assert!(!u.app.ui.home, "the map stays");
        });
    }

    #[test]
    fn ctrl_cmd_n_opens_the_menus_nth_workspace() {
        crate::with_map_mode(|| {
            let mut u = map_ui();
            let order: Vec<String> = menu_rows(&u.app).into_iter().map(|row| row.name).collect();
            assert_eq!(order, ["accounting", "code", "mindmap"]);
            assert!(crate::map_action_catalog::enabled(&u.app, crate::map_action_catalog::find("map.workspace3").unwrap()));
            assert!(!crate::map_action_catalog::enabled(&u.app, crate::map_action_catalog::find("map.workspace4").unwrap()));
            // ⌃⌘2 is `code`, without a map: the browser opens on it for Init.
            press(&mut u, Key::Num2, ctrl_cmd());
            assert!(u.app.ui.home);
            assert_eq!(u.app.ws.selected.as_deref(), Some("w4"));
        });
    }

    #[test]
    fn the_workspace_menu_lists_and_checks_the_open_workspace() {
        let u = map_ui();
        let items = crate::native_menu::workspace_items(&u.app);
        let shown: Vec<(&str, bool, Option<&str>)> = items.iter().map(|i| (i.label.as_str(), i.checked, i.keys)).collect();
        assert_eq!(shown, [("accounting", false, Some("Ctrl+Cmd+1")), ("code", false, Some("Ctrl+Cmd+2")), ("mindmap", true, Some("Ctrl+Cmd+3"))]);
        assert_eq!(items[0].id, "map.workspace1");
        let worktrees = crate::native_menu::worktree_items(&u.app);
        let shown: Vec<(&str, bool)> = worktrees.iter().map(|i| (i.label.as_str(), i.checked)).collect();
        assert_eq!(shown, [("main", true), ("feature/checkout", false)]);
        assert!(worktrees[1].id.starts_with(crate::native_menu::CHOOSE));
        assert!(crate::native_menu::enabled(&u.app, &worktrees[1].id));
    }
}
