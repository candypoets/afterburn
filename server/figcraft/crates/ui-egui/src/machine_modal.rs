//! The Add machine modal: a 460 × 340 card that walks one SSH host through setup.
//!
//! Design: `Widget Add machine` on the Mindmap page (states Pick a host, Review, Setting up,
//! Needs attention, Connected). The hosts come from `~/.ssh/config`; a `user@host` can be typed.
//!
//! The setup itself is the host strip's: [`crate::node_hosts::begin`] runs the same
//! [`prepare`](crate::node_hosts) work the strip runs, reports each phase, and this modal maps
//! its outcome to a step. The step machine is pure state, so its transitions are unit-tested
//! without a window.

use egui::{Id, Rect, Ui, pos2, vec2};

use crate::FigApp;
use crate::theme::Tokens;

/// The card's size, as in the design.
const WIDTH: f32 = 460.0;
const HEIGHT: f32 = 340.0;

/// The five states of the design.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Step {
    /// Which machine? The SSH target and its display name.
    #[default]
    Add,
    /// What setup will do on the machine.
    Review,
    /// The setup is running; each phase reports as it is reached.
    SettingUp,
    /// Herdr is missing, or the machine could not be reached.
    NeedsAttention,
    /// The machine is ready and saved.
    Connected,
}

/// Why setup needs the person again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Attention {
    /// Herdr is not installed on the machine; installing it needs approval.
    HerdrMissing,
    /// The SSH/node setup failed, with the error text to show.
    Error(String),
    /// The machine runs another build and must be updated from the host strip first.
    Update(String),
}

/// The four steps the Setting up state lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    #[default]
    Ssh,
    Copy,
    Start,
    Herdr,
}

impl Stage {
    /// Every stage, in the order setup runs them.
    pub const ALL: [Stage; 4] = [Stage::Ssh, Stage::Copy, Stage::Start, Stage::Herdr];

    fn index(self) -> usize {
        match self {
            Stage::Ssh => 0,
            Stage::Copy => 1,
            Stage::Start => 2,
            Stage::Herdr => 3,
        }
    }

    /// The line's name in the Setting up state.
    pub fn label(self) -> &'static str {
        match self {
            Stage::Ssh => "SSH",
            Stage::Copy => "Copy the node",
            Stage::Start => "Start the node",
            Stage::Herdr => "Check Herdr",
        }
    }

    fn done(self) -> &'static str {
        match self {
            Stage::Ssh => "connected",
            Stage::Copy => "done",
            Stage::Start => "running",
            Stage::Herdr => "ready",
        }
    }

    /// The word while the stage runs.
    fn active(self) -> &'static str {
        match self {
            Stage::Ssh => "connecting",
            Stage::Copy => "copying",
            Stage::Start => "running",
            Stage::Herdr => "checking",
        }
    }
}

/// How a stage reads in the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageState {
    Active,
    Done,
    Waiting,
    /// The setup found the node already installed, so this step did not run.
    Skipped,
    Failed,
    /// Herdr is missing and waits for the person to approve installing it.
    NeedsApproval,
}

/// The modal's state.
#[derive(Default)]
pub struct State {
    /// The modal is open.
    pub open: bool,
    /// The step the design shows.
    pub step: Step,
    /// The SSH target typed on the first step.
    pub target: String,
    /// The display name typed on the first step; falls back to the target.
    pub name: String,
    /// The person typed a display name, so the target must not overwrite it.
    name_touched: bool,
    /// Why Continue refused the fields.
    pub error: Option<String>,
    /// Why setup stopped, when it did.
    pub attention: Option<Attention>,
    /// The machine setup reached.
    #[cfg(not(target_arch = "wasm32"))]
    pub host: Option<crate::node_hosts::Host>,
    /// The setup running on its own thread.
    #[cfg(not(target_arch = "wasm32"))]
    job: Option<crate::node_hosts::Job>,
    /// The stages the setup reported, in the order it reached them.
    reached: Vec<Stage>,
    /// The last stage the setup reported.
    current: Stage,
    /// A warning after connecting (the machine could not be saved locally).
    pub warning: Option<String>,
    /// The workspaces the node listed once connected; `None` until its list arrives.
    pub workspaces: Option<usize>,
    /// The hosts `~/.ssh/config` names, read when the modal opens.
    pub hosts: Vec<ssh_config::SshHost>,
    /// The `user@host` typed under the list (the first state's field).
    pub manual: String,
    /// The host picked in the list, used while nothing is typed.
    pub picked: String,
}

