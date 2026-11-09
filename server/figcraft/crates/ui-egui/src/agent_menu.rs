//! The one agent button and its menu, the same on every screen that asks for an agent: the
//! composer, the Sync panel and Init (the Empty map screen).
//!
//! Design: `State Agent menu` on the Afterburn page. The button is a component whose label is the
//! selected agent (`Backend · Codex · p1 · idle ⌄`); clicking it opens a search field
//! (`Find an agent…`), the workspace's agents in a list that scrolls, and the CLIs a new agent can
//! start pinned below. Init shows busy (working or blocked) agents disabled; the composer lets the
//! human pick one and confirms at Send instead.

use egui::{Align2, Color32, CornerRadius, FontId, Id, Key, Margin, Modifiers, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use crate::a11y;
use crate::compose::{Readiness, TargetInfo, readiness};
use crate::theme::regular;
use crate::widgets;
use crate::workspaces::{AgentDetail, Target};

/// The menu's width, and the scrolling list's height (six rows and a peek at the seventh).
pub(crate) const MENU_W: f32 = 300.0;
const MENU_LIST_H: f32 = 160.0;
const MENU_ROW_H: f32 = 26.0;

/// The warm map palette, as the composer draws it.
#[allow(non_snake_case)]
fn PANEL() -> Color32 {
    crate::map_colors::get().panel
}
#[allow(non_snake_case)]
fn SOFT() -> Color32 {
    crate::map_colors::get().soft
}
#[allow(non_snake_case)]
fn BORDER() -> Color32 {
    crate::map_colors::get().border
}
#[allow(non_snake_case)]
fn INK() -> Color32 {
    crate::map_colors::get().ink
}
#[allow(non_snake_case)]
fn MUTED() -> Color32 {
    crate::map_colors::get().muted
}
#[allow(non_snake_case)]
fn FAINT() -> Color32 {
    crate::map_colors::get().faint
}
#[allow(non_snake_case)]
fn ACCENT() -> Color32 {
    crate::map_colors::get().accent
}
#[allow(non_snake_case)]
fn MAP_INK() -> Color32 {
    crate::map_colors::get().map_ink
}
#[allow(non_snake_case)]
fn DANGER() -> Color32 {
    crate::map_colors::get().danger
}
/// Agent status inks, as the menu draws its dots.
const IDLE: Color32 = Color32::from_rgb(0x2F, 0x7A, 0x4F);
#[allow(non_snake_case)]
fn WORKING() -> Color32 {
    crate::map_colors::get().working_ink
}
const DONE: Color32 = Color32::from_rgb(0x41, 0x6F, 0x91);
/// The button's fill: green for a ready agent, amber for a working one, red for blocked.
const READY_FILL: Color32 = Color32::from_rgb(0xEA, 0xF3, 0xF0);
const WORKING_FILL: Color32 = Color32::from_rgb(0xFF, 0xF4, 0xDB);
const BLOCKED_FILL: Color32 = Color32::from_rgb(0xFB, 0xED, 0xEA);
/// The selected row in the menu.
#[allow(non_snake_case)]
fn CHOSEN_FILL() -> Color32 {
    crate::map_colors::get().soft
}

/// The menu's own state; whether it is open belongs to the screen that owns the button.
#[derive(Clone, Debug, Default)]
pub struct MenuState {
    /// The search text.
    pub(crate) query: String,
    /// The agent row the arrows are on, an index into the filtered rows.
    pub(crate) cursor: usize,
    /// Give the search field the keyboard on its next draw (the menu just opened).
    focus: bool,
}

impl MenuState {
    /// The menu opens on an empty search with the keyboard in it.
    pub(crate) fn opened(&mut self) {
        self.query.clear();
        self.cursor = 0;
        self.focus = true;
    }
}

/// How one screen uses the menu.
#[derive(Clone, Copy)]
pub(crate) struct Options<'a> {
    /// Where the menu opens against the button (above it in the composer, below elsewhere).
    pub align: egui::RectAlign,
    /// Busy agents are shown but can't be picked (Init).
    pub disable_busy: bool,
    /// The agent list is being read.
    pub loading: bool,
    /// Why the list is empty, when the node could not answer.
    pub error: Option<&'a str>,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Self { align: egui::RectAlign::BOTTOM_START, disable_busy: false, loading: false, error: None }
    }
}

