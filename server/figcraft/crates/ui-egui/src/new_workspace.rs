//! The New workspace modal: pick a machine and a folder, then the workspace opens on Init.
//!
//! Design: `Widget New workspace` on the Mindmap page (states Local, Remote, Connect, Creating,
//! Init).
//!
//! The node has no create-workspace RPC today (its methods are listing, Open and Init), so the
//! Create action never claims success: it looks for a workspace at the chosen folder in the
//! listed [`Workspace`](crate::workspaces::Workspace) rows and, when none appears, says so and
//! points at Herdr. The step machine is pure state and unit-tested without a window.

use egui::{Align2, Color32, CornerRadius, FontId, Id, Margin, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::FigApp;
use crate::a11y;
use crate::machine_modal::Machine;
use crate::machine_modal::shell;
use crate::theme::{Tokens, regular, semibold};
use crate::workspaces::{MapState, Workspace};

/// The modal's surface width; the form is narrower than the Add machine wizard.
const WIDTH: f32 = 780.0;
/// The space between the surface and the window edge before the modal shrinks.
const EDGE: f32 = 48.0;
/// How long the Creating state waits for the node's list before saying creation is unavailable.
const CREATING_NOTE_AFTER: f64 = 1.5;

/// The states of the design.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Step {
    /// The folder, branch and name on a machine.
    #[default]
    Local,
    /// Pick a saved SSH machine.
    Remote,
    /// No machine yet: add one first (this opens the Add machine modal).
    Connect,
    /// The brief Creating state.
    Creating,
    /// A matching workspace exists: open it or its Init page.
    Init,
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
}

