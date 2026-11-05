//! The New page dialog: a name, a first prompt, and the agent that will design it.
//!
//! `page.add` selects the new page; if a prompt is given it is sent once to the composer's chosen
//! agent with the service's `send` method. The page exists even if the prompt later fails, so a
//! retry never creates a second page.

use std::sync::mpsc::{self, Receiver};

use egui::{Align2, Color32, CornerRadius, Id, Margin, RichText, Stroke, vec2};
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;

use crate::FigApp;
use crate::compose::SendReply;
#[cfg(not(target_arch = "wasm32"))]
use crate::compose::send_request;
use crate::theme::{regular, semibold};
#[cfg(not(target_arch = "wasm32"))]
use crate::workspaces;

/// The node metadata key that points a component at its detail page.
pub const DETAIL_PAGE_KEY: &str = "mindmap.detail_page";

const PANEL: Color32 = Color32::WHITE;
const BORDER: Color32 = Color32::from_rgb(0xE3, 0xE2, 0xDF);
const INK: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x1A);
const MUTED: Color32 = Color32::from_rgb(0x6E, 0x6C, 0x68);
const FAINT: Color32 = Color32::from_rgb(0x9A, 0x97, 0x91);
const ACCENT: Color32 = Color32::from_rgb(0xB5, 0x45, 0x2F);

/// The component a new page is generated from.
#[derive(Clone, Debug)]
pub struct Source {
    pub page_id: u64,
    pub page_name: String,
    pub node_id: u64,
    pub node_name: String,
}

/// The dialog's state.
#[derive(Default)]
pub struct State {
    pub open: bool,
    name: String,
    prompt: String,
    pub(crate) busy: bool,
    /// The page was created; a retry only sends the prompt again.
    created: bool,
    error: Option<String>,
    rx: Option<Receiver<Result<SendReply, String>>>,
    page_id: Option<u64>,
    source: Option<Source>,
    /// The new page was attached below its source page.
    meta_set: bool,
    /// The source component was linked to the new page.
    link_set: bool,
    focus: bool,
}

impl State {
    /// Open with a suggested name.
    pub fn start(&mut self, number: usize) {
        self.open = true;
        self.name = format!("Page {number}");
        self.prompt.clear();
        self.busy = false;
        self.created = false;
        self.error = None;
        self.rx = None;
        self.page_id = None;
        self.source = None;
        self.meta_set = false;
        self.link_set = false;
        self.focus = true;
    }

    /// The card the page being started is a subpage of.
    pub fn source_node(&self) -> Option<u64> {
        self.source.as_ref().map(|source| source.node_id)
    }

    /// Start a new page from a component: inferred title and prompt, editable, with its source.
    pub fn start_component(&mut self, number: usize, source: Source) {
        self.start(number);
        self.name = format!("{} — detail", source.node_name);
        self.prompt = format!(
            "Explain in detail what is inside the \"{}\" component on the \"{}\" page: its purpose, \n\
             the parts it is made of, how they connect, and the code that implements it. \n\
             Draw this new page with sourced cards.",
            source.node_name, source.page_name
        );
        self.source = Some(source);
    }
}

/// Draw the dialog (map mode). It owns its own send thread so the composer is untouched.
/// Right-click › New subpage (`map.newSubpage`): open the dialog for a subpage of the selected
/// card, with its inferred title and prompt. `false` when nothing is selected.
pub fn start_subpage(app: &mut FigApp) -> bool {
    let Some(node_id) = app.session.selection().first().copied() else { return false };
    let Ok(page_id) = app.session.page() else { return false };
    let Ok(doc) = app.session.doc() else { return false };
    let name =
        |id| doc.node(id).map(|node| if node.name.trim().is_empty() { node.kind.type_name().to_lowercase() } else { node.name.trim().to_string() });
    let (Some(node_name), Some(page_name)) = (name(node_id), name(page_id)) else { return false };
    let number = doc.pages.len() + 1;
    app.new_page.start_component(number, Source { page_id: page_id.0, page_name, node_id: node_id.0, node_name });
    true
}

pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    poll(app);
    if !app.new_page.open {
        return;
    }
    let targets = app.compose.targets.clone();
    let mut selected = app.compose.target.clone();
    let mut close = false;
    let mut create = false;
    let busy = app.new_page.busy;
    let created = app.new_page.created;

    egui::Area::new(Id::new("new_page_modal")).anchor(Align2::CENTER_CENTER, vec2(0.0, -20.0)).order(egui::Order::Foreground).show(ctx, |ui| {
        egui::Frame::NONE
            .fill(PANEL)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(CornerRadius::same(10))
            .shadow(egui::epaint::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha(46) })
            .inner_margin(Margin::symmetric(20, 16))
            .show(ui, |ui| {
                ui.set_width(420.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("NEW PAGE").font(egui::FontId::monospace(9.5)).color(ACCENT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new(RichText::new("×").size(16.0).color(MUTED)).frame(false)).clicked() {
                            close = true;
                        }
                    });
                });
                if let Some(source) = app.new_page.source.as_ref() {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!("From component · {} · {}", source.node_name, source.page_name))
                            .font(egui::FontId::monospace(8.8))
                            .color(ACCENT),
                    );
                }
                ui.add_space(10.0);
                ui.label(RichText::new("Name").font(semibold(11.0)).color(INK));
                ui.add_space(4.0);
                let name = ui.add_enabled(
                    !created && !busy,
                    egui::TextEdit::singleline(&mut app.new_page.name).font(regular(13.0)).desired_width(f32::INFINITY).hint_text("Page name"),
                );
                if app.new_page.focus {
                    name.request_focus();
                    app.new_page.focus = false;
                }
                ui.add_space(12.0);
                ui.label(RichText::new("First prompt").font(semibold(11.0)).color(INK));
                ui.add_space(4.0);
                ui.add(
                    egui::TextEdit::multiline(&mut app.new_page.prompt)
                        .font(regular(12.5))
                        .desired_rows(4)
                        .desired_width(f32::INFINITY)
                        .hint_text("What should this page become?"),
                );
                ui.add_space(12.0);
                ui.label(RichText::new("Agent").font(semibold(11.0)).color(INK));
                ui.add_space(4.0);
                if targets.is_empty() {
                    ui.label(RichText::new("No agent list yet; choose one in the composer.").font(regular(11.0)).color(MUTED));
                } else {
                    let current =
                        selected.as_ref().map(|target| kind_label(target.kind()).to_string()).unwrap_or_else(|| "Choose an agent".to_string());
                    egui::ComboBox::from_id_salt("new_page_agent")
                        .selected_text(RichText::new(current).font(regular(12.0)).color(INK))
                        .width(260.0)
                        .show_ui(ui, |ui| {
                            for info in &targets {
                                let label = if info.new_agent {
                                    format!("start {}", kind_label(info.target.kind()))
                                } else if info.status.trim().is_empty() {
                                    kind_label(info.target.kind()).to_string()
                                } else {
                                    format!("{} · {}", kind_label(info.target.kind()), info.status)
                                };
                                if ui.selectable_label(selected.as_ref() == Some(&info.target), label).clicked() {
                                    selected = Some(info.target.clone());
                                }
                            }
                        });
                }
                if let Some(error) = app.new_page.error.clone() {
                    ui.add_space(10.0);
                    ui.label(RichText::new(error).font(regular(11.0)).color(ACCENT));
                }
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    let prompt_missing = app.new_page.prompt.trim().is_empty();
                    let can_create = !app.new_page.name.trim().is_empty()
                        && !app.new_page.name.trim().starts_with('-')
                        && (prompt_missing || selected.is_some())
                        && !busy;
                    let label = if busy {
                        "Sending…"
                    } else if created {
                        "Send prompt"
                    } else if prompt_missing {
                        "Create page"
                    } else {
                        "Create and prompt"
                    };
                    let button = egui::Button::new(RichText::new(label).font(semibold(11.5)).color(Color32::WHITE))
                        .fill(if can_create { ACCENT } else { Color32::from_rgb(0xC9, 0xB7, 0xB2) })
                        .corner_radius(CornerRadius::same(6))
                        .min_size(vec2(150.0, 34.0));
                    if ui.add_enabled(can_create, button).clicked() {
                        create = true;
                    }
                    ui.add_space(8.0);
                    if ui.add_enabled(!busy, egui::Button::new(RichText::new("Cancel").font(regular(11.5)).color(MUTED)).frame(false)).clicked() {
                        close = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new("The page is blank until the agent draws.").font(egui::FontId::monospace(8.5)).color(FAINT));
                    });
                });
            });
    });

    if selected != app.compose.target
        && let Some(target) = selected
    {
        app.compose.choose(target);
    }
    if close && !busy {
        app.new_page.open = false;
    }
    if create {
        submit(app, ctx);
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !busy {
        app.new_page.open = false;
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn submit(app: &mut FigApp, ctx: &egui::Context) {
    let name = app.new_page.name.trim().to_string();
    if name.is_empty() || name.starts_with('-') {
        app.new_page.error = Some("Name the page; a leading '-' would make it a divider.".to_string());
        return;
    }
    if !app.new_page.created {
        let Some(created) = app.run("page.add", json!({"name": name})) else {
            app.new_page.error = Some(app.ui.last_error.take().unwrap_or_else(|| "The page could not be created.".to_string()));
            return;
        };
        app.new_page.page_id = created.get("id").and_then(serde_json::Value::as_u64);
        app.new_page.created = true;
    }
    if !app.new_page.meta_set
        && let (Some(source), Some(page_id)) = (app.new_page.source.clone(), app.new_page.page_id)
    {
        let limit = app.session.doc().map(|doc| crate::page_tree::depth(doc, figcraft_doc::NodeId(source.page_id))).unwrap_or(0);
        if limit >= 4 {
            app.new_page.error = Some("Subpages can nest at most five levels deep.".to_string());
            return;
        }
        if let Err(error) = crate::page_tree::set_subpage(app, page_id, source.page_id, source.node_id) {
            app.new_page.error = Some(format!("The page was created, but its tree position failed: {error}"));
            return;
        }
        app.new_page.meta_set = true;
    }
    if !app.new_page.link_set
        && let (Some(source), Some(page_id)) = (app.new_page.source.clone(), app.new_page.page_id)
    {
        let params = json!({
            "id": source.node_id,
            "pluginData": { DETAIL_PAGE_KEY: page_id.to_string() },
        });
        if let Err(error) = app.session.execute("node.set", &params) {
            app.new_page.error = Some(format!("The page was created, but the component link failed: {error}"));
            return;
        }
        app.new_page.link_set = true;
    }
    let prompt = app.new_page.prompt.trim().to_string();
    if prompt.is_empty() {
        app.new_page.open = false;
        app.toast(format!("Created {name}"));
        return;
    }
    let Some(workspace) = app.active_workspace.clone() else {
        app.new_page.error = Some("Open a workspace map before prompting an agent.".to_string());
        return;
    };
    let Some(target) = app.compose.target.clone() else {
        app.new_page.error = Some("Choose an agent for the first prompt.".to_string());
        return;
    };
    let text = prompt;
    let node = match workspaces::active_node(app) {
        Ok(node) => node,
        Err(error) => {
            app.new_page.error = Some(error);
            return;
        }
    };
    let request = json!({
        "id": 1,
        "method": "send",
        "workspace": workspace,
        "session": node.session,
        "target": target.line(),
        "prompt": text,
        "comments": [],
        "page_id": app.new_page.page_id,
        "new_page": true,
        "source_page_id": app.new_page.source.as_ref().map(|source| source.page_id),
        "source_node_id": app.new_page.source.as_ref().map(|source| source.node_id),
    })
    .to_string();
    let (tx, rx) = mpsc::channel();
    app.new_page.busy = true;
    app.new_page.error = None;
    app.new_page.rx = Some(rx);
    let address = node.address;
    let context = ctx.clone();
    let spawned = std::thread::Builder::new().name("figcraft-new-page-prompt".into()).spawn(move || {
        let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| send_request(&address, &request)))
            .unwrap_or_else(|_| Err("The first prompt failed unexpectedly.".to_string()));
        let _ = tx.send(reply);
        context.request_repaint();
    });
    if let Err(error) = spawned {
        app.new_page.busy = false;
        app.new_page.rx = None;
        app.new_page.error = Some(format!("Could not send the first prompt: {error}"));
    }
}

