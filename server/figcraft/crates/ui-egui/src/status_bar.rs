//! The map's bottom status bar: the worktree at the left, Page tree at the right.
//!
//! Like an editor's status bar it is chrome, not canvas: it costs no header space. The worktree
//! chip names the open design (`repo / branch`) and opens the Switch worktree card upward, which
//! lists every worktree of the same repository on this machine. `Page tree` opens the page tree.
//!
//! The data comes from the same node listing the Workspaces screen uses: the active session's
//! [`crate::node::SessionRef`] says which worktree is open, and `workspaces::State::rows` lists
//! its siblings. The browser build has no node session, so it shows the bar without a switcher.

use egui::{Align2, Color32, CornerRadius, Id, Rect, Sense, Stroke, Ui, pos2, vec2};
#[cfg(not(target_arch = "wasm32"))]
use egui::StrokeKind;

use crate::FigApp;
use crate::theme::{Tokens, regular, semibold};

#[cfg(not(target_arch = "wasm32"))]
use crate::node::SessionRef;
#[cfg(not(target_arch = "wasm32"))]
use crate::workspaces::Workspace;

/// The bar's height, in points. The design draws 22 pt; the owner asked for a sturdier bar, so it
/// keeps the design's type and spacing in a 28 pt band.
pub const HEIGHT: f32 = 28.0;

/// The Switch worktree card's width and its gap above the bar, as designed.
#[cfg(not(target_arch = "wasm32"))]
const CARD_W: f32 = 420.0;
#[cfg(not(target_arch = "wasm32"))]
const ROW_H: f32 = 64.0;
/// At most this many worktree rows show before the list scrolls.
#[cfg(not(target_arch = "wasm32"))]
const MAX_ROWS: usize = 5;

/// The status bar's own UI state.
#[derive(Default)]
pub struct State {
    /// The Switch worktree card is open above the bar.
    pub switcher_open: bool,
    /// What the search field holds.
    pub query: String,
    /// The highlighted row (↑ ↓ move it), by workspace id.
    pub cursor: Option<String>,
    /// A switch in flight: the old map stays until the new one is ready.
    pub switching: Option<Switch>,
    /// A switch that failed, with the node's reason; the old map is still open.
    pub failed: Option<(Switch, String)>,
    /// The search field takes the keyboard on the frame the card opens.
    focus_search: bool,
}

/// One requested switch: where to, and where the human still is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Switch {
    pub id: String,
    pub branch: String,
    /// The branch still on screen while the other one opens.
    pub from: String,
}

/// What the card shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The worktrees, filtered by the search.
    List,
    /// The search matches nothing.
    NoMatch,
    /// Another worktree is opening.
    Opening,
    /// The switch failed; the current map stays.
    Failed,
}

impl State {
    /// Open the card on the current worktree with an empty search.
    pub fn open(&mut self) {
        self.switcher_open = true;
        self.query.clear();
        self.cursor = None;
        self.failed = None;
        self.focus_search = true;
    }

    /// Close the card; a switch in flight keeps going (Cancel switch is the way to stop it).
    pub fn close(&mut self) {
        self.switcher_open = false;
        self.failed = None;
    }

    /// The card's state, given how many rows the search leaves.
    pub fn phase(&self, matches: usize) -> Phase {
        if self.switching.is_some() {
            Phase::Opening
        } else if self.failed.is_some() {
            Phase::Failed
        } else if matches == 0 && !self.query.trim().is_empty() {
            Phase::NoMatch
        } else {
            Phase::List
        }
    }
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
    /// A linked worktree rather than the repository's own checkout.
    pub linked: bool,
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

/// The chip for a row the browser has selected but no design is open on yet.
#[cfg(not(target_arch = "wasm32"))]
pub fn selected_chip(rows: &[Workspace], selected: Option<&str>) -> Option<String> {
    let row = rows.iter().find(|row| Some(row.id.as_str()) == selected)?;
    Some(format!("{} / {}", repo_name(row), branch_of(row)))
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

/// The switcher's rows: the worktrees of the open design's repository, the main checkout first,
/// then the linked worktrees by branch.
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
            linked: row.worktree,
        })
        .collect();
    out.sort_by(|a, b| a.linked.cmp(&b.linked).then_with(|| a.branch.cmp(&b.branch)));
    out
}

