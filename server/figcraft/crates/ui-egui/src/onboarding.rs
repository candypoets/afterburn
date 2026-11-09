//! The first-run onboarding flow: Welcome, Herdr found, Setup check, No project yet.
//!
//! Shown in map mode when the app has never been set up. The design is `Onboarding section` on
//! the Afterburn page: a title bar with the wordmark, then one centred column per screen.
//!
//! Which screen comes first follows the design's rule: when Herdr already runs workspaces the
//! tour is skipped and "Herdr found" lists them; otherwise Welcome. The setup check asks the
//! node (`host.status`), starts an installed Herdr, and installs it only after the human's click
//! (`host.ensure` with `install_herdr`). The design's fourth screen, "First map", is the map
//! itself: opening a workspace ends the flow.

#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{self, Receiver};

use egui::{Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::a11y;
use crate::theme::{Tokens, regular, semibold, serif};
use crate::{FigApp, TitleBar};

/// The centred content column (the design's 640 pt at 980 pt wide).
const COLUMN: f32 = 640.0;
/// The No project sidebar's width.
const SIDEBAR: f32 = 280.0;
/// The title bar's height off macOS (on macOS it is the toolbar band).
const TITLEBAR: f32 = 44.0;
/// The agent CLIs the setup check names when they are missing.
const KNOWN_AGENTS: [&str; 3] = ["claude", "codex", "kimi"];
/// The config file recording that the flow was finished or skipped.
#[cfg(not(target_arch = "wasm32"))]
const DONE_FILE: &str = "onboarding.json";

/// The screens, in the order the flow visits them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Step {
    #[default]
    Welcome,
    SetupCheck,
    HerdrFound,
    NoProject,
}

impl Step {
    /// A stable id for tests and inspection.
    pub fn id(self) -> &'static str {
        match self {
            Step::Welcome => "welcome",
            Step::SetupCheck => "setupCheck",
            Step::HerdrFound => "herdrFound",
            Step::NoProject => "noProject",
        }
    }
}

/// The Afterburn node line of the setup check.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum NodeCheck {
    #[default]
    Checking,
    Running,
    Failed(String),
}

/// The Herdr line of the setup check: the design's six states, plus starting an installed Herdr.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum HerdrCheck {
    #[default]
    Checking,
    Ready,
    Missing,
    /// Installed but not running: the node starts it (no consent needed, nothing is installed).
    Starting,
    Installing,
    Skipped,
    Failed(String),
}

impl HerdrCheck {
    /// Continue is offered once Herdr runs, or once the human chose to go on without it.
    pub fn lets_continue(&self) -> bool {
        matches!(self, HerdrCheck::Ready | HerdrCheck::Skipped)
    }

    fn busy(&self) -> bool {
        matches!(self, HerdrCheck::Checking | HerdrCheck::Starting | HerdrCheck::Installing)
    }
}

/// Which node request an answer belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    /// `host.status`: is Herdr there and running?
    Status,
    /// `host.ensure` without consent: start the installed Herdr.
    Start,
    /// `host.ensure` with `install_herdr`: run its installer (after the human's click).
    Install,
}

/// What the node said about Herdr.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostStatus {
    pub installed: bool,
    pub running: bool,
}

/// The Herdr line after a node answer, and whether to ask the node to start Herdr next.
pub fn herdr_after(probe: Probe, answer: Result<HostStatus, String>, skipped: bool) -> (HerdrCheck, bool) {
    match answer {
        Err(error) => (HerdrCheck::Failed(error), false),
        Ok(HostStatus { installed: true, running: true }) => (HerdrCheck::Ready, false),
        Ok(HostStatus { installed: true, running: false }) if probe == Probe::Status => (HerdrCheck::Starting, true),
        Ok(HostStatus { installed: true, running: false }) => (HerdrCheck::Failed("Herdr is installed but didn't start".into()), false),
        Ok(HostStatus { installed: false, .. }) if probe == Probe::Install => {
            (HerdrCheck::Failed("the installer finished, but Herdr isn't there".into()), false)
        }
        Ok(HostStatus { installed: false, .. }) if skipped => (HerdrCheck::Skipped, false),
        Ok(HostStatus { installed: false, .. }) => (HerdrCheck::Missing, false),
    }
}

/// The onboarding flow's state.
#[derive(Default)]
pub struct State {
    /// The flow is showing instead of the Workspaces screen.
    pub active: bool,
    /// [`should_start`] was asked once, at the first frame.
    pub decided: bool,
    /// The screen the flow is on.
    pub step: Step,
    /// Where Back returns to, in the order the screens were shown.
    history: Vec<Step>,
    /// The workspace list was asked for while the flow showed.
    #[cfg(not(target_arch = "wasm32"))]
    asked: bool,
    /// The first screen was chosen from the workspace list (Herdr found or Welcome).
    placed: bool,
    pub node: NodeCheck,
    pub herdr: HerdrCheck,
    /// The human chose to go on without Herdr.
    skipped: bool,
    /// git answers `--version` on this machine (unknown until checked).
    pub git: Option<bool>,
    /// "Try the sample map" was clicked: say there is none bundled yet.
    sample_note: bool,
    #[cfg(not(target_arch = "wasm32"))]
    host_rx: Option<Receiver<(Probe, Result<HostStatus, String>)>>,
    #[cfg(not(target_arch = "wasm32"))]
    git_rx: Option<Receiver<bool>>,
}

impl State {
    /// Set [`active`](Self::active) from `should_start` once; later calls do nothing.
    pub fn decide(&mut self, start: bool) {
        if self.decided {
            return;
        }
        self.decided = true;
        self.active = start;
    }

    /// Choose the first screen once the list has answered: Herdr's workspaces skip the tour.
    pub fn place(&mut self, workspaces: usize) {
        if self.placed {
            return;
        }
        self.placed = true;
        if workspaces > 0 && self.step == Step::Welcome && self.history.is_empty() {
            self.step = Step::HerdrFound;
        }
    }

    /// Go to the next screen; the setup check leads to the workspaces, or to No project.
    pub fn advance(&mut self, workspaces: usize) {
        let next = match self.step {
            Step::Welcome => Step::SetupCheck,
            Step::SetupCheck if workspaces > 0 => Step::HerdrFound,
            Step::SetupCheck | Step::HerdrFound | Step::NoProject => Step::NoProject,
        };
        self.go(next);
    }

    /// Return to the screen shown before this one. False on the first screen.
    pub fn back(&mut self) -> bool {
        match self.history.pop() {
            Some(previous) => {
                self.step = previous;
                true
            }
            None => false,
        }
    }

    /// The flow is over (finished or skipped): the Workspaces screen shows next frame.
    pub fn finish(&mut self) {
        self.active = false;
    }

    /// Go on without Herdr; Install now stays offered on its line.
    pub fn skip_herdr(&mut self) {
        self.skipped = true;
        let waiting_on_node = self.herdr == HerdrCheck::Checking && matches!(self.node, NodeCheck::Failed(_));
        if waiting_on_node || matches!(self.herdr, HerdrCheck::Missing | HerdrCheck::Failed(_)) {
            self.herdr = HerdrCheck::Skipped;
        }
    }

    /// Skipping is offered while nothing is under way: Herdr missing or failed, or no node.
    fn can_skip(&self) -> bool {
        !self.herdr.lets_continue() && (!self.herdr.busy() || matches!(self.node, NodeCheck::Failed(_)))
    }

    /// Take in a node answer about Herdr; true when the node should now be asked to start it.
    pub fn on_host(&mut self, probe: Probe, answer: Result<HostStatus, String>) -> bool {
        self.node = match &answer {
            Ok(_) => NodeCheck::Running,
            Err(error) if probe == Probe::Status => NodeCheck::Failed(error.clone()),
            Err(_) => self.node.clone(),
        };
        if probe == Probe::Status && answer.is_err() {
            // No node, no Herdr check: the node line says why.
            self.herdr = HerdrCheck::Checking;
            return false;
        }
        let (herdr, start) = herdr_after(probe, answer, self.skipped);
        self.herdr = herdr;
        start
    }

