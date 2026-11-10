//! The per-machine Afterburn node. Repositories own designs; this process owns live sessions.

mod agent;
mod agent_tools;
mod comments;
mod config;
mod error;
mod herdr;
mod mcp;
mod node;
mod node_brief;
mod node_context;
mod node_documents;
mod node_edit;
mod node_git;
mod node_herdr;
mod node_media;
mod node_migration;
mod workspace;

use crate::error::{Error, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

fn main() {
    if let Err(error) = run() {
        eprintln!("afterburn-node: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    figcraft_text::scan_system_fonts_on_demand();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let address = option(&args, "--bind")
        .or_else(|| option(&args, "--address"))
        .or_else(|| std::env::var("AFTERBURN_NODE").ok())
        .unwrap_or_else(|| "127.0.0.1:47822".into());
    match args.first().map(String::as_str).unwrap_or("help") {
        "serve" => node::serve(&address),
        "migrate" => {
            let records = option(&args, "--records")
                .ok_or_else(|| Error::new("migrate needs --records PATH"))?;
            node_migration::migrate(&address, std::path::Path::new(&records))
        }
        "request" => {
            let input = args
                .get(1)
                .ok_or_else(|| Error::new("request needs one JSON object"))?;
            let result = request(&address, serde_json::from_str(input)?)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            if result["ok"] == false {
                return Err(Error::new("node rejected the request"));
            }
            Ok(())
        }
        "mcp" => mcp_proxy(&address, option(&args, "--path").as_deref()),
        "build-info" => {
            println!(
                "{}",
                json!({"build_id":afterburn_build::BUILD_ID,"version":afterburn_build::VERSION,"protocol":node::PROTOCOL})
            );
            Ok(())
        }
        "--version" | "version" => {
            println!(
                "afterburn-node {} protocol {} build {}",
                env!("CARGO_PKG_VERSION"),
                node::PROTOCOL,
                afterburn_build::BUILD_ID
            );
            Ok(())
        }
        "help" | "--help" | "-h" => {
            println!("afterburn-node serve [--bind 127.0.0.1:47822]\nafterburn-node request '<JSON>' [--address HOST:PORT]\nafterburn-node mcp [--path WORKTREE] [--address HOST:PORT]\n\nOne node per machine; remote clients use one SSH tunnel to its loopback socket.\nRequest methods: hello, host.status, host.ensure, workspaces, workspace.create, workspace.focus, open, init, send, diff, sync, sync_status, sync.prepare, sync.finish, session.inspect, session.command, session.save, media.add, media.read, media.frames, media.probe, edit.apply, merge.plan, merge.apply.\nHerdr installation requires host.ensure with install_herdr:true. Designs live in .afterburn/map.figcraft; no owner link or server key is needed.");
            Ok(())
        }
        command => Err(Error::new(format!("unknown command {command:?}"))),
    }
}

fn option(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find(|p| p[0] == flag).map(|p| p[1].clone())
}

/// A record belongs to the agent that makes it: Herdr gives every pane's processes its pane and
/// workspace ids, so the node can cancel the record when that agent exits.
fn record_owner(request: &mut Value, env: impl Fn(&str) -> Option<String>) {
    if request["method"] != "record" {
        return;
    }
    for (field, key) in [
        ("pane", "HERDR_PANE_ID"),
        ("workspace", "HERDR_WORKSPACE_ID"),
    ] {
        if request[field].as_str().is_none_or(|v| v.trim().is_empty()) {
            if let Some(value) = env(key).filter(|v| !v.trim().is_empty()) {
                request[field] = json!(value);
            }
        }
    }
}

fn request(address: &str, mut request: Value) -> Result<Value> {
    record_owner(&mut request, |key| std::env::var(key).ok());
    request["id"] = json!(1);
    request["version"] = json!(node::PROTOCOL);
    let mut stream = TcpStream::connect(address)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(180)))?;
    serde_json::to_writer(&mut stream, &request)?;
    stream.write_all(b"\n")?;
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        let n = (&mut reader)
            .take(node::MAX_FRAME as u64 + 1)
            .read_line(&mut line)?;
        if n == 0 || n > node::MAX_FRAME {
            return Err(Error::new("node closed connection or exceeded frame limit"));
        }
        let reply: Value = serde_json::from_str(&line)?;
        if reply.get("id") == Some(&json!(1)) {
            return Ok(reply);
        }
    }
}

