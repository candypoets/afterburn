use super::*;
use figcraft_collab::{Json, Prop};
use std::sync::atomic::{AtomicU64, Ordering};

struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "mindmap-documents-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        let repo = Self(path);
        repo.git(&["init", "-q", "-b", "main"]);
        repo.git(&["config", "user.name", "Node Tests"]);
        repo.git(&["config", "user.email", "node-tests@example.invalid"]);
        repo
    }
    fn git(&self, args: &[&str]) {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&self.0)
                .args(args)
                .output()
                .unwrap()
                .status
                .success(),
            "git {args:?}"
        );
    }
    fn disk(&self) -> Document {
        decode(&std::fs::read(self.0.join(DESIGN_PATH)).unwrap()).unwrap()
    }
    fn write(&self, doc: &Document) {
        std::fs::create_dir_all(self.0.join(".mindmap")).unwrap();
        atomic_write(&self.0.join(DESIGN_PATH), &encode(doc).unwrap()).unwrap();
    }
    fn commit(&self) {
        self.git(&["add", ".mindmap/map.figcraft"]);
        self.git(&["commit", "-qm", "design"]);
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn hello(name: &str) -> Hello {
    Hello {
        protocol: figcraft_collab::message::PROTOCOL,
        file: String::new(),
        token: String::new(),
        name: name.into(),
        color: "#123456".into(),
        client_version: "mindmap-node/1".into(),
        resume: None,
    }
}
fn create(manager: &mut NodeDocuments, session: &str, name: &str) -> NodeId {
    let (value, outgoing) = manager
        .execute(
            session,
            "node.create",
            &json!({"type":"rectangle","name":name,"width":100,"height":80}),
        )
        .unwrap();
    assert!(outgoing
        .iter()
        .any(|o| matches!(o, Outgoing::All(ServerMsg::Changes(_)))));
    serde_json::from_value(value["id"].clone()).unwrap()
}
fn edit(manager: &mut NodeDocuments, session: &str, id: NodeId, name: &str) {
    manager
        .execute(
            session,
            "node.set",
            &json!({"ids":[id],"props":{"name":name}}),
        )
        .unwrap();
}

#[test]
fn save_and_restart_preserve_unsaved_authority_and_peer_id_space() {
    let repo = Repo::new();
    let mut manager = NodeDocuments::new();
    let meta = manager.open(&repo.0).unwrap();
    let node = create(&mut manager, &meta.id, "saved");
    manager.save(&meta.id).unwrap();
    assert_eq!(repo.disk().node(node).unwrap().name, "saved");
    edit(&mut manager, &meta.id, node, "cached after save");
    let (peer, _) = manager.join(&meta.id, hello("before restart")).unwrap();
    drop(manager);
    let mut manager = NodeDocuments::new();
    let restored = manager.open(&repo.0).unwrap();
    assert_eq!(restored.id, meta.id);
    assert_eq!(
        manager.document(&meta.id).unwrap().node(node).unwrap().name,
        "cached after save"
    );
    let (next, _) = manager.join(&meta.id, hello("after restart")).unwrap();
    assert!(next > peer);
    manager.save(&meta.id).unwrap();
    assert_eq!(repo.disk().node(node).unwrap().name, "cached after save");
    assert_eq!(
        std::fs::read_dir(repo.0.join(".mindmap")).unwrap().count(),
        1
    );
}

#[test]
fn concurrent_property_edits_are_ordered_and_broadcast() {
    let repo = Repo::new();
    let mut manager = NodeDocuments::new();
    let session = manager.open(&repo.0).unwrap().id;
    let node = create(&mut manager, &session, "before");
    let seq = manager.metadata(&session).unwrap().seq;
    let (a, _) = manager.join(&session, hello("Alice")).unwrap();
    let (b, _) = manager.join(&session, hello("Bob")).unwrap();
    for (peer, prop, value) in [
        (a, Prop::Name, json!("Alice's name")),
        (b, Prop::Width, json!(250.0)),
    ] {
        let outgoing = manager
            .receive(
                &session,
                peer,
                ClientMsg::Changes {
                    base_seq: seq,
                    batch_id: 1,
                    changes: vec![Change::Set {
                        id: node,
                        prop,
                        value: Json(value),
                    }],
                },
            )
            .unwrap();
        assert!(matches!(
            outgoing.first(),
            Some(Outgoing::All(ServerMsg::Changes(_)))
        ));
    }
    let doc = manager.document(&session).unwrap();
    assert_eq!(doc.node(node).unwrap().name, "Alice's name");
    assert_eq!(doc.node(node).unwrap().width, 250.0);
    assert_eq!(manager.metadata(&session).unwrap().seq, seq + 2);
    let outgoing = manager
        .receive(
            &session,
            b,
            ClientMsg::Changes {
                base_seq: seq,
                batch_id: 2,
                changes: vec![Change::Set {
                    id: node,
                    prop: Prop::Name,
                    value: Json(json!("Bob wins")),
                }],
            },
        )
        .unwrap();
    assert!(matches!(
        outgoing.first(),
        Some(Outgoing::All(ServerMsg::Changes(_)))
    ));
    assert_eq!(
        manager.document(&session).unwrap().node(node).unwrap().name,
        "Bob wins"
    );
}

#[test]
fn external_disk_change_merges_with_live_properties_and_broadcasts() {
    let repo = Repo::new();
    let mut manager = NodeDocuments::new();
    let session = manager.open(&repo.0).unwrap().id;
    let node = create(&mut manager, &session, "before");
    manager.save(&session).unwrap();
    let mut external = repo.disk();
    edit(&mut manager, &session, node, "live name");
    external.node_mut(node).unwrap().width = 321.0;
    repo.write(&external);
    let updates = manager.tick();
    assert!(updates[0].error.is_none(), "{:?}", updates[0].error);
    assert!(!updates[0].outgoing.is_empty());
    let doc = manager.document(&session).unwrap();
    assert_eq!(doc.node(node).unwrap().name, "live name");
    assert_eq!(doc.node(node).unwrap().width, 321.0);
    manager.save(&session).unwrap();
    assert_eq!(repo.disk().node(node).unwrap().name, "live name");
    assert_eq!(repo.disk().node(node).unwrap().width, 321.0);
}

#[test]
fn disk_conflict_stays_explicit_until_picked_and_cannot_be_overwritten() {
    let repo = Repo::new();
    let mut manager = NodeDocuments::new();
    let session = manager.open(&repo.0).unwrap().id;
    let node = create(&mut manager, &session, "base");
    manager.save(&session).unwrap();
    let mut external = repo.disk();
    external.node_mut(node).unwrap().name = "disk choice".into();
    edit(&mut manager, &session, node, "live choice");
    repo.write(&external);
    assert!(manager.save(&session).is_err());
    assert!(manager.tick()[0]
        .error
        .as_ref()
        .unwrap()
        .contains("design merge conflicts"));
    assert_eq!(repo.disk().node(node).unwrap().name, "disk choice");
    let conflicts = manager.resolve_disk(&session, &[]).unwrap().conflicts;
    assert_eq!(conflicts.len(), 1);
    let outcome = manager
        .resolve_disk(
            &session,
            &[MergePick {
                key: conflicts[0].key.clone(),
                side: Side::Main,
            }],
        )
        .unwrap();
    assert!(outcome.conflicts.is_empty());
    assert!(!outcome.outgoing.is_empty());
    assert_eq!(
        manager.document(&session).unwrap().node(node).unwrap().name,
        "disk choice"
    );
}

#[test]
fn checkout_branches_have_separate_sessions_and_survive_restart() {
    let repo = Repo::new();
    let mut manager = NodeDocuments::new();
    let main = manager.open(&repo.0).unwrap().id;
    let node = create(&mut manager, &main, "main base");
    manager.save(&main).unwrap();
    repo.commit();
    edit(&mut manager, &main, node, "main unsaved");
    repo.git(&["checkout", "-qb", "feature"]);
    let feature = manager.open(&repo.0).unwrap().id;
    assert_ne!(main, feature);
    assert_eq!(
        manager.document(&feature).unwrap().node(node).unwrap().name,
        "main base"
    );
    edit(&mut manager, &feature, node, "feature");
    manager.save(&feature).unwrap();
    repo.commit();
    assert!(manager.save(&main).is_err());
    assert_eq!(repo.disk().node(node).unwrap().name, "feature");
    drop(manager);
    repo.git(&["checkout", "-q", "main"]);
    let mut manager = NodeDocuments::new();
    assert_eq!(manager.open(&repo.0).unwrap().id, main);
    assert_eq!(
        manager.document(&main).unwrap().node(node).unwrap().name,
        "main unsaved"
    );
    manager.save(&main).unwrap();
    assert_eq!(repo.disk().node(node).unwrap().name, "main unsaved");
}

#[test]
fn imported_foreign_ids_preserve_peers_and_use_one_authority_batch() {
    let repo = Repo::new();
    let mut manager = NodeDocuments::new();
    let session = manager.open(&repo.0).unwrap().id;
    let (peer, _) = manager.join(&session, hello("client")).unwrap();
    let mut external = manager.document(&session).unwrap();
    external.set_client_id(500);
    let page = external.add_page("external page");
    let before = manager.metadata(&session).unwrap().seq;
    let outgoing = manager.replace(&session, external).unwrap();
    assert!(matches!(
        &outgoing[..],
        [Outgoing::All(ServerMsg::Changes(_))]
    ));
    assert!(manager.document(&session).unwrap().contains(page));
    assert_eq!(manager.metadata(&session).unwrap().seq, before + 1);
    let pong = manager.receive(&session, peer, ClientMsg::Ping).unwrap();
    assert!(matches!(&pong[..], [Outgoing::To(id, ServerMsg::Pong)] if *id == peer));
    let (next, _) = manager.join(&session, hello("new")).unwrap();
    assert!(next > 500);
}

#[test]
fn frozen_sync_gate_survives_restart_and_later_edits_remain_drift() {
    let repo = Repo::new();
    let mut manager = NodeDocuments::new();
    let session = manager.open(&repo.0).unwrap().id;
    let node = create(&mut manager, &session, "frozen design");
    manager.save(&session).unwrap();
    let frozen = encode(&manager.document(&session).unwrap()).unwrap();
    edit(&mut manager, &session, node, "later edit");
    manager.prepare_sync_bytes(&session, &frozen).unwrap();
    assert_eq!(repo.disk().node(node).unwrap().name, "frozen design");
    assert!(manager.save(&session).is_err());
    drop(manager);
    let mut manager = NodeDocuments::new();
    assert!(manager.open(&repo.0).unwrap().sync_prepared);
    assert_eq!(
        manager.document(&session).unwrap().node(node).unwrap().name,
        "later edit"
    );
    manager.tick();
    assert_eq!(repo.disk().node(node).unwrap().name, "frozen design");
    edit(&mut manager, &session, node, "latest edit");
    repo.commit();
    manager.finish_sync(&session).unwrap();
    assert_eq!(repo.disk().node(node).unwrap().name, "latest edit");
    assert!(!manager.metadata(&session).unwrap().sync_prepared);
    let committed = Command::new("git")
        .arg("-C")
        .arg(&repo.0)
        .args(["show", "HEAD:.mindmap/map.figcraft"])
        .output()
        .unwrap();
    assert_eq!(
        decode(&committed.stdout).unwrap().node(node).unwrap().name,
        "frozen design"
    );
}

#[test]
fn engine_batch_is_atomic_and_nested_file_commands_cannot_write() {
    let repo = Repo::new();
    let mut manager = NodeDocuments::new();
    let session = manager.open(&repo.0).unwrap().id;
    let seq = manager.metadata(&session).unwrap().seq;
    let commands = json!({"commands": [
        {"command":"node.create","params":{"type":"rectangle"}},
        {"command":"node.set","params":{"ids":[99999999],"props":{"name":"bad"}}}
    ]});
    assert!(manager
        .execute(&session, "command.batch", &commands)
        .is_err());
    assert_eq!(manager.metadata(&session).unwrap().seq, seq);
    let target = repo.0.join("unexpected.figcraft");
    let commands = json!({"commands":[{"command":"command.batch","params":{"commands":[{"command":"file.saveAs","params":{"path":target}}]}}]});
    assert!(manager
        .execute(&session, "command.batch", &commands)
        .is_err());
    assert!(!target.exists());
}

#[test]
fn canonical_paths_reuse_authority_and_missing_disk_is_not_silently_recreated() {
    let repo = Repo::new();
    let mut manager = NodeDocuments::new();
    let session = manager.open(&repo.0).unwrap().id;
    manager.save(&session).unwrap();
    assert_eq!(manager.open(repo.0.join(".mindmap")).unwrap().id, session);
    std::fs::remove_file(repo.0.join(DESIGN_PATH)).unwrap();
    assert!(manager.save(&session).is_err());
    assert!(!repo.0.join(DESIGN_PATH).exists());
}

#[test]
fn debounce_saves_only_after_quiet_and_presence_does_not_touch_cache() {
    let repo = Repo::new();
    let mut manager = NodeDocuments::new();
    let session = manager.open(&repo.0).unwrap().id;
    let node = create(&mut manager, &session, "debounced");
    manager.tick();
    assert!(!repo.0.join(DESIGN_PATH).exists());
    manager.sessions.get_mut(&session).unwrap().edited = Instant::now() - SAVE_DELAY;
    let updates = manager.tick();
    assert!(updates[0].error.is_none());
    assert_eq!(repo.disk().node(node).unwrap().name, "debounced");
    let (a, _) = manager.join(&session, hello("Alice")).unwrap();
    let cache = manager.get(&session).unwrap().cache.clone();
    let modified = std::fs::metadata(&cache).unwrap().modified().unwrap();
    let presence = Presence {
        cursor: Some((manager.document(&session).unwrap().pages[0], 10.0, 20.0)),
        selection: Some(vec![node]),
        viewport: None,
        editing_text: None,
    };
    let outgoing = manager
        .receive(&session, a, ClientMsg::Presence(presence.clone()))
        .unwrap();
    assert!(
        matches!(&outgoing[..], [Outgoing::Others(id, ServerMsg::Presence { .. })] if *id == a)
    );
    assert_eq!(
        std::fs::metadata(&cache).unwrap().modified().unwrap(),
        modified
    );
    let (b, outgoing) = manager.join(&session, hello("Bob")).unwrap();
    assert!(outgoing.iter().any(|o| matches!(o, Outgoing::To(to, ServerMsg::Presence { client_id, presence: p }) if *to == b && *client_id == a && *p == presence)));
}

#[test]
fn linked_worktrees_are_independent_authorities_with_local_caches() {
    let repo = Repo::new();
    let mut manager = NodeDocuments::new();
    let main = manager.open(&repo.0).unwrap().id;
    let node = create(&mut manager, &main, "base");
    manager.save(&main).unwrap();
    repo.commit();
    let linked = repo.0.join("linked-worktree");
    repo.git(&["worktree", "add", "-qb", "linked", linked.to_str().unwrap()]);
    let other = manager.open(&linked).unwrap().id;
    assert_ne!(main, other);
    edit(&mut manager, &main, node, "main");
    edit(&mut manager, &other, node, "linked");
    manager.save(&main).unwrap();
    manager.save(&other).unwrap();
    assert_eq!(repo.disk().node(node).unwrap().name, "main");
    assert_eq!(
        decode(&std::fs::read(linked.join(DESIGN_PATH)).unwrap())
            .unwrap()
            .node(node)
            .unwrap()
            .name,
        "linked"
    );
    assert_ne!(
        manager.get(&main).unwrap().cache.parent(),
        manager.get(&other).unwrap().cache.parent()
    );
}

#[test]
fn prepared_sync_writes_exact_bytes_and_external_conflicts_can_be_picked() {
    let repo = Repo::new();
    let mut manager = NodeDocuments::new();
    let session = manager.open(&repo.0).unwrap().id;
    let node = create(&mut manager, &session, "frozen");
    manager.save(&session).unwrap();
    let frozen = repo.disk();
    let bytes = encode(&frozen).unwrap();
    manager.prepare_sync_bytes(&session, &bytes).unwrap();
    assert_eq!(std::fs::read(repo.0.join(DESIGN_PATH)).unwrap(), bytes);
    edit(&mut manager, &session, node, "live");
    let mut outside = frozen;
    outside.node_mut(node).unwrap().name = "outside".into();
    repo.write(&outside);
    let error = manager.finish_sync(&session).unwrap_err();
    assert!(error.to_string().contains("design merge conflicts"));
    assert!(manager.metadata(&session).unwrap().sync_prepared);
    manager
        .finish_sync_with_picks(
            &session,
            &[MergePick {
                key: format!("{}/name", node),
                side: Side::Branch,
            }],
        )
        .unwrap();
    assert_eq!(repo.disk().node(node).unwrap().name, "live");
    assert!(!manager.metadata(&session).unwrap().sync_prepared);
}

#[test]
fn git_merge_gate_ignores_transient_conflict_file_and_abort_preserves_live() {
    let repo = Repo::new();
    let mut manager = NodeDocuments::new();
    let session = manager.open(&repo.0).unwrap().id;
    let node = create(&mut manager, &session, "HEAD");
    manager.save(&session).unwrap();
    let base = repo.disk();
    edit(&mut manager, &session, node, "live before merge");
    manager
        .prepare_sync_bytes(&session, &encode(&base).unwrap())
        .unwrap();
    std::fs::write(repo.0.join(DESIGN_PATH), b"transient Git binary conflict").unwrap();
    let mut incoming = base.clone();
    incoming.node_mut(node).unwrap().width = 475.0;
    let outcome = manager
        .prepare_merge(&session, &base, &incoming, &[])
        .unwrap();
    assert!(outcome.conflicts.is_empty());
    assert!(!outcome.outgoing.is_empty());
    assert_eq!(repo.disk().node(node).unwrap().name, "live before merge");
    assert_eq!(repo.disk().node(node).unwrap().width, 475.0);
    assert!(manager.metadata(&session).unwrap().sync_prepared);
    edit(&mut manager, &session, node, "later live edit");
    // Controlled git merge --abort restores HEAD; it is deliberately not treated as a new edit.
    repo.write(&base);
    manager.abort_prepared(&session).unwrap();
    assert_eq!(repo.disk().node(node).unwrap().name, "later live edit");
    assert_eq!(repo.disk().node(node).unwrap().width, 475.0);
    assert!(!manager.metadata(&session).unwrap().sync_prepared);
}

#[test]
fn git_merge_conflicts_leave_gate_and_authority_untouched_until_picked() {
    let repo = Repo::new();
    let mut manager = NodeDocuments::new();
    let session = manager.open(&repo.0).unwrap().id;
    let node = create(&mut manager, &session, "base");
    manager.save(&session).unwrap();
    let base = repo.disk();
    edit(&mut manager, &session, node, "live");
    manager
        .prepare_sync_bytes(&session, &encode(&base).unwrap())
        .unwrap();
    let mut incoming = base.clone();
    incoming.node_mut(node).unwrap().name = "incoming".into();
    let outcome = manager
        .prepare_merge(&session, &base, &incoming, &[])
        .unwrap();
    assert_eq!(outcome.conflicts.len(), 1);
    assert_eq!(
        manager.document(&session).unwrap().node(node).unwrap().name,
        "live"
    );
    assert_eq!(repo.disk().node(node).unwrap().name, "base");
    let outcome = manager
        .prepare_merge(
            &session,
            &base,
            &incoming,
            &[MergePick {
                key: outcome.conflicts[0].key.clone(),
                side: Side::Main,
            }],
        )
        .unwrap();
    assert!(outcome.conflicts.is_empty());
    assert_eq!(repo.disk().node(node).unwrap().name, "incoming");
    manager.finish_sync(&session).unwrap();
}

#[test]
fn merge_readiness_and_exact_prepared_bytes_survive_restart_and_clear_on_release() {
    let repo = Repo::new();
    let mut manager = NodeDocuments::new();
    let session = manager.open(&repo.0).unwrap().id;
    let node = create(&mut manager, &session, "base");
    manager.save(&session).unwrap();
    let base = repo.disk();
    manager
        .prepare_sync_bytes(&session, &encode(&base).unwrap())
        .unwrap();
    assert!(!manager.metadata(&session).unwrap().merge_ready);
    let mut incoming = base.clone();
    incoming.node_mut(node).unwrap().width = 600.0;
    manager
        .prepare_merge(&session, &base, &incoming, &[])
        .unwrap();
    assert!(manager.metadata(&session).unwrap().merge_ready);
    let frozen = manager.prepared_bytes(&session).unwrap();
    assert_eq!(frozen, std::fs::read(repo.0.join(DESIGN_PATH)).unwrap());
    drop(manager);
    let mut manager = NodeDocuments::new();
    assert!(manager.open(&repo.0).unwrap().merge_ready);
    edit(&mut manager, &session, node, "later live");
    std::fs::write(repo.0.join(DESIGN_PATH), b"outside writer").unwrap();
    assert_eq!(manager.prepared_bytes(&session).unwrap(), frozen);
    manager.abort_prepared(&session).unwrap();
    assert!(!manager.metadata(&session).unwrap().merge_ready);
    assert!(manager.prepared_bytes(&session).is_err());
    manager
        .prepare_sync_bytes(
            &session,
            &encode(&manager.document(&session).unwrap()).unwrap(),
        )
        .unwrap();
    assert!(!manager.metadata(&session).unwrap().merge_ready);
    manager.finish_sync(&session).unwrap();
    assert!(!manager.metadata(&session).unwrap().merge_ready);
    assert!(manager.prepared_bytes(&session).is_err());

    // Cache files from before merge readiness was introduced remain readable and unready.
    let cache = manager.get(&session).unwrap().cache.clone();
    let mut saved: Value = serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
    saved.as_object_mut().unwrap().remove("merge_ready");
    atomic_write(&cache, &serde_json::to_vec(&saved).unwrap()).unwrap();
    drop(manager);
    let mut manager = NodeDocuments::new();
    assert!(!manager.open(&repo.0).unwrap().merge_ready);
}
