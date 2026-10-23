use super::*;
use figcraft_doc::NodeKind;

struct Repo(PathBuf);

impl Repo {
    fn new() -> Self {
        let id = NEXT_FREEZE.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("mindmap-node-git-test-{}-{id}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        git(&path, &["init", "-b", "main"]).unwrap();
        git(&path, &["config", "user.email", "test@example.invalid"]).unwrap();
        git(&path, &["config", "user.name", "Mindmap test"]).unwrap();
        git(&path, &["config", "commit.gpgsign", "false"]).unwrap();
        git(&path, &["config", "core.hooksPath", "/dev/null"]).unwrap();
        Self(path)
    }

    fn write_design(&self, doc: &Document) {
        fs::create_dir_all(self.0.join(".mindmap")).unwrap();
        fs::write(
            self.0.join(DESIGN_PATH),
            figcraft_format::save_bytes(doc, None).unwrap(),
        )
        .unwrap();
    }

    fn commit(&self, doc: &Document, message: &str) -> String {
        self.write_design(doc);
        git(&self.0, &["add", "--", DESIGN_PATH]).unwrap();
        git(&self.0, &["commit", "--allow-empty", "-m", message]).unwrap();
        head(&self.0).unwrap().unwrap()
    }

    fn marker(&self, doc: &Document, scope: &str) -> String {
        self.commit(doc, &format!("Sync design\n\nMindmap-Sync: {scope}"))
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn design() -> (Document, NodeId, NodeId, NodeId) {
    let mut doc = Document::new("Design");
    let p = doc.first_page().unwrap();
    let node = doc
        .create(p, NodeKind::Rectangle, "Card", 1.0, 2.0, 20.0, 30.0)
        .unwrap();
    let other = doc.add_page("Unrelated");
    (doc, p, node, other)
}

#[test]
fn baselines_are_reachable_scoped_and_actual_trailers() {
    let repo = Repo::new();
    let (doc, page, _, other) = design();
    assert!(baseline(&repo.0, Some(page.0)).unwrap().is_none());
    let all = repo.marker(&doc, "all");
    let selected = repo.marker(&doc, &format!("page={}", page.0));
    repo.commit(
        &doc,
        "Body only\n\nMindmap-Sync: all\nThis is prose, not a trailer block.\n\nA final paragraph.",
    );
    repo.marker(&doc, "implemented some things");
    assert_eq!(
        baseline(&repo.0, Some(page.0)).unwrap().unwrap().commit,
        selected
    );
    assert_eq!(
        baseline(&repo.0, Some(other.0)).unwrap().unwrap().commit,
        all
    );
    assert_eq!(baseline(&repo.0, None).unwrap().unwrap().commit, all);
    git(&repo.0, &["checkout", "-b", "side"]).unwrap();
    let side = repo.marker(&doc, "all");
    git(&repo.0, &["checkout", "main"]).unwrap();
    assert_ne!(baseline(&repo.0, None).unwrap().unwrap().commit, side);
    git(&repo.0, &["merge", "--no-ff", "side", "-m", "Merge side"]).unwrap();
    assert_eq!(baseline(&repo.0, None).unwrap().unwrap().commit, side);
}

#[test]
fn page_only_sync_never_baselines_unsynced_other_page_or_all() {
    let repo = Repo::new();
    let (doc, page, _, other) = design();
    repo.marker(&doc, &page.0.to_string());
    assert!(baseline(&repo.0, Some(page.0)).unwrap().is_some());
    assert!(baseline(&repo.0, Some(other.0)).unwrap().is_none());
    assert!(baseline(&repo.0, None).unwrap().is_none());
    let delta = diff(&repo.0, &doc, Some(other.0)).unwrap();
    assert_eq!(delta["last"]["exists"], false);
    assert!(!delta["map"]["added"].as_array().unwrap().is_empty());
}

#[test]
fn property_diff_scope_pagination_and_code_paths_are_complete() {
    let repo = Repo::new();
    let (mut doc, page, card, other) = design();
    let outside = doc
        .create(other, NodeKind::Rectangle, "Other", 0.0, 0.0, 5.0, 5.0)
        .unwrap();
    let subpage = doc.add_page("Child page");
    doc.get_mut(subpage)
        .unwrap()
        .plugin_data
        .insert("mindmap.parent_page".into(), page.0.to_string());
    let child = doc
        .create(subpage, NodeKind::Rectangle, "Child", 0.0, 0.0, 5.0, 5.0)
        .unwrap();
    fs::write(repo.0.join("code\twith\nnewline.rs"), "old\n").unwrap();
    git(&repo.0, &["add", "--", "code\twith\nnewline.rs"]).unwrap();
    repo.marker(&doc, "all");
    doc.get_mut(card).unwrap().width = 99.0;
    doc.get_mut(card).unwrap().opacity = 0.5;
    doc.get_mut(card)
        .unwrap()
        .plugin_data
        .insert("arrow.target".into(), outside.0.to_string());
    doc.get_mut(outside).unwrap().height = 80.0;
    doc.get_mut(child).unwrap().height = 40.0;
    fs::write(repo.0.join("code\twith\nnewline.rs"), "old\nnew\n").unwrap();
    fs::write(repo.0.join("new-code.rs"), "new\n").unwrap();
    let delta = diff(&repo.0, &doc, Some(page.0)).unwrap();
    let changed = delta["map"]["changed"].as_array().unwrap();
    assert_eq!(changed.len(), 2);
    assert!(changed.iter().all(|v| v["id"] != outside.0));
    let card_delta = changed.iter().find(|v| v["id"] == card.0).unwrap();
    assert_eq!(
        card_delta["fields"],
        json!(["opacity", "pluginData", "width"])
    );
    let files = delta["code"]["files"].as_array().unwrap();
    assert!(files
        .iter()
        .any(|f| f["path"] == "code\twith\nnewline.rs" && f["added"] == 1));
    assert!(files
        .iter()
        .any(|f| f["path"] == "new-code.rs" && f["untracked"] == true));
    assert!(files.iter().all(|f| f["path"] != DESIGN_PATH));
    let full = delta["changes"].as_array().unwrap();
    let mut paged = Vec::new();
    for offset in 0..full.len() {
        let response = diff_paginated(&repo.0, &doc, Some(page.0), offset, 1).unwrap();
        assert_eq!(response["pagination"]["changes"]["total"], full.len());
        paged.extend(response["changes"].as_array().unwrap().clone());
    }
    assert_eq!(&paged, full);
    assert!(
        diff_paginated(&repo.0, &doc, Some(page.0), usize::MAX, 1).unwrap()["changes"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(diff_paginated(&repo.0, &doc, Some(page.0), 0, 0).is_err());
}

#[test]
fn frozen_native_bytes_survive_save_load_and_do_not_touch_live_file() {
    let repo = Repo::new();
    let (mut doc, page, card, _) = design();
    repo.marker(&doc, "all");
    doc.get_mut(card).unwrap().width = 40.0;
    let frozen = freeze(&repo.0, &doc, Some(page.0)).unwrap();
    let bytes = fs::read(&frozen.snapshot).unwrap();
    let loaded = frozen_document(&frozen).unwrap();
    assert_eq!(
        bytes,
        figcraft_format::save_bytes(&loaded, None).unwrap(),
        "document gate may reserialize without changing the frozen blob"
    );
    assert!(!frozen.snapshot.starts_with(repo.0.join(".mindmap")));
    assert!(fs::metadata(&frozen.snapshot)
        .unwrap()
        .permissions()
        .readonly());
    let encoded = serde_json::to_vec(&frozen).unwrap();
    let restored: FrozenSync = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(frozen_document(&restored).unwrap(), loaded);
    doc.get_mut(card).unwrap().width = 70.0;
    repo.write_design(&doc);
    let live_bytes = fs::read(repo.0.join(DESIGN_PATH)).unwrap();
    prepare_commit(&repo.0, &frozen).unwrap();
    assert_eq!(fs::read(repo.0.join(DESIGN_PATH)).unwrap(), live_bytes);
    assert_eq!(
        git_text(&repo.0, &["rev-parse", &format!(":{DESIGN_PATH}")]).unwrap(),
        frozen.blob
    );
    assert!(
        git(&repo.0, &["ls-files", "--others", "--exclude-standard"])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn exact_job_commit_required_and_concurrent_edits_remain_drift() {
    let repo = Repo::new();
    let (mut doc, page, card, other) = design();
    let initial = repo.marker(&doc, "all");
    doc.get_mut(card).unwrap().width = 40.0;
    let frozen = freeze(&repo.0, &doc, Some(page.0)).unwrap();
    assert!(
        find_completed(&repo.0, &frozen).unwrap().is_none(),
        "idle without a commit never completes"
    );
    assert!(verify_completed(&repo.0, &frozen, &initial).is_err());
    // Simulate document gate, then agent's normal git add -A workflow.
    repo.write_design(&frozen_document(&frozen).unwrap());
    prepare_commit(&repo.0, &frozen).unwrap();
    fs::write(repo.0.join("implementation.rs"), "implemented\n").unwrap();
    git(&repo.0, &["add", "-A"]).unwrap();
    git(
        &repo.0,
        &[
            "commit",
            "-m",
            &format!("Implement page\n\n{}", frozen.trailer),
        ],
    )
    .unwrap();
    let completed = find_completed(&repo.0, &frozen).unwrap().unwrap();
    assert_ne!(completed.commit, initial);
    assert_eq!(
        baseline(&repo.0, Some(page.0)).unwrap().unwrap().commit,
        completed.commit
    );
    assert_eq!(
        baseline(&repo.0, Some(other.0)).unwrap().unwrap().commit,
        initial
    );
    doc.get_mut(card).unwrap().width = 80.0;
    repo.write_design(&doc); // document gate restored current live
    let delta = diff(&repo.0, &doc, Some(page.0)).unwrap();
    assert_eq!(delta["map"]["changed"][0]["fields"], json!(["width"]));
    assert_eq!(delta["code"]["files"], json!([]));
}

#[test]
fn wrong_blob_wrong_job_extra_scope_and_branch_switch_are_rejected() {
    let repo = Repo::new();
    let (mut doc, page, card, _) = design();
    repo.marker(&doc, "all");
    let frozen = freeze(&repo.0, &doc, Some(page.0)).unwrap();
    doc.get_mut(card).unwrap().width += 1.0;
    let wrong = repo.commit(&doc, &format!("Wrong design\n\n{}", frozen.trailer));
    assert!(verify_completed(&repo.0, &frozen, &wrong).is_err());
    assert!(find_completed(&repo.0, &frozen).is_err());
    let original = frozen_document(&frozen).unwrap();
    let wrong = repo.marker(&original, &format!("page={} job=another", page.0));
    assert!(verify_completed(&repo.0, &frozen, &wrong).is_err());
    let wrong = repo.commit(
        &original,
        &format!("Too broad\n\n{}\nMindmap-Sync: all", frozen.trailer),
    );
    assert!(verify_completed(&repo.0, &frozen, &wrong).is_err());
    let correct = repo.commit(&original, &format!("Correct\n\n{}", frozen.trailer));
    assert!(verify_completed(&repo.0, &frozen, &correct).is_ok());
    git(&repo.0, &["checkout", "-b", "other"]).unwrap();
    assert!(verify_completed(&repo.0, &frozen, &correct).is_err());
}

#[test]
fn freeze_and_complete_unborn_repository() {
    let repo = Repo::new();
    let (doc, page, _, _) = design();
    let frozen = freeze(&repo.0, &doc, Some(page.0)).unwrap();
    assert!(frozen.start_head.is_none());
    assert!(!diff(&repo.0, &doc, Some(page.0)).unwrap()["map"]["added"]
        .as_array()
        .unwrap()
        .is_empty());
    repo.write_design(&doc);
    prepare_commit(&repo.0, &frozen).unwrap();
    git(
        &repo.0,
        &["commit", "-m", &format!("First sync\n\n{}", frozen.trailer)],
    )
    .unwrap();
    assert!(find_completed(&repo.0, &frozen).unwrap().is_some());
}

#[test]
fn immutable_merge_plan_uses_git_base_and_explicit_property_picks() {
    let repo = Repo::new();
    let (doc, _, card, _) = design();
    let base = repo.commit(&doc, "Base");
    git(&repo.0, &["checkout", "-b", "feature"]).unwrap();
    let mut branch = doc.clone();
    branch.get_mut(card).unwrap().width = 40.0;
    branch.get_mut(card).unwrap().opacity = 0.5;
    let branch_commit = repo.commit(&branch, "Branch");
    git(&repo.0, &["checkout", "main"]).unwrap();
    let mut main = doc.clone();
    main.get_mut(card).unwrap().width = 60.0;
    main.get_mut(card).unwrap().height = 90.0;
    let main_commit = repo.commit(&main, "Main");
    let plan = plan_merge(&repo.0, "feature", "main").unwrap();
    assert_eq!(plan.base, base);
    assert_eq!(plan.branch, branch_commit);
    assert_eq!(plan.main, main_commit);
    assert_eq!(plan.json()["conflicts"].as_array().unwrap().len(), 1);
    assert!(resolve_merge(&plan, &BTreeMap::new()).is_err());
    let pick = BTreeMap::from([(format!("{card}/width"), merge::Side::Branch)]);
    let merged = resolve_merge(&plan, &pick).unwrap();
    let node = merged.node(card).unwrap();
    assert_eq!(node.width, 40.0);
    assert_eq!(node.height, 90.0);
    assert_eq!(node.opacity, 0.5);
    assert_eq!(
        head(&repo.0).unwrap().unwrap(),
        main_commit,
        "planning must not mutate Git"
    );
    let unknown = BTreeMap::from([("typo".into(), merge::Side::Main)]);
    assert!(resolve_merge(&plan, &unknown).is_err());
    assert!(plan_merge(&repo.0, "--all", "main").is_err());
}

#[test]
fn git_merge_code_conflicts_are_explicit_and_commit_has_both_parents() {
    let repo = Repo::new();
    let (doc, _, card, _) = design();
    fs::write(repo.0.join("code.rs"), "base\n").unwrap();
    git(&repo.0, &["add", "code.rs"]).unwrap();
    repo.commit(&doc, "Base");
    git(&repo.0, &["checkout", "-b", "feature"]).unwrap();
    let mut branch = doc.clone();
    branch.get_mut(card).unwrap().width = 50.0;
    fs::write(repo.0.join("code.rs"), "branch\n").unwrap();
    git(&repo.0, &["add", "code.rs"]).unwrap();
    let branch_commit = repo.commit(&branch, "Branch");
    git(&repo.0, &["checkout", "main"]).unwrap();
    let mut main = doc.clone();
    main.get_mut(card).unwrap().height = 80.0;
    fs::write(repo.0.join("code.rs"), "main\n").unwrap();
    git(&repo.0, &["add", "code.rs"]).unwrap();
    let main_commit = repo.commit(&main, "Main");
    let plan = plan_merge(&repo.0, "feature", "main").unwrap();
    let merged = resolve_merge(&plan, &BTreeMap::new()).unwrap();
    let result = begin_merge(&repo.0, "feature").unwrap();
    assert_eq!(result["state"], "conflicts");
    assert_eq!(result["committed"], false);
    assert_eq!(result["code_conflicts"], json!(["code.rs"]));
    assert!(commit_merge(&repo.0, "Merge feature").is_err());
    repo.write_design(&merged);
    stage_design(&repo.0).unwrap();
    assert!(commit_merge(&repo.0, "Merge feature").is_err());
    fs::write(repo.0.join("code.rs"), "resolved\n").unwrap();
    git(&repo.0, &["add", "code.rs"]).unwrap();
    fs::write(repo.0.join("unrelated-untracked.txt"), "keep out\n").unwrap();
    let commit = commit_merge(&repo.0, "Merge feature").unwrap();
    let parents = git_text(&repo.0, &["show", "-s", "--format=%P", &commit]).unwrap();
    assert_eq!(parents, format!("{main_commit} {branch_commit}"));
    assert_eq!(
        load_design(&repo.0, &commit)
            .unwrap()
            .node(card)
            .unwrap()
            .width,
        50.0
    );
    assert!(git_text(
        &repo.0,
        &["ls-tree", "HEAD", "--", "unrelated-untracked.txt"]
    )
    .unwrap()
    .is_empty());
    assert!(commit_merge(&repo.0, "Not a merge").is_err());
}

#[test]
fn linked_worktree_owns_its_frozen_cache_and_rejects_cross_worktree_job() {
    let repo = Repo::new();
    let (doc, page, _, _) = design();
    repo.commit(&doc, "Base");
    let linked = repo.0.join("linked");
    git(
        &repo.0,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    )
    .unwrap();
    let frozen = freeze(&linked, &doc, Some(page.0)).unwrap();
    assert!(frozen
        .snapshot
        .starts_with(repo.0.join(".git/worktrees/linked/mindmap-sync")));
    assert!(prepare_commit(&repo.0, &frozen).is_err());
    assert!(prepare_commit(&linked, &frozen).is_ok());
}

#[test]
fn real_merge_gate_preserves_live_edits_before_and_during_commit_or_abort() {
    use crate::node_documents::NodeDocuments;

    for abort in [false, true] {
        let repo = Repo::new();
        let (doc, _, card, _) = design();
        repo.commit(&doc, "Base");
        git(&repo.0, &["checkout", "-b", "feature"]).unwrap();
        let mut incoming = doc.clone();
        incoming.get_mut(card).unwrap().width = 50.0;
        fs::write(repo.0.join("feature.rs"), "feature implementation\n").unwrap();
        git(&repo.0, &["add", "feature.rs"]).unwrap();
        repo.commit(&incoming, "Feature");
        git(&repo.0, &["checkout", "main"]).unwrap();
        let mut main = doc.clone();
        main.get_mut(card).unwrap().height = 80.0;
        repo.commit(&main, "Main");

        let plan = plan_merge(&repo.0, "HEAD", "feature").unwrap();
        let resolved = resolve_merge(&plan, &BTreeMap::new()).unwrap();
        let head_bytes = load_design_bytes(&repo.0, "HEAD").unwrap();
        let mut documents = NodeDocuments::new();
        let session = documents.open(&repo.0).unwrap().id;
        let mut live = documents.document(&session).unwrap();
        live.get_mut(card).unwrap().opacity = 0.4;
        documents.replace(&session, live).unwrap();
        documents.prepare_sync_bytes(&session, &head_bytes).unwrap();
        assert!(pending_merge(&repo.0).unwrap().is_empty());
        let started = begin_merge(&repo.0, "feature").unwrap();
        assert_eq!(pending_merge(&repo.0).unwrap(), vec![plan.main.clone()]);
        assert_ne!(started["state"], "up_to_date");
        let outcome = documents
            .prepare_merge(&session, &main, &resolved, &[])
            .unwrap();
        assert!(outcome.conflicts.is_empty());
        stage_design(&repo.0).unwrap();

        // A later collaborative edit must stay live while the commit uses the prepared version.
        let mut live = documents.document(&session).unwrap();
        live.get_mut(card).unwrap().name = "After merge preparation".into();
        documents.replace(&session, live).unwrap();
        assert!(documents.metadata(&session).unwrap().sync_prepared);
        if abort {
            // Metadata refresh must not stage or overwrite a real external edit.
            let prepared = fs::read(repo.0.join(DESIGN_PATH)).unwrap();
            let index = git(&repo.0, &["ls-files", "--stage", "-z"]).unwrap();
            fs::write(repo.0.join(DESIGN_PATH), b"external writer").unwrap();
            assert!(abort_merge(&repo.0).is_err());
            assert_eq!(
                fs::read(repo.0.join(DESIGN_PATH)).unwrap(),
                b"external writer"
            );
            assert_eq!(git(&repo.0, &["ls-files", "--stage", "-z"]).unwrap(), index);
            fs::write(repo.0.join(DESIGN_PATH), prepared).unwrap();
            abort_merge(&repo.0).unwrap();
            assert!(pending_merge(&repo.0).unwrap().is_empty());
            assert!(abort_merge(&repo.0).is_err());
            documents.abort_prepared(&session).unwrap();
            assert!(!repo.0.join("feature.rs").exists());
        } else {
            let commit = commit_merge(&repo.0, "Merge feature").unwrap();
            let committed = load_design(&repo.0, &commit).unwrap();
            assert_eq!(committed.node(card).unwrap().name, "Card");
            assert_eq!(committed.node(card).unwrap().opacity, 0.4);
            documents.finish_sync(&session).unwrap();
            assert!(repo.0.join("feature.rs").exists());
        }
        assert!(!documents.metadata(&session).unwrap().sync_prepared);
        let disk = decode_design(&fs::read(repo.0.join(DESIGN_PATH)).unwrap()).unwrap();
        let node = disk.node(card).unwrap();
        assert_eq!(node.name, "After merge preparation");
        assert_eq!(
            node.opacity, 0.4,
            "pre-existing live edits survive both paths"
        );
        assert_eq!(node.width, 50.0);
        assert_eq!(node.height, 80.0);
    }
}