/// An agent in the middle of something (or waiting on its human): Init can't pick it.
pub(crate) fn is_busy(status: &str) -> bool {
    matches!(readiness(status), Readiness::Working | Readiness::Blocked)
}

/// Whether the menu lets the human pick `info`.
fn pickable(info: &TargetInfo, options: &Options<'_>) -> bool {
    !(options.disable_busy && !info.new_agent && is_busy(&info.status))
}

/// The button's label for the chosen target: the agent's identity, `New codex agent` for a CLI,
/// or `fallback` when nobody is chosen.
pub(crate) fn button_label(targets: &[TargetInfo], target: Option<&Target>, fallback: &str) -> String {
    match target {
        Some(Target::New { kind }) => format!("New {} agent", kind_label(kind)),
        Some(target) => {
            let line = target.line();
            match targets.iter().find(|info| info.target.line() == line) {
                Some(info) => recipient(info, false),
                None => "Agent gone · choose again".to_string(),
            }
        }
        None => fallback.to_string(),
    }
}

/// The button's width for its label, within `min..=max`.
pub(crate) fn button_width(ui: &Ui, label: &str, min: f32, max: f32) -> f32 {
    (text_width(ui, label, regular(11.0)) + 46.0).clamp(min, max.max(min))
}

/// The agent button. Its colour is the chosen pane's readiness: green when idle, amber when
/// working, red when blocked; a new CLI or nobody is plain. Clicking it opens or closes the menu.
pub(crate) fn button(ui: &mut Ui, id: Id, targets: &[TargetInfo], target: Option<&Target>, label: &str, size: Vec2, open: bool) -> Response {
    let (_, rect) = ui.allocate_space(size);
    let resp = ui.interact(rect, id, Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    a11y::button(&resp, &format!("Agent: {label}. Choose the agent"));
    let line = target.map(Target::line);
    let chosen = line.as_ref().and_then(|line| targets.iter().find(|info| &info.target.line() == line));
    let (fill, stroke, ink) = match (target, chosen) {
        (Some(Target::New { .. }), _) => (PANEL(), BORDER(), INK()),
        (Some(_), Some(info)) => match readiness(&info.status) {
            Readiness::Ready => (READY_FILL, MAP_INK(), INK()),
            Readiness::Working => (WORKING_FILL, WORKING(), WORKING()),
            Readiness::Blocked => (BLOCKED_FILL, DANGER(), DANGER()),
            Readiness::Unavailable => (PANEL(), BORDER(), MUTED()),
        },
        _ => (PANEL(), BORDER(), MUTED()),
    };
    let enabled = ui.is_enabled();
    let fill = if enabled && (resp.hovered() || open) { fill.lerp_to_gamma(SOFT(), 0.35) } else { fill };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, stroke), StrokeKind::Inside);
    let shown = widgets::elide_font(ui, label, (rect.width() - 40.0).max(24.0), regular(11.0));
    ui.painter().text(pos2(rect.min.x + 12.0, rect.center().y), Align2::LEFT_CENTER, &shown, regular(11.0), ink);
    // A drawn chevron, so no font has to have the glyph.
    let c = pos2(rect.max.x - 14.0, rect.center().y);
    let chevron = Stroke::new(1.2, ink);
    ui.painter().line_segment([pos2(c.x - 3.5, c.y - 1.5), pos2(c.x, c.y + 2.0)], chevron);
    ui.painter().line_segment([pos2(c.x, c.y + 2.0), pos2(c.x + 3.5, c.y - 1.5)], chevron);
    resp
}

/// The menu against `anchor` while `open`: the pick, if any. Esc, a click outside or a pick
/// closes it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn popup(
    ui: &mut Ui,
    id: Id,
    anchor: &Response,
    open: &mut bool,
    state: &mut MenuState,
    targets: &[TargetInfo],
    chosen: Option<&Target>,
    options: Options<'_>,
) -> Option<Target> {
    if !*open {
        return None;
    }
    let frame = egui::Frame::NONE
        .fill(PANEL())
        .stroke(Stroke::new(1.0, BORDER()))
        .corner_radius(CornerRadius::same(8))
        .shadow(egui::epaint::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(24) })
        .inner_margin(Margin::same(12));
    let mut picked = None;
    egui::Popup::new(id, ui.ctx().clone(), anchor, anchor.layer_id)
        .width(MENU_W - 24.0)
        .gap(6.0)
        .align(options.align)
        .frame(frame)
        .open_bool(open)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            picked = body(ui, id, state, targets, chosen, &options);
        });
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        *open = false;
    }
    if picked.is_some() {
        *open = false;
    }
    picked
}