    fn go(&mut self, next: Step) {
        if next != self.step {
            self.history.push(self.step);
            self.step = next;
        }
    }

    /// Ask the node about Herdr (or act on it) on a thread of its own.
    #[cfg(not(target_arch = "wasm32"))]
    fn ask_host(&mut self, ctx: &egui::Context, probe: Probe) {
        if self.host_rx.is_some() {
            return;
        }
        match probe {
            Probe::Status => {
                self.node = NodeCheck::Checking;
                self.herdr = HerdrCheck::Checking;
            }
            Probe::Start => self.herdr = HerdrCheck::Starting,
            Probe::Install => self.herdr = HerdrCheck::Installing,
        }
        let (tx, rx) = mpsc::channel();
        self.host_rx = Some(rx);
        let (context, address) = (ctx.clone(), crate::workspaces::service_addr());
        let spawned = std::thread::Builder::new().name("figcraft-onboarding-host".into()).spawn(move || {
            let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| host_request(&address, probe)))
                .unwrap_or_else(|_| Err("the check failed unexpectedly".to_string()));
            let _ = tx.send((probe, answer));
            context.request_repaint();
        });
        if spawned.is_err() {
            self.host_rx = None;
            self.on_host(probe, Err("couldn't start the check".into()));
        }
    }

    /// Look for git on this machine, once, on a thread of its own.
    #[cfg(not(target_arch = "wasm32"))]
    fn ask_git(&mut self, ctx: &egui::Context) {
        if self.git.is_some() || self.git_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.git_rx = Some(rx);
        let context = ctx.clone();
        let spawned = std::thread::Builder::new().name("figcraft-onboarding-git".into()).spawn(move || {
            let found = std::process::Command::new("git")
                .arg("--version")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success());
            let _ = tx.send(found);
            context.request_repaint();
        });
        if spawned.is_err() {
            self.git_rx = None;
            self.git = Some(false);
        }
    }

    /// Take in finished checks; a status answer may ask the node to start Herdr.
    #[cfg(not(target_arch = "wasm32"))]
    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(found) = self.git_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.git_rx = None;
            self.git = Some(found);
        }
        let answer = match self.host_rx.as_ref().map(Receiver::try_recv) {
            None | Some(Err(mpsc::TryRecvError::Empty)) => return,
            Some(Ok(answer)) => answer,
            Some(Err(mpsc::TryRecvError::Disconnected)) => (Probe::Status, Err("the check ended without an answer".to_string())),
        };
        self.host_rx = None;
        let (probe, answer) = answer;
        if self.on_host(probe, answer) {
            self.ask_host(ctx, Probe::Start);
        }
    }
}

/// One `host.status` or `host.ensure` exchange with the node.
#[cfg(not(target_arch = "wasm32"))]
fn host_request(address: &str, probe: Probe) -> Result<HostStatus, String> {
    let request = match probe {
        Probe::Status => serde_json::json!({"method": "host.status"}),
        Probe::Start => serde_json::json!({"method": "host.ensure", "install_herdr": false}),
        Probe::Install => serde_json::json!({"method": "host.ensure", "install_herdr": true}),
    };
    let timeout = if probe == Probe::Install { std::time::Duration::from_secs(300) } else { std::time::Duration::from_secs(20) };
    let reply = crate::node::rpc(address, request, timeout)?;
    if reply.get("ok").and_then(serde_json::Value::as_bool) == Some(false) {
        return Err(reply.get("error").and_then(serde_json::Value::as_str).unwrap_or("the node refused the request").to_string());
    }
    let flag = |key: &str| reply.get(key).and_then(serde_json::Value::as_bool).unwrap_or(false);
    Ok(HostStatus { installed: flag("installed"), running: flag("running") })
}

/// Whether the flow should start on this run: never completed, no saved machine and no live node
/// session. The browser has neither a node session nor a config folder, so it never starts there.
pub fn should_start(app: &FigApp) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = app;
        false
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let done = done_path().as_deref().is_some_and(is_complete);
        let saved_hosts = hosts_path().as_deref().is_some_and(has_saved_hosts);
        let live = app.session.live().status.active();
        first_run(done, saved_hosts, live)
    }
}

/// The three first-run conditions, separated from the file system so they are testable.
#[cfg(not(target_arch = "wasm32"))]
fn first_run(done: bool, saved_hosts: bool, live: bool) -> bool {
    !done && !saved_hosts && !live
}

// ---------------------------------------------------------------------------------------------
// Persistence: a small marker next to the saved hosts (`~/.config/afterburn/onboarding.json`).

#[cfg(not(target_arch = "wasm32"))]
fn config_path(file: &str) -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config").join("afterburn").join(file))
}

#[cfg(not(target_arch = "wasm32"))]
fn done_path() -> Option<PathBuf> {
    config_path(DONE_FILE)
}

/// The same file `crate::node_hosts` saves its machines in (its path helper is private).
#[cfg(not(target_arch = "wasm32"))]
fn hosts_path() -> Option<PathBuf> {
    config_path("hosts.json")
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(serde::Deserialize)]
struct Done {
    #[serde(default)]
    done: bool,
}

#[cfg(not(target_arch = "wasm32"))]
fn is_complete(path: &Path) -> bool {
    std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice::<Done>(&bytes).ok()).is_some_and(|done| done.done)
}

/// Whether the hosts file has at least one saved machine (an empty list is still a first run).
#[cfg(not(target_arch = "wasm32"))]
fn has_saved_hosts(path: &Path) -> bool {
    std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice::<Vec<serde_json::Value>>(&bytes).ok()).is_some_and(|hosts| !hosts.is_empty())
}

#[cfg(not(target_arch = "wasm32"))]
fn write_done(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Written aside and renamed over the old file, so a crash leaves either version whole.
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temp, br#"{"done":true}"#)?;
    std::fs::rename(temp, path)
}

#[cfg(not(target_arch = "wasm32"))]
fn mark_complete() -> Result<(), String> {
    let path = done_path().ok_or("the home folder is unavailable")?;
    write_done(&path).map_err(|error| format!("{}: {error}", path.display()))
}

/// Record the finished flow; a failure only means it is offered again next time.
fn record_finished() {
    #[cfg(not(target_arch = "wasm32"))]
    if let Err(error) = mark_complete() {
        log::warn!("onboarding: couldn't record the finished setup: {error}");
    }
}

// ---------------------------------------------------------------------------------------------
// Drawing.

/// The design's paper, inks and rules; dark mode keeps the tokens' surfaces.
struct Pal {
    bg: Color32,
    panel: Color32,
    border: Color32,
    text: Color32,
    muted: Color32,
    faint: Color32,
    accent: Color32,
    accent_hover: Color32,
    teal: Color32,
    teal_soft: Color32,
    warm: Color32,
    disabled: Color32,
    danger: Color32,
    hover: Color32,
}

impl Pal {
    fn of(t: &Tokens) -> Pal {
        if t.dark {
            let accent = Color32::from_rgb(0xD9, 0x7A, 0x5F);
            let teal = Color32::from_rgb(0x7F, 0xBD, 0xB5);
            Pal {
                bg: t.canvas,
                panel: t.panel,
                border: t.border,
                text: t.text,
                muted: t.text_secondary,
                faint: t.text_tertiary,
                accent,
                accent_hover: Color32::from_rgb(0xE8, 0x8E, 0x75),
                teal,
                teal_soft: with_alpha(teal, 30),
                warm: with_alpha(accent, 26),
                disabled: t.hover,
                danger: Color32::from_rgb(0xE8, 0x9A, 0x8A),
                hover: t.hover,
            }
        } else {
            let c = crate::map_colors::get();
            Pal {
                bg: c.canvas,
                panel: c.panel,
                border: c.border,
                text: c.ink,
                muted: c.muted,
                faint: c.faint,
                accent: c.accent,
                accent_hover: c.accent_hover,
                teal: c.map_ink,
                teal_soft: Color32::from_rgb(0xEE, 0xF4, 0xF3),
                warm: Color32::from_rgb(0xF1, 0xEC, 0xE7),
                disabled: Color32::from_rgb(0xE8, 0xE5, 0xE0),
                danger: Color32::from_rgb(0x9E, 0x3B, 0x2C),
                hover: Color32::from_rgb(0xF7, 0xF6, 0xF3),
            }
        }
    }
}

