//! The bottom comment composer: one bar that is both the comment box and the prompt box.
//!
//! The selection tells it what a comment is about. Enter turns the input into a draft on that
//! card (the drafts wait above the input, across selections); Cmd+Enter sends the input as the
//! prompt with every draft attached to an agent in the workspace. Clicking a chip opens its draft
//! back in the input to edit; a second click (or Esc) goes back to a new comment, and only the
//! chip's × removes the draft. The service is reached over plain TCP on a thread of its own, so
//! the window never waits on the network.
//!
//! Map mode's right-click menu lives here too: the human gets Copy and Comment, never the
//! editor's tools.

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver};

use egui::text::{CCursor, CCursorRange};
use egui::{Align2, Color32, CornerRadius, FontId, Id, Key, Margin, Modifiers, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_doc::{Document, NodeId};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::FigApp;
use crate::a11y;
use crate::theme::{regular, semibold};
use crate::widgets;
#[cfg(not(target_arch = "wasm32"))]
use crate::workspaces;
use crate::workspaces::Target;

#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

/// The composer spans the canvas with 24 pt either side, never wider than this.
const PANEL_MAX_W: f32 = 1600.0;
/// The least it shrinks to in a narrow window.
const PANEL_MIN_W: f32 = 280.0;
/// Room left at the sides of the canvas (24 pt each).
const PANEL_SIDE: f32 = 48.0;
/// The space between the bar and the status bar under it.
const PANEL_GAP: f32 = 26.0;
/// The round Send button's size.
const SEND_D: f32 = 28.0;
/// The agent chip's height; it sits on the field's bottom edge with Send.
const TARGET_H: f32 = 24.0;
/// How many draft chips fit in the strip before `+N`.
const VISIBLE_DRAFTS: usize = 3;

/// The warm map palette: a white paper bar like the map bar above it.
const PANEL: Color32 = Color32::WHITE;
const SOFT: Color32 = Color32::from_rgb(0xF8, 0xF6, 0xF3);
const BORDER: Color32 = Color32::from_rgb(0xE3, 0xE2, 0xDF);
const INK: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x1A);
const MUTED: Color32 = Color32::from_rgb(0x6E, 0x6C, 0x68);
const FAINT: Color32 = Color32::from_rgb(0x9A, 0x97, 0x91);
const ACCENT: Color32 = Color32::from_rgb(0xB5, 0x45, 0x2F);
const ACCENT_HOVER: Color32 = Color32::from_rgb(0xC2, 0x55, 0x3E);
const MAP_INK: Color32 = Color32::from_rgb(0x2F, 0x6F, 0x6A);
const DANGER: Color32 = Color32::from_rgb(0x9E, 0x3B, 0x2C);
/// The draft open in the field, and that field: a blue that leaves the warm accents alone.
const EDIT: Color32 = Color32::from_rgb(0x2F, 0x6F, 0xDB);
const EDIT_FILL: Color32 = Color32::from_rgb(0xEA, 0xF0, 0xFB);
/// The ×'s hit area at the right end of a draft chip.
const DRAFT_X_W: f32 = 22.0;

#[cfg(not(target_arch = "wasm32"))]
const TARGETS_TIMEOUT: Duration = Duration::from_secs(10);
/// An agent may have to start (and accept a first-run prompt) before the send returns.
#[cfg(not(target_arch = "wasm32"))]
const SEND_TIMEOUT: Duration = Duration::from_secs(120);
/// A comment kept on the bar until a prompt carries it to an agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    pub text: String,
    /// The card's name when the draft was written.
    pub node: Option<String>,
    /// The page's name when the draft was written.
    pub page: Option<String>,
    /// The card the comment is attached to; `None` for the whole page.
    pub node_id: Option<NodeId>,
    /// The page it belongs to.
    pub page_id: Option<NodeId>,
}

/// One choice in the target chip: an agent already in the workspace, or a CLI to start.
#[derive(Clone, Debug)]
pub struct TargetInfo {
    pub target: Target,
    /// The agent's lifecycle state (`idle`, `working`, …); empty for a new CLI.
    pub status: String,
    /// Focused in the user's Herdr UI.
    pub focused: bool,
    /// A new agent rather than one already in the workspace.
    pub new_agent: bool,
}

/// The service's answer to a `send`.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct SendReply {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    /// The agent the service used (`kind`, `name`, `pane`, …).
    #[serde(default)]
    pub agent: Option<Value>,
    /// How many comments the agent received.
    #[serde(default)]
    pub sent: u64,
}

/// The composer's state.
#[derive(Default)]
pub struct State {
    /// The one line being typed.
    pub input: String,
    /// Comments waiting to be sent; they survive selection changes.
    pub drafts: Vec<Draft>,
    /// The draft the field is editing (an index into [`drafts`](Self::drafts)); `None` for a new
    /// comment. Its chip stays in the strip and wears the editing outline.
    pub editing: Option<usize>,
    /// The drafts the in-flight send carries; they leave the bar when it succeeds.
    pub sending: Vec<Draft>,
    /// The agent the send addresses.
    pub target: Option<Target>,
    /// The target menu is open.
    pub menu_open: bool,
    /// The agents in the workspace and the CLIs a new agent can start.
    pub targets: Vec<TargetInfo>,
    /// A send is on its thread.
    pub busy: bool,
    /// The send's answer.
    pub rx: Option<Receiver<Result<SendReply, String>>>,
    /// The selection the input was last cleared for.
    pub last_selection: Vec<NodeId>,
    /// The page the input was last cleared for.
    pub last_page: Option<NodeId>,
    /// Ask the input for the keyboard on the next draw (the context menu's Comment).
    pub focus_input: bool,
    /// The composer's + Page action asked for a page derived from the selected card.
    page_from_card: bool,
    /// The workspace whose targets were asked for (or are being asked for).
    targets_for: Option<String>,
    /// When the agent list was last requested, for the periodic status refresh.
    pub(crate) targets_at: f64,
    /// The target list's answer while it is in flight.
    targets_rx: Option<Receiver<Result<Vec<TargetInfo>, String>>>,
    /// Why the target list is empty, when the service could not answer.
    pub(crate) targets_error: Option<String>,
}

impl State {
    /// Open `drafts[index]` in the field with the caret at its end; a second click on the chip it
    /// belongs to goes back to a new comment instead.
    fn toggle_editing(&mut self, index: usize, ctx: &egui::Context) {
        if self.editing == Some(index) {
            self.stop_editing();
            return;
        }
        let Some(text) = self.drafts.get(index).map(|draft| draft.text.clone()) else { return };
        self.input = text;
        self.editing = Some(index);
        self.focus_input = true;
        let end = self.input.chars().count();
        let mut edit = egui::TextEdit::load_state(ctx, input_id()).unwrap_or_default();
        edit.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(end))));
        egui::TextEdit::store_state(ctx, input_id(), edit);
    }

    /// Back to a new comment; the draft keeps whatever it had.
    fn stop_editing(&mut self) {
        if self.editing.take().is_some() {
            self.input.clear();
            self.focus_input = true;
        }
    }

    /// Save the field's text into the draft being edited, then go back to a new comment.
    fn save_edited(&mut self) {
        let Some(index) = self.editing.take() else { return };
        if let Some(draft) = self.drafts.get_mut(index) {
            let text = self.input.trim();
            if !text.is_empty() {
                draft.text = text.to_string();
            }
        }
        self.input.clear();
    }

    /// Remove one draft, keeping [`editing`](Self::editing) on the draft it was on.
    fn remove_draft(&mut self, index: usize) {
        if index >= self.drafts.len() {
            return;
        }
        self.drafts.remove(index);
        match self.editing {
            Some(active) if active == index => self.stop_editing(),
            Some(active) if active > index => self.editing = Some(active - 1),
            _ => {}
        }
    }
}

