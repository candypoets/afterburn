//! The agent breakdown: the header count, and a modal with every agent in the workspace, drawn
//! as the Agent status widget (page 35): one card per agent in its state's colour.

use std::sync::mpsc::{self, Receiver};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

use egui::{Align2, Color32, CornerRadius, FontId, Id, Margin, RichText, Stroke, vec2};
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;

use crate::FigApp;
use crate::a11y;
use crate::compose::TargetInfo;
use crate::theme::{regular, semibold};
use crate::widgets;
use crate::workspaces::Target;

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
fn TEAL() -> Color32 {
    crate::map_colors::get().map_ink
}
#[allow(non_snake_case)]
fn DANGER() -> Color32 {
    crate::map_colors::get().danger
}
#[allow(non_snake_case)]
fn AMBER() -> Color32 {
    crate::map_colors::get().working_ink
}
#[allow(non_snake_case)]
fn GREY() -> Color32 {
    crate::map_colors::get().faint
}
/// The composer's selected-agent fill.
#[allow(non_snake_case)]
fn CHOSEN() -> Color32 {
    crate::map_colors::get().selected
}
/// An agent card's height, as designed (`One agents modal` on the agent picker page).
const ROW_H: f32 = 50.0;
/// The modal's inner width.
const MODAL_W: f32 = 380.0;
/// How long each creation step may take before the modal says it failed.
#[cfg(not(target_arch = "wasm32"))]
const STEP_TIMEOUT: Duration = Duration::from_secs(75);

/// The Agents modal: every agent in the workspace, and where a new one is made. The header's
/// count, the composer's agent button and the Sync pane's agent button all open it.
#[derive(Default)]
pub struct State {
    pub open: bool,
    /// The modal was just opened by a click; ignore that same click for outside-close.
    pub just_opened: bool,
    view: View,
    /// The search text and the card the arrows are on.
    query: String,
    cursor: usize,
    /// Give the search (or the name field) the keyboard on the next draw.
    focus: bool,
    /// The new agent's form, kept while the modal is open so Back and Retry keep its values.
    form: Form,
    /// The agent being created, or the last one that was: it keeps going when the modal closes.
    creation: Option<Creation>,
}

/// Which face of the modal shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum View {
    #[default]
    List,
    Form,
    Creating,
    Failed,
}

/// The new agent's form: the CLI, the model (an index into [`models_for`]) and the pane's name.
#[derive(Clone, Debug, Default)]
struct Form {
    kind: String,
    model: usize,
    name: String,
}

/// One step of starting an agent, as the modal shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Todo,
    Running,
    Done,
    Failed,
}

/// The steps, in order: the tab, the CLI, the folder-trust prompt, and the first idle.
const STEPS: usize = 4;

/// An agent being created (or the last attempt).
struct Creation {
    form: Form,
    steps: [Step; STEPS],
    /// The pane, once its tab exists.
    pane: Option<String>,
    /// Why it failed, and the pane's last lines.
    error: Option<String>,
    lines: Vec<String>,
    /// When it started, for the elapsed time.
    started: f64,
    /// It is idle and chosen.
    ready: bool,
    rx: Option<Receiver<Progress>>,
}

/// What the creation thread reports.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
enum Progress {
    /// Step `n` is done; the pane once known.
    Done(usize, Option<String>),
    /// The agent is idle in this pane.
    Ready(String),
    /// Step `n` failed.
    Failed { step: usize, error: String, lines: Vec<String>, pane: Option<String> },
}

/// The models a CLI can start with, as `(argument, label)`; the first is always the CLI's own
/// default. Only Claude's model aliases are offered: other CLIs start on their own setting.
pub(crate) fn models_for(kind: &str) -> Vec<(Option<&'static str>, &'static str)> {
    let mut models = vec![(None, "CLI default")];
    if kind.trim().eq_ignore_ascii_case("claude") {
        models.extend([(Some("opus"), "Opus"), (Some("sonnet"), "Sonnet"), (Some("haiku"), "Haiku")]);
    }
    models
}

impl State {
    /// Open on the list (or on the agent being created) with the keyboard in the search.
    pub fn open_list(&mut self) {
        self.open = true;
        self.just_opened = true;
        self.query.clear();
        self.cursor = 0;
        self.focus = true;
        self.view = match &self.creation {
            Some(creation) if !creation.ready && creation.error.is_some() => View::Failed,
            Some(creation) if !creation.ready => View::Creating,
            _ => View::List,
        };
    }

    /// Open the form for a new `kind` agent, named `name` to start with.
    fn open_form(&mut self, kind: &str, name: &str) {
        if self.form.kind != kind {
            self.form.model = 0;
        }
        self.form.kind = kind.to_string();
        if self.form.name.trim().is_empty() {
            self.form.name = name.to_string();
        }
        self.view = View::Form;
        self.focus = true;
    }
}