impl State {
    /// Open on the Local step for `machines`, preferring `current`.
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
    }

    /// Close the modal.
    pub fn close(&mut self) {
        self.open = false;
        self.step = Step::Local;
        self.workspace = None;
    }

    /// The Local step tab.
    pub fn show_local(&mut self) {
        self.step = Step::Local;
        self.error = None;
    }

    /// The Remote step tab.
    pub fn show_remote(&mut self) {
        self.step = Step::Remote;
        self.error = None;
    }

    /// The Connect step: no SSH machine is saved yet, so add one first.
    pub fn show_connect(&mut self) {
        self.step = Step::Connect;
        self.error = None;
    }

    /// The person picked the machine at `machine`.
    pub fn choose_machine(&mut self, machine: Machine) {
        self.machine = machine;
        self.step = Step::Local;
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

    /// Back one step: Creating/Remote/Connect to Local.
    pub fn back(&mut self) {
        self.step = match self.step {
            Step::Creating | Step::Remote | Step::Connect => Step::Local,
            Step::Local | Step::Init => Step::Local,
        };
        self.error = None;
        self.unavailable = false;
    }

    /// Validate the Local form; true when Create may go on.
    fn validate(&mut self) -> bool {
        if self.machine.address.trim().is_empty() && self.machine.ssh.is_none() {
            self.error = Some("Choose the machine the folder lives on.".to_string());
            return false;
        }
        let folder = self.folder.trim().to_string();
        if folder.is_empty() {
            self.error = Some("Enter the folder on the machine (e.g. /Users/you/repo).".to_string());
            return false;
        }
        if folder.len() > 1024 || folder.chars().any(char::is_control) {
            self.error = Some("The folder path is too long or contains a line break.".to_string());
            return false;
        }
        if !(folder.starts_with('/') || folder.starts_with('~')) {
            self.error = Some("Enter the folder's full path (starting with / or ~), not a relative one.".to_string());
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

    /// Create: if a workspace at this folder is already listed, go to Init; otherwise the honest
    /// Creating state (there is no create RPC to call).
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
        if !self.unavailable && now >= self.started && now - self.started >= CREATING_NOTE_AFTER {
            self.unavailable = true;
        }
        None
    }

    /// Check again after the honest note: restart the wait (the caller re-lists).
    pub fn recheck(&mut self, now: f64) {
        self.started = now;
        self.unavailable = false;
        self.refreshed = false;
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

/// A workspace's shown name: its label, or its id when the label is empty.
fn workspace_name(workspace: &Workspace) -> &str {
    if workspace.label.trim().is_empty() { &workspace.id } else { &workspace.label }
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

/// Open the New workspace modal from the Workspaces screen.
pub fn open(app: &mut FigApp) {
    let machines = machines(app);
    let current = current_machine(app, &machines);
    app.new_workspace.begin(&machines, current);
}

/// The machines the machine chips offer: the one in use, then the saved SSH machines.
fn machines(app: &FigApp) -> Vec<Machine> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let current = crate::node_hosts::resolve(&crate::workspaces::service_addr());
        let mut machines = vec![machine_of(&current)];
        for host in crate::node_hosts::known(&app.ws.hosts) {
            if !machines.iter().any(|machine| machine.address == host.address) {
                machines.push(machine_of(&host));
            }
        }
        machines.truncate(64);
        machines
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = app;
        vec![Machine::default()]
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

/// What a button in the modal asks for after the frame.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Action {
    Close,
    /// Open the Add machine modal (from Remote or Connect); this modal stays under it.
    AddMachine,
    /// Re-list the workspaces (Check again).
    Refresh,
    /// Open a workspace's live document, or its Init page where the agent is chosen.
    Open {
        id: String,
        init_page: bool,
    },
}

/// Draw the modal when open.
pub fn show(app: &mut FigApp, ui: &mut Ui) {
    if !app.new_workspace.open {
        return;
    }
    let ctx = ui.ctx().clone();
    let rows = app.ws.rows.clone();
    let machines = machines(app);
    let now = ctx.input(|input| input.time);
    app.new_workspace.tick(now, &rows);
    let tokens = Tokens::get(&ctx);
    let skin = shell::Skin::of(&tokens);
    let width = (WIDTH - 56.0).min((ctx.content_rect().width() - EDGE - 56.0).max(320.0));
    let (should_close, action) = {
        let state = &mut app.new_workspace;
        let response =
            egui::Modal::new(Id::new("new_workspace_modal")).backdrop_color(shell::scrim(&tokens)).frame(shell::surface(&skin)).show(&ctx, |ui| {
                ui.set_width(width);
                body(state, ui, &skin, &machines, &rows, now)
            });
        (response.should_close(), response.inner)
    };
    if should_close {
        app.new_workspace.close();
        return;
    }
    if app.new_workspace.wants_refresh() && app.ws.busy.is_none() {
        app.ws.refresh(&ctx);
        app.new_workspace.asked_refresh();
    }
    match action {
        Some(Action::Close) => app.new_workspace.close(),
        Some(Action::AddMachine) => crate::machine_modal::open(app),
        Some(Action::Refresh) => {
            app.ws.refresh(&ctx);
            app.new_workspace.recheck(now);
        }
        Some(Action::Open { id, init_page }) => {
            if init_page {
                app.ws.select(&id);
                app.ui.home = true;
            } else {
                app.ws.act(&ctx, &id, false);
            }
            app.new_workspace.close();
        }
        None => {}
    }
}

fn body(state: &mut State, ui: &mut Ui, s: &shell::Skin, machines: &[Machine], rows: &[Workspace], now: f64) -> Option<Action> {
    let mut action = None;
    let note = match state.step {
        Step::Local => "LOCAL",
        Step::Remote => "REMOTE",
        Step::Connect => "CONNECT",
        Step::Creating => "CREATING",
        Step::Init => "INIT",
    };
    if shell::header(ui, s, "NEW WORKSPACE", note) {
        action = Some(Action::Close);
    }
    ui.add_space(6.0);
    if matches!(state.step, Step::Local | Step::Remote) {
        tabs(state, ui, s, machines);
    }
    ui.add_space(10.0);
    action = action.or(match state.step {
        Step::Local => local_step(state, ui, s, machines, rows, now),
        Step::Remote => remote_step(state, ui, s, machines),
        Step::Connect => connect_step(state, ui, s, machines),
        Step::Creating => creating_step(state, ui, s),
        Step::Init => init_step(state, ui, s, rows),
    });
    action
}

fn tabs(state: &mut State, ui: &mut Ui, s: &shell::Skin, machines: &[Machine]) {
    ui.horizontal(|ui| {
        if tab(ui, s, "Local", state.step == Step::Local) {
            state.show_local();
        }
        ui.add_space(2.0);
        if tab(ui, s, "Remote", state.step == Step::Remote) {
            if machines.iter().any(|machine| machine.ssh.is_some()) {
                state.show_remote();
            } else {
                state.show_connect();
            }
        }
    });
}

fn tab(ui: &mut Ui, s: &shell::Skin, label: &str, active: bool) -> bool {
    let font = semibold(11.5);
    let width = shell::text_width(ui, label, font.clone()) + 26.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
    a11y::button(&response, label);
    let hot = response.hovered() || response.has_focus();
    let fill = if active {
        shell::with_alpha(s.accent, if s.panel == Color32::WHITE { 12 } else { 30 })
    } else if hot {
        s.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    let border = if active { s.accent } else { s.border };
    ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, border), StrokeKind::Inside);
    let ink = if active {
        s.accent
    } else if hot {
        s.text
    } else {
        s.muted
    };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font, ink);
    response.clicked()
}

fn local_step(state: &mut State, ui: &mut Ui, s: &shell::Skin, machines: &[Machine], rows: &[Workspace], now: f64) -> Option<Action> {
    shell::title(ui, s, "New workspace");
    shell::body(ui, s, "A folder on a machine, in a worktree of its own. The name comes from the folder.");
    ui.add_space(20.0);
    ui.label(RichText::new("MACHINE").font(FontId::monospace(9.5)).color(s.faint));
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        for machine in machines.iter().take(32) {
            let selected = machine.address == state.machine.address;
            let label = if machine.name.trim().is_empty() { machine.address.as_str() } else { machine.name.as_str() };
            if machine_chip(ui, s, label, selected) {
                state.choose_machine(machine.clone());
            }
        }
    });
    ui.add_space(18.0);
    if shell::field(ui, s, "Folder", "/Users/you/repo", &mut state.folder) {
        state.derive_name();
    }
    ui.add_space(14.0);
    shell::field(ui, s, "Branch", "feature/mindmap · empty uses the repository's own checkout", &mut state.branch);
    ui.add_space(14.0);
    if shell::field(ui, s, "Name", "derived from the folder", &mut state.name) {
        state.name_edited();
    }
    if let Some(error) = &state.error {
        ui.add_space(12.0);
        shell::error_note(ui, s, error);
    }
    ui.add_space(12.0);
    shell::note(ui, s, "The workspace keeps its own worktree, so drafts and Sync jobs stay with it. The agent is chosen on the Init page.");
    let clicked = shell::footer(ui, s, &[("Cancel", false, true), ("Create", true, true)]);
    match clicked {
        Some(0) => Some(Action::Close),
        Some(1) => {
            state.create(now, rows);
            None
        }
        _ => None,
    }
}

