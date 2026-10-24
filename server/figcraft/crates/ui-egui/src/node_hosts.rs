//! Host setup and transport. Network and process work always runs outside the UI thread.
use std::collections::HashMap;
use std::io::Read;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;

const LOCAL: &str = "127.0.0.1:47822";
const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Host {
    pub name: String,
    /// Stable RPC pool key; SSH hosts use `ssh:<target>`.
    pub address: String,
    pub ssh: Option<String>,
}

static HOSTS: OnceLock<Mutex<HashMap<String, Host>>> = OnceLock::new();

pub fn resolve(address: &str) -> Host {
    if let Ok(hosts) = HOSTS.get_or_init(Mutex::default).lock()
        && let Some(host) = hosts.get(address)
    {
        return host.clone();
    }
    Host {
        name: address.strip_prefix("ssh:").unwrap_or("This computer").to_owned(),
        address: address.to_owned(),
        ssh: address.strip_prefix("ssh:").map(str::to_owned),
    }
}

fn register(host: &Host) -> Result<(), String> {
    HOSTS.get_or_init(Mutex::default).lock().map_err(|_| "Host registry unavailable".to_owned())?.insert(host.address.clone(), host.clone());
    Ok(())
}

/// The machines the strip knows: this computer and the saved SSH hosts.
pub(crate) fn known(state: &State) -> Vec<Host> {
    let mut hosts = vec![resolve(LOCAL)];
    hosts.extend(state.hosts.iter().filter(|host| host.ssh.is_some()).cloned());
    hosts.truncate(256);
    hosts
}

/// Remember a machine a setup reached and save it for the next launch.
pub(crate) fn remember(state: &mut State, host: &Host) -> Result<(), String> {
    if !state.hosts.iter().any(|known| known.address == host.address) {
        state.hosts.push(host.clone());
        state.hosts.truncate(256);
    }
    state.selected = Some(host.clone());
    save_hosts(&state.hosts)
}

/// The outcome of one host setup. The host strip and the Add machine modal both start the same
/// [`prepare_with`] work; each renders the same outcome its own way.
#[derive(Clone, Debug)]
pub(crate) enum Outcome {
    Ready(Host),
    Approval(Host),
    Update(Host, String),
    Failed(String),
}

/// One step of a setup, reported as the worker reaches it. The modal shows the steps the design
/// names (SSH, copy, start, Herdr) and leaves the ones a setup skipped marked as not needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Reaching the node: the SSH tunnel and the protocol handshake.
    Ssh,
    /// Copying mindmap-node to the host and running its installer.
    Copy,
    /// The node is installed and starts on 127.0.0.1:47822.
    Start,
    /// Checking Herdr and preparing the shared session (asking before installing it).
    Herdr,
}

/// A host setup running on a thread of its own: its progress and, once, its outcome.
pub(crate) struct Job {
    outcome: mpsc::Receiver<Outcome>,
    phases: mpsc::Receiver<Phase>,
    cancelled: Arc<AtomicBool>,
    finished: bool,
}

impl Job {
    /// The phases reported since the last frame (a few at most).
    pub(crate) fn phases(&mut self) -> Vec<Phase> {
        let mut phases = Vec::new();
        while phases.len() < 8 {
            match self.phases.try_recv() {
                Ok(phase) => phases.push(phase),
                Err(_) => break,
            }
        }
        phases
    }

    /// The outcome once the setup has finished.
    pub(crate) fn outcome(&mut self) -> Option<Outcome> {
        if self.finished {
            return None;
        }
        let outcome = match self.outcome.try_recv() {
            Ok(outcome) => outcome,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => Outcome::Failed("Host setup stopped unexpectedly".into()),
        };
        self.finished = true;
        Some(outcome)
    }

    /// Ask the setup to stop before its next step. A step already talking to the host finishes.
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

/// Stop a setup between its phases when its modal was closed.
fn stop_if_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Relaxed) { Err("Host setup cancelled.".into()) } else { Ok(()) }
}

