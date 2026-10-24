//! The New workspace sheet: pick a machine and a folder, then the workspace opens on Init.
//!
//! Design: `Widget New workspace` on the Mindmap page (states Local, Remote, Connect, Creating,
//! Init, Init busy, Init busy no info). One 520 × 390 card: the machine field opens into the
//! machine list in place, and "Connect a machine…" grows an SSH target instead of sending the
//! person elsewhere.
//!
//! The node has no create-workspace RPC today (its methods are listing, Open and Init), so Create
//! never claims success: it looks for a workspace at the chosen folder in the listed
//! [`Workspace`](crate::workspaces::Workspace) rows and, when none appears, says so. The Init
//! busy card is this module's too: Init never sends a second prompt to an agent Herdr reports as
//! working ([`guard_init`]). The step machine is pure state and unit-tested without a window.

use egui::{CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::FigApp;
use crate::machine_modal::Machine;
use crate::machine_modal::shell;
use crate::theme::{Tokens, regular, semibold};
use crate::workspaces::{MapState, Target, Workspace};

/// The card's size, as in the design.
const WIDTH: f32 = 520.0;
const HEIGHT: f32 = 390.0;
/// How long the Creating state waits for the new workspace to be listed before it says so.
const CREATING_NOTE_AFTER: f64 = 6.0;
/// The most folder suggestions shown under the field.
#[cfg(not(target_arch = "wasm32"))]
const SUGGESTIONS: usize = 6;

/// The states of the design.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Step {
    /// The machine, folder, branch and name (the design's Local and Remote states).
    #[default]
    Local,
    /// The machine list is open in place of the form (the design's Connect state when
    /// "Connect a machine…" is chosen).
    Connect,
    /// The brief Creating state.
    Creating,
    /// A matching workspace exists: the sheet hands it to its Init page.
    Init,
}

/// An Init that was not sent because its agent is busy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Busy {
    /// The workspace Init was for.
    pub workspace: String,
    /// The busy agent's pane and kind.
    pub pane: String,
    pub kind: String,
    /// What Herdr says the agent is doing, when it says.
    pub task: Option<String>,
}

/// The modal's state.
#[derive(Default)]
pub struct State {
    /// The modal is open.
    pub open: bool,
    /// The step the design shows.
    pub step: Step,
    /// The machine the folder lives on.
    pub machine: Machine,
    /// The folder on that machine.
    pub folder: String,
    /// The branch to check out; empty uses the repository's own checkout.
    pub branch: String,
    /// The workspace name, derived from the folder until typed by hand.
    pub name: String,
    name_touched: bool,
    /// Why Create refused the fields.
    pub error: Option<String>,
    /// When the Creating state began (egui's seconds).
    started: f64,
    /// The honest note is showing: creating a workspace on the node is not available.
    pub unavailable: bool,
    /// The workspace the Init state addresses.
    pub workspace: Option<String>,
    /// The list was asked for while Creating.
    refreshed: bool,
    /// "Connect a machine…" is chosen in the machine list: the SSH target field shows.
    pub connecting: bool,
    /// The SSH target typed under "Connect a machine…".
    pub target: String,
    /// The machine being connected for this sheet (`ssh:<target>`), until setup finishes.
    pub awaiting: Option<String>,
    /// The Init busy card, when Init found its agent working.
    pub busy: Option<Busy>,
    /// "Wait for it": Init goes to this agent once Herdr reports it idle.
    pub waiting: Option<Busy>,
    /// Where the folder completions were drawn last frame.
    suggest_rect: Option<Rect>,
    /// The node is creating the Herdr workspace now.
    pub creating: bool,
    /// The node created it; Check again only re-lists, it never creates a second one.
    pub created: bool,
    /// Why the node could not create it.
    pub create_error: Option<String>,
    #[cfg(not(target_arch = "wasm32"))]
    create_rx: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
}

