use super::*;
use std::io::{BufRead, BufReader};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = env::temp_dir().join(format!(
            "mm-herdr-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn node(&self) -> NodeHerdr {
        NodeHerdr {
            requested_binary: self.0.join("missing-binary"),
            socket: self.0.join("herdr.sock"),
            session: "test".into(),
            setup: Arc::new(Mutex::new(())),
            prompts: Arc::new(Mutex::new(())),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn shared_session_resolution_matches_herdr_precedence() {
    let home = Some(PathBuf::from("/home/person"));
    assert_eq!(
        resolve_socket(None, None, None, home.clone()).unwrap(),
        (
            PathBuf::from("/home/person/.config/herdr/herdr.sock"),
            "default".into()
        )
    );
    assert_eq!(
        resolve_socket(
            None,
            Some("team.dev"),
            Some(PathBuf::from("/cfg")),
            home.clone()
        )
        .unwrap()
        .0,
        PathBuf::from("/cfg/herdr/sessions/team.dev/herdr.sock")
    );
    assert_eq!(
        resolve_socket(
            Some(PathBuf::from("/cfg/herdr/herdr.sock")),
            Some("other"),
            Some(PathBuf::from("/cfg")),
            home.clone()
        )
        .unwrap()
        .1,
        "default"
    );
    assert_eq!(
        resolve_socket(
            Some(PathBuf::from("/cfg/herdr/sessions/team/herdr.sock")),
            Some("other"),
            Some(PathBuf::from("/cfg")),
            home.clone()
        )
        .unwrap()
        .1,
        "team"
    );
    assert_eq!(
        resolve_socket(
            Some(PathBuf::from("/custom.sock")),
            Some("other"),
            None,
            home.clone()
        )
        .unwrap()
        .0,
        PathBuf::from("/custom.sock")
    );
    assert!(resolve_socket(None, Some("../bad"), None, home).is_err());
}

#[test]
fn absent_binary_requires_approval_without_creating_any_files() {
    let fixture = Fixture::new();
    let status = fixture.node().ensure(false).unwrap();
    assert!(status.approval_required);
    assert!(!status.installed);
    assert!(!status.running);
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 0);
}

fn pane(cwd: Option<&str>, focused: bool) -> Pane {
    Pane {
        pane_id: "w1:p1".into(),
        tab_id: None,
        cwd: cwd.map(str::to_owned),
        foreground_cwd: Some("/wrong-foreground".into()),
        focused,
    }
}

fn workspace() -> Workspace {
    serde_json::from_value(
        json!({"workspace_id":"w1","label":"Project","pane_count":2,"tab_count":1}),
    )
    .unwrap()
}

#[test]
fn inventory_keeps_legacy_shape_and_uses_checkout_then_focused_shell_cwd() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.0.join(".mindmap")).unwrap();
    fs::write(fixture.0.join(".mindmap/map.figcraft"), b"presence only").unwrap();
    let panes = [
        pane(Some("/unfocused"), false),
        pane(fixture.0.to_str(), true),
    ];
    let agents: Vec<Agent> = serde_json::from_value(json!([
        {"workspace_id":"w1","pane_id":"w1:p1","agent":"codex","agent_status":"working"},
        {"workspace_id":"w1","pane_id":"w1:p2","agent":"codex","agent_status":"done"},
        {"workspace_id":"w2","pane_id":"w2:p1","agent":"claude"}
    ]))
    .unwrap();
    let view = workspace_view(workspace(), &agents, &panes, &["codex".into()]);
    assert_eq!(view.path.as_deref(), fixture.0.to_str());
    assert_eq!(view.map, "map");
    assert_eq!(view.agents, 2);
    assert_eq!(view.agent_names, ["codex"]);
    let json = serde_json::to_value(view).unwrap();
    let keys: BTreeSet<_> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "id",
            "label",
            "path",
            "repo",
            "worktree",
            "branch",
            "agents",
            "agent_names",
            "agent_status",
            "agents_detail",
            "clis",
            "focused",
            "map",
            "tabs",
            "panes"
        ]
        .into_iter()
        .collect()
    );
    assert!(json.get("file").is_none());
    let mut explicit = workspace();
    explicit.worktree = Some(crate::herdr::Worktree {
        checkout_path: Some("/checkout".into()),
        ..Default::default()
    });
    assert_eq!(
        workspace_view(explicit, &[], &panes, &[]).path.as_deref(),
        Some("/checkout")
    );
    assert_eq!(
        pane_path(&[pane(Some("  "), true), pane(Some("/fallback"), false)]).as_deref(),
        Some("/fallback")
    );
}

