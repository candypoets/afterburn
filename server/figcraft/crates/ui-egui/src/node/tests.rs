#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use std::net::TcpListener;
use std::time::Instant;

fn read(reader: &mut BufReader<TcpStream>) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}
fn write(stream: &mut TcpStream, value: Value) {
    writeln!(stream, "{value}").unwrap();
}
fn handshake(reader: &mut BufReader<TcpStream>, stream: &mut TcpStream) {
    let request = read(reader);
    assert_eq!(request["version"], 1);
    assert_eq!(request["method"], "hello");
    assert_eq!(request["build_id"], mindmap_build::BUILD_ID);
    write(stream, json!({"id":request["id"],"version":1,"ok":true,"protocol":1,"build_id":mindmap_build::BUILD_ID}));
}
fn listener() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    (listener, address)
}

#[test]
fn concurrent_rpc_and_events_share_one_socket_and_match_out_of_order_ids() {
    let (listener, address) = listener();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        handshake(&mut reader, &mut stream);
        let a = read(&mut reader);
        let b = read(&mut reader);
        assert_ne!(a["id"], b["id"]);
        write(&mut stream, json!({"version":1,"event":"collab","session":"design","message":ServerMsg::Pong}));
        write(&mut stream, json!({"version":1,"event":"herdr","data":{"type":"agents"}}));
        write(&mut stream, json!({"version":1,"event":"job","job":{"id":"j1","state":"done"}}));
        for req in [b, a] {
            write(&mut stream, json!({"version":1,"id":req["id"],"ok":true,"echo":req["method"]}));
        }
        listener.set_nonblocking(true).unwrap();
        assert!(listener.accept().is_err(), "must reuse the same machine connection");
    });
    let connection = connection(&address).unwrap();
    let events = connection.subscribe("design");
    let other = address.clone();
    let a = std::thread::spawn(move || rpc(&other, json!({"method":"a","id":1}), RPC_TIMEOUT).unwrap());
    let b = rpc(&address, json!({"method":"b","id":1}), RPC_TIMEOUT).unwrap();
    assert_eq!(a.join().unwrap()["echo"], "a");
    assert_eq!(b["echo"], "b");
    assert_eq!(events.recv_timeout(Duration::from_secs(2)).unwrap().unwrap()["event"], "collab");
    let pushed = take_events().into_iter().filter(|(host, _)| host == &address).map(|(_, event)| event).collect::<Vec<_>>();
    assert!(pushed.iter().any(|event| event["event"] == "herdr"));
    assert!(pushed.iter().any(|event| event["event"] == "job" && event["job"]["id"] == "j1"));
    server.join().unwrap();
}

#[test]
fn incompatible_protocol_is_an_explicit_error() {
    let (listener, address) = listener();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let hello = read(&mut reader);
        write(&mut stream, json!({"version":2,"id":hello["id"],"ok":true,"protocol":2}));
    });
    let error = rpc(&address, json!({"method":"workspaces"}), RPC_TIMEOUT).unwrap_err();
    assert!(error.contains("Incompatible"), "{error}");
    server.join().unwrap();
}

#[test]
fn disconnect_fails_pending_requests_instead_of_faking_success() {
    let (listener, address) = listener();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        handshake(&mut reader, &mut stream);
        assert_eq!(read(&mut reader)["method"], "send");
    });
    assert!(rpc(&address, json!({"method":"send"}), Duration::from_secs(2)).is_err());
    server.join().unwrap();
}