/// Header text: `3 agents · 1 running`, in a colour that says whether anything is busy.
pub fn summary(targets: &[TargetInfo], error: Option<&str>) -> (String, Color32) {
    let agents: Vec<&TargetInfo> = targets.iter().filter(|info| !info.new_agent).collect();
    if agents.is_empty() {
        if error.is_some() {
            return ("agents unavailable".to_string(), DANGER());
        }
        return ("no agents".to_string(), FAINT());
    }
    let working = agents.iter().filter(|info| info.status.trim() == "working").count();
    let label = if working > 0 {
        format!("{} agent{} · {} running", agents.len(), plural(agents.len()), working)
    } else {
        format!("{} agent{} · idle", agents.len(), plural(agents.len()))
    };
    (label, if working > 0 { ACCENT() } else { TEAL() })
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
            Self::Working => AMBER(),
            Self::Idle => TEAL(),
            Self::Blocked => DANGER(),
            Self::Unknown => GREY(),
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

/// One agent, as the One agents modal draws it: a bordered 50 pt card with the kind's mark (the
/// status rides on it; a kind without a mark keeps a dot and its name), the agent's name, and
/// `p2 · working` in the state's colour. The prompt target wears the composer's selected-agent
/// green, the arrows' card a soft fill; an agent of unknown state cannot be picked. A blocked
/// agent says why it is waiting when its pane title tells.
fn agent_row(ui: &mut egui::Ui, info: &TargetInfo, task: Option<&str>, chosen: bool, cursor: bool, new: bool) -> egui::Response {
    let state = AgentState::of(&info.status);
    let sense = if state.sendable() { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), sense);
    let hot = (resp.hovered() || cursor) && state.sendable();
    let (fill, stroke) = if chosen {
        (CHOSEN(), TEAL())
    } else if hot {
        (SOFT(), BORDER())
    } else {
        (PANEL(), BORDER())
    };
    ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
    ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
    let tint = state.color();
    let mark = egui::pos2(rect.left() + 21.0, rect.center().y);
    if crate::agent_menu::paint_mark(ui.painter(), mark, 18.0, info.target.kind()) {
        crate::agent_menu::paint_status_on_mark(ui.painter(), mark, 18.0, tint);
    } else {
        ui.painter().circle_filled(mark, 4.0, tint);
    }
    let left = rect.left() + 40.0;
    let room = (rect.right() - left - 12.0).max(40.0);
    let pane = match &info.target {
        Target::Existing { pane, .. } => pane.as_str(),
        Target::New { .. } => "",
    };
    let name = row_name(Some(info.label.as_str()), pane);
    let shown = widgets::elide_font(ui, &name, room, semibold(14.0));
    ui.painter().text(egui::pos2(left, rect.top() + 9.0), Align2::LEFT_TOP, shown, semibold(14.0), INK());
    let mut line = crate::agent_menu::row_meta(info);
    if new {
        line.push_str(" · new");
    }
    if state == AgentState::Blocked
        && let Some(task) = task.map(str::trim).filter(|task| !task.is_empty())
    {
        line = format!("{line} · Waiting: {task}");
    }
    let line = widgets::elide_font(ui, &line, room, FontId::monospace(10.5));
    ui.painter().text(egui::pos2(left, rect.top() + 29.0), Align2::LEFT_TOP, line, FontId::monospace(10.5), tint);
    a11y::button(&resp, &format!("{name} · {}", crate::agent_menu::row_meta(info)));
    match state_note(state, task) {
        Some(note) => resp.on_hover_text(note),
        None => resp,
    }
}