impl State {
    /// Open the modal on the first step, from scratch.
    pub fn begin(&mut self) {
        self.cancel_job();
        self.open = true;
        self.step = Step::Add;
        self.target.clear();
        self.name.clear();
        self.name_touched = false;
        self.error = None;
        self.attention = None;
        self.reached.clear();
        self.current = Stage::Ssh;
        self.warning = None;
        self.workspaces = None;
        self.manual.clear();
        self.picked.clear();
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.host = None;
            self.hosts = ssh_config::read();
        }
        // The first host is picked, as the design shows it.
        if let Some(first) = self.hosts.first() {
            self.picked = first.alias.clone();
            self.target = first.alias.clone();
            self.derive_name();
        }
    }

    /// Pick a host from the list: it becomes the SSH target and the display name.
    pub fn pick(&mut self, alias: &str) {
        self.target = alias.to_string();
        self.picked = alias.to_string();
        self.manual.clear();
        self.name_touched = false;
        self.derive_name();
        self.error = None;
    }

    /// The typed `user@host` changed: it replaces the picked host while it has text.
    pub fn typed(&mut self) {
        let manual = self.manual.trim().to_string();
        self.target = if manual.is_empty() { self.picked.clone() } else { manual };
        self.name_touched = false;
        self.derive_name();
        self.error = None;
    }

    /// Close the modal and stop the setup if it is still running.
    pub fn close(&mut self) {
        self.cancel_job();
        self.open = false;
    }

    /// The name the review and connected states use: the display name, or the target.
    pub fn machine_name(&self) -> String {
        let name = self.name.trim();
        if name.is_empty() { self.target.trim().to_string() } else { name.to_string() }
    }

    /// Validate the first step; true when it can move on.
    fn validate(&mut self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let target = self.target.trim();
            if let Err(error) = crate::node_hosts::validate_target(target) {
                self.error = Some(error);
                return false;
            }
            let name = self.name.trim();
            if name.len() > 64 || name.chars().any(char::is_control) {
                self.error = Some("Use a display name of at most 64 characters.".to_string());
                return false;
            }
            self.error = None;
            true
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.error = Some("Machines are only available in the desktop app.".to_string());
            false
        }
    }

    /// Add machine → Review. False (with [`State::error`] set) when the fields are invalid.
    pub fn continue_to_review(&mut self) -> bool {
        if self.step != Step::Add || !self.validate() {
            return false;
        }
        self.step = Step::Review;
        true
    }

    /// Go back one state: Review to Add, or a failed setup to Review.
    pub fn back(&mut self) {
        match self.step {
            Step::Review => {
                self.step = Step::Add;
                self.error = None;
            }
            Step::NeedsAttention => {
                self.step = Step::Review;
                self.attention = None;
            }
            Step::Add | Step::SettingUp | Step::Connected => {}
        }
    }

    /// The name was typed by hand: stop deriving it from the target.
    pub fn name_edited(&mut self) {
        self.name_touched = true;
    }

    /// Derive the display name from the target until the person edits it.
    pub fn derive_name(&mut self) {
        if !self.name_touched {
            self.name = self.target.trim().to_string();
        }
    }

    /// Review → Setting up. Returns the machine to prepare, or None when not on Review.
    pub fn start(&mut self) -> Option<Machine> {
        if self.step != Step::Review || !self.validate() {
            return None;
        }
        let target = self.target.trim().to_string();
        let machine = Machine { name: self.machine_name(), address: format!("ssh:{target}"), ssh: Some(target) };
        self.enter_setup();
        Some(machine)
    }

    /// Needs attention (Herdr missing) → Setting up, this time allowed to install Herdr.
    pub fn install(&mut self) -> Option<Machine> {
        if self.step != Step::NeedsAttention || self.attention != Some(Attention::HerdrMissing) {
            return None;
        }
        let machine = self.machine()?;
        self.enter_setup();
        Some(machine)
    }

    /// Needs attention (an error) → Setting up again with the same machine.
    pub fn retry(&mut self) -> Option<Machine> {
        if self.step != Step::NeedsAttention || !matches!(self.attention, Some(Attention::Error(_))) {
            return None;
        }
        let machine = self.machine()?;
        self.enter_setup();
        Some(machine)
    }

    fn enter_setup(&mut self) {
        self.cancel_job();
        self.step = Step::SettingUp;
        self.attention = None;
        self.error = None;
        self.reached.clear();
        self.current = Stage::Ssh;
    }

    /// The machine this modal prepared, when there is one.
    fn machine(&self) -> Option<Machine> {
        let target = self.target.trim();
        if target.is_empty() {
            return None;
        }
        Some(Machine { name: self.machine_name(), address: format!("ssh:{target}"), ssh: Some(target.to_string()) })
    }

    /// The setup reached its end: Connected, with the machine saved.
    pub fn ready(&mut self) {
        self.step = Step::Connected;
        self.attention = None;
        self.warning = None;
        self.workspaces = None;
        self.current = Stage::Herdr;
    }

    /// The node's list arrived: the Connected state can say how many workspaces it holds.
    pub fn set_workspaces(&mut self, count: usize) {
        self.workspaces = Some(count);
    }

    /// The setup needs Herdr installed on the machine: Needs attention, asking first.
    pub fn needs_herdr(&mut self) {
        self.step = Step::NeedsAttention;
        self.attention = Some(Attention::HerdrMissing);
    }

    /// The setup could not reach or prepare the machine.
    pub fn failed(&mut self, error: impl Into<String>) {
        self.step = Step::NeedsAttention;
        self.attention = Some(Attention::Error(error.into()));
    }

    /// The machine runs a build this app cannot prepare; the strip's Update node does it.
    pub fn needs_update(&mut self, reason: impl Into<String>) {
        self.step = Step::NeedsAttention;
        self.attention = Some(Attention::Update(reason.into()));
    }

    /// A setup could not even start on this thread.
    pub fn start_failed(&mut self, error: impl Into<String>) {
        self.failed(error);
    }

    /// Record a phase reported by the worker.
    #[cfg(not(target_arch = "wasm32"))]
    fn reach(&mut self, phase: crate::node_hosts::Phase) {
        let stage = match phase {
            crate::node_hosts::Phase::Ssh => Stage::Ssh,
            crate::node_hosts::Phase::Copy => Stage::Copy,
            crate::node_hosts::Phase::Start => Stage::Start,
            crate::node_hosts::Phase::Herdr => Stage::Herdr,
        };
        if !self.reached.contains(&stage) {
            self.reached.push(stage);
        }
        self.current = stage;
    }

    /// How `stage` reads now.
    pub fn stage_state(&self, stage: Stage) -> StageState {
        let seen = self.reached.contains(&stage);
        let stage_index = stage.index();
        let current = self.current.index();
        match self.step {
            Step::Connected => {
                if seen || stage != Stage::Copy {
                    StageState::Done
                } else {
                    StageState::Skipped
                }
            }
            Step::Add | Step::Review => StageState::Waiting,
            Step::NeedsAttention => {
                let failed = matches!(self.attention, Some(Attention::Error(_) | Attention::Update(_)));
                if stage_index < current {
                    if seen { StageState::Done } else { StageState::Skipped }
                } else if stage_index == current {
                    if failed {
                        StageState::Failed
                    } else if self.attention == Some(Attention::HerdrMissing) {
                        StageState::NeedsApproval
                    } else {
                        StageState::Waiting
                    }
                } else {
                    StageState::Waiting
                }
            }
            Step::SettingUp => {
                if stage_index < current {
                    if seen { StageState::Done } else { StageState::Skipped }
                } else if stage_index == current {
                    StageState::Active
                } else if seen {
                    StageState::Done
                } else {
                    StageState::Waiting
                }
            }
        }
    }

    /// The word beside a stage in the list.
    pub fn stage_status(&self, stage: Stage) -> &'static str {
        match self.stage_state(stage) {
            StageState::Active => stage.active(),
            StageState::Done => stage.done(),
            StageState::Waiting => "waiting",
            StageState::Skipped => "not needed",
            StageState::Failed => "failed",
            StageState::NeedsApproval => "missing",
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn cancel_job(&mut self) {
        if let Some(job) = self.job.take() {
            job.cancel();
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn cancel_job(&mut self) {}
}

/// A machine to prepare, copied out of [`State`] so the setup can own it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Machine {
    pub name: String,
    pub address: String,
    pub ssh: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
impl From<Machine> for crate::node_hosts::Host {
    fn from(machine: Machine) -> Self {
        Self { name: machine.name, address: machine.address, ssh: machine.ssh }
    }
}

/// Open the Add machine modal from the Workspaces screen.
pub fn open(app: &mut FigApp) {
    app.machine_modal.begin();
}

/// Open the modal straight into setup for `target` (the New workspace sheet's Connect).
pub fn connect(app: &mut FigApp, target: &str) {
    app.machine_modal.begin();
    app.machine_modal.manual = target.trim().to_string();
    app.machine_modal.typed();
    if app.machine_modal.continue_to_review()
        && let Some(machine) = app.machine_modal.start()
    {
        start_setup(app, machine, false);
    }
}

/// What the cards show that lives outside the modal's own state.
struct Context {
    /// The node version the local node reported (`0.1.0`), for the Review and Connected lines.
    node: Option<String>,
    /// The connected machine's workspaces are open in the sidebar.
    listed: bool,
}

/// Draw the modal when open.
pub fn show(app: &mut FigApp, ui: &mut Ui) {
    if !app.machine_modal.open {
        return;
    }
    let ctx = ui.ctx().clone();
    #[cfg(not(target_arch = "wasm32"))]
    poll(app, &ctx);
    if !app.machine_modal.open {
        return;
    }
    if app.machine_modal.step == Step::Connected && app.machine_modal.workspaces.is_none() && app.ws.busy.is_none() && app.ws.error.is_none() {
        let count = app.ws.rows.len();
        app.machine_modal.set_workspaces(count);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let listed = app.machine_modal.host.as_ref().is_some_and(|host| app.ws.open.contains(&host.address));
    #[cfg(target_arch = "wasm32")]
    let listed = false;
    let extra = Context { node: app.ws.node_version.clone(), listed };
    let tokens = Tokens::get(&ctx);
    let skin = shell::Skin::of(&tokens);
    let (should_close, action) = {
        let state = &mut app.machine_modal;
        let response = egui::Modal::new(Id::new("machine_modal"))
            .backdrop_color(shell::scrim(&tokens))
            .frame(shell::surface(&skin))
            .show(&ctx, |ui| body(state, ui, &skin, &extra));
        (response.should_close(), response.inner)
    };
    // Escape and a click outside close the card; a running setup keeps its own Cancel.
    if should_close && app.machine_modal.step != Step::SettingUp {
        app.machine_modal.close();
        return;
    }
    let Some(action) = action else { return };
    match action {
        Action::Close => app.machine_modal.close(),
        Action::Back => app.machine_modal.back(),
        Action::Continue => {
            app.machine_modal.continue_to_review();
        }
        Action::Start => {
            if let Some(machine) = app.machine_modal.start() {
                start_setup(app, machine, false);
            }
        }
        Action::Install => {
            if let Some(machine) = app.machine_modal.install() {
                start_setup(app, machine, true);
            }
        }
        Action::Retry => {
            if let Some(machine) = app.machine_modal.retry() {
                start_setup(app, machine, false);
            }
        }
        Action::Toggle => {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(host) = app.machine_modal.host.clone()
                && !app.ws.open.remove(&host.address)
            {
                app.ws.open.insert(host.address);
            }
        }
        Action::Done => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let host = app.machine_modal.host.clone();
                app.machine_modal.close();
                if let Some(host) = host {
                    let _ = crate::node_hosts::remember(&mut app.ws.hosts, &host);
                    if crate::workspaces::service_addr() != host.address {
                        crate::workspaces::use_host(app, &ctx, &host);
                    }
                    app.ws.hosts.selected = Some(host);
                }
            }
            #[cfg(target_arch = "wasm32")]
            app.machine_modal.close();
        }
    }
}

/// Start the host setup for the modal and keep its progress channel.
fn start_setup(app: &mut FigApp, machine: Machine, install: bool) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        match crate::node_hosts::begin(machine.into(), install) {
            Ok(job) => app.machine_modal.job = Some(job),
            Err(error) => app.machine_modal.start_failed(error),
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (machine, install);
        app.machine_modal.start_failed("Machines are only available in the desktop app.");
    }
}