fn remote_step(state: &mut State, ui: &mut Ui, s: &shell::Skin, machines: &[Machine]) -> Option<Action> {
    shell::title(ui, s, "Which machine?");
    shell::body(ui, s, "Pick a saved SSH machine. Its folders live on that machine, not this one.");
    ui.add_space(20.0);
    let ssh: Vec<&Machine> = machines.iter().filter(|machine| machine.ssh.is_some()).collect();
    if ssh.is_empty() {
        shell::note(ui, s, "No saved machine yet. Add one over SSH first.");
    } else {
        for machine in ssh {
            if machine_option(ui, s, machine, machine.address == state.machine.address) {
                state.choose_machine(machine.clone());
            }
            ui.add_space(4.0);
        }
    }
    let clicked = shell::footer(ui, s, &[("Back", false, true), ("Add machine…", false, true)]);
    match clicked {
        Some(0) => {
            state.back();
            None
        }
        Some(1) => Some(Action::AddMachine),
        _ => None,
    }
}

fn connect_step(state: &mut State, ui: &mut Ui, s: &shell::Skin, machines: &[Machine]) -> Option<Action> {
    shell::title(ui, s, "Connect a machine first");
    shell::body(ui, s, "Workspaces are created on a machine. Add one over SSH, then pick its folder.");
    ui.add_space(20.0);
    let ssh: Vec<&Machine> = machines.iter().filter(|machine| machine.ssh.is_some()).collect();
    if ssh.is_empty() {
        shell::note(ui, s, "This computer runs the map client; remote folders need an SSH machine.");
    } else {
        shell::body(ui, s, "A machine is connected. Pick it to continue.");
        ui.add_space(8.0);
        for machine in ssh {
            if machine_option(ui, s, machine, machine.address == state.machine.address) {
                state.choose_machine(machine.clone());
            }
            ui.add_space(4.0);
        }
    }
    let clicked = shell::footer(ui, s, &[("Back", false, true), ("Add machine…", true, true)]);
    match clicked {
        Some(0) => {
            state.back();
            None
        }
        Some(1) => Some(Action::AddMachine),
        _ => None,
    }
}

