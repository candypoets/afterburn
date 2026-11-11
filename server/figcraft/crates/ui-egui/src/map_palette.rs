//! Afterburn's palette: one box for pages, actions, workspaces, worktrees, settings and agents
//! (design: the `Palette …` frames on the Afterburn page).
//!
//! ⌘P opens it on the pages: recently opened first, then every page, each with its parent path
//! under it; typing ranks pages by name and path and offers to search the map. Typing `>` (or
//! ⇧⌘P, which opens it with `>` typed) turns it into the Actions palette: every map action, the
//! ones that open another list in this box first, recently used next, then every action by group;
//! deleting the `>` goes back to the pages. The other lists are scopes, named by a chip in the
//! field: Workspace (⌃⌘O, the sidebar's head), a workspace's Worktree, Settings (⌘,) and Agents
//! (the status bar's agent count). ⌫ in an empty field goes back to the list the scope was opened
//! from (the actions when it was opened directly); Escape or a click outside closes the box.
//!
//! Colour theme (design: `Palette Actions · theme` and `Palette Colour theme`): typing "theme" in
//! the actions lists the Appearance actions; Colour theme… (and Settings › Colour theme) opens the
//! themes, each with a map and an IDE thumbnail drawn from its own colours. ↑↓ previews the
//! highlighted theme live (`ui.theme_preview`), ↵ applies it ([`apply_theme`]), ⌫ and Escape keep
//! the current one.
//!
//! The actions come from `map_action_catalog` and run through
//! [`crate::menus::activate_map_action`], the guard every map surface shares; the workspaces and
//! worktrees from [`crate::switcher`]; the agents from the composer's targets.

use egui::{Align2, Color32, CornerRadius, FontId, Id, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_doc::NodeId;
use figcraft_engine::keymap::Platform;
use serde_json::json;

use crate::FigApp;
use crate::agents::AgentState;
use crate::map_action_catalog::{self as catalog, ACTIONS, Action};
use crate::map_colors::Theme;
use crate::switcher::Row;
use crate::theme::{Tokens, regular};
use crate::workspaces::{MapState, Target};

/// The box's width (narrower windows shrink it).
const WIDTH: f32 = 560.0;
/// The field's row, above the first rule.
const SEARCH_H: f32 = 44.0;
/// A one-line row, and a row with a line under its label.
const ROW_H: f32 = 32.0;
const ROW2_H: f32 = 40.0;
/// A colour theme's row, tall enough for its thumbnails.
const THEME_ROW_H: f32 = 56.0;
/// A theme thumbnail.
const THUMB: egui::Vec2 = vec2(56.0, 36.0);
/// The footer under the last rule.
const FOOTER_H: f32 = 29.0;
/// The box's top below the window's title row.
const TOP: f32 = 20.0;
/// The header of the recently used actions.
pub(crate) const RECENTLY_USED: &str = "Recently used";
/// The header of the actions that open another list in this box.
pub(crate) const OPEN_IN_BOX: &str = "Open in this box";
/// The actions that open another list in this box, in the order the Actions list shows them.
const BOX_ACTIONS: [&str; 4] = ["map.goToPage", "map.switchWorkspace", "map.switchWorktree", "map.preferences"];

/// What the box lists. The pages and the actions share one scope: a field starting with `>` lists
/// the actions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    Pages,
    /// Every workspace of every machine.
    Workspace,
    /// The worktrees of the workspace holding `id` on the machine at `address`; `current` when it
    /// is the open workspace (Switch worktree).
    Worktree {
        address: String,
        id: String,
        name: String,
        current: bool,
    },
    Settings,
    /// Settings › Language.
    Language,
    /// Settings › Your name in comments: the field holds the name.
    Name,
    /// The open workspace's agents.
    Agents,
    /// The colour themes.
    Theme,
}

/// The box's state beyond the field's text (`ui.palette`) and the highlighted row
/// (`ui.palette_hi`).
#[derive(Clone, Debug, Default)]
pub struct State {
    pub scope: Scope,
    /// Where ⌫ in an empty field goes back to: the lists this one was opened from, with their text.
    back: Vec<(Scope, String)>,
    /// The text was set for the field (`>`, a name, a search going back): its caret goes to the end.
    caret_to_end: bool,
}

/// The box's scope while it is open.
pub fn scope(app: &FigApp) -> Option<&Scope> {
    app.ui.palette.as_ref().map(|_| &app.ui.palette_box.scope)
}

/// Whether the box lists the actions (its field starts with `>`).
pub fn listing_actions(app: &FigApp) -> bool {
    app.ui.palette_box.scope == Scope::Pages && app.ui.palette.as_deref().is_some_and(|text| text.starts_with('>'))
}

/// The text a scope's field starts with.
fn initial_text(app: &FigApp, scope: &Scope) -> String {
    match scope {
        Scope::Name => app.session.prefs.user_name.clone(),
        _ => String::new(),
    }
}

/// Open the box afresh on `scope`.
pub fn open(app: &mut FigApp, scope: Scope) {
    let text = initial_text(app, &scope);
    open_with(app, scope, text);
}

fn open_with(app: &mut FigApp, scope: Scope, text: String) {
    app.ui.palette = Some(text);
    app.ui.palette_box = State { scope, back: Vec::new(), caret_to_end: true };
    app.ui.palette_hi = 0;
    app.ui.palette_armed = true;
}

/// Open the box on `scope`, or close it when it already shows that scope.
pub fn toggle(app: &mut FigApp, scope: Scope) {
    if self::scope(app) == Some(&scope) { close(app) } else { open(app, scope) }
}

/// ⇧⌘P: open the box on the actions (`>` typed), or close it when it lists them.
pub fn toggle_actions(app: &mut FigApp) {
    if listing_actions(app) { close(app) } else { open_with(app, Scope::Pages, ">".to_string()) }
}

/// ⌘P: open the box on the pages, or close it when it lists them.
pub fn toggle_pages(app: &mut FigApp) {
    if scope(app) == Some(&Scope::Pages) && !listing_actions(app) { close(app) } else { open(app, Scope::Pages) }
}

pub fn close(app: &mut FigApp) {
    app.ui.palette = None;
    app.ui.theme_preview = None;
    app.ui.palette_box = State::default();
}

/// Actions whose shortcut works while the box is open: each switches it to its own list.
pub fn box_action(id: &str) -> bool {
    matches!(id, "map.actions" | "map.goToPage" | "map.switchWorkspace" | "map.preferences")
}

/// Show `scope` in the box, remembering the list it came from for ⌫.
fn push(app: &mut FigApp, scope: Scope) {
    let text = app.ui.palette.take().unwrap_or_default();
    let from = std::mem::take(&mut app.ui.palette_box.scope);
    app.ui.palette_box.back.push((from, text));
    app.ui.palette = Some(initial_text(app, &scope));
    app.ui.palette_box.scope = scope;
    app.ui.palette_box.caret_to_end = true;
    app.ui.palette_hi = 0;
}

/// ⌫ in an empty field: back to the list this scope was opened from, else to the actions.
fn pop(app: &mut FigApp) {
    match app.ui.palette_box.back.pop() {
        Some((scope, text)) => {
            app.ui.palette_box.scope = scope;
            app.ui.palette = Some(text);
        }
        None if app.ui.palette_box.scope != Scope::Pages => {
            app.ui.palette_box.scope = Scope::Pages;
            app.ui.palette = Some(">".to_string());
        }
        None => {}
    }
    app.ui.theme_preview = None;
    app.ui.palette_box.caret_to_end = true;
    app.ui.palette_hi = 0;
}

/// The box's inks: the map's warm palette; dark mode keeps the tokens' surfaces.
struct Pal {
    panel: Color32,
    border: Color32,
    rule: Color32,
    text: Color32,
    muted: Color32,
    faint: Color32,
    teal: Color32,
    amber: Color32,
    danger: Color32,
    hi: Color32,
    hover: Color32,
    cap: Color32,
    off: Color32,
    scrim: Color32,
}