/// Take in the setup's progress and its outcome before drawing.
#[cfg(not(target_arch = "wasm32"))]
fn poll(app: &mut FigApp, ctx: &egui::Context) {
    let (phases, outcome) = {
        let state = &mut app.machine_modal;
        let Some(mut job) = state.job.take() else { return };
        let phases = job.phases();
        let outcome = job.outcome();
        if outcome.is_none() {
            state.job = Some(job);
        }
        (phases, outcome)
    };
    for phase in phases {
        app.machine_modal.reach(phase);
    }
    let Some(outcome) = outcome else {
        ctx.request_repaint_after(std::time::Duration::from_millis(150));
        return;
    };
    match outcome {
        crate::node_hosts::Outcome::Ready(host) => {
            let saved = crate::node_hosts::remember(&mut app.ws.hosts, &host);
            app.machine_modal.host = Some(host.clone());
            crate::workspaces::use_host(app, ctx, &host);
            app.ws.hosts.selected = Some(host);
            app.machine_modal.ready();
            if let Err(error) = saved {
                app.machine_modal.warning = Some(error);
            }
        }
        crate::node_hosts::Outcome::Approval(host) => {
            app.machine_modal.host = Some(host);
            app.machine_modal.needs_herdr();
        }
        crate::node_hosts::Outcome::Update(host, reason) => {
            app.machine_modal.host = Some(host);
            app.machine_modal.needs_update(reason);
        }
        crate::node_hosts::Outcome::Failed(error) => app.machine_modal.failed(error),
    }
}

/// What a button in the modal asks for after the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Close,
    Back,
    Continue,
    Start,
    Install,
    Retry,
    Done,
    /// The Connected card's toggle: list the machine's workspaces in the sidebar, or not.
    Toggle,
}

fn body(state: &mut State, ui: &mut Ui, s: &shell::Skin, extra: &Context) -> Option<Action> {
    let card = shell::card(ui, WIDTH, HEIGHT);
    match state.step {
        Step::Add => pick_step(state, ui, s, card),
        Step::Review => review_step(state, ui, s, card, extra),
        Step::SettingUp => setup_step(state, ui, s, card),
        Step::NeedsAttention => attention_step(state, ui, s, card),
        Step::Connected => connected_step(state, ui, s, card, extra),
    }
}

/// The footer: a quiet button at the left, the dark one at the right, both 30 pt at `y`.
fn footer(ui: &mut Ui, s: &shell::Skin, card: Rect, y: f32, left: Option<(&str, f32)>, right: Option<(&str, f32, bool)>) -> (bool, bool) {
    let left = left.is_some_and(|(label, width)| {
        shell::quiet_button(ui, s, Id::new(("machine_modal_left", label)), shell::at(card, 24.0, y, width, 30.0), label)
    });
    let right = right.is_some_and(|(label, width, enabled)| {
        let width = shell::primary_width(ui, label, width);
        shell::primary_button(ui, s, Id::new(("machine_modal_right", label)), shell::at(card, WIDTH - 24.0 - width, y, width, 30.0), label, enabled)
    });
    (left, right)
}

fn pick_step(state: &mut State, ui: &mut Ui, s: &shell::Skin, card: Rect) -> Option<Action> {
    shell::eyebrow(ui, card, 22.0, "CONNECT A MACHINE", s.accent);
    shell::title(ui, s, card, 40.0, "Add an SSH host", 22.0);
    shell::body(ui, s, card, 74.0, WIDTH - 48.0, "A host from ~/.ssh/config, reached without a prompt. Mindmap never sees your password.");
    shell::label(ui, s, card, 116.0, "HOSTS");

    // The hosts, three rows tall; more scroll.
    let list = shell::at(card, 24.0, 132.0, WIDTH - 48.0, 94.0);
    let typing = !state.manual.trim().is_empty();
    let mut picked = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
        ui.set_clip_rect(list.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical().id_salt("machine_modal_hosts").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if state.hosts.is_empty() {
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), egui::Sense::hover());
                shell::mono(ui, pos2(rect.min.x + 10.0, rect.min.y + 9.0), "No hosts in ~/.ssh/config: type one below.", 8.5, s.faint);
            }
            for host in &state.hosts {
                let selected = !typing && state.picked == host.alias;
                if shell::host_row(ui, s, &host.alias, &host.detail(), selected) {
                    picked = Some(host.alias.clone());
                }
            }
        });
    });
    if let Some(alias) = picked {
        state.pick(&alias);
    }

    let field = shell::at(card, 24.0, 234.0, WIDTH - 48.0, 30.0);
    if shell::field(ui, s, Id::new("machine_modal_manual"), field, &mut state.manual, "or type user@host…", shell::FieldFont::Mono(9.0)) {
        state.typed();
    }
    match &state.error {
        Some(error) => shell::mono_wrapped(ui, card, 24.0, 276.0, WIDTH - 48.0, error, 8.5, s.accent),
        None => shell::mono(ui, card.min + vec2(24.0, 276.0), "Display name comes from the host; you can rename it later.", 8.5, s.faint),
    }
    let enabled = !state.target.trim().is_empty();
    match footer(ui, s, card, 300.0, Some(("Cancel", 90.0)), Some(("Continue", 100.0, enabled))) {
        (true, _) => Some(Action::Close),
        (_, true) => Some(Action::Continue),
        _ => {
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            (enter && enabled).then_some(Action::Continue)
        }
    }
}