/// Start the same host preparation the host strip runs, reporting each phase as it happens.
pub(crate) fn begin(host: Host, install: bool) -> Result<Job, String> {
    let (outcome_tx, outcome_rx) = mpsc::channel();
    let (phase_tx, phase_rx) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&cancelled);
    std::thread::Builder::new()
        .name("figcraft-host-setup".into())
        .spawn(move || {
            let result = prepare_with(
                &host,
                install,
                false,
                &mut |phase| {
                    let _ = phase_tx.send(phase);
                },
                &stop,
            )
            .unwrap_or_else(Outcome::Failed);
            let _ = outcome_tx.send(result);
        })
        .map_err(|error| format!("Could not start host setup: {error}"))?;
    Ok(Job { outcome: outcome_rx, phases: phase_rx, cancelled, finished: false })
}

#[derive(Default)]
pub struct State {
    pub selected: Option<Host>,
    hosts: Vec<Host>,
    target: String,
    name: String,
    pending: Option<mpsc::Receiver<Outcome>>,
    approval: Option<Host>,
    update: Option<(Host, String)>,
    error: Option<String>,
    loaded: bool,
}

/// Returns a host only after its node and shared Herdr session are ready.
/// Load the saved machines and take in a finished setup. Returns the host when it is ready.
pub fn poll(state: &mut State) -> Option<Host> {
    if !state.loaded {
        state.loaded = true;
        state.hosts.push(resolve(LOCAL));
        if let Some(path) = hosts_path()
            && let Ok(bytes) = std::fs::read(path)
            && let Ok(saved) = serde_json::from_slice::<Vec<Host>>(&bytes)
        {
            state.hosts.extend(saved.into_iter().filter(|h| h.ssh.is_some()).take(128));
        }
    }
    let result = state.pending.as_ref().and_then(|rx| match rx.try_recv() {
        Ok(value) => Some(value),
        Err(mpsc::TryRecvError::Disconnected) => Some(Outcome::Failed("Host setup stopped unexpectedly".into())),
        Err(mpsc::TryRecvError::Empty) => None,
    });
    if let Some(result) = result {
        state.pending = None;
        match result {
            Outcome::Ready(host) => {
                if let Err(error) = remember(state, &host) {
                    state.error = Some(error);
                }
                state.approval = None;
                state.update = None;
                return Some(host);
            }
            Outcome::Approval(host) => {
                state.update = None;
                state.approval = Some(host);
            }
            Outcome::Update(host, reason) => state.update = Some((host, reason)),
            Outcome::Failed(error) => state.error = Some(error),
        }
    }
    None
}

/// The machines to switch between: one chip each, the selected one highlighted.
///
/// Adding a machine lives in [`crate::machine_modal`]; this row only selects one that is saved.
pub fn chips(ui: &mut egui::Ui, state: &mut State) {
    let enabled = state.pending.is_none() && state.approval.is_none() && state.update.is_none();
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled_ui(enabled, |ui| {
            for host in state.hosts.clone() {
                let selected = state.selected.as_ref().is_some_and(|h| h.address == host.address);
                if ui.selectable_label(selected, &host.name).clicked() {
                    start(state, host, false, false);
                }
            }
        });
    });
}

/// What a running or failed setup says: an approval, an update, progress, the error.
pub fn status(ui: &mut egui::Ui, state: &mut State) {
    if let Some(host) = state.approval.clone() {
        ui.label(format!("Herdr is missing on {}. Install it to run agents?", host.name));
        ui.horizontal(|ui| {
            if ui.button("Install Herdr").clicked() {
                state.approval = None;
                start(state, host, true, false);
            }
            if ui.button("Cancel").clicked() {
                state.approval = None;
            }
        });
    }
    if let Some((host, reason)) = state.update.clone() {
        ui.label(reason);
        ui.label(
            "Updating restarts the Mindmap node. Prepared Sync jobs and pending merges must be resolved first. Herdr and its agents keep running.",
        );
        ui.horizontal(|ui| {
            if ui.add_enabled(state.pending.is_none(), egui::Button::new("Update node")).clicked() {
                start(state, host, false, true);
            }
            if ui.add_enabled(state.pending.is_none(), egui::Button::new("Cancel")).clicked() {
                state.update = None;
            }
        });
    }
    if state.pending.is_some() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Preparing host\u{2026}");
        });
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
    if let Some(error) = &state.error {
        ui.label(error);
    }
}

