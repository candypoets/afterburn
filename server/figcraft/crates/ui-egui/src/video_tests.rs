//! Video clips on the map canvas, driven headless: the play button starts a clip, frames arrive
//! and time moves, the bar pauses it, a seek lands, a clip whose file is missing says so, and a
//! page with a clip that is not playing leaves the window asleep.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use afterburn_video::{Clip, MEDIA_DIR, VIDEO_KEY};
use egui::{Event, pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::video::{Snapshot, Source, Status};

const FILE: &str = "0123456789abcdef0123456789abcdef.mp4";

fn sample() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../afterburn-video/testdata/testsrc.mp4")
}

/// A scratch worktree holding the test clip, and the card that names it.
fn setup(name: &str, with_file: bool) -> (Ui, NodeId, PathBuf) {
    let worktree = std::env::temp_dir().join(format!("afterburn-video-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&worktree);
    std::fs::create_dir_all(worktree.join(MEDIA_DIR)).unwrap();
    let size = std::fs::metadata(sample()).unwrap().len();
    if with_file {
        std::fs::copy(sample(), worktree.join(MEDIA_DIR).join(FILE)).unwrap();
    }
    let clip = Clip { file: FILE.into(), width: 320, height: 180, duration_ms: 4000, size, audio: false };
    let mut s = Session::new();
    let card = NodeId(
        s.execute("node.create", &json!({"type": "rectangle", "name": "Video", "x": 0, "y": 0, "width": 640, "height": 360})).unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    s.execute("node.set", &json!({"ids": [card], "pluginData": {VIDEO_KEY: serde_json::to_string(&clip).unwrap()}})).unwrap();
    s.execute("select.set", &json!({"ids": []})).unwrap();
    let mut u = Ui::new(s);
    u.app.onboarding.active = false;
    u.app.onboarding.decided = true;
    u.app.ui.home = false;
    u.app.ui.videos.source_override = Some(Source { node: None, worktree: Some(worktree.clone()), cache: worktree.join("cache") });
    u.app.run("view.set", json!({"zoom": 1, "cx": 320, "cy": 180}));
    u.frame(vec![]);
    u.frame(vec![]);
    (u, card, worktree)
}

fn click(u: &mut Ui, p: egui::Pos2) {
    u.frame(vec![Event::PointerMoved(p)]);
    u.frame(vec![Event::PointerMoved(p)]);
    u.button(p, true);
    u.button(p, false);
    u.frame(vec![]);
}

/// Draw frames until `done` says so (the worker runs on its own clock).
fn until(u: &mut Ui, card: NodeId, what: &str, done: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let start = Instant::now();
    loop {
        u.frame(vec![]);
        let snapshot = u.app.ui.videos.snapshot(card);
        if let Some(snapshot) = &snapshot
            && done(snapshot)
        {
            return snapshot.clone();
        }
        assert!(start.elapsed() < Duration::from_secs(10), "{what}: {snapshot:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn rect(u: &Ui, card: NodeId) -> egui::Rect {
    crate::video::cards(&u.app).into_iter().find(|(id, _, _)| *id == card).unwrap().2
}

#[test]
fn the_play_button_plays_the_bar_pauses_and_a_seek_lands() {
    crate::with_map_mode(|| {
        let (mut u, card, worktree) = setup("play", true);
        let r = rect(&u, card);
        assert!(u.app.ui.videos.snapshot(card).is_none(), "nothing runs before Play");
        click(&mut u, r.center());
        let playing = until(&mut u, card, "it plays", |s| s.playing && s.has_frame && s.position_ms > 100);
        assert_eq!(playing.status, Status::Ready);
        // The pause button, at the left of the bar along the bottom.
        click(&mut u, pos2(r.left() + 15.0, r.bottom() - 15.0));
        let paused = until(&mut u, card, "it pauses", |s| !s.playing);
        std::thread::sleep(Duration::from_millis(120));
        u.frame(vec![]);
        assert_eq!(u.app.ui.videos.snapshot(card).unwrap().position_ms, paused.position_ms, "a paused clip stands still");
        u.app.ui.videos.seek(card, 3000);
        until(&mut u, card, "the seek lands", |s| s.position_ms == 3000 && !s.playing);
        // Play from there runs to the end and stops on the last frame.
        click(&mut u, r.center());
        until(&mut u, card, "it ends", |s| !s.playing && s.position_ms >= 3900);
        // Leaving the page drops the player.
        u.app.session.execute("page.add", &json!({"name": "Other"})).unwrap();
        u.frame(vec![]);
        assert!(u.app.ui.videos.snapshot(card).is_none());
        let _ = std::fs::remove_dir_all(worktree);
    });
}

#[test]
fn a_missing_clip_says_so() {
    crate::with_map_mode(|| {
        let (mut u, card, worktree) = setup("missing", false);
        let r = rect(&u, card);
        click(&mut u, r.center());
        let failed = until(&mut u, card, "it fails", |s| matches!(s.status, Status::Failed(_)));
        assert!(!failed.playing);
        assert!(matches!(&failed.status, Status::Failed(message) if message.contains(".afterburn/media")), "{failed:?}");
        let _ = std::fs::remove_dir_all(worktree);
    });
}

#[test]
fn a_clicked_play_button_does_not_select_the_card() {
    crate::with_map_mode(|| {
        let (mut u, card, worktree) = setup("select", true);
        let r = rect(&u, card);
        click(&mut u, r.center());
        until(&mut u, card, "it plays", |s| s.playing && s.has_frame);
        assert!(u.app.session.selection().is_empty(), "Play is not a selection");
        let _ = std::fs::remove_dir_all(worktree);
    });
}
