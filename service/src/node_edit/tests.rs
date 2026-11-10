use super::*;

/// A scratch worktree with the test clip as `footage/a.mp4` (4 s, with sound) and a one-second
/// tone as `voice/hello.wav`. These tests need ffmpeg, as the feature does.
fn worktree(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("afterburn-edit-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("footage")).unwrap();
    std::fs::create_dir_all(dir.join("voice")).unwrap();
    let sample = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../server/figcraft/crates/afterburn-video/testdata/testsrc.mp4");
    std::fs::copy(sample, dir.join("footage/a.mp4")).unwrap();
    let made = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=330:duration=1",
        ])
        .arg(dir.join("voice/hello.wav"))
        .status()
        .unwrap();
    assert!(made.success());
    dir
}

fn cut_list() -> Value {
    json!({
        "name": "demo", "width": 320, "height": 180, "fps": 24, "target_seconds": 3.0,
        "clips": [
            {"id": "one", "source": "footage/a.mp4", "in": 0.0, "out": 2.0, "title": "One"},
            {"id": "two", "source": "footage/a.mp4", "in": 2.0, "out": 3.5, "crossfade": 0.5},
        ],
        "voice": [{"id": "hello", "source": "voice/hello.wav", "at": 0.5, "text": "Hello"}],
        "captions": [{"text": "Hello: it's \"quoted\", 100%", "at": 0.6, "until": 1.4, "voice": "hello"}],
        "music": {"source": "voice/hello.wav", "gain_db": -24},
    })
}

