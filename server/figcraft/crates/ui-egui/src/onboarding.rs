//! The first-run onboarding flow: Welcome, Setup check, Herdr found, No project yet, First map.
//!
//! Shown in map mode when the app has never been set up; each step leads to the next, and the
//! last one opens the Workspaces screen. The design is `Onboarding section` on the Mindmap page.
//!
//! The node's workspace list is the screen's one source of truth: while the flow draws, it asks
//! the Workspaces screen's own list request once (no new node RPC), and the setup check reads the
//! same request's busy / error / rows state. The "Herdr found" step falls back to "No project
//! yet" when that list is empty, and the first map step is reached once a workspace or project
//! has been chosen.

#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use egui::{Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::FigApp;
use crate::a11y;
use crate::theme::{Tokens, regular, semibold};
use crate::widgets;

/// The width of the centred content column.
const COLUMN: f32 = 720.0;
/// The config file recording that the flow was finished or skipped.
#[cfg(not(target_arch = "wasm32"))]
const DONE_FILE: &str = "onboarding.json";

/// The steps, in the order the flow visits them.
///
/// The setup check comes before the workspace list: it is where the node and Herdr are checked,
/// then the list says whether there are workspaces (Herdr found) or not (No project yet).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Step {
    #[default]
    Welcome,
    SetupCheck,
    HerdrFound,
    NoProject,
    FirstMap,
}

impl Step {
    /// A stable id for tests and inspection.
    pub fn id(self) -> &'static str {
        match self {
            Step::Welcome => "welcome",
            Step::SetupCheck => "setupCheck",
            Step::HerdrFound => "herdrFound",
            Step::NoProject => "noProject",
            Step::FirstMap => "firstMap",
        }
    }
}

/// The onboarding flow's state.
#[derive(Default)]
pub struct State {
    /// The flow is showing instead of the Workspaces screen.
    pub active: bool,
    /// [`should_start`] was asked once, at the first frame.
    pub decided: bool,
    /// The step the flow is on.
    pub step: Step,
    /// Where Back returns to, in the order the steps were shown.
    history: Vec<Step>,
    /// The workspace list was asked for while the flow showed.
    asked: bool,
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

    /// The step drawn: an empty Herdr-found list is the no-project screen instead.
    pub fn visible(&self, workspaces: usize) -> Step {
        if self.step == Step::HerdrFound && workspaces == 0 { Step::NoProject } else { self.step }
    }

    /// Go to the next step; the setup check picks Herdr found or No project from the list.
    pub fn advance(&mut self, workspaces: usize) {
        let shown = self.visible(workspaces);
        let next = match shown {
            Step::Welcome => Step::SetupCheck,
            Step::SetupCheck if workspaces > 0 => Step::HerdrFound,
            Step::SetupCheck => Step::NoProject,
            Step::HerdrFound => Step::NoProject,
            Step::NoProject | Step::FirstMap => Step::FirstMap,
        };
        self.go(next, shown);
    }

    /// A workspace was picked from the Herdr list: show the first map step.
    pub fn open_first_map(&mut self, workspaces: usize) {
        let shown = self.visible(workspaces);
        self.go(Step::FirstMap, shown);
    }

    /// Return to the step shown before this one. False on the Welcome step.
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

    fn go(&mut self, next: Step, shown: Step) {
        if next != self.step {
            self.history.push(shown);
            self.step = next;
        }
    }
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
// Persistence: a small marker next to the saved hosts (`~/.config/mindmap/onboarding.json`).

#[cfg(not(target_arch = "wasm32"))]
fn config_path(file: &str) -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config").join("mindmap").join(file))
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

/// What the step's controls asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Action {
    Advance,
    Back,
    CheckAgain,
    OpenWorkspace(String),
    Finish,
}

