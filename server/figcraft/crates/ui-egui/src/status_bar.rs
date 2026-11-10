//! The map's bottom status bar: the worktree and the open page's path at the left, the agents at
//! the right (design: `Sidebar pages · window`).
//!
//! Like an editor's status bar it is chrome, not canvas: it costs no header space. The worktree
//! names the open design (`repo / branch`); a click opens the palette on the workspaces. After
//! it, the workspace's agent count (`afterburn / main  ·  3 agents`, design: `Sync pane redesign`;
//! it lived in the sidebar's workspace switcher); a click lists the agents.
//! After it comes the open page's path (`Overview › Canvas and layers › Paint › Fills`): every
//! crumb before the last opens its page. At the right, the agent summary opens the palette on the
//! workspace's agents, beside the count of people on the map.
//!
//! The worktree comes from the same node listing the Workspaces screen uses: the active session's
//! [`crate::node::SessionRef`] says which worktree is open, and `workspaces::State::rows` names
//! its repository. The bar also keeps the switcher's state ([`State`]) and follows a switch in
//! flight, since it is drawn whether or not the sidebar is.

use egui::{Align2, Color32, CornerRadius, Id, Rect, Sense, Stroke, Ui, pos2, vec2};
use figcraft_doc::{Document, NodeId};

use crate::FigApp;
use crate::theme::{Tokens, regular};

#[cfg(not(target_arch = "wasm32"))]
use crate::node::SessionRef;
#[cfg(not(target_arch = "wasm32"))]
use crate::workspaces::Workspace;

/// The bar's height, in points. The design draws 22 pt; the owner asked for a sturdier bar, so it
/// keeps the design's type and spacing in a 28 pt band.
pub const HEIGHT: f32 = 28.0;

/// A workspace switch's state ([`crate::switcher`]): the palette shows it while it shows the
/// workspaces.
#[derive(Default)]
pub struct State {
    /// A switch in flight: the old map stays until the new one is ready.
    pub switching: Option<Switch>,
    /// A switch that failed, with the node's reason; the old map is still open.
    pub failed: Option<(Switch, String)>,
}

/// One requested switch: where to, and where the human still is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Switch {
    pub id: String,
    pub branch: String,
    /// The workspace still on screen while the other one opens.
    pub from: String,
}

/// The worktree's branch name, or its label when git has none.
#[cfg(not(target_arch = "wasm32"))]
fn branch_of(workspace: &Workspace) -> String {
    workspace.branch.clone().filter(|branch| !branch.is_empty()).unwrap_or_else(|| workspace.label.clone())
}

/// The repository's name: git's, else the workspace's own label.
#[cfg(not(target_arch = "wasm32"))]
fn repo_name(workspace: &Workspace) -> String {
    workspace
        .repo
        .clone()
        .filter(|repo| !repo.trim().is_empty())
        .or_else(|| Some(workspace.label.clone()).filter(|label| !label.trim().is_empty()))
        .unwrap_or_else(|| "worktree".to_string())
}

/// The row matching the open session, by workspace id then by checkout path.
#[cfg(not(target_arch = "wasm32"))]
fn active_row<'a>(active: Option<&SessionRef>, rows: &'a [Workspace]) -> Option<&'a Workspace> {
    let active = active?;
    rows.iter().find(|row| !row.id.is_empty() && row.id == active.workspace).or_else(|| {
        let path = active.path.trim_end_matches('/');
        (!path.is_empty()).then_some(()).and_then(|_| rows.iter().find(|row| row.path.as_deref().map(|p| p.trim_end_matches('/')) == Some(path)))
    })
}

/// The bar's worktree for a row the browser has selected but no design is open on yet.
#[cfg(not(target_arch = "wasm32"))]
pub fn selected_chip(rows: &[Workspace], selected: Option<&str>) -> Option<String> {
    let row = rows.iter().find(|row| Some(row.id.as_str()) == selected)?;
    Some(format!("{} / {}", repo_name(row), branch_of(row)))
}