/// The agents a search keeps, in the list's stable order: every word of the query must appear in
/// the agent's label, kind, pane or status.
pub(crate) fn filter_agents<'a>(targets: &'a [TargetInfo], query: &str) -> Vec<&'a TargetInfo> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    targets
        .iter()
        .filter(|info| !info.new_agent)
        .filter(|info| {
            let Target::Existing { pane, kind, .. } = &info.target else { return false };
            let haystack = format!("{} {} {} {} {}", info.label, kind, pane_short(pane), pane, status_text(&info.status)).to_lowercase();
            words.iter().all(|word| haystack.contains(word.as_str()))
        })
        .collect()
}

/// The search field, the list and the CLIs; the pick, if any.
pub(crate) fn body(
    ui: &mut Ui,
    id: Id,
    state: &mut MenuState,
    targets: &[TargetInfo],
    chosen: Option<&Target>,
    options: &Options<'_>,
) -> Option<Target> {
    let mut picked = None;
    ui.set_width(ui.available_width());
    ui.spacing_mut().item_spacing = vec2(6.0, 2.0);

    // The keys the list owns are taken before the search field sees them: arrows move, Enter picks.
    let total = filter_agents(targets, &state.query).len();
    let (down, up, enter) = ui.input_mut(|i| {
        (i.consume_key(Modifiers::NONE, Key::ArrowDown), i.consume_key(Modifiers::NONE, Key::ArrowUp), i.consume_key(Modifiers::NONE, Key::Enter))
    });
    let moved = down || up;
    if down {
        state.cursor = (state.cursor + 1).min(total.saturating_sub(1));
    }
    if up {
        state.cursor = state.cursor.saturating_sub(1);
    }

    let before = state.query.clone();
    let search = egui::Frame::NONE
        .fill(Color32::from_rgb(0xF6, 0xF5, 0xF2))
        .stroke(Stroke::new(1.0, BORDER()))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut state.query)
                    .id(search_id(id))
                    .frame(egui::Frame::NONE)
                    .font(regular(11.5))
                    .hint_text(egui::RichText::new("Find an agent…").font(regular(11.5)).color(MUTED()))
                    .desired_width(ui.available_width()),
            )
        })
        .inner;
    if state.focus {
        search.request_focus();
        state.focus = false;
    }
    if state.query != before {
        state.cursor = 0;
    }
    ui.add_space(8.0);

    let agents = filter_agents(targets, &state.query);
    let count = targets.iter().filter(|info| !info.new_agent).count();
    state.cursor = state.cursor.min(agents.len().saturating_sub(1));
    if enter && let Some(info) = agents.get(state.cursor).filter(|info| pickable(info, options)) {
        picked = Some(info.target.clone());
    }
    eyebrow(ui, &format!("AGENTS IN THIS WORKSPACE · {count}"));
    ui.add_space(2.0);
    if !agents.is_empty() {
        let chosen = chosen.map(Target::line);
        egui::ScrollArea::vertical().id_salt((id, "list")).max_height(MENU_LIST_H).auto_shrink([false, true]).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            for (index, info) in agents.iter().enumerate() {
                let selected = chosen.as_deref() == Some(info.target.line().as_str());
                let enabled = pickable(info, options);
                let resp = agent_row(ui, info, selected, index == state.cursor, enabled);
                if moved && index == state.cursor {
                    resp.scroll_to_me(None);
                }
                if resp.clicked() && enabled {
                    picked = Some(info.target.clone());
                }
            }
        });
    } else if options.loading && count == 0 {
        paragraph(ui, "Looking for agents…", 11.0, MUTED());
    } else if count == 0 {
        paragraph(ui, "No agents in this workspace yet.", 11.0, MUTED());
    } else {
        paragraph(ui, "No agent matches the search.", 11.0, MUTED());
    }
    if let Some(error) = options.error {
        paragraph(ui, error, 10.5, DANGER());
    }

    ui.add_space(8.0);
    divider(ui);
    ui.add_space(8.0);
    eyebrow(ui, "START A NEW AGENT");
    ui.add_space(4.0);
    let clis: Vec<&TargetInfo> = targets.iter().filter(|info| info.new_agent).collect();
    if clis.is_empty() {
        paragraph(ui, "No agent CLIs installed on the project host.", 11.0, MUTED());
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for info in clis {
                let selected = chosen == Some(&info.target);
                if cli_chip(ui, info.target.kind(), selected) {
                    picked = Some(info.target.clone());
                }
            }
        });
    }
    picked
}