/// Draw the composer at the bottom of an open map.
pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    poll_send(app);
    poll_targets(app);
    // The agents belong to the workspace: ask for them when the map first shows.
    if app.active_workspace.is_some() {
        ensure_targets(app, ctx);
    }

    // Selecting another card (or another page) empties the input; the drafts stay above it.
    let selection = app.session.selection();
    let page = app.session.page().ok();
    if selection != app.compose.last_selection || page != app.compose.last_page {
        app.compose.input.clear();
        app.compose.editing = None;
        app.compose.last_selection = selection.clone();
        app.compose.last_page = page;
    }

    let node_id = selection.first().copied();
    let node = node_id.and_then(|id| node_name(app, id));
    let page_name = page.and_then(|id| node_name(app, id));
    let has_selection = !selection.is_empty();
    // What a send from here carries: the drafts in the card's subtree, or on the whole page.
    let included = included_drafts(app, &selection, page);
    let comments = included.len();
    let plural = if comments == 1 { "" } else { "s" };
    let context = match (node.as_deref(), page_name.as_deref()) {
        (Some(node), _) => format!("COMMENTING ON · {node} · {comments} comment{plural} in the tree"),
        (None, Some(page)) => format!("COMMENTING ON · {page} · whole page · {comments} comment{plural}"),
        _ => "COMMENTING ON".to_string(),
    };
    let active = app.active_workspace.is_some();
    draft_glow(app, ctx, page);
    tree_panel(app, ctx, node_id, page, &included);
    // The workspace pane takes the left edge; the composer belongs to the canvas, not the pane.
    let pane = if app.ws.pane_open { 300.0 } else { 0.0 };
    let width = (ctx.content_rect().width() - pane - PANEL_SIDE).clamp(PANEL_MIN_W, PANEL_MAX_W);

    let mut send = false;
    let mut clear = false;
    egui::Area::new(Id::new("map_composer"))
        .anchor(Align2::CENTER_BOTTOM, vec2(pane / 2.0, -(crate::status_bar::HEIGHT + PANEL_GAP)))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(PANEL)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(CornerRadius::same(10))
                .shadow(egui::epaint::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(24) })
                .inner_margin(Margin::symmetric(15, 12))
                .show(ui, |ui| {
                    ui.set_width((width - 30.0).max(40.0));
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let names = Names { node: node.as_deref(), page: page_name.as_deref(), node_id, page_id: page };
                    let out = body(ui, &mut app.compose, &context, &names, has_selection, active, &included, comments);
                    send = out.send;
                    clear = out.clear;
                });
        });
    if clear {
        app.run("select.none", serde_json::json!({}));
    }
    if app.compose.page_from_card {
        app.compose.page_from_card = false;
        if let (Some(node_id), Some(page_id), Some(node_name), Some(page_name)) = (node_id, page, node.as_deref(), page_name.as_deref()) {
            let number = app.session.doc().map(|doc| doc.pages.len() + 1).unwrap_or(1);
            app.new_page.start_component(
                number,
                crate::new_page::Source {
                    page_id: page_id.0,
                    page_name: page_name.to_string(),
                    node_id: node_id.0,
                    node_name: node_name.to_string(),
                },
            );
        }
    }
    if send {
        start_send(app, ctx);
    }
}

/// The names a draft records: the card and page the human was looking at.
struct Names<'a> {
    node: Option<&'a str>,
    page: Option<&'a str>,
    node_id: Option<NodeId>,
    page_id: Option<NodeId>,
}

/// What the human asked of the bar this frame.
#[derive(Default)]
struct BodyOut {
    /// Send was pressed (or Cmd+Enter).
    send: bool,
    /// `×  clear selection`: comment on the whole page again.
    clear: bool,
}

/// The bar's contents.
fn body(
    ui: &mut Ui,
    state: &mut State,
    context: &str,
    names: &Names<'_>,
    has_selection: bool,
    active: bool,
    included: &[usize],
    comments: usize,
) -> BodyOut {
    let mut out = BodyOut::default();
    // A draft may leave the list while its chip was active (sent, or removed elsewhere): never
    // keep editing one that is gone.
    if state.editing.is_some_and(|index| index >= state.drafts.len()) {
        state.stop_editing();
    }
    let editing = state.editing.and_then(|index| state.drafts.get(index).map(|draft| (index, draft)));
    let context_line = match editing {
        Some((index, draft)) => editing_context(draft, included, index),
        None => context.to_string(),
    };
    // The context line is the accent in every state, as the design draws it.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        ui.label(egui::RichText::new(&context_line).font(mono(8.5)).color(ACCENT));
        if has_selection && names.node_id.is_some() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if text_action(ui, "×  clear selection", "Comment on the whole page") {
                    out.clear = true;
                }
                if text_action(ui, "+ Page", "New page from this card") {
                    state.page_from_card = true;
                }
            });
        }
    });

    if !included.is_empty() {
        ui.add_space(6.0);
        draft_strip(ui, state, included);
    }
    ui.add_space(6.0);

    // The field is multiline: Enter belongs to it. Shift+Enter turns the current text into a
    // draft comment (or saves the one being edited); Cmd+Enter sends the prompt and every draft.
    // Both are consumed before the field sees them.
    let input_focused = ui.memory(|m| m.has_focus(input_id()));
    let command_enter = ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));
    let shift_enter = !command_enter && input_focused && ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Enter));
    if shift_enter {
        if state.editing.is_some() {
            state.save_edited();
        } else if !state.input.trim().is_empty() {
            state.drafts.push(Draft {
                text: state.input.trim().to_string(),
                node: names.node.map(str::to_string),
                page: names.page.map(str::to_string),
                node_id: names.node_id,
                page_id: names.page_id,
            });
            state.input.clear();
        }
    }
    // Esc stops editing and leaves the draft as it was; Backspace on an empty field does the
    // same, matching the design's "Esc or an empty field cancels" note. Esc is only read (not
    // consumed): it belongs to any popup or modal open over the map as well.
    let escape = state.editing.is_some() && !state.menu_open && ui.input(|i| i.key_pressed(Key::Escape));
    let backspace = !escape
        && state.editing.is_some()
        && state.input.is_empty()
        && input_focused
        && !state.menu_open
        && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Backspace));
    if escape || backspace {
        state.stop_editing();
    }

    let mut send = false;
    // The agent chip and Send sit on the field's bottom edge, so a growing field rises alone:
    // both take the field's height and draw at its foot.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let available = ui.available_width();
        let chip_label = target_label(state);
        let chip_w = (text_width(ui, &chip_label, regular(10.5)) + 30.0).clamp(62.0, 150.0);
        let input_w = (available - chip_w - SEND_D - 16.0).max(90.0);

        let placeholder = if names.node.is_some() { "Comment on this card and its children…" } else { "Comment on the whole page…" };
        // Grow with the prompt, up to eight rows; the full text stays reachable by scrolling.
        let per_line = ((input_w - 20.0) / 6.5).max(12.0);
        let rows = state.input.split('\n').map(|line| ((line.chars().count() as f32 / per_line).ceil() as usize).max(1)).sum::<usize>().clamp(1, 8);
        let editing_now = state.editing.is_some();
        let input_response = egui::Frame::NONE
            .fill(if editing_now { PANEL } else { SOFT })
            .stroke(Stroke::new(1.0, if editing_now { EDIT } else { BORDER }))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_width((input_w - 20.0).max(40.0));
                ui.add(
                    egui::TextEdit::multiline(&mut state.input)
                        .id(input_id())
                        .frame(egui::Frame::NONE)
                        .font(regular(12.5))
                        .hint_text(egui::RichText::new(placeholder).font(regular(12.5)).color(FAINT))
                        .desired_rows(rows)
                        .desired_width(ui.available_width()),
                )
            })
            .inner;
        if state.focus_input {
            input_response.request_focus();
            state.focus_input = false;
        }

        let row_h = input_response.rect.height().max(SEND_D);
        ui.add_space(8.0);
        let target_response = target_chip(ui, state, chip_w, row_h);
        if target_response.clicked() {
            state.menu_open = !state.menu_open;
        }
        if state.menu_open {
            target_menu(ui, state, &target_response);
        }

        ui.add_space(8.0);
        let can_send = active && state.target.is_some() && (!state.input.trim().is_empty() || !included.is_empty()) && !state.busy;
        if send_button(ui, state.busy, can_send, row_h) {
            send = true;
        }
    });

    if command_enter {
        send = true;
    }
    // Sending a draft that is open in the field sends that draft: put the edit back into it and
    // empty the field, so the comment travels once (and not again as the prompt).
    if send {
        state.save_edited();
    }

    ui.add_space(6.0);
    let hint = if state.editing.is_some() {
        "⇧⏎ save changes   ·   Esc stop editing   ·   × on the chip removes it".to_string()
    } else if !active {
        "add the service to send".to_string()
    } else {
        format!("⏎ new line   ·   ⇧⏎ comment   ·   ⌘⏎ send all   ·   {comments} in the tree")
    };
    ui.label(egui::RichText::new(hint).font(mono(8.5)).color(MUTED));
    out.send = send;
    out
}

