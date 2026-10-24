//! The Add machine modal: one page-wide modal with the setup states side by side.
//!
//! Design: `Widget Add machine` on the Mindmap page (states Add machine, Review, Setting up,
//! Needs attention, Connected).
//!
//! The setup itself is the host strip's: [`crate::node_hosts::begin`] runs the same
//! [`prepare`](crate::node_hosts) work the strip runs, reports each phase, and this modal maps
//! its outcome to a step. The step machine is pure state, so its transitions are unit-tested
//! without a window.

use egui::{Align2, Color32, CornerRadius, FontId, Id, Margin, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::FigApp;
use crate::a11y;
use crate::theme::{Tokens, regular, semibold};

/// The modal's surface width, as in the design.
const WIDTH: f32 = 940.0;
/// The space between the surface and the window edge before the modal shrinks.
const EDGE: f32 = 48.0;

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
            Stage::Copy => "Copy mindmap-node to ~/.local/bin",
            Stage::Start => "Start it on 127.0.0.1:47822",
            Stage::Herdr => "Check Herdr",
        }
    }

    fn done(self) -> &'static str {
        match self {
            Stage::Ssh => "connected",
            Stage::Copy => "copied",
            Stage::Start => "running",
            Stage::Herdr => "ready",
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
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.host = None;
        }
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
            StageState::Active => "working…",
            StageState::Done => stage.done(),
            StageState::Waiting => "waiting",
            StageState::Skipped => "not needed",
            StageState::Failed => "failed",
            StageState::NeedsApproval => "Herdr missing",
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
    let tokens = Tokens::get(&ctx);
    let skin = shell::Skin::of(&tokens);
    let width = (WIDTH - 56.0).min((ctx.content_rect().width() - EDGE - 56.0).max(320.0));
    let (should_close, action) = {
        let state = &mut app.machine_modal;
        let response =
            egui::Modal::new(Id::new("machine_modal")).backdrop_color(shell::scrim(&tokens)).frame(shell::surface(&skin)).show(&ctx, |ui| {
                ui.set_width(width);
                body(state, ui, &skin)
            });
        (response.should_close(), response.inner)
    };
    if should_close {
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

/// Start the host strip's setup for the modal and keep its progress channel.
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
    let Some(outcome) = outcome else { return };
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
}

fn body(state: &mut State, ui: &mut Ui, s: &shell::Skin) -> Option<Action> {
    let mut action = None;
    if shell::header(ui, s, "ADD MACHINE", step_note(state.step)) {
        action = Some(Action::Close);
    }
    ui.add_space(10.0);
    action = action.or(match state.step {
        Step::Add => add_step(state, ui, s),
        Step::Review => review_step(state, ui, s),
        Step::SettingUp => setup_step(state, ui, s),
        Step::NeedsAttention => attention_step(state, ui, s),
        Step::Connected => connected_step(state, ui, s),
    });
    action
}

fn step_note(step: Step) -> &'static str {
    match step {
        Step::Add => "STEP 1 OF 2",
        Step::Review => "STEP 2 OF 2",
        Step::SettingUp | Step::NeedsAttention | Step::Connected => "SETUP",
    }
}

fn add_step(state: &mut State, ui: &mut Ui, s: &shell::Skin) -> Option<Action> {
    shell::title(ui, s, "Which machine?");
    shell::body(ui, s, "An SSH target Mindmap can reach without a prompt: a Host from your ~/.ssh/config, or user@host.");
    ui.add_space(22.0);
    let changed = shell::field(ui, s, "SSH target", "Host from ~/.ssh/config, or user@host", &mut state.target);
    if changed {
        state.derive_name();
    }
    ui.add_space(14.0);
    let name = shell::field(ui, s, "Display name", "e.g. befree", &mut state.name);
    if name {
        state.name_edited();
    }
    if let Some(error) = &state.error {
        ui.add_space(12.0);
        shell::error_note(ui, s, error);
    }
    ui.add_space(16.0);
    shell::note(
        ui,
        s,
        "Mindmap never asks for a password here: set up your SSH key in a terminal first. It never uses sudo, agent forwarding, or copies your keys.",
    );
    let clicked = shell::footer(ui, s, &[("Cancel", false, true), ("Continue", true, true)]);
    match clicked {
        Some(0) => Some(Action::Close),
        Some(1) => Some(Action::Continue),
        _ => None,
    }
}

