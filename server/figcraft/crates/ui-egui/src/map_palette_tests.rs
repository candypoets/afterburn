//! Afterburn's palette, driven through egui headless: ⌘P goes to a page by name or path, `>` turns
//! it into the actions and ⌫ back, ⌃⌘O lists the workspaces (a workspace with several worktrees
//! asks which one first), ⌘, flips settings, and the status bar's agent count lists the agents.
use crate::layers_drag_tests::Ui;
use crate::map_palette::{Right, Scope};
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use figcraft_doc::NodeId;
use figcraft_engine::{Session, keymap::Platform};
use serde_json::json;

fn cmd() -> Modifiers {
    if crate::shortcuts::platform() == Platform::Mac { Modifiers::MAC_CMD | Modifiers::COMMAND } else { Modifiers::CTRL | Modifiers::COMMAND }
}

/// ⌃⌘ on a Mac; Ctrl+Alt elsewhere.
fn ctrl_cmd() -> Modifiers {
    if crate::shortcuts::platform() == Platform::Mac {
        Modifiers::CTRL | Modifiers::MAC_CMD | Modifiers::COMMAND
    } else {
        Modifiers::CTRL | Modifiers::ALT | Modifiers::COMMAND
    }
}

fn press(u: &mut Ui, key: Key, modifiers: Modifiers) {
    let ev = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
    u.frame_with(vec![ev(true), ev(false)], modifiers);
}

fn type_text(u: &mut Ui, text: &str) {
    u.frame(vec![Event::Text(text.into())]);
    u.frame(vec![]);
}

fn click(u: &mut Ui, at: Pos2) {
    u.frame(vec![Event::PointerMoved(at)]);
    let ev = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    u.frame(vec![ev(true)]);
    u.frame(vec![ev(false)]);
}

/// The pages of the design's example: three roots, each with a branch of subpages.
struct Pages {
    overview: NodeId,
    gradients: NodeId,
    graphs: NodeId,
    programs: NodeId,
}

fn setup() -> (Ui, Pages) {
    let mut s = Session::new();
    let overview = s.page().unwrap();
    s.execute("page.rename", &json!({"id": overview.0, "name": "Overview"})).unwrap();
    let add = |s: &mut Session, name: &str, parent: Option<NodeId>| {
        let page = NodeId(s.execute("page.add", &json!({"name": name})).unwrap()["id"].as_u64().unwrap());
        if let Some(parent) = parent {
            s.execute(
                "node.set",
                &json!({"id": page.0, "pluginData": {crate::page_tree::PARENT_KEY: parent.0.to_string(), crate::page_tree::SOURCE_KEY: parent.0.to_string()}}),
            )
            .unwrap();
        }
        page
    };
    let canvas = add(&mut s, "Canvas and layers", Some(overview));
    let paint = add(&mut s, "Paint", Some(canvas));
    let fills = add(&mut s, "Fills", Some(paint));
    let gradients = add(&mut s, "Gradients", Some(fills));
    let components = add(&mut s, "Components", None);
    let onboarding = add(&mut s, "Onboarding", Some(components));
    let programs = add(&mut s, "Programs", Some(onboarding));
    let checkout = add(&mut s, "Checkout", None);
    let reports = add(&mut s, "Reports", Some(checkout));
    let graphs = add(&mut s, "Graphs and charts", Some(reports));
    s.execute("page.select", &json!({"id": overview.0})).unwrap();
    let mut u = Ui::new(s);
    u.app.onboarding.active = false;
    u.app.onboarding.decided = true;
    u.app.ui.home = false;
    u.frame(vec![]);
    u.frame(vec![]);
    (u, Pages { overview, gradients, graphs, programs })
}

fn labels(u: &Ui) -> Vec<String> {
    let text = u.app.ui.palette.clone().unwrap_or_default();
    crate::map_palette::view(&u.app, &text).items().map(|item| item.label.clone()).collect()
}