/// What each agent says it is doing, from the workspace list.
pub(crate) fn agent_tasks(app: &FigApp, agents: &[TargetInfo]) -> Vec<Option<String>> {
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

/// Open the modal on its list (the header count, the composer's and the Sync pane's agent
/// buttons, ⌘⇧A's new agent).
pub fn open(app: &mut FigApp) {
    app.agents_modal.open_list();
}

/// What the creation thread's messages since the last frame changed.
#[derive(Debug, Default, PartialEq, Eq)]
struct Absorbed {
    /// The agent is idle in this pane.
    ready: Option<String>,
    /// A step failed.
    failed: bool,
}

/// Take the creation thread's messages into `creation`'s steps.
fn absorb(creation: &mut Creation) -> Absorbed {
    let mut out = Absorbed::default();
    let Some(rx) = creation.rx.as_ref() else { return out };
    loop {
        match rx.try_recv() {
            Ok(Progress::Done(step, pane)) => {
                if let Some(slot) = creation.steps.get_mut(step) {
                    *slot = Step::Done;
                }
                if let Some(next) = creation.steps.get_mut(step + 1)
                    && *next == Step::Todo
                {
                    *next = Step::Running;
                }
                if pane.is_some() {
                    creation.pane = pane;
                }
            }
            Ok(Progress::Ready(pane)) => {
                creation.steps = [Step::Done; STEPS];
                creation.pane = Some(pane.clone());
                creation.ready = true;
                creation.rx = None;
                out.ready = Some(pane);
                return out;
            }
            Ok(Progress::Failed { step, error, lines, pane }) => {
                if let Some(slot) = creation.steps.get_mut(step) {
                    *slot = Step::Failed;
                }
                for later in creation.steps.iter_mut().skip(step + 1) {
                    *later = Step::Todo;
                }
                if pane.is_some() {
                    creation.pane = pane;
                }
                creation.error = Some(error);
                creation.lines = lines;
                creation.rx = None;
                out.failed = true;
                return out;
            }
            Err(mpsc::TryRecvError::Empty) => return out,
            Err(mpsc::TryRecvError::Disconnected) => {
                creation.rx = None;
                if !creation.ready && creation.error.is_none() {
                    creation.error = Some("The node stopped answering while the agent started.".to_string());
                    if let Some(slot) = creation.steps.iter_mut().find(|step| **step == Step::Running) {
                        *slot = Step::Failed;
                    }
                    out.failed = true;
                }
                return out;
            }
        }
    }
}

/// Read what the creation thread said; the agent being made keeps going while the modal is
/// closed. Called every frame. A ready agent becomes the prompt target.
fn poll(app: &mut FigApp, ctx: &egui::Context) {
    let Some(creation) = app.agents_modal.creation.as_mut() else { return };
    let absorbed = absorb(creation);
    if absorbed.failed && app.agents_modal.open && app.agents_modal.view == View::Creating {
        app.agents_modal.view = View::Failed;
    }
    let Some(pane) = absorbed.ready else { return };
    let Some(creation) = app.agents_modal.creation.as_ref() else { return };
    let kind = creation.form.kind.clone();
    let model = models_for(&kind).get(creation.form.model).and_then(|(arg, label)| arg.map(|_| *label));
    if let Some(model) = model {
        crate::agent_menu::remember_model(&pane, model);
    }
    // The new agent is the prompt target; the prompt in the composer stays as it was.
    app.compose.choose(Target::Existing { pane, kind, status: "idle".to_string() });
    app.agents_modal.view = View::List;
    crate::compose::refresh_targets(app, ctx);
}

/// Start creating `form`'s agent on a thread: one node request per step, each reported as it
/// ends, so the modal shows where it is and where it stopped.
#[cfg(not(target_arch = "wasm32"))]
fn start_creation(app: &mut FigApp, ctx: &egui::Context, form: Form) {
    let Ok(node) = crate::workspaces::active_node(app) else {
        app.toast("Open a connected node workspace to create an agent.");
        return;
    };
    let model = models_for(&form.kind).get(form.model).and_then(|(arg, _)| *arg).map(str::to_string);
    let (tx, rx) = mpsc::channel();
    let now = ctx.input(|input| input.time);
    let mut steps = [Step::Todo; STEPS];
    steps[0] = Step::Running;
    app.agents_modal.creation =
        Some(Creation { form: form.clone(), steps, pane: None, error: None, lines: Vec::new(), started: now, ready: false, rx: Some(rx) });
    app.agents_modal.view = View::Creating;
    let context = ctx.clone();
    let spawned = std::thread::Builder::new().name("figcraft-agent-create".into()).spawn(move || {
        let base = json!({"session": node.session, "workspace": node.workspace});
        let ask = |method: &str, extra: serde_json::Value| -> Result<serde_json::Value, String> {
            let mut request = base.clone();
            request["method"] = json!(method);
            if let (Some(request), Some(extra)) = (request.as_object_mut(), extra.as_object()) {
                request.extend(extra.clone());
            }
            let reply = crate::node::rpc(&node.address, request, STEP_TIMEOUT)?;
            if reply.get("ok") == Some(&json!(false)) {
                return Err(reply["error"].as_str().unwrap_or("The node refused the request.").to_string());
            }
            Ok(reply)
        };
        let send = |progress: Progress| {
            let _ = tx.send(progress);
            context.request_repaint();
        };
        let fail = |step: usize, error: String, pane: Option<String>| {
            let lines = pane
                .as_deref()
                .and_then(|pane| ask("agent.screen", json!({"pane": pane})).ok())
                .and_then(|reply| serde_json::from_value::<Vec<String>>(reply["lines"].clone()).ok())
                .unwrap_or_default();
            send(Progress::Failed { step, error, lines, pane });
        };
        let pane = match ask("agent.tab", json!({"label": form.name})) {
            Ok(reply) => match reply["pane"].as_str() {
                Some(pane) => pane.to_string(),
                None => return fail(0, "Herdr opened no pane for the agent.".to_string(), None),
            },
            Err(error) => return fail(0, error, None),
        };
        send(Progress::Done(0, Some(pane.clone())));
        match ask("agent.start", json!({"pane": pane, "kind": form.kind, "label": form.name, "model": model})) {
            Ok(_) => {
                send(Progress::Done(1, None));
                send(Progress::Done(2, None));
            }
            Err(error) => return fail(1, error, Some(pane)),
        }
        match ask("agent.ready", json!({"pane": pane, "timeout_secs": 60})) {
            Ok(_) => send(Progress::Ready(pane)),
            Err(error) => fail(3, error, Some(pane)),
        }
    });
    if let Err(error) = spawned {
        app.agents_modal.creation = None;
        app.agents_modal.view = View::Form;
        app.toast(format!("Could not start the agent: {error}"));
    }
}

#[cfg(target_arch = "wasm32")]
fn start_creation(app: &mut FigApp, _ctx: &egui::Context, _form: Form) {
    app.toast("Creating an agent needs the desktop app.");
}

/// Ask the node for something about the agent's pane and forget the answer (focus it, close it).
#[cfg(not(target_arch = "wasm32"))]
fn fire(app: &FigApp, method: &'static str, pane: String) {
    let Ok(node) = crate::workspaces::active_node(app) else { return };
    let _ = std::thread::Builder::new().name("figcraft-agent-pane".into()).spawn(move || {
        let request = json!({"method": method, "session": node.session, "workspace": node.workspace, "pane": pane, "confirm": true});
        let _ = crate::node::rpc(&node.address, request, Duration::from_secs(15));
    });
}

#[cfg(target_arch = "wasm32")]
fn fire(_app: &FigApp, _method: &'static str, _pane: String) {}

/// What the human did in the modal this frame.
#[derive(Default)]
struct Out {
    close: bool,
    pick: Option<Target>,
    refresh: bool,
    new_kind: Option<String>,
    create: bool,
    cancel: bool,
    retry: bool,
    focus_pane: bool,
    back: bool,
    /// Back from a failure: the form, with its values.
    to_form: bool,
    show_creation: bool,
}

/// The header's workspace (`mindmap · main`) and where a new agent opens (`/path · main`).
#[cfg(not(target_arch = "wasm32"))]
fn where_it_opens(app: &FigApp) -> (String, String) {
    match crate::workspaces::node_identity(app) {
        Some(node) => {
            let folder = std::path::Path::new(&node.path).file_name().and_then(|name| name.to_str()).unwrap_or(&node.path).to_string();
            (format!("{folder} · {}", node.branch), format!("{} · {}", node.path, node.branch))
        }
        None => (app.active_workspace.clone().unwrap_or_default(), String::new()),
    }
}

#[cfg(target_arch = "wasm32")]
fn where_it_opens(app: &FigApp) -> (String, String) {
    (app.active_workspace.clone().unwrap_or_default(), String::new())
}

/// Draw the modal when open (and keep a creation going when it is not).
pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    // The composer's and the Sync pane's agent buttons ask for the modal without the app at hand.
    if std::mem::take(&mut app.compose.wants_agents) {
        open(app);
        crate::compose::refresh_targets(app, ctx);
    }
    poll(app, ctx);
    // The composer and the Sync pane treat the modal as their open menu (Enter and Esc are its).
    app.compose.menu_open = app.agents_modal.open;
    app.sync.menu_open = app.agents_modal.open;
    if !app.agents_modal.open {
        return;
    }
    let targets = app.compose.targets.clone();
    let selected = app.compose.target.clone();
    let error = app.compose.targets_error.clone();
    let (workspace, opens_in) = where_it_opens(app);
    let page_name = app
        .session
        .page()
        .ok()
        .and_then(|id| app.session.doc().ok().and_then(|doc| doc.node(id).map(|node| node.name.trim().to_string())))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "New agent".to_string());
    let agents: Vec<TargetInfo> = targets.iter().filter(|info| !info.new_agent).cloned().collect();
    let clis: Vec<String> = targets.iter().filter(|info| info.new_agent).map(|info| info.target.kind().to_string()).collect();
    let tasks = agent_tasks(app, &agents);
    let now = ctx.input(|input| input.time);
    let mut out = Out::default();

    let area =
        egui::Area::new(Id::new("agents_modal")).anchor(Align2::CENTER_CENTER, vec2(0.0, -24.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::NONE
                .fill(PANEL())
                .stroke(Stroke::new(1.0, BORDER()))
                .corner_radius(CornerRadius::same(10))
                .shadow(egui::epaint::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha(46) })
                .inner_margin(Margin::symmetric(20, 16))
                .show(ui, |ui| {
                    ui.set_width(MODAL_W);
                    let state = &mut app.agents_modal;
                    match state.view {
                        View::List => list_view(ui, state, &agents, &tasks, &clis, selected.as_ref(), error.as_deref(), &workspace, now, &mut out),
                        View::Form => form_view(ui, state, &clis, &opens_in, &mut out),
                        View::Creating | View::Failed => creation_view(ui, state, &opens_in, now, &mut out),
                    }
                });
        });

    let state = &mut app.agents_modal;
    if let Some(kind) = out.new_kind.take() {
        state.open_form(&kind, &page_name);
    }
    if out.back {
        state.view = View::List;
        state.focus = true;
    }
    if out.to_form {
        if let Some(creation) = state.creation.take() {
            state.form = creation.form;
        }
        state.view = View::Form;
        state.focus = true;
    }
    if out.show_creation {
        state.view = if state.creation.as_ref().is_some_and(|c| c.error.is_some()) { View::Failed } else { View::Creating };
    }
    if out.create {
        let form = state.form.clone();
        if !form.name.trim().is_empty() && !form.kind.is_empty() {
            start_creation(app, ctx, form);
        }
    }
    if out.retry {
        let previous = app.agents_modal.creation.take();
        if let Some(previous) = previous {
            if let Some(pane) = previous.pane {
                fire(app, "pane.close", pane);
            }
            start_creation(app, ctx, previous.form);
        }
    }
    if out.cancel {
        if let Some(creation) = app.agents_modal.creation.take()
            && let Some(pane) = creation.pane
            && !creation.ready
        {
            fire(app, "pane.close", pane);
        }
        app.agents_modal.view = View::Form;
    }
    if out.focus_pane
        && let Some(pane) = app.agents_modal.creation.as_ref().and_then(|c| c.pane.clone())
    {
        fire(app, "agent.focus", pane);
    }
    if let Some(target) = out.pick {
        app.compose.choose(target);
        out.close = true;
    }
    if out.refresh {
        crate::compose::refresh_targets(app, ctx);
    }
    let pressed = ctx.input(|input| input.pointer.any_pressed());
    let pointer = ctx.input(|input| input.pointer.interact_pos());
    let inside = pointer.is_some_and(|pos| area.response.rect.contains(pos));
    let just_opened = app.agents_modal.just_opened;
    app.agents_modal.just_opened = false;
    // A popup of the form (the model list) is outside the modal's rect but still the modal.
    let popup_open = egui::Popup::is_any_open(ctx);
    if (pressed && !inside && !just_opened && !popup_open) || out.close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.agents_modal.open = false;
        app.compose.menu_open = false;
        app.sync.menu_open = false;
    }
}

