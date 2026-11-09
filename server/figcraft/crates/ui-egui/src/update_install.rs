//! Download, verify and install a release ([`crate::update`]) in the background.
//!
//! macOS only: the app archive is downloaded with the system `curl`, checked against the feed's
//! SHA-256, unpacked with `ditto` and checked with `codesign --verify --deep --strict`; the node
//! binary is checked against its SHA-256. Only then, and only if no Sync is prepared and no merge
//! is pending, the new bundle's `install-node.sh --upgrade` replaces the node (its exit 21 means
//! the same wait), the `.app` is swapped for the new one (two renames on one volume, rolled back
//! if the second fails) and a helper reopens it once this process has quit. Herdr and the agents
//! are separate processes and keep running.
//!
//! Not yet exercised end to end: no release feed exists.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use crate::update::{Installed, LOCAL_NODE, Release};

/// The largest feed read.
const MAX_FEED: &str = "1048576";

/// A running install.
pub(crate) struct Job {
    rx: mpsc::Receiver<Installed>,
    cancelled: Arc<AtomicBool>,
}

impl Job {
    /// How the install ended, once it has.
    pub(crate) fn outcome(&self) -> Option<Installed> {
        match self.rx.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Installed::Failed("The installer stopped unexpectedly.".into())),
        }
    }

    /// Stop before anything is installed (checked between steps; the node install and the swap
    /// are not interrupted).
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

/// Start installing `release`.
pub(crate) fn begin(release: Release) -> Job {
    let (tx, rx) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    std::thread::spawn(move || {
        let _ = tx.send(install(&release, &flag));
    });
    Job { rx, cancelled }
}

/// The feed's text, with the system `curl` (HTTPS only, or a local `file://` feed).
pub(crate) fn fetch_text(url: &str) -> Result<String, String> {
    let output = curl(url, &["--max-time", "30", "--max-filesize", MAX_FEED])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| "Could not run curl to read the release feed.".to_string())?;
    if !output.status.success() {
        return Err("Could not read the release feed. Check the connection and try again.".into());
    }
    String::from_utf8(output.stdout).map_err(|_| "The release feed is not text.".into())
}

/// `curl` for `url` with `options` (the URL goes last, after `--`, so it is never an option).
pub(crate) fn curl(url: &str, options: &[&str]) -> Command {
    let mut command = Command::new("curl");
    command.args(["-fsSL", "--proto", "=https,file", "--proto-redir", "=https"]).args(options).arg("--").arg(url);
    command
}

fn download(url: &str, to: &Path) -> Result<(), String> {
    let to = to.to_str().ok_or("The download folder's path isn't text.")?;
    let status = curl(url, &["--max-time", "1800", "-o", to])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| "Could not run curl to download the update.".to_string())?;
    if status.success() { Ok(()) } else { Err("The download failed. Check the connection and try again.".into()) }
}

/// The file's SHA-256 as lower-case hex.
pub(crate) fn sha256_file(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|_| "Could not read the download.".to_string())?;
    let mut hash = Sha256::new();
    let mut bytes = vec![0_u8; 1 << 16];
    loop {
        let count = file.read(&mut bytes).map_err(|_| "Could not read the download.".to_string())?;
        if count == 0 {
            break;
        }
        hash.update(bytes.get(..count).unwrap_or_default());
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// Download `url` to `to` and check it against `sha256`.
fn fetch_verified(url: &str, sha256: &str, to: &Path, what: &str) -> Result<(), String> {
    download(url, to)?;
    if !sha256_file(to)?.eq_ignore_ascii_case(sha256) {
        return Err(format!("The {what} download doesn't match the release's checksum; nothing was installed."));
    }
    Ok(())
}

/// The `.app` this process runs from (`…/Afterburn.app/Contents/MacOS/<exe>`).
fn running_bundle() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|_| "Could not find the running app.".to_string())?;
    let exe = exe.canonicalize().unwrap_or(exe);
    exe.ancestors()
        .nth(3)
        .filter(|bundle| bundle.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf)
        .ok_or_else(|| "Afterburn isn't running from an app bundle, so it can't replace itself. Install the release by hand.".into())
}

fn stop_if_cancelled(cancelled: &AtomicBool) -> Result<(), Installed> {
    if cancelled.load(Ordering::SeqCst) { Err(Installed::Failed("Cancelled. Nothing was installed.".into())) } else { Ok(()) }
}