fn creating_step(state: &mut State, ui: &mut Ui, s: &shell::Skin) -> Option<Action> {
    shell::title(ui, s, &format!("Creating {}", state.name.trim()));
    let body = if state.unavailable {
        "Mindmap could not create the workspace on the node."
    } else {
        "Mindmap is checking the node for a workspace in this folder."
    };
    shell::body(ui, s, body);
    ui.add_space(20.0);
    for line in ["Create the worktree", "Register it with Herdr", "Finish the setup"] {
        creating_line(ui, s, line, state.unavailable);
    }
    if state.unavailable {
        ui.add_space(16.0);
        shell::error_note(
            ui,
            s,
            "Creating a workspace on the node is not available in this build yet. Nothing was created. Create it in Herdr, then press Check again.",
        );
        let clicked = shell::footer(ui, s, &[("Back", false, true), ("Check again", true, true)]);
        match clicked {
            Some(0) => {
                state.back();
                None
            }
            Some(1) => Some(Action::Refresh),
            _ => None,
        }
    } else {
        ui.add_space(16.0);
        let clicked = shell::footer(ui, s, &[("Back", false, true)]);
        match clicked {
            Some(0) => {
                state.back();
                None
            }
            _ => None,
        }
    }
}

fn creating_line(ui: &mut Ui, s: &shell::Skin, text: &str, unavailable: bool) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(18.0, 26.0), Sense::hover());
        let dot = pos2(rect.left() + 4.0, rect.center().y);
        if unavailable {
            ui.painter().line_segment([pos2(dot.x - 3.5, dot.y), pos2(dot.x + 3.5, dot.y)], Stroke::new(1.5, s.faint));
        } else {
            ui.painter().circle_stroke(dot, 4.0, Stroke::new(1.0, s.faint));
        }
        ui.label(RichText::new(text).font(regular(12.5)).color(s.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let status = if unavailable { "not started" } else { "waiting" };
            ui.label(RichText::new(status).font(FontId::monospace(10.0)).color(s.faint));
        });
    });
}

fn init_step(state: &mut State, ui: &mut Ui, s: &shell::Skin, rows: &[Workspace]) -> Option<Action> {
    let workspace = state.workspace.clone();
    let found = workspace.as_deref().and_then(|id| rows.iter().find(|w| w.id == id));
    let name = found.map_or_else(|| state.name.clone(), |w| workspace_name(w).to_string());
    shell::title(ui, s, &format!("{name} is ready"));
    shell::body(ui, s, "The workspace exists. Open its Init page to choose an agent and press Init.");
    ui.add_space(20.0);
    workspace_row(ui, s, found, state);
    if let Some(error) = &state.error {
        ui.add_space(12.0);
        shell::error_note(ui, s, error);
    }
    let map = found.map(Workspace::map_state).unwrap_or(MapState::None);
    let open = |init_page: bool| workspace.clone().map(|id| Action::Open { id, init_page });
    if map == MapState::Map {
        let clicked = shell::footer(ui, s, &[("Done", false, true), ("Open map", true, true)]);
        match clicked {
            Some(0) => Some(Action::Close),
            Some(1) => open(false).or(Some(Action::Close)),
            _ => None,
        }
    } else {
        let clicked = shell::footer(ui, s, &[("Done", false, true), ("Open map", false, true), ("Open Init page", true, true)]);
        match clicked {
            Some(0) => Some(Action::Close),
            Some(1) => open(false).or(Some(Action::Close)),
            Some(2) => open(true).or(Some(Action::Close)),
            _ => None,
        }
    }
}

