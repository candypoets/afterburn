//! Black-box tests of the node's multiplexed TCP protocol. Every process, Git worktree and
//! Herdr socket address is disposable; no installed agent or real record is consulted.
//! Sync uses sync.capture for an existing external worker, without prompting an agent.

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use figcraft_collab::{Change, Client, ClientMsg, Hello, NoRecompute, Presence, Prop, ServerMsg};
use figcraft_doc::NodeId;
use serde_json::{json, Value};

const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_FRAME: u64 = 16 * 1024 * 1024;
const DESIGN: &str = ".mindmap/map.figcraft";

struct Process(Child);

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Fixture {
    root: PathBuf,
    worktree: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "mindmap-node-tcp-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self {
            worktree: root.join("worktree"),
            root,
        };
        fs::create_dir(fixture.root.join("repository")).unwrap();
        fs::create_dir(fixture.root.join("empty-template")).unwrap();
        let repository = fixture.root.join("repository");
        fixture.git_at(&repository, &["init", "-q", "-b", "main"]);
        fixture.git_at(
            &repository,
            &["commit", "-qm", "Initial fixture", "--allow-empty"],
        );
        fixture.git_at(
            &repository,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "design",
                fixture.worktree.to_str().unwrap(),
            ],
        );
        fixture
    }

    // Clear inherited Git/Herdr settings, including an agent's GIT_INDEX_FILE. Explicit
    // paths and empty Git configuration also prevent local hooks or signing from running.
    fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut command = Command::new(program);
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_TEMPLATE_DIR", self.root.join("empty-template"))
            .env("GIT_AUTHOR_NAME", "Node TCP Tests")
            .env("GIT_AUTHOR_EMAIL", "node-tests@example.invalid")
            .env("GIT_COMMITTER_NAME", "Node TCP Tests")
            .env("GIT_COMMITTER_EMAIL", "node-tests@example.invalid")
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null());
        command
    }

    fn git_at(&self, path: &Path, args: &[&str]) -> String {
        let stdout = self.root.join("git.stdout");
        let stderr = self.root.join("git.stderr");
        let mut command = self.command("git");
        command
            .arg("-C")
            .arg(path)
            .args(args)
            .stdout(fs::File::create(&stdout).unwrap())
            .stderr(fs::File::create(&stderr).unwrap());
        let mut process = Process(command.spawn().expect("spawn fixture git"));
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = process.0.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "git {args:?}: {}",
                    fs::read_to_string(stderr).unwrap()
                );
                return fs::read_to_string(stdout).unwrap().trim().to_owned();
            }
            assert!(Instant::now() < deadline, "git {args:?} timed out");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn git(&self, args: &[&str]) -> String {
        self.git_at(&self.worktree, args)
    }

    fn disk(&self) -> figcraft_doc::Document {
        let (document, repairs) = figcraft_format::load_file(&self.worktree.join(DESIGN)).unwrap();
        assert!(
            repairs.is_empty(),
            "saved document needs repairs: {repairs:?}"
        );
        document
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Node {
    _process: Process,
    address: SocketAddr,
}

impl Node {
    fn start(fixture: &Fixture) -> Self {
        let stdout = fixture.root.join("node.stdout");
        let stderr = fixture.root.join("node.stderr");
        let mut command = fixture.command(env!("CARGO_BIN_EXE_mindmap-node"));
        command
            .args(["serve", "--bind", "127.0.0.1:0"])
            .current_dir(&fixture.worktree)
            .env("MINDMAP_ROOT", &fixture.root)
            .env("MINDMAP_DATA", fixture.root.join("records"))
            .env("MINDMAP_LEGACY_DATA", fixture.root.join("legacy"))
            .env("MINDMAP_HERDR", fixture.root.join("no-herdr"))
            .env("HERDR_SOCKET_PATH", fixture.root.join("no-herdr.sock"))
            .env("XDG_CONFIG_HOME", fixture.root.join("config"))
            .stdout(fs::File::create(&stdout).unwrap())
            .stderr(fs::File::create(&stderr).unwrap());
        let mut process = Process(command.spawn().expect("spawn isolated mindmap-node"));
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let output = fs::read_to_string(&stdout).unwrap();
            if let Some(address) = output.lines().find_map(|line| {
                line.strip_prefix("mindmap-node protocol 1 on ")
                    .and_then(|address| address.parse::<SocketAddr>().ok())
            }) {
                assert!(address.ip().is_loopback());
                assert_ne!(address.port(), 0);
                return Self {
                    _process: process,
                    address,
                };
            }
            assert!(
                process.0.try_wait().unwrap().is_none(),
                "node exited: {}",
                fs::read_to_string(&stderr).unwrap()
            );
            assert!(
                Instant::now() < deadline,
                "node startup timed out: {}",
                fs::read_to_string(&stderr).unwrap()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn connect(&self) -> Wire {
        let socket = TcpStream::connect_timeout(&self.address, TIMEOUT).unwrap();
        socket.set_write_timeout(Some(TIMEOUT)).unwrap();
        socket.set_nodelay(true).unwrap();
        Wire {
            reader: BufReader::new(socket),
            next_id: 0,
            events: VecDeque::new(),
        }
    }
}

struct Wire {
    reader: BufReader<TcpStream>,
    next_id: u64,
    events: VecDeque<Value>,
}

impl Wire {
    fn read(&mut self, deadline: Instant) -> Value {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .expect("node response deadline expired");
        self.reader
            .get_ref()
            .set_read_timeout(Some(remaining))
            .unwrap();
        let mut line = String::new();
        let count = (&mut self.reader)
            .take(MAX_FRAME + 1)
            .read_line(&mut line)
            .expect("read node frame before deadline");
        assert!(
            count > 0 && count as u64 <= MAX_FRAME,
            "node closed connection or exceeded frame bound"
        );
        let value: Value = serde_json::from_str(&line).expect("node frame must be JSON");
        assert_eq!(value["version"], 1, "{value}");
        value
    }

    fn request_version(&mut self, mut request: Value, version: Option<Value>) -> Value {
        self.next_id += 1;
        request["id"] = json!(self.next_id);
        if let Some(version) = version {
            request["version"] = version;
        }
        serde_json::to_writer(self.reader.get_mut(), &request).unwrap();
        self.reader.get_mut().write_all(b"\n").unwrap();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let response = self.read(deadline);
            if response.get("id") == Some(&json!(self.next_id)) {
                return response;
            }
            assert!(
                response.get("event").is_some(),
                "unexpected response: {response}"
            );
            assert!(
                self.events.len() < 1024,
                "unbounded node events while awaiting {request}"
            );
            self.events.push_back(response);
        }
    }

    fn ok(&mut self, request: Value) -> Value {
        let response = self.request_version(request, Some(json!(1)));
        assert_eq!(response["ok"], true, "{response}");
        response
    }

    fn open(&mut self, fixture: &Fixture) -> String {
        let response = self.ok(json!({"method":"open","path":fixture.worktree}));
        assert_eq!(response["branch"], "design");
        assert_eq!(
            response["path"],
            json!(fs::canonicalize(&fixture.worktree).unwrap())
        );
        response["session"].as_str().unwrap().to_owned()
    }

    fn command(&mut self, session: &str, command: &str, params: Value) -> Value {
        self.ok(
            json!({"method":"session.command","session":session,"command":command,"params":params}),
        )["result"]
            .clone()
    }

    fn create(&mut self, session: &str, name: &str) -> NodeId {
        let result = self.command(
            session,
            "node.create",
            json!({
                "type":"rectangle", "name":name, "width":100, "height":80,
            }),
        );
        serde_json::from_value(result["id"].clone()).expect("node.create returns an id")
    }

    fn inspect(&mut self, session: &str, id: NodeId) -> Value {
        self.ok(json!({"method":"session.inspect","session":session,"node":id}))["document"].clone()
    }
}

fn hello(session: &str, name: &str) -> Hello {
    let ClientMsg::Hello(hello) = Client::hello(session, "", name, "#123456") else {
        panic!("Client::hello must produce Hello")
    };
    hello
}

struct Participant {
    wire: Wire,
    session: String,
    client: Client,
}

impl Participant {
    fn join(mut wire: Wire, session: &str, name: &str) -> Self {
        let response =
            wire.ok(json!({"method":"session.join","session":session,"hello":hello(session,name)}));
        let mut messages: Vec<ServerMsg> =
            serde_json::from_value(response["messages"].clone()).unwrap();
        let welcome = messages
            .iter()
            .position(|message| matches!(message, ServerMsg::Welcome { .. }))
            .expect("join must return a Welcome snapshot");
        let mut client = Client::join(messages.remove(welcome)).expect("FigCraft accepts Welcome");
        assert_eq!(response["peer"], client.id());
        for message in messages {
            assert!(client.receive(message, &NoRecompute).unwrap().is_empty());
        }
        Self {
            wire,
            session: session.to_owned(),
            client,
        }
    }

    fn send(&mut self, message: ClientMsg) {
        self.wire
            .ok(json!({"method":"session.send","session":self.session,"message":message}));
    }

    fn until(&mut self, description: &str, predicate: impl Fn(&Client) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        while !predicate(&self.client) {
            assert!(Instant::now() < deadline, "waiting for {description}");
            let event = match self.wire.events.pop_front() {
                Some(event) => event,
                None => self.wire.read(deadline),
            };
            if event["event"] != "collab" {
                continue;
            }
            assert_eq!(
                event["session"], self.session,
                "misrouted collaboration event"
            );
            let message: ServerMsg = serde_json::from_value(event["message"].clone()).unwrap();
            for outgoing in self
                .client
                .receive(message, &NoRecompute)
                .expect("FigCraft accepts broadcast")
            {
                self.send(outgoing);
            }
        }
    }
}

#[test]
fn source_identity_and_upgrade_lease_protect_prepared_designs() {
    let fixture = Fixture::new();
    let node = Node::start(&fixture);
    let mut wire = node.connect();
    let identity = wire.ok(json!({"method":"hello"}));
    assert_eq!(identity["build_id"], mindmap_build::BUILD_ID);
    assert!(identity["pid"].as_u64().unwrap() > 0);
    let output = fixture
        .command(env!("CARGO_BIN_EXE_mindmap-node"))
        .arg("build-info")
        .output()
        .unwrap();
    assert!(output.status.success());
    let info: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(info["build_id"], identity["build_id"]);
    assert_eq!(info["protocol"], identity["protocol"]);

    let session = wire.open(&fixture);
    let id = wire.create(&session, "before update");
    let captured = wire.ok(json!({"method":"sync.capture","session":session}));
    let job = captured["job"]["id"].clone();
    let lease = wire.ok(json!({"method":"upgrade.prepare"}));
    assert_eq!(lease["pid"], identity["pid"]);
    assert_eq!(lease["expires_in"], 30);
    for request in [
        json!({"method":"sync.prepare","job":job}),
        json!({"method":"merge.apply","session":session,"incoming":"main"}),
        json!({"method":"open","path":fixture.worktree}),
        json!({"method":"upgrade.prepare"}),
    ] {
        let rejected = wire.request_version(request, Some(json!(1)));
        assert_eq!(rejected["ok"], false, "{rejected}");
        assert!(
            rejected["error"]
                .as_str()
                .unwrap()
                .contains("update is pending"),
            "{rejected}"
        );
    }
    // The reservation prevents new file gates, not live edits or status reads.
    wire.command(
        &session,
        "node.set",
        json!({"ids":[id],"name":"live during update"}),
    );
    assert_eq!(wire.inspect(&session, id)["name"], "live during update");
    assert_eq!(
        wire.request_version(
            json!({"method":"upgrade.cancel","lease":"wrong"}),
            Some(json!(1))
        )["ok"],
        false
    );
    wire.ok(json!({"method":"upgrade.cancel","lease":lease["lease"]}));
    wire.ok(json!({"method":"sync.prepare","job":job}));
    let rejected = wire.request_version(json!({"method":"upgrade.prepare"}), Some(json!(1)));
    assert_eq!(rejected["ok"], false, "{rejected}");
    assert!(rejected["error"]
        .as_str()
        .unwrap()
        .contains("prepared Sync/merge"));
    wire.ok(json!({"method":"sync.cancel","job":job}));
    let retry = wire.ok(json!({"method":"upgrade.prepare"}));
    wire.ok(json!({"method":"upgrade.cancel","lease":retry["lease"]}));
}

#[test]
fn protocol_mismatch_rejects_before_open_and_connection_remains_usable() {
    let fixture = Fixture::new();
    let node = Node::start(&fixture);
    let mut wire = node.connect();
    for version in [None, Some(json!(0)), Some(json!(2)), Some(json!("1"))] {
        let response =
            wire.request_version(json!({"method":"open","path":fixture.worktree}), version);
        assert_eq!(response["ok"], false);
        assert!(
            response["error"].as_str().unwrap().contains("protocol"),
            "{response}"
        );
    }
    assert!(wire.ok(json!({"method":"session.list"}))["sessions"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(
        !fixture.worktree.join(DESIGN).exists(),
        "rejected open must not create a design"
    );
    assert_eq!(
        wire.ok(json!({"method":"hello"}))["service"],
        "mindmap-node"
    );
    let session = wire.open(&fixture);
    let mut incompatible = hello(&session, "old client");
    incompatible.protocol += 1;
    let rejected = wire.request_version(
        json!({"method":"session.join","session":session,"hello":incompatible}),
        Some(json!(1)),
    );
    assert_eq!(rejected["ok"], false, "{rejected}");
    let joined = Participant::join(wire, &session, "compatible client");
    assert!(!joined.client.peers().any(|peer| peer.name == "old client"));
}

#[test]
fn two_clients_converge_on_commands_and_concurrent_edits_then_disconnect_cleans_presence() {
    let fixture = Fixture::new();
    let node = Node::start(&fixture);
    let mut first = node.connect();
    let session = first.open(&fixture);
    let mut second = node.connect();
    assert_eq!(
        second.open(&fixture),
        session,
        "one live session per worktree"
    );
    let mut alice = Participant::join(first, &session, "Alice");
    let mut bob = Participant::join(second, &session, "Bob");
    let bob_id = bob.client.id();
    alice.until("Bob joined", |client| {
        client.peers().any(|peer| peer.client_id == bob_id)
    });
    assert!(bob
        .client
        .peers()
        .any(|peer| peer.client_id == alice.client.id()));

    let id = alice.wire.create(&session, "initial");
    alice.until("created node broadcast", |client| {
        client.document().contains(id)
    });
    bob.until("created node broadcast", |client| {
        client.document().contains(id)
    });
    bob.wire.command(
        &session,
        "node.set",
        json!({"ids":[id],"props":{"name":"command edit","width":210}}),
    );
    for participant in [&mut alice, &mut bob] {
        participant.until("command edit broadcast", |client| {
            client.document().node(id).unwrap().name == "command edit"
        });
        assert_eq!(participant.client.document().node(id).unwrap().width, 210.0);
    }
    assert_eq!(alice.client.seq(), bob.client.seq());

    // Race independent command requests on two TCP connections. FigCraft's real client
    // rejects gaps/reordering while applying the broadcasts; both commands must survive.
    let before = alice.client.seq();
    thread::scope(|scope| {
        let a = scope.spawn(|| {
            alice
                .wire
                .command(&session, "node.set", json!({"ids":[id],"props":{"x":42}}))
        });
        let b = scope.spawn(|| {
            bob.wire
                .command(&session, "node.set", json!({"ids":[id],"props":{"y":17}}))
        });
        a.join().expect("Alice's command completed");
        b.join().expect("Bob's command completed");
    });
    for participant in [&mut alice, &mut bob] {
        participant.until("concurrent commands delivered in sequence", |client| {
            client.seq() >= before + 2
        });
        assert_eq!(participant.client.seq(), before + 2);
        assert_eq!(
            participant
                .client
                .document()
                .node(id)
                .unwrap()
                .transform
                .as_coeffs(),
            [1.0, 0.0, 0.0, 1.0, 42.0, 17.0]
        );
    }

    // Both clients prepare edits against the same confirmed revision, before either sends.
    // Different properties must survive the server's ordering and client-side rebasing.
    let mut a = alice.client.document().clone();
    a.node_mut(id).unwrap().name = "Alice's edit".into();
    let alice_edit = alice.client.edit(&a).unwrap().unwrap();
    let mut b = bob.client.document().clone();
    b.node_mut(id).unwrap().height = 155.0;
    let bob_edit = bob.client.edit(&b).unwrap().unwrap();
    alice.send(alice_edit);
    bob.send(bob_edit);
    for participant in [&mut alice, &mut bob] {
        participant.until("both edits confirmed", |client| {
            let node = client.document().node(id).unwrap();
            client.pending() == 0 && node.name == "Alice's edit" && node.height == 155.0
        });
        assert!(participant.client.take_notices().is_empty());
    }
    assert_eq!(alice.client.seq(), bob.client.seq());
    assert_eq!(
        alice.client.confirmed().snapshot().unwrap(),
        bob.client.confirmed().snapshot().unwrap(),
        "both clients must converge on the complete document"
    );
    assert_eq!(
        alice.wire.inspect(&session, id),
        bob.wire.inspect(&session, id)
    );

    bob.send(ClientMsg::Presence(Presence {
        selection: Some(vec![id]),
        ..Presence::default()
    }));
    alice.until("Bob's presence", |client| {
        client.presence_of(bob_id).is_some()
    });
    drop(bob); // Abrupt TCP EOF, with no session.leave request.
    alice.until("Bob disconnected", |client| {
        !client.peers().any(|peer| peer.client_id == bob_id)
    });
    assert!(alice.client.presence_of(bob_id).is_none());
    let replacement = Participant::join(node.connect(), &session, "Replacement");
    assert_ne!(replacement.client.id(), bob_id);
    assert!(!replacement
        .client
        .peers()
        .any(|peer| peer.client_id == bob_id));
    assert_eq!(
        replacement.client.document().node(id).unwrap().name,
        "Alice's edit"
    );
}

#[test]
fn explicit_save_survives_process_restart_and_hydrates_a_fresh_client() {
    let fixture = Fixture::new();
    let node = Node::start(&fixture);
    let mut wire = node.connect();
    let session = wire.open(&fixture);
    let id = wire.create(&session, "persistent rectangle");
    wire.command(
        &session,
        "node.set",
        json!({"ids":[id],"props":{"x":42,"y":-17,"width":321,"height":123}}),
    );
    let before = wire.inspect(&session, id);
    wire.ok(json!({"method":"session.save","session":session}));
    let disk = fixture.disk();
    assert_eq!(disk.node(id).unwrap().name, "persistent rectangle");
    assert_eq!(disk.node(id).unwrap().width, 321.0);
    assert_eq!(disk.node(id).unwrap().height, 123.0);
    drop(wire);
    drop(node);

    let restarted = Node::start(&fixture);
    let mut wire = restarted.connect();
    assert_eq!(wire.open(&fixture), session);
    assert_eq!(wire.inspect(&session, id), before);
    let joined = Participant::join(wire, &session, "after restart");
    let restored = joined.client.document().node(id).unwrap();
    assert_eq!(restored.name, "persistent rectangle");
    assert_eq!(restored.width, 321.0);
    assert_eq!(
        restored.transform.as_coeffs(),
        [1.0, 0.0, 0.0, 1.0, 42.0, -17.0]
    );
    assert!(!fixture.root.join("records/projects.json").exists());
    assert!(!fixture.root.join("legacy/projects.json").exists());
}

#[test]
fn diff_reports_geometry_values_and_complete_paginated_changes_from_git_baseline() {
    let fixture = Fixture::new();
    let node = Node::start(&fixture);
    let mut wire = node.connect();
    let session = wire.open(&fixture);
    let id = wire.create(&session, "geometry");
    wire.ok(json!({"method":"session.save","session":session}));
    fixture.git(&["add", DESIGN]);
    fixture.git(&["commit", "-qm", "Synced fixture", "-m", "Mindmap-Sync: all"]);
    let baseline = fixture.git(&["rev-parse", "HEAD"]);
    let clean = wire.ok(json!({"method":"diff","session":session}));
    assert_eq!(clean["last"]["commit"], baseline);
    assert!(clean["changes"].as_array().unwrap().is_empty(), "{clean}");

    // Leave name/text unchanged: a text-only summary would miss every one of these changes.
    wire.command(
        &session,
        "node.set",
        json!({"ids":[id],"props":{
            "x":42,"y":-17,"width":321,"height":123,"cornerRadius":12,"opacity":0.5,
        }}),
    );
    let diff = wire.ok(json!({"method":"diff","session":session}));
    assert_eq!(diff["last"]["commit"], baseline);
    assert!(diff["map"]["added"].as_array().unwrap().is_empty());
    assert!(diff["map"]["removed"].as_array().unwrap().is_empty());
    let changed = diff["map"]["changed"].as_array().unwrap();
    assert_eq!(changed.len(), 1, "{diff}");
    assert_eq!(changed[0]["id"], json!(id));
    let fields = changed[0]["fields"].as_array().unwrap();
    for field in ["transform", "width", "height", "corners", "opacity"] {
        assert!(fields.contains(&json!(field)), "missing {field}: {diff}");
    }
    let changes: Vec<Change> = serde_json::from_value(diff["changes"].clone()).unwrap();
    let mut values = BTreeMap::new();
    for change in changes {
        if let Change::Set {
            id: changed_id,
            prop,
            value,
        } = change
        {
            if changed_id == id {
                values.insert(prop, value.0);
            }
        }
    }
    assert_eq!(values.get(&Prop::Width), Some(&json!(321.0)));
    assert_eq!(values.get(&Prop::Height), Some(&json!(123.0)));
    assert_eq!(values.get(&Prop::Opacity), Some(&json!(0.5)));
    assert_eq!(
        values.get(&Prop::Corners).unwrap()["radii"],
        json!([12.0, 12.0, 12.0, 12.0])
    );
    assert_eq!(
        values.get(&Prop::Transform),
        Some(&json!([1.0, 0.0, 0.0, 1.0, 42.0, -17.0]))
    );

    let total = diff["changes"].as_array().unwrap().len();
    let mut collected = Vec::new();
    let mut offset = 0;
    loop {
        assert!(offset < total, "pagination must terminate");
        let page = wire.ok(json!({"method":"diff","session":session,"offset":offset,"limit":2}));
        assert_eq!(page["pagination"]["changes"]["total"], total);
        assert_eq!(
            page["map"], diff["map"],
            "summary remains complete on every page"
        );
        let chunk = page["changes"].as_array().unwrap();
        assert!(!chunk.is_empty() && chunk.len() <= 2);
        collected.extend(chunk.iter().cloned());
        match page["pagination"]["changes"]["next"].as_u64() {
            Some(next) => {
                assert!(next as usize > offset);
                offset = next as usize;
            }
            None => break,
        }
    }
    assert_eq!(
        json!(collected),
        diff["changes"],
        "pagination cannot omit or duplicate geometry"
    );
    assert_eq!(fixture.disk().node(id).unwrap().name, "geometry");
}

#[test]
fn captured_sync_survives_restart_commits_exact_frozen_design_and_keeps_later_edits_as_drift() {
    let fixture = Fixture::new();
    let node = Node::start(&fixture);
    let mut wire = node.connect();
    let session = wire.open(&fixture);
    let id = wire.create(&session, "baseline design");
    wire.ok(json!({"method":"session.save","session":session}));
    fs::write(fixture.worktree.join("feature.rs"), "// baseline code\n").unwrap();
    fixture.git(&["add", DESIGN, "feature.rs"]);
    fixture.git(&["commit", "-qm", "Before Sync"]);
    let start_head = fixture.git(&["rev-parse", "HEAD"]);

    wire.command(
        &session,
        "node.set",
        json!({"ids":[id],"props":{
            "name":"captured design", "width":220,
        }}),
    );
    let captured = wire.ok(json!({"method":"sync.capture","session":session}));
    let job = captured["job"]["id"].as_str().unwrap().to_owned();
    let trailer = captured["trailer"].as_str().unwrap().to_owned();
    assert_eq!(captured["job"]["state"], "in_flight");
    assert_eq!(captured["job"]["agent"]["prompted"], false);
    assert!(trailer.starts_with("Mindmap-Sync: all "), "{captured}");
    let duplicate = wire.request_version(
        json!({"method":"sync.capture","session":session}),
        Some(json!(1)),
    );
    assert_eq!(duplicate["ok"], false, "only one active Sync per session");

    wire.command(
        &session,
        "node.set",
        json!({"ids":[id],"props":{
            "name":"edited after capture", "width":321,
        }}),
    );
    let prepared = wire.ok(json!({"method":"sync.prepare","job":job}));
    let frozen_blob = prepared["blob"].as_str().unwrap().to_owned();
    assert_eq!(prepared["trailer"], trailer);
    assert_eq!(prepared["start_head"], start_head);
    assert_eq!(fixture.git(&["hash-object", DESIGN]), frozen_blob);
    assert_eq!(
        fixture.git(&["rev-parse", &format!(":{DESIGN}")]),
        frozen_blob
    );
    let frozen = fixture.disk();
    assert_eq!(frozen.node(id).unwrap().name, "captured design");
    assert_eq!(frozen.node(id).unwrap().width, 220.0);
    assert_eq!(frozen.node(id).unwrap().height, 80.0);

    // The live session must remain editable even while disk and the index hold the snapshot.
    wire.command(
        &session,
        "node.set",
        json!({"ids":[id],"props":{"height":177}}),
    );
    let live = wire.inspect(&session, id);
    assert_eq!(live["name"], "edited after capture");
    assert_eq!(live["width"], 321.0);
    assert_eq!(live["height"], 177.0);
    let blocked_save = wire.request_version(
        json!({"method":"session.save","session":session}),
        Some(json!(1)),
    );
    assert_eq!(
        blocked_save["ok"], false,
        "save cannot overwrite a prepared snapshot"
    );
    let premature = wire.request_version(
        json!({"method":"sync.finish","job":job,"commit":start_head}),
        Some(json!(1)),
    );
    assert_eq!(
        premature["ok"], false,
        "an old commit cannot complete this job"
    );
    assert_eq!(fixture.git(&["hash-object", DESIGN]), frozen_blob);
    drop(wire);
    drop(node);

    let restarted = Node::start(&fixture);
    let mut wire = restarted.connect();
    assert_eq!(wire.open(&fixture), session);
    let recovered = wire.ok(json!({"method":"sync_status","job":job}));
    assert_eq!(recovered["job"]["state"], "in_flight");
    assert_eq!(recovered["job"]["prepared"], true);
    assert_eq!(wire.inspect(&session, id), live);
    assert_eq!(fixture.git(&["hash-object", DESIGN]), frozen_blob);
    assert_eq!(
        wire.ok(json!({"method":"sync.prepare","job":job}))["blob"],
        frozen_blob,
        "prepare is retryable after a process restart"
    );

    // Simulate an external worker committing code and design together. git add -A must
    // stage the captured document, never edits made after capture or after prepare.
    fs::write(
        fixture.worktree.join("feature.rs"),
        "// implements the captured design\n",
    )
    .unwrap();
    fixture.git(&["add", "-A"]);
    assert_eq!(
        fixture.git(&["rev-parse", &format!(":{DESIGN}")]),
        frozen_blob
    );
    fixture.git(&["commit", "-qm", "Implement captured design", "-m", &trailer]);
    let commit = fixture.git(&["rev-parse", "HEAD"]);
    assert_eq!(
        fixture.git(&["rev-parse", &format!("HEAD:{DESIGN}")]),
        frozen_blob
    );
    assert_eq!(
        fixture.git(&["show", "HEAD:feature.rs"]),
        "// implements the captured design"
    );
    assert_eq!(
        fixture.git(&["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"]),
        format!("{DESIGN}\nfeature.rs"),
        "one commit contains code and the frozen design only"
    );
    let finished = wire.ok(json!({"method":"sync.finish","job":job,"commit":commit}));
    assert_eq!(finished["job"]["state"], "done");
    assert_eq!(finished["job"]["commit"], commit);
    assert_eq!(finished["job"]["prepared"], false);
    assert_eq!(wire.inspect(&session, id), live);
    assert_eq!(
        fixture.disk().node(id).unwrap().name,
        "edited after capture"
    );
    assert_eq!(fixture.disk().node(id).unwrap().width, 321.0);
    assert_eq!(fixture.disk().node(id).unwrap().height, 177.0);

    let drift = wire.ok(json!({"method":"diff","session":session}));
    assert_eq!(drift["last"]["commit"], commit);
    let changes: Vec<Change> = serde_json::from_value(drift["changes"].clone()).unwrap();
    for (property, expected) in [
        (Prop::Name, json!("edited after capture")),
        (Prop::Width, json!(321.0)),
        (Prop::Height, json!(177.0)),
    ] {
        assert!(changes.iter().any(|change| matches!(change,
            Change::Set { id: changed, prop, value } if *changed == id && *prop == property && value.0 == expected
        )), "later {property:?} edit must remain drift: {drift}");
    }
    assert!(
        drift["code"]["files"].as_array().unwrap().is_empty(),
        "code was committed: {drift}"
    );
    wire.ok(json!({"method":"session.save","session":session}));
    drop(wire);
    drop(restarted);

    let restarted = Node::start(&fixture);
    let mut wire = restarted.connect();
    assert_eq!(wire.open(&fixture), session);
    let recovered = wire.ok(json!({"method":"sync_status","job":job}));
    assert_eq!(recovered["job"]["state"], "done");
    assert_eq!(recovered["job"]["commit"], commit);
    assert_eq!(recovered["last"]["commit"], commit);
    assert_eq!(wire.inspect(&session, id), live);
}

#[test]
fn a_recorded_explanation_commits_only_its_own_edits_and_sync_then_sees_only_the_waiting_one() {
    let fixture = Fixture::new();
    let node = Node::start(&fixture);
    let mut wire = node.connect();
    let session = wire.open(&fixture);
    wire.create(&session, "recorded card");
    wire.ok(json!({"method":"session.save","session":session}));
    fixture.git(&["add", DESIGN]);
    fixture.git(&["commit", "-qm", "Sync\n\nMindmap-Sync: all job=seed"]);

    // A design edit waits; then a request adds an explanation note.
    let retry = wire.create(&session, "retry button");
    let request = wire.ok(json!({"method":"record.start","session":session}))["request"]
        .as_str()
        .unwrap()
        .to_owned();
    let note = wire.create(&session, "explanation note");
    let bad = wire.request_version(
        json!({"method":"record","session":session,"request":request,"kind":"proposal"}),
        Some(json!(1)),
    );
    assert_eq!(bad["ok"], false, "a design edit is never recorded");

    let recorded =
        wire.ok(json!({"method":"record","session":session,"request":request,"kind":"explain"}));
    let job = recorded["job"].as_str().unwrap().to_owned();
    let trailer = recorded["trailer"].as_str().unwrap().to_owned();
    assert!(
        trailer.starts_with("Mindmap-Sync: all job=") && trailer.ends_with(" explain"),
        "{recorded}"
    );
    assert_eq!(recorded["kept_waiting"], json!([]));
    let staged = fixture.disk();
    assert!(
        staged.node(note).is_some(),
        "the request's note is recorded"
    );
    assert!(staged.node(retry).is_none(), "the waiting edit is not");

    fixture.git(&["commit", "-qm", &format!("Explain the card\n\n{trailer}")]);
    let commit = fixture.git(&["rev-parse", "HEAD"]);
    let finished = wire.ok(json!({"method":"sync.finish","job":job,"commit":commit}));
    assert_eq!(finished["job"]["state"], "done");
    assert_eq!(finished["job"]["kind"], "explain");
    assert!(
        fixture.disk().node(retry).is_some(),
        "the live map is saved again after the record"
    );

    let diff = wire.ok(json!({"method":"diff","session":session}));
    let added: Vec<&str> = diff["map"]["added"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(added, ["retry button"], "{diff}");
    assert_eq!(diff["last"]["kind"], "explain");
    assert_eq!(diff["last_sync"]["kind"], "sync");
}