/// The rows whose branch or path contains the search, ignoring case.
#[cfg(not(target_arch = "wasm32"))]
pub fn filter_rows(rows: &[Row], query: &str) -> Vec<Row> {
    let query = query.trim().to_lowercase();
    rows.iter()
        .filter(|row| query.is_empty() || row.branch.to_lowercase().contains(&query) || row.path.to_lowercase().contains(&query))
        .cloned()
        .collect()
}

/// The row ↑ or ↓ lands on, wrapping at the ends; with nothing highlighted, the current row.
#[cfg(not(target_arch = "wasm32"))]
pub fn step(rows: &[Row], cursor: Option<&str>, down: bool) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    let at = cursor.and_then(|id| rows.iter().position(|row| row.id == id));
    let next = match at {
        None => rows.iter().position(|row| row.current).unwrap_or(0),
        Some(i) if down => (i + 1) % rows.len(),
        Some(i) => (i + rows.len() - 1) % rows.len(),
    };
    rows.get(next).map(|row| row.id.clone())
}

/// The highlighted row: the cursor while it is listed, else the current worktree, else the first.
#[cfg(not(target_arch = "wasm32"))]
fn highlighted<'a>(rows: &'a [Row], cursor: Option<&str>) -> Option<&'a Row> {
    cursor.and_then(|id| rows.iter().find(|row| row.id == id)).or_else(|| rows.iter().find(|row| row.current)).or_else(|| rows.first())
}

/// The design's palette for the bar and the card.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
struct Ink {
    panel: Color32,
    border: Color32,
    text: Color32,
    muted: Color32,
    faint: Color32,
    field: Color32,
    selected: Color32,
    teal: Color32,
    accent: Color32,
}