fn review_step(state: &State, ui: &mut Ui, s: &shell::Skin) -> Option<Action> {
    let name = state.machine_name();
    shell::title(ui, s, &format!("Mindmap will do this on {name}"));
    shell::body(ui, s, "One setup, no prompts after this.");
    ui.add_space(20.0);
    for line in ["Copy mindmap-node to ~/.local/bin", "Start it on 127.0.0.1:47822", "Install Herdr if it is missing (asks again)"] {
        review_line(ui, s, line);
    }
    review_line(ui, s, &format!("Save {name} as a machine in Herdr"));
    ui.add_space(18.0);
    shell::note(ui, s, "Never: sudo, passwords, agent forwarding, or copying your keys.");
    let clicked = shell::footer(ui, s, &[("Back", false, true), ("Start setup", true, true)]);
    match clicked {
        Some(0) => Some(Action::Back),
        Some(1) => Some(Action::Start),
        _ => None,
    }
}

fn review_line(ui: &mut Ui, s: &shell::Skin, text: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(18.0, 24.0), Sense::hover());
        ui.painter().circle_filled(pos2(rect.left() + 4.0, rect.center().y), 2.5, s.accent);
        ui.add(egui::Label::new(RichText::new(text).font(regular(13.0)).color(s.text)).wrap());
    });
    ui.add_space(4.0);
}

fn setup_step(state: &State, ui: &mut Ui, s: &shell::Skin) -> Option<Action> {
    let name = state.machine_name();
    shell::title(ui, s, &format!("Connecting to {name}"));
    shell::body(ui, s, "Mindmap copies, starts and checks its node over SSH. This can take a minute.");
    ui.add_space(22.0);
    for stage in Stage::ALL {
        stage_line(ui, s, stage, state);
    }
    ui.add_space(18.0);
    let clicked = shell::footer(ui, s, &[("Cancel", false, true)]);
    match clicked {
        Some(0) => Some(Action::Close),
        _ => None,
    }
}

fn stage_line(ui: &mut Ui, s: &shell::Skin, stage: Stage, state: &State) {
    let status = state.stage_state(stage);
    let color = match status {
        StageState::Done => s.success,
        StageState::Failed => s.danger,
        StageState::Active | StageState::NeedsApproval => s.accent,
        StageState::Waiting | StageState::Skipped => s.faint,
    };
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let dot = pos2(rect.left() + 7.0, rect.center().y);
    match status {
        StageState::Done => {
            let stroke = Stroke::new(1.6, color);
            ui.painter().line_segment([pos2(dot.x - 4.0, dot.y), pos2(dot.x - 1.0, dot.y + 3.5)], stroke);
            ui.painter().line_segment([pos2(dot.x - 1.0, dot.y + 3.5), pos2(dot.x + 4.5, dot.y - 4.0)], stroke);
        }
        StageState::Failed => {
            let stroke = Stroke::new(1.6, color);
            ui.painter().line_segment([pos2(dot.x - 4.0, dot.y - 4.0), pos2(dot.x + 4.0, dot.y + 4.0)], stroke);
            ui.painter().line_segment([pos2(dot.x - 4.0, dot.y + 4.0), pos2(dot.x + 4.0, dot.y - 4.0)], stroke);
        }
        StageState::Skipped => {
            ui.painter().line_segment([pos2(dot.x - 4.0, dot.y), pos2(dot.x + 4.0, dot.y)], Stroke::new(1.5, color));
        }
        StageState::Active | StageState::NeedsApproval => {
            ui.painter().circle_stroke(dot, 4.0, Stroke::new(1.5, color));
            ui.painter().circle_filled(dot, 1.6, color);
        }
        StageState::Waiting => {
            ui.painter().circle_stroke(dot, 4.0, Stroke::new(1.0, color));
        }
    }
    ui.painter().text(pos2(rect.left() + 20.0, rect.center().y), Align2::LEFT_CENTER, stage.label(), regular(12.5), s.text);
    let status = state.stage_status(stage);
    let status_w = shell::text_width(ui, status, semibold(10.5));
    ui.painter().text(pos2(rect.right() - status_w - 2.0, rect.center().y), Align2::LEFT_CENTER, status, semibold(10.5), color);
}