impl State {
    /// Open on the form for `machines`, preferring `current`.
    pub fn begin(&mut self, machines: &[Machine], current: Option<Machine>) {
        self.open = true;
        self.step = Step::Local;
        self.machine = current.or_else(|| machines.first().cloned()).unwrap_or_default();
        self.folder.clear();
        self.branch.clear();
        self.name.clear();
        self.name_touched = false;
        self.error = None;
        self.started = 0.0;
        self.unavailable = false;
        self.workspace = None;
        self.refreshed = false;
        self.connecting = false;
        self.target.clear();
        self.awaiting = None;
        self.creating = false;
        self.created = false;
        self.create_error = None;
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.create_rx = None;
        }
    }

    /// Close the modal.
    pub fn close(&mut self) {
        self.open = false;
        self.step = Step::Local;
        self.workspace = None;
        self.connecting = false;
        self.awaiting = None;
    }

    /// Open the machine list in place of the form.
    pub fn pick_machine(&mut self) {
        self.step = Step::Connect;
        self.connecting = false;
        self.error = None;
    }

    /// Choose "Connect a machine…": the SSH target field shows.
    pub fn show_connect(&mut self) {
        self.step = Step::Connect;
        self.connecting = true;
        self.error = None;
    }

    /// The person picked the machine at `machine`.
    pub fn choose_machine(&mut self, machine: Machine) {
        self.machine = machine;
        self.step = Step::Local;
        self.connecting = false;
        self.error = None;
    }

    /// The folder was edited: derive the name from it until the person names the workspace.
    pub fn derive_name(&mut self) {
        if !self.name_touched {
            self.name = folder_name(&self.folder);
        }
    }

    /// The person typed a name: stop deriving it from the folder.
    pub fn name_edited(&mut self) {
        self.name_touched = true;
    }

    /// Back one step: Creating or the machine list to the form.
    pub fn back(&mut self) {
        self.step = Step::Local;
        self.connecting = false;
        self.error = None;
        self.unavailable = false;
    }

    /// Validate "Connect a machine…": the SSH target the Add machine setup will run for.
    pub fn connect_target(&mut self) -> Option<String> {
        let target = self.target.trim().to_string();
        #[cfg(not(target_arch = "wasm32"))]
        if let Err(error) = crate::node_hosts::validate_target(&target) {
            self.error = Some(error);
            return None;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = target;
            self.error = Some("Machines are only available in the desktop app.".to_string());
            return None;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.error = None;
            self.awaiting = Some(format!("ssh:{target}"));
            Some(target)
        }
    }

    /// Validate the form; true when Create may go on.
    fn validate(&mut self) -> bool {
        if self.machine.address.trim().is_empty() && self.machine.ssh.is_none() {
            self.error = Some("Choose the machine the folder lives on.".to_string());
            return false;
        }
        let folder = self.folder.trim().to_string();
        if folder.is_empty() {
            self.error = Some("Pick the folder on the machine.".to_string());
            return false;
        }
        if folder.len() > 1024 || folder.chars().any(char::is_control) {
            self.error = Some("The folder path is too long or contains a line break.".to_string());
            return false;
        }
        if !(folder.starts_with('/') || folder.starts_with('~')) {
            self.error = Some("Pick the folder's full path (starting with / or ~).".to_string());
            return false;
        }
        if self.name.trim().is_empty() {
            self.error = Some("Name the workspace.".to_string());
            return false;
        }
        if self.name.trim().len() > 64 || self.name.chars().any(char::is_control) {
            self.error = Some("Use a name of at most 64 characters.".to_string());
            return false;
        }
        let branch = self.branch.trim();
        if branch.len() > 255 || branch.chars().any(|c| c.is_whitespace() || c.is_control()) {
            self.error = Some("A branch name has no spaces.".to_string());
            return false;
        }
        self.error = None;
        true
    }

    /// Create: if a workspace at this folder is already listed, go to Init; otherwise the Creating
    /// state, where the node creates the Herdr workspace.
    pub fn create(&mut self, now: f64, rows: &[Workspace]) -> Option<String> {
        if !self.validate() {
            return None;
        }
        if let Some(id) = matching(rows, &self.folder, &self.branch) {
            self.enter_init(id.clone());
            return Some(id);
        }
        self.step = Step::Creating;
        self.started = now;
        self.unavailable = false;
        self.refreshed = false;
        self.workspace = None;
        self.created = false;
        self.create_error = None;
        None
    }

    /// While Creating: a matching workspace appearing in the rows wins; after a moment the honest
    /// note says the node cannot create one. Returns the workspace when Init took over.
    pub fn tick(&mut self, now: f64, rows: &[Workspace]) -> Option<String> {
        if self.step != Step::Creating {
            return None;
        }
        if let Some(id) = matching(rows, &self.folder, &self.branch) {
            self.enter_init(id.clone());
            return Some(id);
        }
        if !self.unavailable && !self.creating && now >= self.started && now - self.started >= CREATING_NOTE_AFTER {
            self.unavailable = true;
        }
        None
    }

    /// Check again after the note: restart the wait. A failed create is tried again; a created
    /// workspace is only listed again.
    pub fn recheck(&mut self, now: f64) {
        self.started = now;
        self.unavailable = false;
        self.refreshed = self.created;
        self.create_error = None;
    }

    /// The node answered the create: wait for the new workspace to be listed, or say why not.
    pub fn created(&mut self, now: f64, result: Result<(), String>) {
        self.creating = false;
        match result {
            Ok(()) => {
                self.created = true;
                self.started = now;
            }
            Err(error) => {
                self.create_error = Some(error);
                self.unavailable = true;
            }
        }
    }

    /// Whether the list should be asked for again while Creating.
    pub fn wants_refresh(&self) -> bool {
        self.step == Step::Creating && !self.refreshed
    }

    /// The list was asked for.
    pub fn asked_refresh(&mut self) {
        self.refreshed = true;
    }

    fn enter_init(&mut self, id: String) {
        self.step = Step::Init;
        self.workspace = Some(id);
        self.error = None;
        self.unavailable = false;
    }
}

/// The workspace's short name from its folder: `/Users/you/ledger/` is `ledger`.
pub fn folder_name(folder: &str) -> String {
    let folder = folder.trim().trim_end_matches('/');
    let last = folder.rsplit('/').find(|part| !part.trim().is_empty()).map(str::trim).unwrap_or("");
    let name: String = last.chars().take(64).collect();
    if name.is_empty() { "workspace".to_string() } else { name }
}

/// The workspace already listed at `folder` (its path or repository trunk) on `branch`, if any.
pub fn matching(rows: &[Workspace], folder: &str, branch: &str) -> Option<String> {
    let folder = folder.trim().trim_end_matches('/');
    let branch = branch.trim();
    let branch_matches = |w: &Workspace| branch.is_empty() || w.branch.as_deref().is_some_and(|b| b.trim() == branch);
    rows.iter()
        .find(|w| {
            let path = w.path.as_deref().map(|path| path.trim().trim_end_matches('/'));
            let repo = w.repo.as_deref().map(|repo| repo.trim().trim_end_matches('/'));
            (path == Some(folder) || repo == Some(folder)) && branch_matches(w)
        })
        .map(|w| w.id.clone())
        .filter(|id| !id.is_empty())
}

/// Whether Herdr's status says the agent is busy with something.
fn is_working(status: &str) -> bool {
    matches!(status.trim().to_ascii_lowercase().as_str(), "working" | "busy" | "running" | "thinking")
}

/// The busy card for Init on `workspace` with `target`, when that agent is working right now.
pub fn busy_for(rows: &[Workspace], workspace: &str, target: Option<&Target>) -> Option<Busy> {
    let Some(Target::Existing { pane, .. }) = target else { return None };
    let w = rows.iter().find(|w| w.id == workspace)?;
    let agent = w.agents_detail.iter().find(|agent| &agent.pane == pane)?;
    is_working(&agent.status).then(|| Busy {
        workspace: workspace.to_string(),
        pane: agent.pane.clone(),
        kind: if agent.kind.trim().is_empty() { "agent".to_string() } else { agent.kind.trim().to_string() },
        task: agent.title.as_deref().map(task_of).filter(|task| !task.is_empty()),
    })
}

/// What an agent says it is doing, from its pane title: spinners dropped, and for a
/// `repo | Ready | task | branch` title just the task.
fn task_of(title: &str) -> String {
    let title = title.trim_start_matches(|c: char| !c.is_alphanumeric()).trim();
    let parts: Vec<&str> = title.split(" | ").map(str::trim).collect();
    let task = if parts.len() >= 3 { parts.get(2).copied().unwrap_or(title) } else { parts.first().copied().unwrap_or(title) };
    task.to_lowercase()
}