#[test]
fn shutdown_closes_retained_connections_fails_waiters_and_terminates_tunnel() {
    let (listener, address) = listener();
    let (sent, received) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        handshake(&mut reader, &mut stream);
        assert_eq!(read(&mut reader)["method"], "waiting");
        sent.send(()).unwrap();
        assert_eq!(reader.read(&mut [0]).unwrap(), 0, "shutdown must close the socket");
    });
    // An isolated pool lets this exercise the production shutdown without affecting parallel tests.
    let pool = Pool::default();
    let connection = pool.connection(&address).unwrap();
    let events = connection.subscribe("design");
    #[cfg(unix)]
    let tunnel_exited = {
        let mut child = std::process::Command::new("sleep").arg("30").stdout(std::process::Stdio::piped()).spawn().unwrap();
        let mut output = child.stdout.take().unwrap();
        *connection.tunnel.lock().unwrap() = Some(child);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || tx.send(output.read(&mut [0]).unwrap()).unwrap());
        rx
    };
    let request = connection.clone();
    let waiting = std::thread::spawn(move || request.request(json!({"method":"waiting"}), Duration::from_secs(5)));
    received.recv_timeout(Duration::from_secs(5)).unwrap();
    pool.shutdown();
    pool.shutdown(); // Idempotent, even with a caller retaining a strong connection reference.
    assert!(connection.closed.load(Ordering::SeqCst));
    assert!(pool.slots.lock().unwrap().is_empty());
    assert!(pool.connection(&address).is_err(), "shutdown must prevent reconnection");
    assert!(waiting.join().unwrap().is_err());
    assert!(events.recv_timeout(Duration::from_secs(2)).unwrap().is_err());
    assert!(connection.tunnel.lock().unwrap().is_none());
    #[cfg(unix)]
    assert_eq!(tunnel_exited.recv_timeout(Duration::from_secs(2)).unwrap(), 0, "the sleeping tunnel child must have exited");
    server.join().unwrap();
}

#[test]
fn engine_gets_real_welcome_presence_and_ordered_send_on_same_connection() {
    use figcraft_collab::{Limits, Outgoing, Server};
    let (listener, address) = listener();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        handshake(&mut reader, &mut stream);
        let request = read(&mut reader);
        assert_eq!(request["method"], "session.join");
        assert_eq!(request["session"], "tree");
        let hello: Hello = serde_json::from_value(request["hello"].clone()).unwrap();
        assert!(hello.token.is_empty());
        let mut host = Server::new(figcraft_doc::Document::new("Tree"), "", Limits::default());
        let (peer, messages) = host.join(hello).unwrap();
        let messages: Vec<ServerMsg> = messages
            .into_iter()
            .filter_map(|out| match out {
                Outgoing::To(to, msg) if to == peer => Some(msg),
                _ => None,
            })
            .collect();
        write(&mut stream, json!({"version":1,"id":request["id"],"ok":true,"peer":peer,"messages":messages}));
        write(
            &mut stream,
            json!({"version":1,"event":"collab","session":"tree","message":ServerMsg::PeerJoined(message::Peer {client_id:42,name:"Agent".into(),color:"#0D99FF".into()})}),
        );
        write(
            &mut stream,
            json!({"version":1,"event":"collab","session":"tree","message":ServerMsg::Presence {client_id:42,presence:Presence::default()}}),
        );
        loop {
            let request = read(&mut reader);
            assert_eq!(request["session"], "tree");
            assert!(matches!(request["method"].as_str(), Some("session.send" | "session.leave")));
            write(&mut stream, json!({"version":1,"id":request["id"],"ok":true}));
            if request["method"] == "session.leave" {
                break;
            }
        }
    });
    let mut engine = figcraft_engine::Session::empty();
    engine.live.install(Box::new(NodeLiveLink::new(Arc::new(|| {}))));
    let target = SessionRef { address, session: "tree".into(), workspace: "w1".into(), path: "/repo".into(), branch: "main".into() };
    let joined = engine.execute("collab.join", &json!({"link":target.route()})).unwrap();
    assert!(joined["session"].as_u64().is_some());
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        engine.live_poll();
        if engine.live().presence.contains_key(&42) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(engine.live().status, Status::Live);
    assert_eq!(engine.doc().unwrap().name, "Tree");
    assert!(engine.live().presence.contains_key(&42));
    engine.execute("collab.leave", &json!({})).unwrap();
    server.join().unwrap();
}

