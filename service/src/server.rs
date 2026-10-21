//! The newline-delimited JSON TCP API.
//!
//! One thread per connection, one request per line, one response per line. A connection may send
//! several requests. There is no authentication layer yet; bind it to the tailnet interface or
//! `127.0.0.1` with `MINDMAP_BIND` when exposing it more widely.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use crate::agent;
use crate::config::Config;
use crate::error::{Error, Result, ResultExt};
use crate::project::ProjectService;

/// Refuse a request line larger than this instead of growing a String without bound.
const MAX_REQUEST_BYTES: usize = 1024 * 1024;

/// Run the TCP service until the process is stopped.
pub fn serve(config: Config) -> Result<()> {
    let service = Arc::new(ProjectService::new(config.clone())?);
    let listener = TcpListener::bind(&config.bind)
        .context(format!("bind the project service to {}", config.bind))?;
    let address = listener.local_addr().context("read the bound address")?;
    println!(
        "mindmap-service on tcp://{address} (document server {}, herdr {})",
        config.public_server,
        config.herdr_bin.display()
    );

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let service = Arc::clone(&service);
                let peer = stream
                    .peer_addr()
                    .map(|address| address.to_string())
                    .unwrap_or_else(|_| "unknown".to_string());
                let thread_peer = peer.clone();
                let spawn = thread::Builder::new()
                    .name(format!("mm-client-{peer}"))
                    .spawn(move || {
                        if let Err(error) = handle_connection(stream, &service) {
                            eprintln!("mindmap-service: client {thread_peer}: {error}");
                        }
                    });
                if let Err(error) = spawn {
                    eprintln!("mindmap-service: could not start a client thread: {error}");
                }
            }
            Err(error) => {
                eprintln!("mindmap-service: accept failed: {error}");
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
    Ok(())
}

fn handle_connection(stream: TcpStream, service: &Arc<ProjectService>) -> Result<()> {
    let _ = stream.set_nodelay(true);
    let mut reader = BufReader::new(stream.try_clone().context("clone the client socket")?);
    let mut writer = stream;
    let mut line = String::new();

    loop {
        line.clear();
        let bytes = {
            let mut limited = (&mut reader).take((MAX_REQUEST_BYTES + 1) as u64);
            limited.read_line(&mut line).context("read a request")?
        };
        if bytes == 0 {
            return Ok(());
        }
        if bytes > MAX_REQUEST_BYTES {
            let response = error_response(
                Value::Null,
                format!("request larger than {MAX_REQUEST_BYTES} bytes"),
            );
            let _ = write_response(&mut writer, &response);
            return Ok(());
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Value>(trimmed) {
            Ok(request) => {
                let response = handle_request(service, &request);
                log_request(&request, &response);
                response
            }
            Err(error) => error_response(Value::Null, format!("invalid JSON: {error}")),
        };
        write_response(&mut writer, &response)?;
    }
}

fn handle_request(service: &ProjectService, request: &Value) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    match dispatch(service, request, &id) {
        Ok(response) => response,
        Err(error) => error_response(id, error.message()),
    }
}

fn dispatch(service: &ProjectService, request: &Value, id: &Value) -> Result<Value> {
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::new("request has no string method"))?;
    let workspace = request.get("workspace").and_then(Value::as_str);

    match method {
        "health" => Ok(json!({
            "id": id,
            "ok": true,
            "service": "mindmap-service",
            "version": env!("CARGO_PKG_VERSION"),
            "server": service.config().public_server,
        })),
        "workspaces" => {
            let workspaces = service.workspaces()?;
            // The picker needs the host-wide CLI list; workspace views carry it too, but the
            // client reads it once for the screen.
            let clis = workspaces
                .first()
                .map(|workspace| workspace.clis.clone())
                .unwrap_or_default();
            Ok(json!({ "id": id, "ok": true, "workspaces": workspaces, "clis": clis }))
        }
        "open" => {
            let query =
                workspace.ok_or_else(|| Error::new("open needs a \"workspace\" id or label"))?;
            let opened = service.open(query)?;
            let link = opened
                .record
                .edit
                .clone()
                .ok_or_else(|| Error::new("project record has no edit link"))?;
            Ok(json!({
                "id": id,
                "ok": true,
                "link": link,
                "file": opened.record.file,
                "name": opened.record.name,
            }))
        }
        "init" => {
            let query =
                workspace.ok_or_else(|| Error::new("init needs a \"workspace\" id or label"))?;
            let target = request_target(request)?;
            let init = service.init(query, &target)?;
            let link = init
                .opened
                .record
                .edit
                .clone()
                .ok_or_else(|| Error::new("project record has no edit link"))?;
            Ok(json!({
                "id": id,
                "ok": true,
                "link": link,
                "file": init.opened.record.file,
                "name": init.opened.record.name,
                "agent": init.agent,
            }))
        }
        "adopt" => {
            let query =
                workspace.ok_or_else(|| Error::new("adopt needs a \"workspace\" id or label"))?;
            let path = request
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::new("adopt needs a \"path\" to a .figcraft file"))?;
            let name = match request.get("name") {
                None | Some(Value::Null) => None,
                Some(Value::String(name)) => Some(name.as_str()),
                Some(other) => {
                    return Err(Error::new(format!(
                        "\"name\" must be a string, not {other}"
                    )))
                }
            };
            let adopted = service.adopt(query, Path::new(path), name)?;
            Ok(json!({
                "id": id,
                "ok": true,
                "file": adopted.file,
                "link": adopted.link,
                "name": adopted.name,
            }))
        }
        other => Err(Error::new(format!("unknown method {other:?}"))),
    }
}

/// Read the optional `target` request field, rejecting non-string values.
fn request_target(request: &Value) -> Result<agent::Target> {
    match request.get("target") {
        None | Some(Value::Null) => agent::parse_target(None),
        Some(Value::String(target)) => agent::parse_target(Some(target)),
        Some(other) => Err(Error::new(format!(
            "\"target\" must be a string, not {other}"
        ))),
    }
}

fn error_response(id: Value, message: impl Into<String>) -> Value {
    json!({ "id": id, "ok": false, "error": message.into() })
}

fn write_response(writer: &mut TcpStream, response: &Value) -> Result<()> {
    let mut data = serde_json::to_vec(response)?;
    data.push(b'\n');
    writer.write_all(&data).context("write a response")?;
    writer.flush().context("flush a response")
}

fn log_request(request: &Value, response: &Value) {
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("<invalid>");
    let workspace = request
        .get("workspace")
        .and_then(Value::as_str)
        .map(|workspace| format!(" {workspace}"))
        .unwrap_or_default();
    let ok = response.get("ok").and_then(Value::as_bool).unwrap_or(false);
    if ok {
        eprintln!("mindmap-service: {method}{workspace} -> ok");
    } else {
        let error = response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        eprintln!("mindmap-service: {method}{workspace} -> error: {error}");
    }
}