/// Draw the flow. Returns true while it owns the screen.
pub fn show(app: &mut FigApp, ui: &mut Ui) -> bool {
    if !app.onboarding.active {
        return false;
    }
    let ctx = ui.ctx().clone();
    let t = Tokens::get(&ctx);
    let screen = ui.max_rect();
    ui.painter().rect_filled(screen, CornerRadius::ZERO, t.bg);
    a11y::region(&mut app.ui.nav, ui, a11y::Region::Canvas, egui::accesskit::Role::Pane, "First-run setup");

    // The map's shortcut handling reserves Tab; the flow has no canvas, so hand Tab back to
    // egui: it moves through the step's buttons and links.
    let focus_step = ui.input(|i| {
        i.key_pressed(egui::Key::Tab).then_some(if i.modifiers.shift { egui::FocusDirection::Previous } else { egui::FocusDirection::Next })
    });
    if let Some(direction) = focus_step {
        ui.ctx().memory_mut(|memory| memory.move_focus(direction));
    }

    // The Workspaces screen's own list request: the setup check and the Herdr list read it.
    // Asking once keeps the flow from sending a request on every frame.
    #[cfg(not(target_arch = "wasm32"))]
    if !app.onboarding.asked {
        app.onboarding.asked = true;
        app.ws.refresh(&ctx);
    }

    let visible = app.onboarding.visible(app.ws.rows.len());
    let column = COLUMN.min((screen.width() - 24.0).max(200.0));
    let top = (screen.height() * 0.10).clamp(24.0, 96.0);
    let area =
        Rect::from_min_size(pos2(screen.center().x - column * 0.5, screen.min.y + top), vec2(column, (screen.height() - top - 24.0).max(80.0)));
    let mut action = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(area).layout(Layout::top_down(Align::Min)), |ui| {
        egui::ScrollArea::vertical().id_salt(("onboarding", visible.id())).auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(column);
            ui.spacing_mut().item_spacing = vec2(10.0, 0.0);
            action = step(app, ui, &t, visible);
        });
    });
    if let Some(action) = action {
        apply(app, &ctx, action);
    }
    true
}

fn apply(app: &mut FigApp, ctx: &egui::Context, action: Action) {
    match action {
        Action::Advance => {
            let workspaces = app.ws.rows.len();
            app.onboarding.advance(workspaces);
        }
        Action::Back => {
            app.onboarding.back();
        }
        Action::CheckAgain => {
            #[cfg(not(target_arch = "wasm32"))]
            app.ws.refresh(ctx);
            #[cfg(target_arch = "wasm32")]
            let _ = ctx;
        }
        Action::OpenWorkspace(id) => {
            let workspaces = app.ws.rows.len();
            app.ws.select(&id);
            app.onboarding.open_first_map(workspaces);
        }
        Action::Finish => {
            app.onboarding.finish();
            record_finished();
        }
    }
}

fn step(app: &FigApp, ui: &mut Ui, t: &Tokens, step: Step) -> Option<Action> {
    // Back is offered while there is somewhere to go: not on Welcome, and not on the last screen.
    let mut back = None;
    if matches!(step, Step::SetupCheck | Step::HerdrFound | Step::NoProject) {
        if link_button(ui, t, "‹ Back", "Back").clicked() {
            back = Some(Action::Back);
        }
        ui.add_space(12.0);
    } else {
        ui.add_space(22.0);
    }
    let content = match step {
        Step::Welcome => welcome(ui, t),
        Step::SetupCheck => setup_check(app, ui, t),
        Step::HerdrFound => herdr_found(app, ui, t),
        Step::NoProject => no_project(ui, t),
        Step::FirstMap => first_map(ui, t),
    };
    content.or(back)
}

fn welcome(ui: &mut Ui, t: &Tokens) -> Option<Action> {
    eyebrow(ui, t, "WELCOME TO MINDMAP");
    ui.add_space(14.0);
    title(ui, t, "Edit the design. Agents write the code.");
    ui.add_space(14.0);
    paragraph(
        ui,
        "Mindmap is a code editor built on a live map of your project. You change the map and comment on it; agents change the code; Sync keeps the two in step.",
        14.5,
        t.text_secondary,
    );
    ui.add_space(24.0);
    cards(
        ui,
        t,
        &[
            ("01", "The map", "A live design of your project, saved in the project folder."),
            ("02", "Agents", "Claude, Codex or Kimi read the code and do the work you ask for."),
            ("03", "Herdr", "Runs the agents in the background. You don't need to learn it."),
        ],
    );
    ui.add_space(26.0);
    let mut action = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if primary_button(ui, t, "Set up this Mac").clicked() {
            action = Some(Action::Advance);
        }
        if secondary_button(ui, t, "Connect a remote machine").clicked() {
            action = Some(Action::Advance);
        }
    });
    ui.add_space(16.0);
    footnote(ui, t, "About a minute · nothing is installed without asking");
    action
}

