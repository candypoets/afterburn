//! The workspace switcher (design: `Sidebar pages · switcher` on the Mindmap page): one list of
//! every workspace the app knows, grouped by machine, with the open workspace's worktrees nested
//! under it. Three doors open it: the sidebar's head, ⌃⌘O (and the palette), and the Workspace
//! menu, which lists the same rows ([`menu_rows`], [`worktree_rows`]).
//!
//! The rows come from the same listings the Workspaces screen uses: the machine in use answers
//! through `workspaces::State::rows`, the other machines through the listings the browser already
//! made of them. Choosing a workspace with a map switches the map in place; one without a map
//! opens the browser on it, where Init chooses its agent.
//!
//! The popover's state (open, search, cursor, a switch in flight or failed) is the status bar's
//! [`crate::status_bar::State`], which the palette's Switch worktree already opens.

use egui::{Align2, Color32, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::FigApp;
use crate::status_bar::{Phase, Switch};
use crate::theme::{Tokens, regular};
use crate::workspaces::{self, MapState, Skin, Workspace};

/// The popover's width; it overhangs the 300 pt sidebar, as designed.
const CARD_W: f32 = 378.0;
/// One row's height.
const ROW_H: f32 = 30.0;
/// A machine's eyebrow above its rows.
const GROUP_H: f32 = 24.0;

/// One machine and the workspaces listed for it.
#[derive(Clone, Debug, Default)]
pub struct Machine {
    pub name: String,
    pub address: String,
    /// Reached over SSH (its group is titled `NAME · SSH`).
    pub ssh: bool,
    /// The machine in use: its rows open without switching machines.
    pub in_use: bool,
    pub rows: Vec<Workspace>,
}

/// One row of the switcher (and of the Workspace menu).
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
}

/// Whether `text` contains the lower-cased `query`.
fn hit(text: &str, query: &str) -> bool {
    text.to_lowercase().contains(query)
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
            let row = |w: &Workspace, name: String, nested: bool, open: bool| Row {
                address: m.address.clone(),
                machine: m.name.clone(),
                ssh: m.ssh,
                in_use: m.in_use,
                id: w.id.clone(),
                name,
                map: w.map_state(),
                nested,
                open,
            };
            let worktrees: Vec<Row> = if members.len() > 1 {
                workspaces::worktrees_of(&m.rows, &base)
                    .iter()
                    .map(|w| row(w, workspaces::branch_label(w), true, m.in_use && active == Some(w.id.as_str())))
                    .collect()
            } else {
                Vec::new()
            };
            let base_row = row(&base, base.name().to_string(), false, holds);
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
        let title = if m.ssh { format!("{} · SSH", m.name.to_uppercase()) } else { "THIS COMPUTER".to_string() };
        groups.push(Group { title, rows });
    }
    groups
}

/// The rows in the order the keyboard walks them.
pub fn flat(groups: &[Group]) -> Vec<&Row> {
    groups.iter().flat_map(|g| g.rows.iter()).collect()
}

/// The row ↑ or ↓ lands on, wrapping; with nothing highlighted, the open row (or the first).
pub fn step(rows: &[&Row], cursor: Option<&str>, down: bool) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    let at = cursor.and_then(|key| rows.iter().position(|row| row.key() == key));
    let next = match at {
        None => rows.iter().position(|row| row.open).unwrap_or(0),
        Some(i) if down => (i + 1) % rows.len(),
        Some(i) => (i + rows.len() - 1) % rows.len(),
    };
    rows.get(next).map(|row| row.key())
}