/// Start host setup without granting permission to install Herdr.
pub fn select(state: &mut State, host: Host) {
    if state.pending.is_none() {
        state.approval = None;
        state.update = None;
        start(state, host, false, false);
    }
}

fn start(state: &mut State, host: Host, install: bool, update: bool) {
    state.error = None;
    let (tx, rx) = mpsc::channel();
    state.pending = Some(rx);
    std::thread::spawn(move || {
        let result = prepare(&host, install, update).unwrap_or_else(Outcome::Failed);
        let _ = tx.send(result);
    });
}

/// Host setup without a progress observer: the host strip's path.
fn prepare(host: &Host, install: bool, update: bool) -> Result<Outcome, String> {
    static NOT_CANCELLED: AtomicBool = AtomicBool::new(false);
    prepare_with(host, install, update, &mut |_| {}, &NOT_CANCELLED)
}

/// The one host setup: install the node if needed, prepare Herdr (asking before installing it)
/// and save the machine. `progress` reports each phase; `cancelled` is checked between phases.
fn prepare_with(host: &Host, install: bool, update: bool, progress: &mut dyn FnMut(Phase), cancelled: &AtomicBool) -> Result<Outcome, String> {
    register(host)?;
    progress(Phase::Ssh);
    if update {
        // Keep the legacy connection in the one pool for this read-only safety request.
        let sessions = crate::node::rpc(&host.address, json!({"method":"session.list"}), Duration::from_secs(30))?;
        crate::node::check_update_sessions(&sessions)?;
        stop_if_cancelled(cancelled)?;
        progress(Phase::Copy);
        install_helper(host, true)?;
        progress(Phase::Start);
        crate::node::disconnect(&host.address);
    }
    let compatibility = match crate::node::update_required(&host.address) {
        Ok(value) => value,
        Err(error) if !update => {
            // Only explicit host selection starts setup. Ordinary RPC reconnects never install.
            if error.contains("Incompatible Mindmap node protocol") || (host.ssh.is_none() && host.address != LOCAL) {
                return Err(error);
            }
            stop_if_cancelled(cancelled)?;
            progress(Phase::Copy);
            install_helper(host, false)?;
            stop_if_cancelled(cancelled)?;
            progress(Phase::Start);
            crate::node::update_required(&host.address)?
        }
        Err(error) => return Err(error),
    };
    if let Some(reason) = compatibility {
        return Ok(Outcome::Update(host.clone(), reason));
    }
    // The handshake above proves the node is running on this machine's loopback address.
    progress(Phase::Start);
    stop_if_cancelled(cancelled)?;
    progress(Phase::Herdr);
    let status = crate::node::rpc(&host.address, json!({"method":"host.status"}), Duration::from_secs(30))?;
    if status.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err("The node could not check Herdr setup. Check the node installation and retry.".into());
    }
    if !install && status.get("installed").and_then(|v| v.as_bool()) == Some(false) {
        return Ok(Outcome::Approval(host.clone()));
    }
    stop_if_cancelled(cancelled)?;
    let ready = crate::node::rpc(&host.address, json!({"method":"host.ensure","install_herdr":install}), Duration::from_secs(180))?;
    if ready.get("approval_required").and_then(|v| v.as_bool()) == Some(true) {
        return Ok(Outcome::Approval(host.clone()));
    }
    if ready.get("ok").and_then(|v| v.as_bool()) != Some(true)
        || ready.get("installed").and_then(|v| v.as_bool()) != Some(true)
        || ready.get("running").and_then(|v| v.as_bool()) != Some(true)
    {
        return Err("The node could not prepare its shared Herdr session. Check Herdr setup and retry.".into());
    }
    if let Some(target) = &host.ssh {
        let session = ready.get("session").and_then(|v| v.as_str()).unwrap_or("default");
        stop_if_cancelled(cancelled)?;
        save_machine(target, &host.name, session)?;
    }
    Ok(Outcome::Ready(host.clone()))
}

