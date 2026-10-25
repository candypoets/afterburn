//! The agent breakdown: the header count, and a modal with every agent in the workspace, drawn
//! as the Agent status widget (page 35): one card per agent in its state's colour.

use egui::{Align2, Color32, CornerRadius, FontId, Id, Margin, RichText, Stroke, vec2};

use crate::FigApp;
use crate::a11y;
use crate::compose::TargetInfo;
use crate::theme::{regular, semibold};
use crate::widgets;
use crate::workspaces::Target;

const PANEL: Color32 = Color32::WHITE;
const SOFT: Color32 = Color32::from_rgb(0xF8, 0xF6, 0xF3);
const BORDER: Color32 = Color32::from_rgb(0xE3, 0xE2, 0xDF);
const INK: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x1A);
const MUTED: Color32 = Color32::from_rgb(0x6E, 0x6C, 0x68);
const FAINT: Color32 = Color32::from_rgb(0x9A, 0x97, 0x91);
const ACCENT: Color32 = Color32::from_rgb(0xB5, 0x45, 0x2F);
const TEAL: Color32 = Color32::from_rgb(0x2F, 0x6F, 0x6A);
const DANGER: Color32 = Color32::from_rgb(0x9E, 0x3B, 0x2C);
const AMBER: Color32 = Color32::from_rgb(0xA8, 0x6B, 0x12);
const GREY: Color32 = Color32::from_rgb(0x8A, 0x85, 0x7D);
/// The composer's selected-agent fill.
const CHOSEN: Color32 = Color32::from_rgb(0xEA, 0xF3, 0xF0);
/// An agent card's height, as designed.
const ROW_H: f32 = 66.0;

/// The modal is open.
#[derive(Default)]
pub struct State {
    pub open: bool,
    /// The modal was just opened by a click; ignore that same click for outside-close.
    pub just_opened: bool,
}

/// Header text: `3 agents · 1 running`, in a colour that says whether anything is busy.
pub fn summary(targets: &[TargetInfo], error: Option<&str>) -> (String, Color32) {
    let agents: Vec<&TargetInfo> = targets.iter().filter(|info| !info.new_agent).collect();
    if agents.is_empty() {
        if error.is_some() {
            return ("agents unavailable".to_string(), DANGER);
        }
        return ("no agents".to_string(), FAINT);
    }
    let working = agents.iter().filter(|info| info.status.trim() == "working").count();
    let label = if working > 0 {
        format!("{} agent{} · {} running", agents.len(), plural(agents.len()), working)
    } else {
        format!("{} agent{} · idle", agents.len(), plural(agents.len()))
    };
    (label, if working > 0 { ACCENT } else { TEAL })
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// An agent's state as the agent status widget sorts it: each state has its own colour and says
/// whether a prompt may go to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentState {
    /// Herdr reports it busy: an explicit choice only, and a send asks first.
    Working,
    /// Idle or done: the first idle agent is the default recipient.
    Idle,
    /// Blocked or errored: an explicit choice only, saying why it is waiting.
    Blocked,
    /// Any other status: nothing is sent until its status and connection are known.
    Unknown,
}

impl AgentState {
    pub fn of(status: &str) -> Self {
        match status.trim().to_ascii_lowercase().as_str() {
            "working" | "busy" | "running" | "thinking" => Self::Working,
            "idle" | "done" | "ready" | "waiting" => Self::Idle,
            "blocked" | "error" | "errored" | "failed" => Self::Blocked,
            _ => Self::Unknown,
        }
    }

    pub fn color(self) -> Color32 {
        match self {
            Self::Working => AMBER,
            Self::Idle => TEAL,
            Self::Blocked => DANGER,
            Self::Unknown => GREY,
        }
    }

    /// A prompt may be addressed to it.
    pub fn sendable(self) -> bool {
        self != Self::Unknown
    }
}

/// The pane's short id: `p2` for `w1:p2`.
pub fn short_pane(pane: &str) -> &str {
    let pane = pane.trim();
    pane.rsplit(':').next().unwrap_or(pane)
}

