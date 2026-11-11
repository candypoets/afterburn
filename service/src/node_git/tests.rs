use super::*;
use figcraft_doc::NodeKind;

struct Repo(PathBuf);

impl Repo {
    fn new() -> Self {
        let id = NEXT_FREEZE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "afterburn-node-git-test-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        git(&path, &["init", "-b", "main"]).unwrap();
        git(&path, &["config", "user.email", "test@example.invalid"]).unwrap();
        git(&path, &["config", "user.name", "Afterburn test"]).unwrap();
        git(&path, &["config", "commit.gpgsign", "false"]).unwrap();
        git(&path, &["config", "core.hooksPath", "/dev/null"]).unwrap();
        Self(path)
    }

    fn write_design(&self, doc: &Document) {
        fs::create_dir_all(self.0.join(".afterburn")).unwrap();
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
        self.commit(doc, &format!("Sync design\n\nAfterburn-Sync: {scope}"))
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
        "Body only\n\nAfterburn-Sync: all\nThis is prose, not a trailer block.\n\nA final paragraph.",
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
        .insert("afterburn.parent_page".into(), page.0.to_string());
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
    assert!(!frozen.snapshot.starts_with(repo.0.join(".afterburn")));
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
        &format!("Too broad\n\n{}\nAfterburn-Sync: all", frozen.trailer),
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
fn git_plan_merges_grid_items_but_requires_whole_code_links_pick() {
    let repo = Repo::new();
    let (mut doc, _, card, _) = design();
    doc.get_mut(card).unwrap().grids = [10.0, 20.0]
        .into_iter()
        .map(|size| figcraft_doc::LayoutGrid {
            size,
            ..Default::default()
        })
        .collect();
    doc.get_mut(card).unwrap().code_links = ["react", "vue"]
        .into_iter()
        .map(|language| figcraft_doc::CodeLink {
            language: language.into(),
            ..Default::default()
        })
        .collect();
    let base = repo.commit(&doc, "Base lists");
    git(&repo.0, &["checkout", "-b", "feature"]).unwrap();
    let mut branch = doc.clone();
    branch.get_mut(card).unwrap().grids[0].size = 11.0;
    branch.get_mut(card).unwrap().code_links[0].name = "Branch component".into();
    repo.commit(&branch, "Branch list edits");
    git(&repo.0, &["checkout", "main"]).unwrap();
    let mut main = doc;
    main.get_mut(card).unwrap().grids[1].size = 22.0;
    main.get_mut(card).unwrap().code_links[1].name = "Main component".into();
    let main_commit = repo.commit(&main, "Main list edits");
    let disk_bytes = fs::read(repo.0.join(DESIGN_PATH)).unwrap();

    let plan = plan_merge(&repo.0, "feature", "main").unwrap();
    assert_eq!(plan.base, base);
    let response = plan.json();
    let conflicts = response["conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1, "{response}");
    let key = conflicts[0]["key"].as_str().unwrap();
    assert_eq!(key, format!("{card}/codeLinks"));
    assert!(resolve_merge(&plan, &BTreeMap::new()).is_err());
    // Picks come from the node's JSON plan, and select a whole code-links property.
    for (side, expected) in [(merge::Side::Branch, &branch), (merge::Side::Main, &main)] {
        let picks = BTreeMap::from([(key.to_owned(), side)]);
        let merged = resolve_merge(&plan, &picks).unwrap();
        let node = merged.node(card).unwrap();
        assert_eq!(
            node.grids.iter().map(|g| g.size).collect::<Vec<_>>(),
            [11.0, 22.0]
        );
        assert_eq!(node.code_links, expected.node(card).unwrap().code_links);
    }
    assert_eq!(fs::read(repo.0.join(DESIGN_PATH)).unwrap(), disk_bytes);
    assert_eq!(head(&repo.0).unwrap().unwrap(), main_commit);
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
        .starts_with(repo.0.join(".git/worktrees/linked/afterburn-sync")));
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

fn names(doc: &Document) -> BTreeSet<String> {
    doc.nodes().map(|n| n.name.clone()).collect()
}

#[test]
fn explanations_and_fixes_move_the_design_baseline_but_only_syncs_move_the_code_one() {
    let repo = Repo::new();
    let (doc, page, _, _) = design();
    let scope = format!("page={}", page.0);
    let sync = repo.marker(&doc, &format!("{scope} job=a"));
    let explain = repo.marker(&doc, &format!("{scope} job=b explain"));
    let design = baseline(&repo.0, Some(page.0)).unwrap().unwrap();
    assert_eq!(
        (design.commit.as_str(), design.kind.as_str()),
        (explain.as_str(), "explain")
    );
    let code = sync_baseline(&repo.0, Some(page.0)).unwrap().unwrap();
    assert_eq!(
        (code.commit.as_str(), code.kind.as_str()),
        (sync.as_str(), "sync")
    );
    assert_eq!(marker_kind("all job=c fix"), "fix");
    assert_eq!(marker_kind("page=3 job=c"), "sync");
    let frozen = freeze_as(&repo.0, &doc, Some(page.0), "explain").unwrap();
    assert!(frozen.trailer.ends_with(" explain"), "{}", frozen.trailer);
    assert_eq!(
        trailer_scope(frozen.trailer.strip_prefix("Afterburn-Sync: ").unwrap()),
        Some(Some(page.0))
    );
    assert!(freeze_as(&repo.0, &doc, Some(page.0), "proposal").is_err());
}

#[test]
fn a_record_takes_this_requests_edits_and_leaves_earlier_waiting_edits_out() {
    let repo = Repo::new();
    let (mut doc, page, _, other) = design();
    repo.marker(&doc, &format!("page={} job=a", page.0));
    // Waiting: a design edit on the page, and another on an unrelated page.
    doc.create(page, NodeKind::Rectangle, "Retry", 0.0, 0.0, 5.0, 5.0)
        .unwrap();
    doc.create(other, NodeKind::Rectangle, "Elsewhere", 0.0, 0.0, 5.0, 5.0)
        .unwrap();
    let start = doc.clone();
    let id = save_request(&repo.0, &start).unwrap();
    // The request: a note on the page.
    doc.create(page, NodeKind::Rectangle, "Note", 40.0, 0.0, 5.0, 5.0)
        .unwrap();
    let record = record_document(
        &repo.0,
        &load_request(&repo.0, &id).unwrap(),
        &doc,
        Some(page.0),
    )
    .unwrap();
    let got = names(&record.document);
    assert!(got.contains("Note"), "{got:?}");
    assert!(
        !got.contains("Retry"),
        "the waiting edit is not recorded: {got:?}"
    );
    assert!(
        got.contains("Elsewhere"),
        "other pages stay as they are live: {got:?}"
    );
    assert!(record.had_record && record.waiting.is_empty());
    assert!(record.edits > 0);
    // Committed as the new record, the next Sync sees only the retry button.
    repo.marker(&record.document, &format!("page={} job=b explain", page.0));
    let drift = design_drift(&repo.0, &doc, Some(page.0)).unwrap();
    let added: Vec<_> = drift
        .added
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(added, ["Retry"]);
    assert!(drift.changed.is_empty() && drift.removed.is_empty());
    assert!(load_request(&repo.0, "../escape").is_err());
}

#[test]
fn an_edit_on_top_of_a_waiting_edit_keeps_waiting() {
    let repo = Repo::new();
    let (mut doc, page, card, _) = design();
    repo.marker(&doc, &format!("page={} job=a", page.0));
    doc.node_mut(card).unwrap().name = "Proposed".into();
    let start = doc.clone();
    doc.node_mut(card).unwrap().name = "Explained".into();
    let record = record_document(&repo.0, &start, &doc, Some(page.0)).unwrap();
    assert_eq!(record.document.node(card).unwrap().name, "Card");
    assert_eq!(record.waiting, [format!("Explained (id {})", card.0)]);
}

#[test]
fn without_an_earlier_record_the_page_as_found_is_recorded() {
    let repo = Repo::new();
    let (mut doc, page, _, _) = design();
    repo.commit(&doc, "Code only");
    let start = doc.clone();
    doc.create(page, NodeKind::Rectangle, "Note", 0.0, 0.0, 5.0, 5.0)
        .unwrap();
    let record = record_document(&repo.0, &start, &doc, Some(page.0)).unwrap();
    assert!(!record.had_record);
    assert_eq!(names(&record.document), names(&doc));
}

/// A page with three cards (one with inner layers), recorded by the caller.
fn cards_design() -> (Document, NodeId, [NodeId; 5]) {
    let mut doc = Document::new("Design");
    let page = doc.first_page().unwrap();
    let tabs = doc
        .create(
            page,
            NodeKind::Frame,
            "Widget Page tabs states",
            0.0,
            0.0,
            200.0,
            100.0,
        )
        .unwrap();
    let tab = doc
        .create(tabs, NodeKind::Frame, "tab Afterburn", 0.0, 0.0, 50.0, 20.0)
        .unwrap();
    let underline = doc
        .create(tab, NodeKind::Rectangle, "underline", 0.0, 18.0, 50.0, 2.0)
        .unwrap();
    let old_note = doc
        .create(
            page,
            NodeKind::Frame,
            "Old onboarding note",
            300.0,
            0.0,
            80.0,
            40.0,
        )
        .unwrap();
    let rules = doc
        .create(page, NodeKind::Frame, "Rules", 400.0, 0.0, 80.0, 40.0)
        .unwrap();
    (doc, page, [tabs, tab, underline, old_note, rules])
}

#[test]
fn waiting_layers_group_under_their_cards_with_counts_and_fields() {
    let repo = Repo::new();
    let (mut doc, page, [tabs, tab, underline, old_note, rules]) = cards_design();
    repo.marker(&doc, &format!("page={} job=a", page.0));
    // A new card with two layers, edits deep inside a card plus a new layer in it, a removed card
    // and a loose caption.
    let ide = doc
        .create(page, NodeKind::Frame, "IDE panes", 0.0, 200.0, 100.0, 50.0)
        .unwrap();
    doc.create(ide, NodeKind::Rectangle, "pane", 0.0, 0.0, 10.0, 10.0)
        .unwrap();
    doc.get_mut(tab).unwrap().height = 24.0;
    doc.get_mut(underline).unwrap().opacity = 0.5;
    doc.create(tabs, NodeKind::Rectangle, "separator", 60.0, 0.0, 1.0, 20.0)
        .unwrap();
    doc.remove(old_note).unwrap();
    let caption = doc
        .create(
            page,
            NodeKind::Text(figcraft_doc::TextData::new("A legend")),
            "Legend",
            0.0,
            300.0,
            50.0,
            10.0,
        )
        .unwrap();
    let drift = design_drift(&repo.0, &doc, Some(page.0)).unwrap();
    let cards = &drift.cards;
    let by_id = |id: NodeId| cards.iter().find(|c| c["id"] == id.0).unwrap();
    assert_eq!(cards.len(), 4, "{cards:#?}");
    assert!(
        cards.iter().all(|c| c["id"] != rules.0),
        "an untouched card is not listed"
    );
    let changed = by_id(tabs);
    assert_eq!(changed["state"], "changed");
    assert_eq!(changed["page_id"], page.0);
    let layers: Vec<(&str, &str)> = changed["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["name"].as_str().unwrap(), c["state"].as_str().unwrap()))
        .collect();
    assert_eq!(
        layers,
        [
            ("tab Afterburn", "changed"),
            ("underline", "changed"),
            ("separator", "added")
        ]
    );
    assert_eq!(changed["changes"][0]["fields"], json!(["height"]));
    let new = by_id(ide);
    assert_eq!(
        (new["state"].as_str(), new["layers"].as_u64()),
        (Some("added"), Some(2))
    );
    assert_eq!(new["changes"], json!([]));
    let gone = by_id(old_note);
    assert_eq!(
        (gone["state"].as_str(), gone["name"].as_str()),
        (Some("removed"), Some("Old onboarding note"))
    );
    assert_eq!(by_id(caption)["kind"], "text");
    let delta = diff(&repo.0, &doc, Some(page.0)).unwrap();
    assert_eq!(delta["map"]["cards"].as_array().unwrap().len(), 4);
}

#[test]
fn a_left_out_card_keeps_waiting_and_the_frozen_map_has_only_included_edits() {
    let repo = Repo::new();
    let (mut doc, page, [tabs, tab, _, old_note, _]) = cards_design();
    repo.marker(&doc, &format!("page={} job=a", page.0));
    let ide = doc
        .create(page, NodeKind::Frame, "IDE panes", 0.0, 200.0, 100.0, 50.0)
        .unwrap();
    doc.create(ide, NodeKind::Rectangle, "pane", 0.0, 0.0, 10.0, 10.0)
        .unwrap();
    doc.get_mut(tab).unwrap().height = 24.0;
    doc.remove(old_note).unwrap();
    let elsewhere = doc.add_page("Elsewhere");
    doc.create(
        elsewhere,
        NodeKind::Frame,
        "Other page card",
        0.0,
        0.0,
        5.0,
        5.0,
    )
    .unwrap();
    // Leave out the new IDE panes card and the old note's deletion.
    let excluded = BTreeSet::from([ide.0, old_note.0]);
    let included = leave_out(&repo.0, &doc, Some(page.0), &excluded).unwrap();
    assert!(
        included.node(ide).is_none(),
        "a new left-out card is not frozen"
    );
    assert_eq!(included.node(old_note).unwrap().name, "Old onboarding note");
    assert_eq!(
        included.node(tab).unwrap().height,
        24.0,
        "included edits are frozen"
    );
    assert!(
        names(&included).contains("Other page card"),
        "outside the scope stays live"
    );
    let frozen = freeze(&repo.0, &included, Some(page.0)).unwrap();
    let work = design_drift(&repo.0, &frozen_document(&frozen).unwrap(), Some(page.0)).unwrap();
    let ids: Vec<u64> = work
        .cards
        .iter()
        .map(|c| c["id"].as_u64().unwrap())
        .collect();
    assert_eq!(
        ids,
        [tabs.0],
        "the frozen blob carries only the included card"
    );
    // The agent commits through the usual gate; the integrity checks still hold.
    repo.write_design(&frozen_document(&frozen).unwrap());
    prepare_commit(&repo.0, &frozen).unwrap();
    git(
        &repo.0,
        &["commit", "-m", &format!("Sync tabs\n\n{}", frozen.trailer)],
    )
    .unwrap();
    let done = find_completed(&repo.0, &frozen).unwrap().unwrap();
    assert_eq!(
        baseline(&repo.0, Some(page.0)).unwrap().unwrap().commit,
        done.commit
    );
    // The next Sync sees exactly the two left-out cards still waiting.
    let next = design_drift(&repo.0, &doc, Some(page.0)).unwrap();
    let waiting: BTreeSet<u64> = next
        .cards
        .iter()
        .map(|c| c["id"].as_u64().unwrap())
        .collect();
    assert_eq!(waiting, BTreeSet::from([ide.0, old_note.0]));
    // Nothing left out: the live map is frozen as it is.
    assert_eq!(
        names(&leave_out(&repo.0, &doc, Some(page.0), &BTreeSet::new()).unwrap()),
        names(&doc)
    );
}

#[test]
fn a_card_only_moved_or_slightly_resized_is_not_waiting() {
    let repo = Repo::new();
    let (mut doc, page, card, _) = design();
    let frame = doc
        .create(page, NodeKind::Frame, "Widget", 100.0, 0.0, 200.0, 100.0)
        .unwrap();
    let inner = doc
        .create(frame, NodeKind::Rectangle, "Button", 10.0, 10.0, 40.0, 20.0)
        .unwrap();
    repo.marker(&doc, &format!("page={}", page.0));
    let waiting = |doc: &Document| {
        let drift = design_drift(&repo.0, doc, Some(page.0)).unwrap();
        let ids: Vec<u64> = drift
            .cards
            .iter()
            .filter_map(|c| c["id"].as_u64())
            .collect();
        (drift.is_empty(), ids)
    };
    // Moved across the map, and a little wider and shorter: arrangement, nothing to build.
    for id in [card, frame] {
        let node = doc.get_mut(id).unwrap();
        node.transform = node.transform.then_translate((300.0, 67.0).into());
    }
    doc.get_mut(card).unwrap().width = 21.0;
    doc.get_mut(frame).unwrap().height = 92.0;
    assert_eq!(
        waiting(&doc),
        (true, vec![]),
        "moves and slight resizes wait for nothing"
    );
    // A real resize is design.
    doc.get_mut(card).unwrap().width = 30.0;
    assert_eq!(waiting(&doc), (false, vec![card.0]));
    doc.get_mut(card).unwrap().width = 21.0;
    // So is a move inside a card: the layout it designs.
    let node = doc.get_mut(inner).unwrap();
    node.transform = node.transform.then_translate((5.0, 0.0).into());
    assert_eq!(waiting(&doc), (false, vec![frame.0]));
    let node = doc.get_mut(inner).unwrap();
    node.transform = node.transform.then_translate((-5.0, 0.0).into());
    // And a rotated card.
    let node = doc.get_mut(card).unwrap();
    node.transform = node.transform * figcraft_doc::geom::Affine::rotate(0.3);
    assert_eq!(waiting(&doc), (false, vec![card.0]));
}

#[test]
fn marking_a_pages_mode_never_waits_for_a_sync() {
    let repo = Repo::new();
    let (doc, page, _, _) = design();
    repo.marker(&doc, &format!("page={} job=a", page.0));
    let mut live = doc.clone();
    live.node_mut(page)
        .unwrap()
        .plugin_data
        .insert(PAGE_MODE_KEY.into(), "exploration".into());
    let drift = design_drift(&repo.0, &live, Some(page.0)).unwrap();
    assert!(drift.is_empty(), "{:?}", drift.changes);
    // Any other page data still does.
    live.node_mut(page)
        .unwrap()
        .plugin_data
        .insert("afterburn.parent_page".into(), "7".into());
    let drift = design_drift(&repo.0, &live, Some(page.0)).unwrap();
    assert!(!drift.is_empty());
}

#[test]
fn a_map_committed_before_the_rename_still_baselines_under_the_new_keys() {
    let repo = Repo::new();
    let (mut doc, page, node, _) = design();
    doc.node_mut(node)
        .unwrap()
        .plugin_data
        .insert("mindmap.parent_page".into(), page.0.to_string());
    fs::create_dir_all(repo.0.join(".mindmap")).unwrap();
    fs::write(
        repo.0.join(LEGACY_DESIGN_PATH),
        figcraft_format::save_bytes(&doc, None).unwrap(),
    )
    .unwrap();
    git(&repo.0, &["add", "--", LEGACY_DESIGN_PATH]).unwrap();
    git(
        &repo.0,
        &["commit", "-m", "Old record\n\nMindmap-Sync: all explain"],
    )
    .unwrap();
    let old = head(&repo.0).unwrap().unwrap();

    let found = baseline(&repo.0, None)
        .unwrap()
        .expect("the Mindmap-Sync record still counts");
    assert_eq!(found.commit, old);
    let committed = load_design(&repo.0, &old).unwrap();
    let keys = &committed.node(node).unwrap().plugin_data;
    assert_eq!(keys.get("afterburn.parent_page"), Some(&page.0.to_string()));
    assert!(!keys.contains_key("mindmap.parent_page"));

    // The worktree moved the map: staging the new one also stages the old one's removal.
    fs::remove_file(repo.0.join(LEGACY_DESIGN_PATH)).unwrap();
    repo.write_design(&committed);
    stage_design(&repo.0).unwrap();
    let staged = git_text(&repo.0, &["diff", "--cached", "--name-status"]).unwrap();
    assert!(
        staged.contains(&format!("D\t{LEGACY_DESIGN_PATH}")),
        "{staged}"
    );
    assert!(staged.contains(&format!("A\t{DESIGN_PATH}")), "{staged}");
    // With nothing left to remove, staging is unchanged.
    git(&repo.0, &["commit", "-m", "Move the map"]).unwrap();
    stage_design(&repo.0).unwrap();
}

#[test]
fn legacy_keys_move_to_afterburn_and_a_new_key_wins() {
    let (mut doc, _, node, _) = design();
    let data = &mut doc.node_mut(node).unwrap().plugin_data;
    data.insert("mindmap.page_mode".into(), "exploration".into());
    data.insert("mindmap.source_node".into(), "old".into());
    data.insert("afterburn.source_node".into(), "new".into());
    data.insert("other.key".into(), "kept".into());
    assert!(migrate_legacy_keys(&mut doc));
    let data = &doc.node(node).unwrap().plugin_data;
    assert_eq!(
        data.get("afterburn.page_mode").map(String::as_str),
        Some("exploration")
    );
    assert_eq!(
        data.get("afterburn.source_node").map(String::as_str),
        Some("new")
    );
    assert_eq!(data.get("other.key").map(String::as_str), Some("kept"));
    assert!(data.keys().all(|key| !key.starts_with("mindmap.")));
    assert!(!migrate_legacy_keys(&mut doc), "nothing left to move");
}