/// A quiet mono action at the right of the context line (`×  clear selection`, `+ Page`).
fn text_action(ui: &mut Ui, label: &str, tip: &str) -> bool {
    let size = vec2(text_width(ui, label, mono(8.5)), 12.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    a11y::button(&resp, label);
    let ink = if resp.hovered() { INK } else { MUTED };
    ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, label, mono(8.5), ink);
    resp.on_hover_text(tip).clicked()
}

/// The context line while a draft is open in the field: `EDITING DRAFT · on Card · 1 of 2`.
fn editing_context(draft: &Draft, included: &[usize], index: usize) -> String {
    let on = match (draft.node.as_deref(), draft.page.as_deref()) {
        (Some(node), _) => format!("on {node}"),
        (None, Some(page)) => format!("on {page}"),
        _ => "on the page".to_string(),
    };
    let position = included.iter().position(|queued| *queued == index).map_or(1, |at| at + 1);
    let total = included.len().max(position);
    format!("EDITING DRAFT  ·  {on}  ·  {position} of {total}")
}

/// One line of the comment tree: a card under the selection and the drafts it carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeRow {
    pub name: String,
    pub count: usize,
}

/// The comment tree a send would carry: the root's total, then each other card with drafts, in the
/// order its first draft was written. Drafts on the root itself count only in the total.
pub fn tree_rows(drafts: &[Draft], included: &[usize], root: Option<NodeId>) -> (usize, Vec<TreeRow>) {
    let mut rows: Vec<(Option<NodeId>, TreeRow)> = Vec::new();
    for draft in included.iter().filter_map(|index| drafts.get(*index)) {
        if draft.node_id.is_some() && draft.node_id == root {
            continue;
        }
        let name = draft.node.clone().unwrap_or_else(|| "Whole page".to_string());
        match rows.iter_mut().find(|(id, row)| *id == draft.node_id && (draft.node_id.is_some() || row.name == name)) {
            Some((_, row)) => row.count += 1,
            None => rows.push((draft.node_id, TreeRow { name, count: 1 })),
        }
    }
    (included.len(), rows.into_iter().map(|(_, row)| row).collect())
}

/// `COMMENT TREE` at the top right of the canvas: what a prompt on the selection (or the page)
/// carries, so the human sees the subtree before sending. Only when there is something to carry.
fn tree_panel(app: &FigApp, ctx: &egui::Context, node_id: Option<NodeId>, page: Option<NodeId>, included: &[usize]) {
    if included.is_empty() {
        return;
    }
    let (total, rows) = tree_rows(&app.compose.drafts, included, node_id);
    let root = match node_id {
        Some(id) => node_name(app, id).unwrap_or_else(|| "This card".to_string()),
        None => page.and_then(|id| node_name(app, id)).map(|name| format!("{name} · whole page")).unwrap_or_else(|| "Whole page".to_string()),
    };
    let header_h = if app.ui.title_bar == crate::TitleBar::Inset { crate::MAC_BAND } else { 44.0 };
    let note = match rows.len() {
        0 => format!("A prompt on {} carries {}.", short(&root), if total == 1 { "it" } else { "them" }),
        2 => format!("A prompt on {} carries both.", short(&root)),
        n => format!("A prompt on {} carries all {}.", short(&root), n),
    };
    egui::Area::new(Id::new("map_comment_tree"))
        .anchor(Align2::RIGHT_TOP, vec2(-40.0, header_h + 40.0))
        .order(egui::Order::Middle)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(PANEL)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(CornerRadius::same(8))
                .shadow(egui::epaint::Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(20) })
                .inner_margin(Margin { left: 14, right: 14, top: 12, bottom: 14 })
                .show(ui, |ui| {
                    ui.set_width(252.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(egui::RichText::new("COMMENT TREE").font(mono(8.5)).color(ACCENT));
                    ui.add_space(13.0);
                    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
                    let name = widgets::elide_font(ui, &root, line.width() - 30.0, semibold(11.5));
                    ui.painter().text(line.left_center(), Align2::LEFT_CENTER, name, semibold(11.5), INK);
                    ui.painter().text(line.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, total.to_string(), mono(9.0), ACCENT);
                    if !rows.is_empty() {
                        ui.add_space(8.0);
                        let (rule, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                        ui.painter()
                            .line_segment([rule.left_center() + vec2(8.0, 0.0), rule.right_center() + vec2(8.0, 0.0)], Stroke::new(1.0, BORDER));
                        ui.add_space(6.0);
                        let last = rows.len() - 1;
                        for (index, row) in rows.iter().enumerate() {
                            let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
                            let branch = if index == last { "└" } else { "├" };
                            ui.painter().text(line.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, branch, regular(10.5), INK);
                            let name = widgets::elide_font(ui, &row.name, line.width() - 60.0, regular(10.5));
                            ui.painter().text(line.left_center() + vec2(24.0, 0.0), Align2::LEFT_CENTER, name, regular(10.5), INK);
                            ui.painter().text(line.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, row.count.to_string(), mono(9.0), MUTED);
                        }
                    }
                    ui.add_space(10.0);
                    ui.add(egui::Label::new(egui::RichText::new(note).font(mono(8.5)).color(MUTED)).wrap());
                });
        });
}

