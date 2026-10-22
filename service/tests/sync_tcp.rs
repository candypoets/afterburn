//! Integration coverage for the Sync TCP methods on a private port and scratch state.
//!
//! The test starts its own binary on `127.0.0.1:47899` with an empty `MINDMAP_DATA`. It never
//! touches the live service on 47821, never writes a real record, and never prompts an agent:
//! `diff` only reads, and the `sync` request names a pane that cannot exist. If Herdr is not
//! available (or the workspace is not present on the host) the test skips its assertions rather
//! than depending on a live session.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const ADDRESS: &str = "127.0.0.1:47899";

struct Service {
    child: Child,
    data: PathBuf,
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.data);
    }
}

struct Client {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl Client {
    fn connect() -> Option<Self> {
        let stream = TcpStream::connect(ADDRESS).ok()?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
        let writer = stream.try_clone().ok()?;
        Some(Self {
            reader: BufReader::new(stream),
            writer,
        })
    }

    fn request(&mut self, request: &Value) -> Value {
        let line = serde_json::to_string(request).unwrap_or_default();
        writeln!(self.writer, "{line}").unwrap_or(());
        self.writer.flush().unwrap_or(());
        let mut response = String::new();
        self.reader.read_line(&mut response).unwrap_or(0);
        assert!(!response.is_empty(), "service closed the connection");
        serde_json::from_str(response.trim()).expect("service returned invalid JSON")
    }
}

fn scratch_dir() -> PathBuf {
    std::env::temp_dir().join(format!(
        "mindmap-sync-tcp-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ))
}

fn start_service() -> Option<Service> {
    // Do not risk talking to something that is already listening.
    match TcpListener::bind(ADDRESS) {
        Ok(listener) => drop(listener),
        Err(error) => {
            eprintln!("skipping Sync TCP test: {ADDRESS} is not free ({error})");
            return None;
        }
    }

    let data = scratch_dir();
    fs::create_dir_all(&data).expect("create the scratch data directory");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(PathBuf::from)
        .expect("service directory has a parent");
    let child = Command::new(env!("CARGO_BIN_EXE_mindmap-service"))
        .arg("serve")
        .env("MINDMAP_BIND", ADDRESS)
        .env("MINDMAP_DATA", &data)
        .env("MINDMAP_LEGACY_DATA", data.join("legacy"))
        .env("MINDMAP_ROOT", &root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start the scratch mindmap-service");
    let mut service = Service { child, data };

    for _ in 0..100 {
        if Client::connect().is_some() {
            return Some(service);
        }
        if service.child.try_wait().ok().flatten().is_some() {
            panic!("scratch mindmap-service exited before it accepted a connection");
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("scratch mindmap-service did not accept a connection on {ADDRESS}");
}

fn small_map() -> Value {
    json!({
        "pages": [{"id": 1, "name": "Mindmap"}],
        "nodes": [
            {"id": 36, "name": "heading", "parent": null, "page": 1, "kind": "frame", "text": ""},
            {"id": 37, "name": "title", "parent": 36, "page": 1, "kind": "text", "text": "The workspace client"}
        ]
    })
}

fn workspace_present(client: &mut Client) -> bool {
    let response = client.request(&json!({"id": 1, "method": "workspaces"}));
    if response.get("ok").and_then(Value::as_bool) != Some(true) {
        return false;
    }
    response
        .get("workspaces")
        .and_then(Value::as_array)
        .is_some_and(|workspaces| {
            workspaces
                .iter()
                .any(|workspace| workspace.get("id").and_then(Value::as_str) == Some("wX"))
        })
}

#[test]
fn diff_sync_and_sync_status_over_tcp() {
    let Some(service) = start_service() else {
        return;
    };
    let Some(mut client) = Client::connect() else {
        panic!("scratch service did not accept a second connection");
    };
    let health = client.request(&json!({"id": 1, "method": "health"}));
    assert_eq!(health.get("ok").and_then(Value::as_bool), Some(true));

    if !workspace_present(&mut client) {
        eprintln!("skipping Sync TCP assertions: Herdr has no wX workspace");
        return;
    }

    // Diff: scratch data means no last sync, so both nodes are added and the code lists are empty.
    let diff = client.request(&json!({
        "id": 2,
        "method": "diff",
        "workspace": "wX",
        "map": small_map()
    }));
    assert_eq!(
        diff.get("ok").and_then(Value::as_bool),
        Some(true),
        "{diff}"
    );
    assert_eq!(
        diff.pointer("/last/exists").and_then(Value::as_bool),
        Some(false),
        "{diff}"
    );
    let added = diff
        .pointer("/map/added")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(added.len(), 2, "{diff}");
    assert_eq!(
        added[0].get("name").and_then(Value::as_str),
        Some("heading")
    );
    assert_eq!(
        added[0].get("page").and_then(Value::as_str),
        Some("Mindmap")
    );
    assert_eq!(added[1].get("name").and_then(Value::as_str), Some("title"));
    assert_eq!(
        diff.pointer("/map/removed")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0)
    );
    assert_eq!(
        diff.pointer("/map/changed")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0)
    );
    for pointer in ["/code/files", "/code/commits"] {
        assert_eq!(
            diff.pointer(pointer)
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0),
            "{diff}"
        );
    }
    let record = service.data.join("projects.json");
    assert!(
        !record.exists(),
        "diff must not write project records: {}",
        record.display()
    );

    // A bad snapshot is rejected before Herdr is touched.
    let bad_map = client.request(&json!({
        "id": 3,
        "method": "diff",
        "workspace": "wX",
        "map": {"pages": [{"id": 1, "name": "Mindmap"}],
                "nodes": [{"id": 0, "name": "a", "page": 1, "kind": "text", "text": ""}]}
    }));
    assert_eq!(bad_map.get("ok").and_then(Value::as_bool), Some(false));
    assert!(
        bad_map
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("positive integer"),
        "{bad_map}"
    );

    // Sync with an impossible pane: a request error with no job, prompt, or record write.
    let invalid = client.request(&json!({
        "id": 4,
        "method": "sync",
        "workspace": "wX",
        "target": "pane:wX:p999999",
        "prompt": "do not apply this",
        "map": small_map()
    }));
    assert_eq!(invalid.get("ok").and_then(Value::as_bool), Some(false));
    assert!(
        invalid
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("not an agent"),
        "{invalid}"
    );
    assert!(
        !record.exists(),
        "a rejected sync must not write project records: {}",
        record.display()
    );

    let unknown = client.request(&json!({
        "id": 5,
        "method": "sync_status",
        "job": "sync-does-not-exist"
    }));
    assert_eq!(unknown.get("ok").and_then(Value::as_bool), Some(false));
    assert!(
        unknown
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("unknown sync job"),
        "{unknown}"
    );

    assert!(
        !record.exists(),
        "the scratch service wrote {}",
        record.display()
    );
}