impl Pal {
    fn of(t: &Tokens) -> Pal {
        let theme = crate::map_colors::theme();
        if !matches!(theme, Theme::Classic | Theme::ClassicDark) {
            let c = theme.colors();
            return Pal {
                panel: c.panel,
                border: c.border,
                rule: c.border,
                text: c.ink,
                muted: c.muted,
                faint: c.faint,
                teal: c.map_ink,
                amber: c.working_ink,
                danger: c.danger,
                hi: c.selected,
                hover: c.soft,
                cap: c.soft,
                off: c.border,
                scrim: if theme.dark() { Color32::from_black_alpha(90) } else { with_alpha(c.ink, 0x2E) },
            };
        }
        // Classic and Classic Dark keep the palette's own inks.
        if t.dark {
            Pal {
                panel: t.panel,
                border: t.border,
                rule: t.border,
                text: t.text,
                muted: t.text_secondary,
                faint: t.text_tertiary,
                teal: Color32::from_rgb(0x7F, 0xBD, 0xB5),
                amber: Color32::from_rgb(0xE0, 0xA8, 0x4F),
                danger: Color32::from_rgb(0xE5, 0x8A, 0x78),
                hi: t.hover,
                hover: t.hover,
                cap: t.field,
                off: t.border,
                scrim: Color32::from_black_alpha(90),
            }
        } else {
            Pal {
                panel: Color32::WHITE,
                border: Color32::from_rgb(0xD9, 0xD6, 0xD0),
                rule: Color32::from_rgb(0xE3, 0xE2, 0xDF),
                text: Color32::from_rgb(0x1A, 0x1A, 0x1A),
                muted: Color32::from_rgb(0x62, 0x5D, 0x56),
                faint: Color32::from_rgb(0x9A, 0x95, 0x8D),
                teal: Color32::from_rgb(0x2F, 0x6F, 0x6A),
                amber: Color32::from_rgb(0xA8, 0x6B, 0x12),
                danger: Color32::from_rgb(0x9E, 0x3B, 0x2C),
                hi: Color32::from_rgb(0xEF, 0xED, 0xE8),
                hover: Color32::from_rgb(0xF8, 0xF6, 0xF3),
                cap: Color32::from_rgb(0xF1, 0xF0, 0xED),
                off: Color32::from_rgb(0xD6, 0xD3, 0xCD),
                scrim: Color32::from_rgba_unmultiplied(0x1A, 0x1A, 0x1A, 0x2E),
            }
        }
    }
}

fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

/// The ink a row's tag or status is drawn in, resolved against the palette when drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ink {
    Teal,
    Amber,
    Faint,
    Danger,
}

impl Ink {
    fn color(self, pal: &Pal) -> Color32 {
        match self {
            Ink::Teal => pal.teal,
            Ink::Amber => pal.amber,
            Ink::Faint => pal.faint,
            Ink::Danger => pal.danger,
        }
    }
}

/// What a row shows at its right.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Right {
    None,
    /// Key caps.
    Keys(Vec<String>),
    /// Why the row cannot run now.
    Note(String),
    /// A switch, on or off.
    Toggle(bool),
    /// A state tag (`✓ OPEN`, `MAP`, `3 WORKTREES ›`).
    Tag {
        text: String,
        ink: Ink,
        check: bool,
        more: bool,
    },
    /// An agent's state: a dot and its word.
    Status {
        text: &'static str,
        ink: Ink,
    },
    /// A setting's value (`Thibaut`, `System ›`).
    Value {
        text: String,
        more: bool,
    },
    /// A colour theme's map and IDE thumbnails; the current one wears a check.
    Colour {
        theme: Theme,
        current: bool,
    },
}

/// What choosing a row does.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Pick {
    Page(NodeId),
    SearchMap,
    Action(&'static str),
    Workspace(Row),
    Worktree(Row),
    /// A map action that flips a setting; the box stays open.
    Toggle(&'static str),
    /// A preference command that flips a setting; the box stays open.
    Pref(&'static str),
    /// Open the colour themes in the box.
    Themes,
    /// Apply a colour theme.
    ApplyTheme(Theme),
    /// Flip Follow the system.
    FollowSystem,
    Language(String),
    EditName,
    SaveName,
    Languages,
    Agent(Target),
    NewAgent,
    /// The agent being created: its steps, or why it could not start.
    Creation,
    ShowInHerdr,
    NewWorkspace,
    AddMachine,
    CancelSwitch,
    Retry,
    Stay,
}

/// One row of the box.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Item {
    pub label: String,
    /// The quieter line under the label (a page's path, a worktree's folder, an agent's task).
    pub sub: Option<String>,
    pub sub_ink: Option<Ink>,
    pub right: Right,
    /// `None`: listed but not choosable now.
    pub pick: Option<Pick>,
    /// A little space above it (the rows after a list: Search the map, New workspace…).
    pub gap: bool,
    /// An agent's kind: its mark leads the row, with the status riding on it.
    pub mark: Option<String>,
}

impl Item {
    fn new(label: impl Into<String>, pick: Option<Pick>) -> Item {
        Item { label: label.into(), sub: None, sub_ink: None, right: Right::None, pick, gap: false, mark: None }
    }

    fn sub(mut self, sub: Option<String>) -> Item {
        self.sub = sub.filter(|sub| !sub.trim().is_empty());
        self
    }

    fn right(mut self, right: Right) -> Item {
        self.right = right;
        self
    }

    fn gap(mut self) -> Item {
        self.gap = true;
        self
    }

    fn mark(mut self, kind: &str) -> Item {
        self.mark = Some(kind.to_string());
        self
    }

    fn sub_ink(mut self, ink: Ink) -> Item {
        self.sub_ink = Some(ink);
        self
    }

    fn height(&self) -> f32 {
        if matches!(self.right, Right::Colour { .. }) {
            THEME_ROW_H
        } else if self.sub.is_some() {
            ROW2_H
        } else {
            ROW_H
        }
    }
}

/// A group of rows under its eyebrow; a section without a title is a ranked result.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Section {
    pub title: Option<String>,
    pub items: Vec<Item>,
    /// A rule above it (Follow the system under the themes).
    pub rule: bool,
}

/// Everything the box shows for its scope and text.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct View {
    /// The scope's chip in the field (`Workspace`, `code › Worktree`); none for pages and actions.
    pub chip: Option<String>,
    pub placeholder: String,
    pub sections: Vec<Section>,
    pub hints: String,
    pub count: String,
    /// The highlighted row shows ↵ at its right (pages).
    pub enter_cap: bool,
}

impl View {
    pub(crate) fn items(&self) -> impl Iterator<Item = &Item> {
        self.sections.iter().flat_map(|s| s.items.iter())
    }
}

fn platform() -> Platform {
    crate::shortcuts::platform()
}

/// ⌘ on a Mac, Ctrl elsewhere, as the footer writes it.
fn cmd() -> &'static str {
    if platform() == Platform::Mac { "⌘" } else { "Ctrl+" }
}

/// A row's key cap: its primary shortcut (Help also lists aliases); none for palette-only actions.
fn caps(action: &Action, platform: Platform) -> Vec<String> {
    let keys = action.keys(platform);
    if keys.is_empty() { Vec::new() } else { vec![figcraft_engine::keymap::display(keys, platform)] }
}

fn caps_of(id: &str) -> Vec<String> {
    catalog::find(id).map(|a| caps(a, platform())).unwrap_or_default()
}

// ---- Pages -------------------------------------------------------------------------------------

/// One page as Go to page lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PageHit {
    pub id: NodeId,
    pub name: String,
    /// Its parents, root first (`Overview › Canvas and layers › Paint`); empty for a root.
    pub path: String,
}

/// The header of the recently opened pages, and of the rest.
pub(crate) const RECENTLY_OPENED: &str = "Recently opened";
pub(crate) const ALL_PAGES: &str = "All pages";

/// Every page of the open map in tree order, with its path.
fn all_pages(app: &FigApp) -> Vec<PageHit> {
    let Ok(doc) = app.session.doc() else { return Vec::new() };
    crate::page_tree::rows(doc, &Default::default())
        .into_iter()
        .filter_map(|(id, _)| {
            let name = doc.node(id)?.name.clone();
            let trail = crate::page_tree::path(doc, id);
            let parents: Vec<String> =
                trail.iter().take(trail.len().saturating_sub(1)).filter_map(|p| doc.node(*p).map(|node| node.name.clone())).collect();
            Some(PageHit { id, name, path: parents.join(" › ") })
        })
        .collect()
}

/// Whether the characters of `query` appear in `text` in order.
fn subsequence(text: &str, query: &str) -> bool {
    let mut chars = text.chars();
    query.chars().filter(|c| !c.is_whitespace()).all(|q| chars.any(|c| c == q))
}

/// How well a page matches a lower-cased, trimmed query (lower is better): its name starting with
/// it, a word of its name starting with it, its name containing it, its path containing it, then
/// the query's letters in order in its name, then in its path and name.
pub(crate) fn page_rank(name: &str, path: &str, query: &str) -> Option<u8> {
    let name = name.to_lowercase();
    let path = path.to_lowercase();
    if name.starts_with(query) {
        Some(0)
    } else if name.split(|c: char| !c.is_alphanumeric()).any(|word| word.starts_with(query)) {
        Some(1)
    } else if name.contains(query) {
        Some(2)
    } else if path.contains(query) {
        Some(3)
    } else if subsequence(&name, query) {
        Some(4)
    } else if subsequence(&format!("{path} {name}"), query) {
        Some(5)
    } else {
        None
    }
}