/// The highlighted row: the cursor while it is listed, else the open row, else the first.
fn highlighted<'a>(rows: &[&'a Row], cursor: Option<&str>) -> Option<&'a Row> {
    cursor.and_then(|key| rows.iter().find(|row| row.key() == key)).or_else(|| rows.iter().find(|row| row.open)).or_else(|| rows.first()).copied()
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
    let Some(m) = machines(app).into_iter().find(|m| m.in_use) else { return Vec::new() };
    let Some(base) = workspaces::grouped(&m.rows).into_iter().map(|(base, _)| base).find(|base| {
        let open = workspaces::worktrees_of(&m.rows, base);
        open.iter().any(|w| w.id == active)
    }) else {
        return Vec::new();
    };
    workspaces::worktrees_of(&m.rows, &base)
        .iter()
        .map(|w| Row {
            address: m.address.clone(),
            machine: m.name.clone(),
            ssh: m.ssh,
            in_use: true,
            id: w.id.clone(),
            name: workspaces::branch_label(w),
            map: w.map_state(),
            nested: true,
            open: w.id == active,
        })
        .collect()
}

/// The `n`th workspace of the Workspace menu (1-based), for ⌃⌘`n`.
pub fn nth(app: &FigApp, n: usize) -> Option<Row> {
    n.checked_sub(1).and_then(|i| menu_rows(app).into_iter().nth(i))
}

/// Open the switcher; over a map, the sidebar it hangs from shows too.
pub fn open(app: &mut FigApp) {
    if !app.ui.home {
        app.ws.pane_open = true;
    }
    app.status.open();
}

/// ⌃⌘O: open the switcher, or close it when it is open.
pub fn toggle(app: &mut FigApp) {
    if app.status.switcher_open { app.status.close() } else { open(app) }
}

/// The open workspace's line, for "Still viewing …".
fn current_name(app: &FigApp) -> String {
    groups(app, "").into_iter().flat_map(|g| g.rows).rfind(|row| row.open).map(|row| row.name).unwrap_or_else(|| "this workspace".to_string())
}