/// Called before Init is sent from the Workspaces screen: when the chosen agent is working, the
/// busy card opens and nothing is sent. True when it took over.
pub fn guard_init(app: &mut FigApp, workspace: &str) -> bool {
    match busy_for(&app.ws.rows, workspace, app.ws.target.as_ref()) {
        Some(busy) => {
            app.new_workspace.busy = Some(busy);
            true
        }
        None => false,
    }
}

/// Open the New workspace modal from the Workspaces screen.
pub fn open(app: &mut FigApp) {
    let machines = machines(app);
    let current = current_machine(app, &machines);
    app.new_workspace.begin(&machines, current);
}

/// The machines the list offers: the one in use, then the saved SSH machines.
fn machines(app: &FigApp) -> Vec<Machine> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let current = crate::node_hosts::resolve(&crate::workspaces::service_addr());
        let mut machines = vec![machine_of(&current)];
        // This computer once (a local node on another port is still this computer), then SSH.
        let local = current.ssh.is_none();
        for host in crate::node_hosts::known(&app.ws.hosts) {
            if (host.ssh.is_some() || !local) && !machines.iter().any(|machine| machine.address == host.address) {
                machines.push(machine_of(&host));
            }
        }
        machines.truncate(64);
        machines
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = app;
        vec![Machine { name: "This computer".into(), ..Machine::default() }]
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn machine_of(host: &crate::node_hosts::Host) -> Machine {
    Machine { name: host.name.clone(), address: host.address.clone(), ssh: host.ssh.clone() }
}

fn current_machine(app: &FigApp, machines: &[Machine]) -> Option<Machine> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(host) = &app.ws.hosts.selected {
        return Some(machine_of(host));
    }
    #[cfg(target_arch = "wasm32")]
    let _ = app;
    machines.first().cloned()
}

/// The machine row's right-hand note: `local`, `ssh · connected` or `ssh`.
fn machine_note(machine: &Machine, in_use: &str) -> &'static str {
    match &machine.ssh {
        None => "local",
        Some(_) if machine.address == in_use => "ssh · connected",
        Some(_) => "ssh",
    }
}

/// What a button in the modal asks for after the frame.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Action {
    Close,
    /// Run the Add machine setup for this SSH target, above this sheet.
    Connect(String),
    /// Re-list the workspaces (Check again).
    Refresh,
}

/// What the busy card asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BusyAction {
    Close,
    Wait,
    StartNew,
}

/// Draw the modal (and the Init busy card) when open.
pub fn show(app: &mut FigApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    waiting(app, &ctx);
    busy_card(app, ui);
    if !app.new_workspace.open {
        return;
    }
    // A machine connected from this sheet becomes its machine once its setup closes.
    if let Some(address) = app.new_workspace.awaiting.clone()
        && !app.machine_modal.open
    {
        app.new_workspace.awaiting = None;
        if let Some(machine) = machines(app).into_iter().find(|machine| machine.address == address) {
            app.new_workspace.choose_machine(machine);
        }
    }
    #[allow(unused_mut)]
    let mut rows = app.ws.rows.clone();
    // A workspace created on another open machine is listed under that machine.
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(list) = app.ws.lists.get(&app.new_workspace.machine.address) {
        rows.extend(list.rows.iter().cloned());
    }
    let machines = machines(app);
    let in_use = {
        #[cfg(not(target_arch = "wasm32"))]
        {
            crate::workspaces::service_addr()
        }
        #[cfg(target_arch = "wasm32")]
        String::new()
    };
    let now = ctx.input(|input| input.time);
    if let Some(id) = app.new_workspace.tick(now, &rows) {
        hand_off(app, &ctx, &id);
        return;
    }
    let tokens = Tokens::get(&ctx);
    let skin = shell::Skin::of(&tokens);
    let (should_close, action) = {
        let state = &mut app.new_workspace;
        let response = egui::Modal::new(Id::new("new_workspace_modal"))
            .backdrop_color(shell::scrim(&tokens))
            .frame(shell::surface(&skin))
            .show(&ctx, |ui| body(state, ui, &skin, &machines, &in_use, &rows, now));
        (response.should_close(), response.inner)
    };
    // The Add machine card draws above this one while it connects; its clicks are not ours.
    if should_close && !app.machine_modal.open {
        app.new_workspace.close();
        return;
    }
    if let Some(id) = app.new_workspace.workspace.clone().filter(|_| app.new_workspace.step == Step::Init) {
        hand_off(app, &ctx, &id);
        return;
    }
    if app.new_workspace.wants_refresh() && app.ws.busy.is_none() {
        app.new_workspace.asked_refresh();
        #[cfg(not(target_arch = "wasm32"))]
        start_create(app, &ctx);
        #[cfg(target_arch = "wasm32")]
        app.ws.refresh(&ctx);
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(result) = app.new_workspace.create_rx.as_ref().and_then(|rx| match rx.try_recv() {
        Ok(result) => Some(result),
        Err(std::sync::mpsc::TryRecvError::Empty) => None,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err("The create request ended without an answer.".to_string())),
    }) {
        app.new_workspace.create_rx = None;
        let ok = result.is_ok();
        app.new_workspace.created(now, result);
        if ok {
            let address = app.new_workspace.machine.address.clone();
            if address == crate::workspaces::service_addr() {
                app.ws.refresh(&ctx);
            } else {
                app.ws.list_host(&ctx, &address);
            }
        }
    }
    match action {
        Some(Action::Close) => app.new_workspace.close(),
        Some(Action::Connect(target)) => crate::machine_modal::connect(app, &target),
        Some(Action::Refresh) => {
            app.ws.refresh(&ctx);
            app.new_workspace.recheck(now);
        }
        None => {}
    }
}