/// What the screen's controls asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Action {
    Advance,
    Back,
    CheckAgain,
    InstallHerdr,
    SkipHerdr,
    /// Open a workspace's map, or Init it with its agent, or select it for Init.
    OpenWorkspace {
        id: String,
        act: Open,
    },
    NewProject,
    AddMachine,
    Sample,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Open {
    /// Open its map.
    Map,
    /// Init it with the agent already there.
    Draw,
    /// Show it on the Workspaces screen, where an agent is picked for Init.
    Select,
}

/// Draw the flow. Returns true while it owns the screen.
pub fn show(app: &mut FigApp, ui: &mut Ui) -> bool {
    if !app.onboarding.active {
        return false;
    }
    let ctx = ui.ctx().clone();
    let p = Pal::of(&Tokens::get(&ctx));
    let screen = ui.max_rect();
    ui.painter().rect_filled(screen, CornerRadius::ZERO, p.bg);
    a11y::region(&mut app.ui.nav, ui, a11y::Region::Canvas, egui::accesskit::Role::Pane, "First-run setup");

    // The map's shortcut handling reserves Tab; the flow has no canvas, so hand Tab back to
    // egui: it moves through the screen's buttons.
    let focus_step = ui.input(|i| {
        i.key_pressed(egui::Key::Tab).then_some(if i.modifiers.shift { egui::FocusDirection::Previous } else { egui::FocusDirection::Next })
    });
    if let Some(direction) = focus_step {
        ui.ctx().memory_mut(|memory| memory.move_focus(direction));
    }
    let back_key = ui.input(|i| i.key_pressed(egui::Key::Escape));

    // The Workspaces screen's own list request decides the first screen and fills Herdr found.
    #[cfg(not(target_arch = "wasm32"))]
    {
        if !app.onboarding.asked {
            app.onboarding.asked = true;
            app.ws.refresh(&ctx);
            app.onboarding.ask_host(&ctx, Probe::Status);
            app.onboarding.ask_git(&ctx);
        }
        app.onboarding.poll(&ctx);
        if !app.onboarding.placed && app.ws.busy.is_none() {
            app.onboarding.place(app.ws.rows.len());
        }
    }
    #[cfg(target_arch = "wasm32")]
    app.onboarding.place(0);

    let bar = titlebar(app, ui, &p, screen);
    let body = Rect::from_min_max(pos2(screen.min.x, bar.max.y), screen.max);
    let mut action = None;
    if !app.onboarding.placed {
        // A moment while the node answers: nothing to choose yet.
        let at = pos2(body.center().x - COLUMN * 0.5, body.min.y + 52.0);
        ui.painter().text(at, Align2::LEFT_TOP, "Looking for Herdr on this Mac…", mono(9.0), p.faint);
    } else {
        let step = app.onboarding.step;
        action = match step {
            Step::NoProject => no_project(app, ui, &p, body),
            _ => {
                let column = COLUMN.min((body.width() - 32.0).max(240.0));
                let top = match step {
                    Step::Welcome => 52.0,
                    Step::HerdrFound => 40.0,
                    _ => 36.0,
                };
                let area = Rect::from_min_max(pos2(body.center().x - column * 0.5, body.min.y), pos2(body.center().x + column * 0.5, body.max.y));
                let mut out = None;
                ui.scope_builder(egui::UiBuilder::new().max_rect(area).layout(Layout::top_down(Align::Min)), |ui| {
                    egui::ScrollArea::vertical().id_salt(("onboarding", step.id())).auto_shrink([false, false]).show(ui, |ui| {
                        ui.set_width(column);
                        ui.spacing_mut().item_spacing = vec2(10.0, 0.0);
                        let origin = ui.cursor().min.y + top;
                        out = match step {
                            Step::Welcome => welcome(ui, &p, origin),
                            Step::SetupCheck => setup_check(app, ui, &p, origin),
                            _ => herdr_found(app, ui, &p, origin),
                        };
                        ui.add_space(32.0);
                    });
                });
                out
            }
        };
    }
    if action.is_none() && back_key && !app.onboarding.history.is_empty() {
        action = Some(Action::Back);
    }
    if let Some(action) = action {
        apply(app, &ctx, action);
    }
    true
}

fn apply(app: &mut FigApp, ctx: &egui::Context, action: Action) {
    match action {
        Action::Advance => {
            #[cfg(not(target_arch = "wasm32"))]
            if app.onboarding.step == Step::SetupCheck {
                // Herdr may have just started: list its workspaces again.
                app.ws.refresh(ctx);
            }
            let workspaces = app.ws.rows.len();
            app.onboarding.advance(workspaces);
        }
        Action::Back => {
            app.onboarding.back();
        }
        Action::CheckAgain => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                app.ws.refresh(ctx);
                app.onboarding.ask_host(ctx, Probe::Status);
            }
        }
        Action::InstallHerdr => {
            #[cfg(not(target_arch = "wasm32"))]
            app.onboarding.ask_host(ctx, Probe::Install);
        }
        Action::SkipHerdr => app.onboarding.skip_herdr(),
        Action::OpenWorkspace { id, act } => {
            end(app);
            match act {
                Open::Map => app.ws.act(ctx, &id, false),
                Open::Draw => app.ws.act(ctx, &id, true),
                Open::Select => app.ws.select(&id),
            }
        }
        Action::NewProject => {
            end(app);
            crate::new_workspace::open(app);
        }
        Action::AddMachine => {
            end(app);
            crate::machine_modal::open(app);
        }
        Action::Sample => app.onboarding.sample_note = true,
    }
    #[cfg(target_arch = "wasm32")]
    let _ = ctx;
}

/// The flow hands over to the Workspaces screen (and its modals, or the map it opens).
fn end(app: &mut FigApp) {
    app.onboarding.finish();
    record_finished();
}

/// The white title bar with the wordmark; on macOS it is the toolbar band the window buttons
/// sit in, and empty space moves the window.
fn titlebar(app: &FigApp, ui: &mut Ui, p: &Pal, screen: Rect) -> Rect {
    let inset = app.ui.mac_inset();
    let height = if inset { crate::MAC_BAND } else { TITLEBAR };
    let bar = Rect::from_min_size(screen.min, vec2(screen.width(), height));
    ui.painter().rect_filled(bar, CornerRadius::ZERO, p.panel);
    ui.painter().line_segment([bar.left_bottom() - vec2(0.0, 0.5), bar.right_bottom() - vec2(0.0, 0.5)], Stroke::new(1.0, p.border));
    if app.ui.title_bar != TitleBar::Native {
        crate::titlebar_drag(ui, bar);
    }
    let x = bar.min.x + if inset { crate::MAC_BUTTONS_W } else { 16.0 };
    ui.painter().text(pos2(x, bar.center().y), Align2::LEFT_CENTER, "Afterburn", semibold(13.0), p.text);
    bar
}

/// Move the flow's cursor down to `origin + y` (the design's position for the next element).
fn at(ui: &mut Ui, origin: f32, y: f32) {
    let gap = origin + y - ui.cursor().min.y;
    if gap > 0.0 {
        ui.add_space(gap);
    }
}