#[test]
fn cmd_p_goes_to_a_page_ranked_by_name_then_path() {
    crate::with_map_mode(|| {
        let (mut u, p) = setup();
        press(&mut u, Key::P, cmd());
        assert_eq!(crate::map_palette::scope(&u.app), Some(&Scope::Pages));
        let view = crate::map_palette::view(&u.app, "");
        let titles: Vec<_> = view.sections.iter().map(|s| s.title.clone().unwrap_or_default()).collect();
        assert_eq!(titles, ["Recently opened", "All pages"]);
        assert_eq!(view.count, "11 pages");
        assert_eq!(view.hints.split("   ").next(), Some("↑↓ choose"));
        // Every row has its parent path under it; a root has none.
        let gradients = view.items().find(|item| item.label == "Gradients").unwrap();
        assert_eq!(gradients.sub.as_deref(), Some("Overview › Canvas and layers › Paint › Fills"));
        assert_eq!(view.items().find(|item| item.label == "Checkout").unwrap().sub, None);

        type_text(&mut u, "gra");
        assert_eq!(labels(&u), ["Gradients", "Graphs and charts", "Programs", "Search the map for “gra”"]);
        let view = crate::map_palette::view(&u.app, "gra");
        assert_eq!(view.count, "3 of 11");
        assert_eq!(view.items().nth(1).unwrap().sub.as_deref(), Some("Checkout › Reports"));
        // A path answers after the names: "reports" finds the page and the one under it.
        let hits: Vec<_> = crate::map_palette::pages(&u.app, "reports")[0].1.iter().map(|hit| hit.name.clone()).collect();
        assert_eq!(hits, ["Reports", "Graphs and charts"]);
        // Letters in order match too.
        let hits: Vec<_> = crate::map_palette::pages(&u.app, "grdnts")[0].1.iter().map(|hit| hit.id).collect();
        assert_eq!(hits, [p.gradients]);

        // ↵ opens the highlighted page in the active tab.
        press(&mut u, Key::Enter, Modifiers::NONE);
        assert!(u.app.ui.palette.is_none());
        assert_eq!(u.app.session.page().unwrap(), p.gradients);
        assert_eq!(crate::map_tabs::open(&u.app), vec![p.gradients], "the active tab shows it instead");

        // ⌘↵ opens it in a tab of its own.
        press(&mut u, Key::P, cmd());
        type_text(&mut u, "programs");
        press(&mut u, Key::Enter, cmd());
        assert_eq!(u.app.session.page().unwrap(), p.programs);
        assert_eq!(crate::map_tabs::open(&u.app), vec![p.gradients, p.programs]);

        // Recently opened leads the list, the latest first.
        u.frame(vec![]);
        press(&mut u, Key::P, cmd());
        let view = crate::map_palette::view(&u.app, "");
        let recent: Vec<_> = view.sections[0].items.iter().map(|item| item.label.clone()).collect();
        assert_eq!(recent, ["Programs", "Gradients", "Overview"]);
        assert!(view.sections[1].items.iter().all(|item| !recent.contains(&item.label)), "each page once");
        press(&mut u, Key::Escape, Modifiers::NONE);
        assert!(u.app.ui.palette.is_none());
        let _ = (p.overview, p.graphs);
    });
}

#[test]
fn greater_than_lists_the_actions_and_backspace_goes_back() {
    crate::with_map_mode(|| {
        let (mut u, _) = setup();
        press(&mut u, Key::P, cmd());
        type_text(&mut u, ">");
        assert!(crate::map_palette::listing_actions(&u.app));
        let view = crate::map_palette::view(&u.app, ">");
        assert_eq!(view.sections[0].title.as_deref(), Some("Open in this box"));
        let boxed: Vec<_> = view.sections[0].items.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(boxed, ["Go to page…", "Switch workspace…", "Switch worktree…", "Settings…"]);
        assert_eq!(view.sections[0].items[2].right, Right::Keys(vec!["›".into()]));
        assert!(view.hints.contains("⌫ back to pages"));
        // Toggles are switches.
        let pane = view.items().find(|item| item.label == "Workspace pane").unwrap();
        assert!(matches!(pane.right, Right::Toggle(_)));

        // Deleting the > goes back to the pages.
        press(&mut u, Key::Backspace, Modifiers::NONE);
        assert_eq!(u.app.ui.palette.as_deref(), Some(""));
        assert!(!crate::map_palette::listing_actions(&u.app));
        // ⌫ in the empty page search does nothing more.
        press(&mut u, Key::Backspace, Modifiers::NONE);
        assert_eq!(u.app.ui.palette.as_deref(), Some(""));

        // ⇧⌘P opens the same box with > typed; Settings… opens its list in the box.
        press(&mut u, Key::Escape, Modifiers::NONE);
        press(&mut u, Key::P, cmd() | Modifiers::SHIFT);
        assert_eq!(u.app.ui.palette.as_deref(), Some(">"));
        type_text(&mut u, "settings");
        press(&mut u, Key::Enter, Modifiers::NONE);
        assert_eq!(crate::map_palette::scope(&u.app), Some(&Scope::Settings));
        // ⌫ in its empty field goes back to the actions.
        press(&mut u, Key::Backspace, Modifiers::NONE);
        assert_eq!(crate::map_palette::scope(&u.app), Some(&Scope::Pages));
        assert_eq!(u.app.ui.palette.as_deref(), Some(">"));
    });
}