fn workspace_row(ui: &mut Ui, s: &shell::Skin, workspace: Option<&Workspace>, state: &State) {
    let (title, path, branch) = match workspace {
        Some(w) => (
            workspace_name(w).to_string(),
            w.path.clone().unwrap_or_else(|| w.repo.clone().unwrap_or_default()),
            w.branch.clone().unwrap_or_default(),
        ),
        None => (state.name.clone(), state.folder.clone(), state.branch.clone()),
    };
    egui::Frame::NONE
        .fill(s.field)
        .stroke(Stroke::new(1.0, s.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).font(semibold(13.0)).color(s.text));
            if !path.is_empty() {
                ui.label(RichText::new(path).font(FontId::monospace(10.5)).color(s.muted));
            }
            if !branch.is_empty() {
                ui.label(RichText::new(format!("branch {branch}")).font(FontId::monospace(10.0)).color(s.faint));
            }
        });
}

fn machine_chip(ui: &mut Ui, s: &shell::Skin, label: &str, selected: bool) -> bool {
    let font = semibold(11.0);
    let width = (shell::text_width(ui, label, font.clone()) + 24.0).min(ui.available_width().max(80.0));
    let (rect, response) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
    a11y::button(&response, label);
    let hot = response.hovered() || response.has_focus();
    let fill = if selected {
        shell::with_alpha(s.accent, if s.panel == Color32::WHITE { 12 } else { 30 })
    } else if hot {
        s.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(14), fill);
    let border = if selected { s.accent } else { s.border };
    ui.painter().rect_stroke(rect, CornerRadius::same(14), Stroke::new(1.0, border), StrokeKind::Inside);
    let ink = if selected {
        s.accent
    } else if hot {
        s.text
    } else {
        s.muted
    };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font, ink);
    response.clicked()
}

fn machine_option(ui: &mut Ui, s: &shell::Skin, machine: &Machine, selected: bool) -> bool {
    let label = if machine.name.trim().is_empty() { machine.address.clone() } else { machine.name.clone() };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
    a11y::button(&response, &label);
    let hot = response.hovered() || response.has_focus();
    if hot || selected {
        let fill = if selected { shell::with_alpha(s.accent, if s.panel == Color32::WHITE { 10 } else { 26 }) } else { s.hover };
        ui.painter().rect_filled(rect, CornerRadius::same(7), fill);
    }
    ui.painter().rect_stroke(rect, CornerRadius::same(7), Stroke::new(1.0, if selected { s.accent } else { s.border }), StrokeKind::Inside);
    ui.painter().text(pos2(rect.left() + 12.0, rect.center().y - 8.0), Align2::LEFT_CENTER, &label, semibold(12.0), s.text);
    let address = machine.ssh.clone().unwrap_or_else(|| machine.address.clone());
    ui.painter().text(pos2(rect.left() + 12.0, rect.center().y + 9.0), Align2::LEFT_CENTER, address, FontId::monospace(10.0), s.muted);
    ui.painter().text(pos2(rect.right() - 12.0, rect.center().y), Align2::RIGHT_CENTER, "SSH", FontId::monospace(9.0), s.faint);
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn opening_starts_on_the_local_step_with_the_folder_name() {
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
    fn the_tabs_and_remote_choice_move_between_steps() {
        let mut state = form();
        state.show_remote();
        assert_eq!(state.step, Step::Remote);
        state.choose_machine(Machine { name: "work".into(), address: "ssh:work".into(), ssh: Some("work".into()) });
        assert_eq!(state.step, Step::Local);
        assert_eq!(state.machine.address, "ssh:work");
        state.show_connect();
        assert_eq!(state.step, Step::Connect);
        state.back();
        assert_eq!(state.step, Step::Local);
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
}