fn welcome(ui: &mut Ui, p: &Pal, origin: f32) -> Option<Action> {
    at(ui, origin, 0.0);
    eyebrow(ui, p, "WELCOME TO AFTERBURN");
    at(ui, origin, 22.0);
    title(ui, p, "Edit the design. Agents write the code.", 34.0);
    at(ui, origin, 66.0);
    body(
        ui,
        p,
        "Afterburn is a code editor built on a live map of your project. You change the map and comment on it; agents change the code; Sync keeps the two in step.",
        14.0,
        21.0,
    );
    at(ui, origin, 122.0);
    tiles(
        ui,
        p,
        &[
            ("01", "The map", "A live design of your project, saved in the project folder."),
            ("02", "Agents", "Claude, Codex or Kimi read the code and do the work you ask for."),
            ("03", "Herdr", "Runs the agents in the background. You don't need to learn it."),
        ],
    );
    at(ui, origin, 250.0);
    let mut action = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if primary(ui, p, "Set up this Mac", true).clicked() {
            action = Some(Action::Advance);
        }
        if secondary(ui, p, "Connect a remote machine").clicked() {
            action = Some(Action::AddMachine);
        }
    });
    at(ui, origin, 293.0);
    note(ui, p, "About a minute  ·  nothing is installed without asking", p.faint);
    action
}

fn herdr_found(app: &FigApp, ui: &mut Ui, p: &Pal, origin: f32) -> Option<Action> {
    at(ui, origin, 0.0);
    eyebrow(ui, p, "WELCOME TO AFTERBURN  ·  HERDR FOUND ON THIS MAC");
    at(ui, origin, 27.0);
    title(ui, p, "Your Herdr workspaces are ready to open", 28.0);
    at(ui, origin, 62.0);
    let count = app.ws.rows.len();
    body(
        ui,
        p,
        &format!(
            "Afterburn found Herdr running with {count} workspace{}. Open one to see its map; the agents already running there stay where they are.",
            if count == 1 { "" } else { "s" }
        ),
        13.5,
        20.0,
    );
    at(ui, origin, 116.0);
    let opened = workspace_list(app, ui, p);
    ui.add_space(14.0);
    let mut action = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if secondary(ui, p, "Open another folder").clicked() {
            action = Some(Action::NewProject);
        }
        if secondary(ui, p, "Connect a remote machine").clicked() {
            action = Some(Action::AddMachine);
        }
    });
    ui.add_space(14.0);
    note(ui, p, "Nothing is installed or restarted  ·  the setup check runs only if something is missing", p.faint);
    opened.or(action)
}

fn setup_check(app: &FigApp, ui: &mut Ui, p: &Pal, origin: f32) -> Option<Action> {
    let mut action = None;
    if !app.onboarding.history.is_empty() && back_link(ui, p).clicked() {
        action = Some(Action::Back);
    }
    at(ui, origin, 0.0);
    eyebrow(ui, p, "STEP 1 OF 2  ·  THIS MAC");
    at(ui, origin, 25.0);
    title(ui, p, "Getting this Mac ready", 28.0);
    at(ui, origin, 66.0);
    if let Some(chosen) = checklist(app, ui, p) {
        action = Some(chosen);
    }
    ui.add_space(12.0);
    note(ui, p, "Install Herdr runs herdr.dev/install.sh in your home folder. You can skip it; agents can't run until it's there.", p.muted);
    ui.add_space(12.0);
    let ready = app.onboarding.herdr.lets_continue();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if primary(ui, p, "Continue", ready).clicked() && ready {
            action = Some(Action::Advance);
        }
        if !ready {
            ui.vertical(|ui| {
                ui.add_space(1.0);
                note(ui, p, "  Continue once Herdr is ready", p.faint);
                ui.add_space(4.0);
                if app.onboarding.can_skip() && quiet_link(ui, p, "  or skip it for now").clicked() {
                    action = Some(Action::SkipHerdr);
                }
            });
        }
    });
    action
}

/// The No project screen: the empty machine sidebar, then the three ways to get a project.
fn no_project(app: &mut FigApp, ui: &mut Ui, p: &Pal, body_rect: Rect) -> Option<Action> {
    let mut action = None;
    let side = Rect::from_min_size(body_rect.min, vec2(SIDEBAR.min(body_rect.width() * 0.4), body_rect.height()));
    ui.painter().rect_filled(side, CornerRadius::ZERO, p.panel);
    ui.painter().line_segment([side.right_top() - vec2(0.5, 0.0), side.right_bottom() - vec2(0.5, 0.0)], Stroke::new(1.0, p.border));
    ui.painter().text(pos2(side.min.x + 17.0, side.min.y + 17.0), Align2::LEFT_TOP, "THIS MAC", mono(9.0), p.muted);
    ui.painter().text(pos2(side.min.x + 17.0, side.min.y + 37.0), Align2::LEFT_TOP, "No projects yet", regular(12.5), p.muted);
    let label = "+ New project";
    let button = Rect::from_min_size(pos2(side.min.x + 17.0, side.min.y + 60.0), vec2(text_width(ui, label, semibold(11.0)) + 30.0, 29.0));
    let resp = ui.interact(button, Id::new("onboarding_new_project"), Sense::click());
    paint_secondary(ui, p, button, &resp, label);
    if resp.clicked() {
        action = Some(Action::NewProject);
    }

    let content_x = side.max.x + 50.0;
    let width = (body_rect.max.x - content_x - 50.0).clamp(240.0, 600.0);
    let area = Rect::from_min_max(pos2(content_x, body_rect.min.y), pos2(content_x + width, body_rect.max.y));
    ui.scope_builder(egui::UiBuilder::new().max_rect(area).layout(Layout::top_down(Align::Min)), |ui| {
        egui::ScrollArea::vertical().id_salt("onboarding_no_project").auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing = vec2(10.0, 0.0);
            let origin = ui.cursor().min.y + 66.0;
            if !app.onboarding.history.is_empty() {
                at(ui, origin, -26.0);
                if back_link(ui, p).clicked() {
                    action = Some(Action::Back);
                }
            }
            at(ui, origin, 0.0);
            eyebrow(ui, p, "STEP 2 OF 2  ·  YOUR FIRST PROJECT");
            at(ui, origin, 27.0);
            title(ui, p, "Start with a project", 30.0);
            at(ui, origin, 69.0);
            body(
                ui,
                p,
                "A project is a folder of code. Afterburn keeps its map inside the folder, in .afterburn/, so it travels with the code and its branches.",
                13.5,
                20.0,
            );
            at(ui, origin, 117.0);
            let options = [
                (
                    "01",
                    "Open a folder on this Mac",
                    "Pick a repository. Afterburn creates the project and asks an agent to draw the first map.",
                    Action::NewProject,
                ),
                ("02", "Connect a machine", "Projects on a server reached with SSH, such as a build box.", Action::AddMachine),
                ("03", "Try the sample map", "A small example project, to look around before using your own code.", Action::Sample),
            ];
            let hovered = options.iter().position(|(n, ..)| ui.ctx().read_response(Id::new(("onboarding_option", *n))).is_some_and(|r| r.hovered()));
            for (index, (number, name, detail, chosen)) in options.into_iter().enumerate() {
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 62.0), Sense::hover());
                let resp = ui.interact(rect, Id::new(("onboarding_option", number)), Sense::click());
                a11y::button(&resp, name);
                // The first way in is the suggested one until another is pointed at.
                let lit = hovered.map_or(index == 0, |h| h == index);
                option(ui, p, rect, &resp, lit, number, name, detail);
                if resp.clicked() {
                    action = Some(chosen);
                }
                ui.add_space(10.0);
            }
            if app.onboarding.sample_note {
                note(ui, p, "The sample map isn't bundled with this build yet. Open a folder or connect a machine instead.", p.muted);
            }
        });
    });
    action
}

// ---------------------------------------------------------------------------------------------
// The setup checklist.