/// Cards waiting with a draft comment glow blue on the canvas until a send carries them away
/// ("before: cards glow; after: no glow, sent once").
fn draft_glow(app: &FigApp, ctx: &egui::Context, page: Option<NodeId>) {
    let Ok(doc) = app.session.doc() else { return };
    let mut cards: Vec<NodeId> = app.compose.drafts.iter().filter(|draft| draft.page_id == page).filter_map(|draft| draft.node_id).collect();
    cards.sort_by_key(|id| id.0);
    cards.dedup();
    if cards.is_empty() {
        return;
    }
    let zoom = app.session.ds().map(|ds| ds.view.zoom).unwrap_or(1.0) as f32;
    let clip = app.canvas.rect;
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, Id::new("map_draft_glow"))).with_clip_rect(clip);
    let radius = (8.0 * zoom).clamp(0.0, 40.0) as u8;
    for id in cards {
        let Some(bounds) = doc.world_bounds(id) else { continue };
        let min = app.to_screen(bounds.x0, bounds.y0);
        let max = app.to_screen(bounds.x1, bounds.y1);
        let rect = Rect::from_min_max(min, max);
        if !rect.is_finite() || !rect.intersects(clip) {
            continue;
        }
        // A soft blue halo, then the card's pale blue edge.
        for (grow, alpha) in [(6.0, 14u8), (4.0, 26), (2.5, 40), (1.0, 60)] {
            painter.rect_stroke(
                rect.expand(grow),
                CornerRadius::same(radius.saturating_add(grow as u8)),
                Stroke::new(2.0, Color32::from_rgba_unmultiplied(0x2F, 0x6F, 0xDB, alpha)),
                StrokeKind::Outside,
            );
        }
        painter.rect_stroke(rect, CornerRadius::same(radius), Stroke::new(1.0, Color32::from_rgb(0xA9, 0xC2, 0xEE)), StrokeKind::Inside);
    }
}

/// A root name without the ` · whole page` suffix, for the note.
fn short(root: &str) -> &str {
    root.split(" · ").next().unwrap_or(root)
}

/// The draft chips above the input: the first few, each editable and removable, then `+N`.
fn draft_strip(ui: &mut Ui, state: &mut State, included: &[usize]) {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
        let visible = included.len().min(VISIBLE_DRAFTS);
        for &index in included.iter().take(visible) {
            let Some(draft) = state.drafts.get(index) else { continue };
            let label = draft.text.replace('\n', " ");
            if let Some(clicked) = draft_chip(ui, index, &label, state.editing == Some(index)) {
                action = Some(clicked);
            }
        }
        let hidden = included.len().saturating_sub(visible);
        if hidden > 0 {
            ui.label(egui::RichText::new(format!("+{hidden}")).font(mono(9.0)).color(MUTED));
        }
    });
    match action {
        Some(DraftAction::Edit(index)) => state.toggle_editing(index, ui.ctx()),
        Some(DraftAction::Remove(index)) => state.remove_draft(index),
        None => {}
    }
}

/// What a click on a draft chip asks for.
#[derive(Clone, Copy)]
enum DraftAction {
    /// Open the draft in the field (a second click on the active chip goes back to a new comment).
    Edit(usize),
    /// The × was pressed: delete the draft, and only this one.
    Remove(usize),
}

/// One draft chip: clicking its label opens the draft for editing; the × at its right removes it.
fn draft_chip(ui: &mut Ui, index: usize, text: &str, active: bool) -> Option<DraftAction> {
    let shown = widgets::elide_font(ui, text, 200.0, regular(10.5));
    let width = text_width(ui, &shown, regular(10.5)) + 33.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, 20.0), Sense::hover());
    let x_rect = Rect::from_min_max(pos2(rect.max.x - DRAFT_X_W, rect.min.y), rect.max);
    let body_rect = Rect::from_min_max(rect.min, pos2(rect.max.x - DRAFT_X_W, rect.max.y));
    // The × is added last, so it sits above the label's click area and gets the click.
    let body = ui.interact(body_rect, draft_body_id(index), Sense::click());
    let close = ui.interact(x_rect, draft_remove_id(index), Sense::click());
    a11y::button(&body, &format!("Edit draft: {text}"));
    a11y::button(&close, &format!("Remove draft: {text}"));
    let close = close.on_hover_text("Remove");
    let hot = body.hovered() || close.hovered();
    let fill = if active {
        EDIT_FILL
    } else if hot {
        Color32::from_rgb(0xF1, 0xF0, 0xED)
    } else {
        SOFT
    };
    // A pill: 20 pt tall, fully rounded.
    ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
    let border = if active { EDIT } else { BORDER };
    ui.painter().rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, border), StrokeKind::Inside);
    ui.painter().text(pos2(rect.min.x + 9.0, rect.center().y), Align2::LEFT_CENTER, &shown, regular(10.5), if active { EDIT } else { INK });
    let x_color = if close.hovered() { ACCENT } else { FAINT };
    ui.painter().text(pos2(rect.max.x - 9.0, rect.center().y), Align2::RIGHT_CENTER, "×", regular(10.5), x_color);
    if close.clicked() {
        Some(DraftAction::Remove(index))
    } else if body.clicked() {
        Some(DraftAction::Edit(index))
    } else {
        None
    }
}

/// What the target chip shows: the chosen agent's kind, or `auto` before one is chosen.
fn target_label(state: &State) -> String {
    match &state.target {
        Some(target) => kind_label(target.kind()).to_string(),
        None => "auto".to_string(),
    }
}

/// The target chip; clicking it opens or closes the agent menu.
fn target_chip(ui: &mut Ui, state: &State, width: f32, row_h: f32) -> Response {
    let (outer, _) = ui.allocate_exact_size(vec2(width, row_h), Sense::hover());
    // 24 pt tall, 3.5 pt above the field's bottom edge, like Send.
    let bottom = outer.max.y - 3.5;
    let rect = Rect::from_min_max(pos2(outer.min.x, bottom - TARGET_H), pos2(outer.max.x, bottom));
    let resp = ui.interact(rect, Id::new("map_composer_target_chip"), Sense::click());
    a11y::button(&resp, "Choose the agent to send the comment to");
    let hot = resp.hovered() || state.menu_open;
    ui.painter().rect_filled(rect, CornerRadius::same(6), if hot { SOFT } else { PANEL });
    ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, BORDER), StrokeKind::Inside);
    let label = widgets::elide_font(ui, &target_label(state), (width - 30.0).max(24.0), regular(10.5));
    ui.painter().text(pos2(rect.min.x + 11.0, rect.center().y), Align2::LEFT_CENTER, &label, regular(10.5), INK);
    let c = pos2(rect.max.x - 12.0, rect.center().y + 0.5);
    ui.painter().add(egui::Shape::convex_polygon(
        vec![pos2(c.x - 3.5, c.y - 1.5), pos2(c.x + 3.5, c.y - 1.5), pos2(c.x, c.y + 2.0)],
        MUTED,
        Stroke::NONE,
    ));
    resp
}

/// The compact agent menu above the chip: agents in the workspace, then the CLIs to start.
fn target_menu(ui: &mut Ui, state: &mut State, anchor: &Response) {
    let ctx = ui.ctx().clone();
    let frame = egui::Frame::NONE
        .fill(PANEL)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(8))
        .shadow(egui::epaint::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(24) })
        .inner_margin(Margin::symmetric(12, 10));
    let mut open = state.menu_open;
    let mut picked = None;
    egui::Popup::new(Id::new("map_composer_targets"), ctx, anchor, anchor.layer_id)
        .width(250.0)
        .gap(6.0)
        .align(egui::RectAlign::TOP_END)
        .frame(frame)
        .open_bool(&mut open)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            picked = target_menu_body(ui, state);
        });
    if let Some(target) = picked {
        state.target = Some(target);
        open = false;
    }
    state.menu_open = open;
}