fn ws(id: &str, label: &str, repo: &str, branch: &str, map: &str, worktree: bool) -> crate::workspaces::Workspace {
    crate::workspaces::Workspace {
        id: id.into(),
        label: label.into(),
        repo: Some(repo.into()),
        path: Some(format!("/root/code/{id}")),
        branch: Some(branch.into()),
        map: map.into(),
        worktree,
        ..Default::default()
    }
}

fn with_workspaces(u: &mut Ui) {
    u.app.ws.rows = vec![
        ws("w1", "afterburn", "/root/code/mindmap", "main", "map", false),
        ws("w2", "checkout", "/root/code/mindmap", "feature/checkout", "map", true),
        ws("w3", "accounting", "/root/code/accounting", "main", "map", false),
        ws("w4", "code", "/root/code/code", "main", "map", false),
        ws("w5", "sync-pane", "/root/code/code", "feature/sync-pane", "map", true),
        ws("w6", "login", "/root/code/code", "fix/login", "none", true),
        ws("w7", "nuts-ios", "/root/code/nuts-ios", "main", "none", false),
    ];
    u.app.active_workspace = Some("w1".into());
    u.frame(vec![]);
}

#[test]
fn ctrl_cmd_o_lists_workspaces_and_one_with_worktrees_asks_which() {
    crate::with_map_mode(|| {
        let (mut u, _) = setup();
        with_workspaces(&mut u);
        press(&mut u, Key::O, ctrl_cmd());
        assert_eq!(crate::map_palette::scope(&u.app), Some(&Scope::Workspace));
        // Without a node the frame forgets which workspace is open: name it again.
        u.app.active_workspace = Some("w1".into());
        let view = crate::map_palette::view(&u.app, "");
        assert_eq!(view.chip.as_deref(), Some("Workspace"));
        assert_eq!(view.placeholder, "Switch workspace…");
        assert_eq!(view.sections[0].title.as_deref(), Some("THIS COMPUTER"));
        let rows: Vec<_> = view.items().map(|item| (item.label.as_str(), item.right.clone())).collect();
        let tag = |text: &str, check: bool, more: bool| Right::Tag { text: text.into(), ink: crate::map_palette::Ink::Teal, check, more };
        assert_eq!(rows[0], ("accounting", tag("MAP", false, false)));
        assert_eq!(rows[1], ("code", tag("3 WORKTREES", false, true)));
        assert_eq!(rows[2], ("afterburn", tag("OPEN", true, false)));
        assert_eq!(rows[3], ("nuts-ios", tag("INIT", false, false)));
        assert_eq!(rows[4].0, "New workspace…");
        assert_eq!(rows[5].0, "Add machine…");
        assert_eq!(view.items().nth(2).unwrap().sub.as_deref(), Some("main"), "the open workspace names its worktree");
        assert_eq!(view.count, "4 workspaces");

        // A workspace with several worktrees asks which one first: a search finds it by a
        // worktree's branch.
        type_text(&mut u, "fix/login");
        assert_eq!(labels(&u)[0], "code");
        press(&mut u, Key::Enter, Modifiers::NONE);
        let Some(Scope::Worktree { name, current, .. }) = crate::map_palette::scope(&u.app).cloned() else { panic!("worktree step") };
        assert_eq!((name.as_str(), current), ("code", false));
        let view = crate::map_palette::view(&u.app, "");
        assert_eq!(view.chip.as_deref(), Some("code › Worktree"));
        assert_eq!(view.placeholder, "Branch or folder…");
        let rows: Vec<_> = view.items().map(|item| (item.label.as_str(), item.sub.as_deref())).collect();
        assert_eq!(rows, [("main", Some("/root/code/w4")), ("feature/sync-pane", Some("/root/code/w5")), ("fix/login", Some("/root/code/w6"))]);
        assert!(matches!(&view.items().nth(2).unwrap().right, Right::Tag { text, .. } if text == "NO MAP"));
        assert!(view.hints.contains("new window tab"));
        // ⌫ goes back to the workspaces, with the search as it was.
        press(&mut u, Key::Backspace, Modifiers::NONE);
        assert_eq!(crate::map_palette::scope(&u.app), Some(&Scope::Workspace));
        assert_eq!(u.app.ui.palette.as_deref(), Some("fix/login"));
        // ⌫ in an empty Workspace search goes to the actions.
        for _ in 0.."fix/login".len() {
            press(&mut u, Key::Backspace, Modifiers::NONE);
        }
        assert_eq!(u.app.ui.palette.as_deref(), Some(""));
        press(&mut u, Key::Backspace, Modifiers::NONE);
        assert!(crate::map_palette::listing_actions(&u.app));
    });
}