fn attention_step(state: &State, ui: &mut Ui, s: &shell::Skin) -> Option<Action> {
    let name = state.machine_name();
    match &state.attention {
        Some(Attention::HerdrMissing) => {
            shell::title(ui, s, &format!("Herdr is missing on {name}"));
            shell::body(ui, s, "Agents need Herdr. Install it now, or finish the setup and add it later.");
            ui.add_space(18.0);
            shell::note(ui, s, "Installing Herdr runs the machine's own package manager. Mindmap asks again before it does.");
            let clicked = shell::footer(ui, s, &[("Cancel", false, true), ("Install Herdr", true, true)]);
            match clicked {
                Some(0) => Some(Action::Close),
                Some(1) => Some(Action::Install),
                _ => None,
            }
        }
        Some(Attention::Update(reason)) => {
            shell::title(ui, s, &format!("Update {name} first"));
            shell::body(ui, s, "The machine runs another Mindmap node build. Select the host in the strip and approve Update node, then try again.");
            ui.add_space(16.0);
            shell::error_note(ui, s, reason);
            let clicked = shell::footer(ui, s, &[("Cancel", false, true), ("Back", true, true)]);
            match clicked {
                Some(0) => Some(Action::Close),
                Some(1) => Some(Action::Back),
                _ => None,
            }
        }
        Some(Attention::Error(error)) => {
            shell::title(ui, s, &format!("Could not set up {name}"));
            shell::body(ui, s, "The machine did not answer as the setup expected. Check its SSH access and try again.");
            ui.add_space(16.0);
            shell::error_note(ui, s, error);
            let clicked = shell::footer(ui, s, &[("Cancel", false, true), ("Retry", true, true)]);
            match clicked {
                Some(0) => Some(Action::Close),
                Some(1) => Some(Action::Retry),
                _ => None,
            }
        }
        None => {
            shell::title(ui, s, &format!("Could not set up {name}"));
            shell::body(ui, s, "The setup stopped. Close this and try again from the host strip.");
            Some(Action::Close)
        }
    }
}

fn connected_step(state: &State, ui: &mut Ui, s: &shell::Skin) -> Option<Action> {
    let name = state.machine_name();
    shell::title(ui, s, &format!("{name} is ready"));
    let line = match state.workspaces {
        Some(count) => format!("{count} workspace{} · one SSH tunnel · saved as a machine in Herdr.", if count == 1 { "" } else { "s" }),
        None => "One SSH tunnel · saved as a machine in Herdr.".to_string(),
    };
    shell::body(ui, s, &line);
    ui.add_space(20.0);
    machine_row(state, ui, s, &name);
    if let Some(error) = &state.warning {
        ui.add_space(12.0);
        shell::error_note(ui, s, error);
    }
    // This build has no helper that can hand a saved machine to a local Herdr, so the design's
    // optional "Open in Herdr" button is hidden rather than faked.
    let mut buttons = vec![("Done", true, true)];
    if HAS_HERDR_OPENER {
        buttons.insert(0, ("Open in Herdr", false, true));
    }
    let clicked = shell::footer(ui, s, &buttons);
    match clicked {
        Some(0) if HAS_HERDR_OPENER => Some(Action::Close),
        Some(0) => Some(Action::Done),
        Some(1) => Some(Action::Done),
        _ => None,
    }
}