/// Ask the sheet's machine to create the Herdr workspace, on a thread of its own.
#[cfg(not(target_arch = "wasm32"))]
fn start_create(app: &mut FigApp, ctx: &egui::Context) {
    let state = &mut app.new_workspace;
    if state.creating || state.created {
        return;
    }
    let address = state.machine.address.clone();
    let request = serde_json::json!({"method": "workspace.create", "path": state.folder.trim(), "label": state.name.trim()});
    let (tx, rx) = std::sync::mpsc::channel();
    state.create_rx = Some(rx);
    state.creating = true;
    let context = ctx.clone();
    let spawned = std::thread::Builder::new().name("figcraft-workspace-create".into()).spawn(move || {
        let result = crate::node::rpc(&address, request, std::time::Duration::from_secs(40)).map(|_| ());
        let _ = tx.send(result);
        context.request_repaint();
    });
    if let Err(e) = spawned {
        state.create_rx = None;
        state.creating = false;
        state.create_error = Some(format!("Could not start the create request: {e}"));
        state.unavailable = true;
    }
}

/// The workspace exists: it opens its map, or the Init page where the agent is chosen.
fn hand_off(app: &mut FigApp, ctx: &egui::Context, id: &str) {
    let map = app.ws.rows.iter().find(|w| w.id == id).map(Workspace::map_state).unwrap_or(MapState::None);
    app.new_workspace.close();
    app.ws.select(id);
    if map == MapState::Map {
        app.ws.act(ctx, id, false);
    } else {
        app.ui.home = true;
    }
}

/// "Wait for it": once Herdr reports the agent idle, Init goes to it.
fn waiting(app: &mut FigApp, ctx: &egui::Context) {
    let Some(wait) = app.new_workspace.waiting.clone() else { return };
    let agent = app.ws.rows.iter().find(|w| w.id == wait.workspace).and_then(|w| w.agents_detail.iter().find(|a| a.pane == wait.pane)).cloned();
    match agent {
        // The agent left the workspace: nothing to wait for.
        None if app.ws.busy.is_none() && !app.ws.rows.is_empty() => app.new_workspace.waiting = None,
        Some(agent) if !is_working(&agent.status) && app.ws.busy.is_none() => {
            app.new_workspace.waiting = None;
            app.ws.select(&wait.workspace);
            app.ws.target = Some(Target::Existing { pane: agent.pane, kind: agent.kind, status: agent.status });
            app.ws.act(ctx, &wait.workspace, true);
        }
        _ => {}
    }
}

fn body(state: &mut State, ui: &mut Ui, s: &shell::Skin, machines: &[Machine], in_use: &str, rows: &[Workspace], now: f64) -> Option<Action> {
    let card = shell::card(ui, WIDTH, HEIGHT);
    match state.step {
        Step::Local | Step::Init => form_step(state, ui, s, card, machines, in_use, rows, now),
        Step::Connect => machine_step(state, ui, s, card, machines, in_use),
        Step::Creating => creating_step(state, ui, s, card),
    }
}

/// The footer: a quiet button at the left, the dark one at the right, both 30 pt at `y`.
fn footer(ui: &mut Ui, s: &shell::Skin, card: Rect, y: f32, left: Option<&str>, right: Option<(&str, f32, bool)>) -> (bool, bool) {
    let left =
        left.is_some_and(|label| shell::quiet_button(ui, s, Id::new(("new_workspace_left", label)), shell::at(card, 24.0, y, 90.0, 30.0), label));
    let right = right.is_some_and(|(label, width, enabled)| {
        let width = shell::primary_width(ui, label, width);
        shell::primary_button(ui, s, Id::new(("new_workspace_right", label)), shell::at(card, WIDTH - 24.0 - width, y, width, 30.0), label, enabled)
    });
    (left, right)
}

fn heading(ui: &Ui, s: &shell::Skin, card: Rect, title: &str, body: &str) {
    shell::eyebrow(ui, card, 22.0, "NEW WORKSPACE", s.accent);
    shell::title(ui, s, card, 40.0, title, 22.0);
    shell::body(ui, s, card, 74.0, WIDTH - 48.0, body);
}

/// The design's Local and Remote states: the same form, over this computer or an SSH machine.
#[allow(clippy::too_many_arguments)]
fn form_step(
    state: &mut State,
    ui: &mut Ui,
    s: &shell::Skin,
    card: Rect,
    machines: &[Machine],
    in_use: &str,
    rows: &[Workspace],
    now: f64,
) -> Option<Action> {
    let _ = machines;
    heading(ui, s, card, "Where should it live?", "A workspace is a folder on a machine. Pick the machine, then the folder.");

    // MACHINE: a select that opens the machine list in place.
    shell::label(ui, s, card, 112.0, "MACHINE");
    let field = shell::at(card, 24.0, 126.0, WIDTH - 48.0, 36.0);
    let response = ui.interact(field, Id::new("new_workspace_machine"), Sense::click());
    crate::a11y::button(&response, "Machine");
    ui.painter().rect_filled(field, CornerRadius::same(6), if response.hovered() { s.hover } else { s.panel });
    ui.painter().rect_stroke(field, CornerRadius::same(6), Stroke::new(1.0, s.border), StrokeKind::Inside);
    let name = if state.machine.name.trim().is_empty() { state.machine.address.clone() } else { state.machine.name.clone() };
    shell::text(ui, field.min + vec2(12.0, 10.0), &name, regular(11.0), s.text);
    let note = machine_note(&state.machine, in_use).split(" · ").next().unwrap_or("local");
    shell::mono(ui, field.min + vec2(396.0, 12.0), note, 8.5, s.faint);
    chevron(ui, pos2(field.max.x - 21.0, field.min.y + 16.0), s.faint);
    if response.clicked() {
        state.pick_machine();
        return None;
    }

    // FOLDER: an existing directory, completed from this computer's filesystem.
    shell::label(ui, s, card, 174.0, "FOLDER");
    let local = state.machine.ssh.is_none();
    let folder_rect = shell::at(card, 24.0, 188.0, WIDTH - 48.0, 36.0);
    // Only an existing directory can be created; a prefix that still completes is not an error yet.
    let absent = local && !state.folder.trim().is_empty() && !folder_exists(&state.folder);
    let missing = absent && suggestions(&state.folder).is_empty();
    let folder = shell::field_response(
        ui,
        s,
        Id::new("new_workspace_folder"),
        folder_rect,
        &mut state.folder,
        "/path/to/the/repository",
        shell::FieldFont::Mono(9.5),
        missing,
    );
    if folder.changed() {
        state.derive_name();
        state.error = None;
    }

    shell::label(ui, s, card, 236.0, "BRANCH");
    shell::mono(ui, card.min + vec2(276.0, 236.0), "NAME", 8.5, s.muted);
    let branch = shell::at(card, 24.0, 250.0, 220.0, 36.0);
    if shell::field(ui, s, Id::new("new_workspace_branch"), branch, &mut state.branch, "main", shell::FieldFont::Mono(9.5)) {
        state.error = None;
    }
    let name_rect = shell::at(card, 276.0, 250.0, 220.0, 36.0);
    if shell::field(ui, s, Id::new("new_workspace_name"), name_rect, &mut state.name, "derived from the folder", shell::FieldFont::Inter(11.0)) {
        state.name_edited();
        state.error = None;
    }

    let note =
        if missing { Some(("No such folder on this computer.".to_string(), s.accent)) } else { state.error.clone().map(|error| (error, s.accent)) };
    match note {
        Some((text, color)) => shell::mono(ui, card.min + vec2(24.0, 300.0), &text, 8.5, color),
        None if local => shell::mono(ui, card.min + vec2(24.0, 300.0), "The name comes from the folder; you can change it.", 8.5, s.faint),
        None => {
            shell::mono(ui, card.min + vec2(24.0, 300.0), &format!("The folder is on {name}; the node lists its Herdr workspaces."), 8.5, s.faint)
        }
    }

    let ready = !state.folder.trim().is_empty() && !absent;
    let (cancel, create) = footer(ui, s, card, 332.0, Some("Cancel"), Some(("Create workspace", 140.0, ready)));

    // The completions sit over the fields below the folder, so they draw (and take clicks) last.
    // They stay up while the pointer is over them, so a press that takes the field's focus still
    // lands on the row.
    let over = state.suggest_rect.zip(ui.input(|i| i.pointer.hover_pos())).is_some_and(|(rect, pos)| rect.contains(pos));
    state.suggest_rect = None;
    if local && (folder.has_focus() || over) {
        let options = suggestions(&state.folder);
        let (pick, rect) = suggestion_list(ui, s, shell::at(card, 24.0, 226.0, WIDTH - 48.0, 0.0), &options);
        state.suggest_rect = rect;
        if let Some(pick) = pick {
            state.folder = pick;
            state.derive_name();
            state.error = None;
            folder.request_focus();
        } else if folder.has_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Tab))
            && let Some(first) = options.first()
        {
            state.folder = first.clone();
            state.derive_name();
            folder.request_focus();
        }
    }
    if cancel {
        return Some(Action::Close);
    }
    if create {
        state.create(now, rows);
    }
    None
}