fn setup_check(app: &FigApp, ui: &mut Ui, t: &Tokens) -> Option<Action> {
    eyebrow(ui, t, "SETUP CHECK");
    ui.add_space(14.0);
    title(ui, t, "One check before the agents run");
    ui.add_space(20.0);
    check_panel(app, ui, t);
    ui.add_space(22.0);
    let mut action = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if secondary_button(ui, t, "Check again").clicked() {
            action = Some(Action::CheckAgain);
        }
        if primary_button(ui, t, "Continue").clicked() {
            action = Some(Action::Advance);
        }
    });
    action
}

fn herdr_found(app: &FigApp, ui: &mut Ui, t: &Tokens) -> Option<Action> {
    if app.ws.rows.is_empty() {
        return no_project(ui, t);
    }
    eyebrow(ui, t, "WELCOME TO MINDMAP · HERDR FOUND ON THIS MAC");
    ui.add_space(14.0);
    title(ui, t, "Your Herdr workspaces are ready to open");
    ui.add_space(12.0);
    let count = app.ws.rows.len();
    paragraph(
        ui,
        &format!(
            "Mindmap found Herdr running with {count} workspace{}. Open one to see its map; the agents already running there stay where they are.",
            if count == 1 { "" } else { "s" }
        ),
        14.0,
        t.text_secondary,
    );
    ui.add_space(20.0);
    let opened = workspace_list(app, ui, t);
    ui.add_space(20.0);
    let mut action = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if secondary_button(ui, t, "Open another folder").clicked() {
            action = Some(Action::Advance);
        }
        if secondary_button(ui, t, "Connect a remote machine").clicked() {
            action = Some(Action::Advance);
        }
    });
    ui.add_space(14.0);
    footnote(ui, t, "Nothing is installed or restarted · the setup check runs only if something is missing");
    opened.or(action)
}

fn no_project(ui: &mut Ui, t: &Tokens) -> Option<Action> {
    eyebrow(ui, t, "NO PROJECT YET");
    ui.add_space(14.0);
    title(ui, t, "Point Mindmap at a project");
    ui.add_space(20.0);
    let mut action = None;
    let options = [
        ("01", "Open a folder on this Mac", "Pick a repository. Mindmap creates the project and asks an agent to draw the first map."),
        ("02", "Connect a machine", "Projects on a server reached with SSH, such as a build box."),
        ("03", "Try the sample map", "A small example project, to look around before using your own code."),
    ];
    for (number, name, detail) in options {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 66.0), Sense::click());
        a11y::button(&resp, name);
        option_row(ui, t, rect, &resp, number, name, detail);
        if resp.clicked() {
            action = Some(Action::Advance);
        }
        ui.add_space(10.0);
    }
    if link_button(ui, t, "Skip for now", "Skip for now").clicked() {
        action = Some(Action::Finish);
    }
    action
}

fn first_map(ui: &mut Ui, t: &Tokens) -> Option<Action> {
    eyebrow(ui, t, "FIRST MAP");
    ui.add_space(14.0);
    title(ui, t, "The map is yours now");
    ui.add_space(22.0);
    let tips = [
        ("01", "Comment on a card, then Send — the comment goes to an agent with the tree below it."),
        ("02", "Sync turns the design into the code; the map stays editable while it works."),
    ];
    for (number, tip) in tips {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 66.0), Sense::hover());
        let p = ui.painter().clone();
        p.rect_filled(rect, CornerRadius::same(10), t.canvas);
        p.rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, t.border), StrokeKind::Inside);
        let inner = rect.shrink2(vec2(18.0, 14.0));
        p.text(pos2(inner.min.x, inner.min.y + 6.0), Align2::LEFT_TOP, number, semibold(10.5), t.accent_text);
        let text = p.layout(tip.to_string(), regular(13.0), t.text, inner.width() - 34.0);
        p.galley(pos2(inner.min.x + 34.0, inner.min.y), text, t.text);
        ui.add_space(10.0);
    }
    ui.add_space(14.0);
    let mut action = None;
    if primary_button(ui, t, "Open the map").clicked() {
        action = Some(Action::Finish);
    }
    action
}