impl Ink {
    fn of(t: &Tokens) -> Ink {
        if t.dark {
            Ink {
                panel: t.panel,
                border: t.border,
                text: t.text,
                muted: t.text_secondary,
                faint: t.text_tertiary,
                field: t.canvas,
                selected: t.hover,
                teal: Color32::from_rgb(0x7F, 0xBD, 0xB5),
                accent: Color32::from_rgb(0xD9, 0x7A, 0x5F),
            }
        } else {
            Ink {
                panel: Color32::WHITE,
                border: Color32::from_rgb(0xE3, 0xE2, 0xDF),
                text: Color32::from_rgb(0x1A, 0x1A, 0x1A),
                muted: Color32::from_rgb(0x6E, 0x6C, 0x68),
                faint: Color32::from_rgb(0x96, 0x93, 0x8D),
                field: Color32::from_rgb(0xF4, 0xF4, 0xF2),
                selected: Color32::from_rgb(0xF1, 0xF0, 0xED),
                teal: Color32::from_rgb(0x2F, 0x6F, 0x6A),
                accent: Color32::from_rgb(0xB5, 0x45, 0x2F),
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
    let active = crate::workspaces::active_node(app).ok();
    // On the Workspaces screen the bar names the selected row; over a map, the open worktree.
    #[cfg(not(target_arch = "wasm32"))]
    let key = if home { selected_session(app.ws.selected.as_deref()).or(active.clone()) } else { active.clone() };
    #[cfg(not(target_arch = "wasm32"))]
    let chip = if home {
        selected_chip(&app.ws.rows, app.ws.selected.as_deref()).unwrap_or_else(|| label(key.as_ref(), &app.ws.rows))
    } else {
        label(key.as_ref(), &app.ws.rows)
    };
    #[cfg(target_arch = "wasm32")]
    let chip = "no design open".to_string();

    // The worktree, Inter Medium at the left; it opens Switch worktree.
    let chip_font = semibold(11.0);
    let chip_w = ui.painter().layout_no_wrap(chip.clone(), chip_font.clone(), ink.text).size().x;
    let chip_rect = Rect::from_min_size(pos2(bar.min.x + 6.0, bar.min.y + 3.0), vec2(chip_w + 12.0, bar.height() - 5.0));
    let chip_resp = ui.interact(chip_rect, Id::new("status_worktree_chip"), Sense::click());
    crate::a11y::button(&chip_resp, "Switch worktree");
    if chip_resp.hovered() || app.status.switcher_open {
        ui.painter().rect_filled(chip_rect, CornerRadius::same(4), ink.selected);
    }
    ui.painter().text(pos2(bar.min.x + 12.0, bar.center().y + 0.5), Align2::LEFT_CENTER, &chip, chip_font, ink.text);
    let chip_clicked = chip_resp.on_hover_text("Switch worktree").clicked();

    // Page tree at the right, only over a map.
    if !home {
        let label = "Page tree";
        let font = regular(10.5);
        let w = ui.painter().layout_no_wrap(label.to_string(), font.clone(), ink.muted).size().x + 18.0;
        let rect = Rect::from_min_size(pos2(bar.max.x - 12.0 - w - 6.0, bar.min.y + 3.0), vec2(w + 12.0, bar.height() - 5.0));
        let resp = ui.interact(rect, Id::new("status_page_tree"), Sense::click());
        crate::a11y::button(&resp, "Page tree");
        if resp.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(4), ink.selected);
        }
        ui.painter().text(
            pos2(rect.min.x + 6.0, bar.center().y + 0.5),
            Align2::LEFT_CENTER,
            label,
            font,
            if resp.hovered() { ink.text } else { ink.muted },
        );
        chevron(ui, pos2(rect.max.x - 10.0, bar.center().y + 0.5), ink.muted);
        if resp.on_hover_text("All pages and subpages (⌘⇧P)").clicked() {
            app.page_tree.open = true;
            app.page_tree.just_opened = true;
            app.status.close();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        if chip_clicked {
            if app.status.switcher_open {
                app.status.close();
            } else {
                app.status.open();
            }
        }
        track_switch(app);
        if app.status.switcher_open {
            let rows = switcher_rows(key.as_ref(), &app.ws.rows);
            switcher(app, ui, &ink, bar, chip_rect, &rows, key.as_ref());
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = chip_clicked;
}

/// A small downward chevron, drawn.
fn chevron(ui: &Ui, c: egui::Pos2, color: Color32) {
    let stroke = Stroke::new(1.1, color);
    ui.painter().line_segment([pos2(c.x - 3.0, c.y - 1.5), pos2(c.x, c.y + 1.5)], stroke);
    ui.painter().line_segment([pos2(c.x, c.y + 1.5), pos2(c.x + 3.0, c.y - 1.5)], stroke);
}

/// Follow a switch in flight: done when the other worktree is the open one, failed when the
/// Workspaces state reports an error, still opening while a request or a join is pending.
#[cfg(not(target_arch = "wasm32"))]
fn track_switch(app: &mut FigApp) {
    let Some(switch) = app.status.switching.clone() else { return };
    if app.active_workspace.as_deref() == Some(switch.id.as_str()) {
        app.status.switching = None;
        app.status.close();
        return;
    }
    if app.ws.busy.is_some() || app.ws.join.is_some() {
        return;
    }
    app.status.switching = None;
    match app.ws.error.clone() {
        Some(error) => {
            app.status.failed = Some((switch, error));
            app.status.switcher_open = true;
        }
        None => app.status.close(),
    }
}

/// What a click or a key asked the card to do this frame.
#[cfg(not(target_arch = "wasm32"))]
enum Ask {
    Open(Row),
    Close,
    Clear,
    Cancel,
    Retry,
}

/// The Switch worktree card, anchored above the bar's left end.
#[cfg(not(target_arch = "wasm32"))]
fn switcher(app: &mut FigApp, ui: &mut Ui, ink: &Ink, bar: Rect, chip: Rect, all: &[Row], key: Option<&SessionRef>) {
    let ctx = ui.ctx().clone();
    let rows = filter_rows(all, &app.status.query);
    let phase = app.status.phase(rows.len());
    let current = all.iter().find(|row| row.current).map(|row| row.branch.clone()).unwrap_or_else(|| "this worktree".to_string());
    let repo = app
        .ws
        .rows
        .iter()
        .find(|w| all.iter().any(|row| row.current && row.id == w.id))
        .map(repo_name)
        .unwrap_or_else(|| key.map(|k| k.branch.clone()).filter(|b| !b.is_empty()).unwrap_or_else(|| "this project".to_string()));
    let machine = app.ws.hosts.selected.as_ref().map(|host| host.name.clone()).unwrap_or_else(|| "This computer".to_string());
    let any_without_map = rows.iter().any(|row| row.map != "map");

    // Height follows the content: title, context, field, then the phase's body and the footer.
    let list_h = rows.len().clamp(1, MAX_ROWS) as f32 * (ROW_H + 8.0);
    let body_h = match phase {
        Phase::List => list_h + if any_without_map { 28.0 } else { 0.0 } + 4.0,
        Phase::NoMatch => 164.0,
        Phase::Opening => 196.0,
        Phase::Failed => 176.0 + 30.0,
    };
    let height = 104.0 + body_h + 32.0;
    let window_h = ctx.content_rect().height();
    let height = height.min((bar.top() - 8.0).max(200.0)).min(window_h);
    let card = Rect::from_min_size(pos2(bar.min.x + 12.0, bar.top() - 4.0 - height), vec2(CARD_W, height));

    // Keys: ↑ ↓ choose, Enter opens (or retries), Esc closes (or cancels, or stays).
    let (up, down, enter, esc) = ctx.input(|i| {
        (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape))
    });
    let mut ask: Option<Ask> = None;
    if phase == Phase::List && (up || down) {
        app.status.cursor = step(&rows, app.status.cursor.as_deref(), down);
    }
    if esc {
        ask = Some(match phase {
            Phase::Opening => Ask::Cancel,
            _ => Ask::Close,
        });
    } else if enter {
        ask = match phase {
            Phase::List => highlighted(&rows, app.status.cursor.as_deref()).cloned().map(Ask::Open),
            Phase::Failed => Some(Ask::Retry),
            _ => None,
        };
    }

    let shown = egui::Area::new(Id::new("worktree_switcher")).order(egui::Order::Foreground).fixed_pos(card.min).show(&ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(card.size(), Sense::click());
        let painter = ui.painter();
        painter.add(
            egui::epaint::Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha(24) }.as_shape(rect, CornerRadius::same(8)),
        );
        painter.rect_filled(rect, CornerRadius::same(8), ink.panel);
        painter.rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0, ink.border), StrokeKind::Inside);
        let x = rect.min.x + 16.0;
        let inner_w = rect.width() - 32.0;
        painter.text(pos2(x, rect.min.y + 14.0), Align2::LEFT_TOP, "Switch worktree", regular(14.0), ink.text);
        painter.text(pos2(x, rect.min.y + 38.0), Align2::LEFT_TOP, format!("{repo}  ·  {machine}"), regular(10.0), ink.muted);

        // The search field.
        let field = Rect::from_min_size(pos2(rect.min.x + 12.0, rect.min.y + 66.0), vec2(rect.width() - 24.0, 32.0));
        ui.painter().rect_filled(field, CornerRadius::same(5), ink.field);
        ui.painter().rect_stroke(field, CornerRadius::same(5), Stroke::new(1.0, ink.border), StrokeKind::Inside);
        let edit_rect = Rect::from_min_max(pos2(field.min.x + 12.0, field.min.y + 7.0), pos2(field.max.x - 10.0, field.max.y - 7.0));
        let edit = ui.put(
            edit_rect,
            egui::TextEdit::singleline(&mut app.status.query)
                .id(Id::new("worktree_switcher_search"))
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .font(regular(11.0))
                .text_color(ink.text)
                .hint_text(egui::RichText::new("Search branch or worktree path…").font(regular(11.0)).color(ink.faint)),
        );
        if app.status.focus_search {
            edit.request_focus();
            app.status.focus_search = false;
        }
        if edit.changed() {
            app.status.cursor = None;
            app.status.failed = None;
        }

        let top = rect.min.y + 110.0;
        let foot_rule = rect.max.y - 32.0;
        let footer = match phase {
            Phase::List => {
                let list =
                    Rect::from_min_max(pos2(rect.min.x + 8.0, top), pos2(rect.max.x - 8.0, foot_rule - if any_without_map { 36.0 } else { 8.0 }));
                let hot = highlighted(&rows, app.status.cursor.as_deref()).map(|row| row.id.clone());
                ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
                    egui::ScrollArea::vertical().id_salt("worktree_switcher_rows").auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 8.0;
                        for row in &rows {
                            if worktree_row(ui, ink, row, hot.as_deref() == Some(row.id.as_str())) {
                                ask = Some(Ask::Open(row.clone()));
                            }
                        }
                    });
                });
                if any_without_map {
                    ui.painter().text(
                        pos2(x, foot_rule - 28.0),
                        Align2::LEFT_TOP,
                        "No map yet? Selecting that worktree opens Init.",
                        regular(10.0),
                        ink.muted,
                    );
                }
                "↑ ↓ to choose   ·   Enter to open   ·   Esc to close".to_string()
            }
            Phase::NoMatch => {
                ui.painter().text(pos2(x, top + 16.0), Align2::LEFT_TOP, "No matching worktree", regular(13.0), ink.text);
                wrapped(
                    ui,
                    pos2(x, top + 46.0 - 16.0),
                    inner_w,
                    "Try another branch name or folder path. Only worktrees of this project on this machine appear here.",
                    ink.muted,
                );
                if card_button(ui, ink, Id::new("worktree_switcher_clear"), pos2(x, top + 16.0 + 102.0), "Clear search", false) {
                    ask = Some(Ask::Clear);
                }
                "Esc to close  ·  Clear search to show all".to_string()
            }
            Phase::Opening => {
                let to = app.status.switching.as_ref().map(|s| s.branch.clone()).unwrap_or_default();
                let from = app.status.switching.as_ref().map(|s| s.from.clone()).unwrap_or_else(|| current.clone());
                ui.painter().text(pos2(x, top + 16.0), Align2::LEFT_TOP, format!("Opening {to}…"), regular(13.0), ink.text);
                wrapped(
                    ui,
                    pos2(x, top + 16.0 + 34.0),
                    inner_w,
                    &format!("Still viewing {from}. The header and canvas change together when the other map is ready."),
                    ink.muted,
                );
                ui.painter().text(
                    pos2(x, top + 16.0 + 96.0),
                    Align2::LEFT_TOP,
                    "Agents keep working in their own worktrees.",
                    regular(10.0),
                    ink.teal,
                );
                if card_button(ui, ink, Id::new("worktree_switcher_cancel"), pos2(x, top + 16.0 + 134.0), "Cancel switch", false) {
                    ask = Some(Ask::Cancel);
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
                format!("Esc cancels switching  ·  {from} stays open")
            }
            Phase::Failed => {
                let (to, from, reason) = app.status.failed.as_ref().map(|(s, e)| (s.branch.clone(), s.from.clone(), e.clone())).unwrap_or_default();
                ui.painter().text(pos2(x, top + 16.0), Align2::LEFT_TOP, format!("Could not open {to}"), regular(13.0), ink.accent);
                let below = wrapped(
                    ui,
                    pos2(x, top + 16.0 + 34.0),
                    inner_w,
                    &format!("The worktree is unavailable. You are still on {from}; your map and draft are unchanged."),
                    ink.muted,
                );
                let reason_h = wrapped_font(ui, pos2(x, below + 6.0), inner_w, &reason, ink.faint, egui::FontId::monospace(9.0), 2);
                let y = (reason_h + 14.0).max(top + 16.0 + 114.0);
                if card_button(ui, ink, Id::new("worktree_switcher_retry"), pos2(x, y), "Retry", true) {
                    ask = Some(Ask::Retry);
                }
                let stay = format!("Stay on {from}");
                if card_button(ui, ink, Id::new("worktree_switcher_stay"), pos2(x + 92.0, y), &stay, false) {
                    ask = Some(Ask::Close);
                }
                format!("Enter to retry  ·  Esc to stay on {from}")
            }
        };
        ui.painter().line_segment([pos2(rect.min.x + 12.0, foot_rule), pos2(rect.max.x - 12.0, foot_rule)], Stroke::new(1.0, ink.border));
        ui.painter().text(pos2(x, foot_rule + 11.0), Align2::LEFT_TOP, footer, egui::FontId::monospace(9.0), ink.faint);
        rect
    });

    // A click anywhere else closes the card (the chip toggles it itself).
    let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !shown.inner.contains(p) && !chip.contains(p)));
    if outside && phase != Phase::Opening && ask.is_none() {
        ask = Some(Ask::Close);
    }

    match ask {
        Some(Ask::Open(row)) => {
            if row.current {
                app.status.close();
            } else if row.map == "map" {
                app.status.switching = Some(Switch { id: row.id.clone(), branch: row.branch.clone(), from: current });
                app.status.failed = None;
                app.ws.act(&ctx, &row.id, false);
            } else {
                // A worktree without a map opens its Init; no agent starts until asked.
                app.status.close();
                app.ws.select(&row.id);
                app.ws.worktree_init = true;
                app.ui.home = true;
            }
        }
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
        Some(Ask::Clear) => {
            app.status.query.clear();
            app.status.cursor = None;
            app.status.focus_search = true;
        }
        Some(Ask::Close) => {
            app.status.close();
        }
        None => {}
    }
}

