//! Integration coverage for the Sync TCP methods on a private port and scratch state.
//!
//! The tests start their own binaries on `127.0.0.1:47899` with an empty `MINDMAP_DATA`. They never
//! touch the live service on 47821 and never write a real record. The first test only reads a real
//! Herdr session; the second replaces Herdr with a tiny fake so the whole prompt → job →
//! `last_sync` lifecycle can run without starting a real agent.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const ADDRESS: &str = "127.0.0.1:47899";

/// Both tests use the same fixed port, so run them one at a time.
static PORT_LOCK: Mutex<()> = Mutex::new(());

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

fn unique_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "mindmap-sync-tcp-{tag}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(PathBuf::from)
        .expect("service directory has a parent")
}

fn start_service(herdr: Option<&Path>) -> Option<Service> {
    // Do not risk talking to something that is already listening.
    match TcpListener::bind(ADDRESS) {
        Ok(listener) => drop(listener),
        Err(error) => {
            eprintln!("skipping Sync TCP test: {ADDRESS} is not free ({error})");
            return None;
        }
    }

    let data = unique_dir("data");
    fs::create_dir_all(&data).expect("create the scratch data directory");
    let root = repo_root();
    let mut command = Command::new(env!("CARGO_BIN_EXE_mindmap-service"));
    command
        .arg("serve")
        .env("MINDMAP_BIND", ADDRESS)
        .env("MINDMAP_DATA", &data)
        .env("MINDMAP_LEGACY_DATA", data.join("legacy"))
        .env("MINDMAP_ROOT", &root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(herdr) = herdr {
        command.env("MINDMAP_HERDR", herdr);
    }
    let child = command.spawn().expect("start the scratch mindmap-service");
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

/// A disposable two-commit repository so the fake-agent test does not depend on the shared
/// checkout's current HEAD (which the parent agent may move while tests run).
struct FixtureRepo {
    dir: PathBuf,
    head: String,
    previous: String,
}

impl FixtureRepo {
    fn new() -> Option<Self> {
        let dir = unique_dir("git-fixture");
        fs::create_dir_all(&dir).ok()?;
        let setup = (|| -> Option<()> {
            if !git_ok(&dir, &["init", "-q"]) {
                return None;
            }
            if !git_ok(&dir, &["config", "user.email", "sync@example.invalid"]) {
                return None;
            }
            if !git_ok(&dir, &["config", "user.name", "Sync Test"]) {
                return None;
            }
            fs::write(dir.join("file.txt"), "one\n").ok()?;
            if !git_ok(&dir, &["add", "file.txt"]) {
                return None;
            }
            if !git_ok(&dir, &["commit", "-q", "-m", "first"]) {
                return None;
            }
            fs::write(dir.join("file.txt"), "one\ntwo\n").ok()?;
            if !git_ok(&dir, &["add", "file.txt"]) {
                return None;
            }
            if !git_ok(&dir, &["commit", "-q", "-m", "second"]) {
                return None;
            }
            Some(())
        })();
        if setup.is_none() {
            let _ = fs::remove_dir_all(&dir);
            return None;
        }
        let head = git_stdout(&dir, &["rev-parse", "HEAD"]);
        let previous = git_stdout(&dir, &["rev-parse", "HEAD~1"]);
        if head.is_empty() || previous.is_empty() {
            let _ = fs::remove_dir_all(&dir);
            return None;
        }
        Some(Self {
            dir,
            head,
            previous,
        })
    }
}

impl Drop for FixtureRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn git_stdout(dir: &Path, args: &[&str]) -> String {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_default()
}

#[test]
fn diff_sync_and_sync_status_over_tcp() {
    let _guard = PORT_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let Some(service) = start_service(None) else {
        return;
    };
    let Some(mut client) = Client::connect() else {
        panic!("scratch service did not accept a second connection");
    };
    let health = client.request(&json!({"id": 1, "method": "health"}));
    assert_eq!(health.get("ok").and_then(Value::as_bool), Some(true));

    if workspace_present(&mut client) {
        // Diff: scratch data means no last sync, so both nodes are added and the code lists empty.
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

        // Sync with an impossible pane: a request error with no job and no record write.
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

        let record = service.data.join("projects.json");
        assert!(
            !record.exists(),
            "the scratch service wrote {}",
            record.display()
        );
    } else {
        eprintln!("skipping real-Herdr Sync TCP assertions: Herdr has no wX workspace");
    }
}

const FAKE_HERDR: &str = r#"#!/bin/sh
case "$1 $2" in
  "workspace list")
    printf '%s\n' '{"result":{"workspaces":[{"workspace_id":"wX","label":"mindmap","agent_status":"idle","focused":true,"tab_count":1,"pane_count":1,"worktree":{"checkout_path":__ROOT_JSON__,"repo_name":"mindmap","is_linked_worktree":false}}]}}'
    ;;
  "agent list")
    status=$(cat "__STATUS__" 2>/dev/null || printf 'idle')
    printf '{"result":{"agents":[{"workspace_id":"wX","agent":"codex","name":"fake-agent","pane_id":"wX:p2","tab_id":"wX:t1","agent_status":"%s","focused":true}]}}\n' "$status"
    ;;
  "agent prompt")
    printf '%s\n' "$4" > "__PROMPT__"
    printf '%s\n' '{"result":{"prompted":true}}'
    ;;
  *)
    printf '%s\n' '{"result":{}}'
    ;;
