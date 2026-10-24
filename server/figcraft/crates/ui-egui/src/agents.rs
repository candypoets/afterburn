//! The agent breakdown: the header count, and a modal with every agent in the workspace.

use egui::{Align2, Color32, CornerRadius, FontId, Id, Margin, RichText, Stroke, vec2};

use crate::FigApp;
use crate::a11y;
use crate::compose::TargetInfo;
use crate::theme::{regular, semibold};
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

/// The modal is open.
#[derive(Default)]
pub struct State {
    pub open: bool,
    /// The modal was just opened by a click; ignore that same click for outside-close.
    pub just_opened: bool,
}

/// Header text: `3 agents · 1 working`, in a colour that says whether anything is busy.
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

fn status_color(status: &str) -> Color32 {
    match status.trim() {
        "working" => ACCENT,
        "blocked" | "error" | "errored" | "failed" => DANGER,
        "idle" | "done" => TEAL,
        _ => FAINT,
    }
}

/// One agent, as the agent status widget draws it: a bordered 31 pt row with the state dot, the
/// agent's kind and its Herdr state. The prompt target wears the accent border; the pane id and
/// whether Herdr has it focused sit quietly at the right.
fn agent_row(ui: &mut egui::Ui, info: &TargetInfo, chosen: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 31.0), egui::Sense::click());
    let hot = resp.hovered();
    ui.painter().rect_filled(rect, CornerRadius::same(8), if hot { SOFT } else { PANEL });
    ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0, if chosen { ACCENT } else { BORDER }), egui::StrokeKind::Inside);
    let (kind, pane) = match &info.target {
        Target::Existing { pane, kind, .. } => (kind.as_str(), pane.clone()),
        Target::New { kind } => (kind.as_str(), "new agent".to_string()),
    };
    let status = info.status.trim();
    let status = if status.is_empty() { "unknown" } else { status };
    ui.painter().circle_filled(egui::pos2(rect.left() + 17.0, rect.center().y), 4.0, status_color(status));
    let kind_w = ui.painter().layout_no_wrap(kind.to_string(), semibold(11.5), INK).size().x;
    ui.painter().text(egui::pos2(rect.left() + 31.0, rect.center().y), Align2::LEFT_CENTER, kind, semibold(11.5), INK);
    ui.painter().text(egui::pos2(rect.left() + 41.0 + kind_w, rect.center().y), Align2::LEFT_CENTER, status, FontId::monospace(9.5), MUTED);
    let trail = match (info.focused, chosen) {
        (true, true) => format!("{pane} · focused · target"),
        (true, false) => format!("{pane} · focused"),
        (false, true) => format!("{pane} · target"),
        (false, false) => pane,
    };
    ui.painter().text(
        egui::pos2(rect.right() - 12.0, rect.center().y),
        Align2::RIGHT_CENTER,
        trail,
        FontId::monospace(8.5),
        if chosen { ACCENT } else { FAINT },
    );
    a11y::button(&resp, &format!("{kind} · {status}"));
    resp.clicked()
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
    let working = agents.iter().filter(|info| info.status.trim() == "working").count();
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
                    ui.set_width(440.0);
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
                    egui::ScrollArea::vertical().id_salt("agents_modal_scroll").max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
                        if let Some(error) = error.as_deref() {
                            ui.label(RichText::new(error).font(regular(11.0)).color(DANGER));
                            ui.add_space(8.0);
                        }
                        if agents.is_empty() && error.is_none() {
                            ui.label(RichText::new("Start an agent from the composer's target picker.").font(regular(11.0)).color(MUTED));
                        }
                        for info in &agents {
                            let chosen = selected.as_ref() == Some(&info.target);
                            if agent_row(ui, info, chosen) {
                                pick = Some(info.target.clone());
                            }
                            ui.add_space(6.0);
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
