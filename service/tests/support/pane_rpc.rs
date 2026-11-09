use super::*;
use std::os::unix::net::UnixListener;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[test]
fn pane_requests_route_through_node_with_workspace_and_confirmation_guards() {
    let fixture = Fixture::new();
    let listener = UnixListener::bind(fixture.root.join("no-herdr.sock")).unwrap();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = Arc::clone(&stop);
    let server = thread::spawn(move || {
        let deadline = Instant::now() + TIMEOUT * 3;
        let mut calls = Vec::new();
        while !stopping.load(Ordering::Relaxed) && Instant::now() < deadline {
            let (mut stream, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("fake Herdr accept: {error}"),
            };
            stream.set_read_timeout(Some(TIMEOUT)).unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream).read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let mut response = match request["method"].as_str().unwrap() {
                // The node's monitor is independent of these RPCs. Keep it disconnected so
                // this test exercises only request routing and never needs a live session.
                "session.snapshot" => json!({"error":{"code":"test_monitor_unavailable"}}),
                "pane.get" => json!({"result":{"type":"pane_info","pane":{
                    "pane_id":"w1:p1","workspace_id":"w1"
                }}}),
                "pane.rename" if request["params"]["label"] == "rejected" => {
                    json!({"error":{"code":"invalid_label","message":"private upstream detail"}})
                }
                "pane.rename" => json!({"result":{"type":"pane_info","pane":{
                    "pane_id":"w1:p1","workspace_id":"w1","label":request["params"]["label"]
                }}}),
                "pane.close" => json!({"result":{"type":"ok"}}),
                method => panic!("unexpected Herdr method {method}"),
            };
            if request["method"] != "session.snapshot" {
                calls.push(json!({"method":request["method"],"params":request["params"]}));
            }
            response["id"] = request["id"].clone();
            writeln!(stream, "{response}").unwrap();
        }
        calls
    });
    let node = Node::start(&fixture);
    let mut wire = node.connect();
    let renamed = wire.ok(json!({
        "method":"pane.rename","workspace":"w1","pane":"w1:p1","label":"Backend"
    }));
    assert_eq!(renamed["pane"]["label"], "Backend");
    wire.ok(json!({"method":"pane.close","workspace":"w1","pane":"w1:p1","confirm":true}));

    for (request, message) in [
        (
            json!({"method":"pane.rename","pane":"w1:p1","label":"Backend"}),
            "workspace",
        ),
        (
            json!({"method":"pane.close","workspace":"w1","confirm":true}),
            "pane",
        ),
        (
            json!({"method":"pane.close","workspace":"w1","pane":"w1:p1"}),
            "confirm: true",
        ),
        (
            json!({"method":"pane.rename","workspace":"w2","pane":"w1:p1","label":"Backend"}),
            "does not belong",
        ),
        (
            json!({"method":"pane.rename","workspace":"w1","pane":"w1:p1","label":"rejected"}),
            "invalid_label",
        ),
    ] {
        let response = wire.request_version(request, Some(json!(1)));
        assert_eq!(response["ok"], false, "{response}");
        assert!(
            response["error"].as_str().unwrap().contains(message),
            "{response}"
        );
        assert!(!response.to_string().contains("private upstream detail"));
    }
    stop.store(true, Ordering::Relaxed);
    let calls = server.join().unwrap();
    assert_eq!(
        calls,
        vec![
            json!({"method":"pane.get","params":{"pane_id":"w1:p1"}}),
            json!({"method":"pane.rename","params":{"pane_id":"w1:p1","label":"Backend"}}),
            json!({"method":"pane.get","params":{"pane_id":"w1:p1"}}),
            json!({"method":"pane.close","params":{"pane_id":"w1:p1"}}),
            json!({"method":"pane.get","params":{"pane_id":"w1:p1"}}),
            json!({"method":"pane.get","params":{"pane_id":"w1:p1"}}),
            json!({"method":"pane.rename","params":{"pane_id":"w1:p1","label":"rejected"}}),
        ]
    );
}