// ---------------------------------------------------------------------------------------------
// The setup check's three lines.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CheckState {
    Ok,
    Pending,
    Failed,
}

struct Check {
    state: CheckState,
    title: &'static str,
    detail: String,
}

fn check_panel(app: &FigApp, ui: &mut Ui, t: &Tokens) {
    let checks = checks(app);
    a11y::container(ui, egui::accesskit::Role::Group, "Setup check");
    let line = 60.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), line * checks.len() as f32), Sense::hover());
    let p = ui.painter().clone();
    p.rect_filled(rect, CornerRadius::same(10), t.canvas);
    p.rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, t.border), StrokeKind::Inside);
    for (index, check) in checks.iter().enumerate() {
        let row = Rect::from_min_size(pos2(rect.min.x, rect.min.y + index as f32 * line), vec2(rect.width(), line));
        if index > 0 {
            p.line_segment([row.left_top(), row.right_top()], Stroke::new(1.0, t.border));
        }
        let (mark, color) = match check.state {
            CheckState::Ok => ("✓", t.success),
            CheckState::Pending => ("…", t.accent_text),
            CheckState::Failed => ("!", t.danger),
        };
        let center = pos2(row.min.x + 34.0, row.center().y);
        p.circle_filled(center, 11.0, with_alpha(color, if t.dark { 40 } else { 18 }));
        p.text(center, Align2::CENTER_CENTER, mark, semibold(12.0), color);
        p.text(pos2(row.min.x + 56.0, row.center().y - 9.0), Align2::LEFT_CENTER, check.title, semibold(13.0), t.text);
        let room = (row.width() - 56.0 - 20.0).max(40.0);
        let detail = widgets::elide_font(ui, &check.detail, room, regular(11.5));
        p.text(pos2(row.min.x + 56.0, row.center().y + 10.0), Align2::LEFT_CENTER, detail, regular(11.5), t.text_secondary);
    }
}

fn checks(app: &FigApp) -> Vec<Check> {
    let busy = app.ws.busy.is_some();
    let error = app.ws.error.as_deref();
    let asked = app.onboarding.asked;
    let rows = app.ws.rows.len();
    let node = if busy {
        Check { state: CheckState::Pending, title: "Mindmap node", detail: "Checking the connection…".into() }
    } else if let Some(error) = error {
        Check { state: CheckState::Failed, title: "Mindmap node", detail: format!("Not reachable. {error}") }
    } else if asked {
        Check { state: CheckState::Ok, title: "Mindmap node", detail: format!("Running · {}", crate::workspaces::service_addr()) }
    } else {
        Check { state: CheckState::Pending, title: "Mindmap node", detail: "Not checked yet.".into() }
    };
    let herdr = if busy {
        Check { state: CheckState::Pending, title: "Herdr, which runs your agents", detail: "Looking for running workspaces…".into() }
    } else if error.is_some() {
        Check { state: CheckState::Failed, title: "Herdr, which runs your agents", detail: "Couldn't be reached yet.".into() }
    } else if !asked {
        Check { state: CheckState::Pending, title: "Herdr, which runs your agents", detail: "Waiting for the node.".into() }
    } else if rows > 0 {
        Check {
            state: CheckState::Ok,
            title: "Herdr, which runs your agents",
            detail: format!("Running with {rows} workspace{}.", if rows == 1 { "" } else { "s" }),
        }
    } else {
        Check { state: CheckState::Ok, title: "Herdr, which runs your agents", detail: "Running, with no workspaces yet.".into() }
    };
    vec![node, herdr, machine(app, busy)]
}