/// The Connect state: the machine list in place of the form, with "Connect a machine…".
fn machine_step(state: &mut State, ui: &mut Ui, s: &shell::Skin, card: Rect, machines: &[Machine], in_use: &str) -> Option<Action> {
    heading(ui, s, card, "Where should it live?", "Pick a saved machine, or connect a new one without leaving this sheet.");
    shell::label(ui, s, card, 112.0, "MACHINE");
    // At most four saved machines show (two while the SSH target is open, as designed); the rest
    // scroll.
    let shown = machines.len().min(if state.connecting { 2 } else { 4 });
    let list = shell::at(card, 24.0, 126.0, WIDTH - 48.0, shown as f32 * 32.0);
    let mut chosen = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
        ui.set_clip_rect(list.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical().id_salt("new_workspace_machines").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for machine in machines {
                let name = if machine.name.trim().is_empty() { machine.address.as_str() } else { machine.name.as_str() };
                let selected = !state.connecting && machine.address == state.machine.address;
                if option_row(ui, s, name, machine_note(machine, in_use), selected, false) {
                    chosen = Some(machine.clone());
                }
            }
        });
    });
    if let Some(machine) = chosen {
        state.choose_machine(machine);
        return None;
    }
    let connect_row = shell::at(card, 24.0, list.max.y - card.min.y, WIDTH - 48.0, 30.0);
    let connect = ui
        .scope_builder(egui::UiBuilder::new().max_rect(connect_row), |ui| option_row(ui, s, "Connect a machine…", "new host", state.connecting, true))
        .inner;
    if connect {
        state.show_connect();
    }
    let bottom = connect_row.max.y - card.min.y;
    if state.connecting {
        let label_y = bottom + 12.0;
        shell::label(ui, s, card, label_y, "SSH TARGET");
        let field = shell::at(card, 24.0, label_y + 14.0, WIDTH - 48.0, 36.0);
        let response = shell::field_response(
            ui,
            s,
            Id::new("new_workspace_target"),
            field,
            &mut state.target,
            "user@host or a Host from ~/.ssh/config",
            shell::FieldFont::Mono(9.5),
            state.error.is_some(),
        );
        if response.changed() {
            state.error = None;
        }
        match &state.error {
            Some(error) => shell::mono(ui, card.min + vec2(24.0, 300.0), error, 8.5, s.accent),
            None => shell::mono(ui, card.min + vec2(24.0, 300.0), "OpenSSH asks for anything it needs; no password is stored.", 8.5, s.faint),
        }
        let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let enabled = !state.target.trim().is_empty();
        let (cancel, go) = footer(ui, s, card, 332.0, Some("Cancel"), Some(("Connect", 100.0, enabled)));
        if cancel {
            state.back();
            return None;
        }
        if (go || (enter && enabled))
            && let Some(target) = state.connect_target()
        {
            return Some(Action::Connect(target));
        }
        return None;
    }
    let (cancel, _) = footer(ui, s, card, 332.0, Some("Cancel"), None);
    if cancel {
        state.back();
    }
    None
}

