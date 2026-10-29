//! Check for Updates: the Update sheet (`Update` on the Mindmap page, screen 06) and the checks
//! behind it.
//!
//! Updates come from prebuilt releases, never from source. A release feed is one JSON document
//! at an HTTPS URL: [`FEED_ENV`] when set, else [`DEFAULT_FEED`] (none yet: no release pipeline
//! publishes one, so Check ends Up to date with a note saying so).
//!
//! ```json
//! {
//!   "version": "0.8.0",
//!   "notes": ["Faster map open and sync checks", "Update from the app, no terminal"],
//!   "platforms": {
//!     "darwin-arm64": {
//!       "app":  { "url": "https://…/Mindmap-0.8.0-darwin-arm64.zip", "sha256": "<64 hex>" },
//!       "node": { "url": "https://…/mindmap-node-0.8.0-darwin-arm64", "sha256": "<64 hex>",
//!                 "build_id": "<the node's mindmap_build::BUILD_ID>" }
//!     }
//!   }
//! }
//! ```
//!
//! `version` is the app's Cargo version (`mindmap_build::VERSION`); a release is offered when it
//! is newer than the running app's. Platforms use the node bundle's names (`darwin-arm64`,
//! `darwin-x64`). The app archive is a `ditto -c -k --keepParent` zip of a signed `Mindmap.app`;
//! the node is the bare signed binary. Both are checked against their SHA-256 before anything is
//! installed, and the app with `codesign --verify --deep --strict`.
//!
//! Install (macOS app only, [`crate::update_install`]): download, verify, refuse while a Sync is
//! prepared or a merge is pending (the update waits), install the node with the new bundle's
//! `install-node.sh --upgrade` (its exit 21 means the same wait), swap the `.app`, relaunch.
//! Herdr and the agents keep running throughout.
//!
//! Mindmap checks at launch and daily ([`due`], the last check kept in the preferences). The
//! states, what each shows ([`view`]) and the feed handling are pure and unit-tested.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use egui::{Color32, CornerRadius, FontId, Id, Rect, Sense, Stroke, StrokeKind, Ui, vec2};
use serde::Deserialize;

use crate::FigApp;
use crate::machine_modal::shell;
use crate::theme::{Tokens, regular, semibold};

/// The environment variable naming the release feed's URL.
pub const FEED_ENV: &str = "MINDMAP_UPDATE_FEED";
/// The release feed built into the app. `None` until releases are published.
pub const DEFAULT_FEED: Option<&str> = None;
/// How often Mindmap looks for a release by itself.
pub const CHECK_EVERY_SECS: u64 = 24 * 60 * 60;
/// What the sheet says when no feed is configured.
pub const NO_FEED: &str = "No release feed is set up yet.";
/// The local node an update replaces.
pub const LOCAL_NODE: &str = "127.0.0.1:47822";

/// The release feed's URL: the environment's, else the built-in one. Empty means none.
pub fn feed_url() -> Option<String> {
    feed_url_from(std::env::var(FEED_ENV).ok(), DEFAULT_FEED)
}

/// [`feed_url`] with its inputs given.
pub fn feed_url_from(env: Option<String>, default: Option<&str>) -> Option<String> {
    env.map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
        .or_else(|| default.map(str::trim).filter(|url| !url.is_empty()).map(str::to_string))
}

/// The release feed (see the module documentation).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Feed {
    pub version: String,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub platforms: BTreeMap<String, Platform>,
}

/// One platform's downloads.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Platform {
    pub app: Asset,
    pub node: NodeAsset,
}

/// A download and its checksum.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Asset {
    pub url: String,
    pub sha256: String,
}

/// The node binary, with the build it reports in its hello.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct NodeAsset {
    pub url: String,
    pub sha256: String,
    pub build_id: String,
}