fn save_machine(target: &str, label: &str, session: &str) -> Result<(), String> {
    if session == "socket_override" {
        return Err("This node uses a custom Herdr socket. Configure a shared default or named Herdr session before adding its SSH machine.".into());
    }
    let listing = run(Command::new(herdr_binary()).args(["machine", "list", "--json"]), TIMEOUT, true)
        .map_err(|_| "Node is ready, but local Herdr is unavailable. Install Herdr locally and retry.".to_owned())?;
    let profiles: serde_json::Value = serde_json::from_str(&listing).map_err(|_| "Could not read saved Herdr machines")?;
    let profiles = profiles.as_array().ok_or("Unrecognized Herdr machine list")?;
    for profile in profiles {
        let saved_target = profile.get("target").and_then(|v| v.as_str()).ok_or("Unrecognized Herdr machine target")?;
        let saved_session = profile.get("session").and_then(|v| v.as_str()).ok_or("Unrecognized Herdr machine session")?;
        if saved_target == target && saved_session == session {
            if profile.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
                let id = profile.get("id").and_then(|v| v.as_str()).ok_or("Saved Herdr machine has no ID")?;
                run(Command::new(herdr_binary()).args(["machine", "enable", id]), TIMEOUT, false)?;
            }
            return Ok(());
        }
    }
    run(Command::new(herdr_binary()).args(["machine", "add", target, "--label", label, "--remote-session", session]), Duration::from_secs(60), false)
        .map_err(|_| {
            "Node is ready, but saving its shared Herdr machine failed. Check local Herdr compatibility and SSH access, then retry.".to_owned()
        })?;
    Ok(())
}

fn hosts_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/mindmap/hosts.json"))
}

fn save_hosts(hosts: &[Host]) -> Result<(), String> {
    let path = hosts_path().ok_or("Home directory unavailable")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| "Could not save hosts")?;
    }
    let bytes = serde_json::to_vec(hosts).map_err(|_| "Could not encode hosts")?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temp, bytes).map_err(|_| "Could not save hosts")?;
    std::fs::rename(temp, path).map_err(|_| "Could not save hosts".to_owned())
}

pub(crate) fn validate_target(target: &str) -> Result<(), String> {
    if target.is_empty()
        || target.starts_with('-')
        || target.len() > 253
        || !target.bytes().all(|b| b.is_ascii_alphanumeric() || b"@._-".contains(&b))
    {
        return Err("Enter an SSH config alias or user@hostname. Configure custom ports in SSH config.".into());
    }
    Ok(())
}

fn ssh(target: &str) -> Command {
    let mut command = Command::new("ssh");
    command.args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-o", "StrictHostKeyChecking=yes", target]);
    command
}