fn target_menu_body(ui: &mut Ui, state: &State) -> Option<Target> {
    let mut picked = None;
    ui.set_width(ui.available_width());
    ui.spacing_mut().item_spacing = vec2(6.0, 2.0);
    eyebrow(ui, "AGENTS IN THIS WORKSPACE");
    ui.add_space(1.0);

    let agents: Vec<&TargetInfo> = state.targets.iter().filter(|info| !info.new_agent).collect();
    if !agents.is_empty() {
        for info in agents {
            let selected = state.target.as_ref() == Some(&info.target);
            if agent_row(ui, info, selected) {
                picked = Some(info.target.clone());
            }
        }
    } else if state.targets_rx.is_some() {
        paragraph(ui, "Looking for agents…", 11.0, MUTED);
    } else {
        paragraph(ui, "No agents in this workspace yet.", 11.0, MUTED);
    }
    if let Some(error) = state.targets_error.as_deref() {
        paragraph(ui, error, 10.5, DANGER);
    }

    ui.add_space(5.0);
    divider(ui);
    ui.add_space(5.0);
    eyebrow(ui, "START A NEW AGENT");
    ui.add_space(1.0);
    let clis: Vec<&TargetInfo> = state.targets.iter().filter(|info| info.new_agent).collect();
    if clis.is_empty() {
        paragraph(ui, "No agent CLIs installed on the project host.", 11.0, MUTED);
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for info in clis {
                let selected = state.target.as_ref() == Some(&info.target);
                if cli_chip(ui, info.target.kind(), selected) {
                    picked = Some(info.target.clone());
                }
            }
        });
    }
    picked
}

fn agent_row(ui: &mut Ui, info: &TargetInfo, selected: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    let kind = kind_label(info.target.kind());
    let status = info.status.trim();
    a11y::button(&resp, &if status.is_empty() { kind.to_string() } else { format!("{kind} · {status}") });
    let hot = resp.hovered();
    if selected || hot {
        ui.painter().rect_filled(
            rect.shrink2(vec2(4.0, 1.0)),
            CornerRadius::same(5),
            if selected { Color32::from_rgb(0xF1, 0xEC, 0xE7) } else { SOFT },
        );
    }
    let dot = pos2(rect.min.x + 9.0, rect.center().y);
    ui.painter().circle_filled(dot, 3.0, status_color(status));
    if info.focused {
        ui.painter().circle_stroke(dot, 5.0, Stroke::new(1.0, MAP_INK));
    }
    ui.painter().text(pos2(rect.min.x + 19.0, rect.center().y), Align2::LEFT_CENTER, kind, semibold(11.5), INK);
    if !status.is_empty() && !status.eq_ignore_ascii_case("unknown") {
        let trail = if selected { 22.0 } else { 8.0 };
        ui.painter().text(pos2(rect.max.x - trail, rect.center().y), Align2::RIGHT_CENTER, status, mono(9.5), MUTED);
    }
    if selected {
        let c = pos2(rect.max.x - 10.0, rect.center().y);
        let stroke = Stroke::new(1.6, ACCENT);
        ui.painter().line_segment([pos2(c.x - 4.0, c.y), pos2(c.x - 1.0, c.y + 3.5)], stroke);
        ui.painter().line_segment([pos2(c.x - 1.0, c.y + 3.5), pos2(c.x + 4.5, c.y - 4.0)], stroke);
    }
    resp.clicked()
}

fn cli_chip(ui: &mut Ui, kind: &str, selected: bool) -> bool {
    let label = kind_label(kind);
    let width = text_width(ui, label, semibold(10.5)) + 22.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
    a11y::button(&resp, &format!("Start a new {}", label));
    let hot = resp.hovered();
    let fill = if selected {
        Color32::from_rgb(0xF1, 0xEC, 0xE7)
    } else if hot {
        SOFT
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(5), fill);
    let border = if selected || hot { ACCENT } else { BORDER };
    ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, border), StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, semibold(10.5), if selected || hot { ACCENT } else { INK });
    resp.clicked()
}

/// The round Send button: a drawn arrow up, so no font has to have the glyph.
fn send_button(ui: &mut Ui, busy: bool, enabled: bool, row_h: f32) -> bool {
    let (outer, _) = ui.allocate_exact_size(vec2(SEND_D, row_h), Sense::hover());
    let rect = Rect::from_min_size(pos2(outer.min.x, outer.max.y - 1.5 - SEND_D), vec2(SEND_D, SEND_D));
    let resp = ui.interact(rect, Id::new("map_composer_send"), Sense::click());
    a11y::button(&resp, "Send the prompt");
    // The accent in every state, as the design draws it; hover lightens it only when it can send.
    let fill = if enabled && resp.hovered() { ACCENT_HOVER } else { ACCENT };
    ui.painter().circle_filled(rect.center(), SEND_D / 2.0, fill);
    let c = rect.center();
    if busy {
        ui.painter().text(c, Align2::CENTER_CENTER, "…", regular(12.0), Color32::WHITE);
    } else {
        let stroke = Stroke::new(1.5, Color32::WHITE);
        ui.painter().line_segment([c + vec2(0.0, 4.5), c + vec2(0.0, -4.5)], stroke);
        ui.painter().line_segment([c + vec2(-3.5, -1.0), c + vec2(0.0, -4.5)], stroke);
        ui.painter().line_segment([c + vec2(3.5, -1.0), c + vec2(0.0, -4.5)], stroke);
    }
    resp.clicked() && enabled
}

/// The map's right-click menu: Copy and Comment, and nothing from the editor.
pub fn context_menu(app: &mut FigApp, ctx: &egui::Context, pos: Pos2) {
    let copy = selection_text(app);
    let mut close = false;
    let area = egui::Area::new(Id::new("map_context_menu")).fixed_pos(pos).order(egui::Order::Foreground).constrain(true).show(ctx, |ui| {
        egui::Frame::NONE
            .fill(PANEL)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(CornerRadius::same(8))
            .shadow(egui::epaint::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(20) })
            .inner_margin(Margin::symmetric(6, 6))
            .show(ui, |ui| {
                ui.set_width(170.0);
                ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                if map_menu_item(ui, "Copy", "⌘C", !copy.is_empty(), false) {
                    if !copy.is_empty() {
                        crate::egui_copy_text(ctx, &copy);
                    }
                    close = true;
                }
                if map_menu_item(ui, "Comment", "", true, true) {
                    app.compose.focus_input = true;
                    ctx.request_repaint();
                    close = true;
                }
                let selected = app.session.selection().first().copied();
                let page = app.session.page().ok();
                if let Some(node_id) = selected {
                    let detail_page = app
                        .session
                        .doc()
                        .ok()
                        .and_then(|doc| doc.node(node_id))
                        .and_then(|node| node.plugin_data.get(crate::new_page::DETAIL_PAGE_KEY))
                        .and_then(|id| id.parse::<u64>().ok());
                    if let Some(detail_page) = detail_page
                        && map_menu_item(ui, "Open detail page", "", true, false)
                    {
                        app.run("page.select", json!({"id": detail_page}));
                        close = true;
                    }
                }
                if let Some(node_id) = selected
                    && let Some(page_id) = page
                    && let Some(page_name) = node_name(app, page_id)
                    && let Some(node_name) = node_name(app, node_id)
                    && map_menu_item(ui, "New page from card", "", true, false)
                {
                    let number = app.session.doc().map(|doc| doc.pages.len() + 1).unwrap_or(1);
                    app.new_page.start_component(number, crate::new_page::Source { page_id: page_id.0, page_name, node_id: node_id.0, node_name });
                    close = true;
                }
            });
    });
    let outside = ctx.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer();
    if close || outside || ctx.input(|i| i.key_pressed(Key::Escape)) {
        app.ui.context_menu = None;
    }
}

fn map_menu_item(ui: &mut Ui, label: &str, shortcut: &str, enabled: bool, accent: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), if enabled { Sense::click() } else { Sense::hover() });
    a11y::button(&resp, label);
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(5), SOFT);
    }
    let ink = if !enabled {
        FAINT
    } else if accent {
        ACCENT
    } else {
        INK
    };
    ui.painter().text(pos2(rect.min.x + 8.0, rect.center().y), Align2::LEFT_CENTER, label, regular(11.5), ink);
    if !shortcut.is_empty() {
        ui.painter().text(pos2(rect.max.x - 8.0, rect.center().y), Align2::RIGHT_CENTER, shortcut, mono(9.0), FAINT);
    }
    resp.clicked() && enabled
}