#[test]
fn switch_worktree_lists_the_open_workspaces_worktrees() {
    crate::with_map_mode(|| {
        let (mut u, _) = setup();
        with_workspaces(&mut u);
        u.app.active_workspace = Some("w1".into());
        crate::map_actions::run(&mut u.app, &u.ctx.clone(), "map.switchWorktree");
        let view = crate::map_palette::view(&u.app, "");
        assert_eq!(view.chip.as_deref(), Some("afterburn › Worktree"));
        let first = view.items().next().unwrap();
        assert_eq!(first.label, "main");
        assert_eq!(first.right, Right::Tag { text: "CURRENT".into(), ink: crate::map_palette::Ink::Teal, check: true, more: false });
        assert!(!view.hints.contains("new window tab"));
        assert_eq!(view.count, "2 worktrees");
        // The sidebar's head opens the workspaces, as ⌃⌘O does.
        crate::switcher::close(&mut u.app);
        assert!(u.app.ui.palette.is_none());
        crate::switcher::toggle(&mut u.app);
        assert_eq!(crate::map_palette::scope(&u.app), Some(&Scope::Workspace));
        crate::switcher::toggle(&mut u.app);
        assert!(u.app.ui.palette.is_none());
    });
}

#[test]
fn cmd_comma_settings_flip_with_enter() {
    crate::with_map_mode(|| {
        let (mut u, _) = setup();
        press(&mut u, Key::Comma, cmd());
        assert_eq!(crate::map_palette::scope(&u.app), Some(&Scope::Settings));
        assert!(!u.app.ui.preferences, "the palette replaces the Preferences window");
        let view = crate::map_palette::view(&u.app, "");
        let titles: Vec<_> = view.sections.iter().map(|s| s.title.clone().unwrap_or_default()).collect();
        assert_eq!(titles, ["MAP", "APP"]);
        assert_eq!(
            labels(&u),
            ["Comments", "Collaborator cursors", "Workspace pane", "Colour theme", "Reduce motion", "Your name in comments", "Language"]
        );
        assert_eq!(view.count, "7 settings");

        let comments = u.app.session.ds().unwrap().view.comments;
        press(&mut u, Key::Enter, Modifiers::NONE);
        assert_eq!(u.app.session.ds().unwrap().view.comments, !comments, "↵ flips the highlighted setting");
        assert_eq!(crate::map_palette::scope(&u.app), Some(&Scope::Settings), "and the box stays open");

        let pane = u.app.ws.pane_open;
        press(&mut u, Key::ArrowDown, Modifiers::NONE);
        press(&mut u, Key::ArrowDown, Modifiers::NONE);
        press(&mut u, Key::Enter, Modifiers::NONE);
        assert_eq!(u.app.ws.pane_open, !pane);

        let motion = u.app.session.prefs.reduce_motion;
        type_text(&mut u, "reduce");
        press(&mut u, Key::Enter, Modifiers::NONE);
        assert_eq!(u.app.session.prefs.reduce_motion, !motion);

        for _ in 0..6 {
            press(&mut u, Key::Backspace, Modifiers::NONE);
        }
        type_text(&mut u, "theme");
        let current = crate::map_palette::current_theme(&u.app).name().to_string();
        assert_eq!(crate::map_palette::view(&u.app, "theme").items().next().unwrap().right, Right::Value { text: current, more: true });
        // Colour theme ›: the themes' list, in the box; ⌫ comes back with the filter.
        press(&mut u, Key::Enter, Modifiers::NONE);
        assert_eq!(crate::map_palette::scope(&u.app), Some(&Scope::Theme));
        press(&mut u, Key::Backspace, Modifiers::NONE);
        assert_eq!(crate::map_palette::scope(&u.app), Some(&Scope::Settings));
        assert_eq!(u.app.ui.palette.as_deref(), Some("theme"));

        // Your name: the field holds it, ↵ saves it and goes back to the settings.
        for _ in 0..5 {
            press(&mut u, Key::Backspace, Modifiers::NONE);
        }
        type_text(&mut u, "your name");
        press(&mut u, Key::Enter, Modifiers::NONE);
        assert_eq!(crate::map_palette::scope(&u.app), Some(&Scope::Name));
        let name = u.app.session.prefs.user_name.clone();
        assert_eq!(u.app.ui.palette.as_deref(), Some(name.as_str()), "the field holds the name");
        for _ in name.chars() {
            press(&mut u, Key::Backspace, Modifiers::NONE);
        }
        type_text(&mut u, "Ada");
        press(&mut u, Key::Enter, Modifiers::NONE);
        assert_eq!(u.app.session.prefs.user_name, "Ada");
        assert_eq!(crate::map_palette::scope(&u.app), Some(&Scope::Settings));
        press(&mut u, Key::Escape, Modifiers::NONE);
        assert!(u.app.ui.palette.is_none());
    });
}