fn review_step(state: &State, ui: &mut Ui, s: &shell::Skin, card: Rect, extra: &Context) -> Option<Action> {
    let name = state.machine_name();
    shell::eyebrow(ui, card, 22.0, "REVIEW", s.accent);
    shell::title(ui, s, card, 40.0, &format!("Mindmap will do this on {name}"), 20.0);
    let copy = match extra.node.as_deref().map(short_version) {
        Some(version) => format!("Copy mindmap-node {version} to ~/.local/bin"),
        None => "Copy mindmap-node to ~/.local/bin".to_string(),
    };
    let rows =
        [copy, "Start it on 127.0.0.1:47822".to_string(), format!("Add {name} as a machine in Herdr"), "Install Herdr if it is missing".to_string()];
    for (index, row) in rows.iter().enumerate() {
        let y = 86.0 + 26.0 * index as f32;
        shell::mark(ui, card, y, shell::Mark::Check, s.teal);
        shell::row_text(ui, s, card, 44.0, y, row);
    }
    shell::note_box(
        ui,
        s,
        shell::at(card, 24.0, 200.0, WIDTH - 48.0, 42.0),
        "Never: sudo, passwords, agent forwarding, or copying your keys.",
        false,
    );
    match footer(ui, s, card, 300.0, Some(("Back", 70.0)), Some(("Start setup", 120.0, true))) {
        (true, _) => Some(Action::Back),
        (_, true) => Some(Action::Start),
        _ => None,
    }
}

fn setup_step(state: &State, ui: &mut Ui, s: &shell::Skin, card: Rect) -> Option<Action> {
    let name = state.machine_name();
    shell::eyebrow(ui, card, 22.0, "SETTING UP", s.accent);
    shell::title(ui, s, card, 40.0, &format!("Connecting to {name}"), 22.0);
    for (index, stage) in Stage::ALL.into_iter().enumerate() {
        stage_line(ui, s, card, 86.0 + 26.0 * index as f32, stage, state);
    }
    shell::note_box(
        ui,
        s,
        shell::at(card, 24.0, 204.0, WIDTH - 48.0, 44.0),
        "One SSH connection stays open; the node listens on loopback only.",
        false,
    );
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    match footer(ui, s, card, 300.0, Some(("Cancel", 80.0)), None) {
        (true, _) => Some(Action::Close),
        _ => None,
    }
}

/// One setup step: its mark, its name, and its word in the status column.
fn stage_line(ui: &mut Ui, s: &shell::Skin, card: Rect, y: f32, stage: Stage, state: &State) {
    let status = state.stage_state(stage);
    let (mark, color) = match status {
        StageState::Done => (shell::Mark::Check, s.teal),
        StageState::Active => (shell::Mark::Dot, s.teal),
        StageState::Failed | StageState::NeedsApproval => (shell::Mark::Cross, s.accent),
        StageState::Skipped => (shell::Mark::Dash, s.faint),
        StageState::Waiting => (shell::Mark::Small, s.faint),
    };
    shell::mark(ui, card, y, mark, color);
    shell::row_text(ui, s, card, 44.0, y, stage.label());
    shell::mono(ui, card.min + vec2(300.0, y + 1.0), state.stage_status(stage), 8.5, color);
}

fn attention_step(state: &State, ui: &mut Ui, s: &shell::Skin, card: Rect) -> Option<Action> {
    let name = state.machine_name();
    shell::eyebrow(ui, card, 22.0, "NEEDS ATTENTION", s.accent);
    let (title, body, warning, buttons) = match &state.attention {
        Some(Attention::HerdrMissing) => (
            format!("Herdr is missing on {name}"),
            "Agents need Herdr. Install it now, or finish the setup and add it later.".to_string(),
            "herdr server is not running · install it to reach this machine's agents".to_string(),
            (("Cancel", 80.0), ("Install Herdr", 136.0), Action::Install),
        ),
        Some(Attention::Update(reason)) => (
            format!("Update {name} first"),
            "The machine runs another Mindmap node build. Select it in the sidebar and approve Update node, then try again.".to_string(),
            reason.clone(),
            (("Cancel", 80.0), ("Back", 100.0), Action::Back),
        ),
        Some(Attention::Error(error)) => (
            format!("Could not reach {name}"),
            "The setup stopped before the machine was ready. Check that SSH reaches it without a prompt, then retry.".to_string(),
            error.clone(),
            (("Cancel", 80.0), ("Retry", 100.0), Action::Retry),
        ),
        None => (
            format!("Could not set up {name}"),
            "The setup stopped. Close this and try again.".to_string(),
            String::new(),
            (("Cancel", 80.0), ("Back", 100.0), Action::Back),
        ),
    };
    shell::title(ui, s, card, 40.0, &title, 20.0);
    shell::body(ui, s, card, 80.0, WIDTH - 48.0, &body);
    if !warning.is_empty() {
        shell::note_box(ui, s, shell::at(card, 24.0, 144.0, WIDTH - 48.0, 50.0), &warning, true);
    }
    let ((left, left_w), (right, right_w), then) = buttons;
    match footer(ui, s, card, 300.0, Some((left, left_w)), Some((right, right_w, true))) {
        (true, _) => Some(Action::Close),
        (_, true) => Some(then),
        _ => None,
    }
}

fn connected_step(state: &State, ui: &mut Ui, s: &shell::Skin, card: Rect, extra: &Context) -> Option<Action> {
    let name = state.machine_name();
    shell::eyebrow(ui, card, 22.0, "CONNECTED", s.teal);
    shell::title(ui, s, card, 40.0, &format!("{name} is ready"), 22.0);
    let mut parts = Vec::new();
    if let Some(count) = state.workspaces {
        parts.push(format!("{count} workspace{}", if count == 1 { "" } else { "s" }));
    }
    parts.push("one SSH tunnel".to_string());
    parts.push("saved as a machine in Herdr.".to_string());
    shell::body(ui, s, card, 76.0, WIDTH - 48.0, &parts.join("  ·  "));

    #[cfg(not(target_arch = "wasm32"))]
    let address = state.host.as_ref().map_or_else(|| format!("ssh:{}", state.target.trim()), |host| host.address.clone());
    #[cfg(target_arch = "wasm32")]
    let address = format!("ssh:{}", state.target.trim());
    let mut detail = vec![address];
    detail.extend(extra.node.as_deref().map(|v| format!("node {}", short_version(v))));
    let box_rect = shell::at(card, 24.0, 132.0, WIDTH - 48.0, 54.0);
    ui.painter().rect_filled(box_rect, egui::CornerRadius::same(8), s.soft);
    shell::text(ui, box_rect.min + vec2(12.0, 8.0), &name, crate::theme::semibold(12.0), s.text);
    shell::mono(ui, box_rect.min + vec2(12.0, 28.0), &detail.join("  ·  "), 8.5, s.faint);
    let toggle = shell::at(card, 396.0, 151.0, 30.0, 16.0);
    let mut action = None;
    if shell::toggle(ui, s, Id::new("machine_modal_listed"), toggle, extra.listed, "List its workspaces in the sidebar") {
        action = Some(Action::Toggle);
    }
    if let Some(error) = &state.warning {
        shell::mono_wrapped(ui, card, 24.0, 200.0, WIDTH - 48.0, error, 8.5, s.accent);
    }
    // This build has no helper that opens a machine in a local Herdr window, so the design's
    // "Open Herdr" stays out rather than doing nothing.
    match footer(ui, s, card, 300.0, None, Some(("Done", 100.0, true))) {
        (_, true) => Some(Action::Done),
        _ => action,
    }
}