/// The header: `AGENTS  mindmap · main` and ×, or `← Agents` on the form.
fn header(ui: &mut egui::Ui, back: bool, workspace: &str, out: &mut Out) {
    ui.horizontal(|ui| {
        if back {
            if ui.add(egui::Button::new(RichText::new("← Agents").font(regular(11.0)).color(MUTED())).frame(false)).clicked() {
                out.back = true;
            }
        } else {
            ui.label(RichText::new("AGENTS").font(FontId::monospace(9.5)).color(ACCENT()));
            ui.label(RichText::new(workspace).font(FontId::monospace(9.0)).color(MUTED()));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new(RichText::new("×").size(16.0).color(MUTED())).frame(false)).clicked() {
                out.close = true;
            }
        });
    });
}

fn eyebrow(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).font(FontId::monospace(8.5)).color(MUTED()));
}

#[allow(clippy::too_many_arguments)]
fn list_view(
    ui: &mut egui::Ui,
    state: &mut State,
    agents: &[TargetInfo],
    tasks: &[Option<String>],
    clis: &[String],
    selected: Option<&Target>,
    error: Option<&str>,
    workspace: &str,
    now: f64,
    out: &mut Out,
) {
    header(ui, false, workspace, out);
    ui.add_space(6.0);
    let working = agents.iter().filter(|info| AgentState::of(&info.status) == AgentState::Working).count();
    let summary = if agents.is_empty() {
        "No agents in this workspace yet.".to_string()
    } else if working > 0 {
        format!("{} agent{} · {} running", agents.len(), plural(agents.len()), working)
    } else {
        format!("{} agent{} · all idle", agents.len(), plural(agents.len()))
    };
    ui.label(RichText::new(summary).font(regular(12.0)).color(INK()));
    ui.add_space(8.0);
    // The search, with the arrows and Enter taken before it sees them.
    let shown: Vec<usize> = {
        let kept = crate::agent_menu::filter_agents(agents, &state.query);
        kept.iter().filter_map(|info| agents.iter().position(|a| std::ptr::eq(a, *info))).collect()
    };
    let (down, up, enter) = ui.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
        )
    });
    if down {
        state.cursor = (state.cursor + 1).min(shown.len().saturating_sub(1));
    }
    if up {
        state.cursor = state.cursor.saturating_sub(1);
    }
    let before = state.query.clone();
    let search = egui::Frame::NONE
        .fill(SOFT())
        .stroke(Stroke::new(1.0, BORDER()))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut state.query)
                    .id(search_id())
                    .frame(egui::Frame::NONE)
                    .font(regular(11.5))
                    .hint_text(RichText::new("Find an agent…").font(regular(11.5)).color(MUTED()))
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
    state.cursor = state.cursor.min(shown.len().saturating_sub(1));
    ui.add_space(10.0);
    // The agent being created, if the modal was closed while it started (or failed).
    if let Some(creation) = state.creation.as_ref().filter(|c| !c.ready) {
        if pending_row(ui, creation, now) {
            out.show_creation = true;
        }
        ui.add_space(8.0);
    }
    let new_pane = state.creation.as_ref().filter(|c| c.ready).and_then(|c| c.pane.clone());
    egui::ScrollArea::vertical().id_salt("agents_modal_scroll").max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
        if let Some(error) = error {
            ui.label(RichText::new(error).font(regular(11.0)).color(DANGER()));
            ui.add_space(8.0);
        }
        if shown.is_empty() && !agents.is_empty() {
            ui.label(RichText::new("No agent matches the search.").font(regular(11.0)).color(MUTED()));
        }
        for (row, &index) in shown.iter().enumerate() {
            let (Some(info), task) = (agents.get(index), tasks.get(index).and_then(|t| t.as_deref())) else { continue };
            let chosen = selected.map(Target::line) == Some(info.target.line());
            let new = matches!(&info.target, Target::Existing { pane, .. } if Some(pane) == new_pane.as_ref());
            let resp = agent_row(ui, info, task, chosen, row == state.cursor, new);
            if (down || up) && row == state.cursor {
                resp.scroll_to_me(None);
            }
            if resp.clicked() && AgentState::of(&info.status).sendable() {
                out.pick = Some(info.target.clone());
            }
            ui.add_space(8.0);
        }
    });
    if enter
        && let Some(info) = shown.get(state.cursor).and_then(|index| agents.get(*index))
        && AgentState::of(&info.status).sendable()
    {
        out.pick = Some(info.target.clone());
    }
    ui.add_space(4.0);
    divider(ui);
    ui.add_space(8.0);
    eyebrow(ui, "START A NEW AGENT");
    ui.add_space(4.0);
    if clis.is_empty() {
        ui.label(RichText::new("No agent CLIs installed on the project host.").font(regular(11.0)).color(MUTED()));
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for kind in clis {
                if crate::agent_menu::cli_chip(ui, kind, false) {
                    out.new_kind = Some(kind.clone());
                }
            }
        });
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if ui.add(egui::Button::new(RichText::new("Refresh").font(regular(11.0)).color(MUTED())).frame(false)).clicked() {
            out.refresh = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let foot =
                if new_pane.is_some() { "closes on ↵ · the prompt stays in the composer" } else { "↑↓ move · ↵ choose · esc close" };
            ui.label(RichText::new(foot).font(FontId::monospace(8.5)).color(FAINT()));
        });
    });
}