fn creating_step(state: &mut State, ui: &mut Ui, s: &shell::Skin, card: Rect) -> Option<Action> {
    let machine = if state.machine.name.trim().is_empty() { state.machine.address.clone() } else { state.machine.name.clone() };
    let mut line = vec![machine.clone(), state.folder.trim().to_string()];
    if !state.branch.trim().is_empty() {
        line.push(state.branch.trim().to_string());
    }
    heading(ui, s, card, &format!("Creating {}", state.name.trim()), &line.join("   ·   "));
    let steps = [("Create the Herdr workspace".to_string()), "Attach the map session".to_string(), format!("Add the row under {machine}")];
    for (index, step) in steps.iter().enumerate() {
        let y = 126.0 + 26.0 * index as f32;
        let failed = state.unavailable && !state.created;
        let (mark, word, color) = match (index, failed, state.created) {
            (0, true, _) => (shell::Mark::Cross, "failed", s.accent),
            (0, false, true) => (shell::Mark::Check, "done", s.teal),
            (0, false, false) => (shell::Mark::Dot, "running", s.teal),
            (1, false, true) if !state.unavailable => (shell::Mark::Dot, "running", s.teal),
            (_, true, _) => (shell::Mark::Dash, "not started", s.faint),
            _ => (shell::Mark::Small, "waiting", s.faint),
        };
        shell::mark(ui, card, y, mark, color);
        shell::row_text(ui, s, card, 44.0, y, step);
        shell::mono(ui, card.min + vec2(324.0, y + 1.0), word, 8.5, color);
    }
    let note = shell::at(card, 24.0, 206.0, WIDTH - 48.0, 44.0);
    if state.unavailable {
        let text = match &state.create_error {
            Some(error) => format!("Herdr did not create it: {error}"),
            None => "Herdr created the workspace, but the node has not listed it yet. Check again in a moment.".to_string(),
        };
        shell::note_box(ui, s, note, &text, true);
        let (back, again) = footer(ui, s, card, 332.0, Some("Back"), Some(("Check again", 110.0, true)));
        if back {
            state.back();
        }
        return again.then_some(Action::Refresh);
    }
    shell::note_box(ui, s, note, "The folder stays a plain directory; the map is created on first open.", false);
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    let (cancel, _) = footer(ui, s, card, 332.0, Some("Cancel"), None);
    if cancel {
        state.back();
    }
    None
}

/// The Init busy card: the agent turned busy before the prompt, and nothing is sent.
fn busy_card(app: &mut FigApp, ui: &mut Ui) {
    let Some(busy) = app.new_workspace.busy.clone() else { return };
    let ctx = ui.ctx().clone();
    let tokens = Tokens::get(&ctx);
    let s = shell::Skin::of(&tokens);
    let response = egui::Modal::new(Id::new("init_busy_modal")).backdrop_color(shell::scrim(&tokens)).frame(shell::surface(&s)).show(&ctx, |ui| {
        let card = shell::card(ui, WIDTH, HEIGHT);
        let kind = busy.kind.as_str();
        shell::eyebrow(ui, card, 22.0, "INIT · AGENT BUSY", s.accent);
        shell::title(ui, &s, card, 40.0, &format!("{kind} is busy"), 22.0);
        let (body, sub, note) = match &busy.task {
            Some(task) => (
                format!("{kind} is working on {task}. Mindmap won't send it a second prompt."),
                format!("working · {task}"),
                "Herdr reported this; nothing is queued until you choose.",
            ),
            None => (
                format!("{kind} is working, but Herdr doesn't say on what. Mindmap doesn't guess."),
                "working · task unknown".to_string(),
                "The step says only that it is busy; ask Herdr or pick another agent.",
            ),
        };
        shell::body(ui, &s, card, 74.0, WIDTH - 48.0, &body);
        let info = shell::at(card, 24.0, 142.0, WIDTH - 48.0, 54.0);
        ui.painter().rect_filled(info, CornerRadius::same(6), s.soft);
        ui.painter().circle_filled(info.min + vec2(15.0, 17.0), 3.0, s.accent);
        shell::text(ui, info.min + vec2(24.0, 10.0), kind, semibold(11.0), s.text);
        let sub = crate::widgets::elide_font(ui, &sub, info.width() - 36.0, egui::FontId::monospace(8.5));
        shell::mono(ui, info.min + vec2(24.0, 28.0), &sub, 8.5, s.faint);
        shell::mono(ui, card.min + vec2(24.0, 212.0), note, 8.5, s.faint);
        let pick = shell::quiet_button(ui, &s, Id::new("init_busy_pick"), shell::at(card, 24.0, 300.0, 120.0, 30.0), "Pick another");
        let wait = shell::quiet_button(ui, &s, Id::new("init_busy_wait"), shell::at(card, 156.0, 300.0, 110.0, 30.0), "Wait for it");
        let start = shell::primary_button(ui, &s, Id::new("init_busy_start"), shell::at(card, 278.0, 300.0, 172.0, 30.0), "Start a new agent", true);
        if pick {
            Some(BusyAction::Close)
        } else if wait {
            Some(BusyAction::Wait)
        } else if start {
            Some(BusyAction::StartNew)
        } else {
            None
        }
    });
    let action = if response.should_close() { Some(BusyAction::Close) } else { response.inner };
    let Some(action) = action else { return };
    app.new_workspace.busy = None;
    match action {
        // The agent menu beside Init is where another agent is picked.
        BusyAction::Close => {}
        BusyAction::Wait => app.new_workspace.waiting = Some(busy),
        BusyAction::StartNew => {
            app.ws.select(&busy.workspace);
            app.ws.target = Some(Target::New { kind: busy.kind.clone() });
            app.ws.act(&ctx, &busy.workspace, true);
        }
    }
}

/// One row of the machine list: name at the left, its note in the column at 300 pt.
fn option_row(ui: &mut Ui, s: &shell::Skin, name: &str, note: &str, selected: bool, strong: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    crate::a11y::button(&response, name);
    let hot = response.hovered() || response.has_focus();
    if selected || hot {
        ui.painter().rect_filled(rect, CornerRadius::same(6), if selected { s.soft } else { s.hover });
    }
    let font = if strong { semibold(10.5) } else { regular(10.5) };
    shell::text(ui, rect.min + vec2(10.0, 8.0), name, font, s.text);
    ui.painter().text(pos2(rect.max.x - 12.0, rect.min.y + 10.0), egui::Align2::RIGHT_TOP, note, egui::FontId::monospace(8.5), s.faint);
    response.clicked()
}

/// The select's small down arrow, drawn.
fn chevron(ui: &Ui, c: egui::Pos2, color: egui::Color32) {
    ui.painter().add(egui::Shape::convex_polygon(
        vec![pos2(c.x - 3.0, c.y - 1.5), pos2(c.x + 3.0, c.y - 1.5), pos2(c.x, c.y + 2.0)],
        color,
        Stroke::NONE,
    ));
}