/// `0.1.0` reads `0.1` on the cards.
fn short_version(version: &str) -> String {
    let parts: Vec<&str> = version.trim().split('.').collect();
    match parts.as_slice() {
        [major, minor, ..] => format!("{major}.{minor}"),
        _ => version.trim().to_string(),
    }
}

/// The hosts `~/.ssh/config` names: each `Host` alias without wildcards, with its user and address.
pub mod ssh_config {
    /// One host the config names.
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct SshHost {
        pub alias: String,
        pub user: Option<String>,
        pub hostname: Option<String>,
    }

    impl SshHost {
        /// `root@100.104.9.15  ·  ~/.ssh/config`
        pub fn detail(&self) -> String {
            let host = self.hostname.clone().unwrap_or_else(|| self.alias.clone());
            let target = match &self.user {
                Some(user) => format!("{user}@{host}"),
                None => host,
            };
            format!("{target}  ·  ~/.ssh/config")
        }
    }

    /// The most hosts the list shows.
    const MAX_HOSTS: usize = 64;

    /// Read `~/.ssh/config` and the files its `Include` lines name (one level, `*` allowed).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read() -> Vec<SshHost> {
        let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else { return Vec::new() };
        let dir = home.join(".ssh");
        let Ok(text) = std::fs::read_to_string(dir.join("config")) else { return Vec::new() };
        let mut text = text;
        for include in includes(&text) {
            for path in expand(&dir, &home, &include).into_iter().take(32) {
                if let Ok(more) = std::fs::read_to_string(path) {
                    text.push('\n');
                    text.push_str(&more);
                }
            }
        }
        parse(&text)
    }

    /// The `Include` patterns in `text`.
    pub fn includes(text: &str) -> Vec<String> {
        text.lines()
            .filter_map(|line| {
                let (key, value) = split(line)?;
                key.eq_ignore_ascii_case("include").then(|| value.split_whitespace().map(str::to_string).collect::<Vec<_>>())
            })
            .flatten()
            .take(16)
            .collect()
    }

    /// The files an `Include` pattern names: relative to `~/.ssh`, `~` expanded, one `*` matched.
    #[cfg(not(target_arch = "wasm32"))]
    fn expand(dir: &std::path::Path, home: &std::path::Path, pattern: &str) -> Vec<std::path::PathBuf> {
        let path = if let Some(rest) = pattern.strip_prefix("~/") {
            home.join(rest)
        } else if pattern.starts_with('/') {
            std::path::PathBuf::from(pattern)
        } else {
            dir.join(pattern)
        };
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else { return Vec::new() };
        let Some((prefix, suffix)) = name.split_once('*') else { return vec![path] };
        let Some(parent) = path.parent() else { return Vec::new() };
        let Ok(entries) = std::fs::read_dir(parent) else { return Vec::new() };
        let mut found: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(prefix) && n.ends_with(suffix)) && p.is_file())
            .collect();
        found.sort();
        found
    }

    /// `Key value` or `Key=value`, comments and blanks skipped.
    fn split(line: &str) -> Option<(&str, &str)> {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let at = line.find(|c: char| c.is_whitespace() || c == '=')?;
        let (key, rest) = line.split_at(at);
        let value = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '=').trim();
        Some((key, value.trim_matches('"')))
    }

    /// The concrete hosts in a config's text, in order, without duplicates.
    pub fn parse(text: &str) -> Vec<SshHost> {
        let mut hosts: Vec<SshHost> = Vec::new();
        // The hosts the current `Host` block applies to (indexes into `hosts`).
        let mut block: Vec<usize> = Vec::new();
        for line in text.lines() {
            let Some((key, value)) = split(line) else { continue };
            if key.eq_ignore_ascii_case("host") {
                block.clear();
                for alias in value.split_whitespace() {
                    let concrete = !alias.contains(['*', '?', '!']) && alias.chars().all(|c| c.is_ascii_alphanumeric() || "._-@".contains(c));
                    if !concrete {
                        continue;
                    }
                    match hosts.iter().position(|h| h.alias == alias) {
                        Some(index) => block.push(index),
                        None if hosts.len() < MAX_HOSTS => {
                            hosts.push(SshHost { alias: alias.to_string(), ..SshHost::default() });
                            block.push(hosts.len() - 1);
                        }
                        None => {}
                    }
                }
            } else if key.eq_ignore_ascii_case("match") {
                block.clear();
            } else if key.eq_ignore_ascii_case("hostname") || key.eq_ignore_ascii_case("user") {
                let hostname = key.eq_ignore_ascii_case("hostname");
                for &index in &block {
                    if let Some(host) = hosts.get_mut(index) {
                        // ssh uses the first value it sees for each option.
                        let slot = if hostname { &mut host.hostname } else { &mut host.user };
                        if slot.is_none() && !value.is_empty() {
                            *slot = Some(value.to_string());
                        }
                    }
                }
            }
        }
        hosts
    }
}