/// Go to page's rows for `query`: recently opened then all pages, or one ranked list.
pub(crate) fn pages(app: &FigApp, query: &str) -> Vec<(Option<&'static str>, Vec<PageHit>)> {
    let all = all_pages(app);
    let query = query.trim().to_lowercase();
    if !query.is_empty() {
        let mut hits: Vec<(u8, usize, PageHit)> =
            all.into_iter().enumerate().filter_map(|(i, hit)| page_rank(&hit.name, &hit.path, &query).map(|r| (r, i, hit))).collect();
        hits.sort_by_key(|(rank, i, _)| (*rank, *i));
        return vec![(None, hits.into_iter().map(|(_, _, hit)| hit).collect())];
    }
    let recent: Vec<PageHit> = crate::map_tabs::recent(app).into_iter().filter_map(|id| all.iter().find(|hit| hit.id == id).cloned()).collect();
    let rest: Vec<PageHit> = all.into_iter().filter(|hit| !recent.iter().any(|r| r.id == hit.id)).collect();
    let mut out = Vec::new();
    if !recent.is_empty() {
        out.push((Some(RECENTLY_OPENED), recent));
    }
    if !rest.is_empty() {
        out.push((Some(ALL_PAGES), rest));
    }
    out
}

fn pages_view(app: &FigApp, query: &str) -> View {
    let total = all_pages(app).len();
    let groups = pages(app, query);
    let matched: usize = groups.iter().map(|(_, hits)| hits.len()).sum();
    let mut sections: Vec<Section> = groups
        .into_iter()
        .map(|(title, hits)| Section {
            rule: false,
            title: title.map(str::to_string),
            items: hits.into_iter().map(|hit| Item::new(hit.name, Some(Pick::Page(hit.id))).sub(Some(hit.path))).collect(),
        })
        .collect();
    if !query.trim().is_empty() {
        let search = catalog::find("map.search").filter(|a| catalog::enabled(app, a));
        let row = Item::new(search_map_label(query), search.map(|_| Pick::SearchMap)).right(Right::Keys(caps_of("map.search"))).gap();
        match sections.first_mut() {
            Some(section) => section.items.push(row),
            None => sections.push(Section { rule: false, title: None, items: vec![row] }),
        }
    }
    let count = if query.trim().is_empty() { format!("{total} page{}", plural(total)) } else { format!("{matched} of {total}") };
    View {
        chip: None,
        placeholder: "Go to page…".to_string(),
        sections,
        hints: format!("↑↓ choose   ↵ open   {}↵ new tab   > actions", cmd()),
        count,
        enter_cap: true,
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

// ---- Actions -----------------------------------------------------------------------------------

/// The Actions list's rows for `query`, by section: a section without a title is the ranked search
/// result. Every catalog action appears at most once; an empty result means nothing matched.
pub(crate) fn sections(app: &FigApp, query: &str) -> Vec<(Option<&'static str>, Vec<&'static Action>)> {
    let platform = platform();
    if !query.trim().is_empty() {
        let hits = catalog::ranked(query, platform);
        return if hits.is_empty() { Vec::new() } else { vec![(None, hits)] };
    }
    let boxed: Vec<&'static Action> = BOX_ACTIONS.iter().filter_map(|id| catalog::find(id)).collect();
    let recent: Vec<&'static Action> = app
        .session
        .prefs
        .recent_actions
        .iter()
        .filter_map(|key| key.strip_prefix("command:"))
        .filter_map(catalog::find)
        .filter(|action| !BOX_ACTIONS.contains(&action.id))
        .take(figcraft_engine::MAX_RECENT_ACTIONS)
        .collect();
    let mut out: Vec<(Option<&'static str>, Vec<&'static Action>)> = vec![(Some(OPEN_IN_BOX), boxed)];
    if !recent.is_empty() {
        out.push((Some(RECENTLY_USED), recent.clone()));
    }
    let lead = out.len();
    for action in ACTIONS.iter().filter(|action| !BOX_ACTIONS.contains(&action.id) && !recent.iter().any(|r| r.id == action.id)) {
        if let Some((_, rows)) = out.iter_mut().skip(lead).find(|(group, _)| *group == Some(action.group)) {
            rows.push(action);
        } else {
            out.push((Some(action.group), vec![action]));
        }
    }
    out
}

/// The footer's right side: every action without a query, else how many matched.
pub(crate) fn count_label(query: &str, matched: usize) -> String {
    if query.trim().is_empty() { format!("{} actions", ACTIONS.len()) } else { format!("{matched} of {}", ACTIONS.len()) }
}

/// The row that searches the map's text for the query.
pub(crate) fn search_map_label(query: &str) -> String {
    format!("Search the map for “{}”", query.trim())
}

pub(crate) fn no_match_label(query: &str) -> String {
    format!("No action named “{}”.   ↵ search the map", query.trim())
}

/// An action as a row: its key caps, its switch when it is a toggle, or why it cannot run.
fn action_item(app: &FigApp, action: &'static Action, boxed: bool) -> Item {
    let reason = catalog::disabled_reason(app, action);
    let label = if boxed || action.id == "map.colorTheme" { format!("{}…", action.label) } else { action.label.to_string() };
    let right = match (reason, catalog::toggle_state(app, action)) {
        (Some(reason), _) => Right::Note(reason.to_string()),
        (None, Some(state)) => Right::Toggle(matches!(state, "Shown" | "On")),
        // Colour theme… names the current theme.
        (None, None) if action.id == "map.colorTheme" => {
            Right::Tag { text: current_theme(app).name().to_string(), ink: Ink::Teal, check: false, more: true }
        }
        (None, None) if action.id == "map.switchWorktree" => Right::Keys(vec!["›".to_string()]),
        (None, None) => Right::Keys(caps(action, platform())),
    };
    Item::new(label, reason.is_none().then_some(Pick::Action(action.id))).right(right)
}

/// The colour theme actions' group.
pub(crate) const APPEARANCE: &str = "Appearance";

/// A search that finds an Appearance action lists the whole group, then Settings (design:
/// `Palette Actions · theme`): "theme" finds Colour theme…, Next colour theme, Follow system
/// appearance and Settings ›. Other hits follow, untitled.
fn appearance_first(groups: Vec<(Option<&'static str>, Vec<&'static Action>)>) -> Vec<(Option<&'static str>, Vec<&'static Action>)> {
    let hits: Vec<&'static Action> = match groups.as_slice() {
        [(None, hits)] if hits.iter().any(|a| a.group == APPEARANCE) => hits.clone(),
        _ => return groups,
    };
    let group: Vec<&'static Action> = ACTIONS.iter().filter(|a| a.group == APPEARANCE).chain(catalog::find("map.preferences")).collect();
    let rest: Vec<&'static Action> = hits.into_iter().filter(|a| !group.iter().any(|g| g.id == a.id)).collect();
    let mut out = vec![(Some(APPEARANCE), group)];
    if !rest.is_empty() {
        out.push((None, rest));
    }
    out
}

fn actions_view(app: &FigApp, query: &str) -> View {
    let groups = appearance_first(sections(app, query));
    let matched: usize = groups.iter().map(|(_, rows)| rows.len()).sum();
    let mut sections: Vec<Section> = groups
        .into_iter()
        .map(|(title, rows)| {
            let boxed = title == Some(OPEN_IN_BOX);
            let appearance = title == Some(APPEARANCE);
            let items = rows
                .into_iter()
                .map(|action| {
                    let item = action_item(app, action, boxed);
                    // Settings under the Appearance actions opens its list: › rather than its keys.
                    if appearance && action.id == "map.preferences" && item.pick.is_some() {
                        item.right(Right::Value { text: String::new(), more: true })
                    } else {
                        item
                    }
                })
                .collect();
            Section { rule: false, title: title.map(|t| t.to_string()), items }
        })
        .collect();
    let mut hints = "↑↓ choose   ↵ run   ⌫ back to pages".to_string();
    if matched == 0 && !query.trim().is_empty() {
        let search = catalog::find("map.search");
        let reason = search.and_then(|a| catalog::disabled_reason(app, a));
        let right = match reason {
            Some(reason) => Right::Note(reason.to_string()),
            None => Right::Keys(caps_of("map.search")),
        };
        sections.push(Section {
            rule: false,
            title: None,
            items: vec![Item::new(search_map_label(query), reason.is_none().then_some(Pick::SearchMap)).right(right)],
        });
        hints = no_match_label(query);
    }
    View { chip: None, placeholder: "Run an action…".to_string(), sections, hints, count: count_label(query, matched), enter_cap: false }
}

// ---- Workspaces and worktrees ------------------------------------------------------------------

/// A switch in flight or failed, in place of the rows.
fn switch_view(app: &FigApp) -> Option<Vec<Section>> {
    if let Some(switch) = &app.status.switching {
        let items = vec![
            Item::new(format!("Opening {}…", switch.branch), None).sub(Some(format!("Still viewing {}.", switch.from))),
            Item::new("Cancel switch", Some(Pick::CancelSwitch)).gap(),
        ];
        return Some(vec![Section { rule: false, title: None, items }]);
    }
    let (switch, reason) = app.status.failed.as_ref()?;
    let mut failed = Item::new(format!("Could not open {}", switch.branch), None).sub(Some(reason.clone()));
    failed.sub_ink = Some(Ink::Danger);
    let items = vec![failed, Item::new("Retry", Some(Pick::Retry)).gap(), Item::new(format!("Stay on {}", switch.from), Some(Pick::Stay))];
    Some(vec![Section { rule: false, title: None, items }])
}

/// A workspace's tag: ✓ OPEN, its worktrees, or its map's state.
fn workspace_tag(row: &Row) -> Right {
    let (text, check, more) = if row.open {
        ("OPEN".to_string(), true, false)
    } else if row.worktrees > 1 {
        (format!("{} WORKTREES", row.worktrees), false, true)
    } else {
        (row.badge().text().to_string(), false, false)
    };
    Right::Tag { text, ink: Ink::Teal, check, more }
}

fn workspace_view(app: &FigApp, query: &str) -> View {
    let machines = crate::switcher::machines(app);
    let groups = crate::switcher::workspace_groups(&machines, app.active_workspace.as_deref(), query);
    let listed: usize = groups.iter().map(|g| g.rows.len()).sum();
    let sections = switch_view(app).unwrap_or_else(|| {
        let mut sections: Vec<Section> = groups
            .into_iter()
            .map(|group| {
                let mut items: Vec<Item> = group
                    .rows
                    .into_iter()
                    .map(|row| {
                        let tag = workspace_tag(&row);
                        Item::new(row.name.clone(), Some(Pick::Workspace(row.clone()))).sub(Some(row.detail())).right(tag)
                    })
                    .collect();
                if items.is_empty() {
                    let note = if app.ws.busy.is_some() {
                        "Listing workspaces…"
                    } else if app.ws.error.is_some() {
                        "Could not reach the node"
                    } else {
                        "No workspaces yet"
                    };
                    items.push(Item::new(note, None));
                }
                Section { rule: false, title: Some(group.title), items }
            })
            .collect();
        if sections.is_empty() {
            sections.push(Section {
                rule: false,
                title: None,
                items: vec![Item::new("No matching workspace", None).sub(Some("Try a name, a branch or a folder.".into()))],
            });
        }
        let tail = vec![
            Item::new("New workspace…", Some(Pick::NewWorkspace)).right(Right::Keys(caps_of("map.newWorkspace"))).gap(),
            Item::new("Add machine…", Some(Pick::AddMachine)),
        ];
        if let Some(last) = sections.last_mut() {
            last.items.extend(tail);
        }
        sections
    });
    View {
        chip: Some("Workspace".to_string()),
        placeholder: "Switch workspace…".to_string(),
        sections,
        hints: format!("↑↓ choose   ↵ new window tab   {}↵ open here   ⌫ back", cmd()),
        count: format!("{listed} workspace{}", plural(listed)),
        enter_cap: false,
    }
}

fn worktree_view(app: &FigApp, query: &str, address: &str, id: &str, name: &str, current: bool) -> View {
    let machines = crate::switcher::machines(app);
    let rows = crate::switcher::worktrees_in(&machines, address, id, app.active_workspace.as_deref(), query);
    let listed = rows.len();
    let sections = switch_view(app).unwrap_or_else(|| {
        let items: Vec<Item> = if rows.is_empty() {
            vec![Item::new(if query.trim().is_empty() { "No worktrees listed" } else { "No matching worktree" }, None)]
        } else {
            rows.into_iter()
                .map(|row| {
                    let tag = if row.open {
                        Right::Tag { text: "CURRENT".into(), ink: Ink::Teal, check: true, more: false }
                    } else if row.map == MapState::None {
                        Right::Tag { text: "NO MAP".into(), ink: Ink::Faint, check: false, more: false }
                    } else {
                        Right::Tag { text: "MAP".into(), ink: Ink::Teal, check: false, more: false }
                    };
                    Item::new(row.name.clone(), Some(Pick::Worktree(row.clone()))).sub(row.path.clone()).right(tag)
                })
                .collect()
        };
        vec![Section { rule: false, title: None, items }]
    });
    let hints = if current {
        "↑↓ choose   ↵ open   ⌫ back".to_string()
    } else {
        format!("↑↓ choose   ↵ new window tab   {}↵ open here   ⌫ back", cmd())
    };
    View {
        chip: Some(format!("{name} › Worktree")),
        placeholder: "Branch or folder…".to_string(),
        sections,
        hints,
        count: format!("{listed} worktree{}", plural(listed)),
        enter_cap: false,
    }
}

// ---- Colour theme ------------------------------------------------------------------------------

/// The theme the window draws with (without a preview): the family lit as the window is now.
pub(crate) fn current_theme(app: &FigApp) -> Theme {
    Theme::of(&app.session.prefs.color_theme, app.ui.dark)
}

/// Whether the theme follows the system's light and dark.
pub(crate) fn follows_system(app: &FigApp) -> bool {
    app.session.prefs.theme.eq_ignore_ascii_case("system")
}

/// Apply `theme`. The family (`view.colorTheme`) always becomes the theme's. Its lightness: with
/// Follow the system on, a theme as light or as dark as the system is now keeps following it (so
/// Classic chosen by day is Classic by day and Classic Dark by night); a theme of the other
/// lightness turns following off and sets its own (`view.themeLight` / `view.themeDark`), since
/// the person asked to see that theme now. With following off, the theme's lightness is set.
pub(crate) fn apply_theme(app: &mut FigApp, theme: Theme) {
    app.ui.theme_preview = None;
    app.run("view.colorTheme", json!({"theme": theme.family()}));
    if follows_system(app) && theme.dark() == app.ui.dark {
        return;
    }
    app.run(if theme.dark() { "view.themeDark" } else { "view.themeLight" }, json!({}));
    app.ui.dark = theme.dark();
}

/// Next colour theme: the one after the current in the picker's order, applied as a choice.
pub(crate) fn next_theme(app: &mut FigApp) {
    let current = current_theme(app);
    let at = Theme::ALL.iter().position(|t| *t == current).unwrap_or(0);
    if let Some(next) = Theme::ALL.get((at + 1) % Theme::ALL.len()) {
        apply_theme(app, *next);
    }
}

/// Follow the system: on, the system lights the family; off, the window keeps the lightness it
/// shows now.
pub(crate) fn follow_system(app: &mut FigApp) {
    if follows_system(app) {
        app.run(if app.ui.dark { "view.themeDark" } else { "view.themeLight" }, json!({}));
    } else {
        app.run("view.themeSystem", json!({}));
    }
}

/// Open the box afresh on the colour themes, the current one highlighted.
pub(crate) fn open_themes(app: &mut FigApp) {
    open(app, Scope::Theme);
    highlight_current_theme(app);
}

fn highlight_current_theme(app: &mut FigApp) {
    let current = current_theme(app);
    app.ui.palette_hi = Theme::ALL.iter().position(|t| *t == current).unwrap_or(0);
}

fn theme_view(app: &FigApp, query: &str) -> View {
    let current = current_theme(app);
    let query = query.trim().to_lowercase();
    let mut items: Vec<Item> = Theme::ALL
        .iter()
        .filter(|t| query.is_empty() || t.name().to_lowercase().contains(&query) || t.note().to_lowercase().contains(&query))
        .map(|t| {
            Item::new(t.name(), Some(Pick::ApplyTheme(*t))).sub(Some(t.note().to_string())).right(Right::Colour { theme: *t, current: *t == current })
        })
        .collect();
    let shown = items.len();
    if items.is_empty() {
        items.push(Item::new("No matching theme", None));
    }
    let (day, night) = (Theme::of(current.family(), false), Theme::of(current.family(), true));
    let follow = Item::new("Follow the system", Some(Pick::FollowSystem))
        .sub(Some(format!("{} by day · {} by night", day.name(), night.name())))
        .right(Right::Toggle(follows_system(app)));
    View {
        chip: Some("Colour theme".to_string()),
        placeholder: "Filter themes…".to_string(),
        sections: vec![Section { rule: false, title: Some("Themes".to_string()), items }, Section { rule: true, title: None, items: vec![follow] }],
        hints: format!("↑↓ preview   ↵ apply   ⌫ back   esc keep {}", current.name()),
        count: format!("{shown} theme{}", plural(shown)),
        enter_cap: false,
    }
}

// ---- Settings ----------------------------------------------------------------------------------

/// The languages the UI can show: System (the OS's), then each by its own name.
fn languages() -> Vec<(&'static str, &'static str)> {
    std::iter::once(("system", "System"))
        .chain(crate::i18n::languages().iter().filter(|l| crate::cjk::can_show(l.code)).map(|l| (l.code, l.name)))
        .collect()
}

fn language_name(app: &FigApp) -> &'static str {
    languages().into_iter().find(|(code, _)| app.session.prefs.language.eq_ignore_ascii_case(code)).map_or("System", |(_, name)| name)
}

