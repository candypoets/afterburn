//! What an agent needs before Init can ask it for a map: the mindmap skill and the node's MCP
//! server, installed for each agent CLI on this machine.
//!
//! The skill ships inside the node, so it is always the version the node speaks. The MCP entry
//! is user-scoped and points at this node binary with no `--path`: the adapter opens the folder
//! the agent runs in, which Herdr sets to the worktree. A running agent loaded its MCP servers
//! when it started, so after an install Init starts a new agent rather than prompting one.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::error::{Error, Result};

/// The skill, compiled in from `skills/mindmap/SKILL.md` (part of the build id; Mac deploys copy it).
const SKILL: &str = include_str!("../../skills/mindmap/SKILL.md");

/// The MCP server's name in each CLI's configuration.
const SERVER: &str = "mindmap";

/// The address `mindmap-node` reaches without `--address`.
const DEFAULT_ADDRESS: &str = "127.0.0.1:47822";

/// The agent CLIs whose skill and MCP setup the node manages, and where each reads its skills.
const CLIS: [(&str, &str); 2] = [("claude", ".claude/skills"), ("codex", ".agents/skills")];

/// One CLI's setup on this machine.
struct Cli {
    kind: &'static str,
    skill: PathBuf,
}

fn clis(home: &Path) -> Vec<Cli> {
    CLIS.iter()
        .filter(|(kind, _)| crate::agent::find_in_path(kind).is_some())
        .map(|(kind, dir)| Cli {
            kind,
            skill: home.join(dir).join(SERVER).join("SKILL.md"),
        })
        .collect()
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

/// The command line every MCP entry runs: this binary, `mcp`, and the node's address when it is
/// not the default.
fn mcp_command(address: &str) -> Result<Vec<String>> {
    let exe = std::env::current_exe()
        .map_err(|error| Error::new(format!("cannot find the node binary: {error}")))?;
    let mut command = vec![exe.to_string_lossy().into_owned(), "mcp".to_owned()];
    if address != DEFAULT_ADDRESS {
        command.extend(["--address".to_owned(), address.to_owned()]);
    }
    Ok(command)
}

/// Whether `kind` has an MCP entry named `mindmap` that runs exactly `command`.
fn mcp_registered(home: &Path, kind: &str, command: &[String]) -> bool {
    match kind {
        "claude" => std::fs::read_to_string(home.join(".claude.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .is_some_and(|config| {
                let entry = &config["mcpServers"][SERVER];
                let args: Vec<&str> = entry["args"]
                    .as_array()
                    .map(|args| args.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                entry["command"].as_str() == command.first().map(String::as_str)
                    && args
                        == command
                            .get(1..)
                            .unwrap_or_default()
                            .iter()
                            .map(String::as_str)
                            .collect::<Vec<_>>()
            }),
        "codex" => std::fs::read_to_string(home.join(".codex/config.toml"))
            .ok()
            .is_some_and(|text| codex_registered(&text, command)),
        _ => true,
    }
}

/// The `[mcp_servers.mindmap]` table names this command and these args.
fn codex_registered(config: &str, command: &[String]) -> bool {
    let header = format!("[mcp_servers.{SERVER}]");
    let Some(start) = config.find(&header) else {
        return false;
    };
    let rest = &config[start + header.len()..];
    let table = rest.find("\n[").map_or(rest, |end| &rest[..end]);
    let quoted = |s: &str| serde_json::to_string(s).unwrap_or_default();
    let args = command
        .get(1..)
        .unwrap_or_default()
        .iter()
        .map(|arg| quoted(arg))
        .collect::<Vec<_>>()
        .join(", ");
    command
        .first()
        .is_some_and(|exe| table.contains(&format!("command = {}", quoted(exe))))
        && table.contains(&format!("args = [{args}]"))
}

/// What Init would have to install first, one line each (`skill for claude`, `MCP for codex`);
/// empty when every agent CLI here is ready.
pub fn missing(address: &str) -> Vec<String> {
    let (Some(home), Ok(command)) = (home(), mcp_command(address)) else {
        return Vec::new();
    };
    let mut missing = Vec::new();
    for cli in clis(&home) {
        if !cli.skill.is_file() {
            missing.push(format!("skill for {}", cli.kind));
        }
        if !mcp_registered(&home, cli.kind, &command) {
            missing.push(format!("MCP for {}", cli.kind));
        }
    }
    missing
}

/// Install whatever is missing and refresh a skill an older node wrote. Returns what was missing
/// before: when it is not empty, agents already running lack the tools.
pub fn ensure(address: &str) -> Result<Vec<String>> {
    let home = home().ok_or_else(|| Error::new("HOME is not set; cannot install agent tools"))?;
    let command = mcp_command(address)?;
    let before = missing(address);
    for cli in clis(&home) {
        if std::fs::read_to_string(&cli.skill).ok().as_deref() != Some(SKILL) {
            if let Some(dir) = cli.skill.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&cli.skill, SKILL)?;
        }
        if !mcp_registered(&home, cli.kind, &command) {
            register(cli.kind, &command)?;
        }
    }
    Ok(before)
}

/// Replace `kind`'s `mindmap` MCP entry with `command`, through the CLI's own `mcp` commands.
fn register(kind: &str, command: &[String]) -> Result<()> {
    let scope: &[&str] = if kind == "claude" {
        &["-s", "user"]
    } else {
        &[]
    };
    // An entry from an older node points elsewhere; removing a missing one fails harmlessly.
    let _ = Command::new(kind)
        .args(["mcp", "remove"])
        .args(scope)
        .arg(SERVER)
        .output();
    let output = Command::new(kind)
        .args(["mcp", "add"])
        .args(scope)
        .args([SERVER, "--"])
        .args(command)
        .output()
        .map_err(|error| Error::new(format!("cannot run {kind} mcp add: {error}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::new(format!(
            "{kind} mcp add failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skill_is_compiled_in() {
        assert!(SKILL.starts_with("---\nname: mindmap"));
    }

    /// Runs the real `claude` and `codex` CLIs against a scratch HOME.
    #[test]
    #[ignore = "runs the installed agent CLIs"]
    fn ensure_installs_what_missing_reports_into_a_fresh_home() {
        let home = std::env::temp_dir().join(format!("mindmap-tools-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        std::env::set_var("HOME", &home);
        let before = missing(DEFAULT_ADDRESS);
        assert!(!before.is_empty(), "a fresh home has nothing");
        assert_eq!(ensure(DEFAULT_ADDRESS).unwrap(), before);
        assert!(
            missing(DEFAULT_ADDRESS).is_empty(),
            "{:?}",
            missing(DEFAULT_ADDRESS)
        );
        assert!(
            ensure(DEFAULT_ADDRESS).unwrap().is_empty(),
            "a second Init installs nothing"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_codex_entry_must_run_this_binary() {
        let command = vec!["/opt/mindmap-node".to_owned(), "mcp".to_owned()];
        let ok = "[other]\nx = 1\n\n[mcp_servers.mindmap]\ncommand = \"/opt/mindmap-node\"\nargs = [\"mcp\"]\n\n[mcp_servers.else]\nargs = []\n";
        assert!(codex_registered(ok, &command));
        let moved = ok.replace("/opt/", "/old/");
        assert!(!codex_registered(&moved, &command), "an older binary");
        assert!(!codex_registered("[mcp_servers.else]\n", &command));
        let addressed = vec![
            "/opt/mindmap-node".to_owned(),
            "mcp".to_owned(),
            "--address".to_owned(),
            "127.0.0.1:1".to_owned(),
        ];
        assert!(!codex_registered(ok, &addressed), "another node address");
    }
}