/// The four requirements, each a row with its state mark and at most one action.
fn checklist(app: &FigApp, ui: &mut Ui, p: &Pal) -> Option<Action> {
    let o = &app.onboarding;
    let address = crate::workspaces::service_addr();
    let node = match &o.node {
        NodeCheck::Checking => Item::new(Mark::Pending, "Afterburn node", "Checking…".into()),
        NodeCheck::Running => Item::new(Mark::Ok, "Afterburn node", format!("Running on this Mac · {address}")),
        NodeCheck::Failed(error) => Item {
            danger: true,
            button: Some(("Retry", Action::CheckAgain, false)),
            ..Item::new(Mark::Failed, "Afterburn node", format!("Not reachable: {error}"))
        },
    };
    let herdr_title = "Herdr, which runs your agents";
    let herdr = match &o.herdr {
        HerdrCheck::Checking if matches!(o.node, NodeCheck::Failed(_)) => Item::new(Mark::Pending, herdr_title, "Waits for the node.".into()),
        HerdrCheck::Checking => Item::new(Mark::Pending, herdr_title, "Checking…".into()),
        HerdrCheck::Ready => Item::new(Mark::Ok, herdr_title, "Running · your agents can start".into()),
        HerdrCheck::Missing => Item {
            attention: true,
            button: Some(("Install Herdr", Action::InstallHerdr, true)),
            ..Item::new(Mark::Missing, herdr_title, "Not installed. Afterburn installs it for you, as you, with no admin rights.".into())
        },
        HerdrCheck::Starting => Item::new(Mark::Pending, herdr_title, "Installed · starting it…".into()),
        HerdrCheck::Installing => Item::new(Mark::Pending, herdr_title, "Installing… about 20 seconds".into()),
        HerdrCheck::Skipped => Item {
            button: Some(("Install now", Action::InstallHerdr, false)),
            ..Item::new(Mark::Skipped, herdr_title, "Skipped · agents can't run until Herdr is installed".into())
        },
        HerdrCheck::Failed(error) => Item {
            danger: true,
            button: Some(("Retry", Action::InstallHerdr, false)),
            ..Item::new(Mark::Failed, herdr_title, format!("Couldn't install: {error}"))
        },
    };
    let agents = agents_item(app);
    let git = match o.git {
        None => Item::new(Mark::Pending, "git", "Checking…".into()),
        Some(true) => Item::new(Mark::Ok, "git", "Found. Maps are saved and synced through git.".into()),
        Some(false) => Item {
            attention: true,
            button: Some(("Check again", Action::CheckAgain, false)),
            ..Item::new(Mark::Missing, "git", "Not found. Install git (xcode-select --install), then check again.".into())
        },
    };
    let items = [node, herdr, agents, git];
    a11y::container(ui, egui::accesskit::Role::Group, "Setup check");
    let line = 60.0;
    let height = line * items.len() as f32 + (items.len() as f32 - 1.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height + 2.0), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(rect, CornerRadius::same(10), p.panel);
    let mut action = None;
    for (index, item) in items.iter().enumerate() {
        let row = Rect::from_min_size(pos2(rect.min.x + 1.0, rect.min.y + 1.0 + index as f32 * (line + 1.0)), vec2(rect.width() - 2.0, line));
        if index > 0 {
            painter.line_segment([row.left_top() - vec2(0.0, 0.5), row.right_top() - vec2(0.0, 0.5)], Stroke::new(1.0, p.border));
        }
        if item.attention {
            painter.rect_filled(row, CornerRadius::ZERO, p.warm);
        }
        status_mark(&painter, p, pos2(row.min.x + 27.0, row.center().y), 11.0, item.mark, ui.input(|i| i.time));
        painter.text(pos2(row.min.x + 50.0, row.min.y + 14.0), Align2::LEFT_TOP, item.title, semibold(12.5), p.text);
        let mut room = row.width() - 50.0 - 24.0;
        if let Some((label, chosen, is_primary)) = &item.button {
            let size = vec2(text_width(ui, label, semibold(11.0)) + if *is_primary { 28.0 } else { 30.0 }, if *is_primary { 27.0 } else { 29.0 });
            let button = Rect::from_min_size(pos2(row.max.x - 41.0 - size.x, row.center().y - size.y * 0.5), size);
            room = button.min.x - 16.0 - (row.min.x + 50.0);
            let resp = ui.interact(button, Id::new(("onboarding_check", index, *label)), Sense::click());
            if *is_primary {
                paint_primary(ui, p, button, &resp, label, true);
            } else {
                paint_secondary(ui, p, button, &resp, label);
            }
            if resp.clicked() {
                action = Some(chosen.clone());
            }
        }
        let galley = painter.layout(item.detail.clone(), regular(11.0), if item.danger { p.danger } else { p.muted }, room.max(60.0));
        painter.galley(pos2(row.min.x + 50.0, row.min.y + 31.0), galley, p.muted);
    }
    painter.rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, p.border), StrokeKind::Inside);
    if items.iter().any(|item| item.mark == Mark::Pending) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }
    action
}

/// The agent CLIs the list found, and the known ones it didn't.
fn agents_item(app: &FigApp) -> Item {
    let found: Vec<&str> = app.ws.clis.iter().map(|kind| kind.trim()).filter(|kind| !kind.is_empty()).collect();
    if found.is_empty() && matches!(app.onboarding.node, NodeCheck::Failed(_)) {
        return Item::new(Mark::Pending, "AI agents", "Waits for the node.".into());
    }
    if app.ws.busy.is_some() && found.is_empty() {
        return Item::new(Mark::Pending, "AI agents", "Checking…".into());
    }
    let missing: Vec<&str> = KNOWN_AGENTS.iter().copied().filter(|kind| !found.iter().any(|f| f.eq_ignore_ascii_case(kind))).collect();
    if found.is_empty() {
        return Item {
            attention: true,
            button: Some(("Check again", Action::CheckAgain, false)),
            ..Item::new(Mark::Missing, "AI agents", "None found. Install claude, codex or kimi; Afterburn never installs agents.".into())
        };
    }
    let mut detail = format!("Found {}.", and_list(&found));
    if !missing.is_empty() {
        detail.push_str(&format!(" {} {} installed.", or_list(&missing), if missing.len() == 1 { "isn't" } else { "aren't" }));
    }
    Item { button: Some(("Add another", Action::CheckAgain, false)), ..Item::new(Mark::Ok, "AI agents", detail) }
}

/// `claude`, `claude and kimi`, `claude, codex and kimi`.
fn and_list(items: &[&str]) -> String {
    join_words(items, "and")
}

fn or_list(items: &[&str]) -> String {
    join_words(items, "or")
}

fn join_words(items: &[&str], word: &str) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} {word} {last}", rest.join(", ")),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
    Ok,
    Pending,
    Missing,
    Skipped,
    Failed,
}

