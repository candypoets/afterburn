//! The map's bottom status bar: the worktree at the left, Page tree at the right.
//!
//! Like an editor's status bar it is chrome, not canvas: it costs no header space. The worktree
//! chip names the open design (`repo / branch`) and opens the switcher upward, listing the other
//! worktrees of the same repository. `Page tree` opens the page tree popover.
//!
//! The data comes from the same node listing the Workspaces screen uses: the active session's
//! [`crate::node::SessionRef`] says which worktree is open, and `workspaces::State::rows` lists
//! its siblings. The browser build has no node session, so it shows the bar without a switcher.

use egui::{Align, Color32, CornerRadius, Id, Layout, Stroke, Ui};

use crate::FigApp;
use crate::theme::Tokens;

#[cfg(not(target_arch = "wasm32"))]
use crate::node::SessionRef;
#[cfg(not(target_arch = "wasm32"))]
use crate::workspaces::Workspace;

/// The bar's height, in points.
pub const HEIGHT: f32 = 22.0;

/// The status bar's own UI state.
#[derive(Default)]
pub struct State {
    /// The worktree switcher is open above the bar.
    pub switcher_open: bool,
}

/// One worktree the switcher lists.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The workspace id to open (`ws.act`).
    pub id: String,
    /// The branch name, or the worktree's short name.
    pub branch: String,
    /// The checkout path.
    pub path: String,
    /// `map`, `empty` or `none`, as the node reports it.
    pub map: String,
    /// The design this bar belongs to is this worktree.
    pub current: bool,
}

/// The repository a workspace belongs to: its git repo or its own path.
#[cfg(not(target_arch = "wasm32"))]
fn repo_key(workspace: &Workspace) -> String {
    workspace.repo.clone().or_else(|| workspace.path.clone()).unwrap_or_default()
}

/// The worktree's branch name, or its label when git has none.
#[cfg(not(target_arch = "wasm32"))]
fn branch_of(workspace: &Workspace) -> String {
    workspace.branch.clone().filter(|branch| !branch.is_empty()).unwrap_or_else(|| workspace.label.clone())
}

/// The row matching the open session, by workspace id then by checkout path.
#[cfg(not(target_arch = "wasm32"))]
fn active_row<'a>(active: Option<&SessionRef>, rows: &'a [Workspace]) -> Option<&'a Workspace> {
    let active = active?;
    rows.iter().find(|row| !row.id.is_empty() && row.id == active.workspace).or_else(|| {
        let path = active.path.trim_end_matches('/');
        rows.iter().find(|row| row.path.as_deref().map(|p| p.trim_end_matches('/')) == Some(path))
    })
}

/// The chip for a row the browser has selected but no design is open on yet.
#[cfg(not(target_arch = "wasm32"))]
pub fn selected_chip(rows: &[Workspace], selected: Option<&str>) -> Option<String> {
    let row = rows.iter().find(|row| Some(row.id.as_str()) == selected)?;
    let repo = row.repo.clone().filter(|repo| !repo.is_empty()).unwrap_or_else(|| "worktree".to_string());
    Some(format!("{repo} / {}", branch_of(row)))
}

/// The session a selected workspace stands in for, so the switcher can key it by repository.
#[cfg(not(target_arch = "wasm32"))]
fn selected_session(selected: Option<&str>) -> Option<SessionRef> {
    selected.filter(|id| !id.is_empty()).map(|id| SessionRef { workspace: id.to_string(), ..SessionRef::default() })
}

/// The chip's text: `repo / branch`, or `branch`, or a hint when no design is open.
#[cfg(not(target_arch = "wasm32"))]
pub fn label(active: Option<&SessionRef>, rows: &[Workspace]) -> String {
    match active_row(active, rows) {
        Some(row) => {
            let repo = row.repo.clone().filter(|repo| !repo.is_empty()).unwrap_or_else(|| "worktree".to_string());
            format!("{repo} / {}", branch_of(row))
        }
        None => match active {
            Some(target) => {
                let branch = target.branch.trim();
                if branch.is_empty() { "no worktree".to_string() } else { branch.to_string() }
            }
            None => "no design open".to_string(),
        },
    }
}