/// The folder completions under the field, above everything in the sheet. Returns the one
/// clicked and where the list is.
fn suggestion_list(ui: &mut Ui, s: &shell::Skin, anchor: Rect, options: &[String]) -> (Option<String>, Option<Rect>) {
    if options.is_empty() {
        return (None, None);
    }
    let mut picked = None;
    let area = egui::Area::new(Id::new("new_workspace_suggest")).order(egui::Order::Tooltip).fixed_pos(anchor.min).show(ui.ctx(), |ui| {
        egui::Frame::NONE
            .fill(s.panel)
            .stroke(Stroke::new(1.0, s.border))
            .corner_radius(CornerRadius::same(6))
            .shadow(egui::epaint::Shadow { offset: [0, 6], blur: 16, spread: 0, color: shell::with_alpha(egui::Color32::BLACK, 24) })
            .inner_margin(egui::Margin::same(4))
            .show(ui, |ui| {
                ui.set_width(anchor.width() - 8.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                for option in options {
                    let (row, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                    if response.hovered() {
                        ui.painter().rect_filled(row, CornerRadius::same(4), s.soft);
                    }
                    let shown = crate::widgets::elide_start_font(ui, option, row.width() - 16.0, egui::FontId::monospace(9.5));
                    ui.painter().text(row.min + vec2(8.0, 6.0), egui::Align2::LEFT_TOP, shown, egui::FontId::monospace(9.5), s.text);
                    if response.clicked() {
                        picked = Some(option.clone());
                    }
                }
            });
    });
    (picked, Some(area.response.rect))
}

/// `~/x` on this computer.
#[cfg(not(target_arch = "wasm32"))]
fn expand_home(path: &str) -> std::path::PathBuf {
    let path = path.trim();
    match path.strip_prefix('~') {
        Some(rest) => {
            std::env::var_os("HOME").map(std::path::PathBuf::from).map(|home| home.join(rest.trim_start_matches('/'))).unwrap_or_else(|| path.into())
        }
        None => path.into(),
    }
}

/// The folder exists on this computer.
fn folder_exists(folder: &str) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        expand_home(folder).is_dir()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = folder;
        true
    }
}