/// Open `row`: with a map, switch the map in place (the switcher follows the switch); without
/// one, open the browser on it for Init. The open row just closes the switcher.
pub fn choose(app: &mut FigApp, ctx: &egui::Context, row: &Row) {
    if row.open {
        app.status.close();
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
        app.status.query.clear();
        app.ws.act(ctx, &row.id, false);
    } else {
        app.status.close();
        app.ws.select(&row.id);
        app.ws.worktree_init = row.nested;
        app.ui.home = true;
    }
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

/// What a click or a key asked for this frame.
enum Ask {
    Choose(Row),
    Close,
    Cancel,
    Retry,
    All,
    New,
    Machine,
}

/// The popover, hanging under the sidebar's `head`. Clicks on `head` toggle it, so they don't
/// count as outside.
pub fn show(app: &mut FigApp, ui: &mut Ui, head: Rect) {
    if !app.status.switcher_open {
        return;
    }
    let ctx = ui.ctx().clone();
    let s = Skin::of(&Tokens::get(&ctx));
    let groups = groups(app, &app.status.query.clone());
    let rows = flat(&groups);
    let phase = app.status.phase(rows.len());

    let (up, down, enter) = ctx.input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::Enter)));
    let mut ask = None;
    if phase == Phase::List && (up || down) {
        app.status.cursor = step(&rows, app.status.cursor.as_deref(), down);
    }
    if enter {
        ask = match phase {
            Phase::List => highlighted(&rows, app.status.cursor.as_deref()).cloned().map(Ask::Choose),
            Phase::Failed => Some(Ask::Retry),
            _ => None,
        };
    }

    // Height follows the content, up to the window's foot.
    let list_h: f32 = groups.iter().map(|g| GROUP_H + g.rows.len().max(1) as f32 * ROW_H + 8.0).sum::<f32>().max(ROW_H);
    let body_h = match phase {
        Phase::List => list_h,
        Phase::NoMatch => 64.0,
        Phase::Opening => 120.0,
        Phase::Failed => 150.0,
    };
    let top = head.bottom() + 6.0;
    let window = ctx.content_rect();
    let height = (12.0 + 30.0 + 14.0 + body_h + 12.0 + 44.0 + 28.0).min((window.bottom() - 36.0 - top).max(220.0));
    let card = Rect::from_min_size(pos2(head.left() + 1.0, top), vec2(CARD_W.min(window.width() - head.left() - 8.0), height));
    let hot = highlighted(&rows, app.status.cursor.as_deref()).map(Row::key);

    let shown = egui::Area::new(Id::new("workspace_switcher")).order(egui::Order::Foreground).fixed_pos(card.min).show(&ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(card.size(), Sense::click());
        let painter = ui.painter();
        painter.add(
            egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(28) }.as_shape(rect, CornerRadius::same(10)),
        );
        painter.rect_filled(rect, CornerRadius::same(10), s.panel);
        painter.rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, s.border), StrokeKind::Inside);

        // The search field.
        let field = Rect::from_min_size(rect.min + vec2(12.0, 12.0), vec2(rect.width() - 24.0, 30.0));
        ui.painter().rect_filled(field, CornerRadius::same(5), s.bg);
        ui.painter().rect_stroke(field, CornerRadius::same(5), Stroke::new(1.0, s.border), StrokeKind::Inside);
        let edit_rect = Rect::from_min_max(field.min + vec2(10.0, 7.0), field.max - vec2(10.0, 7.0));
        let edit = ui.put(
            edit_rect,
            egui::TextEdit::singleline(&mut app.status.query)
                .id(Id::new("workspace_switcher_search"))
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .font(regular(11.5))
                .text_color(s.text)
                .hint_text(egui::RichText::new("Switch workspace or worktree…").font(regular(11.5)).color(s.faint)),
        );
        if app.status.take_focus() {
            edit.request_focus();
        }
        if edit.changed() {
            app.status.cursor = None;
            app.status.failed = None;
        }

        let foot_rule = rect.max.y - 72.0;
        let body = Rect::from_min_max(pos2(rect.min.x + 8.0, field.max.y + 14.0), pos2(rect.max.x - 8.0, foot_rule - 8.0));
        let x = rect.min.x + 18.0;
        match phase {
            Phase::List => {
                ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
                    egui::ScrollArea::vertical().id_salt("workspace_switcher_rows").auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for group in &groups {
                            let (eyebrow, _) = ui.allocate_exact_size(vec2(ui.available_width(), GROUP_H), Sense::hover());
                            ui.painter().text(
                                pos2(eyebrow.min.x + 10.0, eyebrow.center().y),
                                Align2::LEFT_CENTER,
                                &group.title,
                                egui::FontId::monospace(8.5),
                                s.muted,
                            );
                            if group.rows.is_empty() {
                                let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
                                let note = if app.ws.busy.is_some() {
                                    "Listing workspaces…"
                                } else if app.ws.error.is_some() {
                                    "Could not reach the node"
                                } else {
                                    "No workspaces yet"
                                };
                                ui.painter().text(pos2(line.min.x + 10.0, line.center().y), Align2::LEFT_CENTER, note, regular(11.0), s.faint);
                            }
                            for row in &group.rows {
                                if row_ui(ui, &s, row, hot.as_deref() == Some(row.key().as_str())) {
                                    ask = Some(Ask::Choose(row.clone()));
                                }
                            }
                            ui.add_space(8.0);
                        }
                    });
                });
            }
            Phase::NoMatch => {
                ui.painter().text(pos2(x, body.min.y + 8.0), Align2::LEFT_TOP, "No matching workspace", regular(12.5), s.text);
                ui.painter().text(pos2(x, body.min.y + 30.0), Align2::LEFT_TOP, "Try a name, a branch or a folder.", regular(11.0), s.muted);
            }
            Phase::Opening => {
                let (to, from) = app.status.switching.as_ref().map(|sw| (sw.branch.clone(), sw.from.clone())).unwrap_or_default();
                ui.painter().text(pos2(x, body.min.y + 8.0), Align2::LEFT_TOP, format!("Opening {to}…"), regular(12.5), s.text);
                ui.painter().text(pos2(x, body.min.y + 32.0), Align2::LEFT_TOP, format!("Still viewing {from}."), regular(11.0), s.muted);
                let cancel = Rect::from_min_size(pos2(x, body.min.y + 62.0), vec2(110.0, 28.0));
                if workspaces::secondary_button(ui, &s, Id::new("workspace_switcher_cancel"), cancel, "Cancel switch", true) {
                    ask = Some(Ask::Cancel);
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
            Phase::Failed => {
                let (to, from, reason) =
                    app.status.failed.as_ref().map(|(sw, e)| (sw.branch.clone(), sw.from.clone(), e.clone())).unwrap_or_default();
                ui.painter().text(pos2(x, body.min.y + 8.0), Align2::LEFT_TOP, format!("Could not open {to}"), regular(12.5), s.danger);
                let reason = crate::widgets::elide_font(ui, &reason, body.width() - 20.0, egui::FontId::monospace(9.0));
                ui.painter().text(pos2(x, body.min.y + 32.0), Align2::LEFT_TOP, reason, egui::FontId::monospace(9.0), s.faint);
                ui.painter().text(pos2(x, body.min.y + 50.0), Align2::LEFT_TOP, format!("You are still on {from}."), regular(11.0), s.muted);
                let retry = Rect::from_min_size(pos2(x, body.min.y + 80.0), vec2(80.0, 28.0));
                if workspaces::secondary_button(ui, &s, Id::new("workspace_switcher_retry"), retry, "Retry", true) {
                    ask = Some(Ask::Retry);
                }
                let stay = Rect::from_min_size(pos2(x + 90.0, body.min.y + 80.0), vec2(80.0, 28.0));
                if workspaces::secondary_button(ui, &s, Id::new("workspace_switcher_stay"), stay, "Stay", true) {
                    ask = Some(Ask::Close);
                }
            }
        }

        // The rule, the three ways out, and the keys.
        ui.painter().line_segment([pos2(rect.min.x + 12.0, foot_rule), pos2(rect.max.x - 12.0, foot_rule)], Stroke::new(1.0, s.border));
        let foot_y = foot_rule + 6.0;
        let mut fx = rect.min.x + 8.0;
        for (label, id, plain) in [("All workspaces…", "all", true), ("+ New workspace", "new", false), ("+ Add machine", "machine", false)] {
            let w = ui.painter().layout_no_wrap(label.to_string(), regular(11.5), s.text).size().x + 20.0;
            let r = Rect::from_min_size(pos2(fx, foot_y), vec2(w, 28.0));
            let resp = ui.interact(r, Id::new(("workspace_switcher_foot", id)), Sense::click());
            crate::a11y::button(&resp, label.trim_start_matches("+ "));
            if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(5), s.soft);
            }
            workspaces::focus_ring(ui, &s, r, &resp, 5);
            let ink = if plain || resp.hovered() { s.text } else { s.muted };
            ui.painter().text(pos2(r.min.x + 10.0, r.center().y), Align2::LEFT_CENTER, label, regular(11.5), ink);
            if resp.clicked() {
                ask = Some(match id {
                    "all" => Ask::All,
                    "new" => Ask::New,
                    _ => Ask::Machine,
                });
            }
            fx = r.max.x + 4.0;
        }
        ui.painter().text(
            pos2(rect.min.x + 18.0, rect.max.y - 18.0),
            Align2::LEFT_CENTER,
            "↑↓ choose  ·  Enter open  ·  Esc close  ·  INIT opens Init",
            egui::FontId::monospace(8.0),
            s.faint,
        );
        rect
    });

    // A click anywhere else closes it (the head toggles it itself).
    let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !shown.inner.contains(p) && !head.contains(p)));
    if outside && phase != Phase::Opening && ask.is_none() {
        ask = Some(Ask::Close);
    }

    match ask {
        Some(Ask::Choose(row)) => choose(app, &ctx, &row),
        Some(Ask::Retry) => {
            if let Some((switch, _)) = app.status.failed.take() {
                app.status.switching = Some(switch.clone());
                app.ws.act(&ctx, &switch.id, false);
            }
        }
        Some(Ask::Cancel) => {
            app.ws.cancel_open();
            app.status.switching = None;
            app.status.close();
        }
        Some(Ask::Close) => app.status.close(),
        Some(Ask::All) => {
            app.status.close();
            crate::map_actions::run(app, &ctx, "map.openWorkspace");
        }
        Some(Ask::New) => {
            app.status.close();
            crate::map_actions::run(app, &ctx, "map.newWorkspace");
        }
        Some(Ask::Machine) => {
            app.status.close();
            crate::native_menu::run(app, crate::native_menu::ADD_MACHINE);
        }
        None => {}
    }
}