/// Parse and validate a feed: a comparable version, HTTPS (or local `file://`) downloads and
/// 64-digit hex checksums.
pub fn parse_feed(text: &str) -> Result<Feed, String> {
    let feed: Feed = serde_json::from_str(text).map_err(|error| format!("The release feed is not valid: {error}"))?;
    if version_parts(&feed.version).is_none() {
        return Err(format!("The release feed's version {:?} is not a version number.", feed.version));
    }
    for (name, platform) in &feed.platforms {
        for (url, sha) in [(&platform.app.url, &platform.app.sha256), (&platform.node.url, &platform.node.sha256)] {
            if !(url.starts_with("https://") || url.starts_with("file://")) {
                return Err(format!("The release feed's {name} download is not an HTTPS URL."));
            }
            if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!("The release feed's {name} checksum is not a SHA-256."));
            }
        }
        if platform.node.build_id.trim().is_empty() {
            return Err(format!("The release feed's {name} node has no build id."));
        }
    }
    Ok(feed)
}

/// `1.2.3` (an optional leading `v`, a `-pre` suffix kept apart) as numbers and its pre-release.
fn version_parts(version: &str) -> Option<(Vec<u64>, Option<&str>)> {
    let version = version.trim().trim_start_matches('v');
    let (core, pre) = match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version, None),
    };
    let numbers: Option<Vec<u64>> = core.split('.').map(|part| part.parse().ok()).collect();
    numbers.filter(|n| !n.is_empty()).map(|n| (n, pre))
}

/// Compare two versions: numerically part by part (missing parts are 0), a pre-release before
/// its release. Unparseable versions compare equal (never offered as newer).
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let (Some((a, a_pre)), Some((b, b_pre))) = (version_parts(a), version_parts(b)) else { return Ordering::Equal };
    for i in 0..a.len().max(b.len()) {
        match a.get(i).copied().unwrap_or(0).cmp(&b.get(i).copied().unwrap_or(0)) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    match (a_pre, b_pre) {
        (None, None) => Ordering::Equal,
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (Some(a), Some(b)) => a.cmp(b),
    }
}

/// This machine's platform name in the feed (`darwin-arm64`), as the node bundle names it.
pub fn platform_name(os: &str, arch: &str) -> Option<String> {
    let os = match os {
        "macos" => "darwin",
        "linux" => "linux",
        _ => return None,
    };
    let arch = match arch {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        _ => return None,
    };
    Some(format!("{os}-{arch}"))
}

/// A release this machine can install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub notes: Vec<String>,
    pub app: Asset,
    pub node: NodeAsset,
}

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Checked {
    /// Nothing newer; the note says why when it is not simply the latest (no feed, no build for
    /// this machine).
    UpToDate(Option<String>),
    Available(Release),
    Failed(String),
}

/// Whether `feed` offers this app something newer on `platform`.
pub fn decide(feed: &Feed, platform: Option<&str>, app_version: &str) -> Checked {
    if compare_versions(&feed.version, app_version) != Ordering::Greater {
        return Checked::UpToDate(None);
    }
    match platform.and_then(|name| feed.platforms.get(name)) {
        Some(build) => {
            Checked::Available(Release { version: feed.version.clone(), notes: feed.notes.clone(), app: build.app.clone(), node: build.node.clone() })
        }
        None => Checked::UpToDate(Some(format!("Mindmap {} has no build for this machine yet.", feed.version))),
    }
}

/// Whether a check is due: never checked, or [`CHECK_EVERY_SECS`] since the last one (or a
/// clock that went back).
pub fn due(last: Option<u64>, now: u64) -> bool {
    last.is_none_or(|last| now < last || now - last >= CHECK_EVERY_SECS)
}

/// How an install ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Installed {
    /// The node is updated and the app replaced; Mindmap quits and reopens.
    Restarting,
    /// A prepared Sync or a pending merge holds the update; nothing changed.
    Waits(String),
    Failed(String),
}

/// A running check's result: what it found and the node's build id.
#[cfg(not(target_arch = "wasm32"))]
type CheckReply = std::sync::mpsc::Receiver<(Checked, Option<String>)>;

/// Where the sheet is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    /// Never checked.
    #[default]
    Idle,
    Checking,
    UpToDate(Option<String>),
    Available(Release),
    Downloading(Release),
    /// Available, but a prepared Sync or pending merge holds the install.
    Waiting {
        release: Release,
        reason: String,
    },
    Restarting(Release),
    Failed {
        error: String,
        release: Option<Release>,
    },
}