/// The switcher's rows: the worktrees of the open design's repository, the current one first.
#[cfg(not(target_arch = "wasm32"))]
pub fn switcher_rows(active: Option<&SessionRef>, rows: &[Workspace]) -> Vec<Row> {
    let Some(current) = active_row(active, rows) else {
        return Vec::new();
    };
    let key = repo_key(current);
    let mut out: Vec<Row> = rows
        .iter()
        .filter(|row| !row.id.is_empty())
        .filter(|row| repo_key(row) == key)
        .map(|row| Row {
            id: row.id.clone(),
            branch: branch_of(row),
            path: row.path.clone().unwrap_or_default(),
            map: if row.map.is_empty() { "none".to_string() } else { row.map.clone() },
            current: row.id == current.id,
        })
        .collect();
    out.sort_by(|a, b| b.current.cmp(&a.current).then_with(|| a.branch.cmp(&b.branch)));
    out
}

/// The badge for a worktree's map state.
#[cfg(not(target_arch = "wasm32"))]
fn badge(map: &str) -> (&'static str, Color32) {
    match map {
        "map" => ("MAP", Color32::from_rgb(0x2F, 0x6F, 0x6A)),
        "empty" => ("INIT", Color32::from_rgb(0xB5, 0x45, 0x2F)),
        _ => ("NO MAP", Color32::from_rgb(0x9A, 0x97, 0x91)),
    }
}