/// Directories on this computer that complete `typed`: the entries of its parent that start with
/// its last part (hidden ones only when that part starts with a dot). An exact directory lists its
/// children.
fn suggestions(typed: &str) -> Vec<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let typed = typed.trim();
        if !(typed.starts_with('/') || typed.starts_with('~')) {
            return Vec::new();
        }
        let (parent, prefix) = match typed.rfind('/') {
            Some(at) => (typed.get(..=at).unwrap_or(typed), typed.get(at + 1..).unwrap_or("")),
            None => return Vec::new(),
        };
        let Ok(entries) = std::fs::read_dir(expand_home(parent)) else { return Vec::new() };
        let mut found: Vec<String> = entries
            .filter_map(Result::ok)
            .take(4096)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.starts_with(prefix) && (prefix.starts_with('.') || !name.starts_with('.')))
            .filter(|name| name != prefix)
            .map(|name| format!("{parent}{name}"))
            .collect();
        found.sort();
        found.truncate(SUGGESTIONS);
        found
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = typed;
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspaces::AgentDetail;

    fn machine() -> Machine {
        Machine { name: "befree".into(), address: "ssh:befree".into(), ssh: Some("befree".into()) }
    }

    fn form() -> State {
        let mut state = State::default();
        state.begin(&[machine()], Some(machine()));
        state.folder = "/Users/you/ledger".into();
        state.derive_name();
        state
    }

    fn row(id: &str, path: &str, branch: Option<&str>) -> Workspace {
        Workspace { id: id.into(), label: id.into(), path: Some(path.into()), branch: branch.map(str::to_owned), ..Workspace::default() }
    }

    #[test]
    fn opening_starts_on_the_form_with_the_folder_name() {
        let state = form();
        assert!(state.open);
        assert_eq!(state.step, Step::Local);
        assert_eq!(state.machine.name, "befree");
        assert_eq!(state.name, "ledger");
        assert_eq!(folder_name("/Users/you/ledger/"), "ledger");
        assert_eq!(folder_name("~/code/mindmap"), "mindmap");
        assert_eq!(folder_name("/"), "workspace");
    }

    #[test]
    fn the_machine_list_opens_in_place_and_closes_on_a_choice() {
        let mut state = form();
        state.pick_machine();
        assert_eq!(state.step, Step::Connect);
        assert!(!state.connecting);
        state.choose_machine(Machine { name: "work".into(), address: "ssh:work".into(), ssh: Some("work".into()) });
        assert_eq!(state.step, Step::Local);
        assert_eq!(state.machine.address, "ssh:work");
        state.show_connect();
        assert_eq!(state.step, Step::Connect);
        assert!(state.connecting);
        state.back();
        assert_eq!(state.step, Step::Local);
        assert!(!state.connecting);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn connect_needs_a_valid_ssh_target_and_waits_for_that_machine() {
        let mut state = form();
        state.show_connect();
        state.target = "-oProxyCommand=x".into();
        assert!(state.connect_target().is_none());
        assert!(state.error.is_some());
        state.target = "alice@work.example".into();
        assert_eq!(state.connect_target().as_deref(), Some("alice@work.example"));
        assert_eq!(state.awaiting.as_deref(), Some("ssh:alice@work.example"));
        assert!(state.error.is_none());
    }

    #[test]
    fn machine_notes_say_where_it_is() {
        let local = Machine { name: "This computer".into(), address: "127.0.0.1:47822".into(), ssh: None };
        assert_eq!(machine_note(&local, "127.0.0.1:47822"), "local");
        assert_eq!(machine_note(&machine(), "ssh:befree"), "ssh · connected");
        assert_eq!(machine_note(&machine(), "127.0.0.1:47822"), "ssh");
    }

    #[test]
    fn create_refuses_a_relative_folder_and_an_empty_name() {
        let mut state = form();
        state.folder = "repo".into();
        assert!(state.create(0.0, &[]).is_none());
        assert!(state.error.is_some());
        assert_eq!(state.step, Step::Local);

        let mut state = form();
        state.name.clear();
        state.name_edited();
        assert!(state.create(0.0, &[]).is_none());
        assert!(state.error.is_some());

        let mut state = form();
        state.branch = "two words".into();
        assert!(state.create(0.0, &[]).is_none());
        assert!(state.error.is_some());
    }

    #[test]
    fn create_opens_a_listed_workspace_instead_of_creating_another() {
        let mut state = form();
        state.branch = "main".into();
        let rows = [row("w1", "/Users/you/ledger/", Some("main")), row("w2", "/Users/you/other", None)];
        assert_eq!(state.create(0.0, &rows).as_deref(), Some("w1"));
        assert_eq!(state.step, Step::Init);
        assert_eq!(state.workspace.as_deref(), Some("w1"));
    }

    #[test]
    fn create_without_a_matching_workspace_shows_creating_then_the_honest_note() {
        let mut state = form();
        let rows = [row("w2", "/Users/you/other", None)];
        assert!(state.create(0.0, &rows).is_none());
        assert_eq!(state.step, Step::Creating);
        assert!(!state.unavailable);
        assert!(state.tick(0.5, &rows).is_none());
        assert!(!state.unavailable);
        assert!(state.tick(CREATING_NOTE_AFTER + 0.1, &rows).is_none());
        assert!(state.unavailable);
        assert_eq!(state.step, Step::Creating);

        state.recheck(10.0);
        assert!(!state.unavailable);
        assert!(state.tick(10.5, &rows).is_none());
        assert!(!state.unavailable);
    }

    #[test]
    fn a_created_workspace_is_only_listed_again_and_a_failed_one_is_retried() {
        let mut state = form();
        assert!(state.create(0.0, &[]).is_none());
        assert!(state.wants_refresh());
        state.asked_refresh();
        state.creating = true;
        // While the node works, the wait never times out.
        assert!(state.tick(CREATING_NOTE_AFTER + 5.0, &[]).is_none());
        assert!(!state.unavailable);
        state.created(20.0, Err("not a folder".into()));
        assert!(state.unavailable);
        assert_eq!(state.create_error.as_deref(), Some("not a folder"));
        state.recheck(21.0);
        assert!(state.wants_refresh(), "a failed create is tried again");
        state.asked_refresh();
        state.created(22.0, Ok(()));
        assert!(state.created && !state.unavailable);
        assert!(state.tick(22.0 + CREATING_NOTE_AFTER + 0.1, &[]).is_none());
        assert!(state.unavailable);
        state.recheck(40.0);
        assert!(!state.wants_refresh(), "a created workspace is never created twice");
        let rows = [row("w1", "/Users/you/ledger", None)];
        assert_eq!(state.tick(40.5, &rows).as_deref(), Some("w1"));
    }

    #[test]
    fn a_workspace_appearing_while_creating_wins_over_the_note() {
        let mut state = form();
        assert!(state.create(0.0, &[]).is_none());
        assert_eq!(state.step, Step::Creating);
        let rows = [row("w1", "/Users/you/ledger", None)];
        assert_eq!(state.tick(0.1, &rows).as_deref(), Some("w1"));
        assert_eq!(state.step, Step::Init);
        assert!(!state.unavailable);
    }

    #[test]
    fn matching_reads_paths_repositories_and_branches() {
        let rows = [
            row("base", "/repo", Some("main")),
            Workspace { id: "trunk".into(), label: "trunk".into(), repo: Some("/tmp/trunk".into()), ..Workspace::default() },
        ];
        assert_eq!(matching(&rows, "/repo/", "").as_deref(), Some("base"));
        assert_eq!(matching(&rows, "/repo", "main").as_deref(), Some("base"));
        assert_eq!(matching(&rows, "/tmp/trunk", "").as_deref(), Some("trunk"));
        assert!(matching(&rows, "/repo", "feature").is_none());
        assert!(matching(&rows, "/elsewhere", "").is_none());
    }

    #[test]
    fn back_from_creating_returns_to_the_form_and_closing_clears_the_workspace() {
        let mut state = form();
        assert!(state.create(0.0, &[]).is_none());
        state.back();
        assert_eq!(state.step, Step::Local);
        assert!(!state.unavailable);
        state.workspace = Some("w1".into());
        state.close();
        assert!(!state.open);
        assert!(state.workspace.is_none());
        assert_eq!(state.step, Step::Local);
    }

    fn with_agent(status: &str, title: Option<&str>) -> Vec<Workspace> {
        vec![Workspace {
            id: "w1".into(),
            agents_detail: vec![AgentDetail {
                pane: "w1:p2".into(),
                kind: "codex".into(),
                status: status.into(),
                focused: false,
                title: title.map(str::to_owned),
            }],
            ..Workspace::default()
        }]
    }

    #[test]
    fn init_to_a_working_agent_opens_the_busy_card_with_its_task() {
        let target = Target::Existing { pane: "w1:p2".into(), kind: "codex".into(), status: "idle".into() };
        let rows = with_agent("working", Some("mindmap | Working | Sync backend code | main"));
        let busy = busy_for(&rows, "w1", Some(&target)).expect("busy");
        assert_eq!(busy.kind, "codex");
        assert_eq!(busy.task.as_deref(), Some("sync backend code"));

        let rows = with_agent("working", None);
        assert_eq!(busy_for(&rows, "w1", Some(&target)).and_then(|b| b.task), None);

        assert!(busy_for(&with_agent("idle", None), "w1", Some(&target)).is_none());
        assert!(busy_for(&rows, "w1", Some(&Target::New { kind: "claude".into() })).is_none());
        assert!(busy_for(&rows, "w1", None).is_none());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn folder_suggestions_list_matching_directories_only() {
        let root = std::env::temp_dir().join(format!("mindmap-suggest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("alpha")).expect("dir");
        std::fs::create_dir_all(root.join("alps")).expect("dir");
        std::fs::create_dir_all(root.join(".hidden")).expect("dir");
        std::fs::write(root.join("alfile"), b"x").expect("file");
        let base = root.to_string_lossy().to_string();
        let found = suggestions(&format!("{base}/al"));
        assert_eq!(found, vec![format!("{base}/alpha"), format!("{base}/alps")]);
        assert_eq!(suggestions(&format!("{base}/.h")), vec![format!("{base}/.hidden")]);
        assert!(folder_exists(&format!("{base}/alpha")));
        assert!(!folder_exists(&format!("{base}/nope")));
        assert!(suggestions("relative/path").is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