fn agent(name: &str, pane: &str, status: &str) -> crate::compose::TargetInfo {
    crate::compose::TargetInfo {
        target: crate::workspaces::Target::Existing { pane: pane.into(), kind: "claude".into(), status: status.into() },
        label: name.into(),
        title: None,
        status: status.into(),
        focused: false,
        new_agent: false,
    }
}

#[test]
fn the_status_bars_agent_count_opens_the_agents() {
    crate::with_map_mode(|| {
        let (mut u, _) = setup();
        u.app.compose.targets =
            vec![agent("design lead", "w1:p7", "working"), agent("sync", "w1:p11", "blocked"), agent("reviewer", "w1:p9", "idle")];
        u.frame(vec![]);
        // The summary sits at the bar's right end.
        click(&mut u, egui::pos2(1440.0 - 40.0, 900.0 - 12.0));
        // The count opens the Agents modal (the agent picker page's One agents modal) and the
        // click that opened it does not close it; the palette's Agents scope lists the same agents.
        assert!(u.app.agents_modal.open, "the click that opened it does not close it");
        u.app.agents_modal.open = false;
        crate::map_palette::open(&mut u.app, Scope::Agents);
        assert_eq!(crate::map_palette::scope(&u.app), Some(&Scope::Agents));
        let view = crate::map_palette::view(&u.app, "");
        assert_eq!(view.chip.as_deref(), Some("Agents"));
        let rows: Vec<_> = view.items().map(|item| (item.label.as_str(), item.right.clone())).collect();
        let status = |text: &'static str, ink| Right::Status { text, ink };
        use crate::map_palette::Ink;
        assert_eq!(rows[0], ("design lead", status("RUNNING", Ink::Teal)));
        assert_eq!(rows[1], ("sync", status("BLOCKED", Ink::Amber)));
        assert_eq!(rows[2], ("reviewer", status("IDLE", Ink::Faint)));
        assert_eq!(rows[3].0, "New agent…");
        assert_eq!(rows[4].0, "Show in Herdr");
        assert_eq!(view.items().next().unwrap().sub.as_deref(), Some("Claude · pane p7 · working"));
        assert_eq!(view.count, "3 agents");
        // ↵ on an agent makes it the one prompts go to, and closes the box.
        press(&mut u, Key::ArrowDown, Modifiers::NONE);
        press(&mut u, Key::Enter, Modifiers::NONE);
        assert!(u.app.ui.palette.is_none());
        assert_eq!(u.app.compose.target.as_ref(), Some(&u.app.compose.targets[1].target));
    });
}