impl Phase {
    /// The release this phase is about.
    pub fn release(&self) -> Option<&Release> {
        match self {
            Phase::Available(r) | Phase::Downloading(r) | Phase::Restarting(r) | Phase::Waiting { release: r, .. } => Some(r),
            Phase::Failed { release, .. } => release.as_ref(),
            _ => None,
        }
    }

    /// A check or an install is running.
    pub fn busy(&self) -> bool {
        matches!(self, Phase::Checking | Phase::Downloading(_) | Phase::Restarting(_))
    }
}

/// The sheet's state.
#[derive(Default)]
pub struct State {
    /// The sheet is open.
    pub open: bool,
    pub phase: Phase,
    /// The running node's build id, from its hello (`None`: not reachable).
    pub node_build: Option<String>,
    /// The version Later was chosen for: an automatic check doesn't open the sheet for it again.
    pub dismissed: Option<String>,
    /// The last check this run (Unix seconds), so a check with nowhere to save still waits a day.
    pub last_check: Option<u64>,
    #[cfg(not(target_arch = "wasm32"))]
    check: Option<(CheckReply, bool)>,
    #[cfg(not(target_arch = "wasm32"))]
    install: Option<crate::update_install::Job>,
}

impl State {
    /// A closed sheet at `phase` (tests and previews).
    pub fn at(phase: Phase) -> Self {
        Self { phase, ..Self::default() }
    }

    /// A check started.
    pub fn checking(&mut self) {
        self.phase = Phase::Checking;
    }

    /// A check ended. An automatic one opens the sheet only for a release not put off with Later.
    pub fn checked(&mut self, checked: Checked, node_build: Option<String>, automatic: bool) {
        self.node_build = node_build;
        self.phase = match checked {
            Checked::UpToDate(note) => Phase::UpToDate(note),
            Checked::Available(release) => {
                if automatic && self.dismissed.as_deref() != Some(release.version.as_str()) {
                    self.open = true;
                }
                Phase::Available(release)
            }
            Checked::Failed(error) => Phase::Failed { error, release: None },
        };
    }

    /// Install and restart was chosen.
    pub fn downloading(&mut self) -> Option<Release> {
        let release = match &self.phase {
            Phase::Available(r) | Phase::Waiting { release: r, .. } | Phase::Failed { release: Some(r), .. } => r.clone(),
            _ => return None,
        };
        self.phase = Phase::Downloading(release.clone());
        Some(release)
    }

    /// The install ended.
    pub fn installed(&mut self, outcome: Installed) {
        let Some(release) = self.phase.release().cloned() else { return };
        self.phase = match outcome {
            Installed::Restarting => Phase::Restarting(release),
            Installed::Waits(reason) => Phase::Waiting { release, reason },
            Installed::Failed(error) => Phase::Failed { error, release: Some(release) },
        };
    }

    /// Later: close, and don't open by itself again for this version.
    pub fn later(&mut self) {
        if let Some(release) = self.phase.release() {
            self.dismissed = Some(release.version.clone());
        }
        self.close();
    }

    /// Close the sheet, unless an install is running (its Cancel stops it first).
    pub fn close(&mut self) {
        if !matches!(self.phase, Phase::Downloading(_) | Phase::Restarting(_)) {
            self.open = false;
        }
    }
}

/// What a button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Install,
    Later,
    Close,
    CheckAgain,
    Cancel,
}

/// A button of the sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    pub label: &'static str,
    pub enabled: bool,
    pub act: Act,
}

impl Button {
    fn new(label: &'static str, act: Act) -> Self {
        Self { label, enabled: true, act }
    }

    fn off(label: &'static str, act: Act) -> Self {
        Self { label, enabled: false, act }
    }
}

/// What the sheet shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    /// The design's state name: Checking, Up to date, Available, Downloading, Restarting, Failed.
    pub state: &'static str,
    pub title: String,
    pub body: String,
    /// Label and value: Mindmap, Node, Agents.
    pub rows: Vec<(&'static str, String)>,
    /// What's new.
    pub notes: Vec<String>,
    /// A boxed note above the buttons; `true`: the accent warning.
    pub note: Option<(String, bool)>,
    pub primary: Option<Button>,
    pub secondary: Option<Button>,
}