esac
"#;

struct FakeHerdr {
    dir: PathBuf,
    script: PathBuf,
    status: PathBuf,
    prompt: PathBuf,
}

impl FakeHerdr {
    fn new(workspace: &Path) -> Option<Self> {
        #[cfg(not(unix))]
        {
            let _ = workspace;
            return None;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let dir = unique_dir("fake-herdr");
            fs::create_dir_all(&dir).ok()?;
            let script = dir.join("herdr");
            let status = dir.join("status");
            let prompt = dir.join("prompt");
            fs::write(&status, "idle\n").ok()?;
            let root_json = serde_json::to_string(&workspace.to_string_lossy().to_string()).ok()?;
            let body = FAKE_HERDR
                .replace("__ROOT_JSON__", &root_json)
                .replace("__STATUS__", &status.to_string_lossy())
                .replace("__PROMPT__", &prompt.to_string_lossy());
            fs::write(&script, body).ok()?;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).ok()?;
            Some(Self {
                dir,
                script,
                status,
                prompt,
            })
        }
    }

    fn set_status(&self, status: &str) {
        fs::write(&self.status, format!("{status}\n")).expect("write the fake Herdr status");
    }

    fn prompt(&self) -> String {
        fs::read_to_string(&self.prompt).unwrap_or_default()
    }
}

impl Drop for FakeHerdr {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn a_fake_agent_finishes_a_sync_and_writes_last_sync() {
    let _guard = PORT_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let Some(fixture) = FixtureRepo::new() else {
        eprintln!("skipping fake-Herdr Sync job test: could not create a git fixture");
        return;
    };
    let Some(fake) = FakeHerdr::new(&fixture.dir) else {
        eprintln!("skipping fake-Herdr Sync job test: needs a Unix executable script");
        return;
    };
    let Some(service) = start_service(Some(&fake.script)) else {
        return;
    };

    let head = fixture.head.clone();
    let previous = fixture.previous.clone();
    let root = &fixture.dir;
    let previous_map = json!({
        "pages": [{"id": 1, "name": "Mindmap"}],
        "nodes": [
            {"id": 36, "name": "heading", "parent": null, "page": 1, "kind": "frame", "text": "old"},
            {"id": 7, "name": "Old card", "parent": null, "page": 1, "kind": "text", "text": ""}
        ]
    });
    let record = json!({
        "wX": {
            "id": "wX",
            "label": "mindmap",
            "path": root.to_string_lossy(),
            "file": "file-id",
            "edit": "figcraft://join?file=file-id&token=t",
            "name": "Mindmap · mindmap",
            "built": true,
            "created": "2026-01-01T00:00:00Z",
            "future_field": {"keep": 1},
            "last_sync": {
                "commit": previous,
                "at": "2026-01-01T00:00:00Z",
                "map": previous_map
            }
        }
    });
    fs::write(
        service.data.join("projects.json"),
        serde_json::to_vec_pretty(&record).expect("serialize the scratch record"),
    )
    .expect("write the scratch record");