#[cfg(not(target_arch = "wasm32"))]
fn machine(app: &FigApp, busy: bool) -> Check {
    match app.ws.hosts.selected.as_ref() {
        Some(host) => {
            let name = if host.name.trim().is_empty() { host.address.as_str() } else { host.name.as_str() };
            Check {
                state: if busy { CheckState::Pending } else { CheckState::Ok },
                title: "Selected machine",
                detail: format!("{name} · {}", host.address),
            }
        }
        None => Check { state: CheckState::Ok, title: "This Mac", detail: format!("This Mac · {}", crate::workspaces::service_addr()) },
    }
}

#[cfg(target_arch = "wasm32")]
fn machine(_app: &FigApp, _busy: bool) -> Check {
    Check { state: CheckState::Pending, title: "Selected machine", detail: "The node runs in the desktop app.".into() }
}

// ---------------------------------------------------------------------------------------------
// Cards, rows and buttons.

fn cards(ui: &mut Ui, t: &Tokens, entries: &[(&str, &str, &str)]) {
    if entries.is_empty() {
        return;
    }
    let gap = 12.0;
    let width = ui.available_width();
    if width >= 540.0 {
        let card_w = ((width - gap * (entries.len() as f32 - 1.0)) / entries.len() as f32).max(100.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (number, name, body) in entries {
                let (rect, _) = ui.allocate_exact_size(vec2(card_w, 132.0), Sense::hover());
                card(ui, t, rect, number, name, body);
            }
        });
    } else {
        for (number, name, body) in entries {
            let (rect, _) = ui.allocate_exact_size(vec2(width, 104.0), Sense::hover());
            card(ui, t, rect, number, name, body);
            ui.add_space(10.0);
        }
    }
}

fn card(ui: &Ui, t: &Tokens, rect: Rect, number: &str, name: &str, body: &str) {
    let p = ui.painter().clone();
    p.rect_filled(rect, CornerRadius::same(10), t.canvas);
    p.rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, t.border), StrokeKind::Inside);
    let inner = rect.shrink2(vec2(16.0, 14.0));
    p.text(inner.left_top(), Align2::LEFT_TOP, number, semibold(10.5), t.accent_text);
    p.text(pos2(inner.min.x, inner.min.y + 18.0), Align2::LEFT_TOP, name, semibold(14.0), t.text);
    let text = p.layout(body.to_string(), regular(11.5), t.text_secondary, inner.width());
    p.galley(pos2(inner.min.x, inner.min.y + 44.0), text, t.text_secondary);
}

fn workspace_list(app: &FigApp, ui: &mut Ui, t: &Tokens) -> Option<Action> {
    if app.ws.rows.is_empty() {
        return None;
    }
    let line = 56.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), line * app.ws.rows.len() as f32), Sense::hover());
    let p = ui.painter().clone();
    p.rect_filled(rect, CornerRadius::same(10), t.canvas);
    p.rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, t.border), StrokeKind::Inside);
    let mut action = None;
    for (index, workspace) in app.ws.rows.iter().enumerate() {
        let row = Rect::from_min_size(pos2(rect.min.x, rect.min.y + index as f32 * line), vec2(rect.width(), line));
        let resp = ui.interact(row, ui.id().with(("onboarding_workspace", index, &workspace.id)), Sense::click());
        let name = workspace_name(workspace);
        a11y::button(&resp, &format!("Open {name}"));
        if resp.hovered() {
            p.rect_filled(row.shrink2(vec2(2.0, 3.0)), CornerRadius::same(7), t.hover);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if index > 0 {
            p.line_segment([row.left_top(), row.right_top()], Stroke::new(1.0, t.border));
        }
        if resp.has_focus() {
            p.rect_stroke(row.shrink2(vec2(2.0, 3.0)), CornerRadius::same(7), Stroke::new(2.0, t.focus), StrokeKind::Inside);
        }

        let badge_text = if workspace.map == "map" { "Has a map" } else { "No map yet" };
        let badge_font = semibold(10.0);
        let badge_w = text_width(ui, badge_text, badge_font.clone()) + 16.0;
        let badge = Rect::from_min_size(pos2(row.max.x - 16.0 - badge_w, row.center().y - 9.0), vec2(badge_w, 18.0));
        let (badge_ink, badge_fill) =
            if workspace.map == "map" { (t.accent_text, with_alpha(t.accent, if t.dark { 40 } else { 18 })) } else { (t.text_secondary, t.hover) };
        p.rect_filled(badge, CornerRadius::same(9), badge_fill);
        p.rect_stroke(badge, CornerRadius::same(9), Stroke::new(1.0, with_alpha(badge_ink, 80)), StrokeKind::Inside);
        p.text(badge.center(), Align2::CENTER_CENTER, badge_text, badge_font, badge_ink);

        let room = (badge.min.x - 12.0 - (row.min.x + 16.0)).max(40.0);
        let shown = widgets::elide_font(ui, name, room, semibold(13.5));
        p.text(pos2(row.min.x + 16.0, row.center().y - 9.0), Align2::LEFT_CENTER, shown, semibold(13.5), t.text);
        let path = workspace.path.as_deref().filter(|path| !path.trim().is_empty()).unwrap_or("no folder");
        let path = widgets::elide_start_font(ui, path, room, egui::FontId::monospace(10.5));
        p.text(pos2(row.min.x + 16.0, row.center().y + 10.0), Align2::LEFT_CENTER, path, egui::FontId::monospace(10.5), t.text_secondary);

        let resp = resp.on_hover_text(format!("Open {name}"));
        if resp.clicked() {
            action = Some(Action::OpenWorkspace(workspace.id.clone()));
        }
    }
    action
}