/// The design's six state names, in its order.
pub const STATES: [&str; 6] = ["Checking", "Up to date", "Available", "Downloading", "Restarting", "Failed"];

/// The body of the Available sheet.
pub const AVAILABLE_BODY: &str = "The app and its node update together. Agents keep running; a prepared Sync waits until the restart.";
/// The line under the buttons.
pub const SAFETY: &str = "Prepared Sync or pending merge? The update waits.";

/// A build id as the sheet shows it: its first six characters.
pub fn short_build(build: &str) -> String {
    build.chars().take(6).collect()
}

/// What `state` shows, for an app at `app_version` that can (`can_install`) install here.
pub fn view(state: &State, app_version: &str, can_install: bool) -> View {
    let node_now = state.node_build.as_deref().map_or_else(|| "not reachable".to_string(), short_build);
    let rows = |release: Option<&Release>| -> Vec<(&'static str, String)> {
        let (app, node) = match release {
            Some(r) => (format!("{app_version}  →  {}", r.version), format!("{node_now}  →  {}", short_build(&r.node.build_id))),
            None => (app_version.to_string(), node_now.clone()),
        };
        vec![("Mindmap", app), ("Node", node), ("Agents", "Herdr keeps running".to_string())]
    };
    let notes = |release: &Release| release.notes.clone();
    match &state.phase {
        Phase::Idle | Phase::Checking => View {
            state: "Checking",
            title: "Checking for updates…".into(),
            body: "Looking for a newer Mindmap app and node. Agents keep running.".into(),
            rows: rows(None),
            notes: Vec::new(),
            note: None,
            primary: Some(Button::off("Checking…", Act::Install)),
            secondary: Some(Button::new("Close", Act::Close)),
        },
        Phase::UpToDate(note) => View {
            state: "Up to date",
            title: format!("Mindmap {app_version} is up to date"),
            body: "The app and its node are on the latest release. Mindmap checks again at launch and daily.".into(),
            rows: rows(None),
            notes: Vec::new(),
            note: note.clone().map(|note| (note, false)),
            primary: Some(Button::new("Done", Act::Close)),
            secondary: Some(Button::new("Check again", Act::CheckAgain)),
        },
        Phase::Available(release) => View {
            state: "Available",
            title: format!("Mindmap {} is ready", release.version),
            body: AVAILABLE_BODY.into(),
            rows: rows(Some(release)),
            notes: notes(release),
            note: (!can_install).then(|| ("Updates install from the Mindmap app on a Mac.".to_string(), false)),
            primary: Some(Button { enabled: can_install, ..Button::new("Install and restart", Act::Install) }),
            secondary: Some(Button::new("Later", Act::Later)),
        },
        Phase::Downloading(release) => View {
            state: "Downloading",
            title: format!("Downloading Mindmap {}", release.version),
            body: "Downloading and verifying the app and its node, then installing the node. Agents keep running.".into(),
            rows: rows(Some(release)),
            notes: notes(release),
            note: None,
            primary: Some(Button::off("Downloading…", Act::Install)),
            secondary: Some(Button::new("Cancel", Act::Cancel)),
        },
        Phase::Waiting { release, reason } => View {
            state: "Available",
            title: format!("Mindmap {} is waiting", release.version),
            body: "A prepared Sync or a pending merge holds the update; nothing was changed. Finish or cancel it, then install.".into(),
            rows: rows(Some(release)),
            notes: notes(release),
            note: Some((reason.clone(), true)),
            primary: Some(Button { enabled: can_install, ..Button::new("Try again", Act::Install) }),
            secondary: Some(Button::new("Later", Act::Later)),
        },
        Phase::Restarting(release) => View {
            state: "Restarting",
            title: format!("Restarting into Mindmap {}", release.version),
            body: "The node is updated. Mindmap quits and opens again in a moment; agents keep running.".into(),
            rows: rows(Some(release)),
            notes: notes(release),
            note: None,
            primary: Some(Button::off("Restarting…", Act::Install)),
            secondary: None,
        },
        Phase::Failed { error, release } => View {
            state: "Failed",
            title: "The update didn't finish".into(),
            body: "Agents keep running. Try again, or install the release by hand.".into(),
            rows: rows(release.as_ref()),
            notes: Vec::new(),
            note: Some((error.clone(), true)),
            primary: Some(match release {
                Some(_) => Button { enabled: can_install, ..Button::new("Try again", Act::Install) },
                None => Button::new("Try again", Act::CheckAgain),
            }),
            secondary: Some(Button::new("Close", Act::Close)),
        },
    }
}