fn map_toggle(app: &FigApp, id: &'static str, label: &str) -> Item {
    let action = catalog::find(id);
    let reason = action.and_then(|a| catalog::disabled_reason(app, a));
    let on = action.and_then(|a| catalog::toggle_state(app, a)).is_some_and(|state| matches!(state, "Shown" | "On"));
    let right = match reason {
        Some(reason) => Right::Note(reason.to_string()),
        None => Right::Toggle(on),
    };
    Item::new(label, reason.is_none().then_some(Pick::Toggle(id))).right(right)
}

fn settings_view(app: &FigApp, query: &str) -> View {
    let prefs = &app.session.prefs;
    let name = if prefs.user_name.trim().is_empty() { "You".to_string() } else { prefs.user_name.clone() };
    let map = vec![
        map_toggle(app, "map.comments", "Comments"),
        map_toggle(app, "map.cursors", "Collaborator cursors"),
        map_toggle(app, "map.workspacePane", "Workspace pane"),
    ];
    let general = vec![
        Item::new("Colour theme", Some(Pick::Themes)).right(Right::Value { text: current_theme(app).name().to_string(), more: true }),
        Item::new("Reduce motion", Some(Pick::Pref("pref.reduceMotion"))).right(Right::Toggle(prefs.reduce_motion)),
        Item::new("Your name in comments", Some(Pick::EditName)).right(Right::Value { text: name, more: false }),
        Item::new("Language", Some(Pick::Languages)).right(Right::Value { text: language_name(app).to_string(), more: true }),
    ];
    let query = query.trim().to_lowercase();
    let keep =
        |items: Vec<Item>| -> Vec<Item> { items.into_iter().filter(|item| query.is_empty() || item.label.to_lowercase().contains(&query)).collect() };
    let mut sections = Vec::new();
    for (title, items) in [("MAP", keep(map)), ("APP", keep(general))] {
        if !items.is_empty() {
            sections.push(Section { rule: false, title: Some(title.to_string()), items });
        }
    }
    let shown: usize = sections.iter().map(|s| s.items.len()).sum();
    if sections.is_empty() {
        sections.push(Section { rule: false, title: None, items: vec![Item::new("No matching setting", None)] });
    }
    View {
        chip: Some("Settings".to_string()),
        placeholder: "Filter settings…".to_string(),
        sections,
        hints: "↑↓ choose   ↵ toggle   ⌫ back".to_string(),
        count: format!("{shown} setting{}", plural(shown)),
        enter_cap: false,
    }
}