/// Called by the persistent RPC worker, never directly by an egui frame.
/// The caller owns the tunnel and must kill and reap it when discarding the connection.
pub fn connect(host: &Host) -> Result<(TcpStream, Option<Child>), String> {
    if let Some(target) = &host.ssh {
        validate_target(target)?;
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| "Could not allocate tunnel port")?;
        let port = listener.local_addr().map_err(|_| "Could not read tunnel port")?.port();
        drop(listener);
        let mut command = Command::new("ssh");
        command.args([
            "-N",
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "ExitOnForwardFailure=yes",
            "-o",
            "ServerAliveInterval=15",
            "-o",
            "ServerAliveCountMax=3",
            "-L",
            &format!("127.0.0.1:{port}:127.0.0.1:47822"),
            target,
        ]);
        let mut child = command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|_| "Could not start SSH tunnel")?;
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        let deadline = Instant::now() + TIMEOUT;
        while Instant::now() < deadline {
            match child.try_wait() {
                Ok(Some(_)) => return Err("SSH tunnel failed; verify SSH access and the host key in a terminal".into()),
                Ok(None) => {}
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("Could not check SSH tunnel".into());
                }
            }
            if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_millis(200)) {
                return Ok((stream, Some(child)));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
        return Err("SSH tunnel did not become ready".into());
    }
    let address: SocketAddr = host.address.parse().map_err(|_| "Expected a numeric loopback node address")?;
    if !address.ip().is_loopback() {
        return Err("Remote nodes require an SSH host; direct connections must use loopback.".into());
    }
    if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_secs(3)) {
        return Ok((stream, None));
    }
    if host.address != LOCAL {
        return Err("Could not connect to node".into());
    }
    Err("Local node is unavailable. Select This computer to install or start its supervised helper.".into())
}

fn platform(os: &str, arch: &str) -> Result<String, String> {
    let os = match os.trim() {
        "Darwin" | "macos" => "darwin",
        "Linux" | "linux" => "linux",
        _ => return Err("This host operating system is unsupported".into()),
    };
    let arch = match arch.trim() {
        "arm64" | "aarch64" => "arm64",
        "x86_64" | "amd64" => "x64",
        _ => return Err("This host CPU is unsupported".into()),
    };
    Ok(format!("{os}-{arch}"))
}

fn resources() -> Result<PathBuf, String> {
    if let Ok(exe) = std::env::current_exe()
        && let Some(contents) = exe.parent().and_then(|p| p.parent())
    {
        let path = contents.join("Resources");
        if path.join("install-node.sh").is_file() {
            return Ok(path);
        }
    }
    Err("Install a complete Mindmap app bundle containing its node and portable installer.".into())
}

fn validate_asset_info(info: &serde_json::Value, platform: &str) -> Result<(), String> {
    if info.get("build_id").and_then(|v| v.as_str()) != Some(mindmap_build::BUILD_ID)
        || info.get("protocol").and_then(|v| v.as_u64()) != Some(crate::node::PROTOCOL)
        || info.get("version").and_then(|v| v.as_str()) != Some(mindmap_build::VERSION)
        || info.get("platform").and_then(|v| v.as_str()) != Some(platform)
    {
        return Err("The bundled node does not match this app build. Install a complete matching Mindmap app bundle.".into());
    }
    Ok(())
}

fn bundled_node(resources: &std::path::Path, platform: &str) -> Result<PathBuf, String> {
    let directory = resources.join("node").join(platform);
    let info = std::fs::read(directory.join("build-info.json")).map_err(|_| "The bundled node is missing its build identity")?;
    let info: serde_json::Value = serde_json::from_slice(&info).map_err(|_| "Invalid bundled node build identity")?;
    validate_asset_info(&info, platform)?;
    let binary = directory.join("mindmap-node");
    if !binary.is_file() || !resources.join("install-node.py").is_file() {
        return Err(format!("The app does not include a complete node installer for {platform}."));
    }
    validate_asset_hash(&binary, &info)?;
    Ok(binary)
}