/// One agent in the menu: its status dot, its label, and `Kind · pN · status` beside it. A busy
/// agent Init can't take is drawn faint and takes no click.
fn agent_row(ui: &mut Ui, info: &TargetInfo, selected: bool, cursor: bool, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), MENU_ROW_H), if enabled { Sense::click() } else { Sense::hover() });
    let meta = match &info.target {
        Target::Existing { pane, kind, .. } => format!("{} · {} · {}", kind_title(kind), pane_short(pane), status_text(&info.status)),
        Target::New { kind } => kind_label(kind).to_string(),
    };
    a11y::button(&resp, &format!("{} · {meta}", info.label));
    let hot = enabled && (resp.hovered() || cursor);
    if selected || hot {
        ui.painter().rect_filled(rect.shrink2(vec2(2.0, 0.0)), CornerRadius::same(5), if selected { CHOSEN_FILL() } else { SOFT() });
    }
    let dot = pos2(rect.min.x + 12.0, rect.center().y);
    ui.painter().circle_filled(dot, 3.0, status_color(&info.status));
    if info.focused {
        ui.painter().circle_stroke(dot, 5.0, Stroke::new(1.0, MAP_INK()));
    }
    let (ink, meta_ink) = if enabled { (INK(), MUTED()) } else { (FAINT(), FAINT()) };
    let meta_x = rect.min.x + 132.0;
    let label = widgets::elide_font(ui, &info.label, (meta_x - rect.min.x - 32.0).max(24.0), regular(11.5));
    ui.painter().text(pos2(rect.min.x + 24.0, rect.center().y), Align2::LEFT_CENTER, &label, regular(11.5), ink);
    let meta = widgets::elide_font(ui, &meta, (rect.max.x - meta_x - 8.0).max(24.0), regular(10.5));
    ui.painter().text(pos2(meta_x, rect.center().y), Align2::LEFT_CENTER, &meta, regular(10.5), meta_ink);
    if enabled { resp } else { resp.on_hover_text("Busy: it can't take Init now") }
}

/// A CLI to start as a new agent: a small pill under the list.
fn cli_chip(ui: &mut Ui, kind: &str, selected: bool) -> bool {
    let label = kind_label(kind);
    let width = (text_width(ui, label, regular(10.5)) + 24.0).max(58.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 22.0), Sense::click());
    a11y::button(&resp, &format!("Start a new {}", label));
    let hot = resp.hovered();
    let fill = if selected {
        CHOSEN_FILL()
    } else if hot {
        SOFT()
    } else {
        PANEL()
    };
    ui.painter().rect_filled(rect, CornerRadius::same(11), fill);
    let border = if selected || hot { ACCENT() } else { BORDER() };
    ui.painter().rect_stroke(rect, CornerRadius::same(11), Stroke::new(1.0, border), StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, regular(10.5), if selected || hot { ACCENT() } else { INK() });
    resp.clicked()
}