/// The agent being created, as a list row: amber and dashed while it starts, red with Retry
/// when it could not. A click shows its steps.
fn pending_row(ui: &mut egui::Ui, creation: &Creation, now: f64) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), egui::Sense::click());
    let failed = creation.error.is_some();
    let (fill, ink) = if failed { (DANGER().gamma_multiply(0.08), DANGER()) } else { (AMBER().gamma_multiply(0.08), AMBER()) };
    ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
    ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0, ink), egui::StrokeKind::Inside);
    let mark = egui::pos2(rect.left() + 21.0, rect.center().y);
    if !crate::agent_menu::paint_mark(ui.painter(), mark, 18.0, &creation.form.kind) {
        ui.painter().circle_filled(mark, 4.0, ink);
    }
    let left = rect.left() + 40.0;
    ui.painter().text(egui::pos2(left, rect.top() + 9.0), Align2::LEFT_TOP, &creation.form.name, semibold(14.0), INK());
    let line = if failed {
        "couldn't start · open to see why".to_string()
    } else {
        let pane = creation.pane.as_deref().map(short_pane).map(|p| format!("{p} · ")).unwrap_or_default();
        format!("{pane}starting · {}s", (now - creation.started).max(0.0) as u64)
    };
    ui.painter().text(egui::pos2(left, rect.top() + 29.0), Align2::LEFT_TOP, line, FontId::monospace(10.5), ink);
    a11y::button(&resp, &format!("{} · {}", creation.form.name, if failed { "couldn't start" } else { "starting" }));
    resp.clicked()
}