fn install(release: &Release, cancelled: &AtomicBool) -> Installed {
    if !cfg!(target_os = "macos") {
        return Installed::Failed("Updates install from the Afterburn app on a Mac.".into());
    }
    let bundle = match running_bundle() {
        Ok(bundle) => bundle,
        Err(error) => return Installed::Failed(error),
    };
    let Some(parent) = bundle.parent().map(Path::to_path_buf) else {
        return Installed::Failed("Could not find the app's folder.".into());
    };
    // Staged beside the app so the swap is a rename on one volume.
    let staging = parent.join(format!(".afterburn-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    if std::fs::create_dir(&staging).is_err() {
        return Installed::Failed(format!("Afterburn can't write to {}. Move the app to a folder you own, then try again.", parent.display()));
    }
    let outcome = staged(release, &bundle, &staging, cancelled);
    if outcome != Installed::Restarting {
        let _ = std::fs::remove_dir_all(&staging);
    }
    outcome
}

fn staged(release: &Release, bundle: &Path, staging: &Path, cancelled: &AtomicBool) -> Installed {
    let result = (|| -> Result<Installed, Installed> {
        let failed = Installed::Failed;
        let archive = staging.join("app.zip");
        fetch_verified(&release.app.url, &release.app.sha256, &archive, "app").map_err(failed)?;
        stop_if_cancelled(cancelled)?;
        let node = staging.join("afterburn-node");
        fetch_verified(&release.node.url, &release.node.sha256, &node, "node").map_err(failed)?;
        set_executable(&node).map_err(failed)?;
        stop_if_cancelled(cancelled)?;
        let unpacked = staging.join("app");
        run(Command::new("ditto").args(["-x", "-k"]).arg(&archive).arg(&unpacked), "unpack the app")?;
        let new_app = single_app(&unpacked).map_err(failed)?;
        run(Command::new("codesign").args(["--verify", "--deep", "--strict"]).arg(&new_app), "verify the app's signature")?;
        stop_if_cancelled(cancelled)?;
        // A prepared Sync or a pending merge would not survive the node restarting.
        let sessions =
            crate::node::rpc(LOCAL_NODE, serde_json::json!({"method":"session.list"}), Duration::from_secs(30)).map_err(Installed::Waits)?;
        crate::node::check_update_sessions(&sessions).map_err(Installed::Waits)?;
        stop_if_cancelled(cancelled)?;
        install_node(&new_app, bundle, &node, &release.node.build_id)?;
        crate::node::disconnect(LOCAL_NODE);
        swap(bundle, &new_app, staging).map_err(failed)?;
        relaunch(bundle, staging).map_err(failed)?;
        Ok(Installed::Restarting)
    })();
    result.unwrap_or_else(|outcome| outcome)
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(|_| "Could not prepare the node binary.".into())
}

#[cfg(not(unix))]
fn set_executable(_: &Path) -> Result<(), String> {
    Ok(())
}

/// The one `.app` an archive unpacked to.
fn single_app(dir: &Path) -> Result<PathBuf, String> {
    let apps: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|_| "Could not read the unpacked app.".to_string())?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|e| e == "app") && path.is_dir())
        .collect();
    match apps.as_slice() {
        [app] => Ok(app.clone()),
        _ => Err("The app download doesn't hold exactly one app.".into()),
    }
}

/// Run a step; a failure names it.
fn run(command: &mut Command, step: &str) -> Result<(), Installed> {
    let status = command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    match status {
        Ok(status) if status.success() => Ok(()),
        _ => Err(Installed::Failed(format!("Could not {step}; nothing was installed."))),
    }
}

/// The new bundle's installer (else this one's) replaces the node: `--upgrade`, its exit 21 is
/// the same wait as a prepared Sync.
fn install_node(new_app: &Path, bundle: &Path, node: &Path, build_id: &str) -> Result<(), Installed> {
    let installer = [new_app, bundle]
        .iter()
        .map(|app| app.join("Contents/Resources/install-node.sh"))
        .find(|path| path.is_file())
        .ok_or_else(|| Installed::Failed("The app has no node installer; nothing was installed.".into()))?;
    let status = Command::new("sh")
        .arg(&installer)
        .arg("--binary")
        .arg(node)
        .args(["--build-id", build_id, "--upgrade"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| Installed::Failed("Could not run the node installer; nothing was installed.".into()))?;
    installer_outcome(status.code())
}

/// What the node installer's exit status means.
pub(crate) fn installer_outcome(code: Option<i32>) -> Result<(), Installed> {
    match code {
        Some(0) => Ok(()),
        Some(21) => {
            Err(Installed::Waits("A prepared Sync or a pending merge is open. Finish or cancel it on every workspace, then try again.".into()))
        }
        Some(22) => Err(Installed::Failed(
            "The running node isn't managed by Afterburn's service, so it wasn't replaced. Stop or migrate it, then try again.".into(),
        )),
        _ => Err(Installed::Failed("The node installer failed; the app was not replaced.".into())),
    }
}

/// Put `new_app` where `bundle` is: move the old bundle aside into `staging`, the new one in;
/// the old one comes back if the second rename fails.
fn swap(bundle: &Path, new_app: &Path, staging: &Path) -> Result<(), String> {
    let old = staging.join("previous.app");
    std::fs::rename(bundle, &old).map_err(|_| "The node is updated but the app couldn't be replaced. Install the release by hand.".to_string())?;
    if std::fs::rename(new_app, bundle).is_err() {
        let _ = std::fs::rename(&old, bundle);
        return Err("The node is updated but the app couldn't be replaced. Install the release by hand.".into());
    }
    Ok(())
}

/// A detached helper waits for this process to quit, removes the staging folder (with the old
/// bundle) and opens the new app.
fn relaunch(bundle: &Path, staging: &Path) -> Result<(), String> {
    let script = "while kill -0 \"$1\" 2>/dev/null; do sleep 0.2; done; rm -rf -- \"$2\"; open -- \"$3\"";
    Command::new("sh")
        .args(["-c", script, "afterburn-relaunch"])
        .arg(std::process::id().to_string())
        .arg(staging)
        .arg(bundle)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|_| "The update is installed; open Afterburn again to finish.".into())
}