/// One row: the name (a nested worktree indented and quieter), the badge at the right.
fn row_ui(ui: &mut Ui, s: &Skin, row: &Row, hot: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let badge = row.badge();
    crate::a11y::button(&resp, &format!("{} · {}", row.name, if badge == Badge::Here { "open" } else { badge.text() }));
    if hot || resp.hovered() || (row.open && !row.nested) {
        ui.painter().rect_filled(rect, CornerRadius::same(5), s.selected);
    }
    workspaces::focus_ring(ui, s, rect, &resp, 5);
    let (x, font, ink) = if row.nested { (rect.min.x + 26.0, regular(11.5), s.muted) } else { (rect.min.x + 10.0, regular(12.5), s.text) };
    let badge_x = rect.max.x - 72.0;
    let name = crate::widgets::elide_font(ui, &row.name, badge_x - 12.0 - x, font.clone());
    ui.painter().text(pos2(x, rect.center().y), Align2::LEFT_CENTER, name, font, ink);
    let tint = match badge {
        Badge::Open | Badge::Here | Badge::Map | Badge::Empty => s.map_ink,
        Badge::Init => s.accent,
    };
    let mut bx = badge_x;
    if matches!(badge, Badge::Open | Badge::Here) {
        // The monospace face has no check glyph: the mark is drawn.
        workspaces::check_mark(ui, pos2(bx + 3.5, rect.center().y), tint);
        bx += 12.0;
    }
    ui.painter().text(pos2(bx, rect.center().y), Align2::LEFT_CENTER, badge.text(), egui::FontId::monospace(8.5), tint);
    resp.clicked()
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
        assert_eq!(titles, ["THIS COMPUTER", "BEFREE · SSH"]);
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
    fn arrows_start_on_the_open_row_and_wrap() {
        let groups = build(&machines(), Some("w3"), "");
        let rows = flat(&groups);
        let first = step(&rows, None, true);
        assert_eq!(first.as_deref(), Some(rows[0].key().as_str()), "accounting is open and first");
        let next = step(&rows, first.as_deref(), true);
        assert_eq!(next.as_deref(), Some(rows[1].key().as_str()));
        let back = step(&rows, first.as_deref(), false);
        assert_eq!(back.as_deref(), Some(rows[rows.len() - 1].key().as_str()));
        assert_eq!(step(&[], None, true), None);
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
    fn ctrl_cmd_o_opens_the_switcher_with_the_sidebar_and_escape_closes_it() {
        crate::with_map_mode(|| {
            let mut u = map_ui();
            u.app.ws.pane_open = false;
            press(&mut u, Key::O, ctrl_cmd());
            assert!(u.app.status.switcher_open);
            assert!(u.app.ws.pane_open, "the switcher hangs from the sidebar");
            press(&mut u, Key::Escape, Modifiers::NONE);
            assert!(!u.app.status.switcher_open);
            // ⌘O, Open workspace's old key, opens it too.
            press(&mut u, Key::O, cmd());
            assert!(u.app.status.switcher_open);
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