struct Item {
    mark: Mark,
    title: &'static str,
    detail: String,
    /// Label, action, and whether it is the accent button.
    button: Option<(&'static str, Action, bool)>,
    /// The warm row of the one thing that needs the human.
    attention: bool,
    danger: bool,
}

impl Item {
    fn new(mark: Mark, title: &'static str, detail: String) -> Item {
        Item { mark, title, detail, button: None, attention: false, danger: false }
    }
}

/// The round state mark: a tick, a spinner, `!`, `–` or `×`, drawn rather than set in a font.
fn status_mark(painter: &egui::Painter, p: &Pal, c: egui::Pos2, r: f32, mark: Mark, time: f64) {
    match mark {
        Mark::Ok => {
            painter.circle_filled(c, r, p.teal_soft);
            let s = Stroke::new(1.5, p.teal);
            painter.line_segment([pos2(c.x - 3.5, c.y + 0.2), pos2(c.x - 1.0, c.y + 2.8)], s);
            painter.line_segment([pos2(c.x - 1.0, c.y + 2.8), pos2(c.x + 3.8, c.y - 2.8)], s);
        }
        Mark::Pending => {
            painter.circle_filled(c, r, p.panel);
            painter.circle_stroke(c, r - 0.5, Stroke::new(1.0, p.faint));
            // A short arc turning: the check is under way.
            let start = (time * 4.0) as f32;
            let points: Vec<egui::Pos2> =
                (0..=10).map(|i| start + i as f32 * 0.16).map(|a| pos2(c.x + (r - 5.0) * a.cos(), c.y + (r - 5.0) * a.sin())).collect();
            painter.add(egui::Shape::line(points, Stroke::new(1.4, p.faint)));
        }
        Mark::Missing => {
            painter.circle_filled(c, r, p.panel);
            painter.circle_stroke(c, r - 0.5, Stroke::new(1.0, p.accent));
            painter.line_segment([pos2(c.x, c.y - 4.5), pos2(c.x, c.y + 1.2)], Stroke::new(1.5, p.accent));
            painter.circle_filled(pos2(c.x, c.y + 4.0), 0.9, p.accent);
        }
        Mark::Skipped => {
            painter.circle_filled(c, r, p.panel);
            painter.circle_stroke(c, r - 0.5, Stroke::new(1.0, p.muted));
            painter.line_segment([pos2(c.x - 3.0, c.y), pos2(c.x + 3.0, c.y)], Stroke::new(1.3, p.muted));
        }
        Mark::Failed => {
            painter.circle_filled(c, r, p.panel);
            painter.circle_stroke(c, r - 0.5, Stroke::new(1.0, p.danger));
            let s = Stroke::new(1.4, p.danger);
            painter.line_segment([pos2(c.x - 3.0, c.y - 3.0), pos2(c.x + 3.0, c.y + 3.0)], s);
            painter.line_segment([pos2(c.x - 3.0, c.y + 3.0), pos2(c.x + 3.0, c.y - 3.0)], s);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Tiles, the workspace list, the options and buttons.

/// The Welcome screen's three numbered tiles, each as tall as its text.
fn tiles(ui: &mut Ui, p: &Pal, entries: &[(&str, &str, &str)]) {
    if entries.is_empty() {
        return;
    }
    let gap = 12.0;
    let width = ui.available_width();
    let across = width >= 520.0;
    let tile_w = if across { ((width - gap * (entries.len() as f32 - 1.0)) / entries.len() as f32).max(120.0) } else { width };
    let body_w = tile_w - 28.0;
    let heights: Vec<f32> =
        entries.iter().map(|(_, _, text)| 54.0 + ui.painter().layout(text.to_string(), regular(11.0), p.muted, body_w).size().y + 15.0).collect();
    let draw = |ui: &mut Ui, (number, name, text): &(&str, &str, &str), height: f32| {
        let (rect, _) = ui.allocate_exact_size(vec2(tile_w, height), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(10), p.panel);
        painter.rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, p.border), StrokeKind::Inside);
        painter.text(pos2(rect.min.x + 15.0, rect.min.y + 15.0), Align2::LEFT_TOP, *number, mono(9.0), p.accent);
        painter.text(pos2(rect.min.x + 15.0, rect.min.y + 33.0), Align2::LEFT_TOP, *name, semibold(13.0), p.text);
        let galley = painter.layout(text.to_string(), regular(11.0), p.muted, body_w);
        painter.galley(pos2(rect.min.x + 15.0, rect.min.y + 54.0), galley, p.muted);
    };
    if across {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (entry, height) in entries.iter().zip(&heights) {
                draw(ui, entry, *height);
            }
        });
    } else {
        for (entry, height) in entries.iter().zip(&heights) {
            draw(ui, entry, *height);
            ui.add_space(gap);
        }
    }
}

/// Herdr's workspaces in one white list: a state dot, name and details, the map badge, and the
/// one way in (Open a map; Open and draw a map where an agent can draw it; Open otherwise).
fn workspace_list(app: &FigApp, ui: &mut Ui, p: &Pal) -> Option<Action> {
    let rows = &app.ws.rows;
    if rows.is_empty() {
        return None;
    }
    let line = 53.0;
    let height = rows.len() as f32 * (line + 1.0) + 1.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(rect, CornerRadius::same(10), p.panel);
    let mut action = None;
    // One suggested way in: the first workspace with a map gets the accent button.
    let suggested = rows.iter().position(|w| w.map == "map");
    for (index, workspace) in rows.iter().enumerate() {
        let row = Rect::from_min_size(pos2(rect.min.x + 1.0, rect.min.y + 1.0 + index as f32 * (line + 1.0)), vec2(rect.width() - 2.0, line));
        if index > 0 {
            painter.line_segment([row.left_top() - vec2(0.0, 0.5), row.right_top() - vec2(0.0, 0.5)], Stroke::new(1.0, p.border));
        }
        let has_map = workspace.map == "map";
        let dot = if workspace.agents == 0 {
            p.border
        } else if workspace.agents_detail.iter().any(|a| is_working(&a.status)) || is_working(&workspace.agent_status) {
            p.teal
        } else {
            p.faint
        };
        painter.circle_filled(pos2(row.min.x + 21.0, row.min.y + 26.0), 5.0, dot);

        let (badge_text, badge_ink, badge_fill) = if has_map { ("Has a map", p.teal, p.teal_soft) } else { ("No map yet", p.accent, p.warm) };
        let badge_w = text_width(ui, badge_text, mono(8.5)) + 16.0;
        let badge = Rect::from_min_size(pos2(row.min.x + 350.0, row.min.y + 18.0), vec2(badge_w, 17.0));
        painter.rect_filled(badge, CornerRadius::same(5), badge_fill);
        painter.text(badge.center(), Align2::CENTER_CENTER, badge_text, mono(8.5), badge_ink);

        let (label, act, is_primary) = if has_map {
            ("Open", Open::Map, suggested == Some(index))
        } else if workspace.agents > 0 {
            ("Open and draw a map", Open::Draw, false)
        } else {
            ("Open", Open::Select, false)
        };
        let size = vec2(text_width(ui, label, semibold(11.0)) + if is_primary { 28.0 } else { 30.0 }, if is_primary { 27.0 } else { 29.0 });
        let button = Rect::from_min_size(pos2(badge.max.x + 12.0, row.center().y - size.y * 0.5), size);
        let resp = ui.interact(button, Id::new(("onboarding_workspace", index, &workspace.id)), Sense::click());
        if is_primary {
            paint_primary(ui, p, button, &resp, label, true);
        } else {
            paint_secondary(ui, p, button, &resp, label);
        }
        if resp.clicked() {
            action = Some(Action::OpenWorkspace { id: workspace.id.clone(), act });
        }

        let name = if workspace.label.trim().is_empty() { workspace.id.as_str() } else { workspace.label.as_str() };
        let room = (badge.min.x - 12.0 - (row.min.x + 38.0)).max(40.0);
        let shown = crate::widgets::elide_font(ui, name, room, semibold(13.0));
        painter.text(pos2(row.min.x + 38.0, row.min.y + 12.0), Align2::LEFT_TOP, shown, semibold(13.0), p.text);
        let details = crate::widgets::elide_font(ui, &workspace_details(workspace), room, mono(9.0));
        painter.text(pos2(row.min.x + 38.0, row.min.y + 29.0), Align2::LEFT_TOP, details, mono(9.0), p.muted);
    }
    painter.rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, p.border), StrokeKind::Inside);
    action
}

/// `/root/code/mindmap · main  ·  2 agents working`
fn workspace_details(workspace: &crate::workspaces::Workspace) -> String {
    let path = workspace.path.clone().unwrap_or_else(|| "no folder".into());
    let home = std::env::var("HOME").ok().filter(|home| home.len() > 1);
    let path = match home.as_deref().and_then(|home| path.strip_prefix(home)) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("~{rest}"),
        _ => path,
    };
    let place = match workspace.branch.as_deref().filter(|branch| !branch.trim().is_empty()) {
        Some(branch) => format!("{path} · {branch}"),
        None => path,
    };
    let agents = match workspace.agents {
        0 => "no agents".to_string(),
        n => {
            let state = workspace.agent_status.trim();
            let noun = if n == 1 { "agent" } else { "agents" };
            if state.is_empty() || state.eq_ignore_ascii_case("unknown") { format!("{n} {noun}") } else { format!("{n} {noun} {state}") }
        }
    };
    format!("{place}  ·  {agents}")
}

fn is_working(status: &str) -> bool {
    matches!(status.trim().to_ascii_lowercase().as_str(), "working" | "busy" | "running" | "thinking")
}