use crate::map_colors::Theme;

/// The themes' list, opened from the actions by typing "theme" and choosing Colour theme….
fn open_theme_list(u: &mut Ui) {
    press(u, Key::P, cmd() | Modifiers::SHIFT);
    type_text(u, "theme");
    press(u, Key::Enter, Modifiers::NONE);
    assert_eq!(crate::map_palette::scope(&u.app), Some(&Scope::Theme));
}

#[test]
fn typing_theme_lists_the_appearance_actions_then_settings() {
    crate::with_map_mode(|| {
        let (mut u, _) = setup();
        press(&mut u, Key::P, cmd() | Modifiers::SHIFT);
        type_text(&mut u, "theme");
        let view = crate::map_palette::view(&u.app, ">theme");
        assert_eq!(view.sections[0].title.as_deref(), Some("Appearance"));
        assert_eq!(labels(&u), ["Colour theme…", "Next colour theme", "Follow system appearance", "Settings"]);
        let rows: Vec<_> = view.items().map(|item| item.right.clone()).collect();
        assert_eq!(rows[0], Right::Tag { text: "Afterburn".into(), ink: crate::map_palette::Ink::Teal, check: false, more: true });
        assert!(matches!(rows[2], Right::Toggle(_)));
        assert_eq!(rows[3], Right::Value { text: String::new(), more: true });
        // Next colour theme flips and the box stays.
        press(&mut u, Key::ArrowDown, Modifiers::NONE);
        press(&mut u, Key::Enter, Modifiers::NONE);
        assert_eq!(u.app.session.prefs.color_theme, "afterburn");
        assert_eq!(crate::map_palette::current_theme(&u.app), Theme::AfterburnNight);
        assert!(u.app.ui.palette.is_some());
    });
}

#[test]
fn the_theme_list_previews_applies_and_escapes() {
    crate::with_map_mode(|| {
        let (mut u, _) = setup();
        u.app.run("view.themeLight", json!({}));
        u.app.run("view.colorTheme", json!({"theme": "afterburn"}));
        u.frame(vec![]);
        open_theme_list(&mut u);
        let view = crate::map_palette::view(&u.app, "");
        assert_eq!(view.chip.as_deref(), Some("Colour theme"));
        assert_eq!(view.placeholder, "Filter themes…");
        assert_eq!(labels(&u), ["Afterburn", "Afterburn Night", "Classic", "Classic Dark", "Follow the system"]);
        assert_eq!(view.count, "4 themes");
        assert_eq!(view.hints, "↑↓ preview   ↵ apply   ⌫ back   esc keep Afterburn");
        let current: Vec<_> = view.items().map(|item| matches!(item.right, Right::Colour { current: true, .. })).collect();
        assert_eq!(current, [true, false, false, false, false], "the current theme wears the check");
        assert_eq!(view.items().nth(1).unwrap().sub.as_deref(), Some("Dark"));

        // ↓ previews Afterburn Night live; the next frame draws with it.
        press(&mut u, Key::ArrowDown, Modifiers::NONE);
        assert_eq!(u.app.ui.theme_preview, Some(Theme::AfterburnNight));
        u.frame(vec![]);
        assert_eq!(crate::map_colors::theme(), Theme::AfterburnNight);
        assert_eq!(u.app.session.prefs.theme, "light", "a preview changes no preference");

        // Escape keeps the current theme.
        press(&mut u, Key::Escape, Modifiers::NONE);
        assert!(u.app.ui.palette.is_none());
        assert_eq!(u.app.ui.theme_preview, None);
        u.frame(vec![]);
        assert_eq!(crate::map_colors::theme(), Theme::Afterburn);

        // ⌫ goes back to the actions and drops the preview too.
        open_theme_list(&mut u);
        press(&mut u, Key::ArrowDown, Modifiers::NONE);
        assert!(u.app.ui.theme_preview.is_some());
        press(&mut u, Key::Backspace, Modifiers::NONE);
        assert!(crate::map_palette::listing_actions(&u.app));
        assert_eq!(u.app.ui.theme_preview, None);
        press(&mut u, Key::Escape, Modifiers::NONE);

        // ↵ applies Classic Dark: the family and the lightness.
        open_theme_list(&mut u);
        for _ in 0..3 {
            press(&mut u, Key::ArrowDown, Modifiers::NONE);
        }
        assert_eq!(u.app.ui.theme_preview, Some(Theme::ClassicDark));
        press(&mut u, Key::Enter, Modifiers::NONE);
        assert!(u.app.ui.palette.is_none());
        assert_eq!(u.app.ui.theme_preview, None);
        assert_eq!((u.app.session.prefs.color_theme.as_str(), u.app.session.prefs.theme.as_str()), ("classic", "dark"));
        u.frame(vec![]);
        assert_eq!(crate::map_colors::theme(), Theme::ClassicDark);
    });
}