/// Whether this build has a helper that opens a machine in a local Herdr window. It does not
/// today; the Connected state hides the optional action instead of pretending it worked.
const HAS_HERDR_OPENER: bool = false;

fn machine_row(state: &State, ui: &mut Ui, s: &shell::Skin, name: &str) {
    let mut address = format!("ssh:{}", state.target.trim());
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(host) = &state.host {
        address = host.address.clone();
    }
    egui::Frame::NONE
        .fill(s.field)
        .stroke(Stroke::new(1.0, s.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(16.0, 22.0), Sense::hover());
                ui.painter().circle_filled(pos2(rect.left() + 4.0, rect.center().y), 4.0, s.success);
                ui.vertical(|ui| {
                    ui.label(RichText::new(name).font(semibold(13.0)).color(s.text));
                    ui.label(RichText::new(&address).font(FontId::monospace(10.5)).color(s.muted));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new("CONNECTED").font(FontId::monospace(9.5)).color(s.success));
                });
            });
        });
}

/// The modal's shared drawing pieces. The New workspace modal uses the same surface and controls.
pub(crate) mod shell {
    use super::*;

    /// The modal's colours, all from [`Tokens`].
    pub(crate) struct Skin {
        pub(crate) panel: Color32,
        pub(crate) text: Color32,
        pub(crate) muted: Color32,
        pub(crate) faint: Color32,
        pub(crate) border: Color32,
        pub(crate) border_strong: Color32,
        pub(crate) accent: Color32,
        pub(crate) accent_hover: Color32,
        pub(crate) hover: Color32,
        pub(crate) field: Color32,
        pub(crate) danger: Color32,
        pub(crate) success: Color32,
        pub(crate) shadow: Color32,
        pub(crate) focus: Color32,
    }

    impl Skin {
        pub(crate) fn of(t: &Tokens) -> Self {
            Self {
                panel: t.panel,
                text: t.text,
                muted: t.text_secondary,
                faint: t.text_tertiary,
                border: t.border,
                border_strong: t.border_strong,
                accent: t.accent,
                accent_hover: t.accent_hover,
                hover: t.hover,
                field: t.field,
                danger: t.danger,
                success: t.success,
                shadow: t.shadow,
                focus: t.focus,
            }
        }
    }

    /// The dimmed backdrop behind the modal.
    pub(crate) fn scrim(t: &Tokens) -> Color32 {
        Color32::from_black_alpha(if t.dark { 130 } else { 80 })
    }

    /// The white (or dark) modal surface: the theme's panel colour, rounded, lifted.
    pub(crate) fn surface(s: &Skin) -> egui::Frame {
        egui::Frame::NONE
            .fill(s.panel)
            .stroke(Stroke::new(1.0, s.border))
            .corner_radius(CornerRadius::same(14))
            .shadow(egui::epaint::Shadow { offset: [0, 18], blur: 48, spread: 0, color: s.shadow })
            .inner_margin(Margin::same(28))
    }