fn validate_asset_hash(binary: &std::path::Path, info: &serde_json::Value) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(binary).map_err(|_| "Could not read bundled node")?;
    let mut hash = Sha256::new();
    let mut bytes = [0_u8; 65536];
    loop {
        let count = file.read(&mut bytes).map_err(|_| "Could not verify bundled node")?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    if info.get("sha256").and_then(|v| v.as_str()) != Some(format!("{:x}", hash.finalize()).as_str()) {
        return Err("The bundled node checksum is missing or incorrect. Reinstall the complete Mindmap app bundle.".into());
    }
    Ok(())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn remote_install_command(stage: &str, update: bool) -> String {
    format!(
        "scope=user; if [ \"$(uname -s)\" = Linux ] && [ \"$(id -u)\" = 0 ]; then scope=system; fi; sh {stage}/install-node.sh --binary {stage}/mindmap-node --build-id {} --scope \"$scope\"{}",
        shell_quote(mindmap_build::BUILD_ID),
        if update { " --upgrade" } else { "" }
    )
}

fn validate_stage(stage: &str) -> Result<(), String> {
    let path = std::path::Path::new(stage);
    let valid_name = path
        .file_name()
        .and_then(|v| v.to_str())
        .and_then(|v| v.strip_prefix("install."))
        .is_some_and(|suffix| suffix.len() >= 8 && suffix.bytes().all(|b| b.is_ascii_alphanumeric()));
    if !path.is_absolute()
        || !valid_name
        || !path.parent().is_some_and(|p| p.ends_with(".cache/mindmap"))
        || stage.contains(['\n', '\r'])
        || path.components().any(|part| part == std::path::Component::ParentDir)
    {
        return Err("Invalid remote installer staging path".into());
    }
    Ok(())
}

fn install_helper(host: &Host, update: bool) -> Result<(), String> {
    let resources = resources()?;
    if let Some(target) = &host.ssh {
        validate_target(target)?;
        let output = run(ssh(target).arg("uname -s; uname -m"), TIMEOUT, true)?;
        let mut lines = output.lines();
        let platform = platform(lines.next().unwrap_or(""), lines.next().unwrap_or(""))?;
        let binary = bundled_node(&resources, &platform)?;
        // mktemp owns a unique private staging directory; no shell interpolation of SSH output.
        let stage = run(
            ssh(target).arg("umask 077; mkdir -p \"$HOME/.cache/mindmap\" && mktemp -d \"$HOME/.cache/mindmap/install.XXXXXXXX\""),
            TIMEOUT,
            true,
        )?;
        let stage = stage.trim();
        validate_stage(stage)?;
        let quoted = shell_quote(stage);
        let result = (|| {
            for source in [binary, resources.join("install-node.sh"), resources.join("install-node.py")] {
                let filename = source.file_name().and_then(|v| v.to_str()).ok_or("Invalid installer filename")?;
                let mut copy = Command::new("scp");
                // Legacy SCP interprets shell-quoted paths; SFTP treats the quotes literally.
                copy.args(["-O", "-q", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes", "-o", "ConnectTimeout=10"]);
                copy.arg(&source).arg(format!("{target}:{}/{}", shell_quote(stage), filename));
                run(&mut copy, Duration::from_secs(120), false)?;
            }
            run(ssh(target).arg(remote_install_command(&quoted, update)), Duration::from_secs(120), true)?;
            Ok(())
        })();
        let _ = run(ssh(target).arg(format!("rm -rf -- {quoted}")), TIMEOUT, false);
        result
    } else {
        if host.address != LOCAL {
            return Err("Automatic helper installation requires the default local node address.".into());
        }
        let platform = platform(std::env::consts::OS, std::env::consts::ARCH)?;
        let binary = bundled_node(&resources, &platform)?;
        let mut command = Command::new("sh");
        command.arg(resources.join("install-node.sh")).arg("--binary").arg(binary).args(["--build-id", mindmap_build::BUILD_ID]);
        if std::env::consts::OS == "linux" && run(Command::new("id").arg("-u"), TIMEOUT, true)?.trim() == "0" {
            command.args(["--scope", "system"]);
        }
        if update {
            command.arg("--upgrade");
        }
        run(&mut command, Duration::from_secs(120), true)?;
        Ok(())
    }
}

fn herdr_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("HERDR_BIN_PATH").filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }
    let mut candidates = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        candidates.push(home.join(".local/bin/herdr"));
        candidates.push(home.join(".cargo/bin/herdr"));
    }
    candidates.extend([PathBuf::from("/opt/homebrew/bin/herdr"), PathBuf::from("/usr/local/bin/herdr")]);
    candidates.into_iter().find(|path| path.is_file()).unwrap_or_else(|| PathBuf::from("herdr"))
}