    let Some(mut client) = Client::connect() else {
        panic!("scratch service did not accept a second connection");
    };
    let diff = client.request(&json!({
        "id": 1,
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
        diff.pointer("/last/commit").and_then(Value::as_str),
        Some(previous.as_str()),
        "{diff}"
    );
    assert_eq!(
        diff.pointer("/code/head").and_then(Value::as_str),
        Some(head.as_str()),
        "{diff}"
    );
    assert!(
        diff.pointer("/code/files")
            .and_then(Value::as_array)
            .is_some_and(|files| !files.is_empty()),
        "{diff}"
    );
    assert!(
        diff.pointer("/code/commits")
            .and_then(Value::as_array)
            .is_some_and(|commits| !commits.is_empty()),
        "{diff}"
    );

    let started = client.request(&json!({
        "id": 2,
        "method": "sync",
        "workspace": "wX",
        "target": "pane:wX:p2",
        "prompt": "keep it small",
        "map": small_map()
    }));
    assert_eq!(
        started.get("ok").and_then(Value::as_bool),
        Some(true),
        "{started}"
    );
    assert_eq!(
        started.pointer("/job/state").and_then(Value::as_str),
        Some("in_flight"),
        "{started}"
    );
    assert_eq!(
        started.pointer("/agent/state").and_then(Value::as_str),
        Some("existing"),
        "{started}"
    );
    assert_eq!(
        started.pointer("/agent/pane").and_then(Value::as_str),
        Some("wX:p2"),
        "{started}"
    );
    let job = started
        .pointer("/job/id")
        .and_then(Value::as_str)
        .expect("sync returned a job id")
        .to_string();

    let prompt = fake.prompt();
    assert!(
        prompt.contains("Sync mindmap (") && prompt.contains(") against the code."),
        "{prompt}"
    );
    assert!(
        prompt.contains("DESIGN CHANGES SINCE last sync:"),
        "{prompt}"
    );
    assert!(prompt.contains("+ title (Mindmap)"), "{prompt}");
    assert!(prompt.contains("~ heading (text)"), "{prompt}");
    assert!(prompt.contains("- Old card (Mindmap)"), "{prompt}");
    assert!(
        prompt.contains(&format!("CODE SINCE {previous}:")),
        "{prompt}"
    );
    assert!(prompt.contains("NOTE: keep it small"), "{prompt}");

    fake.set_status("working");
    let working = client.request(&json!({
        "id": 3,
        "method": "sync_status",
        "job": job
    }));
    assert_eq!(
        working.get("ok").and_then(Value::as_bool),
        Some(true),
        "{working}"
    );
    assert_eq!(
        working.pointer("/job/state").and_then(Value::as_str),
        Some("in_flight"),
        "{working}"
    );

    fake.set_status("idle");
    let finished = client.request(&json!({
        "id": 4,
        "method": "sync_status",
        "job": job
    }));
    assert_eq!(
        finished.get("ok").and_then(Value::as_bool),
        Some(true),
        "{finished}"
    );
    assert_eq!(
        finished.pointer("/job/state").and_then(Value::as_str),
        Some("done"),
        "{finished}"
    );
    assert_eq!(
        finished.pointer("/last/exists").and_then(Value::as_bool),
        Some(true),
        "{finished}"
    );
    assert_eq!(
        finished.pointer("/last/commit").and_then(Value::as_str),
        Some(head.as_str()),
        "{finished}"
    );
    assert!(
        finished
            .pointer("/last/at")
            .and_then(Value::as_str)
            .is_some_and(|at| !at.is_empty()),
        "{finished}"
    );

    let stored: Value = serde_json::from_slice(
        &fs::read(service.data.join("projects.json")).expect("read the scratch store"),
    )
    .expect("parse the scratch store");
    assert_eq!(
        stored
            .pointer("/wX/future_field/keep")
            .and_then(Value::as_u64),
        Some(1),
        "{stored}"
    );
    assert_eq!(
        stored.pointer("/wX/edit").and_then(Value::as_str),
        Some("figcraft://join?file=file-id&token=t"),
        "last_sync must not disturb the links"
    );
    assert_eq!(
        stored
            .pointer("/wX/last_sync/commit")
            .and_then(Value::as_str),
        Some(head.as_str()),
        "{stored}"
    );
    assert_eq!(
        stored.pointer("/wX/last_sync/map"),
        Some(&small_map()),
        "{stored}"
    );
}