#[allow(clippy::too_many_arguments)]
fn option(ui: &Ui, p: &Pal, rect: Rect, resp: &Response, lit: bool, number: &str, name: &str, detail: &str) {
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(10), p.panel);
    painter.rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, if lit { p.accent } else { p.border }), StrokeKind::Inside);
    if resp.has_focus() {
        painter.rect_stroke(rect, CornerRadius::same(10), Stroke::new(2.0, p.accent), StrokeKind::Outside);
    }
    painter.text(pos2(rect.min.x + 17.0, rect.center().y), Align2::LEFT_CENTER, number, mono(10.0), p.accent);
    painter.text(pos2(rect.min.x + 43.0, rect.min.y + 15.0), Align2::LEFT_TOP, name, semibold(13.0), p.text);
    let room = (rect.width() - 43.0 - 60.0).max(40.0);
    let detail = crate::widgets::elide_font(ui, detail, room, regular(11.0));
    painter.text(pos2(rect.min.x + 43.0, rect.min.y + 32.0), Align2::LEFT_TOP, detail, regular(11.0), p.muted);
    // The chevron, drawn.
    let c = pos2(rect.max.x - 39.0, rect.center().y);
    let s = Stroke::new(1.4, if lit { p.accent } else { p.faint });
    painter.line_segment([pos2(c.x - 2.5, c.y - 5.0), pos2(c.x + 2.5, c.y)], s);
    painter.line_segment([pos2(c.x + 2.5, c.y), pos2(c.x - 2.5, c.y + 5.0)], s);
}

fn eyebrow(ui: &mut Ui, p: &Pal, text: &str) {
    ui.label(egui::RichText::new(text).font(mono(10.0)).color(p.teal));
}

fn title(ui: &mut Ui, p: &Pal, text: &str, size: f32) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(serif(size)).color(p.text)).wrap());
    // The serif's line box is taller than the design's text box: keep a gap under it.
    ui.add_space(8.0);
}

/// Body copy with the design's line height.
fn body(ui: &mut Ui, p: &Pal, text: &str, size: f32, line: f32) {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_string(),
        egui::TextFormat { font_id: regular(size), color: p.muted, line_height: Some(line), ..Default::default() },
    );
    job.wrap.max_width = ui.available_width();
    ui.label(job);
}

fn note(ui: &mut Ui, _p: &Pal, text: &str, color: Color32) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(mono(9.0)).color(color)).wrap());
}

fn back_link(ui: &mut Ui, p: &Pal) -> Response {
    quiet_link(ui, p, "‹ Back")
}

fn quiet_link(ui: &mut Ui, p: &Pal, label: &str) -> Response {
    let font = mono(9.0);
    let width = text_width(ui, label, font.clone());
    let (rect, resp) = ui.allocate_exact_size(vec2(width + 2.0, 16.0), Sense::click());
    a11y::link(&resp, label.trim());
    let hot = resp.hovered() || resp.has_focus();
    let color = if hot { p.text } else { p.muted };
    ui.painter().text(pos2(rect.min.x, rect.center().y), Align2::LEFT_CENTER, label, font, color);
    if hot {
        let lead = text_width(ui, label.trim_end(), mono(9.0)) - text_width(ui, label.trim(), mono(9.0));
        ui.painter().line_segment([pos2(rect.min.x + lead, rect.max.y - 2.0), pos2(rect.min.x + width, rect.max.y - 2.0)], Stroke::new(1.0, color));
    }
    resp
}

/// The accent button (27 pt tall, 14 pt in); disabled, it is the design's grey.
fn primary(ui: &mut Ui, p: &Pal, label: &str, enabled: bool) -> Response {
    let size = vec2(text_width(ui, label, semibold(11.0)) + 28.0, 27.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    paint_primary(ui, p, rect, &resp, label, enabled);
    resp
}

fn paint_primary(ui: &Ui, p: &Pal, rect: Rect, resp: &Response, label: &str, enabled: bool) {
    a11y::button(resp, label);
    let fill = if !enabled {
        p.disabled
    } else if resp.hovered() {
        p.accent_hover
    } else {
        p.accent
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(7), fill);
    painter.text(rect.center(), Align2::CENTER_CENTER, label, semibold(11.0), if enabled { Color32::WHITE } else { p.faint });
    if resp.has_focus() {
        painter.rect_stroke(rect, CornerRadius::same(7), Stroke::new(2.0, p.accent), StrokeKind::Outside);
    }
}

/// The white outlined button (29 pt tall, 15 pt in).
fn secondary(ui: &mut Ui, p: &Pal, label: &str) -> Response {
    let size = vec2(text_width(ui, label, semibold(11.0)) + 30.0, 29.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    paint_secondary(ui, p, rect, &resp, label);
    resp
}

fn paint_secondary(ui: &Ui, p: &Pal, rect: Rect, resp: &Response, label: &str) {
    a11y::button(resp, label);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(7), if resp.hovered() { p.hover } else { p.panel });
    painter.rect_stroke(rect, CornerRadius::same(7), Stroke::new(1.0, p.border), StrokeKind::Inside);
    painter.text(rect.center(), Align2::CENTER_CENTER, label, semibold(11.0), p.text);
    if resp.has_focus() {
        painter.rect_stroke(rect, CornerRadius::same(7), Stroke::new(2.0, p.accent), StrokeKind::Outside);
    }
}

fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.painter().layout_no_wrap(text.to_string(), font, Color32::WHITE).size().x
}

fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

