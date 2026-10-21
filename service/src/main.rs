//! Mindmap's project service.
//!
//! A Herdr workspace is the project. `list` shows the workspaces on this host and whether each one
//! has a map document; `open` publishes a one-page skeleton on first use and returns the internal
//! edit link; `init` additionally ensures an agent and prompts it; `serve` exposes the same
//! operations on a newline-delimited JSON TCP socket.
//!
//! The edit links are stored in `service/data/projects.json` and never printed by the server. The
//! Python reference (`project/service.py`) remains the behavioral spec for inventory and publish.

mod agent;
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

use serde_json::json;

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
            run_open(query, false)
        }
        "init" => {
            let query = args
                .get(1)
                .ok_or_else(|| Error::new("init needs a workspace id or label"))?;
            run_open(query, true)
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

fn run_open(query: &str, init: bool) -> Result<()> {
    let config = Config::discover()?;
    let service = ProjectService::new(config)?;
    if init {
        let result = service.init(query)?;
        let output = open_json(&result.opened);
        let output = match output {
            serde_json::Value::Object(mut object) => {
                object.insert("agent".to_string(), serde_json::to_value(result.agent)?);
                serde_json::Value::Object(object)
            }
            other => other,
        };
        print_json(&output)
    } else {
        let opened = service.open(query)?;
        print_json(&open_json(&opened))
    }
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
  mindmap-service init <workspace-id|label>  Open it and ensure an agent is working
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