fn divider(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().line_segment([rect.left_center(), rect.right_center()], Stroke::new(1.0, BORDER()));
}

/// The search field's id, for tests.
pub(crate) fn search_id() -> Id {
    Id::new("agents_modal_search")
}

/// The name field's id, for tests.
pub(crate) fn name_id() -> Id {
    Id::new("agents_modal_name")
}

/// A modal button: the accent action, a plain one, or a greyed one that takes no click.
fn modal_button(ui: &mut egui::Ui, label: &str, width: f32, primary: bool, enabled: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 30.0), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    a11y::button(&resp, label);
    let (fill, stroke, ink) = match (primary, enabled) {
        (_, false) => (SOFT(), BORDER(), FAINT()),
        (true, true) => (if resp.hovered() { ACCENT().gamma_multiply(0.9) } else { ACCENT() }, ACCENT(), Color32::WHITE),
        (false, true) => (if resp.hovered() { SOFT() } else { PANEL() }, BORDER(), INK()),
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, regular(11.5), ink);
    resp.clicked() && enabled
}

/// The new agent's form: the CLI, the model, the pane's name and where it opens.
fn form_view(ui: &mut egui::Ui, state: &mut State, clis: &[String], opens_in: &str, out: &mut Out) {
    header(ui, true, "", out);
    ui.add_space(6.0);
    ui.label(RichText::new("New agent").font(regular(20.0)).color(INK()));
    ui.add_space(12.0);
    eyebrow(ui, "CLI");
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for kind in clis {
            if crate::agent_menu::cli_chip(ui, kind, state.form.kind == *kind) && state.form.kind != *kind {
                state.form.kind = kind.clone();
                state.form.model = 0;
            }
        }
    });
    ui.add_space(12.0);
    eyebrow(ui, "MODEL");
    ui.add_space(4.0);
    let models = models_for(&state.form.kind);
    let current = models.get(state.form.model).map(|(_, label)| *label).unwrap_or("CLI default");
    egui::ComboBox::from_id_salt("agents_modal_model").width(MODAL_W - 8.0).selected_text(RichText::new(current).font(regular(12.0))).show_ui(
        ui,
        |ui| {
            for (index, (arg, label)) in models.iter().enumerate() {
                let text = if arg.is_none() { format!("{label} · the CLI's own setting") } else { (*label).to_string() };
                ui.selectable_value(&mut state.form.model, index, RichText::new(text).font(regular(12.0)));
            }
        },
    );
    ui.add_space(12.0);
    eyebrow(ui, "PANE NAME");
    ui.add_space(4.0);
    let name = ui.add(egui::TextEdit::singleline(&mut state.form.name).id(name_id()).font(regular(12.5)).desired_width(MODAL_W - 8.0));
    if state.focus {
        name.request_focus();
        state.focus = false;
    }
    ui.label(RichText::new("Herdr's tab and pane").font(regular(10.5)).color(MUTED()));
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        eyebrow(ui, "OPENS IN");
        // A long checkout keeps its end (the folder and branch) and never widens the modal.
        let shown = widgets::elide_start_font(ui, opens_in, (ui.available_width() - 8.0).max(60.0), FontId::monospace(10.0));
        ui.label(RichText::new(shown).font(FontId::monospace(10.0)).color(MUTED()));
    });
    ui.add_space(16.0);
    let ready = !state.form.name.trim().is_empty() && !state.form.kind.is_empty();
    let enter = ready && ui.input(|i| i.key_pressed(egui::Key::Enter));
    ui.horizontal(|ui| {
        if modal_button(ui, "Back", 90.0, false, true) {
            out.back = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if modal_button(ui, "Create agent  ↵", 150.0, true, ready) || enter {
                out.create = true;
            }
        });
    });
}