/// Do not surface subprocess output: SSH diagnostics can contain private configuration.
fn run(command: &mut Command, timeout: Duration, capture: bool) -> Result<String, String> {
    command.stdin(Stdio::null()).stderr(Stdio::null());
    command.stdout(if capture { Stdio::piped() } else { Stdio::null() });
    let mut child = command.spawn().map_err(|_| "Could not start host setup command")?;
    let reader = child.stdout.take().map(|stdout| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout.take(1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|_| "Could not read host setup result")?;
            if bytes.len() > 1024 * 1024 {
                return Err("Host setup result was too large");
            }
            Ok(bytes)
        })
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let bytes = match reader {
                    Some(reader) => reader.join().map_err(|_| "Could not read host setup result")??,
                    None => Vec::new(),
                };
                return Ok(String::from_utf8_lossy(&bytes).into_owned());
            }
            Ok(Some(status)) if status.code() == Some(20) => {
                return Err(
                    "Update node: an installed helper uses another build. Start that helper, select its host, and approve Update node.".into()
                );
            }
            Ok(Some(status)) if status.code() == Some(21) => {
                return Err(
                    "Update pending: session safety could not be confirmed. Finish prepared Sync jobs and pending merges, then retry Update node."
                        .into(),
                );
            }
            Ok(Some(status)) if status.code() == Some(22) => return Err(
                "The running node is not owned by the selected service manager. Stop or migrate that installation explicitly, then retry host setup."
                    .into(),
            ),
            Ok(Some(_)) => return Err("Host setup command failed. Verify SSH access and host keys in a terminal.".into()),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Host setup command timed out".into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    fn temporary_directory() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "mindmap helper's test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn remote_stage_cleanup_is_confined_to_a_private_installer_directory() {
        for invalid in [
            "/",
            "/tmp/install.12345678",
            "/home/me/.cache/mindmap/..",
            "/home/me/.cache/mindmap/install.x",
            "/home/me/.cache/mindmap/install.12345678\n",
            "/home/../.cache/mindmap/install.12345678",
        ] {
            assert!(validate_stage(invalid).is_err(), "{invalid}");
        }
        validate_stage("/Users/Alice O'Brien/.cache/mindmap/install.12ABcd34").unwrap();
    }

    #[test]
    fn asset_validation_rejects_other_builds_platforms_and_corrupted_bytes() {
        use sha2::{Digest, Sha256};
        let directory = temporary_directory();
        let binary = directory.join("mindmap-node");
        std::fs::write(&binary, b"synthetic binary").unwrap();
        let info = json!({"build_id":mindmap_build::BUILD_ID,"protocol":crate::node::PROTOCOL,"version":mindmap_build::VERSION,"platform":"darwin-arm64","sha256":format!("{:x}",Sha256::digest(b"synthetic binary"))});
        validate_asset_info(&info, "darwin-arm64").unwrap();
        validate_asset_hash(&binary, &info).unwrap();
        assert!(validate_asset_info(&info, "linux-x64").is_err());
        for key in ["build_id", "protocol", "version", "platform"] {
            let mut invalid = info.clone();
            invalid[key] = json!("wrong");
            assert!(validate_asset_info(&invalid, "darwin-arm64").is_err(), "{key}");
        }
        std::fs::write(&binary, b"modified binary").unwrap();
        assert!(validate_asset_hash(&binary, &info).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn bootstrap_passes_quoted_paths_and_requires_explicit_upgrade_flag() {
        let directory = temporary_directory();
        std::fs::write(directory.join("install-node.sh"), "printf '%s\\n' \"$@\"\n").unwrap();
        for upgrade in [false, true] {
            let script = remote_install_command(&shell_quote(directory.to_str().unwrap()), upgrade);
            let output = run(Command::new("sh").args(["-c", &script]), TIMEOUT, true).unwrap();
            let args: Vec<_> = output.lines().collect();
            assert_eq!(args[0], "--binary");
            assert_eq!(args[1], directory.join("mindmap-node").to_str().unwrap());
            assert_eq!(args[2], "--build-id");
            assert_eq!(args[3], mindmap_build::BUILD_ID);
            assert_eq!(args[4], "--scope");
            assert!(matches!(args[5], "system" | "user"));
            assert_eq!(args.contains(&"--upgrade"), upgrade);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn installer_refusals_are_actionable_errors() {
        for (code, reason) in [(20, "Update node"), (21, "Update pending"), (22, "not owned")] {
            let error = run(Command::new("sh").args(["-c", &format!("exit {code}")]), TIMEOUT, false).unwrap_err();
            assert!(error.contains(reason), "{error}");
        }
    }

    #[test]
    fn host_selection_offers_update_without_contacting_herdr_or_installing() {
        use std::io::{BufRead, BufReader, Write};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = Host { name: "Synthetic node".into(), address: listener.local_addr().unwrap().to_string(), ssh: None };
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], "hello");
            writeln!(
                stream,
                "{}",
                json!({"id":request["id"],"version":crate::node::PROTOCOL,"protocol":crate::node::PROTOCOL,"ok":true,"build_id":"old"})
            )
            .unwrap();
            line.clear();
            assert_eq!(reader.read_line(&mut line).unwrap(), 0, "selection must stop for approval before host.ensure or installation");
        });
        assert!(matches!(prepare(&host, false, false).unwrap(), Outcome::Update(_, reason) if reason.contains("Update node")));
        crate::node::disconnect(&host.address);
        server.join().unwrap();
    }

    #[test]
    fn approved_update_stops_before_installer_when_sessions_are_prepared() {
        use std::io::{BufRead, BufReader, Write};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = Host { name: "Synthetic node".into(), address: listener.local_addr().unwrap().to_string(), ssh: None };
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            for method in ["hello", "session.list"] {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                assert_eq!(request["method"], method);
                writeln!(stream, "{}", json!({"id":request["id"],"version":crate::node::PROTOCOL,"protocol":crate::node::PROTOCOL,"ok":true,"build_id":"old","sessions":[{"syncPrepared":true,"mergeReady":false}]})).unwrap();
            }
            assert_eq!(reader.read(&mut [0]).unwrap(), 0);
        });
        let error = prepare(&host, false, true).err().unwrap();
        assert!(error.contains("Update pending"), "{error}");
        crate::node::disconnect(&host.address);
        server.join().unwrap();
    }

    #[test]
    fn ssh_target_cannot_inject_options_or_shell_syntax() {
        for bad in ["", "-oProxyCommand=bad", "host;touch x", "user@host\n", "$(id)", "host:22", "host/other"] {
            assert!(validate_target(bad).is_err(), "{bad}");
        }
        for good in ["befree", "alice@work.example", "192.0.2.10", "my-host_2"] {
            assert!(validate_target(good).is_ok());
        }
    }

    #[test]
    fn remote_direct_connections_are_rejected() {
        let host = Host { name: "Remote".into(), address: "192.0.2.1:47822".into(), ssh: None };
        assert!(connect(&host).err().is_some_and(|error| error.contains("require an SSH host")));
    }

    #[test]
    fn existing_direct_node_needs_no_child_process() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener");
        let host = Host { name: "Synthetic node".into(), address: listener.local_addr().expect("test address").to_string(), ssh: None };
        let (stream, tunnel) = connect(&host).expect("connect to existing node");
        assert!(tunnel.is_none());
        assert_eq!(stream.peer_addr().expect("peer"), listener.local_addr().expect("address"));
    }

    #[test]
    fn platform_detection_matches_bundle_layout() {
        assert_eq!(platform("Darwin", "aarch64").ok().as_deref(), Some("darwin-arm64"));
        assert_eq!(platform("Linux", "x86_64").ok().as_deref(), Some("linux-x64"));
        assert!(platform("Linux", "riscv64").is_err());
    }
}