/// The cards' shared drawing pieces: the New workspace sheet uses the same surface and controls.
/// Everything is placed at the design's offsets from the card's top-left corner.
pub(crate) mod shell {
    use egui::text::{LayoutJob, TextFormat};
    use egui::{Align2, Color32, CornerRadius, FontFamily, FontId, Id, Margin, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

    use crate::a11y;
    use crate::theme::{Tokens, regular, semibold};

    /// The cards' colours: the map design's inks on light, the theme's surfaces on dark.
    pub(crate) struct Skin {
        pub(crate) panel: Color32,
        pub(crate) text: Color32,
        pub(crate) muted: Color32,
        pub(crate) faint: Color32,
        pub(crate) border: Color32,
        /// The selected row, the notes and the machine card.
        pub(crate) soft: Color32,
        /// Hover on a quiet row or button.
        pub(crate) hover: Color32,
        pub(crate) accent: Color32,
        pub(crate) teal: Color32,
        /// The dark primary button and its label.
        pub(crate) ink: Color32,
        pub(crate) ink_hover: Color32,
        pub(crate) on_ink: Color32,
        pub(crate) warn_fill: Color32,
        pub(crate) warn_stroke: Color32,
        pub(crate) shadow: Color32,
        pub(crate) focus: Color32,
    }

    impl Skin {
        pub(crate) fn of(t: &Tokens) -> Self {
            if t.dark {
                let accent = Color32::from_rgb(0xD9, 0x7A, 0x5F);
                Self {
                    panel: t.panel,
                    text: t.text,
                    muted: t.text_secondary,
                    faint: t.text_tertiary,
                    border: t.border,
                    soft: t.hover,
                    hover: t.hover,
                    accent,
                    teal: Color32::from_rgb(0x7F, 0xBD, 0xB5),
                    ink: Color32::from_rgb(0xEC, 0xEA, 0xE6),
                    ink_hover: Color32::WHITE,
                    on_ink: Color32::from_rgb(0x1A, 0x1A, 0x1A),
                    warn_fill: with_alpha(accent, 24),
                    warn_stroke: with_alpha(accent, 90),
                    shadow: t.shadow,
                    focus: t.focus,
                }
            } else {
                let c = crate::map_colors::get();
                Self {
                    panel: c.panel,
                    text: c.ink,
                    muted: c.muted,
                    faint: c.faint,
                    border: c.border,
                    soft: c.tab_inactive,
                    hover: c.tab_row,
                    accent: c.accent,
                    teal: c.map_ink,
                    ink: c.ink,
                    ink_hover: Color32::from_rgb(0x33, 0x33, 0x33),
                    on_ink: Color32::WHITE,
                    warn_fill: Color32::from_rgb(0xFB, 0xF4, 0xF1),
                    warn_stroke: Color32::from_rgb(0xE8, 0xD5, 0xCE),
                    shadow: t.shadow,
                    focus: t.focus,
                }
            }
        }
    }

    /// The dimmed backdrop behind the card.
    pub(crate) fn scrim(t: &Tokens) -> Color32 {
        Color32::from_black_alpha(if t.dark { 130 } else { 70 })
    }

    /// The card: white, a hairline border, 12 pt corners, lifted off the scrim. No padding: the
    /// content is placed at the design's offsets.
    pub(crate) fn surface(s: &Skin) -> egui::Frame {
        egui::Frame::NONE
            .fill(s.panel)
            .stroke(Stroke::new(1.0, s.border))
            .corner_radius(CornerRadius::same(12))
            .shadow(egui::epaint::Shadow { offset: [0, 16], blur: 40, spread: 0, color: s.shadow })
            .inner_margin(Margin::ZERO)
    }

    /// Claim the card's area (shrunk to fit a small window) and return it.
    pub(crate) fn card(ui: &mut Ui, width: f32, height: f32) -> Rect {
        let room = ui.ctx().content_rect().size() - vec2(24.0, 24.0);
        let size = vec2(width.min(room.x.max(240.0)), height.min(room.y.max(200.0)));
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        rect
    }

    /// The rect at `(x, y)` from the card's corner.
    pub(crate) fn at(card: Rect, x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect::from_min_size(card.min + vec2(x, y), vec2(width, height))
    }

    pub(crate) fn text(ui: &Ui, pos: Pos2, text: &str, font: FontId, color: Color32) {
        ui.painter().text(pos, Align2::LEFT_TOP, text, font, color);
    }

    pub(crate) fn mono(ui: &Ui, pos: Pos2, text: &str, size: f32, color: Color32) {
        ui.painter().text(pos, Align2::LEFT_TOP, text, FontId::monospace(size), color);
    }

    /// Mono text wrapped to `width`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn mono_wrapped(ui: &Ui, card: Rect, x: f32, y: f32, width: f32, text: &str, size: f32, color: Color32) {
        let galley = ui.painter().layout(text.to_string(), FontId::monospace(size), color, width);
        ui.painter().galley(card.min + vec2(x, y), galley, color);
    }

    /// The red mono eyebrow over the title.
    pub(crate) fn eyebrow(ui: &Ui, card: Rect, y: f32, text: &str, color: Color32) {
        mono(ui, card.min + vec2(24.0, y), text, 9.0, color);
    }

    /// A field's or a list's mono label (HOSTS, MACHINE, FOLDER…).
    pub(crate) fn label(ui: &Ui, s: &Skin, card: Rect, y: f32, text: &str) {
        mono(ui, card.min + vec2(24.0, y), text, 8.5, s.muted);
    }

    /// The serif title. The design's box tops sit a few points above the glyphs.
    pub(crate) fn title(ui: &Ui, s: &Skin, card: Rect, y: f32, text: &str, size: f32) {
        let font = serif(ui.ctx(), size);
        let galley = ui.painter().layout(text.to_string(), font, s.text, card.width() - 48.0);
        ui.painter().galley(card.min + vec2(24.0, y - 4.0), galley, s.text);
    }

    /// Body copy: Inter 11 on a 16 pt line, wrapped to `width`.
    pub(crate) fn body(ui: &Ui, s: &Skin, card: Rect, y: f32, width: f32, text: &str) {
        let mut job = LayoutJob::default();
        job.wrap.max_width = width;
        job.append(text, 0.0, TextFormat { font_id: regular(11.0), color: s.muted, line_height: Some(16.0), ..TextFormat::default() });
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(card.min + vec2(24.0, y), galley, s.muted);
    }

    /// A step or check row's text.
    pub(crate) fn row_text(ui: &Ui, s: &Skin, card: Rect, x: f32, y: f32, text: &str) {
        let room = card.width() - x - 24.0;
        let galley = ui.painter().layout(text.to_string(), regular(10.5), s.text, room);
        ui.painter().galley(card.min + vec2(x, y), galley, s.text);
    }