/// The name a card leads with: the agent's own name when it has one, else `Pane 2`.
pub fn row_name(name: Option<&str>, pane: &str) -> String {
    if let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) {
        return name.to_string();
    }
    let short = short_pane(pane);
    match short.strip_prefix('p').filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())) {
        Some(n) => format!("Pane {n}"),
        None if short.is_empty() => "New agent".to_string(),
        None => format!("Pane {short}"),
    }
}

/// An agent kind as a card shows it: `codex` reads `Codex`.
pub fn kind_label(kind: &str) -> String {
    let kind = kind.trim();
    let mut chars = kind.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Agent".to_string(),
    }
}

/// The card's status line: `Claude · p2 · working`.
pub fn status_line(kind: &str, pane: &str, status: &str) -> String {
    let status = status.trim();
    let status = if status.is_empty() { "unknown" } else { status };
    let pane = short_pane(pane);
    if pane.is_empty() { format!("{} · {status}", kind_label(kind)) } else { format!("{} · {pane} · {status}", kind_label(kind)) }
}

/// What a card says when hovered, by state; `task` is what the agent says it is doing.
pub fn state_note(state: AgentState, task: Option<&str>) -> Option<String> {
    match state {
        AgentState::Idle => None,
        AgentState::Working => Some("Working: sending to it asks first.".to_string()),
        AgentState::Blocked => Some(match task.map(str::trim).filter(|task| !task.is_empty()) {
            Some(task) => format!("Waiting: {task}"),
            None => "Blocked: it is waiting for you in Herdr.".to_string(),
        }),
        AgentState::Unknown => Some("Cannot send until status and connection are known.".to_string()),
    }
}

/// One agent, as the agent status widget draws it: a bordered 66 pt card with the state dot, the
/// agent's name, and `Kind · pane · state` in the state's colour. The prompt target wears the
/// composer's selected-agent green; an agent of unknown state cannot be picked. A blocked agent
/// says why it is waiting when its pane title tells.
fn agent_row(ui: &mut egui::Ui, info: &TargetInfo, task: Option<&str>, chosen: bool) -> bool {
    let (kind, pane) = match &info.target {
        Target::Existing { pane, kind, .. } => (kind.as_str(), pane.as_str()),
        Target::New { kind } => (kind.as_str(), ""),
    };
    let state = AgentState::of(&info.status);
    let sense = if state.sendable() { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), sense);
    let hot = resp.hovered() && state.sendable();
    let (fill, stroke) = if chosen {
        (CHOSEN, TEAL)
    } else if hot {
        (SOFT, BORDER)
    } else {
        (PANEL, BORDER)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
    ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
    let tint = state.color();
    ui.painter().circle_filled(egui::pos2(rect.left() + 18.0, rect.top() + 20.0), 4.0, tint);
    let room = (rect.width() - 34.0 - 16.0).max(40.0);
    let name = row_name(None, pane);
    let shown = widgets::elide_font(ui, &name, room, semibold(14.0));
    ui.painter().text(egui::pos2(rect.left() + 34.0, rect.top() + 10.0), Align2::LEFT_TOP, shown, semibold(14.0), INK);
    let status = status_line(kind, pane, &info.status);
    let mut line = status.clone();
    if state == AgentState::Blocked
        && let Some(task) = task.map(str::trim).filter(|task| !task.is_empty())
    {
        line = format!("{line} · {task}");
    }
    let line = widgets::elide_font(ui, &line, room, FontId::monospace(11.0));
    ui.painter().text(egui::pos2(rect.left() + 34.0, rect.top() + 35.0), Align2::LEFT_TOP, line, FontId::monospace(11.0), tint);
    a11y::button(&resp, &format!("{name} · {status}"));
    let resp = match state_note(state, task) {
        Some(note) => resp.on_hover_text(note),
        None => resp,
    };
    resp.clicked() && state.sendable()
}

/// What each agent says it is doing, from the workspace list.
fn agent_tasks(app: &FigApp, agents: &[TargetInfo]) -> Vec<Option<String>> {
    let row = app.ws.rows.iter().find(|row| app.active_workspace.as_deref() == Some(row.id.as_str()));
    agents
        .iter()
        .map(|info| {
            let Target::Existing { pane, .. } = &info.target else { return None };
            row.and_then(|row| row.agents_detail.iter().find(|agent| &agent.pane == pane))
                .and_then(|agent| agent.title.as_deref())
                .map(crate::workspaces::agent_task)
                .filter(|task| !task.is_empty())
        })
        .collect()
}