/// This app can install a release here: the native macOS app.
pub fn can_install() -> bool {
    cfg!(all(target_os = "macos", not(target_arch = "wasm32")))
}

fn now_secs() -> u64 {
    web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Check for Updates…: open the sheet and look again (unless an install is under way).
pub fn open(app: &mut FigApp, ctx: &egui::Context) {
    app.update.open = true;
    if !matches!(app.update.phase, Phase::Downloading(_) | Phase::Restarting(_) | Phase::Waiting { .. }) {
        start_check(app, false);
    }
    ctx.request_repaint();
}

/// Look for a release: with no feed this ends at once, Up to date with [`NO_FEED`].
fn start_check(app: &mut FigApp, automatic: bool) {
    let now = now_secs();
    app.update.last_check = Some(now);
    let feed = feed_url();
    if feed.is_some() {
        app.session.prefs.update_checked = Some(now);
        app.services.save_prefs(&app.session.prefs);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if automatic && feed.is_none() {
            // Nothing to ask; don't wake the node for it.
            app.update.checked(Checked::UpToDate(Some(NO_FEED.into())), app.update.node_build.clone(), true);
            return;
        }
        app.update.checking();
        let (tx, rx) = std::sync::mpsc::channel();
        app.update.check = Some((rx, automatic));
        std::thread::spawn(move || {
            let node = node_build();
            let checked = match feed {
                None => Checked::UpToDate(Some(NO_FEED.into())),
                Some(url) => match crate::update_install::fetch_text(&url).and_then(|text| parse_feed(&text)) {
                    Ok(feed) => decide(&feed, platform_name(std::env::consts::OS, std::env::consts::ARCH).as_deref(), mindmap_build::VERSION),
                    Err(error) => Checked::Failed(error),
                },
            };
            let _ = tx.send((checked, node));
        });
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = feed;
        app.update.checked(Checked::UpToDate(Some("Updates install from the Mindmap app.".into())), None, automatic);
    }
}

/// The running local node's build id, from its hello.
#[cfg(not(target_arch = "wasm32"))]
fn node_build() -> Option<String> {
    let reply = crate::node::rpc(LOCAL_NODE, serde_json::json!({"method":"hello"}), std::time::Duration::from_secs(10)).ok()?;
    reply.get("build_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_string)
}

/// Every frame in map mode: take in a finished check or install, and check at launch and daily.
pub fn tick(app: &mut FigApp, ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    poll(app, ctx);
    // Tests never reach for the network or the node by themselves.
    if cfg!(test) || !crate::map_mode() || app.update.phase.busy() || matches!(app.update.phase, Phase::Waiting { .. }) {
        return;
    }
    let now = now_secs();
    let last = app.session.prefs.update_checked.max(app.update.last_check);
    if due(last, now) {
        start_check(app, true);
    } else if let Some(last) = last {
        ctx.request_repaint_after(std::time::Duration::from_secs((last + CHECK_EVERY_SECS).saturating_sub(now).max(60)));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn poll(app: &mut FigApp, ctx: &egui::Context) {
    if let Some((rx, automatic)) = app.update.check.take() {
        match rx.try_recv() {
            Ok((checked, node)) => app.update.checked(checked, node, automatic),
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                app.update.check = Some((rx, automatic));
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                app.update.checked(Checked::Failed("The update check stopped unexpectedly.".into()), None, automatic)
            }
        }
    }
    if let Some(job) = app.update.install.take() {
        match job.outcome() {
            Some(outcome) => {
                let restart = outcome == Installed::Restarting;
                app.update.installed(outcome);
                if restart {
                    // The relaunch helper waits for this process to end, then opens the new app.
                    app.ui.quit = true;
                    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
                }
            }
            None => {
                app.update.install = Some(job);
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
        }
    }
}

fn act(app: &mut FigApp, ctx: &egui::Context, act: Act) {
    match act {
        Act::Install => {
            if !can_install() {
                return;
            }
            if let Some(release) = app.update.downloading() {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    app.update.install = Some(crate::update_install::begin(release));
                }
                #[cfg(target_arch = "wasm32")]
                let _ = release;
            }
        }
        Act::Later => app.update.later(),
        Act::Close => app.update.close(),
        Act::CheckAgain => start_check(app, false),
        Act::Cancel => {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(job) = app.update.install.take() {
                job.cancel();
            }
            app.update.installed(Installed::Failed("Cancelled. Nothing was installed.".into()));
        }
    }
    ctx.request_repaint();
}

/// The sheet's width, as designed; its height follows the content (470 for the design's).
const WIDTH: f32 = 560.0;
/// The content's left edge.
const X: f32 = 28.0;
/// Where the row values start.
const VALUE_X: f32 = 150.0;

/// The colours the design adds to the shared modal skin.
struct Ink {
    notes: Color32,
    later_border: Color32,
}

impl Ink {
    fn of(t: &Tokens) -> Self {
        if t.dark {
            Self { notes: t.text_secondary, later_border: t.border }
        } else {
            Self { notes: Color32::from_rgb(0x55, 0x53, 0x4F), later_border: Color32::from_rgb(0xD8, 0xD5, 0xCF) }
        }
    }
}

/// Draw the sheet when open.
pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    if !app.update.open {
        return;
    }
    let tokens = Tokens::get(ctx);
    let skin = shell::Skin::of(&tokens);
    let ink = Ink::of(&tokens);
    let view = view(&app.update, mindmap_build::VERSION, can_install());
    let response = egui::Modal::new(Id::new("update_sheet"))
        .backdrop_color(shell::scrim(&tokens))
        .frame(shell::surface(&skin))
        .show(ctx, |ui| sheet(ui, &skin, &ink, &view));
    if response.should_close() {
        app.update.close();
    }
    if let Some(chosen) = response.inner {
        act(app, ctx, chosen);
    }
}

fn galley(ui: &Ui, text: &str, font: FontId, color: Color32, width: f32, line: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = width;
    job.append(text, 0.0, egui::TextFormat { font_id: font, color, line_height: Some(line), ..Default::default() });
    ui.painter().layout_job(job)
}

/// The sheet at the design's offsets; the button pressed, if any.
fn sheet(ui: &mut Ui, s: &shell::Skin, ink: &Ink, view: &View) -> Option<Act> {
    let text_w = WIDTH - 2.0 * X;
    let title = galley(ui, &view.title, shell::serif(ui.ctx(), 30.0), s.text, text_w, 34.0);
    let body = galley(ui, &view.body, regular(12.0), s.muted, text_w, 18.0);
    let notes = galley(ui, &view.notes.iter().map(|n| format!("• {n}")).collect::<Vec<_>>().join("\n"), regular(11.0), ink.notes, text_w, 16.0);
    let note = view
        .note
        .as_ref()
        .map(|(text, warn)| (galley(ui, text, FontId::monospace(8.5), if *warn { s.accent } else { s.muted }, text_w - 24.0, 12.0), *warn));
    // The design: body at 90, the rule 22 below it, rows every 30, What's new 40 below the last.
    let body_y = 90.0f32.max(44.0 + title.size().y + 12.0);
    let rule_y = body_y + body.size().y.max(18.0) + 22.0;
    let rows_y = rule_y + 18.0;
    let mut y = rows_y + 30.0 * view.rows.len().saturating_sub(1) as f32 + 40.0;
    let notes_y = y + 20.0;
    if !view.notes.is_empty() {
        y = notes_y + notes.size().y + 22.0;
    }
    let note_y = y;
    if let Some((galley, _)) = &note {
        y += galley.size().y + 20.0 + 14.0;
    }
    let buttons_y = y;
    let height = buttons_y + 98.0;

    let card = shell::card(ui, WIDTH, height);
    let painter = ui.painter().clone();
    let at = |x: f32, y: f32| card.min + vec2(x, y);
    painter.text(at(X, 26.0), egui::Align2::LEFT_TOP, "UPDATE", FontId::monospace(9.0), s.accent);
    painter.galley(at(X, 44.0 - 6.0), title, s.text);
    painter.galley(at(X, body_y), body, s.muted);
    painter.rect_filled(Rect::from_min_size(at(X, rule_y), vec2(text_w, 1.0)), CornerRadius::ZERO, s.border);
    for (i, (label, value)) in view.rows.iter().enumerate() {
        let y = rows_y + 30.0 * i as f32;
        painter.text(at(X, y), egui::Align2::LEFT_TOP, *label, semibold(10.5), s.muted);
        painter.text(at(VALUE_X, y), egui::Align2::LEFT_TOP, value, FontId::monospace(10.0), s.text);
    }
    if !view.notes.is_empty() {
        painter.text(at(X, notes_y - 20.0), egui::Align2::LEFT_TOP, "WHAT'S NEW", FontId::monospace(9.0), s.faint);
        painter.galley(at(X, notes_y), notes, ink.notes);
    }
    if let Some((galley, warn)) = note {
        let rect = Rect::from_min_size(at(X, note_y), vec2(text_w, galley.size().y + 20.0));
        let (fill, color) = if warn { (s.warn_fill, s.accent) } else { (s.soft, s.muted) };
        painter.rect_filled(rect, CornerRadius::same(6), fill);
        if warn {
            painter.rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, s.warn_stroke), StrokeKind::Inside);
        }
        painter.galley(rect.min + vec2(12.0, 10.0), galley, color);
    }
    let mut chosen = None;
    let mut x = X;
    if let Some(primary) = &view.primary {
        let width = 200.0f32.max(shell::text_width(ui, primary.label, semibold(11.5)) + 40.0);
        if button(ui, s, ink, Rect::from_min_size(at(x, buttons_y), vec2(width, 38.0)), primary, true) {
            chosen = Some(primary.act);
        }
        x += width + 12.0;
    }
    if let Some(secondary) = &view.secondary {
        let width = 90.0f32.max(shell::text_width(ui, secondary.label, semibold(11.5)) + 36.0);
        if button(ui, s, ink, Rect::from_min_size(at(x, buttons_y), vec2(width, 38.0)), secondary, false) {
            chosen = Some(secondary.act);
        }
    }
    painter.text(at(X, buttons_y + 54.0), egui::Align2::LEFT_TOP, SAFETY, FontId::monospace(8.5), s.faint);
    chosen
}