/// The selected cards' text, for Copy: every text layer under them, else their names.
fn selection_text(app: &FigApp) -> String {
    let Ok(doc) = app.session.doc() else { return String::new() };
    let ids = app.session.selection();
    let mut parts = Vec::new();
    for id in &ids {
        collect_text(doc, *id, &mut parts, 0);
    }
    if parts.is_empty() {
        for id in &ids {
            if let Some(node) = doc.node(*id)
                && !node.name.trim().is_empty()
            {
                parts.push(node.name.trim().to_string());
            }
        }
    }
    parts.join("\n").trim().to_string()
}

fn collect_text(doc: &figcraft_doc::Document, id: NodeId, parts: &mut Vec<String>, depth: u32) {
    if depth > 64 {
        return;
    }
    let Some(node) = doc.node(id) else { return };
    if let Some(text) = node.kind.text() {
        let characters = text.characters.trim();
        if !characters.is_empty() {
            parts.push(characters.to_string());
        }
    }
    for child in &node.children {
        collect_text(doc, *child, parts, depth + 1);
    }
}

/// The card's name (the kind's name when it is unnamed).
fn node_name(app: &FigApp, id: NodeId) -> Option<String> {
    let doc = app.session.doc().ok()?;
    let node = doc.node(id)?;
    Some(if node.name.trim().is_empty() { node.kind.type_name().to_lowercase() } else { node.name.trim().to_string() })
}

/// Force the next frame to re-read the agent list (the Agents modal's Refresh).
pub fn refresh_targets(app: &mut FigApp, ctx: &egui::Context) {
    app.compose.targets_at = 0.0;
    ensure_targets(app, ctx);
}

#[cfg(not(target_arch = "wasm32"))]
fn ensure_targets(app: &mut FigApp, ctx: &egui::Context) {
    let Some(workspace) = app.active_workspace.clone() else { return };
    let Ok(node) = workspaces::active_node(app) else { return };
    let identity = format!("{}:{}", node.address, node.session);
    let now = ctx.input(|input| input.time);
    let same = app.compose.targets_for.as_deref() == Some(identity.as_str());
    if same && now - app.compose.targets_at < 3.0 {
        return;
    }
    if same && app.compose.targets_rx.is_some() {
        return;
    }
    if !same {
        app.compose.targets.clear();
        app.compose.target = None;
        app.compose.targets_error = None;
    }
    app.compose.targets_for = Some(identity);
    app.compose.targets_at = now;
    let (tx, rx) = mpsc::channel();
    app.compose.targets_rx = Some(rx);
    let address = node.address;
    let context = ctx.clone();
    let spawned = std::thread::Builder::new().name("figcraft-compose-targets".into()).spawn(move || {
        let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fetch_targets(&address, &workspace, &node.session)))
            .unwrap_or_else(|_| Err("The agent list request failed unexpectedly.".to_string()));
        let _ = tx.send(reply);
        context.request_repaint();
    });
    if let Err(e) = spawned {
        app.compose.targets_rx = None;
        app.compose.targets_error = Some(format!("Could not ask for the agents: {e}"));
        app.toast(format!("Could not ask for the agents: {e}"));
    }
}

#[cfg(target_arch = "wasm32")]
fn ensure_targets(app: &mut FigApp, _ctx: &egui::Context) {
    let Some(workspace) = app.active_workspace.clone() else { return };
    if app.compose.targets_for.as_deref() == Some(workspace.as_str()) {
        return;
    }
    app.compose.targets_for = Some(workspace);
    app.compose.targets_error = Some("The node is only available in the desktop app.".to_string());
}

fn poll_targets(app: &mut FigApp) {
    let received = app.compose.targets_rx.as_ref().map(Receiver::try_recv);
    let received = match received {
        None => return,
        Some(Ok(reply)) => reply,
        Some(Err(mpsc::TryRecvError::Empty)) => return,
        Some(Err(mpsc::TryRecvError::Disconnected)) => {
            app.compose.targets_rx = None;
            app.compose.targets_error = Some("The agent list request ended without an answer.".to_string());
            app.toast("The agent list request ended without an answer.");
            return;
        }
    };
    app.compose.targets_rx = None;
    match received {
        Ok(targets) => {
            app.compose.targets = targets;
            app.compose.targets_error = None;
            // Keep the human's chosen agent, but take its refreshed status from the new list.
            let refreshed = app
                .compose
                .target
                .as_ref()
                .and_then(|current| app.compose.targets.iter().find(|info| info.target == *current))
                .map(|info| info.target.clone());
            app.compose.target = refreshed.or_else(|| default_target(&app.compose.targets));
        }
        Err(error) => {
            app.compose.targets_error = Some(error.clone());
            app.toast(format!("Can't read the agents: {error}"));
        }
    }
}

/// The service's reply to `workspaces`: only what the composer reads.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Default, Deserialize)]
struct TargetsReply {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    workspaces: Option<Vec<workspaces::Workspace>>,
    #[serde(default)]
    clis: Vec<String>,
}

/// Read the workspace's agents and the installed CLIs from the service.
#[cfg(not(target_arch = "wasm32"))]
fn fetch_targets(address: &str, workspace: &str, session: &str) -> Result<Vec<TargetInfo>, String> {
    let line = ask(address, &json!({"method":"workspaces", "session":session}).to_string(), TARGETS_TIMEOUT)?;
    let reply: TargetsReply = serde_json::from_str(&line).map_err(|e| format!("The node sent an agent list this app can't read: {e}"))?;
    if !reply.ok {
        return Err(reply.error.unwrap_or_else(|| "The node refused the request.".to_string()));
    }
    let row = reply
        .workspaces
        .unwrap_or_default()
        .into_iter()
        .find(|row| row.id == workspace)
        .ok_or_else(|| "The workspace is no longer listed by the node.".to_string())?;
    Ok(build_targets(row.agents_detail, &reply.clis))
}

/// The target list from the service's answer: its agents, then the CLIs to start.
#[cfg(not(target_arch = "wasm32"))]
fn build_targets(agents: Vec<workspaces::AgentDetail>, clis: &[String]) -> Vec<TargetInfo> {
    let mut targets = Vec::new();
    for agent in agents {
        if agent.pane.trim().is_empty() {
            continue;
        }
        targets.push(TargetInfo {
            target: Target::Existing { pane: agent.pane, kind: agent.kind, status: agent.status.clone() },
            status: agent.status,
            focused: agent.focused,
            new_agent: false,
        });
    }
    for kind in clis {
        let kind = kind.trim();
        if kind.is_empty() {
            continue;
        }
        targets.push(TargetInfo { target: Target::New { kind: kind.to_string() }, status: String::new(), focused: false, new_agent: true });
    }
    targets
}

/// The target a fresh list starts on: the focused agent, else the first non-blocked one, else any
/// agent; with no agent, the first CLI (the service starts it).
fn default_target(targets: &[TargetInfo]) -> Option<Target> {
    targets
        .iter()
        .find(|info| !info.new_agent && info.focused)
        .or_else(|| targets.iter().find(|info| !info.new_agent && !blocked(&info.status)))
        .or_else(|| targets.iter().find(|info| !info.new_agent))
        .or_else(|| targets.first())
        .map(|info| info.target.clone())
}

fn blocked(status: &str) -> bool {
    status.trim().eq_ignore_ascii_case("blocked")
}

