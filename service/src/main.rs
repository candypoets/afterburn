//! Mindmap's project service.
//!
//! A Herdr workspace is the project. `list` shows the workspaces on this host and whether each one
//! has a map document; `open` publishes a one-page skeleton on first use and returns the internal
//! edit link; `init` additionally ensures an agent and prompts it; `adopt` publishes an
//! already-built map file without touching an agent; `serve` exposes the same operations on a
//! newline-delimited JSON TCP socket. The TCP API also has `send`, which submits canvas comments
//! to an agent without opening or publishing a document.
//!
//! The edit links are stored in `service/data/projects.json` and never printed by the server. The
//! Python reference (`project/service.py`) remains the behavioral spec for inventory and publish.

mod agent;
mod comments;
mod config;
mod error;
mod herdr;
mod link;
mod mcp;
mod project;
mod records;
mod server;
mod time;

use std::env;
use std::path::Path;

use serde_json::{json, Value};

use crate::agent::Target;
use crate::config::Config;
use crate::error::{Error, Result};
use crate::project::{Opened, ProjectService};

fn main() {
    if let Err(error) = run() {
        eprintln!("mindmap-service: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        print_usage();
        return Ok(());
    };

    match command {
        "-h" | "--help" | "help" => {
            print_usage();
            Ok(())
        }
        "-V" | "--version" => {
            println!("mindmap-service {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "list" => run_list(),
        "open" => {
            let query = args
                .get(1)
                .ok_or_else(|| Error::new("open needs a workspace id or label"))?;
            run_open(query)
        }
        "init" => {
            let query = args
                .get(1)
                .ok_or_else(|| Error::new("init needs a workspace id or label"))?;
            let target = parse_option(&args[2..], "--target")?;
            let target = agent::parse_target(target.as_deref())?;
            run_init(query, &target)
        }
        "adopt" => {
            let query = args
                .get(1)
                .ok_or_else(|| Error::new("adopt needs a workspace id or label"))?;
            let path = args
                .get(2)
                .ok_or_else(|| Error::new("adopt needs a .figcraft file path"))?;
            let name = parse_option(&args[3..], "--name")?;
            run_adopt(query, Path::new(path), name.as_deref())
        }
        "serve" => {
            let config = Config::discover()?;
            server::serve(config)
        }
        other => Err(Error::new(format!(
            "unknown command {other:?}; try `mindmap-service --help`"
        ))),
    }
}

fn run_list() -> Result<()> {
    let config = Config::discover()?;
    let service = ProjectService::new(config)?;
    for workspace in service.workspaces()? {
        let path = workspace.path.as_deref().unwrap_or("—");
        let agents = if workspace.agents == 1 { "" } else { "s" };
        println!(
            "{:<4}  {:<18}  {:<5}  {} agent{} · {:<8}  {}",
            workspace.id,
            workspace.label,
            workspace.map,
            workspace.agents,
            agents,
            workspace.agent_status,
            path
        );
    }
    Ok(())
}

fn run_open(query: &str) -> Result<()> {
    let config = Config::discover()?;
    let service = ProjectService::new(config)?;
    let opened = service.open(query)?;
    print_json(&open_json(&opened))
}

fn run_init(query: &str, target: &Target) -> Result<()> {
    let config = Config::discover()?;
    let service = ProjectService::new(config)?;
    let result = service.init(query, target)?;
    let mut output = open_json(&result.opened);
    if let Value::Object(object) = &mut output {
        object.insert("agent".to_string(), serde_json::to_value(result.agent)?);
    }
    print_json(&output)
}

fn run_adopt(query: &str, path: &Path, name: Option<&str>) -> Result<()> {
    let config = Config::discover()?;
    let service = ProjectService::new(config)?;
    let adopted = service.adopt(query, path, name)?;
    print_json(&serde_json::to_value(adopted)?)
}

/// Parse one `--flag value` or `--flag=value` option from the trailing arguments.
fn parse_option(args: &[String], flag: &str) -> Result<Option<String>> {
    let prefix = format!("{flag}=");
    let mut value: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let found = if let Some(rest) = arg.strip_prefix(&prefix) {
            index += 1;
            Some(rest.to_string())
        } else if arg == flag {
            let next = args
                .get(index + 1)
                .ok_or_else(|| Error::new(format!("{flag} needs a value")))?;
            index += 2;
            Some(next.clone())
        } else {
            return Err(Error::new(format!("unexpected argument {arg:?}")));
        };
        if let Some(found) = found {
            if value.replace(found).is_some() {
                return Err(Error::new(format!("{flag} was given more than once")));
            }
        }
    }
    Ok(value)
}

fn open_json(opened: &Opened) -> serde_json::Value {
    json!({
        "workspace": opened.workspace.id,
        "label": opened.workspace.label,
        "path": opened.workspace.path,
        "link": opened.record.edit,
        "file": opened.record.file,
        "name": opened.record.name,
    })
}

fn print_json(value: &serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn print_usage() {
    println!(
        "\
Mindmap project service

Usage:
  mindmap-service list                       Herdr workspaces and their map state
  mindmap-service open <workspace-id|label>  Reuse or publish the project document
  mindmap-service init <workspace-id|label> [--target <target>]
                                             Open it and ensure an agent is working
      target: auto (default), pane:<pane-id>, or kind:<agent-kind>
  mindmap-service adopt <workspace-id|label> <file.figcraft> [--name <NAME>]
                                             Publish an already-built map, no agent
  mindmap-service serve                      Run the TCP API
  mindmap-service --help

Environment:
  MINDMAP_ROOT          repository root (auto-detected from the checkout)
  MINDMAP_DATA          state directory (default <root>/service/data)
  MINDMAP_BIND          TCP API address (default 0.0.0.0:47821)
  MINDMAP_SERVER        public document server address (default {})
  MINDMAP_LOCAL_SERVER  server used for publishing (default {})
  MINDMAP_HERDR         herdr executable (default herdr)
  MINDMAP_FIGCRAFT_CLI  figcraft-cli path (default <root>/server/figcraft/target/release/figcraft-cli)",
        config::DEFAULT_SERVER,
        config::DEFAULT_LOCAL_SERVER
    );
}