fn language_view(app: &FigApp, query: &str) -> View {
    let query = query.trim().to_lowercase();
    let items: Vec<Item> = languages()
        .into_iter()
        .filter(|(code, name)| query.is_empty() || name.to_lowercase().contains(&query) || code.contains(query.as_str()))
        .map(|(code, name)| {
            let on = app.session.prefs.language.eq_ignore_ascii_case(code) || (code == "system" && app.session.prefs.language.is_empty());
            let right = if on { Right::Tag { text: "CURRENT".into(), ink: Ink::Teal, check: true, more: false } } else { Right::None };
            Item::new(name, Some(Pick::Language(code.to_string()))).right(right)
        })
        .collect();
    let shown = items.len();
    View {
        chip: Some("Settings › Language".to_string()),
        placeholder: "Filter languages…".to_string(),
        sections: vec![Section { rule: false, title: None, items }],
        hints: "↑↓ choose   ↵ use it   ⌫ back".to_string(),
        count: format!("{shown} language{}", plural(shown)),
        enter_cap: false,
    }
}

fn name_view(text: &str) -> View {
    let name = text.trim();
    let label = if name.is_empty() { "Sign comments as “You”".to_string() } else { format!("Sign comments as “{name}”") };
    View {
        chip: Some("Settings › Your name".to_string()),
        placeholder: "Your name in comments…".to_string(),
        sections: vec![Section { rule: false, title: None, items: vec![Item::new(label, Some(Pick::SaveName))] }],
        hints: "↵ save   esc cancel".to_string(),
        count: String::new(),
        enter_cap: true,
    }
}

// ---- Agents ------------------------------------------------------------------------------------

/// An agent's state as its row says it: RUNNING, BLOCKED, IDLE.
fn agent_status(status: &str) -> (&'static str, Ink) {
    match AgentState::of(status) {
        AgentState::Working => ("RUNNING", Ink::Teal),
        AgentState::Blocked => ("BLOCKED", Ink::Amber),
        AgentState::Idle => ("IDLE", Ink::Faint),
        AgentState::Unknown => ("UNKNOWN", Ink::Faint),
    }
}

/// One agent's line under its name: `Opus 5.5 · pane p2 · Review the composer`. The kind leads
/// only when it has no mark; the model follows when the node (or the app, for an agent it
/// created) knows it; a blocked agent says what it waits for.
pub(crate) fn agent_line(info: &crate::compose::TargetInfo, task: Option<&str>, new: bool) -> String {
    let (kind, pane) = match &info.target {
        Target::Existing { pane, kind, .. } => (kind.as_str(), pane.as_str()),
        Target::New { kind } => (kind.as_str(), ""),
    };
    let mut line = Vec::new();
    if !crate::agent_menu::has_mark(kind) {
        line.push(crate::agents::kind_label(kind));
    }
    if let Some(model) = crate::agent_menu::model_of(pane) {
        line.push(model);
    }
    let short = crate::agents::short_pane(pane);
    if !short.is_empty() {
        line.push(format!("pane {short}"));
    }
    let task = task.map(str::trim).filter(|task| !task.is_empty());
    match (AgentState::of(&info.status), task) {
        (AgentState::Blocked, Some(task)) => line.push(format!("Waiting: {task}")),
        (_, Some(task)) => line.push(task.to_string()),
        (_, None) => {}
    }
    if new {
        line.push("new".to_string());
    }
    line.join(" · ")
}

fn agents_view(app: &FigApp, query: &str) -> View {
    let agents: Vec<crate::compose::TargetInfo> = app.compose.targets.iter().filter(|info| !info.new_agent).cloned().collect();
    let tasks = crate::agents::agent_tasks(app, &agents);
    let new_pane = crate::agents::new_pane(app);
    let query = query.trim().to_lowercase();
    let mut items: Vec<Item> = agents
        .iter()
        .zip(tasks)
        .filter_map(|(info, task)| {
            let (kind, pane) = match &info.target {
                Target::Existing { pane, kind, .. } => (kind.as_str(), pane.as_str()),
                Target::New { kind } => (kind.as_str(), ""),
            };
            let name = crate::agents::row_name(Some(info.label.as_str()), pane);
            let line = agent_line(info, task.as_deref(), new_pane == Some(pane));
            // The kind is searchable even when only its mark shows.
            let hit = query.is_empty() || name.to_lowercase().contains(&query) || format!("{kind} {line}").to_lowercase().contains(&query);
            let (text, ink) = agent_status(&info.status);
            // An agent of unknown state is listed, but no prompt can go to it.
            let pick = AgentState::of(&info.status).sendable().then(|| Pick::Agent(info.target.clone()));
            hit.then(|| Item::new(name, pick).sub(Some(line)).right(Right::Status { text, ink }).mark(kind))
        })
        .collect();
    let shown = items.len();
    // The agent being created here (or that could not start) keeps a row of its own.
    if let Some((name, line, failed)) = crate::agents::pending(app) {
        let (text, ink) = if failed { ("FAILED", Ink::Danger) } else { ("STARTING", Ink::Amber) };
        items.push(Item::new(name, Some(Pick::Creation)).sub(Some(line)).sub_ink(Ink::Amber).right(Right::Status { text, ink }));
    }
    if shown == 0 {
        let note = match app.compose.targets_error.as_deref() {
            Some(error) => error.to_string(),
            None if !query.is_empty() => "No matching agent".to_string(),
            None => "No agents in this workspace".to_string(),
        };
        items.push(Item::new(note, None));
    }
    items.push(Item::new("Show in Herdr", Some(Pick::ShowInHerdr)).gap());
    items.push(Item::new("New agent…", Some(Pick::NewAgent)).right(Right::Keys(caps_of("map.newAgent"))));
    let workspace = crate::switcher::current_name(app);
    let title = if workspace == "this workspace" { "THIS WORKSPACE".to_string() } else { format!("THIS WORKSPACE · {}", workspace.to_uppercase()) };
    View {
        chip: Some("Agents".to_string()),
        placeholder: "Filter agents…".to_string(),
        sections: vec![Section { rule: false, title: Some(title), items }],
        hints: "↑↓ choose   ↵ send the prompt to it   ⌫ back".to_string(),
        count: format!("{shown} agent{}", plural(shown)),
        enter_cap: false,
    }
}

/// What the box shows for its scope and `text`.
pub(crate) fn view(app: &FigApp, text: &str) -> View {
    match &app.ui.palette_box.scope {
        Scope::Pages => match text.strip_prefix('>') {
            Some(query) => actions_view(app, query),
            None => pages_view(app, text),
        },
        Scope::Workspace => workspace_view(app, text),
        Scope::Worktree { address, id, name, current } => worktree_view(app, text, address, id, name, *current),
        Scope::Settings => settings_view(app, text),
        Scope::Language => language_view(app, text),
        Scope::Name => name_view(text),
        Scope::Agents => agents_view(app, text),
        Scope::Theme => theme_view(app, text),
    }
}