// ---------------------------------------------------------------------------------------------
// Tests.

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> Option<PathBuf> {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|time| time.as_nanos()).unwrap_or_default();
        let path = std::env::temp_dir().join(format!("afterburn-onboarding-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).ok().map(|_| path)
    }

    #[test]
    fn welcome_advances_to_the_setup_check() {
        let mut state = State { placed: true, ..State::default() };
        state.advance(0);
        assert_eq!(state.step, Step::SetupCheck);
    }

    #[test]
    fn herdr_workspaces_skip_the_tour() {
        let mut state = State::default();
        state.place(3);
        assert_eq!(state.step, Step::HerdrFound);
        assert!(!state.back(), "the tour was skipped: nothing to go back to");

        let mut state = State::default();
        state.place(0);
        assert_eq!(state.step, Step::Welcome, "no workspaces: the tour");
        state.place(3);
        assert_eq!(state.step, Step::Welcome, "the first screen is chosen once");
    }

    #[test]
    fn the_setup_check_continues_to_the_workspaces_or_to_no_project() {
        let mut state = State { step: Step::SetupCheck, ..State::default() };
        state.advance(2);
        assert_eq!(state.step, Step::HerdrFound);

        let mut state = State { step: Step::SetupCheck, ..State::default() };
        state.advance(0);
        assert_eq!(state.step, Step::NoProject);
    }

    #[test]
    fn back_returns_to_the_screen_that_was_shown() {
        let mut state = State::default();
        assert!(!state.back(), "nothing precedes the welcome");
        state.advance(0); // Setup check
        state.advance(2); // Herdr found
        state.advance(2); // No project
        assert!(state.back());
        assert_eq!(state.step, Step::HerdrFound);
        assert!(state.back());
        assert_eq!(state.step, Step::SetupCheck);
        assert!(state.back());
        assert_eq!(state.step, Step::Welcome);
        assert!(!state.back());
    }

    #[test]
    fn finishing_hides_the_flow() {
        let mut state = State { active: true, step: Step::NoProject, ..State::default() };
        state.finish();
        assert!(!state.active);
    }

    #[test]
    fn deciding_starts_the_flow_only_once() {
        let mut state = State::default();
        state.decide(true);
        assert!(state.active && state.decided);
        state.decide(false);
        assert!(state.active, "the first decision wins");
    }

    #[test]
    fn herdr_states_follow_the_node() {
        let ok = |installed, running| Ok(HostStatus { installed, running });
        assert_eq!(herdr_after(Probe::Status, ok(true, true), false), (HerdrCheck::Ready, false));
        assert_eq!(herdr_after(Probe::Status, ok(true, false), false), (HerdrCheck::Starting, true), "an installed Herdr is started");
        assert!(matches!(herdr_after(Probe::Start, ok(true, false), false).0, HerdrCheck::Failed(_)), "a second try is not looped");
        assert_eq!(herdr_after(Probe::Status, ok(false, false), false), (HerdrCheck::Missing, false));
        assert_eq!(herdr_after(Probe::Status, ok(false, false), true), (HerdrCheck::Skipped, false));
        assert!(matches!(herdr_after(Probe::Install, ok(false, false), false).0, HerdrCheck::Failed(_)));
        assert_eq!(herdr_after(Probe::Install, ok(true, true), false), (HerdrCheck::Ready, false));
        assert_eq!(herdr_after(Probe::Install, Err("no network".into()), false), (HerdrCheck::Failed("no network".into()), false));
    }

    #[test]
    fn a_node_failure_is_the_node_line_not_herdr() {
        let mut state = State::default();
        assert!(!state.on_host(Probe::Status, Err("refused".into())));
        assert_eq!(state.node, NodeCheck::Failed("refused".into()));
        assert_eq!(state.herdr, HerdrCheck::Checking);
        assert!(state.on_host(Probe::Status, Ok(HostStatus { installed: true, running: false })), "start the installed Herdr");
        assert_eq!(state.node, NodeCheck::Running);
    }

    #[test]
    fn continue_waits_for_herdr_or_a_skip() {
        let mut state = State { herdr: HerdrCheck::Missing, ..State::default() };
        assert!(!state.herdr.lets_continue());
        state.skip_herdr();
        assert_eq!(state.herdr, HerdrCheck::Skipped);
        assert!(state.herdr.lets_continue());
        // A later status keeps the choice.
        state.on_host(Probe::Status, Ok(HostStatus { installed: false, running: false }));
        assert_eq!(state.herdr, HerdrCheck::Skipped);
    }

    #[test]
    fn without_a_node_the_flow_can_still_go_on() {
        let mut state = State::default();
        assert!(!state.can_skip(), "the first check is under way");
        state.on_host(Probe::Status, Err("refused".into()));
        assert!(state.can_skip(), "no node: skip to connecting another machine");
        state.skip_herdr();
        assert!(state.herdr.lets_continue());
        state.step = Step::SetupCheck;
        state.advance(0);
        assert_eq!(state.step, Step::NoProject);
    }

    #[test]
    fn agent_lists_read_as_sentences() {
        assert_eq!(and_list(&["claude"]), "claude");
        assert_eq!(and_list(&["claude", "kimi"]), "claude and kimi");
        assert_eq!(or_list(&["claude", "codex", "kimi"]), "claude, codex or kimi");
        assert_eq!(and_list(&[]), "");
    }

    #[test]
    fn a_first_run_needs_no_marker_no_saved_host_and_no_live_session() {
        assert!(first_run(false, false, false));
        assert!(!first_run(true, false, false), "it was finished");
        assert!(!first_run(false, true, false), "a machine was saved");
        assert!(!first_run(false, false, true), "a node session is live");
    }

    #[test]
    fn the_done_marker_round_trips() {
        let Some(dir) = temp_dir("done") else { return };
        let path = dir.join("onboarding.json");
        assert!(!is_complete(&path), "no marker: a first run");
        assert!(write_done(&path).is_ok());
        assert!(is_complete(&path));
        assert!(std::fs::write(&path, b"not json").is_ok());
        assert!(!is_complete(&path), "a junk marker never counts as finished");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_a_non_empty_hosts_file_is_a_saved_machine() {
        let Some(dir) = temp_dir("hosts") else { return };
        let path = dir.join("hosts.json");
        assert!(!has_saved_hosts(&path), "no file: a first run");
        assert!(std::fs::write(&path, b"[]").is_ok());
        assert!(!has_saved_hosts(&path), "an empty list is still a first run");
        assert!(std::fs::write(&path, br#"[{"name":"Build box","address":"ssh:build","ssh":"build"}]"#).is_ok());
        assert!(has_saved_hosts(&path));
        assert!(std::fs::write(&path, b"not json").is_ok());
        assert!(!has_saved_hosts(&path), "junk never blocks the first run");
        let _ = std::fs::remove_dir_all(dir);
    }

    fn workspace(id: &str, path: &str, map: &str, agents: u32) -> crate::workspaces::Workspace {
        crate::workspaces::Workspace {
            id: id.into(),
            label: id.into(),
            path: Some(path.into()),
            map: map.into(),
            agents,
            branch: Some("main".into()),
            ..Default::default()
        }
    }

    #[test]
    fn workspace_details_name_place_branch_and_agents() {
        let mut w = workspace("acme", "/code/acme", "none", 1);
        w.agent_status = "idle".into();
        assert_eq!(workspace_details(&w), "/code/acme · main  ·  1 agent idle");
        assert_eq!(workspace_details(&workspace("x", "/code/x", "map", 0)), "/code/x · main  ·  no agents");
    }

    fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk(shape, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in shapes {
            walk(&clipped.shape, &mut out);
        }
        out
    }

    #[test]
    fn every_screen_draws_its_title_copy_and_actions() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, false, 0);
        let mut app = FigApp::new(figcraft_engine::Session::empty(), Box::new(crate::NoServices));
        app.onboarding.active = true;
        app.onboarding.decided = true;
        // Don't start node requests from a unit test: the state is what these screens draw.
        app.onboarding.asked = true;
        app.onboarding.placed = true;
        app.onboarding.node = NodeCheck::Running;
        app.onboarding.herdr = HerdrCheck::Missing;
        app.onboarding.git = Some(true);
        app.ws.clis = vec!["claude".into(), "kimi".into()];
        let mut time = 0.0_f64;
        let cases: [(Step, &[&str], bool); 4] = [
            (
                Step::Welcome,
                &[
                    "Afterburn",
                    "WELCOME TO AFTERBURN",
                    "Edit the design. Agents write the code.",
                    "The map",
                    "Agents",
                    "Herdr",
                    "Set up this Mac",
                    "Connect a remote machine",
                    "About a minute",
                ],
                false,
            ),
            (
                Step::SetupCheck,
                &[
                    "STEP 1 OF 2  ·  THIS MAC",
                    "Getting this Mac ready",
                    "Afterburn node",
                    "Running on this Mac",
                    "Herdr, which runs your agents",
                    "Install Herdr",
                    "AI agents",
                    "Found claude and kimi. codex isn't installed.",
                    "Add another",
                    "git",
                    "Continue",
                    "Continue once Herdr is ready",
                ],
                false,
            ),
            (
                Step::HerdrFound,
                &[
                    "HERDR FOUND ON THIS MAC",
                    "Your Herdr workspaces are ready to open",
                    "with 2 workspaces",
                    "acme",
                    "Has a map",
                    "web",
                    "No map yet",
                    "Open and draw a map",
                    "Open another folder",
                    "Connect a remote machine",
                ],
                true,
            ),
            (
                Step::NoProject,
                &[
                    "THIS MAC",
                    "No projects yet",
                    "+ New project",
                    "STEP 2 OF 2  ·  YOUR FIRST PROJECT",
                    "Start with a project",
                    "Open a folder on this Mac",
                    "Connect a machine",
                    "Try the sample map",
                ],
                false,
            ),
        ];
        for (step, expected, has_workspaces) in cases {
            app.onboarding.step = step;
            app.ws.rows =
                if has_workspaces { vec![workspace("acme", "/code/acme", "map", 2), workspace("web", "/code/web", "none", 1)] } else { Vec::new() };
            time += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                time: Some(time),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
                    assert!(show(&mut app, ui), "the flow keeps the screen");
                });
            });
            out.textures_delta.clear();
            let text = texts(&out.shapes);
            for &needle in expected {
                assert!(text.iter().any(|line| line.contains(needle)), "{step:?} doesn't show {needle:?} in {text:?}");
            }
        }
    }
}