/// The drafts a send from this selection would carry: the subtree for a card, the page for no
/// selection. Detached drafts on other cards and pages stay behind.
fn included_drafts(app: &FigApp, selection: &[NodeId], page: Option<NodeId>) -> Vec<usize> {
    let Ok(doc) = app.session.doc() else { return Vec::new() };
    let root = selection.first().copied();
    let mut ids = HashSet::new();
    if let Some(root) = root {
        collect_subtree(doc, root, &mut ids);
    }
    app.compose
        .drafts
        .iter()
        .enumerate()
        .filter(|(_, draft)| match root {
            Some(_) => draft.node_id.is_some_and(|id| ids.contains(&id)),
            None => draft.page_id.is_some() && draft.page_id == page,
        })
        .map(|(index, _)| index)
        .collect()
}

fn collect_subtree(doc: &Document, id: NodeId, out: &mut HashSet<NodeId>) {
    if !out.insert(id) {
        return;
    }
    if let Some(node) = doc.node(id) {
        for child in &node.children {
            collect_subtree(doc, *child, out);
        }
    }
}

/// Send the input as the prompt, with every draft attached, on a thread of its own.
#[cfg(not(target_arch = "wasm32"))]
fn start_send(app: &mut FigApp, ctx: &egui::Context) {
    if app.compose.busy {
        return;
    }
    let Some(workspace) = app.active_workspace.clone() else {
        app.toast("Add the node to send comments.");
        return;
    };
    let Some(target) = app.compose.target.clone() else {
        app.toast("Choose an agent to send the comment to.");
        return;
    };
    let prompt = app.compose.input.trim().to_string();
    let selection = app.session.selection();
    let page = app.session.page().ok();
    let included = included_drafts(app, &selection, page);
    let sending: Vec<Draft> = included.iter().filter_map(|index| app.compose.drafts.get(*index).cloned()).collect();
    if prompt.is_empty() && sending.is_empty() {
        return;
    }
    let comments: Vec<Value> = sending
        .iter()
        .map(|draft| {
            json!({
                "text": draft.text,
                "node": draft.node,
                "page": draft.page,
                "node_id": draft.node_id.map(|id| id.0),
                "page_id": draft.page_id.map(|id| id.0),
            })
        })
        .collect();
    let page_id = app.session.page().ok().map(|id| id.0);
    let Ok(node) = workspaces::active_node(app) else {
        app.toast("Open a connected node workspace first.");
        return;
    };
    let request = json!({
        "id": 1,
        "method": "send",
        "workspace": workspace,
        "session": node.session,
        "target": target.line(),
        "prompt": prompt,
        "comments": comments,
        "page_id": page_id,
        "new_page": false,
    })
    .to_string();
    let (tx, rx) = mpsc::channel();
    app.compose.busy = true;
    app.compose.sending = sending;
    app.compose.rx = Some(rx);
    let address = node.address;
    let context = ctx.clone();
    let spawned = std::thread::Builder::new().name("figcraft-comment-send".into()).spawn(move || {
        let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| send_request(&address, &request)))
            .unwrap_or_else(|_| Err("The comment request failed unexpectedly.".to_string()));
        let _ = tx.send(reply);
        context.request_repaint();
    });
    if let Err(e) = spawned {
        app.compose.busy = false;
        app.compose.rx = None;
        app.compose.sending.clear();
        app.toast(format!("Could not send the comment: {e}"));
    }
}

#[cfg(target_arch = "wasm32")]
fn start_send(app: &mut FigApp, _ctx: &egui::Context) {
    app.toast("The node is only available in the desktop app.");
}

fn poll_send(app: &mut FigApp) {
    let received = app.compose.rx.as_ref().map(Receiver::try_recv);
    let received = match received {
        None => return,
        Some(Ok(reply)) => reply,
        Some(Err(mpsc::TryRecvError::Empty)) => return,
        Some(Err(mpsc::TryRecvError::Disconnected)) => {
            app.compose.rx = None;
            app.compose.busy = false;
            app.compose.sending.clear();
            app.toast("The comment request ended without an answer.");
            return;
        }
    };
    app.compose.rx = None;
    app.compose.busy = false;
    match received {
        Ok(reply) if reply.ok => {
            let sent = if reply.sent > 0 { reply.sent } else { app.compose.sending.len() as u64 };
            let agent = reply
                .agent
                .as_ref()
                .and_then(agent_kind)
                .or_else(|| app.compose.target.as_ref().map(|target| kind_label(target.kind()).to_string()))
                .unwrap_or_else(|| "the agent".to_string());
            let sending = std::mem::take(&mut app.compose.sending);
            app.compose.drafts.retain(|draft| !sending.contains(draft));
            app.compose.input.clear();
            let what = if sent == 0 { "the prompt".to_string() } else { format!("{sent} comment{}", if sent == 1 { "" } else { "s" }) };
            app.toast(format!("Sent {what} to {agent}"));
        }
        Ok(reply) => {
            app.compose.sending.clear();
            app.toast(reply.error.unwrap_or_else(|| "The node refused the comment.".to_string()));
        }
        Err(error) => {
            app.compose.sending.clear();
            app.toast(error);
        }
    }
}

/// The agent's `kind` (or `name`) from the service's reply.
fn agent_kind(agent: &Value) -> Option<String> {
    ["kind", "name"]
        .into_iter()
        .filter_map(|key| agent.get(key).and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn send_request(address: &str, request: &str) -> Result<SendReply, String> {
    let line = ask(address, request, SEND_TIMEOUT)?;
    serde_json::from_str(&line).map_err(|e| format!("The node sent a reply this app can't read: {e}"))
}

/// Ask the node one newline-terminated JSON request and return its reply line.
#[cfg(not(target_arch = "wasm32"))]
fn ask(address: &str, request: &str, timeout: Duration) -> Result<String, String> {
    crate::node::ask(address, request, timeout)
}

fn eyebrow(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(mono(8.5)).color(MUTED));
}

fn paragraph(ui: &mut Ui, text: &str, size: f32, color: Color32) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(regular(size)).color(color)).wrap());
}

fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment([rect.left_center(), rect.right_center()], Stroke::new(1.0, BORDER));
}

fn status_color(status: &str) -> Color32 {
    match status.to_ascii_lowercase().as_str() {
        "working" | "busy" | "running" | "thinking" => ACCENT,
        "blocked" | "error" | "failed" => DANGER,
        "idle" | "done" | "ready" | "waiting" => MAP_INK,
        _ => FAINT,
    }
}

/// An agent kind as shown: the service may not know it.
fn kind_label(kind: &str) -> &str {
    let kind = kind.trim();
    if kind.is_empty() { "agent" } else { kind }
}

fn input_id() -> Id {
    Id::new("map_composer_input")
}

/// The clickable label half of a draft chip (it opens the draft in the field).
fn draft_body_id(index: usize) -> Id {
    Id::new(("map_composer_draft", index, false))
}

/// The × half of a draft chip (it removes the draft).
fn draft_remove_id(index: usize) -> Id {
    Id::new(("map_composer_draft", index, true))
}

fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.painter().layout_no_wrap(text.to_string(), font, Color32::WHITE).size().x
}

fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live_tests::{flat, has_text};
    use egui::{Event, PointerButton, RawInput, Shape};

    /// The composer's bar alone, drawn headless: its strip of drafts, its field, target and Send.
    struct Bar {
        ctx: egui::Context,
        state: State,
        /// The drafts a send from this selection would carry (all of them in these tests).
        included: Vec<usize>,
        time: f64,
    }

    impl Bar {
        /// A bar with `(card, text)` drafts.
        fn new(drafts: &[(&str, &str)]) -> Self {
            let mut state = State::default();
            for (node, text) in drafts {
                state.drafts.push(Draft {
                    text: (*text).to_string(),
                    node: Some((*node).to_string()),
                    page: Some("Page".to_string()),
                    node_id: None,
                    page_id: None,
                });
            }
            let included = (0..state.drafts.len()).collect();
            let mut bar = Self { ctx: egui::Context::default(), state, included, time: 0.0 };
            bar.frame(vec![]);
            bar
        }

        /// One frame of the bar: whether it asked to send, and what it painted.
        fn frame(&mut self, events: Vec<Event>) -> (bool, Vec<Shape>) {
            self.time += 1.0 / 60.0;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 900.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let state = &mut self.state;
            let included = &self.included;
            let mut send = false;
            let mut out = self.ctx.run_ui(input, |ui| {
                ui.set_width(760.0);
                let names = Names { node: Some("Card"), page: Some("Page"), node_id: None, page_id: None };
                send = body(ui, state, "COMMENTING ON · Card · 0 comments", &names, true, true, included, 0).send;
            });
            out.textures_delta.clear();
            (send, flat(&out.shapes))
        }

        fn rect(&self, id: Id) -> Rect {
            self.ctx.read_response(id).map_or_else(|| panic!("no widget {id:?}"), |response| response.rect)
        }

        /// A full press and release on `at`, then a quiet frame.
        fn click(&mut self, at: Pos2) {
            for pressed in [true, false] {
                self.frame(vec![
                    Event::PointerMoved(at),
                    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE },
                ]);
            }
            self.frame(vec![Event::PointerMoved(at)]);
        }

        fn click_chip(&mut self, index: usize) {
            let at = self.rect(draft_body_id(index)).center();
            self.click(at);
        }

        fn click_remove(&mut self, index: usize) {
            let at = self.rect(draft_remove_id(index)).center();
            self.click(at);
        }

        /// Where the field's caret is now, in characters.
        fn caret(&self) -> Option<usize> {
            egui::TextEdit::load_state(&self.ctx, input_id()).and_then(|state| state.cursor.char_range()).map(|range| range.primary.index.0)
        }

        fn key(&mut self, key: Key, modifiers: Modifiers) -> bool {
            let (send, _) = self.frame(vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }]);
            send
        }
    }

    #[test]
    fn clicking_a_chip_loads_that_draft_into_the_field() {
        let mut bar = Bar::new(&[("Alpha", "Fix the lane"), ("Beta", "Merge agents")]);
        bar.click_chip(0);
        assert_eq!(bar.state.input, "Fix the lane");
        assert_eq!(bar.state.editing, Some(0));
        assert!(!bar.state.drafts.is_empty(), "opening a draft does not send or remove it");

        bar.frame(vec![]);
        assert!(bar.ctx.memory(|m| m.has_focus(input_id())), "the field takes the keyboard");
        assert_eq!(bar.caret(), Some("Fix the lane".chars().count()), "the caret waits at the end");
        let (send, shapes) = bar.frame(vec![]);
        assert!(!send);
        assert!(has_text(&shapes, "Fix the lane"), "the chip stays in the strip, showing the draft's text");
    }

    #[test]
    fn the_remove_control_deletes_only_that_draft() {
        let mut bar = Bar::new(&[("Alpha", "Fix the lane"), ("Beta", "Merge agents")]);
        bar.click_chip(0);
        assert_eq!(bar.state.editing, Some(0));

        bar.click_remove(0);
        assert_eq!(bar.state.drafts.len(), 1);
        assert_eq!(bar.state.drafts[0].text, "Merge agents");
        assert_eq!(bar.state.editing, None, "removing the edited draft ends the edit");
        assert!(bar.state.input.is_empty());

        let (_, shapes) = bar.frame(vec![]);
        assert!(has_text(&shapes, "Merge agents"));
        assert!(!has_text(&shapes, "Fix the lane"));
    }

    #[test]
    fn a_second_click_on_the_active_chip_returns_to_a_new_comment() {
        let mut bar = Bar::new(&[("Alpha", "Fix the lane"), ("Beta", "Merge agents")]);
        bar.click_chip(0);
        assert_eq!(bar.state.editing, Some(0));
        bar.click_chip(0);
        assert_eq!(bar.state.editing, None);
        assert!(bar.state.input.is_empty());
        assert_eq!(bar.state.drafts.len(), 2, "a second click must not delete the draft");
    }

    #[test]
    fn sending_while_editing_sends_the_edited_text_and_clears_editing() {
        let mut bar = Bar::new(&[("Alpha", "Fix the lane")]);
        bar.click_chip(0);
        bar.state.input = "Fix the lane: Lane Shared is gone".to_string();

        assert!(bar.key(Key::Enter, Modifiers::COMMAND), "Cmd+Enter asks for a send");
        assert_eq!(bar.state.editing, None);
        assert!(bar.state.input.is_empty(), "the edit is not sent a second time as the prompt");
        assert_eq!(bar.state.drafts[0].text, "Fix the lane: Lane Shared is gone");
    }

    #[test]
    fn shift_enter_saves_the_edit_into_the_draft() {
        let mut bar = Bar::new(&[("Alpha", "Fix the lane")]);
        bar.click_chip(0);
        bar.state.input = "Fix the lane: renamed".to_string();

        assert!(!bar.key(Key::Enter, Modifiers::SHIFT), "saving is not sending");
        assert_eq!(bar.state.editing, None);
        assert!(bar.state.input.is_empty());
        assert_eq!(bar.state.drafts[0].text, "Fix the lane: renamed");
    }

    #[test]
    fn escape_and_backspace_stop_editing_and_leave_the_draft() {
        let mut bar = Bar::new(&[("Alpha", "Fix the lane")]);
        bar.click_chip(0);
        bar.key(Key::Escape, Modifiers::NONE);
        assert_eq!(bar.state.editing, None);
        assert!(bar.state.input.is_empty());
        assert_eq!(bar.state.drafts[0].text, "Fix the lane");

        bar.click_chip(0);
        bar.state.input.clear();
        bar.key(Key::Backspace, Modifiers::NONE);
        assert_eq!(bar.state.editing, None);
        assert_eq!(bar.state.drafts[0].text, "Fix the lane", "Backspace keeps the draft");
    }
}

#[cfg(test)]
mod tree_tests {
    use super::*;

    fn draft(text: &str, node: Option<(&str, u64)>) -> Draft {
        Draft {
            text: text.to_string(),
            node: node.map(|(name, _)| name.to_string()),
            page: Some("Overview".to_string()),
            node_id: node.map(|(_, id)| NodeId(id)),
            page_id: Some(NodeId(1)),
        }
    }

    #[test]
    fn the_tree_counts_the_root_and_lists_each_card_below_it() {
        let drafts = vec![
            draft("on the card", Some(("Components", 10))),
            draft("a variant", Some(("Variants", 11))),
            draft("an override", Some(("Overrides", 12))),
            draft("another override", Some(("Overrides", 12))),
        ];
        let (total, rows) = tree_rows(&drafts, &[0, 1, 2, 3], Some(NodeId(10)));
        assert_eq!(total, 4);
        assert_eq!(
            rows,
            vec![TreeRow { name: "Variants".into(), count: 1 }, TreeRow { name: "Overrides".into(), count: 2 }],
            "the root's own draft counts in the total only"
        );
    }

    #[test]
    fn a_page_tree_groups_page_drafts_under_whole_page() {
        let drafts = vec![draft("page note", None), draft("card note", Some(("Card", 5))), draft("second page note", None)];
        let (total, rows) = tree_rows(&drafts, &[0, 1, 2], None);
        assert_eq!(total, 3);
        assert_eq!(rows, vec![TreeRow { name: "Whole page".into(), count: 2 }, TreeRow { name: "Card".into(), count: 1 }]);
        assert_eq!(tree_rows(&drafts, &[], None), (0, Vec::new()));
    }
}