/// The session a selected workspace stands in for.
#[cfg(not(target_arch = "wasm32"))]
fn selected_session(selected: Option<&str>) -> Option<SessionRef> {
    selected.filter(|id| !id.is_empty()).map(|id| SessionRef { workspace: id.to_string(), ..SessionRef::default() })
}

/// The worktree's text: `repo / branch`, or `branch`, or a hint when no design is open.
#[cfg(not(target_arch = "wasm32"))]
pub fn label(active: Option<&SessionRef>, rows: &[Workspace]) -> String {
    match active_row(active, rows) {
        Some(row) => format!("{} / {}", repo_name(row), branch_of(row)),
        None => match active {
            Some(target) => {
                let branch = target.branch.trim();
                if branch.is_empty() { "no worktree".to_string() } else { branch.to_string() }
            }
            None => "no design open".to_string(),
        },
    }
}

/// One crumb of the open page's path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crumb {
    pub page: NodeId,
    pub name: String,
    /// Every crumb before the last opens its page.
    pub link: bool,
}

/// The open page's path, root first: `Overview › Canvas and layers › Paint › Fills`.
pub fn crumbs(doc: &Document, page: NodeId) -> Vec<Crumb> {
    let path = crate::page_tree::path(doc, page);
    let last = path.len().saturating_sub(1);
    path.into_iter().enumerate().filter_map(|(i, id)| doc.node(id).map(|node| Crumb { page: id, name: node.name.clone(), link: i < last })).collect()
}

/// The bar's inks.
struct Ink {
    panel: Color32,
    border: Color32,
    text: Color32,
    faint: Color32,
    selected: Color32,
}

impl Ink {
    fn of(t: &Tokens) -> Ink {
        let theme = crate::map_colors::theme();
        if !matches!(theme, crate::map_colors::Theme::Classic | crate::map_colors::Theme::ClassicDark) {
            let c = theme.colors();
            return Ink { panel: c.panel, border: c.border, text: c.ink, faint: c.faint, selected: c.selected };
        }
        // Classic and Classic Dark keep the bar's own inks.
        if t.dark {
            Ink { panel: t.panel, border: t.border, text: t.text, faint: t.text_tertiary, selected: t.hover }
        } else {
            Ink {
                panel: Color32::WHITE,
                border: Color32::from_rgb(0xE3, 0xE2, 0xDF),
                text: Color32::from_rgb(0x1A, 0x1A, 0x1A),
                faint: Color32::from_rgb(0x96, 0x93, 0x8D),
                selected: Color32::from_rgb(0xF1, 0xF0, 0xED),
            }
        }
    }
}