/// Opt-in smoke test against a running node; joins and reads, never edits the design.
#[test]
#[ignore = "requires MINDMAP_TEST_SESSION and a running loopback node"]
fn live_node_opens_its_existing_document_through_the_engine() {
    let session = std::env::var("MINDMAP_TEST_SESSION").unwrap();
    let address = std::env::var("MINDMAP_TEST_NODE").unwrap_or_else(|_| "127.0.0.1:47822".into());
    let mut engine = figcraft_engine::Session::empty();
    engine.live.install(Box::new(NodeLiveLink::new(Arc::new(|| {}))));
    let target =
        SessionRef { address: address.clone(), session: session.clone(), workspace: String::new(), path: String::new(), branch: String::new() };
    engine.execute("collab.join", &json!({"link":target.route()})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        engine.live_poll();
        if engine.live().status == Status::Live && engine.doc().is_ok() {
            break;
        }
        assert!(!matches!(engine.live().status, Status::Failed { .. }), "{:?}", engine.live().status);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(engine.live().status, Status::Live);
    assert!(!engine.doc().unwrap().pages.is_empty());
    let inspected = rpc(&address, json!({"method":"session.inspect","session":session,"depth":1}), RPC_TIMEOUT).unwrap();
    assert_eq!(inspected["ok"], true);
    assert!(inspected["document"].is_object());
    engine.execute("collab.leave", &json!({})).unwrap();
}

#[test]
fn legacy_and_different_builds_block_workspace_rpc_but_reuse_socket_for_update_preflight() {
    for build in [Value::Null, json!("different-build"), json!("")] {
        let (listener, address) = listener();
        let (release, wait) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let hello = read(&mut reader);
            let mut response = json!({"id":hello["id"],"version":PROTOCOL,"protocol":PROTOCOL,"ok":true});
            if !build.is_null() {
                response["build_id"] = build;
            }
            write(&mut stream, response);
            let request = read(&mut reader);
            assert_eq!(request["method"], "session.list", "workspace mutation must never reach the node");
            write(&mut stream, json!({"id":request["id"],"version":PROTOCOL,"ok":true,"sessions":[]}));
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
            listener.set_nonblocking(true).unwrap();
            assert!(listener.accept().is_err(), "update inspection must reuse the existing connection");
        });
        let reason = update_required(&address).unwrap().unwrap();
        assert!(reason.contains("Update node"));
        assert!(reason.contains(mindmap_build::BUILD_ID));
        assert_eq!(rpc(&address, json!({"method":"session.command"}), RPC_TIMEOUT).unwrap_err(), reason);
        assert_eq!(rpc(&address, json!({"method":"host.ensure"}), RPC_TIMEOUT).unwrap_err(), reason);
        let sessions = rpc(&address, json!({"method":"session.list"}), RPC_TIMEOUT).unwrap();
        check_update_sessions(&sessions).unwrap();
        release.send(()).unwrap();
        server.join().unwrap();
        disconnect(&address);
    }
}

#[test]
fn update_preflight_requires_explicit_safe_flags_for_every_session() {
    for reply in [
        json!({}),
        json!({"ok":false,"sessions":[]}),
        json!({"ok":true,"sessions":null}),
        json!({"ok":true,"sessions":[{}]}),
        json!({"ok":true,"sessions":[{"syncPrepared":true,"mergeReady":false}]}),
        json!({"ok":true,"sessions":[{"syncPrepared":false,"mergeReady":true}]}),
        json!({"ok":true,"sessions":[{"syncPrepared":false,"mergeReady":false},{"syncPrepared":null,"mergeReady":false}]}),
    ] {
        assert!(check_update_sessions(&reply).unwrap_err().contains("Update pending"), "{reply}");
    }
    check_update_sessions(&json!({"ok":true,"sessions":[]})).unwrap();
    check_update_sessions(&json!({"ok":true,"sessions":[{"syncPrepared":false,"mergeReady":false}]})).unwrap();
}

#[test]
fn disconnect_after_update_reopens_same_pool_slot_and_verifies_new_build() {
    let (listener, address) = listener();
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            handshake(&mut reader, &mut stream);
            assert_eq!(reader.read(&mut [0]).unwrap(), 0);
        }
    });
    let pool = Pool::default();
    let first = pool.connection(&address).unwrap();
    pool.disconnect(&address);
    let second = pool.connection(&address).unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    assert!(first.closed.load(Ordering::SeqCst));
    assert_eq!(pool.slots.lock().unwrap().len(), 1);
    assert!(second.update_required.get().unwrap().is_none());
    pool.shutdown();
    server.join().unwrap();
}