/// The agent being started, step by step; or why it could not start, with its pane's last lines.
fn creation_view(ui: &mut egui::Ui, state: &mut State, opens_in: &str, now: f64, out: &mut Out) {
    let Some(creation) = state.creation.as_ref() else {
        state.view = View::List;
        return;
    };
    let failed = creation.error.is_some();
    ui.horizontal(|ui| {
        ui.label(RichText::new("NEW AGENT").font(FontId::monospace(9.5)).color(ACCENT()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new(RichText::new("×").size(16.0).color(MUTED())).frame(false)).clicked() {
                out.close = true;
            }
        });
    });
    ui.add_space(8.0);
    let kind = creation.form.kind.clone();
    let title = if failed { format!("Couldn't start {}", creation.form.name) } else { format!("Starting {}", creation.form.name) };
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
        if failed {
            ui.painter().circle_filled(rect.center(), 11.0, DANGER().gamma_multiply(0.12));
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, "!", semibold(13.0), DANGER());
        } else if !crate::agent_menu::paint_mark(ui.painter(), rect.center(), 18.0, &kind) {
            ui.painter().circle_filled(rect.center(), 5.0, AMBER());
        }
        ui.label(RichText::new(title).font(regular(18.0)).color(if failed { DANGER() } else { INK() }));
    });
    let model = models_for(&kind).get(creation.form.model).map(|(_, label)| *label).unwrap_or("CLI default");
    let sub = match &creation.error {
        Some(error) => error.clone(),
        None => {
            // The kind and model stay whole; a long checkout loses its start instead.
            let lead = format!("{kind} · {model} · ");
            let room = (MODAL_W - ui.painter().layout_no_wrap(lead.clone(), regular(11.0), Color32::WHITE).size().x).max(60.0);
            format!("{lead}{}", widgets::elide_start_font(ui, opens_in, room, regular(11.0)))
        }
    };
    ui.add(egui::Label::new(RichText::new(sub).font(regular(11.0)).color(MUTED())).wrap());
    ui.add_space(12.0);
    let labels = ["Tab opened".to_string(), format!("{kind} started"), "Folder trusted".to_string(), "Ready: idle".to_string()];
    for (step, label) in creation.steps.iter().zip(labels) {
        step_row(ui, *step, &label, if *step == Step::Running { Some((now - creation.started).max(0.0) as u64) } else { None });
        ui.add_space(6.0);
    }
    if failed && !creation.lines.is_empty() {
        ui.add_space(4.0);
        eyebrow(ui, "THE PANE'S LAST LINES");
        ui.add_space(4.0);
        egui::Frame::NONE.fill(Color32::from_rgb(0x2B, 0x2F, 0x36)).corner_radius(CornerRadius::same(6)).inner_margin(Margin::same(10)).show(
            ui,
            |ui| {
                ui.set_width(ui.available_width());
                for line in &creation.lines {
                    ui.add(egui::Label::new(RichText::new(line).font(FontId::monospace(10.5)).color(Color32::from_rgb(0xE8, 0xE6, 0xE1))).wrap());
                }
            },
        );
    }
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        if failed {
            if modal_button(ui, "Back", 80.0, false, true) {
                out.to_form = true;
            }
            if creation.pane.is_some() && modal_button(ui, "Open the pane", 120.0, false, true) {
                out.focus_pane = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if modal_button(ui, "Retry", 110.0, true, true) {
                    out.retry = true;
                }
            });
        } else {
            if modal_button(ui, "Cancel", 90.0, false, true) {
                out.cancel = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                modal_button(ui, "Creating…", 150.0, true, false);
            });
        }
    });
}