/// The accent Install button (`primary`) or the outlined Later button.
fn button(ui: &mut Ui, s: &shell::Skin, ink: &Ink, rect: Rect, button: &Button, primary: bool) -> bool {
    let response = ui.interact(rect, Id::new(("update_sheet", button.label)), if button.enabled { Sense::click() } else { Sense::hover() });
    crate::a11y::button(&response, button.label);
    let hot = button.enabled && response.hovered();
    let (fill, stroke, color) = match (primary, button.enabled) {
        (true, true) => (if hot { s.accent.gamma_multiply(0.9) } else { s.accent }, None, Color32::WHITE),
        (true, false) => (s.soft, None, s.faint),
        (false, _) => (if hot { s.hover } else { s.panel }, Some(ink.later_border), if button.enabled { s.text } else { s.faint }),
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(8), fill);
    if let Some(stroke) = stroke {
        painter.rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0, stroke), StrokeKind::Inside);
    }
    painter.text(rect.min + vec2(if primary { 20.0 } else { 18.0 }, 12.0), egui::Align2::LEFT_TOP, button.label, semibold(11.5), color);
    if response.has_focus() {
        painter.rect_stroke(rect, CornerRadius::same(8), Stroke::new(2.0, s.focus), StrokeKind::Outside);
    }
    button.enabled && response.clicked()
}