/// Draw the bar. `ui` is the bottom panel's UI, `app` the whole map.
pub fn show(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let bar = ui.max_rect();
    ui.painter().line_segment([bar.left_top(), bar.right_top()], Stroke::new(1.0, t.border));

    let mut toggle = false;
    let mut open_tree = false;

    #[cfg(not(target_arch = "wasm32"))]
    let active = crate::workspaces::active_node(app).ok();
    #[cfg(not(target_arch = "wasm32"))]
    let selected = selected_session(app.ws.selected.as_deref());
    #[cfg(not(target_arch = "wasm32"))]
    let key = active.clone().or(selected);
    #[cfg(not(target_arch = "wasm32"))]
    let rows = switcher_rows(key.as_ref(), &app.ws.rows);
    #[cfg(not(target_arch = "wasm32"))]
    let chip = selected_chip(&app.ws.rows, app.ws.selected.as_deref()).unwrap_or_else(|| label(key.as_ref(), &app.ws.rows));
    #[cfg(target_arch = "wasm32")]
    let chip = "no design open".to_string();

    ui.horizontal_centered(|ui| {
        ui.add_space(12.0);
        let response = ui.add(egui::Button::new(egui::RichText::new(chip).size(10.0).color(t.text)).frame(false));
        #[cfg(not(target_arch = "wasm32"))]
        if response.on_hover_text("Switch worktree").clicked() {
            toggle = true;
        }
        if app.status.switcher_open {
            ui.label(egui::RichText::new("▴").size(8.5).color(t.text_tertiary));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(12.0);
            let response = ui.add(egui::Button::new(egui::RichText::new("Page tree  ⌄").size(9.5).color(t.text_secondary)).frame(false));
            crate::a11y::button(&response, "Page tree");
            if response.on_hover_text("All pages and subpages (⌘⇧P)").clicked() {
                open_tree = true;
            }
        });
    });

    if toggle {
        app.status.switcher_open = !app.status.switcher_open;
    }
    if open_tree {
        app.page_tree.open = true;
        app.page_tree.just_opened = true;
        app.status.switcher_open = false;
    }

    #[cfg(not(target_arch = "wasm32"))]
    if app.status.switcher_open {
        // Above the bar, anchored to its left: an upward popover, as the design draws it.
        let width = 300.0;
        let height = 34.0 + rows.len().max(1) as f32 * 34.0;
        let pos = egui::pos2(bar.left() + 8.0, bar.top() - height - 6.0);
        let mut clicked: Option<String> = None;
        let frame = egui::Frame::NONE.fill(t.menu).stroke(Stroke::new(1.0, t.border)).corner_radius(CornerRadius::same(8)).inner_margin(8);
        egui::Area::new(Id::new("worktree_switcher")).order(egui::Order::Foreground).fixed_pos(pos).show(ui.ctx(), |ui| {
            frame.show(ui, |ui| {
                ui.set_width(width);
                ui.label(egui::RichText::new("Switch worktree").size(9.5).strong().color(t.text));
                ui.add_space(4.0);
                if rows.is_empty() {
                    ui.label(egui::RichText::new("No worktree listed for this repository yet.").size(9.0).color(t.text_tertiary));
                }
                for row in &rows {
                    let inner = ui.horizontal(|ui| {
                        let text = if row.current { format!("{}  ·  open", row.branch) } else { row.branch.clone() };
                        let name = ui.add(
                            egui::Button::new(egui::RichText::new(text).size(10.0).color(if row.current { t.text } else { t.text_secondary }))
                                .frame(false),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let (badge_text, color) = badge(&row.map);
                            ui.label(egui::RichText::new(badge_text).size(8.0).color(color));
                        });
                        if !row.current {
                            ui.label(egui::RichText::new(&row.path).size(8.0).color(t.text_tertiary));
                        }
                        name
                    });
                    if inner.inner.clicked() && !row.current {
                        clicked = Some(row.id.clone());
                    }
                }
                ui.add_space(2.0);
                ui.label(egui::RichText::new("Esc to close").size(8.0).color(t.text_tertiary));
            });
        });
        if let Some(id) = clicked {
            app.ws.act(ui.ctx(), &id, false);
            app.status.switcher_open = false;
        } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            app.status.switcher_open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn target(workspace: &str, path: &str, branch: &str) -> SessionRef {
        SessionRef { address: "127.0.0.1:47822".into(), session: "s".into(), workspace: workspace.into(), path: path.into(), branch: branch.into() }
    }

    #[test]
    fn the_chip_names_the_repository_and_branch() {
        let rows = vec![ws("w1", "mindmap", "mindmap", "/root/code/mindmap", Some("main"), "map")];
        assert_eq!(label(Some(&target("w1", "/root/code/mindmap", "main")), &rows), "mindmap / main");
    }

    #[test]
    fn the_chip_falls_back_to_the_branch_alone() {
        assert_eq!(label(Some(&target("w9", "/gone", "feature/x")), &[]), "feature/x");
        assert_eq!(label(None, &[]), "no design open");
    }

    #[test]
    fn the_switcher_lists_only_the_same_repository_current_first() {
        let rows = vec![
            ws("w1", "mindmap", "mindmap", "/root/code/mindmap", Some("main"), "map"),
            ws("w2", "branchy", "mindmap", "/root/code/mindmap-wt/branchy", Some("feature/branchy"), "empty"),
            ws("w3", "other", "other", "/root/code/other", Some("main"), "map"),
        ];
        let listed = switcher_rows(Some(&target("w1", "/root/code/mindmap", "main")), &rows);
        assert_eq!(listed.len(), 2);
        assert!(listed[0].current);
        assert_eq!(listed[0].branch, "main");
        assert_eq!(listed[1].branch, "feature/branchy");
        assert_eq!(listed[1].map, "empty");
    }

    #[test]
    fn a_selected_workspace_names_the_chip_before_any_map_is_open() {
        let rows = vec![ws("w2", "branchy", "mindmap", "/wt/branchy", Some("feature/x"), "empty")];
        assert_eq!(selected_chip(&rows, Some("w2")).as_deref(), Some("mindmap / feature/x"));
        assert_eq!(selected_chip(&rows, None), None);
        assert_eq!(selected_chip(&rows, Some("missing")), None);
    }

    #[test]
    fn an_unknown_session_lists_nothing() {
        let rows = vec![ws("w1", "mindmap", "mindmap", "/root/code/mindmap", Some("main"), "map")];
        assert!(switcher_rows(Some(&target("nope", "/nope", "main")), &rows).is_empty());
        assert!(switcher_rows(None, &rows).is_empty());
    }
}