/// A workspace's label, or its id when it has no label (the field method is private).
fn workspace_name(workspace: &crate::workspaces::Workspace) -> &str {
    if workspace.label.trim().is_empty() { &workspace.id } else { &workspace.label }
}

fn option_row(ui: &Ui, t: &Tokens, rect: Rect, resp: &Response, number: &str, name: &str, detail: &str) {
    let p = ui.painter().clone();
    let fill = if resp.hovered() { t.hover } else { t.canvas };
    p.rect_filled(rect, CornerRadius::same(10), fill);
    p.rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, if resp.hovered() { t.accent } else { t.border }), StrokeKind::Inside);
    if resp.has_focus() {
        p.rect_stroke(rect, CornerRadius::same(10), Stroke::new(2.0, t.focus), StrokeKind::Outside);
    }
    p.text(pos2(rect.min.x + 18.0, rect.center().y - 9.0), Align2::LEFT_CENTER, number, semibold(10.5), t.accent_text);
    p.text(pos2(rect.min.x + 54.0, rect.center().y - 9.0), Align2::LEFT_CENTER, name, semibold(14.0), t.text);
    let room = (rect.width() - 54.0 - 40.0).max(40.0);
    let detail = widgets::elide_font(ui, detail, room, regular(11.5));
    p.text(pos2(rect.min.x + 54.0, rect.center().y + 11.0), Align2::LEFT_CENTER, detail, regular(11.5), t.text_secondary);
    let chevron = if resp.hovered() { t.accent_text } else { t.text_tertiary };
    p.text(pos2(rect.max.x - 18.0, rect.center().y), Align2::RIGHT_CENTER, "›", semibold(18.0), chevron);
}

fn eyebrow(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.label(egui::RichText::new(text).font(semibold(10.0)).color(t.accent_text));
}

fn title(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(semibold(30.0)).color(t.text)).wrap());
}

fn paragraph(ui: &mut Ui, text: &str, size: f32, color: Color32) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(regular(size)).color(color)).wrap());
}

fn footnote(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(regular(10.5)).color(t.text_secondary)).wrap());
}

fn primary_button(ui: &mut Ui, t: &Tokens, label: &str) -> Response {
    let font = semibold(12.5);
    let (rect, resp) = ui.allocate_exact_size(vec2(text_width(ui, label, font.clone()) + 32.0, 34.0), Sense::click());
    a11y::button(&resp, label);
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(7), if resp.hovered() { t.accent_hover } else { t.accent });
    p.text(rect.center(), Align2::CENTER_CENTER, label, font, Color32::WHITE);
    if resp.has_focus() {
        p.rect_stroke(rect, CornerRadius::same(7), Stroke::new(2.0, t.focus), StrokeKind::Outside);
    }
    resp
}