/// One step: a check when done, an amber ring while it runs (with its seconds), a red cross when
/// it failed, a faint ring before it starts.
fn step_row(ui: &mut egui::Ui, step: Step, label: &str, seconds: Option<u64>) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::hover());
        let c = rect.center();
        let ink = match step {
            Step::Done => {
                let green = crate::agent_menu::status_color("idle");
                ui.painter().line_segment([c + vec2(-5.0, 0.0), c + vec2(-1.5, 3.5)], Stroke::new(1.6, green));
                ui.painter().line_segment([c + vec2(-1.5, 3.5), c + vec2(5.0, -4.0)], Stroke::new(1.6, green));
                INK()
            }
            Step::Running => {
                ui.painter().circle_stroke(c, 5.5, Stroke::new(1.6, AMBER()));
                AMBER()
            }
            Step::Failed => {
                ui.painter().circle_filled(c, 7.0, DANGER().gamma_multiply(0.12));
                ui.painter().text(c, Align2::CENTER_CENTER, "✕", regular(10.0), DANGER());
                DANGER()
            }
            Step::Todo => {
                ui.painter().circle_stroke(c, 4.5, Stroke::new(1.0, FAINT()));
                FAINT()
            }
        };
        ui.label(RichText::new(label).font(regular(12.0)).color(ink));
        if let Some(seconds) = seconds {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("{seconds}s")).font(FontId::monospace(10.5)).color(AMBER()));
            });
        }
    });
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
        assert_eq!(AgentState::Working.color(), crate::map_colors::get().working_ink);
        assert_eq!(AgentState::Idle.color(), TEAL());
        assert_eq!(AgentState::Blocked.color(), DANGER());
        assert_eq!(AgentState::Unknown.color(), GREY());
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
            ui.set_width(MODAL_W);
            for (index, info) in infos.iter().enumerate() {
                let top = ui.cursor().top();
                let resp = agent_row(ui, info, Some("waiting on review"), index == 1, false, false);
                let picked = resp.clicked() && AgentState::of(&info.status).sendable();
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
                label: format!("Pane {index}"),
                title: None,
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
    fn claude_offers_its_models_and_every_cli_starts_on_its_own_default() {
        let claude: Vec<&str> = models_for("claude").iter().map(|(_, label)| *label).collect();
        assert_eq!(claude, ["CLI default", "Opus", "Sonnet", "Haiku"]);
        assert_eq!(models_for("claude")[1].0, Some("opus"));
        assert_eq!(models_for("codex"), vec![(None, "CLI default")]);
    }

    fn creation(rx: Receiver<Progress>) -> Creation {
        let form = Form { kind: "claude".into(), model: 1, name: "Agent picker".into() };
        let mut steps = [Step::Todo; STEPS];
        steps[0] = Step::Running;
        Creation { form, steps, pane: None, error: None, lines: Vec::new(), started: 0.0, ready: false, rx: Some(rx) }
    }

    #[test]
    fn each_step_shows_as_it_ends_and_a_ready_agent_is_reported_once() {
        let (tx, rx) = mpsc::channel();
        let mut made = creation(rx);
        tx.send(Progress::Done(0, Some("w1:p8".into()))).unwrap();
        assert_eq!(absorb(&mut made), Absorbed::default());
        assert_eq!(made.steps, [Step::Done, Step::Running, Step::Todo, Step::Todo]);
        assert_eq!(made.pane.as_deref(), Some("w1:p8"));
        tx.send(Progress::Done(1, None)).unwrap();
        tx.send(Progress::Done(2, None)).unwrap();
        absorb(&mut made);
        assert_eq!(made.steps, [Step::Done, Step::Done, Step::Done, Step::Running]);
        tx.send(Progress::Ready("w1:p8".into())).unwrap();
        assert_eq!(absorb(&mut made).ready.as_deref(), Some("w1:p8"));
        assert!(made.ready && made.rx.is_none());
        assert_eq!(absorb(&mut made), Absorbed::default(), "nothing more to read");
    }

    #[test]
    fn a_failed_step_keeps_why_and_the_panes_last_lines() {
        let (tx, rx) = mpsc::channel();
        let mut made = creation(rx);
        tx.send(Progress::Done(0, Some("w1:p8".into()))).unwrap();
        tx.send(Progress::Failed { step: 1, error: "herdr agent.start failed".into(), lines: vec!["Error: not logged in.".into()], pane: None })
            .unwrap();
        assert!(absorb(&mut made).failed);
        assert_eq!(made.steps, [Step::Done, Step::Failed, Step::Todo, Step::Todo]);
        assert_eq!(made.error.as_deref(), Some("herdr agent.start failed"));
        assert_eq!(made.lines, vec!["Error: not logged in.".to_string()]);
        assert_eq!(made.pane.as_deref(), Some("w1:p8"), "the pane stays known, to open it or retry");

        // The node going away mid-start is a failure too, at the step that was running.
        let (tx, rx) = mpsc::channel();
        let mut made = creation(rx);
        tx.send(Progress::Done(0, Some("w1:p9".into()))).unwrap();
        drop(tx);
        assert!(absorb(&mut made).failed);
        assert_eq!(made.steps[1], Step::Failed);
    }

    /// Draw the modal's current face alone, returning its texts.
    fn face(state: &mut State, now: f64) -> (Vec<egui::epaint::Shape>, Out) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, false, 0);
        let agents = vec![TargetInfo {
            target: Target::Existing { pane: "w1:p1".into(), kind: "codex".into(), status: "idle".into() },
            label: "Backend".into(),
            title: None,
            status: "idle".into(),
            focused: false,
            new_agent: false,
        }];
        let clis = vec!["claude".to_string(), "codex".to_string()];
        let mut out = Out::default();
        let mut shapes = Vec::new();
        for frame in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(480.0, 800.0))),
                time: Some(frame as f64),
                ..Default::default()
            };
            out = Out::default();
            let mut output = ctx.run_ui(input, |ui| {
                ui.set_width(MODAL_W);
                match state.view {
                    View::List => list_view(ui, state, &agents, &[None], &clis, None, None, "mindmap · main", now, &mut out),
                    View::Form => form_view(ui, state, &clis, "/root/code/mindmap · main", &mut out),
                    View::Creating | View::Failed => creation_view(ui, state, "/root/code/mindmap · main", now, &mut out),
                }
            });
            output.textures_delta.clear();
            shapes = crate::live_tests::flat(&output.shapes);
        }
        (shapes, out)
    }

    #[test]
    fn the_modal_lists_agents_then_makes_one_step_by_step() {
        use crate::live_tests::has_text;
        let mut state = State::default();
        state.open_list();
        let (shapes, _) = face(&mut state, 0.0);
        for text in ["AGENTS", "1 agent · all idle", "Backend", "p1 · idle", "START A NEW AGENT", "claude", "↑↓ move · ↵ choose · esc close"]
        {
            assert!(has_text(&shapes, text), "list: missing {text:?}");
        }
        state.open_form("claude", "Agent picker");
        let (shapes, _) = face(&mut state, 0.0);
        for text in ["← Agents", "New agent", "CLI", "MODEL", "CLI default", "PANE NAME", "OPENS IN", "/root/code/mindmap · main", "Create agent  ↵"]
        {
            assert!(has_text(&shapes, text), "form: missing {text:?}");
        }
        assert_eq!(state.form.name, "Agent picker", "the pane is named after the page to start with");

        let (_tx, rx) = mpsc::channel();
        state.creation = Some(creation(rx));
        state.view = View::Creating;
        let (shapes, _) = face(&mut state, 8.0);
        for text in
            ["NEW AGENT", "Starting Agent picker", "Tab opened", "claude started", "Folder trusted", "Ready: idle", "8s", "Cancel", "Creating…"]
        {
            assert!(has_text(&shapes, text), "creating: missing {text:?}");
        }

        if let Some(made) = state.creation.as_mut() {
            made.steps = [Step::Done, Step::Failed, Step::Todo, Step::Todo];
            made.error = Some("herdr agent.start failed".into());
            made.lines = vec!["Error: not logged in.".into()];
            made.pane = Some("w1:p8".into());
        }
        state.view = View::Failed;
        let (shapes, _) = face(&mut state, 9.0);
        for text in [
            "Couldn't start Agent picker",
            "herdr agent.start failed",
            "THE PANE'S LAST LINES",
            "Error: not logged in.",
            "Back",
            "Open the pane",
            "Retry",
        ] {
            assert!(has_text(&shapes, text), "failed: missing {text:?}");
        }

        // Closed mid-start, the list keeps the agent as a row of its own.
        if let Some(made) = state.creation.as_mut() {
            made.error = None;
            made.steps = [Step::Done, Step::Running, Step::Todo, Step::Todo];
        }
        state.view = View::List;
        let (shapes, _) = face(&mut state, 8.0);
        assert!(has_text(&shapes, "p8 · starting · 8s"), "the starting agent shows in the list");
    }

    #[test]
    fn a_blocked_agent_says_why_it_waits() {
        assert_eq!(state_note(AgentState::Blocked, Some("approve the migration")).as_deref(), Some("Waiting: approve the migration"));
        assert!(state_note(AgentState::Blocked, None).is_some());
        assert_eq!(state_note(AgentState::Unknown, None).as_deref(), Some("Cannot send until status and connection are known."));
        assert_eq!(state_note(AgentState::Idle, Some("x")), None);
    }
}