#[cfg(target_arch = "wasm32")]
fn submit(app: &mut FigApp, _ctx: &egui::Context) {
    app.new_page.error = Some("Node workspaces are only available in the desktop app.".to_string());
}

fn poll(app: &mut FigApp) {
    let received = app.new_page.rx.as_ref().map(Receiver::try_recv);
    let received = match received {
        None => return,
        Some(Ok(reply)) => reply,
        Some(Err(mpsc::TryRecvError::Empty)) => return,
        Some(Err(mpsc::TryRecvError::Disconnected)) => {
            app.new_page.rx = None;
            app.new_page.busy = false;
            app.new_page.error = Some("The first prompt ended without an answer.".to_string());
            return;
        }
    };
    app.new_page.rx = None;
    app.new_page.busy = false;
    match received {
        Ok(reply) if reply.ok => {
            app.new_page.open = false;
            let agent = reply.agent.as_ref().and_then(|agent| agent.get("kind").and_then(serde_json::Value::as_str)).unwrap_or("the agent");
            app.toast(format!("Created {}; prompt sent to {agent}", app.new_page.name.trim()));
            // The agent marks the page's mode; the tree says when it makes it an exploration.
            if let Some(page) = app.new_page.page_id {
                crate::page_tree::watch_new(app, figcraft_doc::NodeId(page));
            }
        }
        Ok(reply) => {
            app.new_page.error = Some(reply.error.unwrap_or_else(|| "The prompt was refused.".to_string()));
        }
        Err(error) => app.new_page.error = Some(error),
    }
}

fn kind_label(kind: &str) -> &str {
    match kind {
        "codex" => "codex",
        "claude" => "claude",
        "kimi" => "kimi",
        "dsh" | "dsh-tui" => "dsh",
        other => other,
    }
}