#[test]
fn worktree_mutations_require_explicit_context() {
    let fixture = Fixture::new();
    let node = fixture.node();
    assert!(node
        .create_worktree(&CreateWorktree::default())
        .unwrap_err()
        .to_string()
        .contains("exactly one"));
    assert!(node
        .create_worktree(&CreateWorktree {
            workspace_id: Some("w1".into()),
            cwd: Some("/repo".into()),
            ..Default::default()
        })
        .is_err());
}

#[test]
fn worktree_creation_uses_explicit_context_and_keeps_ui_focus() {
    let fixture = Fixture::new();
    let node = fixture.node();
    let listener = UnixListener::bind(&node.socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || {
        let mut stream = accept(&listener);
        let request = read_request(&mut stream);
        assert_eq!(request["method"], "worktree.create");
        assert_eq!(
            request["params"],
            json!({"workspace_id":"w1","branch":"feature","focus":false,"trust_repository":false})
        );
        write_json(
            &mut stream,
            json!({"id":"node","result":{"workspace":{"workspace_id":"w2"}}}),
        );
    });
    let result = node
        .create_worktree(&CreateWorktree {
            workspace_id: Some("w1".into()),
            branch: Some("feature".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(result["workspace"]["workspace_id"], "w2");
    server.join().unwrap();
}

#[test]
fn installed_headless_server_starts_on_the_shared_socket_without_install_approval() {
    let fixture = Fixture::new();
    let mut node = fixture.node();
    let binary = fixture.0.join("fake-herdr");
    // This is a disposable fake server, never the installed Herdr and never an agent.
    fs::write(
        &binary,
        r#"#!/usr/bin/env python3
import os, socket, sys, json
assert sys.argv[1:] == ['server']
assert os.environ['HERDR_ENV'] == '1'
s = socket.socket(socket.AF_UNIX)
s.bind(os.environ['HERDR_SOCKET_PATH'])
s.listen(1)
s.settimeout(5)
c, _ = s.accept()
request = json.loads(c.makefile().readline())
assert request['method'] == 'ping'
c.sendall(json.dumps({'id': request['id'], 'result': {'type': 'pong'}}).encode() + b'\n')
c.close()
s.close()
"#,
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    node.requested_binary = binary;
    let status = node.ensure(false).unwrap();
    assert!(status.installed && status.running && !status.approval_required);
    assert_eq!(status.socket, fixture.0.join("herdr.sock"));
}

fn accept(listener: &UnixListener) -> UnixStream {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                return stream;
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(5))
            }
            Err(error) => panic!("mock Herdr accept failed: {error}"),
        }
    }
}