#[test]
fn follow_the_system_keeps_the_family_and_its_lightness_rule() {
    crate::with_map_mode(|| {
        let (mut u, _) = setup();
        u.app.run("view.themeLight", json!({}));
        u.frame(vec![]);
        open_theme_list(&mut u);
        // The toggle row under the themes: on, then back to the lightness shown now.
        for _ in 0..4 {
            press(&mut u, Key::ArrowDown, Modifiers::NONE);
        }
        assert_eq!(u.app.ui.theme_preview, None, "the toggle previews nothing");
        let follow = crate::map_palette::view(&u.app, "").items().last().cloned().unwrap();
        assert_eq!(follow.sub.as_deref(), Some("Afterburn by day · Afterburn Night by night"));
        assert_eq!(follow.right, Right::Toggle(false));
        press(&mut u, Key::Enter, Modifiers::NONE);
        assert_eq!(u.app.session.prefs.theme, "system");
        assert_eq!(crate::map_palette::scope(&u.app), Some(&Scope::Theme), "the box stays open");
        // The test screen has no system theme: it reads as light.
        u.frame(vec![]);
        assert!(!u.app.ui.dark);
        // Following the system, a theme as light as the system now keeps following it.
        press(&mut u, Key::ArrowUp, Modifiers::NONE);
        press(&mut u, Key::ArrowUp, Modifiers::NONE);
        assert_eq!(u.app.ui.theme_preview, Some(Theme::Classic));
        press(&mut u, Key::Enter, Modifiers::NONE);
        assert_eq!((u.app.session.prefs.color_theme.as_str(), u.app.session.prefs.theme.as_str()), ("classic", "system"));
        // A theme of the other lightness turns following off.
        crate::map_palette::apply_theme(&mut u.app, Theme::AfterburnNight);
        assert_eq!((u.app.session.prefs.color_theme.as_str(), u.app.session.prefs.theme.as_str()), ("afterburn", "dark"));
        // Turning it off keeps what the window shows.
        crate::map_palette::follow_system(&mut u.app);
        assert_eq!(u.app.session.prefs.theme, "system");
        u.frame(vec![]);
        crate::map_palette::follow_system(&mut u.app);
        assert_eq!(u.app.session.prefs.theme, "light");
    });
}

#[test]
fn settings_colour_theme_opens_the_same_list() {
    crate::with_map_mode(|| {
        let (mut u, _) = setup();
        press(&mut u, Key::Comma, cmd());
        type_text(&mut u, "colour");
        press(&mut u, Key::Enter, Modifiers::NONE);
        assert_eq!(crate::map_palette::scope(&u.app), Some(&Scope::Theme));
        assert_eq!(u.app.ui.palette_hi, 0, "the current theme (Afterburn) is highlighted");
    });
}