#[test]
fn the_gates_give_every_reason_at_once() {
    let dir = worktree("gates");
    let edit: Edit = serde_json::from_value(json!({
        "name": "Bad Name", "target_seconds": 30.0,
        "clips": [
            {"id": "one", "source": "footage/a.mp4", "in": 0.0, "out": 9.0, "crossfade": 1.0},
            {"id": "one", "source": "footage/missing.mp4", "in": 0.0, "out": 1.0},
            {"id": "esc", "source": "../../etc/passwd", "in": 0.0, "out": 1.0},
        ],
        "voice": [{"id": "hello", "source": "voice/hello.wav", "at": 99.0}],
        "captions": [{"text": "late", "at": 0.0, "until": 0.5, "voice": "hello"}, {"text": "none", "at": 0.6, "until": 0.9, "voice": "nobody"}],
    }))
    .unwrap();
    let errors = check(&dir, edit).err().unwrap().join("\n");
    for expected in [
        "name:",
        "past the end of footage/a.mp4",
        "nothing to fade from",
        "id must be unique",
        "footage/missing.mp4: no such file",
        "../../etc/passwd",
        "the target is 30.000s",
        "past the end of the edit",
        "outside its voice hello",
        "no voice segment nobody",
    ] {
        assert!(
            errors.contains(expected),
            "missing {expected:?} in:\n{errors}"
        );
    }
    // An unknown field is a mistake, not ignored.
    assert!(
        serde_json::from_value::<Edit>(json!({"name": "x", "clips": [], "colour": "red"})).is_err()
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_edit_renders_draws_and_redraws_in_place() {
    let dir = worktree("apply");
    let (plan, stored, probed) = build(&dir, &cut_list()).unwrap();
    assert!((plan.total - 3.0).abs() < 1e-9);
    assert_eq!(plan.starts, vec![0.0, 1.5]);
    assert!(
        probed.audio && (probed.seconds - 3.0).abs() < 0.2,
        "{probed:?}"
    );
    assert_eq!((stored.clip.width, stored.clip.height), (320, 180));
    let file = dir.join(EDITS_DIR).join("demo.json");
    let saved: Edit = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(saved.clips.len(), 2);

    let mut engine = figcraft_engine::Session::new();
    let page = engine.page().unwrap().0;
    assert!(
        place(&mut engine, &plan, &stored, &probed, &json!({})).is_err(),
        "the first apply needs a parent"
    );
    let drawn = place(
        &mut engine,
        &plan,
        &stored,
        &probed,
        &json!({"parent": page, "x": 40, "y": 40}),
    )
    .unwrap();
    let (frame, take) = (
        drawn["frame"].as_u64().unwrap(),
        NodeId(drawn["take"].as_u64().unwrap()),
    );
    let item = |doc: &Document, key: &str| {
        doc.node(NodeId(frame))
            .unwrap()
            .children
            .iter()
            .copied()
            .find(|id| {
                doc.node(*id)
                    .unwrap()
                    .plugin_data
                    .get(ITEM_KEY)
                    .map(String::as_str)
                    == Some(key)
            })
    };
    let doc = engine.doc().unwrap().clone();
    assert_eq!(
        Clip::parse(doc.node(take).unwrap().plugin_data.get(VIDEO_KEY).unwrap()).unwrap(),
        stored.clip
    );
    assert_eq!(
        current_take(&doc, "demo").as_deref(),
        Some(stored.clip.file.as_str())
    );
    let (one, two) = (
        item(&doc, "shot one").unwrap(),
        item(&doc, "shot two").unwrap(),
    );
    assert_eq!(
        doc.node(two).unwrap().transform.translation().x,
        LANE_X + 1.5 * PPS
    );

    // A re-cut: shot two goes, shot one gets shorter. Same frame, same layers, a new take.
    let mut recut = cut_list();
    recut["clips"] = json!([{"id": "one", "source": "footage/a.mp4", "in": 0.0, "out": 3.0}]);
    let (plan2, stored2, probed2) = build(&dir, &recut).unwrap();
    assert_ne!(stored2.clip.file, stored.clip.file);
    let redrawn = place(&mut engine, &plan2, &stored2, &probed2, &json!({})).unwrap();
    assert_eq!(redrawn["frame"].as_u64(), Some(frame));
    assert_eq!(redrawn["take"].as_u64(), Some(take.0));
    let doc = engine.doc().unwrap().clone();
    assert_eq!(item(&doc, "shot one"), Some(one), "a shot keeps its layer");
    assert_eq!(item(&doc, "shot two"), None, "a cut shot's layer goes");
    assert_eq!(doc.node(one).unwrap().width, 3.0 * PPS - 1.0);
    // A shot is a frame holding its words: the map selects it as one card.
    let shot = doc.node(one).unwrap();
    assert!(matches!(shot.kind, figcraft_doc::NodeKind::Frame));
    let title = shot
        .children
        .iter()
        .copied()
        .find(|id| {
            doc.node(*id)
                .unwrap()
                .plugin_data
                .get(ITEM_KEY)
                .map(String::as_str)
                == Some("shot title one")
        })
        .unwrap();
    assert_eq!(
        doc.node(title).unwrap().kind.text().unwrap().characters,
        "one"
    );
    // A caption shows all of its words, wrapped inside its block.
    let caption = item(&doc, "caption 0").unwrap();
    let words = doc.node(caption).unwrap().children[0];
    let words = doc.node(words).unwrap();
    assert_eq!(
        words.kind.text().unwrap().characters,
        "Hello: it's \"quoted\", 100%"
    );
    assert!(
        words.height <= doc.node(caption).unwrap().height,
        "the block holds its words"
    );
    assert!(
        locked(
            &doc,
            "node.set",
            &json!({"ids": [title.0], "characters": "x"})
        )
        .is_some(),
        "nested layers are locked too"
    );
    assert!(locked(
        &doc,
        "node.create",
        &json!({"type": "text", "parent": one.0})
    )
    .is_some());
    // The first take is on no card now: its file goes.
    let media = dir.join(afterburn_video::MEDIA_DIR);
    assert!(media.join(&stored.clip.file).is_file());
    drop_unused_takes(&dir, &doc, Some(&stored.clip.file));
    assert!(!media.join(&stored.clip.file).exists());
    assert!(media.join(&stored2.clip.file).is_file());
    drop_unused_takes(&dir, &doc, Some(&stored2.clip.file));
    assert!(
        media.join(&stored2.clip.file).is_file(),
        "the take in use stays"
    );

    // The lock: generated layers change only through edit.apply.
    for (command, params) in [
        ("node.set", json!({"ids": [one.0], "x": 5})),
        ("edit.delete", json!({"ids": [take.0]})),
        ("node.create", json!({"type": "rectangle", "parent": frame})),
        (
            "command.batch",
            json!({"commands": [{"command": "node.set", "params": {"ids": [one.0], "name": "x"}}]}),
        ),
    ] {
        assert!(locked(&doc, command, &params).is_some(), "{command}");
    }
    assert!(
        locked(&doc, "node.set", &json!({"ids": [frame], "x": 5})).is_none(),
        "the frame itself moves"
    );
    assert!(locked(
        &doc,
        "node.create",
        &json!({"type": "rectangle", "parent": page})
    )
    .is_none());
    assert!(locked(&doc, "select.set", &json!({"ids": [one.0]})).is_none());

    // A refused edit changes nothing on disk.
    let before = std::fs::read_to_string(&file).unwrap();
    let mut bad = cut_list();
    bad["target_seconds"] = json!(10.0);
    let refused = build(&dir, &bad).err().unwrap().to_string();
    assert!(
        refused.contains("edit refused") && refused.contains("the target is 10.000s"),
        "{refused}"
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn frames_are_a_contact_sheet_and_probes_are_numbers() {
    let dir = worktree("frames");
    let clip = dir.join("footage/a.mp4");
    let sheet = frames(&clip, &json!({"times": [0.0, 1.0, 2.0, 3.5]})).unwrap();
    assert_eq!(sheet["columns"], 3);
    let seconds: Vec<f64> = sheet["seconds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_f64().unwrap())
        .collect();
    assert_eq!(seconds.len(), 4);
    assert!(
        (seconds[1] - 1.0).abs() < 0.05 && (seconds[3] - 3.5).abs() < 0.05,
        "{seconds:?}"
    );
    let png = base64::engine::general_purpose::STANDARD
        .decode(sheet["data"].as_str().unwrap())
        .unwrap();
    let picture = image::load_from_memory(&png).unwrap();
    assert_eq!(
        (picture.width(), picture.height()),
        (3 * 384 + 4 * 6, 2 * 216 + 3 * 6)
    );
    assert_eq!(
        frames(&clip, &json!({})).unwrap()["seconds"]
            .as_array()
            .unwrap()
            .len(),
        6
    );

    let numbers = probe(&clip).unwrap();
    assert!(numbers.audio && (numbers.seconds - 4.0).abs() < 0.2);
    assert!(
        numbers
            .loudness_lufs
            .is_some_and(|l| (-40.0..0.0).contains(&l)),
        "{numbers:?}"
    );
    assert!(
        numbers.black_seconds < 0.5 && numbers.silence_seconds < 0.5,
        "{numbers:?}"
    );
    let _ = std::fs::remove_dir_all(dir);
}