    /// The eyebrow line and the close button. True when the close button was clicked.
    pub(crate) fn header(ui: &mut Ui, s: &Skin, eyebrow: &str, note: &str) -> bool {
        let mut close = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(eyebrow).font(FontId::monospace(9.5)).color(s.accent));
            ui.add_space(10.0);
            ui.label(RichText::new(note).font(FontId::monospace(9.0)).color(s.faint));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let response = ui.add(egui::Button::new(RichText::new("×").font(regular(17.0)).color(s.muted)).frame(false));
                a11y::button(&response, "Close");
                if response.clicked() {
                    close = true;
                }
            });
        });
        close
    }

    pub(crate) fn title(ui: &mut Ui, s: &Skin, text: &str) {
        ui.add(egui::Label::new(RichText::new(text).font(semibold(27.0)).color(s.text)).wrap());
    }

    pub(crate) fn body(ui: &mut Ui, s: &Skin, text: &str) {
        ui.add_space(8.0);
        ui.add(egui::Label::new(RichText::new(text).font(regular(13.0)).color(s.muted)).wrap());
    }

    /// A bordered aside for the "never" promises and other quiet notes.
    pub(crate) fn note(ui: &mut Ui, s: &Skin, text: &str) {
        egui::Frame::NONE.fill(s.hover).corner_radius(CornerRadius::same(6)).inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).font(regular(11.5)).color(s.muted)).wrap());
        });
    }

    /// An error the person can read, not infer.
    pub(crate) fn error_note(ui: &mut Ui, s: &Skin, text: &str) {
        egui::Frame::NONE
            .fill(with_alpha(s.danger, 18))
            .stroke(Stroke::new(1.0, with_alpha(s.danger, 90)))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(RichText::new(text).font(regular(12.0)).color(s.danger)).wrap());
            });
    }

    /// A labelled single-line field; true when the text changed.
    pub(crate) fn field(ui: &mut Ui, s: &Skin, label: &str, hint: &str, value: &mut String) -> bool {
        ui.label(RichText::new(label).font(regular(11.0)).color(s.muted));
        ui.add_space(5.0);
        let response = ui.add(
            egui::TextEdit::singleline(value)
                .font(regular(13.0))
                .hint_text(RichText::new(hint).color(s.faint))
                .desired_width(ui.available_width())
                .frame(egui::Frame::NONE.fill(s.field).stroke(Stroke::new(1.0, s.border)).corner_radius(CornerRadius::same(6)))
                .margin(Margin::symmetric(10, 8)),
        );
        response.changed()
    }

    /// The footer's buttons, right aligned. Returns the index of the button clicked.
    pub(crate) fn footer(ui: &mut Ui, s: &Skin, buttons: &[(&str, bool, bool)]) -> Option<usize> {
        divider(ui, s);
        ui.add_space(16.0);
        let mut clicked = None;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for (index, (label, primary, enabled)) in buttons.iter().enumerate().rev() {
                if button(ui, s, label, *primary, *enabled) {
                    clicked = Some(index);
                }
                ui.add_space(8.0);
            }
        });
        clicked
    }

    fn button(ui: &mut Ui, s: &Skin, label: &str, primary: bool, enabled: bool) -> bool {
        let font = semibold(12.0);
        let width = text_width(ui, label, font.clone()) + 30.0;
        let (rect, response) = ui.allocate_exact_size(vec2(width, 32.0), if enabled { Sense::click() } else { Sense::hover() });
        a11y::button(&response, label);
        let hot = enabled && (response.hovered() || response.has_focus());
        if primary {
            let fill = if !enabled {
                s.hover
            } else if hot {
                s.accent_hover
            } else {
                s.accent
            };
            ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
        } else {
            if hot {
                ui.painter().rect_filled(rect, CornerRadius::same(6), s.hover);
            }
            let border = if !enabled {
                s.border
            } else if hot {
                s.accent
            } else {
                s.border_strong
            };
            ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, border), StrokeKind::Inside);
        }
        let ink = if !enabled {
            s.faint
        } else if primary {
            Color32::WHITE
        } else if hot {
            s.accent
        } else {
            s.text
        };
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font, ink);
        if response.has_focus() {
            ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(2.0, s.focus), StrokeKind::Outside);
        }
        response.clicked() && enabled
    }

    pub(crate) fn divider(ui: &mut Ui, s: &Skin) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter().line_segment([rect.left_center(), rect.right_center()], Stroke::new(1.0, s.border));
    }

    pub(crate) fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
        ui.painter().layout_no_wrap(text.to_string(), font, Color32::WHITE).size().x
    }

    pub(crate) fn with_alpha(color: Color32, alpha: u8) -> Color32 {
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
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
        assert!(state.target.is_empty());
        assert!(state.attention.is_none());
    }
}