/// One worktree: branch and path, the state at the right; highlighted when chosen.
#[cfg(not(target_arch = "wasm32"))]
fn worktree_row(ui: &mut Ui, ink: &Ink, row: &Row, hot: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    crate::a11y::button(&resp, &format!("{} · {}", row.branch, row.path));
    if hot || resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(5), ink.selected);
    }
    let (state, color) = if row.current {
        ("CURRENT", ink.teal)
    } else if row.map == "map" {
        ("MAP", ink.muted)
    } else if row.map == "empty" {
        ("EMPTY", ink.muted)
    } else {
        ("NO MAP", ink.muted)
    };
    let state_x = rect.min.x + 310.0;
    // The monospace face has no check glyph: the mark is drawn before CURRENT.
    let state_x = if row.current {
        crate::workspaces::check_mark(ui, pos2(state_x + 3.5, rect.min.y + 14.5), color);
        state_x + 12.0
    } else {
        state_x
    };
    ui.painter().text(pos2(state_x, rect.min.y + 10.0), Align2::LEFT_TOP, state, egui::FontId::monospace(8.0), color);
    let branch = crate::widgets::elide_font(ui, &row.branch, state_x - 12.0 - (rect.min.x + 12.0), regular(12.0));
    ui.painter().text(pos2(rect.min.x + 12.0, rect.min.y + 8.0), Align2::LEFT_TOP, branch, regular(12.0), ink.text);
    let path = crate::widgets::elide_start_font(ui, &row.path, rect.width() - 24.0, egui::FontId::monospace(9.0));
    ui.painter().text(pos2(rect.min.x + 12.0, rect.min.y + 32.0), Align2::LEFT_TOP, path, egui::FontId::monospace(9.0), ink.muted);
    resp.clicked()
}