/// The menu's targets from a workspace's agents and the installed CLIs: its agents, then the CLIs
/// to start.
pub(crate) fn targets_of(agents: Vec<AgentDetail>, clis: &[String]) -> Vec<TargetInfo> {
    let mut targets = Vec::new();
    for agent in agents {
        if agent.pane.trim().is_empty() {
            continue;
        }
        let label = agent_label(agent.name.as_deref(), agent.tab.as_deref(), &agent.pane);
        let title = agent.title.map(|title| title.trim().to_string()).filter(|title| !title.is_empty());
        targets.push(TargetInfo {
            target: Target::Existing { pane: agent.pane, kind: agent.kind, status: agent.status.clone() },
            label,
            title,
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
        targets.push(TargetInfo {
            target: Target::New { kind: kind.to_string() },
            label: kind.to_string(),
            title: None,
            status: String::new(),
            focused: false,
            new_agent: true,
        });
    }
    targets
}

/// What the human calls a pane: Herdr's agent name, else its tab's name, else `Pane N`. Never the
/// terminal title: that is the session's task, not the pane's name.
pub(crate) fn agent_label(name: Option<&str>, tab: Option<&str>, pane: &str) -> String {
    [name, tab]
        .into_iter()
        .flatten()
        .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
        .find(|text| !text.is_empty())
        .unwrap_or_else(|| format!("Pane {}", pane_number(pane)))
}

/// One agent's identity: `Backend · Codex · p1 · idle` (`pane 1` in full).
pub(crate) fn recipient(info: &TargetInfo, full: bool) -> String {
    match &info.target {
        Target::New { kind } => format!("New {} agent", kind_label(kind)),
        Target::Existing { pane, kind, .. } => {
            let pane = if full { format!("pane {}", pane_number(pane)) } else { pane_short(pane).to_string() };
            format!("{} · {} · {pane} · {}", info.label, kind_title(kind), status_text(&info.status))
        }
    }
}

/// The search field of the menu with this id.
pub(crate) fn search_id(id: Id) -> Id {
    id.with("search")
}

fn eyebrow(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(FontId::monospace(8.5)).color(MUTED()));
}

fn paragraph(ui: &mut Ui, text: &str, size: f32, color: Color32) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(regular(size)).color(color)).wrap());
}

fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment([rect.left_center(), rect.right_center()], Stroke::new(1.0, BORDER()));
}

/// A status dot's ink: green idle, amber working, red blocked, blue done.
pub(crate) fn status_color(status: &str) -> Color32 {
    match status.trim().to_ascii_lowercase().as_str() {
        "working" | "busy" | "running" | "thinking" => WORKING(),
        "blocked" | "waiting" | "error" | "failed" => ACCENT(),
        "idle" | "ready" => IDLE,
        "done" => DONE,
        _ => FAINT(),
    }
}

/// A status as shown: lowercase, `unknown` when the node has none.
pub(crate) fn status_text(status: &str) -> String {
    let status = status.trim().to_lowercase();
    if status.is_empty() { "unknown".to_string() } else { status }
}

/// An agent kind as shown: the service may not know it.
pub(crate) fn kind_label(kind: &str) -> &str {
    let kind = kind.trim();
    if kind.is_empty() { "agent" } else { kind }
}

/// An agent kind as a recipient names it: `Codex`, `Claude`.
pub(crate) fn kind_title(kind: &str) -> String {
    let mut chars = kind_label(kind).chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// The pane's own part of a Herdr pane id: `p2` for `w1:p2`.
pub(crate) fn pane_short(pane: &str) -> &str {
    let pane = pane.trim();
    pane.rsplit(':').next().filter(|short| !short.is_empty()).unwrap_or(pane)
}

/// The pane's number for `Pane 2` and `pane 2`: `2` for `w1:p2`; the short id when it has none.
pub(crate) fn pane_number(pane: &str) -> &str {
    let short = pane_short(pane);
    short.strip_prefix('p').filter(|number| !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())).unwrap_or(short)
}

fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.painter().layout_no_wrap(text.to_string(), font, Color32::WHITE).size().x
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live_tests::{flat, has_text};
    use egui::{Event, PointerButton, Pos2, RawInput, Rect, Shape};

    fn agent(pane: &str, kind: &str, status: &str, name: &str) -> TargetInfo {
        TargetInfo {
            target: Target::Existing { pane: pane.into(), kind: kind.into(), status: status.into() },
            label: name.into(),
            title: None,
            status: status.into(),
            focused: false,
            new_agent: false,
        }
    }

    /// The menu's body alone, headless, for Init (busy agents disabled) or the composer.
    struct Menu {
        ctx: egui::Context,
        state: MenuState,
        targets: Vec<TargetInfo>,
        disable_busy: bool,
        time: f64,
    }

    impl Menu {
        fn frame(&mut self, events: Vec<Event>) -> (Option<Target>, Vec<Shape>) {
            self.time += 1.0 / 60.0;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut picked = None;
            let (state, targets) = (&mut self.state, &self.targets);
            let options = Options { disable_busy: self.disable_busy, ..Options::default() };
            let mut out = self.ctx.run_ui(input, |ui| {
                ui.set_width(MENU_W - 24.0);
                picked = body(ui, Id::new("test_menu"), state, targets, None, &options);
            });
            out.textures_delta.clear();
            (picked, flat(&out.shapes))
        }

        fn click_row(&mut self, label: &str) -> Option<Target> {
            // Rows are 26 pt tall under the search field and the eyebrow; find the row by its text.
            let (_, shapes) = self.frame(vec![]);
            let at = shapes
                .iter()
                .find_map(|shape| match shape {
                    Shape::Text(text) if text.galley.text() == label => Some(text.pos + vec2(4.0, 4.0)),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("no row {label:?}"));
            let mut picked = None;
            for pressed in [true, false] {
                let (got, _) = self.frame(vec![
                    Event::PointerMoved(at),
                    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE },
                ]);
                picked = picked.or(got);
            }
            picked
        }

        fn enter(&mut self) -> Option<Target> {
            self.frame(vec![Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]).0
        }
    }

    fn menu(disable_busy: bool) -> Menu {
        let targets = vec![
            agent("w1:p1", "codex", "working", "Backend"),
            agent("w1:p2", "claude", "blocked", "UI review"),
            agent("w1:p3", "kimi", "idle", "First map"),
            TargetInfo {
                target: Target::New { kind: "codex".into() },
                label: "codex".into(),
                title: None,
                status: String::new(),
                focused: false,
                new_agent: true,
            },
        ];
        Menu { ctx: egui::Context::default(), state: MenuState::default(), targets, disable_busy, time: 0.0 }
    }

    #[test]
    fn init_shows_busy_agents_disabled_in_the_same_menu() {
        let mut init = menu(true);
        let (_, shapes) = init.frame(vec![]);
        for line in ["Find an agent…", "AGENTS IN THIS WORKSPACE · 3", "Backend", "Codex · p1 · working", "START A NEW AGENT", "codex"] {
            assert!(has_text(&shapes, line), "missing {line:?}");
        }
        assert_eq!(init.click_row("Backend"), None, "a working agent can't be picked for Init");
        assert_eq!(init.click_row("UI review"), None, "nor a blocked one");
        assert_eq!(init.enter(), None, "Enter on a busy row picks nothing");
        assert_eq!(init.click_row("First map").map(|t| t.line()).as_deref(), Some("pane:w1:p3"), "an idle agent can");
        assert_eq!(init.click_row("codex").map(|t| t.line()).as_deref(), Some("kind:codex"), "and a new CLI");
    }

    #[test]
    fn the_composer_menu_lets_a_busy_agent_be_picked() {
        let mut composer = menu(false);
        assert_eq!(composer.click_row("Backend").map(|t| t.line()).as_deref(), Some("pane:w1:p1"), "Send confirms it instead");
        assert_eq!(composer.enter().map(|t| t.line()).as_deref(), Some("pane:w1:p1"));
    }

    #[test]
    fn the_button_names_the_chosen_agent() {
        let targets = menu(false).targets;
        let first_map = targets[2].target.clone();
        assert_eq!(button_label(&targets, Some(&first_map), "Choose an agent"), "First map · Kimi · p3 · idle");
        assert_eq!(button_label(&targets, Some(&Target::New { kind: "claude".into() }), ""), "New claude agent");
        let gone = Target::Existing { pane: "w1:p9".into(), kind: "codex".into(), status: "idle".into() };
        assert_eq!(button_label(&targets, Some(&gone), ""), "Agent gone · choose again");
        assert_eq!(button_label(&targets, None, "Choose an agent"), "Choose an agent");
        assert!(is_busy("working") && is_busy("blocked") && !is_busy("idle") && !is_busy("done"));
    }
}