// ---- Choosing ----------------------------------------------------------------------------------

/// Run the search row: close the box and open map search with the query typed. Declines (the box
/// stays open) when map search cannot run.
pub(crate) fn search_map(app: &mut FigApp, ctx: &egui::Context, query: &str) -> bool {
    let Some(action) = catalog::find("map.search") else { return false };
    if !catalog::enabled(app, action) {
        return false;
    }
    close(app);
    if let Some(focus) = ctx.memory(|m| m.focused()) {
        ctx.memory_mut(|m| m.surrender_focus(focus));
    }
    let opened = crate::map_navigation::open_search(app, ctx, query);
    ctx.request_repaint();
    opened
}

/// Bring the open workspace to the front in Herdr.
fn show_in_herdr(app: &FigApp) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(id) = app.active_workspace.as_deref() {
        crate::workspaces::open_in_herdr(&crate::workspaces::service_addr(), id);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = app;
}

/// Do what choosing `pick` asks; `new_tab` when ⌘↵ chose it (for a workspace or worktree, which
/// opens in a new window tab by default, ⌘↵ switches this window in place).
pub(crate) fn choose(app: &mut FigApp, ctx: &egui::Context, pick: Pick, text: &str, new_tab: bool) {
    match pick {
        Pick::Page(page) => {
            close(app);
            if new_tab { crate::map_tabs::open_new_tab(app, page) } else { crate::map_tabs::open_here(app, page) }
        }
        Pick::SearchMap => {
            search_map(app, ctx, text.strip_prefix('>').unwrap_or(text));
        }
        // The Appearance actions stay in the box: Colour theme… opens its list here (⌫ comes
        // back), the others flip and show their new state.
        Pick::Action("map.colorTheme") => {
            push(app, Scope::Theme);
            highlight_current_theme(app);
        }
        Pick::Action(id @ ("map.nextColorTheme" | "map.followSystemAppearance")) => {
            crate::map_actions::run(app, ctx, id);
        }
        Pick::Action(id) => {
            crate::menus::activate_map_action(app, ctx, id);
        }
        Pick::Workspace(row) => {
            if row.worktrees > 1 {
                push(app, Scope::Worktree { address: row.address.clone(), id: row.id.clone(), name: row.name.clone(), current: row.open });
            } else if new_tab {
                crate::switcher::switch_here(app, ctx, &row);
            } else {
                crate::switcher::choose(app, ctx, &row);
            }
        }
        Pick::Worktree(row) => {
            if new_tab {
                crate::switcher::switch_here(app, ctx, &row)
            } else {
                crate::switcher::choose(app, ctx, &row)
            }
        }
        Pick::Toggle(id) => {
            crate::map_actions::run(app, ctx, id);
        }
        Pick::Pref(command) => {
            app.run(command, json!({}));
        }
        Pick::Themes => {
            push(app, Scope::Theme);
            highlight_current_theme(app);
        }
        Pick::ApplyTheme(theme) => {
            apply_theme(app, theme);
            close(app);
        }
        Pick::FollowSystem => follow_system(app),
        Pick::Language(code) => {
            app.run("pref.language", json!({"language": code}));
            pop(app);
        }
        Pick::EditName => push(app, Scope::Name),
        Pick::SaveName => {
            app.run("pref.userName", json!({"name": text.trim()}));
            pop(app);
        }
        Pick::Languages => push(app, Scope::Language),
        Pick::Agent(target) => {
            // ↵ makes it the agent prompts go to; ⌘↵ also shows its pane in Herdr.
            app.compose.choose(target);
            if new_tab {
                show_in_herdr(app);
            }
            close(app);
        }
        Pick::NewAgent => {
            close(app);
            crate::agents::new_agent(app);
        }
        Pick::Creation => {
            close(app);
            crate::agents::show_creation(app);
        }
        Pick::ShowInHerdr => {
            show_in_herdr(app);
            close(app);
        }
        Pick::NewWorkspace => {
            close(app);
            crate::map_shell::new_workspace_form(app);
        }
        Pick::AddMachine => {
            close(app);
            crate::native_menu::run(app, crate::native_menu::ADD_MACHINE);
        }
        Pick::CancelSwitch => crate::switcher::cancel(app),
        Pick::Retry => crate::switcher::retry(app, ctx),
        Pick::Stay => crate::switcher::close(app),
    }
    ctx.request_repaint();
}

// ---- Drawing -----------------------------------------------------------------------------------

/// Paint `text` as a key cap ending at `right`, vertically centred on `cy`; returns its left edge.
fn keycap(ui: &Ui, pal: &Pal, text: &str, right: f32, cy: f32) -> f32 {
    let font = FontId::monospace(9.5);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, pal.muted);
    let w = (galley.size().x + 10.0).max(22.0);
    let r = Rect::from_min_size(pos2(right - w, cy - 9.0), vec2(w, 18.0));
    ui.painter().rect_filled(r, CornerRadius::same(4), pal.cap);
    ui.painter().galley(pos2(r.center().x - galley.size().x / 2.0, cy - galley.size().y / 2.0), galley, pal.muted);
    r.min.x
}

/// A switch, 30 × 18, ending at `right`.
fn switch(ui: &Ui, pal: &Pal, on: bool, right: f32, cy: f32) -> f32 {
    let r = Rect::from_min_size(pos2(right - 30.0, cy - 9.0), vec2(30.0, 18.0));
    ui.painter().rect_filled(r, CornerRadius::same(9), if on { pal.teal } else { pal.off });
    let knob = if on { r.max.x - 9.0 } else { r.min.x + 9.0 };
    ui.painter().circle_filled(pos2(knob, cy), 7.0, Color32::WHITE);
    r.min.x
}

/// The thumbnails' columns in a row ending at `right`: the map's, then the IDE's.
fn thumb_columns(right: f32) -> (f32, f32) {
    let ide = right - 12.0 - 72.0 - 10.0 - THUMB.x;
    (ide - 8.0 - THUMB.x, ide)
}

/// A map in miniature, in `theme`'s colours: the sidebar with its rows, the tab strip with the
/// open tab's accent, and cards on the paper (one outlined in the accent).
fn map_thumb(ui: &Ui, rect: Rect, theme: Theme) {
    let c = theme.colors();
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(3), c.canvas);
    let side = Rect::from_min_max(rect.min, pos2(rect.min.x + 13.0, rect.max.y));
    p.rect_filled(side, CornerRadius { nw: 3, sw: 3, ne: 0, se: 0 }, c.panel);
    for (i, ink) in [c.map_ink, c.faint, c.faint, c.faint].into_iter().enumerate() {
        let y = side.min.y + 7.0 + i as f32 * 3.5;
        let w = if i == 0 { 8.0 } else { 6.0 };
        p.line_segment([pos2(side.min.x + 2.5, y), pos2(side.min.x + 2.5 + w, y)], Stroke::new(1.2, ink));
    }
    let tabs = Rect::from_min_max(pos2(side.max.x, rect.min.y), pos2(rect.max.x, rect.min.y + 6.0));
    p.rect_filled(tabs, CornerRadius { ne: 3, ..CornerRadius::ZERO }, c.tab_row);
    let tab = Rect::from_min_size(pos2(tabs.min.x + 3.0, tabs.min.y), vec2(12.0, tabs.height()));
    p.rect_filled(tab, CornerRadius::ZERO, c.panel);
    p.line_segment([pos2(tab.min.x, tab.min.y + 1.0), pos2(tab.max.x, tab.min.y + 1.0)], Stroke::new(1.0, c.accent));
    let card = |r: Rect, stroke: Color32| {
        p.rect_filled(r, CornerRadius::same(1), c.panel);
        p.rect_stroke(r, CornerRadius::same(1), Stroke::new(0.7, stroke), StrokeKind::Inside);
    };
    card(Rect::from_min_size(pos2(side.max.x + 5.0, tabs.max.y + 4.0), vec2(14.0, 9.0)), c.border);
    card(Rect::from_min_size(pos2(side.max.x + 23.0, tabs.max.y + 4.0), vec2(15.0, 9.0)), c.border);
    card(Rect::from_min_size(pos2(side.max.x + 5.0, tabs.max.y + 17.0), vec2(14.0, 8.0)), c.accent);
    p.rect_stroke(rect, CornerRadius::same(3), Stroke::new(1.0, c.border), StrokeKind::Inside);
}

