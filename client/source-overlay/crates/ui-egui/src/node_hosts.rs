//! Host setup and transport. Network and process work always runs outside the UI thread.
use std::collections::HashMap;
use std::io::Read;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock, mpsc};
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

enum Outcome {
    Ready(Host),
    Approval(Host),
    Failed(String),
}

#[derive(Default)]
pub struct State {
    pub selected: Option<Host>,
    hosts: Vec<Host>,
    target: String,
    name: String,
    pending: Option<mpsc::Receiver<Outcome>>,
    approval: Option<Host>,
    error: Option<String>,
    loaded: bool,
}

/// Returns a host only after its node and shared Herdr session are ready.
pub fn show(ui: &mut egui::Ui, state: &mut State) -> Option<Host> {
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
                if !state.hosts.iter().any(|h| h.address == host.address) {
                    state.hosts.push(host.clone());
                }
                if let Err(error) = save_hosts(&state.hosts) {
                    state.error = Some(error);
                }
                state.selected = Some(host.clone());
                state.approval = None;
                return Some(host);
            }
            Outcome::Approval(host) => state.approval = Some(host),
            Outcome::Failed(error) => state.error = Some(error),
        }
    }
    ui.horizontal_wrapped(|ui| {
        ui.label("Host");
        ui.add_enabled_ui(state.pending.is_none() && state.approval.is_none(), |ui| {
            for host in state.hosts.clone() {
                let selected = state.selected.as_ref().is_some_and(|h| h.address == host.address);
                if ui.selectable_label(selected, &host.name).clicked() {
                    start(state, host, false);
                }
            }
        });
    });
    ui.add_enabled_ui(state.pending.is_none() && state.approval.is_none(), |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::TextEdit::singleline(&mut state.target).hint_text("SSH host or user@host").desired_width(180.0));
            ui.add(egui::TextEdit::singleline(&mut state.name).hint_text("Display name").desired_width(120.0));
            if ui.button("Add host").clicked() {
                let target = state.target.trim().to_owned();
                match validate_target(&target) {
                    Ok(()) => {
                        let name = if state.name.trim().is_empty() { target.clone() } else { state.name.trim().to_owned() };
                        start(state, Host { name, address: format!("ssh:{target}"), ssh: Some(target) }, false);
                    }
                    Err(error) => state.error = Some(error),
                }
            }
        });
    });
    if let Some(host) = state.approval.clone() {
        ui.label(format!("Herdr is missing on {}. Install it to run agents?", host.name));
        ui.horizontal(|ui| {
            if ui.button("Install Herdr").clicked() {
                state.approval = None;
                start(state, host, true);
            }
            if ui.button("Cancel").clicked() {
                state.approval = None;
            }
        });
    }
    if state.pending.is_some() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Preparing host…");
        });
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
    if let Some(error) = &state.error {
        ui.label(error);
    }
    None
}

/// Start host setup without granting permission to install Herdr.
pub fn select(state: &mut State, host: Host) {
    if state.pending.is_none() {
        state.approval = None;
        start(state, host, false);
    }
}

fn start(state: &mut State, host: Host, install: bool) {
    state.error = None;
    let (tx, rx) = mpsc::channel();
    state.pending = Some(rx);
    std::thread::spawn(move || {
        let result = prepare(&host, install).unwrap_or_else(Outcome::Failed);
        let _ = tx.send(result);
    });
}

fn prepare(host: &Host, install: bool) -> Result<Outcome, String> {
    register(host)?;
    let status = crate::node::rpc(&host.address, json!({"method":"host.status"}), Duration::from_secs(30))?;
    if status.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err("The node could not check Herdr setup. Check the node installation and retry.".into());
    }
    if !install && status.get("installed").and_then(|v| v.as_bool()) == Some(false) {
        return Ok(Outcome::Approval(host.clone()));
    }
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

fn validate_target(target: &str) -> Result<(), String> {
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
        bootstrap_remote(target)?;
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
    let platform = platform(std::env::consts::OS, std::env::consts::ARCH)?;
    let binary = bundled_node(&platform)?;
    let mut command = Command::new(binary);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command
        .args(["serve", "--bind", LOCAL])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Could not start bundled node")?;
    // Reap the server after it eventually exits without tying its lifetime to a UI connection.
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    let deadline = Instant::now() + TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_millis(200)) {
            return Ok((stream, None));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("Local node did not become ready".into())
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

fn bundled_node(platform: &str) -> Result<PathBuf, String> {
    let relative = PathBuf::from("node").join(platform).join("mindmap-node");
    if let Ok(exe) = std::env::current_exe()
        && let Some(contents) = exe.parent().and_then(|p| p.parent())
    {
        let path = contents.join("Resources").join(&relative);
        if path.is_file() {
            return Ok(path);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let path = PathBuf::from(home).join("Applications/Mindmap.app/Contents/Resources").join(relative);
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(format!("The app does not include a node for {platform}. Install a complete Mindmap app bundle."))
}

fn bootstrap_remote(target: &str) -> Result<(), String> {
    let output = run(ssh(target).arg("uname -s; uname -m"), TIMEOUT, true)?;
    let mut lines = output.lines();
    let platform = platform(lines.next().unwrap_or(""), lines.next().unwrap_or(""))?;
    // Reuse a running installation. Updates are coordinated separately to avoid stopping agents.
    if run(ssh(target).arg("test -x \"$HOME/.local/bin/mindmap-node\""), TIMEOUT, false).is_err() {
        let binary = bundled_node(&platform)?;
        run(ssh(target).arg("mkdir -p \"$HOME/.local/bin\""), TIMEOUT, false)?;
        let remote = format!("{target}:.local/bin/mindmap-node.{}.tmp", std::process::id());
        let mut copy = Command::new("scp");
        copy.args(["-q", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes", "-o", "ConnectTimeout=10"]);
        copy.arg(binary).arg(remote);
        run(&mut copy, Duration::from_secs(120), false)?;
        run(ssh(target).arg(format!("chmod 755 \"$HOME/.local/bin/mindmap-node.{0}.tmp\" && mv \"$HOME/.local/bin/mindmap-node.{0}.tmp\" \"$HOME/.local/bin/mindmap-node\"", std::process::id())), TIMEOUT, false)?;
    }
    run(ssh(target).arg("nohup \"$HOME/.local/bin/mindmap-node\" serve --bind 127.0.0.1:47822 </dev/null >/dev/null 2>&1 &"), TIMEOUT, false)?;
    // The tunnel opens next; leave a short interval for the remote listener to start.
    std::thread::sleep(Duration::from_millis(300));
    Ok(())
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
    use super::*;

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