/// Draw the bar. `ui` is the bottom panel's UI, `app` the whole map.
pub fn show(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let ink = Ink::of(&t);
    let bar = ui.max_rect();
    ui.painter().rect_filled(bar, CornerRadius::ZERO, ink.panel);
    ui.painter().line_segment([bar.left_top(), bar.right_top()], Stroke::new(1.0, ink.border));
    let home = app.ui.home || !app.has_files();

    #[cfg(not(target_arch = "wasm32"))]
    let chip = {
        let active = crate::workspaces::node_identity(app);
        if home {
            selected_chip(&app.ws.rows, app.ws.selected.as_deref())
                .unwrap_or_else(|| label(selected_session(app.ws.selected.as_deref()).or(active).as_ref(), &app.ws.rows))
        } else {
            label(active.as_ref(), &app.ws.rows)
        }
    };
    #[cfg(target_arch = "wasm32")]
    let chip = "no design open".to_string();

    // The worktree at the left: a click opens the workspace switcher.
    let font = regular(11.0);
    let chip_w = ui.painter().layout_no_wrap(chip.clone(), font.clone(), ink.text).size().x;
    let chip_rect = Rect::from_min_size(pos2(bar.min.x + 6.0, bar.min.y + 3.0), vec2(chip_w + 12.0, bar.height() - 5.0));
    let chip_resp = ui.interact(chip_rect, Id::new("status_worktree_chip"), Sense::click());
    crate::a11y::button(&chip_resp, "Switch workspace");
    if chip_resp.hovered() {
        ui.painter().rect_filled(chip_rect, CornerRadius::same(4), ink.selected);
    }
    ui.painter().text(pos2(bar.min.x + 12.0, bar.center().y + 0.5), Align2::LEFT_CENTER, &chip, font.clone(), ink.text);
    if chip_resp.on_hover_text("Switch workspace (⌃⌘O)").clicked() {
        crate::switcher::toggle(app);
    }
    #[allow(unused_mut)]
    let mut after_chip = chip_rect.max.x;
    #[cfg(not(target_arch = "wasm32"))]
    if !home && let Some(count) = agent_count(app) {
        after_chip = agents_after_chip(app, ui, &ink, bar, after_chip, &count);
    }

    if !home {
        // The right end first: the crumbs stop short of it.
        let mut right = bar.max.x - 12.0;
        let (summary, color) = crate::agents::summary(&app.compose.targets, app.compose.targets_error.as_deref());
        let summary_font = egui::FontId::monospace(9.5);
        let w = ui.painter().layout_no_wrap(summary.clone(), summary_font.clone(), color).size().x;
        let agents = Rect::from_min_max(pos2(right - w - 8.0, bar.min.y + 3.0), pos2(right + 4.0, bar.max.y - 2.0));
        let resp = ui.interact(agents, Id::new("status_agents"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        crate::a11y::button(&resp, "Agents");
        if resp.hovered() {
            ui.painter().rect_filled(agents, CornerRadius::same(4), ink.selected);
        }
        ui.painter().text(pos2(right - 2.0, bar.center().y + 0.5), Align2::RIGHT_CENTER, &summary, summary_font, color);
        if resp.on_hover_text("Agents").clicked() {
            crate::agents::open(app);
        }
        right = agents.min.x - 10.0;
        // The people on the map: small and quiet beside the agents.
        let (people, joined, reconnecting) = {
            let live = app.session.live();
            (
                live.peers.len() + 1,
                matches!(live.status, figcraft_engine::live::Status::Live),
                matches!(live.status, figcraft_engine::live::Status::Reconnecting { .. } | figcraft_engine::live::Status::Connecting),
            )
        };
        let live = if joined {
            format!("{people} live")
        } else if reconnecting {
            "Reconnecting…".to_string()
        } else {
            "Offline".to_string()
        };
        let live_font = egui::FontId::monospace(8.5);
        let live_w = ui.painter().layout_no_wrap(live.clone(), live_font.clone(), ink.faint).size().x;
        let live_rect = Rect::from_min_max(pos2(right - live_w - 5.0, bar.min.y + 3.0), pos2(right + 5.0, bar.max.y - 2.0));
        let response = ui.interact(live_rect, Id::new("status_connection"), if joined { Sense::hover() } else { Sense::click() });
        if !joined && response.hovered() {
            ui.painter().rect_filled(live_rect, CornerRadius::same(4), ink.selected);
        }
        ui.painter().text(pos2(right, bar.center().y + 0.5), Align2::RIGHT_CENTER, live, live_font, ink.faint);
        if !joined {
            crate::a11y::button(&response, "Reconnect to node");
            #[cfg(not(target_arch = "wasm32"))]
            {
                let reason = crate::workspaces::node_identity(app).and_then(|target| crate::node::recovery_error(&target));
                let tip = reason.map_or_else(
                    || "Click to reconnect now. Comments are kept and are not sent automatically.".to_string(),
                    |reason| format!("{reason}\nClick to retry now. Comments are kept and are not sent automatically."),
                );
                if response.on_hover_text(tip).clicked() {
                    crate::map_actions::run(app, ui.ctx(), "map.reconnect");
                }
            }
        }
        right -= live_w + 16.0;

        let path = app.session.doc().ok().zip(app.session.page().ok()).map(|(doc, page)| crumbs(doc, page)).unwrap_or_default();
        if let Some(page) = path_ui(ui, &ink, after_chip + 2.0, right, bar, &path) {
            app.run("page.select", serde_json::json!({"id": page.0}));
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    track_switch(app);
    // Over a map the creation modals (⇧⌘N, Add machine) draw here: the bar shows whether or not
    // the sidebar does. The Workspaces screen draws its own.
    if !home {
        crate::workspaces::map_modals(app, ui);
    }
}

/// The open workspace's agent count (`3 agents`), when the listing knows the workspace.
#[cfg(not(target_arch = "wasm32"))]
pub fn agent_count(app: &FigApp) -> Option<String> {
    let active = crate::workspaces::node_identity(app);
    active_row(active.as_ref(), &app.ws.rows).map(crate::workspaces::pane_agents)
}

/// `  ·  3 agents` after the worktree from `x`; a click lists the agents. Returns where it ends.
#[cfg(not(target_arch = "wasm32"))]
fn agents_after_chip(app: &mut FigApp, ui: &mut Ui, ink: &Ink, bar: Rect, x: f32, count: &str) -> f32 {
    let font = regular(11.0);
    let y = bar.center().y + 0.5;
    ui.painter().text(pos2(x + 4.0, y), Align2::LEFT_CENTER, "·", font.clone(), ink.faint);
    let w = ui.painter().layout_no_wrap(count.to_string(), font.clone(), ink.text).size().x;
    let rect = Rect::from_min_size(pos2(x + 10.0, bar.min.y + 3.0), vec2(w + 12.0, bar.height() - 5.0));
    let resp = ui.interact(rect, Id::new("status_agent_count"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    crate::a11y::button(&resp, "Agents");
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), ink.selected);
    }
    ui.painter().text(pos2(rect.min.x + 6.0, y), Align2::LEFT_CENTER, count, font, ink.text);
    if resp.on_hover_text("Agents").clicked() {
        crate::map_palette::open(app, crate::map_palette::Scope::Agents);
    }
    rect.max.x
}

/// ` · Overview › Canvas and layers › Fills` from `x`, up to `right`; returns a clicked crumb.
fn path_ui(ui: &Ui, ink: &Ink, x: f32, right: f32, bar: Rect, path: &[Crumb]) -> Option<NodeId> {
    if path.is_empty() {
        return None;
    }
    let font = regular(11.0);
    let y = bar.center().y + 0.5;
    ui.painter().text(pos2(x + 4.0, y), Align2::LEFT_CENTER, "·", font.clone(), ink.faint);
    let mut x = x + 16.0;
    let mut open = None;
    for (i, crumb) in path.iter().enumerate() {
        if i > 0 {
            ui.painter().text(pos2(x, y), Align2::LEFT_CENTER, "›", font.clone(), ink.faint);
            x += 12.0;
        }
        let room = right - x;
        if room < 24.0 {
            break;
        }
        let name = crate::widgets::elide_font(ui, &crumb.name, room, font.clone());
        let w = ui.painter().layout_no_wrap(name.clone(), font.clone(), ink.text).size().x;
        let rect = Rect::from_min_size(pos2(x, bar.min.y + 4.0), vec2(w, bar.height() - 7.0));
        ui.painter().text(pos2(x, y), Align2::LEFT_CENTER, &name, font.clone(), ink.text);
        if crumb.link {
            let resp = ui.interact(rect, Id::new(("status_crumb", crumb.page.0)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            let tip = format!("Open {}", crumb.name);
            crate::a11y::button(&resp, &tip);
            if resp.hovered() {
                let line = y + 7.0;
                ui.painter().line_segment([pos2(rect.left(), line), pos2(rect.right(), line)], Stroke::new(1.0, ink.text));
            }
            if resp.on_hover_text(tip).clicked() {
                open = Some(crumb.page);
            }
        }
        x = rect.right() + 6.0;
    }
    open
}

/// Follow a switch in flight: done when the other workspace is the open one, failed when the
/// Workspaces state reports an error, still opening while a request or a join is pending.
#[cfg(not(target_arch = "wasm32"))]
fn track_switch(app: &mut FigApp) {
    let Some(switch) = app.status.switching.clone() else { return };
    if app.active_workspace.as_deref() == Some(switch.id.as_str()) {
        app.status.switching = None;
        crate::switcher::close(app);
        return;
    }
    if app.ws.busy.is_some() || app.ws.join.is_some() {
        return;
    }
    app.status.switching = None;
    match app.ws.error.clone() {
        Some(error) => {
            app.status.failed = Some((switch, error));
            if !crate::switcher::is_open(app) {
                crate::map_palette::open(app, crate::map_palette::Scope::Workspace);
            }
        }
        None => crate::switcher::close(app),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    fn ws(id: &str, label: &str, repo: &str, path: &str, branch: Option<&str>, map: &str) -> Workspace {
        Workspace {
            id: id.to_string(),
            label: label.to_string(),
            repo: Some(repo.to_string()),
            path: Some(path.to_string()),
            branch: branch.map(|b| b.to_string()),
            map: map.to_string(),
            ..Workspace::default()
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn target(workspace: &str, path: &str, branch: &str) -> SessionRef {
        SessionRef { address: "127.0.0.1:47822".into(), session: "s".into(), workspace: workspace.into(), path: path.into(), branch: branch.into() }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_bar_names_the_repository_and_branch() {
        let rows = vec![ws("w1", "afterburn", "afterburn", "/root/code/mindmap", Some("main"), "map")];
        assert_eq!(label(Some(&target("w1", "/root/code/mindmap", "main")), &rows), "afterburn / main");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_bar_falls_back_to_the_branch_alone() {
        assert_eq!(label(Some(&target("w9", "/gone", "feature/x")), &[]), "feature/x");
        assert_eq!(label(None, &[]), "no design open");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_selected_workspace_names_the_bar_before_any_map_is_open() {
        let rows = vec![ws("w2", "branchy", "afterburn", "/wt/branchy", Some("feature/x"), "empty")];
        assert_eq!(selected_chip(&rows, Some("w2")).as_deref(), Some("afterburn / feature/x"));
        assert_eq!(selected_chip(&rows, None), None);
        assert_eq!(selected_chip(&rows, Some("missing")), None);
    }

    #[test]
    fn the_path_links_every_crumb_but_the_open_page() {
        let mut doc = Document::new("Map");
        let overview = doc.pages[0];
        let sub = |doc: &mut Document, parent: NodeId, name: &str| {
            let page = doc.add_page(name);
            doc.node_mut(page).unwrap().plugin_data.insert(crate::page_tree::PARENT_KEY.into(), parent.0.to_string());
            page
        };
        let canvas = sub(&mut doc, overview, "Canvas and layers");
        let paint = sub(&mut doc, canvas, "Paint");
        let fills = sub(&mut doc, paint, "Fills");
        let path = crumbs(&doc, fills);
        let names: Vec<&str> = path.iter().map(|c| c.name.as_str()).collect();
        let first = doc.node(overview).unwrap().name.clone();
        assert_eq!(names, [first.as_str(), "Canvas and layers", "Paint", "Fills"]);
        assert_eq!(path.iter().map(|c| c.link).collect::<Vec<_>>(), [true, true, true, false]);
        assert_eq!(path[1].page, canvas);
        let root = crumbs(&doc, overview);
        assert_eq!(root.len(), 1);
        assert!(!root[0].link, "a root page is the whole path, and no link");
    }
}