/// Shape a node reply the way FigCraft's own MCP does: reads return the node itself, commands
/// return their result, and a screenshot returns an image.
fn tool_result(tool: &str, reply: &Value) -> Result<Value> {
    if reply["ok"] != true {
        return Ok(
            json!({"content":[{"type":"text","text":serde_json::to_string_pretty(reply)?}],"isError":true}),
        );
    }
    if tool == "video_frames" {
        if let Some(data) = reply["data"].as_str() {
            let about = json!({"seconds": reply["seconds"], "columns": reply["columns"], "order": reply["order"]});
            return Ok(json!({"content":[
                {"type":"image","data":data,"mimeType":"image/png"},
                {"type":"text","text":serde_json::to_string(&about)?}
            ]}));
        }
    }
    if tool == "screenshot" {
        if let Some(data) = reply["data"].as_str() {
            return Ok(json!({"content":[{"type":"image","data":data,"mimeType":"image/png"}]}));
        }
    }
    let body = match tool {
        "inspect_document" | "get_node" => &reply["document"],
        "run_command" | "create_node" | "set_properties" => &reply["result"],
        _ => reply,
    };
    Ok(json!({"content":[{"type":"text","text":serde_json::to_string_pretty(body)?}]}))
}

/// Small stdio MCP adapter for agents. Commands always execute inside the node's live session.
/// Without `--path` it opens the folder it runs in: the agent's worktree, so one user-scoped MCP
/// entry serves every worktree.
fn mcp_proxy(address: &str, path: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let path = path.map_or_else(|| cwd.to_string_lossy().into_owned(), str::to_owned);
    let path = path.as_str();
    let opened = request(address, json!({"method":"open","path":path}))?;
    if opened["ok"] != true {
        return Err(Error::new(
            opened["error"].as_str().unwrap_or("cannot open worktree"),
        ));
    }
    let mut session = opened["session"].clone();
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.len() > node::MAX_FRAME {
            return Err(Error::new("MCP request too large"));
        }
        let value: Value = serde_json::from_str(&line)?;
        let Some(id) = value.get("id") else { continue };
        let result = match value["method"].as_str().unwrap_or("") {
            "initialize" => {
                json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"afterburn-node","version":env!("CARGO_PKG_VERSION")}})
            }
            "ping" => json!({}),
            "tools/list" => json!({"tools":[
                {"name":"inspect_document","description":"Read the authoritative live design","inputSchema":{"type":"object","properties":{"depth":{"type":"integer"}}}},
                {"name":"get_node","description":"Read a layer or page and its children","inputSchema":{"type":"object","properties":{"id":{"type":"integer"},"depth":{"type":"integer"}},"required":["id"]}},
                {"name":"create_node","description":"Create a layer in the live session (node.create): type, parent, name, x, y, width, height, text, path, props","inputSchema":{"type":"object","properties":{"type":{"type":"string"},"parent":{"type":"integer"},"name":{"type":"string"},"x":{"type":"number"},"y":{"type":"number"},"width":{"type":"number"},"height":{"type":"number"},"text":{"type":"string"},"path":{"type":"string"},"index":{"type":"integer"},"props":{"type":"object"}},"required":["type"]}},
                {"name":"set_properties","description":"Set properties on layers (node.set): ids and a props object","inputSchema":{"type":"object","properties":{"ids":{"type":"array","items":{"type":"integer"}},"props":{"type":"object"}},"required":["ids","props"]}},
                {"name":"screenshot","description":"PNG of layers or a page of the live design, to check what you drew; nothing is written","inputSchema":{"type":"object","properties":{"ids":{"type":"array","items":{"type":"integer"}},"page":{"type":"integer"},"scale":{"type":"number"}}}},
                {"name":"run_command","description":"Execute a FigCraft command in the live worktree session","inputSchema":{"type":"object","properties":{"command":{"type":"string"},"params":{"type":"object"}},"required":["command"]}},
                {"name":"list_commands","description":"Discover FigCraft commands and parameter schemas","inputSchema":{"type":"object","properties":{"filter":{"type":"string"}}}},
                {"name":"add_video","description":"Put a video clip on the canvas (media.add): source is the clip's absolute path on the node's machine; parent is the page or card id; x, y are relative to it. The card shows the first frame; the app plays the clip in place. Screenshots show the first frame only","inputSchema":{"type":"object","properties":{"source":{"type":"string"},"parent":{"type":"integer"},"x":{"type":"number"},"y":{"type":"number"},"width":{"type":"number"},"name":{"type":"string"}},"required":["source","parent"]}},
                {"name":"apply_edit","description":"Video editing (edit.apply): send a whole cut list {name, width?, height?, fps?, target_seconds?, clips:[{id, source, in, out, title?, crossfade?}], voice?:[{id, source, at, text?}], captions?:[{text, at, until, voice?}], music?:{source, gain_db?}}; sources are files in the worktree, times in seconds. The node checks it, renders it with ffmpeg and redraws the edit's frame (the playable take and a timeline picture); a refused edit lists every reason. parent, x, y are needed the first time only. Never move timeline layers: they are locked","inputSchema":{"type":"object","properties":{"edit":{"type":"object"},"parent":{"type":"integer"},"x":{"type":"number"},"y":{"type":"number"}},"required":["edit"]}},
                {"name":"video_frames","description":"Look at a video (media.frames): a contact sheet of its frames at the given times in seconds, or count frames evenly spread (default 6). node is a video card's id (an edit's take, or a clip card); file is a clip's file name","inputSchema":{"type":"object","properties":{"node":{"type":"integer"},"file":{"type":"string"},"times":{"type":"array","items":{"type":"number"}},"count":{"type":"integer"}}}},
                {"name":"video_probe","description":"Measure a video you cannot hear (media.probe): length, whether it has sound, loudness in LUFS, seconds of silence and of black picture","inputSchema":{"type":"object","properties":{"node":{"type":"integer"},"file":{"type":"string"}}}},
                {"name":"node_request","description":"Call node diff, sync.prepare, sync.finish, merge or other worktree APIs","inputSchema":{"type":"object","properties":{"request":{"type":"object"}},"required":["request"]}}
            ]}),
            "tools/call" => {
                // Opening is idempotent and restores the worktree after a node restart. Never
                // retry a command whose reply was lost; only this read/setup call is repeated.
                let reopened = request(address, json!({"method":"open","path":path}));
                if let Ok(value) = &reopened {
                    if value["ok"] == true {
                        session = value["session"].clone();
                    }
                }
                let args = &value["params"]["arguments"];
                let tool = value["params"]["name"].as_str().unwrap_or("");
                let mut req = match tool {
                    "inspect_document" => json!({"method":"session.inspect","depth":args["depth"]}),
                    "get_node" => {
                        json!({"method":"session.inspect","node":args["id"],"depth":args["depth"]})
                    }
                    "run_command" => {
                        json!({"method":"session.command","command":args["command"],"params":args["params"]})
                    }
                    "list_commands" => json!({"method":"session.commands","filter":args["filter"]}),
                    "create_node" => {
                        let mut params = args.clone();
                        if params.get("select").is_none() {
                            params["select"] = json!(false);
                        }
                        json!({"method":"session.command","command":"node.create","params":params})
                    }
                    "set_properties" => {
                        let mut params = args["props"].clone();
                        if !params.is_object() {
                            params = json!({});
                        }
                        params["ids"] = args["ids"].clone();
                        json!({"method":"session.command","command":"node.set","params":params})
                    }
                    "screenshot" => {
                        json!({"method":"session.render","ids":args["ids"],"page":args["page"],"scale":args["scale"]})
                    }
                    "apply_edit" | "video_frames" | "video_probe" => {
                        let mut request = args.clone();
                        if !request.is_object() {
                            request = json!({});
                        }
                        request["method"] = json!(match tool {
                            "apply_edit" => "edit.apply",
                            "video_frames" => "media.frames",
                            _ => "media.probe",
                        });
                        request
                    }
                    "add_video" => {
                        let mut request = args.clone();
                        if !request.is_object() {
                            request = json!({});
                        }
                        request["method"] = json!("media.add");
                        request
                    }
                    "node_request" => args["request"].clone(),
                    _ => json!({"method":"unknown"}),
                };
                req["session"] = session.clone();
                let reply = match reopened {
                    Ok(value) if value["ok"]==true=>request(address,req).unwrap_or_else(|e|json!({"ok":false,"error":format!("{e}; command outcome may be unknown, inspect before retrying")})),
                    Ok(value)=>value,
                    Err(error)=>json!({"ok":false,"error":error.to_string()}),
                };
                tool_result(tool, &reply)?
            }
            _ => {
                println!(
                    "{}",
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"unknown method"}})
                );
                continue;
            }
        };
        println!("{}", json!({"jsonrpc":"2.0","id":id,"result":result}));
        std::io::stdout().flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod record_owner_tests {
    use super::*;

    #[test]
    fn a_record_carries_the_pane_and_workspace_of_the_agent_that_sends_it() {
        let env = |key: &str| match key {
            "HERDR_PANE_ID" => Some("wX:p2".to_owned()),
            "HERDR_WORKSPACE_ID" => Some("wX".to_owned()),
            _ => None,
        };
        let mut record = json!({"method":"record","request":"r"});
        record_owner(&mut record, env);
        assert_eq!(
            (record["pane"].as_str(), record["workspace"].as_str()),
            (Some("wX:p2"), Some("wX"))
        );
        let mut named = json!({"method":"record","pane":"w1:p1"});
        record_owner(&mut named, env);
        assert_eq!(named["pane"], "w1:p1", "an explicit pane wins");
        let mut diff = json!({"method":"diff"});
        record_owner(&mut diff, env);
        assert!(diff.get("pane").is_none());
    }
}
