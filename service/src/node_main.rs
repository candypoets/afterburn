//! The per-machine Mindmap node. Repositories own designs; this process owns live sessions.

mod agent;
mod comments;
mod config;
mod error;
mod herdr;
mod mcp;
mod node;
mod node_context;
mod node_documents;
mod node_git;
mod node_herdr;
mod node_migration;
mod workspace;

use crate::error::{Error, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

fn main() {
    if let Err(error) = run() {
        eprintln!("mindmap-node: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    figcraft_text::scan_system_fonts_on_demand();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let address = option(&args, "--bind")
        .or_else(|| option(&args, "--address"))
        .or_else(|| std::env::var("MINDMAP_NODE").ok())
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
        "--version" | "version" => {
            println!(
                "mindmap-node {} protocol {}",
                env!("CARGO_PKG_VERSION"),
                node::PROTOCOL
            );
            Ok(())
        }
        "help" | "--help" | "-h" => {
            println!("mindmap-node serve [--bind 127.0.0.1:47822]\nmindmap-node request '<JSON>' [--address HOST:PORT]\nmindmap-node mcp --path WORKTREE [--address HOST:PORT]\n\nOne node per machine; remote clients use one SSH tunnel to its loopback socket.\nRequest methods: hello, host.status, host.ensure, workspaces, open, init, send, diff, sync, sync_status, sync.prepare, sync.finish, session.inspect, session.command, session.save, merge.plan, merge.apply.\nHerdr installation requires host.ensure with install_herdr:true. Designs live in .mindmap/map.figcraft; no owner link or server key is needed.");
            Ok(())
        }
        command => Err(Error::new(format!("unknown command {command:?}"))),
    }
}

fn option(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find(|p| p[0] == flag).map(|p| p[1].clone())
}

fn request(address: &str, mut request: Value) -> Result<Value> {
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
fn mcp_proxy(address: &str, path: Option<&str>) -> Result<()> {
    let path = path.ok_or_else(|| Error::new("mcp needs --path WORKTREE"))?;
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
                json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"mindmap-node","version":env!("CARGO_PKG_VERSION")}})
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