    /// The mark at the start of a check or step row.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Mark {
        Check,
        Dot,
        Small,
        Dash,
        Cross,
    }

    /// Draw `mark` in the 14 pt column at x = 24; drawn, so it never depends on a font's glyphs.
    pub(crate) fn mark(ui: &Ui, card: Rect, y: f32, mark: Mark, color: Color32) {
        let c = card.min + vec2(28.0, y + 6.5);
        let painter = ui.painter();
        match mark {
            Mark::Check => {
                let stroke = Stroke::new(1.4, color);
                painter.line_segment([pos2(c.x - 3.5, c.y), pos2(c.x - 1.0, c.y + 3.0)], stroke);
                painter.line_segment([pos2(c.x - 1.0, c.y + 3.0), pos2(c.x + 4.0, c.y - 3.5)], stroke);
            }
            Mark::Dot => {
                painter.circle_filled(c, 2.5, color);
            }
            Mark::Small => {
                painter.circle_filled(c, 1.2, color);
            }
            Mark::Dash => {
                painter.line_segment([pos2(c.x - 3.0, c.y), pos2(c.x + 3.0, c.y)], Stroke::new(1.3, color));
            }
            Mark::Cross => {
                let stroke = Stroke::new(1.4, color);
                painter.line_segment([pos2(c.x - 3.0, c.y - 3.0), pos2(c.x + 3.0, c.y + 3.0)], stroke);
                painter.line_segment([pos2(c.x - 3.0, c.y + 3.0), pos2(c.x + 3.0, c.y - 3.0)], stroke);
            }
        }
    }

    /// A rounded note: soft and muted, or (`warn`) the pale red box with accent text.
    pub(crate) fn note_box(ui: &Ui, s: &Skin, rect: Rect, text: &str, warn: bool) {
        let (fill, ink) = if warn { (s.warn_fill, s.accent) } else { (s.soft, s.muted) };
        ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
        if warn {
            ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, s.warn_stroke), StrokeKind::Inside);
        }
        let galley = ui.painter().layout(text.to_string(), FontId::monospace(8.5), ink, rect.width() - 24.0);
        ui.painter().with_clip_rect(rect).galley(rect.min + vec2(12.0, if warn { 10.0 } else { 8.0 }), galley, ink);
    }

    /// One host in a pick list: name over its address, the picked one shaded with a check.
    pub(crate) fn host_row(ui: &mut Ui, s: &Skin, name: &str, detail: &str, selected: bool) -> bool {
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
        a11y::button(&response, name);
        let hot = response.hovered() || response.has_focus();
        if selected || hot {
            ui.painter().rect_filled(rect, CornerRadius::same(6), if selected { s.soft } else { s.hover });
        }
        text(ui, rect.min + vec2(10.0, 2.0), name, semibold(11.0), s.text);
        mono(ui, rect.min + vec2(10.0, 16.0), detail, 8.0, s.faint);
        if selected {
            let c = pos2(rect.max.x - 22.0, rect.center().y);
            let stroke = Stroke::new(1.4, s.teal);
            ui.painter().line_segment([pos2(c.x - 3.5, c.y), pos2(c.x - 1.0, c.y + 3.0)], stroke);
            ui.painter().line_segment([pos2(c.x - 1.0, c.y + 3.0), pos2(c.x + 4.0, c.y - 3.5)], stroke);
        }
        response.clicked()
    }

    /// What a field's text is set in.
    #[derive(Clone, Copy, Debug)]
    pub(crate) enum FieldFont {
        Mono(f32),
        Inter(f32),
    }

    /// A bordered single-line field at `rect`; true when the text changed.
    pub(crate) fn field(ui: &mut Ui, s: &Skin, id: Id, rect: Rect, value: &mut String, hint: &str, font: FieldFont) -> bool {
        field_response(ui, s, id, rect, value, hint, font, false).changed()
    }

    /// [`field`] returning the whole response; `invalid` draws the border in the accent.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn field_response(
        ui: &mut Ui,
        s: &Skin,
        id: Id,
        rect: Rect,
        value: &mut String,
        hint: &str,
        font: FieldFont,
        invalid: bool,
    ) -> Response {
        let font = match font {
            FieldFont::Mono(size) => FontId::monospace(size),
            FieldFont::Inter(size) => regular(size),
        };
        ui.painter().rect_filled(rect, CornerRadius::same(6), s.panel);
        let inner = rect.shrink2(vec2(12.0, 0.0));
        let response = ui.put(
            inner,
            egui::TextEdit::singleline(value)
                .id(id)
                .font(font.clone())
                .text_color(s.text)
                .hint_text(egui::RichText::new(hint).font(font).color(s.faint))
                .frame(egui::Frame::NONE)
                .margin(Margin::ZERO)
                .vertical_align(egui::Align::Center)
                .desired_width(inner.width()),
        );
        let border = if invalid {
            s.accent
        } else if response.has_focus() {
            s.muted
        } else {
            s.border
        };
        ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, border), StrokeKind::Inside);
        response
    }

    /// A quiet text button (Cancel, Back): the label 10 pt in, a soft fill on hover.
    pub(crate) fn quiet_button(ui: &mut Ui, s: &Skin, id: Id, rect: Rect, label: &str) -> bool {
        let response = ui.interact(rect, id, Sense::click());
        a11y::button(&response, label);
        if response.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(6), s.hover);
        }
        text(ui, rect.min + vec2(10.0, 8.0), label, semibold(10.5), s.text);
        focus(ui, s, rect, &response);
        response.clicked()
    }

    /// The dark primary button's width: the design's, or wider for a longer label.
    pub(crate) fn primary_width(ui: &Ui, label: &str, design: f32) -> f32 {
        design.max(text_width(ui, label, semibold(10.5)) + 24.0)
    }

    /// The dark primary button: the label 10 pt in, white on near-black.
    pub(crate) fn primary_button(ui: &mut Ui, s: &Skin, id: Id, rect: Rect, label: &str, enabled: bool) -> bool {
        let response = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
        a11y::button(&response, label);
        let fill = if !enabled {
            s.soft
        } else if response.hovered() {
            s.ink_hover
        } else {
            s.ink
        };
        ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
        text(ui, rect.min + vec2(10.0, 8.0), label, semibold(10.5), if enabled { s.on_ink } else { s.faint });
        focus(ui, s, rect, &response);
        enabled && response.clicked()
    }

    /// The small on/off switch of the Connected card.
    pub(crate) fn toggle(ui: &mut Ui, s: &Skin, id: Id, rect: Rect, on: bool, label: &str) -> bool {
        let response = ui.interact(rect, id, Sense::click());
        a11y::button(&response, label);
        let response = response.on_hover_text(label);
        ui.painter().rect_filled(rect, CornerRadius::same(8), if on { s.teal } else { s.border });
        let knob = if on { rect.right_center() - vec2(8.0, 0.0) } else { rect.left_center() + vec2(8.0, 0.0) };
        ui.painter().circle_filled(knob, 6.0, Color32::WHITE);
        focus(ui, s, rect, &response);
        response.clicked()
    }

    fn focus(ui: &Ui, s: &Skin, rect: Rect, response: &Response) {
        if response.has_focus() {
            ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(2.0, s.focus), StrokeKind::Outside);
        }
    }

    pub(crate) fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
        ui.painter().layout_no_wrap(text.to_string(), font, Color32::WHITE).size().x
    }

    pub(crate) fn with_alpha(color: Color32, alpha: u8) -> Color32 {
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
    }

    /// The family name the serif title face is registered under.
    const SERIF: &str = "mindmap-serif";

    /// The titles' serif (Charter, as designed, or the nearest serif the system has). Until the
    /// face is registered, and where none exists, the title falls back to Inter.
    pub(crate) fn serif(ctx: &egui::Context, size: f32) -> FontId {
        let family = FontFamily::Name(SERIF.into());
        if ctx.fonts(|fonts| fonts.definitions().families.contains_key(&family)) {
            return FontId::new(size, family);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(data) = serif_face() {
            ctx.add_font(egui::epaint::text::FontInsert::new(
                SERIF,
                data.clone(),
                vec![egui::epaint::text::InsertFontFamily { family, priority: egui::epaint::text::FontPriority::Highest }],
            ));
        }
        regular(size)
    }

    /// The first serif family the system has, read once.
    #[cfg(not(target_arch = "wasm32"))]
    fn serif_face() -> Option<&'static egui::FontData> {
        static FACE: std::sync::OnceLock<Option<egui::FontData>> = std::sync::OnceLock::new();
        FACE.get_or_init(|| {
            const FAMILIES: [&str; 8] =
                ["Charter", "Iowan Old Style", "Georgia", "New York", "Source Serif 4", "Noto Serif", "DejaVu Serif", "Liberation Serif"];
            FAMILIES.iter().find_map(|family| {
                if !figcraft_text::has_family(family) {
                    return None;
                }
                let face = figcraft_text::find_face(family, "Regular")?;
                face.family.eq_ignore_ascii_case(family).then(|| crate::theme::font_data(&face))
            })
        })
        .as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typing(target: &str) -> State {
        let mut state = State::default();
        state.begin();
        state.target = target.to_string();
        state.derive_name();
        state
    }

    #[test]
    fn the_first_step_needs_a_valid_ssh_target() {
        let mut state = typing("-oProxyCommand=bad");
        assert!(!state.continue_to_review());
        assert_eq!(state.step, Step::Add);
        assert!(state.error.is_some());

        let mut state = typing("alice@work.example");
        assert!(state.continue_to_review());
        assert_eq!(state.step, Step::Review);
        assert!(state.error.is_none());
    }

    #[test]
    fn review_back_returns_to_the_form_and_start_names_the_ssh_machine() {
        let mut state = typing("befree");
        assert_eq!(state.machine_name(), "befree");
        assert!(state.continue_to_review());
        assert_eq!(state.step, Step::Review);

        let machine = state.start().expect("start from review");
        assert_eq!(machine.address, "ssh:befree");
        assert_eq!(machine.ssh.as_deref(), Some("befree"));
        assert_eq!(machine.name, "befree");
        assert_eq!(state.step, Step::SettingUp);

        let mut state = typing("befree");
        assert!(state.continue_to_review());
        state.back();
        assert_eq!(state.step, Step::Add);
        assert!(state.attention.is_none());
    }

    #[test]
    fn a_typed_display_name_survives_and_an_empty_one_falls_back_to_the_target() {
        let mut state = typing("befree");
        state.name = "Ben's box".into();
        state.name_edited();
        state.derive_name();
        assert_eq!(state.machine_name(), "Ben's box");

        let mut state = typing("befree");
        state.name.clear();
        state.name_edited();
        assert_eq!(state.machine_name(), "befree");

        let mut state = typing("befree");
        state.name = "x".repeat(65);
        assert!(!state.continue_to_review());
        assert!(state.error.is_some());
    }

    #[test]
    fn setup_outcomes_map_to_the_design_states() {
        let mut state = typing("befree");
        assert!(state.continue_to_review());
        state.start().expect("start");
        state.ready();
        state.set_workspaces(4);
        assert_eq!(state.step, Step::Connected);
        assert_eq!(state.workspaces, Some(4));

        let mut state = typing("befree");
        assert!(state.continue_to_review());
        state.start().expect("start");
        state.needs_herdr();
        assert_eq!(state.step, Step::NeedsAttention);
        assert_eq!(state.attention, Some(Attention::HerdrMissing));
        let machine = state.install().expect("install from approval");
        assert_eq!(machine.address, "ssh:befree");
        assert_eq!(state.step, Step::SettingUp);
        assert!(state.attention.is_none());

        let mut state = typing("befree");
        assert!(state.continue_to_review());
        state.start().expect("start");
        state.failed("SSH tunnel failed");
        assert_eq!(state.attention, Some(Attention::Error("SSH tunnel failed".into())));
        assert_eq!(state.step, Step::NeedsAttention);
        state.retry().expect("retry after an error");
        assert_eq!(state.step, Step::SettingUp);

        let mut state = typing("befree");
        assert!(state.continue_to_review());
        state.start().expect("start");
        state.needs_update("Update node");
        assert_eq!(state.attention, Some(Attention::Update("Update node".into())));
        assert!(state.install().is_none());
        assert!(state.retry().is_none());
        state.back();
        assert_eq!(state.step, Step::Review);
    }

    #[test]
    fn stage_progress_marks_a_skipped_copy_and_the_current_step() {
        let mut state = typing("befree");
        assert!(state.continue_to_review());
        state.start().expect("start");
        assert_eq!(state.stage_state(Stage::Ssh), StageState::Active);
        assert_eq!(state.stage_state(Stage::Copy), StageState::Waiting);

        state.reach(crate::node_hosts::Phase::Ssh);
        state.reach(crate::node_hosts::Phase::Start);
        state.reach(crate::node_hosts::Phase::Herdr);
        assert_eq!(state.stage_state(Stage::Ssh), StageState::Done);
        assert_eq!(state.stage_state(Stage::Copy), StageState::Skipped);
        assert_eq!(state.stage_state(Stage::Start), StageState::Done);
        assert_eq!(state.stage_state(Stage::Herdr), StageState::Active);
        assert_eq!(state.stage_status(Stage::Copy), "not needed");

        state.ready();
        assert_eq!(state.stage_state(Stage::Copy), StageState::Skipped);
        assert_eq!(state.stage_state(Stage::Herdr), StageState::Done);
        assert_eq!(state.stage_status(Stage::Herdr), "ready");
    }

    #[test]
    fn closing_and_reopening_starts_a_clean_wizard() {
        let mut state = typing("befree");
        assert!(state.continue_to_review());
        state.start().expect("start");
        state.close();
        assert!(!state.open);
        assert!(state.job.is_none());

        state.begin();
        assert!(state.open);
        assert_eq!(state.step, Step::Add);
        // The first host in ~/.ssh/config is picked, as the design shows; none, nothing.
        assert_eq!(state.target, state.hosts.first().map(|host| host.alias.clone()).unwrap_or_default());
        assert!(state.manual.is_empty());
        assert!(state.attention.is_none());
    }

    #[test]
    fn picking_a_host_and_typing_one_set_the_target() {
        let mut state = State::default();
        state.begin();
        state.pick("studio");
        assert_eq!(state.target, "studio");
        assert_eq!(state.machine_name(), "studio");
        state.manual = "alice@work.example".into();
        state.typed();
        assert_eq!(state.target, "alice@work.example");
        assert_eq!(state.machine_name(), "alice@work.example");
        state.manual.clear();
        state.typed();
        assert_eq!(state.target, "studio");
    }

    #[test]
    fn ssh_config_lists_concrete_hosts_with_their_user_and_address() {
        let text = "# work\nHost befree box\n  HostName 100.104.9.15\n  User root\n  User ignored\nHost *.internal !bad\n  User nobody\nHost=studio\n\tHostname=studio.local\nMatch host x\n  User skipped\nInclude config.d/*\nHost befree\n  HostName later.example\n";
        let hosts = ssh_config::parse(text);
        let aliases: Vec<&str> = hosts.iter().map(|h| h.alias.as_str()).collect();
        assert_eq!(aliases, ["befree", "box", "studio"]);
        assert_eq!(hosts[0].user.as_deref(), Some("root"));
        assert_eq!(hosts[0].hostname.as_deref(), Some("100.104.9.15"));
        assert_eq!(hosts[0].detail(), "root@100.104.9.15  ·  ~/.ssh/config");
        assert_eq!(hosts[2].hostname.as_deref(), Some("studio.local"));
        assert_eq!(hosts[2].user, None);
        assert_eq!(hosts[2].detail(), "studio.local  ·  ~/.ssh/config");
        assert_eq!(ssh_config::includes(text), ["config.d/*"]);
        assert!(ssh_config::parse("Host *\n User x\n").is_empty());
    }

    #[test]
    fn versions_read_major_and_minor() {
        assert_eq!(short_version("0.1.0"), "0.1");
        assert_eq!(short_version("2"), "2");
    }
}
