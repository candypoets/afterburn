//! Check for Updates: the feed, version compare, the sheet's states and what each one shows.

use std::cmp::Ordering;

use egui::{Pos2, RawInput, Rect, vec2};
use figcraft_engine::Session;
use serde_json::json;

use crate::update::{
    Act, Checked, Installed, NO_FEED, Phase, Release, STATES, State, compare_versions, decide, due, feed_url_from, parse_feed, platform_name,
    short_build, view,
};
use crate::{FigApp, NoServices};

const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn feed_json(version: &str) -> serde_json::Value {
    json!({
        "version": version,
        "notes": ["Faster map open and sync checks", "Update from the app, no terminal", "Smaller node, cleaner logs"],
        "platforms": {
            "darwin-arm64": {
                "app": {"url": "https://releases.example/Mindmap-0.8.0.zip", "sha256": SHA},
                "node": {"url": "https://releases.example/mindmap-node", "sha256": SHA, "build_id": "3a1c07e5b2"}
            }
        }
    })
}

fn release() -> Release {
    match decide(&parse_feed(&feed_json("0.8.0").to_string()).unwrap(), Some("darwin-arm64"), "0.7.0") {
        Checked::Available(release) => release,
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_feed_comes_from_the_environment_else_the_built_in_default_which_is_empty() {
    assert_eq!(crate::update::DEFAULT_FEED, None, "no release pipeline publishes a feed yet");
    assert_eq!(feed_url_from(None, None), None);
    assert_eq!(feed_url_from(Some("  ".into()), None), None);
    assert_eq!(feed_url_from(Some(" https://a/feed.json ".into()), Some("https://b")), Some("https://a/feed.json".into()));
    assert_eq!(feed_url_from(None, Some("https://b/feed.json")), Some("https://b/feed.json".into()));
}

#[test]
fn feeds_parse_and_bad_ones_are_refused() {
    let feed = parse_feed(&feed_json("0.8.0").to_string()).unwrap();
    assert_eq!(feed.version, "0.8.0");
    assert_eq!(feed.notes.len(), 3);
    assert_eq!(feed.platforms["darwin-arm64"].node.build_id, "3a1c07e5b2");
    assert!(parse_feed("not json").is_err());
    assert!(parse_feed(&feed_json("soon").to_string()).is_err());
    let mut insecure = feed_json("0.8.0");
    insecure["platforms"]["darwin-arm64"]["app"]["url"] = json!("http://releases.example/app.zip");
    assert!(parse_feed(&insecure.to_string()).unwrap_err().contains("HTTPS"));
    let mut short = feed_json("0.8.0");
    short["platforms"]["darwin-arm64"]["node"]["sha256"] = json!("abc");
    assert!(parse_feed(&short.to_string()).unwrap_err().contains("SHA-256"));
    let mut anonymous = feed_json("0.8.0");
    anonymous["platforms"]["darwin-arm64"]["node"]["build_id"] = json!(" ");
    assert!(parse_feed(&anonymous.to_string()).is_err());
    assert!(parse_feed(&json!({"version": "0.8.0"}).to_string()).is_ok(), "notes and platforms are optional");
}

#[test]
fn versions_compare_numerically_with_prereleases_first() {
    assert_eq!(compare_versions("0.8.0", "0.7.0"), Ordering::Greater);
    assert_eq!(compare_versions("0.10.0", "0.9.9"), Ordering::Greater);
    assert_eq!(compare_versions("v0.7", "0.7.0"), Ordering::Equal);
    assert_eq!(compare_versions("0.8.0-beta.1", "0.8.0"), Ordering::Less);
    assert_eq!(compare_versions("0.8.0", "0.8.0-rc.1"), Ordering::Greater);
    assert_eq!(compare_versions("1.0.0", "garbage"), Ordering::Equal, "never offered against an unreadable version");
}

#[test]
fn a_newer_release_for_this_platform_is_available_and_nothing_else_is() {
    let feed = parse_feed(&feed_json("0.8.0").to_string()).unwrap();
    assert!(matches!(decide(&feed, Some("darwin-arm64"), "0.7.0"), Checked::Available(r) if r.version == "0.8.0" && r.notes.len() == 3));
    assert_eq!(decide(&feed, Some("darwin-arm64"), "0.8.0"), Checked::UpToDate(None));
    assert_eq!(decide(&feed, Some("darwin-arm64"), "0.9.0"), Checked::UpToDate(None));
    assert!(matches!(decide(&feed, Some("darwin-x64"), "0.7.0"), Checked::UpToDate(Some(note)) if note.contains("no build for this machine")));
    assert_eq!(platform_name("macos", "aarch64").as_deref(), Some("darwin-arm64"));
    assert_eq!(platform_name("macos", "x86_64").as_deref(), Some("darwin-x64"));
    assert_eq!(platform_name("windows", "x86_64"), None);
}

#[test]
fn checks_are_due_at_first_launch_and_daily() {
    assert!(due(None, 1_000));
    assert!(!due(Some(1_000), 1_000 + 3_600));
    assert!(due(Some(1_000), 1_000 + 24 * 3_600));
    assert!(due(Some(5_000), 1_000), "a clock that went back checks again");
}

#[test]
fn the_state_machine_moves_through_the_designs_states() {
    let mut state = State::default();
    state.checking();
    assert_eq!(view(&state, "0.7.0", true).state, "Checking");
    state.checked(Checked::UpToDate(Some(NO_FEED.into())), Some("fe9dca11".into()), true);
    assert!(!state.open, "an automatic check that finds nothing stays quiet");
    let shown = view(&state, "0.7.0", true);
    assert_eq!(shown.state, "Up to date");
    assert_eq!(shown.note, Some((NO_FEED.to_string(), false)), "no feed is a muted note, not an error");

    state.checked(Checked::Available(release()), Some("fe9dca11".into()), true);
    assert!(state.open, "an automatic check that finds a release opens the sheet");
    assert_eq!(view(&state, "0.7.0", true).state, "Available");
    state.later();
    assert!(!state.open);
    state.checked(Checked::Available(release()), Some("fe9dca11".into()), true);
    assert!(!state.open, "Later keeps that version from opening the sheet by itself again");

    state.open = true;
    assert_eq!(state.downloading(), Some(release()));
    assert_eq!(view(&state, "0.7.0", true).state, "Downloading");
    state.close();
    assert!(state.open, "a running install keeps the sheet");
    state.installed(Installed::Waits("A prepared Sync is open.".into()));
    assert!(matches!(&state.phase, Phase::Waiting { reason, .. } if reason == "A prepared Sync is open."));
    assert_eq!(state.downloading(), Some(release()), "the waiting update can be tried again");
    state.installed(Installed::Failed("The download failed.".into()));
    assert_eq!(view(&state, "0.7.0", true).state, "Failed");
    assert_eq!(state.downloading(), Some(release()), "a failed install can be tried again");
    state.installed(Installed::Restarting);
    assert_eq!(view(&state, "0.7.0", true).state, "Restarting");

    let mut failed = State::default();
    failed.checked(Checked::Failed("Could not read the release feed.".into()), None, false);
    let shown = view(&failed, "0.7.0", true);
    assert_eq!(shown.state, "Failed");
    assert_eq!(shown.primary.map(|b| b.act), Some(Act::CheckAgain), "a failed check checks again");
    let shown_states: Vec<_> = [
        Phase::Checking,
        Phase::UpToDate(None),
        Phase::Available(release()),
        Phase::Downloading(release()),
        Phase::Restarting(release()),
        Phase::Failed { error: "x".into(), release: None },
    ]
    .into_iter()
    .map(|phase| view(&State::at(phase), "0.7.0", true).state)
    .collect();
    assert_eq!(shown_states, STATES);
}

#[test]
fn the_available_sheet_says_what_the_design_says() {
    let mut state = State::at(Phase::Available(release()));
    state.node_build = Some("fe9dca42".into());
    let shown = view(&state, "0.7.0", true);
    assert_eq!(shown.title, "Mindmap 0.8.0 is ready");
    assert_eq!(shown.body, "The app and its node update together. Agents keep running; a prepared Sync waits until the restart.");
    assert_eq!(
        shown.rows,
        vec![("Mindmap", "0.7.0  →  0.8.0".to_string()), ("Node", "fe9dca  →  3a1c07".to_string()), ("Agents", "Herdr keeps running".to_string())]
    );
    assert_eq!(shown.notes, ["Faster map open and sync checks", "Update from the app, no terminal", "Smaller node, cleaner logs"]);
    let primary = shown.primary.unwrap();
    assert_eq!((primary.label, primary.enabled, primary.act), ("Install and restart", true, Act::Install));
    assert_eq!(shown.secondary.map(|b| (b.label, b.act)), Some(("Later", Act::Later)));
    // Elsewhere than the Mac app the release shows but can't install.
    let elsewhere = view(&state, "0.7.0", false);
    assert!(!elsewhere.primary.unwrap().enabled);
    assert!(elsewhere.note.is_some());
    assert_eq!(short_build("3a1c07e5b2"), "3a1c07");
    let unreachable = view(&State::at(Phase::Available(release())), "0.7.0", true);
    assert_eq!(unreachable.rows[1].1, "not reachable  →  3a1c07");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_node_installers_exit_21_means_the_update_waits() {
    use crate::update_install::installer_outcome;
    assert_eq!(installer_outcome(Some(0)), Ok(()));
    assert!(matches!(installer_outcome(Some(21)), Err(Installed::Waits(_))));
    assert!(matches!(installer_outcome(Some(22)), Err(Installed::Failed(_))));
    assert!(matches!(installer_outcome(Some(1)), Err(Installed::Failed(_))));
    assert!(matches!(installer_outcome(None), Err(Installed::Failed(_))));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn downloads_are_checked_against_their_sha256() {
    use sha2::{Digest, Sha256};
    let path = std::env::temp_dir().join(format!("mindmap-update-test-{}", std::process::id()));
    std::fs::write(&path, b"synthetic release").unwrap();
    assert_eq!(crate::update_install::sha256_file(&path).unwrap(), format!("{:x}", Sha256::digest(b"synthetic release")));
    let _ = std::fs::remove_file(&path);
}

fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

#[test]
fn the_sheet_draws_the_available_release_with_install_and_later() {
    let mut app = FigApp::new(Session::new(), Box::new(NoServices));
    app.update.open = true;
    app.update.node_build = Some("fe9dca42".into());
    app.update.phase = Phase::Available(release());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx, false, 0);
    let mut time = 0.0f64;
    let mut run = |app: &mut FigApp| {
        time += 1.0 / 60.0;
        let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), time: Some(time), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            crate::update::show(app, &ctx);
        });
        out.textures_delta.clear();
        texts(&out.shapes)
    };
    run(&mut app);
    let shown = run(&mut app);
    let versions = format!("{}  →  0.8.0", mindmap_build::VERSION);
    for needle in [
        "UPDATE",
        "Mindmap 0.8.0 is ready",
        "Mindmap",
        versions.as_str(),
        "fe9dca  →  3a1c07",
        "Herdr keeps running",
        "WHAT'S NEW",
        "Install and restart",
        "Later",
        crate::update::SAFETY,
        crate::update::ENTRY,
    ] {
        assert!(shown.iter().any(|text| text == needle), "{needle:?} not in {shown:?}");
    }
    assert!(shown.iter().any(|text| text.contains("• Faster map open and sync checks")), "{shown:?}");
    assert!(app.update.open);
    // The opener runs from the catalog's action, as the App menu will.
    let mut fresh = FigApp::new(Session::new(), Box::new(NoServices));
    assert!(crate::map_action_catalog::find("app.checkForUpdates").is_some_and(|a| a.label == "Check for Updates…"));
    fresh.update.phase = Phase::Downloading(release());
    assert!(crate::map_actions::run(&mut fresh, &ctx, "app.checkForUpdates"));
    assert!(fresh.update.open, "the action opens the sheet");
    assert!(matches!(fresh.update.phase, Phase::Downloading(_)), "opening it never restarts a running install");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn curl_takes_the_url_last_and_reads_a_local_feed() {
    let command = crate::update_install::curl("https://releases.example/feed.json", &["--max-time", "30"]);
    let args: Vec<_> = command.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
    assert_eq!(args[args.len() - 4..], ["--max-time", "30", "--", "https://releases.example/feed.json"], "options before `--`");
    let path = std::env::temp_dir().join(format!("mindmap-update-feed-{}.json", std::process::id()));
    std::fs::write(&path, feed_json("0.8.0").to_string()).unwrap();
    if let Ok(text) = crate::update_install::fetch_text(&format!("file://{}", path.display())) {
        assert_eq!(parse_feed(&text).unwrap().version, "0.8.0");
    }
    let _ = std::fs::remove_file(&path);
}