/// The design editor in miniature, in `theme`'s tokens: the toolbar, the layers panel, the canvas
/// with an artboard, and the properties panel.
fn ide_thumb(ui: &Ui, rect: Rect, theme: Theme) {
    let t = theme.tokens();
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(3), t.canvas);
    let bar = Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + 5.0));
    p.rect_filled(bar, CornerRadius { nw: 3, ne: 3, ..CornerRadius::ZERO }, t.panel);
    p.line_segment([pos2(bar.center().x - 5.0, bar.min.y + 1.5), pos2(bar.center().x + 5.0, bar.min.y + 1.5)], Stroke::new(1.2, t.accent));
    let left = Rect::from_min_max(pos2(rect.min.x, bar.max.y), pos2(rect.min.x + 10.0, rect.max.y));
    let right = Rect::from_min_max(pos2(rect.max.x - 10.0, bar.max.y), rect.max);
    for panel in [left, right] {
        p.rect_filled(panel, CornerRadius::ZERO, t.panel);
        for i in 0..4 {
            let y = panel.min.y + 4.0 + i as f32 * 3.5;
            p.line_segment([pos2(panel.min.x + 2.0, y), pos2(panel.max.x - 3.0, y)], Stroke::new(1.0, t.text_tertiary));
        }
    }
    let board = Rect::from_min_max(pos2(left.max.x + 6.0, bar.max.y + 7.0), pos2(right.min.x - 6.0, rect.max.y - 7.0));
    p.rect_filled(board, CornerRadius::ZERO, t.panel);
    p.rect_stroke(board, CornerRadius::ZERO, Stroke::new(1.0, t.accent), StrokeKind::Inside);
    p.rect_stroke(rect, CornerRadius::same(3), Stroke::new(1.0, t.border), StrokeKind::Inside);
}

/// One row; returns its response.
fn item_ui(ui: &mut Ui, pal: &Pal, item: &Item, highlighted: bool, enter_cap: bool) -> Response {
    if item.gap {
        ui.add_space(4.0);
    }
    let w = ui.available_width();
    let enabled = item.pick.is_some();
    let (r, resp) = ui.allocate_exact_size(vec2(w, item.height()), if enabled { Sense::click() } else { Sense::hover() });
    crate::a11y::button(&resp, &item.label);
    let pill = r.shrink2(vec2(6.0, 0.0));
    if highlighted && enabled {
        ui.painter().rect_filled(pill, CornerRadius::same(6), pal.hi);
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(pill, CornerRadius::same(6), pal.hover);
    }
    let cy = pill.center().y;
    let mut left_of = pill.max.x;
    match &item.right {
        Right::None if highlighted && enabled && enter_cap => left_of = keycap(ui, pal, "↵", pill.max.x - 12.0, cy),
        Right::None => {}
        Right::Keys(keys) => {
            let mut right = pill.max.x - 12.0;
            for (i, k) in keys.iter().enumerate().rev() {
                right = keycap(ui, pal, k, right, cy) - if i > 0 { 4.0 } else { 0.0 };
            }
            left_of = right;
        }
        Right::Note(note) => {
            let galley = ui.painter().layout_no_wrap(note.clone(), regular(10.5), pal.faint);
            left_of = pill.max.x - 12.0 - galley.size().x;
            ui.painter().galley(pos2(left_of, cy - galley.size().y / 2.0), galley, pal.faint);
        }
        Right::Toggle(on) => left_of = switch(ui, pal, *on, pill.max.x - 12.0, cy),
        Right::Tag { text, ink, check, more } => {
            let color = ink.color(pal);
            let mut right = pill.max.x - 16.0;
            if *more {
                let g = ui.painter().layout_no_wrap("›".to_string(), regular(11.0), color);
                right -= g.size().x;
                ui.painter().galley(pos2(right, cy - g.size().y / 2.0), g, color);
                right -= 6.0;
            }
            let g = ui.painter().layout_no_wrap(text.clone(), FontId::monospace(9.0), color);
            right -= g.size().x;
            ui.painter().galley(pos2(right, cy - g.size().y / 2.0), g, color);
            if *check {
                right -= 10.0;
                crate::workspaces::check_mark(ui, pos2(right, cy), color);
                right -= 4.0;
            }
            left_of = right;
        }
        Right::Status { text, ink } => {
            let color = ink.color(pal);
            let g = ui.painter().layout_no_wrap(text.to_string(), FontId::monospace(9.5), color);
            let x = pill.max.x - 20.0 - g.size().x;
            ui.painter().galley(pos2(x, cy - g.size().y / 2.0), g, color);
            ui.painter().circle_filled(pos2(x - 12.0, cy), 4.0, color);
            left_of = x - 16.0;
        }
        Right::Value { text, more } => {
            let shown = if *more { format!("{text} ›") } else { text.clone() };
            let g = ui.painter().layout_no_wrap(shown, regular(12.0), pal.muted);
            left_of = pill.max.x - 16.0 - g.size().x;
            ui.painter().galley(pos2(left_of, cy - g.size().y / 2.0), g, pal.muted);
        }
        Right::Colour { theme, current } => {
            let (map_x, ide_x) = thumb_columns(pill.max.x);
            map_thumb(ui, Rect::from_min_size(pos2(map_x, cy - THUMB.y / 2.0), THUMB), *theme);
            ide_thumb(ui, Rect::from_min_size(pos2(ide_x, cy - THUMB.y / 2.0), THUMB), *theme);
            let zone = Rect::from_min_max(pos2(ide_x + THUMB.x + 10.0, pill.min.y), pos2(pill.max.x - 12.0, pill.max.y));
            if *current {
                crate::workspaces::check_mark(ui, pos2(zone.center().x, cy), pal.teal);
            } else if highlighted {
                ui.painter().text(pos2(zone.center().x, cy), Align2::CENTER_CENTER, "↵ apply", FontId::monospace(9.5), pal.teal);
            }
            left_of = map_x;
        }
    }
    let mut x = pill.min.x + 12.0;
    // An agent leads with its kind's mark; its status rides on it. A kind without a mark keeps
    // its name in the line below.
    if let Some(kind) = &item.mark {
        let at = pos2(x + 9.0, cy);
        if crate::agent_menu::paint_mark(ui.painter(), at, 18.0, kind) {
            if let Right::Status { ink, .. } = &item.right {
                crate::agent_menu::paint_status_on_mark(ui.painter(), at, 18.0, ink.color(pal));
            }
            x += 28.0;
        }
    }
    let room = (left_of - 12.0 - x).max(20.0);
    let ink = if enabled { pal.text } else { pal.faint };
    let label = crate::widgets::elide_font(ui, &item.label, room, regular(13.0));
    if let (Right::Colour { .. }, Some(sub)) = (&item.right, &item.sub) {
        // A theme: its name over its note, centred on the thumbnails.
        ui.painter().text(pos2(x, cy - 2.0), Align2::LEFT_BOTTOM, label, regular(14.0), ink);
        let sub = crate::widgets::elide_font(ui, sub, room, FontId::monospace(9.5));
        ui.painter().text(pos2(x, cy + 4.0), Align2::LEFT_TOP, sub, FontId::monospace(9.5), pal.muted);
        return resp;
    }
    match &item.sub {
        Some(sub) => {
            ui.painter().text(pos2(x, pill.min.y + 5.0), Align2::LEFT_TOP, label, regular(13.0), ink);
            let sub_ink = item.sub_ink.map_or(pal.faint, |i| i.color(pal));
            let sub = crate::widgets::elide_font(ui, sub, room, regular(11.0));
            ui.painter().text(pos2(x, pill.min.y + 22.0), Align2::LEFT_TOP, sub, regular(11.0), sub_ink);
        }
        None => {
            ui.painter().text(pos2(x, cy), Align2::LEFT_CENTER, label, regular(13.0), ink);
        }
    }
    resp
}

/// The field's prompt: a small chevron centred on `c` (drawn: the face's › sits on the baseline).
fn chevron(ui: &Ui, c: Pos2, color: Color32) {
    let stroke = Stroke::new(1.3, color);
    ui.painter().line_segment([pos2(c.x - 2.0, c.y - 4.0), pos2(c.x + 2.0, c.y)], stroke);
    ui.painter().line_segment([pos2(c.x + 2.0, c.y), pos2(c.x - 2.0, c.y + 4.0)], stroke);
}

/// The ⌫ key, drawn (the monospace face has no glyph for it), from `x` on the line `cy`; returns
/// its width.
fn erase_key(ui: &Ui, x: f32, cy: f32, color: Color32) -> f32 {
    let stroke = Stroke::new(1.0, color);
    let pts = [pos2(x, cy), pos2(x + 3.5, cy - 3.5), pos2(x + 12.0, cy - 3.5), pos2(x + 12.0, cy + 3.5), pos2(x + 3.5, cy + 3.5), pos2(x, cy)];
    for pair in pts.windows(2) {
        if let [a, b] = pair {
            ui.painter().line_segment([*a, *b], stroke);
        }
    }
    let m = pos2(x + 7.5, cy);
    ui.painter().line_segment([m + vec2(-1.6, -1.6), m + vec2(1.6, 1.6)], stroke);
    ui.painter().line_segment([m + vec2(-1.6, 1.6), m + vec2(1.6, -1.6)], stroke);
    12.0
}

