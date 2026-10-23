//! One-time, explicit migration of legacy hosted documents. Tokens are read privately and never
//! copied into repository files, node state or diagnostics. Existing repository designs win.
use crate::error::{Error, Result};
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, Instant};

pub fn migrate(address: &str, records: &Path) -> Result<()> {
    let config = crate::config::Config::discover()?;
    let values: Value = serde_json::from_slice(&std::fs::read(records)?)?;
    let values = values
        .as_object()
        .ok_or_else(|| Error::new("legacy records must be an object"))?;
    for (workspace, record) in values {
        let Some(path) = record["path"].as_str().filter(|p| !p.is_empty()) else {
            continue;
        };
        let Some(link) = record["edit"].as_str().filter(|s| !s.is_empty()) else {
            continue;
        };
        let root = Path::new(path);
        if !root.is_dir() {
            return Err(Error::new(format!(
                "workspace {workspace} checkout is unavailable"
            )));
        }
        if root.join(".mindmap/map.figcraft").exists() {
            println!(
                "{}",
                json!({"workspace":workspace,"state":"kept_existing_repository_design"})
            );
            continue;
        }
        let mut client = crate::mcp::FigcraftSession::start(&config.figcraft_cli)?;
        client
            .run_command("collab.join", json!({"link":link}))
            .map_err(|_| Error::new(format!("cannot join legacy document for {workspace}")))?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let state = client
                .run_command("collab.peers", json!({}))
                .map_err(|_| Error::new("cannot inspect legacy session"))?;
            if state["status"] == "live" {
                break;
            }
            if Instant::now() >= deadline {
                return Err(Error::new(format!(
                    "legacy document for {workspace} did not connect"
                )));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["rev-parse", "--absolute-git-dir"])
            .output()?;
        if !output.status.success() {
            return Err(Error::new("migration target must be a Git worktree"));
        }
        let cache = std::path::PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
            .join("mindmap-migration");
        std::fs::create_dir_all(&cache)?;
        let export = cache.join("legacy.figcraft");
        client
            .call_tool("save_file", json!({"path":export}))
            .map_err(|_| Error::new(format!("cannot export legacy document for {workspace}")))?;
        let opened = super::request(
            address,
            json!({"method":"open","path":root,"workspace":workspace}),
        )?;
        if opened["ok"] != true {
            return Err(Error::new("node refused migration target"));
        }
        let imported = super::request(
            address,
            json!({"method":"session.import","session":opened["session"],"source":export}),
        )?;
        if imported["ok"] != true {
            return Err(Error::new("node refused exported legacy design"));
        }
        let (before, _) =
            figcraft_format::load_file(&export).map_err(|e| Error::new(e.to_string()))?;
        let (after, _) = figcraft_format::load_file(&root.join(".mindmap/map.figcraft"))
            .map_err(|e| Error::new(e.to_string()))?;
        let drift = figcraft_collab::diff(&figcraft_collab::Replica::new(before), &after)
            .map_err(|e| Error::new(e.to_string()))?;
        if !drift.changes.is_empty() {
            return Err(Error::new("migrated document verification failed"));
        }
        println!(
            "{}",
            json!({"workspace":workspace,"state":"migrated","pages":after.pages.len(),"nodes":after.nodes().count()})
        );
    }
    Ok(())
}