fn secondary_button(ui: &mut Ui, t: &Tokens, label: &str) -> Response {
    let font = semibold(12.5);
    let (rect, resp) = ui.allocate_exact_size(vec2(text_width(ui, label, font.clone()) + 28.0, 34.0), Sense::click());
    a11y::button(&resp, label);
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(7), t.hover);
    }
    p.rect_stroke(rect, CornerRadius::same(7), Stroke::new(1.0, if resp.hovered() { t.accent } else { t.border_strong }), StrokeKind::Inside);
    p.text(rect.center(), Align2::CENTER_CENTER, label, font, t.text);
    if resp.has_focus() {
        p.rect_stroke(rect, CornerRadius::same(7), Stroke::new(2.0, t.focus), StrokeKind::Outside);
    }
    resp
}

fn link_button(ui: &mut Ui, t: &Tokens, label: &str, name: &str) -> Response {
    let font = regular(11.5);
    let width = text_width(ui, label, font.clone());
    let (rect, resp) = ui.allocate_exact_size(vec2(width + 4.0, 20.0), Sense::click());
    a11y::link(&resp, name);
    let color = if resp.hovered() || resp.has_focus() { t.accent_text } else { t.text_secondary };
    let p = ui.painter();
    p.text(pos2(rect.min.x + 2.0, rect.center().y), Align2::LEFT_CENTER, label, font, color);
    if resp.hovered() || resp.has_focus() {
        p.line_segment([pos2(rect.min.x + 2.0, rect.bottom() - 2.0), pos2(rect.min.x + 2.0 + width, rect.bottom() - 2.0)], Stroke::new(1.0, color));
    }
    resp
}

fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.painter().layout_no_wrap(text.to_string(), font, Color32::WHITE).size().x
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

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|time| time.as_nanos()).unwrap_or_default();
        let path = std::env::temp_dir().join(format!("mindmap-onboarding-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).expect("the temporary folder");
        path
    }

    #[test]
    fn welcome_advances_to_the_setup_check() {
        let mut state = State::default();
        state.advance(0);
        assert_eq!(state.step, Step::SetupCheck);
        assert_eq!(state.visible(0), Step::SetupCheck);
    }

    #[test]
    fn the_setup_check_continues_to_the_workspaces_or_to_no_project() {
        let mut state = State { step: Step::SetupCheck, ..State::default() };
        state.advance(2);
        assert_eq!(state.visible(2), Step::HerdrFound);

        let mut state = State { step: Step::SetupCheck, ..State::default() };
        state.advance(0);
        assert_eq!(state.step, Step::NoProject);
    }

    #[test]
    fn an_empty_herdr_list_shows_no_project_and_skips_to_the_first_map() {
        let mut state = State { step: Step::HerdrFound, ..State::default() };
        assert_eq!(state.visible(0), Step::NoProject, "the no-project step shows instead");
        state.advance(0);
        assert_eq!(state.step, Step::FirstMap);
    }

    #[test]
    fn choosing_a_project_reaches_the_first_map() {
        let mut state = State { step: Step::NoProject, ..State::default() };
        state.advance(0);
        assert_eq!(state.step, Step::FirstMap);
        assert_eq!(state.visible(0), Step::FirstMap);
    }

    #[test]
    fn opening_a_listed_workspace_reaches_the_first_map() {
        let mut state = State { step: Step::HerdrFound, ..State::default() };
        state.open_first_map(3);
        assert_eq!(state.step, Step::FirstMap);
        assert!(state.back(), "Back returns to the workspace list");
        assert_eq!(state.step, Step::HerdrFound);
    }

    #[test]
    fn back_returns_to_the_step_that_was_shown() {
        let mut state = State::default();
        assert!(!state.back(), "nothing precedes the welcome");
        state.advance(0); // Setup check
        state.advance(2); // Herdr found
        state.advance(2); // No project (Open another folder)
        assert!(state.back());
        assert_eq!(state.step, Step::HerdrFound);
        assert!(state.back());
        assert_eq!(state.step, Step::SetupCheck);
        assert!(state.back());
        assert_eq!(state.step, Step::Welcome);
        assert!(!state.back());
    }

    #[test]
    fn the_empty_list_fallback_is_the_step_back_returns_to() {
        let mut state = State { step: Step::HerdrFound, ..State::default() };
        state.advance(0); // shows No project, then goes to First map
        assert_eq!(state.step, Step::FirstMap);
        assert!(state.back());
        assert_eq!(state.step, Step::NoProject, "Back returns to the step that was drawn");
    }

    #[test]
    fn finishing_hides_the_flow() {
        let mut state = State { active: true, step: Step::FirstMap, ..State::default() };
        state.finish();
        assert!(!state.active);
        assert_eq!(state.step, Step::FirstMap, "the last step stays for an inspect");
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
    fn a_first_run_needs_no_marker_no_saved_host_and_no_live_session() {
        assert!(first_run(false, false, false));
        assert!(!first_run(true, false, false), "it was finished");
        assert!(!first_run(false, true, false), "a machine was saved");
        assert!(!first_run(false, false, true), "a node session is live");
    }

    #[test]
    fn the_done_marker_round_trips() {
        let dir = temp_dir("done");
        let path = dir.join("onboarding.json");
        assert!(!is_complete(&path), "no marker: a first run");
        write_done(&path).expect("write the marker");
        assert!(is_complete(&path));
        std::fs::write(&path, b"not json").expect("write junk");
        assert!(!is_complete(&path), "a junk marker never counts as finished");
        std::fs::remove_dir_all(dir).expect("clean up");
    }

    #[test]
    fn only_a_non_empty_hosts_file_is_a_saved_machine() {
        let dir = temp_dir("hosts");
        let path = dir.join("hosts.json");
        assert!(!has_saved_hosts(&path), "no file: a first run");
        std::fs::write(&path, b"[]").expect("write an empty list");
        assert!(!has_saved_hosts(&path), "an empty list is still a first run");
        std::fs::write(&path, br#"[{"name":"Build box","address":"ssh:build","ssh":"build"}]"#).expect("write a machine");
        assert!(has_saved_hosts(&path));
        std::fs::write(&path, b"not json").expect("write junk");
        assert!(!has_saved_hosts(&path), "junk never blocks the first run");
        std::fs::remove_dir_all(dir).expect("clean up");
    }

    fn workspace(id: &str, path: &str, map: &str) -> crate::workspaces::Workspace {
        crate::workspaces::Workspace { id: id.into(), label: id.into(), path: Some(path.into()), map: map.into(), ..Default::default() }
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
    fn every_step_draws_its_title_copy_and_actions() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, false, 0);
        let mut app = FigApp::new(figcraft_engine::Session::empty(), Box::new(crate::NoServices));
        app.onboarding.active = true;
        app.onboarding.decided = true;
        // Don't start the list request from a unit test: the state is what these steps draw.
        app.onboarding.asked = true;
        let mut time = 0.0_f64;
        let cases: [(Step, &[&str], bool); 5] = [
            (
                Step::Welcome,
                &[
                    "WELCOME TO MINDMAP",
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
                    "SETUP CHECK",
                    "One check before the agents run",
                    "Mindmap node",
                    "Herdr, which runs your agents",
                    "This Mac",
                    "Check again",
                    "Continue",
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
                    "/code/acme",
                    "Has a map",
                    "web",
                    "/code/web",
                    "No map yet",
                    "Open another folder",
                    "Connect a remote machine",
                ],
                true,
            ),
            (
                Step::NoProject,
                &[
                    "NO PROJECT YET",
                    "Point Mindmap at a project",
                    "Open a folder on this Mac",
                    "Connect a machine",
                    "Try the sample map",
                    "Skip for now",
                ],
                false,
            ),
            (
                Step::FirstMap,
                &["FIRST MAP", "The map is yours now", "Comment on a card, then Send", "Sync turns the design into the code", "Open the map"],
                false,
            ),
        ];
        for (step, expected, has_workspaces) in cases {
            app.onboarding.step = step;
            app.ws.rows =
                if has_workspaces { vec![workspace("acme", "/code/acme", "map"), workspace("web", "/code/web", "none")] } else { Vec::new() };
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