/// Draw the breakdown when open.
pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    if !app.agents_modal.open {
        return;
    }
    let targets = app.compose.targets.clone();
    let selected = app.compose.target.clone();
    let error = app.compose.targets_error.clone();
    let workspace = app.active_workspace.clone().unwrap_or_default();
    let agents: Vec<TargetInfo> = targets.iter().filter(|info| !info.new_agent).cloned().collect();
    let tasks = agent_tasks(app, &agents);
    let working = agents.iter().filter(|info| AgentState::of(&info.status) == AgentState::Working).count();
    let mut close = false;
    let mut pick: Option<Target> = None;
    let mut refresh = false;

    let area =
        egui::Area::new(Id::new("agents_modal")).anchor(Align2::CENTER_CENTER, vec2(0.0, -24.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::NONE
                .fill(PANEL)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(CornerRadius::same(10))
                .shadow(egui::epaint::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha(46) })
                .inner_margin(Margin::symmetric(20, 16))
                .show(ui, |ui| {
                    ui.set_width(410.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("AGENTS").font(FontId::monospace(9.5)).color(ACCENT));
                        ui.label(RichText::new(workspace).font(FontId::monospace(9.0)).color(MUTED));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(egui::Button::new(RichText::new("×").size(16.0).color(MUTED)).frame(false)).clicked() {
                                close = true;
                            }
                        });
                    });
                    ui.add_space(8.0);
                    let summary = if agents.is_empty() {
                        "No agents in this workspace.".to_string()
                    } else if working > 0 {
                        format!("{} agent{} · {} running", agents.len(), plural(agents.len()), working)
                    } else {
                        format!("{} agent{} · all idle", agents.len(), plural(agents.len()))
                    };
                    ui.label(RichText::new(summary).font(regular(12.0)).color(INK));
                    ui.add_space(10.0);
                    egui::ScrollArea::vertical().id_salt("agents_modal_scroll").max_height(400.0).auto_shrink([false, true]).show(ui, |ui| {
                        if let Some(error) = error.as_deref() {
                            ui.label(RichText::new(error).font(regular(11.0)).color(DANGER));
                            ui.add_space(8.0);
                        }
                        if agents.is_empty() && error.is_none() {
                            ui.label(RichText::new("Start an agent from the composer's target picker.").font(regular(11.0)).color(MUTED));
                        }
                        for (info, task) in agents.iter().zip(&tasks) {
                            let chosen = selected.as_ref() == Some(&info.target);
                            if agent_row(ui, info, task.as_deref(), chosen) {
                                pick = Some(info.target.clone());
                            }
                            ui.add_space(8.0);
                        }
                    });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(RichText::new("Refresh").font(regular(11.0)).color(MUTED)).frame(false)).clicked() {
                            refresh = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new("Click an agent to make it the prompt target.").font(FontId::monospace(8.5)).color(FAINT));
                        });
                    });
                });
        });

    if let Some(target) = pick {
        app.compose.target = Some(target);
    }
    if refresh {
        crate::compose::refresh_targets(app, ctx);
    }
    let pressed = ctx.input(|input| input.pointer.any_pressed());
    let pointer = ctx.input(|input| input.pointer.interact_pos());
    let inside = pointer.is_some_and(|pos| area.response.rect.contains(pos));
    let just_opened = app.agents_modal.just_opened;
    app.agents_modal.just_opened = false;
    if (pressed && !inside && !just_opened) || close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.agents_modal.open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_state_has_the_designed_colour_and_sendability() {
        assert_eq!(AgentState::of("working"), AgentState::Working);
        assert_eq!(AgentState::of(" Idle "), AgentState::Idle);
        assert_eq!(AgentState::of("done"), AgentState::Idle);
        assert_eq!(AgentState::of("blocked"), AgentState::Blocked);
        assert_eq!(AgentState::of("error"), AgentState::Blocked);
        assert_eq!(AgentState::of(""), AgentState::Unknown);
        assert_eq!(AgentState::of("unknown"), AgentState::Unknown);
        assert_eq!(AgentState::Working.color(), Color32::from_rgb(0xA8, 0x6B, 0x12));
        assert_eq!(AgentState::Idle.color(), Color32::from_rgb(0x2F, 0x6F, 0x6A));
        assert_eq!(AgentState::Blocked.color(), Color32::from_rgb(0x9E, 0x3B, 0x2C));
        assert_eq!(AgentState::Unknown.color(), Color32::from_rgb(0x8A, 0x85, 0x7D));
        assert!(AgentState::Working.sendable() && AgentState::Blocked.sendable() && AgentState::Idle.sendable());
        assert!(!AgentState::Unknown.sendable());
    }

    #[test]
    fn the_card_reads_name_then_kind_pane_and_state() {
        assert_eq!(row_name(Some("UI review"), "w1:p2"), "UI review");
        assert_eq!(row_name(None, "w1:p4"), "Pane 4");
        assert_eq!(row_name(Some("  "), "w1:p4"), "Pane 4");
        assert_eq!(row_name(None, ""), "New agent");
        assert_eq!(row_name(None, "w1:x"), "Pane x");
        assert_eq!(status_line("claude", "w1:p2", "working"), "Claude · p2 · working");
        assert_eq!(status_line("codex", "w1:p4", ""), "Codex · p4 · unknown");
        assert_eq!(status_line("", "", "idle"), "Agent · idle");
    }

    /// Draw one card per state, top to bottom, returning each card's rect and whether it was picked.
    fn draw_cards(ctx: &egui::Context, time: f64, events: Vec<egui::Event>, infos: &[TargetInfo]) -> Vec<(egui::Rect, bool)> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(480.0, 600.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut cards = Vec::new();
        ctx.run_ui(input, |ui| {
            ui.set_width(410.0);
            for (index, info) in infos.iter().enumerate() {
                let top = ui.cursor().top();
                let picked = agent_row(ui, info, Some("waiting on review"), index == 1);
                let rect = egui::Rect::from_min_max(egui::pos2(ui.min_rect().left(), top), egui::pos2(ui.min_rect().right(), ui.cursor().top()));
                cards.push((rect, picked));
            }
        })
        .textures_delta
        .clear();
        cards
    }

    #[test]
    fn every_state_draws_a_card_and_only_known_states_can_be_picked() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, false, 0);
        let infos: Vec<TargetInfo> = ["working", "idle", "blocked", "", "a very long status that does not fit anywhere at all"]
            .iter()
            .enumerate()
            .map(|(index, status)| TargetInfo {
                target: Target::Existing { pane: format!("w1:p{index}"), kind: "codex".into(), status: status.to_string() },
                status: status.to_string(),
                focused: false,
                new_agent: false,
            })
            .collect();
        let mut time = 0.0;
        let mut frame = |events: Vec<egui::Event>| {
            time += 1.0 / 60.0;
            draw_cards(&ctx, time, events, &infos)
        };
        frame(vec![]);
        let cards = frame(vec![]);
        assert_eq!(cards.len(), infos.len());
        assert!(cards.iter().all(|(rect, _)| (rect.height() - ROW_H).abs() <= 4.0), "{cards:?}");
        let click = |at: egui::Pos2, frame: &mut dyn FnMut(Vec<egui::Event>) -> Vec<(egui::Rect, bool)>| {
            let press =
                |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
            frame(vec![egui::Event::PointerMoved(at)]);
            frame(vec![press(true)]);
            let picked = frame(vec![press(false)]);
            frame(vec![]);
            picked
        };
        let idle = cards.get(1).map(|(rect, _)| rect.center()).unwrap_or_default();
        let unknown = cards.get(3).map(|(rect, _)| rect.center()).unwrap_or_default();
        assert!(click(idle, &mut frame).get(1).is_some_and(|(_, picked)| *picked));
        assert!(!click(unknown, &mut frame).get(3).is_some_and(|(_, picked)| *picked));
    }

    #[test]
    fn a_blocked_agent_says_why_it_waits() {
        assert_eq!(state_note(AgentState::Blocked, Some("approve the migration")).as_deref(), Some("Waiting: approve the migration"));
        assert!(state_note(AgentState::Blocked, None).is_some());
        assert_eq!(state_note(AgentState::Unknown, None).as_deref(), Some("Cannot send until status and connection are known."));
        assert_eq!(state_note(AgentState::Idle, Some("x")), None);
    }
}