fn read_request(stream: &mut UnixStream) -> Value {
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

fn write_json(stream: &mut UnixStream, value: Value) {
    writeln!(stream, "{value}").unwrap();
}

#[test]
fn monitor_subscribes_before_resync_streams_status_and_recovers_after_disconnect() {
    let fixture = Fixture::new();
    let node = fixture.node();
    let listener = UnixListener::bind(&node.socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || {
        for round in 0..2 {
            let status = if round == 0 { "working" } else { "done" };
            let snapshot = json!({"panes":[{"pane_id":"w1:p1"}],"agents":[{"workspace_id":"w1","pane_id":"w1:p1","agent":"codex","agent_status":status}]});
            let mut first = accept(&listener);
            assert_eq!(read_request(&mut first)["method"], "session.snapshot");
            write_json(
                &mut first,
                json!({"id":"node","result":{"type":"session_snapshot","snapshot":snapshot}}),
            );
            let mut events = accept(&listener);
            let request = read_request(&mut events);
            assert_eq!(request["method"], "events.subscribe");
            assert!(request["params"]["subscriptions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["type"] == "pane.agent_status_changed" && s["pane_id"] == "w1:p1"));
            write_json(
                &mut events,
                json!({"id":"subscribe","result":{"type":"subscription_started"}}),
            );
            let mut second = accept(&listener);
            assert_eq!(read_request(&mut second)["method"], "session.snapshot");
            write_json(
                &mut second,
                json!({"id":"node","result":{"type":"session_snapshot","snapshot":snapshot}}),
            );
            if round == 0 {
                write_json(
                    &mut events,
                    json!({"event":"pane.agent_status_changed","data":{"workspace_id":"w1","pane_id":"w1:p1","agent_status":"blocked","agent":"codex","title":"must not escape into node events"}}),
                );
            } else {
                // Give the receiver time to observe the second resync before closing the peer.
                thread::sleep(Duration::from_millis(100));
            }
        }
    });
    let monitor = node.monitor().unwrap();
    let mut saw_initial = false;
    let mut saw_status = false;
    let mut saw_disconnect = false;
    loop {
        let event = monitor.events.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!serde_json::to_string(&event)
            .unwrap()
            .contains("must not escape"));
        match event {
            HerdrEvent::Resync { agents } if agents[0].status == "working" => saw_initial = true,
            HerdrEvent::AgentStatus { agent } => {
                assert_eq!(agent.status, "blocked");
                saw_status = true;
            }
            HerdrEvent::Disconnected { .. } => saw_disconnect = true,
            HerdrEvent::Resync { agents } if agents[0].status == "done" => break,
            _ => {}
        }
    }
    assert!(saw_initial && saw_status && saw_disconnect);
    drop(monitor);
    server.join().unwrap();
}

/// Explicitly opted-in read-only integration check. It never installs, starts, prompts, or
/// creates anything on the real server, and prints no workspace/agent content.
#[test]
#[ignore = "requires a running local Herdr; read-only inspection"]
fn live_shared_herdr_read_only() {
    let node = NodeHerdr::new(&Config::discover().unwrap()).unwrap();
    assert!(node.status().unwrap().running);
    node.workspaces().unwrap();
    let monitor = node.monitor().unwrap();
    loop {
        match monitor
            .events
            .recv_timeout(Duration::from_secs(15))
            .unwrap()
        {
            HerdrEvent::Resync { .. } => break,
            HerdrEvent::Disconnected { reason } => {
                panic!("read-only subscription failed: {reason}")
            }
            _ => {}
        }
    }
}

#[test]
fn agents_take_their_tab_name_unless_the_tab_only_shows_its_number() {
    let mut agents: Vec<Agent> = serde_json::from_value(json!([
        {"workspace_id":"w1","pane_id":"w1:p1","tab_id":"w1:t1","agent":"codex","terminal_title_stripped":"repo | Ready | Fix tests | main"},
        {"workspace_id":"w1","pane_id":"w1:p2","tab_id":"w1:t2","agent":"claude"},
        {"workspace_id":"w1","pane_id":"w1:p3","agent":"kimi"}
    ]))
    .unwrap();
    let tabs: Vec<Tab> = serde_json::from_value(json!([
        {"tab_id":"w1:t1","number":1,"label":" Backend "},
        {"tab_id":"w1:t2","number":2,"label":"2"}
    ]))
    .unwrap();
    name_tabs(&mut agents, &tabs);
    let labels: Vec<_> = agents.iter().map(|a| a.tab_label.as_deref()).collect();
    assert_eq!(labels, [Some("Backend"), None, None]);
    let view = workspace_view(workspace(), &agents, &[], &[]);
    let detail = serde_json::to_value(&view.agents_detail).unwrap();
    assert_eq!(detail[0]["tab"], "Backend");
    assert!(detail[1].get("tab").is_none());
}