/// The footer's hints from `at`, stopping short of `right`; each ⌫ is drawn.
fn hints_ui(ui: &Ui, at: Pos2, hints: &str, right: f32, font: FontId, color: Color32) {
    let mut x = at.x;
    for (i, part) in hints.split('⌫').enumerate() {
        if i > 0 {
            if x + 12.0 > right {
                return;
            }
            x += erase_key(ui, x, at.y, color) + 2.0;
        }
        let shown = crate::widgets::elide_font(ui, part, (right - x).max(0.0), font.clone());
        let galley = ui.painter().layout_no_wrap(shown, font.clone(), color);
        let w = galley.size().x;
        ui.painter().galley(pos2(x, at.y - galley.size().y / 2.0), galley, color);
        x += w;
    }
}

fn rule(ui: &mut Ui, pal: &Pal) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::ZERO, pal.rule);
}

/// What a click or a key chose this frame.
struct Chosen {
    pick: Pick,
    new_tab: bool,
}

/// Draw the box and handle its keys and clicks. Escape and a click outside close it.
pub(crate) fn show(app: &mut FigApp, ctx: &egui::Context) {
    let Some(mut text) = app.ui.palette.clone() else {
        app.ui.theme_preview = None;
        return;
    };
    let pal = Pal::of(&Tokens::get(ctx));
    let (step, new_tab, enter, escape) = ctx.input_mut(|i| {
        let step = i.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) as i64
            - i.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) as i64;
        let new_tab = i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter);
        (step, new_tab, i.consume_key(egui::Modifiers::NONE, egui::Key::Enter), i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    });
    // ⌫ in an empty field steps back a list.
    if text.is_empty()
        && (app.ui.palette_box.scope != Scope::Pages || !app.ui.palette_box.back.is_empty())
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace))
    {
        pop(app);
        text = app.ui.palette.clone().unwrap_or_default();
    }
    let pressed = ctx.input(|i| i.pointer.any_pressed());
    let armed = std::mem::replace(&mut app.ui.palette_armed, false);
    let screen = ctx.content_rect();
    let header = if app.ui.home || app.session.ds().is_ok_and(|ds| ds.view.ui_hidden) {
        0.0
    } else if app.ui.mac_inset() {
        crate::MAC_BAND
    } else {
        44.0
    };
    let width = WIDTH.min(screen.width() - 32.0).max(240.0);
    let search_id = Id::new("map_action_search");
    let scope_before = app.ui.palette_box.scope.clone();
    if std::mem::take(&mut app.ui.palette_box.caret_to_end) {
        let mut state = egui::text_edit::TextEditState::load(ctx, search_id).unwrap_or_default();
        let end = egui::text::CCursor::new(text.chars().count());
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
        state.store(ctx, search_id);
    }
    let mut chosen: Option<Chosen> = None;
    let mut preview: Option<Theme> = None;
    let mut changed = false;
    let mut view = view(app, &text);
    let area = egui::Area::new(Id::new("palette"))
        .anchor(Align2::CENTER_TOP, vec2(0.0, header + TOP))
        .order(egui::Order::Foreground)
        .constrain(true)
        .show(ctx, |ui| {
            // Painted first on the box's own layer, so it dims the window and never the box.
            ui.ctx().layer_painter(ui.layer_id()).rect_filled(screen, CornerRadius::ZERO, pal.scrim);
            egui::Frame::NONE
                .fill(pal.panel)
                .stroke(Stroke::new(1.0, pal.border))
                .corner_radius(CornerRadius::same(10))
                .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(22) })
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    // The field: the prompt or the scope's chip, then the text.
                    let bar = Rect::from_min_size(ui.cursor().min, vec2(width, SEARCH_H));
                    let field_left = match &view.chip {
                        Some(chip) => {
                            let g = ui.painter().layout_no_wrap(chip.clone(), regular(12.5), pal.text);
                            let r = Rect::from_min_size(pos2(bar.min.x + 12.0, bar.min.y + 10.0), vec2(g.size().x + 20.0, 24.0));
                            ui.painter().rect_filled(r, CornerRadius::same(6), pal.hi);
                            ui.painter().galley(pos2(r.min.x + 10.0, r.center().y - g.size().y / 2.0), g, pal.text);
                            r.max.x + 10.0
                        }
                        None => {
                            chevron(ui, pos2(bar.min.x + 19.5, bar.center().y), pal.faint);
                            bar.min.x + 34.0
                        }
                    };
                    let field = Rect::from_min_max(pos2(field_left, bar.center().y - 10.0), pos2(bar.max.x - 16.0, bar.center().y + 10.0));
                    let response = ui.put(
                        field,
                        egui::TextEdit::singleline(&mut text)
                            .id(search_id)
                            .frame(egui::Frame::NONE)
                            .margin(egui::Margin::ZERO)
                            .font(regular(14.0))
                            .text_color(pal.text)
                            .hint_text(egui::RichText::new(view.placeholder.clone()).font(regular(14.0)).color(pal.faint))
                            .desired_width(field.width()),
                    );
                    response.request_focus();
                    // The field is placed inside the row; the row, not the field, sets where the list starts.
                    ui.advance_cursor_after_rect(bar);
                    if response.changed() {
                        changed = true;
                        app.ui.palette = Some(text.clone());
                        view = self::view(app, &text);
                    }
                    let flat: Vec<&Item> = view.items().collect();
                    if changed || scope_before != app.ui.palette_box.scope {
                        app.ui.palette_hi = flat.iter().position(|item| item.pick.is_some()).unwrap_or(0);
                    }
                    let count = flat.len();
                    let hi = (app.ui.palette_hi as i64 + step).clamp(0, count.saturating_sub(1) as i64) as usize;
                    app.ui.palette_hi = hi;
                    if (enter || new_tab)
                        && let Some(pick) = flat.get(hi).and_then(|item| item.pick.clone())
                    {
                        chosen = Some(Chosen { pick, new_tab });
                    }
                    // ↑↓ in the themes previews the highlighted one live.
                    if app.ui.palette_box.scope == Scope::Theme {
                        preview = match flat.get(hi).map(|item| &item.right) {
                            Some(Right::Colour { theme, .. }) => Some(*theme),
                            _ => None,
                        };
                    }
                    rule(ui, &pal);
                    let max_h = (screen.height() - header - TOP - SEARCH_H - FOOTER_H - 48.0).clamp(ROW_H * 3.0, 420.0);
                    egui::ScrollArea::vertical().id_salt("palette_rows").max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        let mut index = 0;
                        for (n, section) in view.sections.iter().enumerate() {
                            if section.rule {
                                ui.add_space(8.0);
                                rule(ui, &pal);
                            }
                            match &section.title {
                                Some(title) => {
                                    ui.add_space(if n == 0 { 12.0 } else { 8.0 });
                                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 12.0), Sense::hover());
                                    let eyebrow = |x: f32, text: &str| {
                                        ui.painter().text(pos2(x, r.center().y), Align2::LEFT_CENTER, text, FontId::monospace(9.0), pal.faint)
                                    };
                                    eyebrow(r.min.x + 16.0, &title.to_uppercase());
                                    // The themes name their thumbnails' columns.
                                    if section.items.iter().any(|item| matches!(item.right, Right::Colour { .. })) {
                                        let (map_x, ide_x) = thumb_columns(r.max.x - 6.0);
                                        eyebrow(map_x, "MAP");
                                        eyebrow(ide_x, "IDE");
                                    }
                                    ui.add_space(6.0);
                                }
                                None => ui.add_space(8.0),
                            }
                            for item in &section.items {
                                let resp = item_ui(ui, &pal, item, index == hi, view.enter_cap);
                                if let Some(pick) = item.pick.clone().filter(|_| resp.clicked()) {
                                    chosen = Some(Chosen { pick, new_tab: ui.input(|i| i.modifiers.command) });
                                }
                                if step != 0 && index == hi {
                                    resp.scroll_to_me(None);
                                }
                                index += 1;
                            }
                        }
                        ui.add_space(8.0);
                    });
                    rule(ui, &pal);
                    let (foot, _) = ui.allocate_exact_size(vec2(width, FOOTER_H), Sense::hover());
                    let font = FontId::monospace(9.5);
                    let count_w = ui.painter().layout_no_wrap(view.count.clone(), font.clone(), pal.faint).size().x;
                    hints_ui(ui, pos2(foot.min.x + 16.0, foot.center().y), &view.hints, foot.max.x - count_w - 28.0, font.clone(), pal.faint);
                    ui.painter().text(pos2(foot.max.x - 20.0, foot.center().y), Align2::RIGHT_CENTER, &view.count, font, pal.faint);
                });
        });
    let on_toggle = app.ui.actions_rect.is_some_and(|rect| ctx.input(|i| i.pointer.hover_pos().is_some_and(|p| rect.contains(p))));
    let outside = pressed && !armed && !on_toggle && !area.response.contains_pointer();
    if escape || outside {
        close(app);
        ctx.memory_mut(|m| m.surrender_focus(search_id));
        return;
    }
    if app.ui.palette.is_some() {
        app.ui.palette = Some(text.clone());
    }
    if app.ui.theme_preview != preview {
        app.ui.theme_preview = preview;
        ctx.request_repaint();
    }
    if let Some(Chosen { pick, new_tab }) = chosen {
        choose(app, ctx, pick, &text, new_tab);
    }
}