/// Wrapped body text at `pos`; returns the bottom of what it drew.
#[cfg(not(target_arch = "wasm32"))]
fn wrapped(ui: &Ui, pos: egui::Pos2, width: f32, text: &str, color: Color32) -> f32 {
    wrapped_font(ui, pos, width, text, color, regular(11.0), 3)
}

#[cfg(not(target_arch = "wasm32"))]
fn wrapped_font(ui: &Ui, pos: egui::Pos2, width: f32, text: &str, color: Color32, font: egui::FontId, max_rows: usize) -> f32 {
    if text.trim().is_empty() {
        return pos.y;
    }
    let mut job = egui::text::LayoutJob::simple(text.to_string(), font, color, width);
    job.wrap.max_rows = max_rows;
    job.wrap.break_anywhere = false;
    let galley = ui.painter().layout_job(job);
    let height = galley.size().y;
    ui.painter().galley(pos, galley, color);
    pos.y + height
}

/// A 32 pt card button: white with a rule, or filled in the accent.
#[cfg(not(target_arch = "wasm32"))]
fn card_button(ui: &mut Ui, ink: &Ink, id: Id, at: egui::Pos2, label: &str, filled: bool) -> bool {
    let font = regular(11.0);
    let w = ui.painter().layout_no_wrap(label.to_string(), font.clone(), ink.text).size().x + 24.0;
    let rect = Rect::from_min_size(at, vec2(w.max(82.0), 32.0));
    let resp = ui.interact(rect, id, Sense::click());
    crate::a11y::button(&resp, label);
    if filled {
        ui.painter().rect_filled(rect, CornerRadius::same(6), if resp.hovered() { Color32::from_rgb(0xC2, 0x55, 0x3E) } else { ink.accent });
        ui.painter().text(pos2(rect.min.x + 12.0, rect.center().y), Align2::LEFT_CENTER, label, font, Color32::WHITE);
    } else {
        ui.painter().rect_filled(rect, CornerRadius::same(6), if resp.hovered() { ink.selected } else { ink.panel });
        ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, ink.border), StrokeKind::Inside);
        ui.painter().text(pos2(rect.min.x + 12.0, rect.center().y), Align2::LEFT_CENTER, label, font, ink.text);
    }
    resp.clicked()
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

    fn three() -> Vec<Workspace> {
        let mut linked = ws("w2", "checkout", "mindmap", "/root/code/mindmap-worktrees/checkout", Some("feature/checkout"), "map");
        linked.worktree = true;
        let mut import = ws("w3", "import", "mindmap", "/root/code/mindmap-worktrees/import", Some("experiment/import"), "none");
        import.worktree = true;
        vec![
            import,
            linked,
            ws("w1", "mindmap", "mindmap", "/root/code/mindmap", Some("main"), "map"),
            ws("w9", "other", "other", "/root/code/other", Some("main"), "map"),
        ]
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
    fn the_switcher_lists_the_repository_main_checkout_first() {
        let listed = switcher_rows(Some(&target("w2", "", "")), &three());
        let branches: Vec<&str> = listed.iter().map(|row| row.branch.as_str()).collect();
        assert_eq!(branches, ["main", "experiment/import", "feature/checkout"]);
        assert!(listed.iter().find(|row| row.branch == "feature/checkout").is_some_and(|row| row.current));
        assert_eq!(listed.iter().filter(|row| row.current).count(), 1);
    }

    #[test]
    fn search_matches_branch_or_path_ignoring_case() {
        let listed = switcher_rows(Some(&target("w1", "/root/code/mindmap", "main")), &three());
        assert_eq!(filter_rows(&listed, "CHECKOUT").len(), 1);
        assert_eq!(filter_rows(&listed, "mindmap-worktrees").len(), 2);
        assert_eq!(filter_rows(&listed, "  ").len(), 3);
        assert!(filter_rows(&listed, "billing").is_empty());
    }

    #[test]
    fn arrows_start_on_the_current_row_and_wrap() {
        let listed = switcher_rows(Some(&target("w2", "", "")), &three());
        assert_eq!(step(&listed, None, true).as_deref(), Some("w2"));
        assert_eq!(step(&listed, Some("w2"), true).as_deref(), Some("w1"));
        assert_eq!(step(&listed, Some("w1"), false).as_deref(), Some("w2"));
        assert_eq!(step(&[], None, true), None);
    }

    #[test]
    fn the_card_phase_follows_search_and_switching() {
        let mut state = State::default();
        state.open();
        assert_eq!(state.phase(3), Phase::List);
        state.query = "billing".into();
        assert_eq!(state.phase(0), Phase::NoMatch);
        state.query.clear();
        assert_eq!(state.phase(0), Phase::List);
        let switch = Switch { id: "w2".into(), branch: "feature/checkout".into(), from: "main".into() };
        state.switching = Some(switch.clone());
        assert_eq!(state.phase(3), Phase::Opening);
        state.switching = None;
        state.failed = Some((switch, "unavailable".into()));
        assert_eq!(state.phase(3), Phase::Failed);
        state.open();
        assert_eq!(state.phase(3), Phase::List);
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
