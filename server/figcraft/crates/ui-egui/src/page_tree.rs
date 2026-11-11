//! Page/subpage hierarchy stored as document metadata.
//!
//! A page with `afterburn.parent_page` is a subpage. A subpage created from a card also stores
//! `afterburn.source_node`; if that card disappears, the subpage and its descendants are removed.
//! The tree is docked in the map's sidebar under the workspace switcher (design: `Sidebar pages ·
//! window`): every page, indented by level, the open one marked; a click opens a page. ⌘P (the
//! `map.pageTree` action) reveals the sidebar. Pages are renamed here: a double-click on a row (or
//! Rename page in its menu) turns its name into a field; Enter or a click elsewhere renames the
//! page, Esc keeps the old name.

use std::collections::HashSet;

use egui::{Align2, Color32, CornerRadius, FontId, Id, Rect, Sense, Stroke, Ui, pos2, vec2};
use figcraft_doc::{Document, NodeId};

use crate::FigApp;
use crate::theme::{Tokens, regular};
use crate::workspaces::Skin;

pub const PARENT_KEY: &str = "afterburn.parent_page";
/// Pages nest at most this deep (the root counts as the first level).
pub const MAX_LEVELS: usize = 5;
pub const SOURCE_KEY: &str = "afterburn.source_node";
/// A page's mode: `exploration` for a new idea not built yet; absent (or anything else) for a page
/// that describes the code as it is, the state of the art.
pub const MODE_KEY: &str = "afterburn.page_mode";
/// The [`MODE_KEY`] value of an exploration page.
pub const EXPLORATION: &str = "exploration";

/// Mark `page` an exploration, or the state of the art (the mode is removed).
pub fn set_mode(app: &mut FigApp, page: NodeId, exploration: bool) {
    let mode = if exploration { serde_json::Value::from(EXPLORATION) } else { serde_json::Value::Null };
    // Page data, like the tree's own links: allowed in map mode, where drawing is not.
    let params = serde_json::json!({"ids": [page.0], "pluginData": {MODE_KEY: mode}});
    if let Err(error) = app.session.execute("node.set", &params) {
        app.toast(format!("The page's mode could not be set: {error}"));
    }
}

/// Watch `page`, just made with a first prompt: when its agent marks it an exploration, a toast
/// says so.
pub fn watch_new(app: &mut FigApp, page: NodeId) {
    app.page_tree.new_pages.retain(|(id, _)| *id != page);
    app.page_tree.new_pages.push((page, None));
}

/// Say which watched new pages their agent marked an exploration; forget them, and any watched
/// too long or gone.
fn note_new_explorations(app: &mut FigApp, now: f64) {
    if app.page_tree.new_pages.is_empty() {
        return;
    }
    let Ok(doc) = app.session.doc() else { return };
    let mut marked = Vec::new();
    app.page_tree.new_pages.retain_mut(|(page, since)| {
        let since = *since.get_or_insert(now);
        if doc.node(*page).is_none() || now - since > NEW_PAGE_WATCH {
            return false;
        }
        if is_exploration(doc, *page) {
            marked.push(page_name(doc, *page));
            return false;
        }
        true
    });
    if let Some(name) = marked.pop() {
        let ink = Mode::Exploration.ink(true);
        app.toast_with(
            format!("New page · {name} · the agent marked it Exploration"),
            crate::ToastMark { icon: crate::icons::Icon::PageExplore, ink, action: None },
        );
    }
}

/// After a Sync of `scope` (see `sync::State::building`) finished: its exploration pages are
/// built, so they are the state of the art now. Says which.
pub fn built(app: &mut FigApp, scope: Option<u64>) {
    let Ok(doc) = app.session.doc() else { return };
    let pages: Vec<NodeId> = building_pages(doc, Some(scope)).into_iter().filter(|page| is_exploration(doc, *page)).collect();
    let mut names: Vec<String> = pages.iter().map(|page| page_name(doc, *page)).collect();
    if pages.is_empty() {
        return;
    }
    for page in &pages {
        set_mode(app, *page, false);
    }
    names.sort();
    let msg = match names.as_slice() {
        [one] => format!("Built · {one} is now the state of the art"),
        many => format!("Built · {} pages are now the state of the art", many.len()),
    };
    let ink = Mode::Current.ink(true);
    app.toast_with(msg, crate::ToastMark { icon: crate::icons::Icon::PageCurrent, ink, action: Some(crate::ToastAction::OpenSync) });
}

/// What a page's icon in the tree says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The page describes the code as it is.
    Current,
    /// A new idea, not built yet.
    Exploration,
}

impl Mode {
    fn icon(self) -> crate::icons::Icon {
        match self {
            Mode::Current => crate::icons::Icon::PageCurrent,
            Mode::Exploration => crate::icons::Icon::PageExplore,
        }
    }

    /// The icon's ink: blue for the state of the art, violet for an exploration (lighter on the
    /// dark skin and on toasts).
    fn ink(self, light: bool) -> Color32 {
        match (self, light) {
            (Mode::Current, false) => Color32::from_rgb(0x41, 0x6F, 0x91),
            (Mode::Current, true) => Color32::from_rgb(0x8F, 0xB4, 0xD1),
            (Mode::Exploration, false) => Color32::from_rgb(0x6B, 0x4F, 0xA0),
            (Mode::Exploration, true) => Color32::from_rgb(0xB3, 0x9D, 0xDB),
        }
    }
}

/// What is happening to a page now, at the right of its row; its mode icon stays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Activity {
    /// A Sync covers it: a CD burning.
    Burning,
    /// An agent (`Claude`) is designing it: a brush in a turning arc.
    Designing(String),
}

impl Activity {
    fn tip(&self, page: &str) -> String {
        match self {
            Activity::Burning => format!("Burning {page} into code"),
            Activity::Designing(agent) => format!("{agent} is designing {page}"),
        }
    }
}

/// `page`'s activities, right to left: burning while a Sync covers it, and designing while
/// `designing` names the agent at work on it. Both show at once; neither hides the other.
pub fn activity(page: NodeId, building: &HashSet<NodeId>, designing: Option<String>) -> Vec<Activity> {
    building.contains(&page).then_some(Activity::Burning).into_iter().chain(designing.map(Activity::Designing)).collect()
}

/// Whether `page` is an exploration (see [`MODE_KEY`]).
pub fn is_exploration(doc: &Document, page: NodeId) -> bool {
    doc.node(page).and_then(|n| n.plugin_data.get(MODE_KEY)).is_some_and(|mode| mode.trim() == EXPLORATION)
}

/// The pages a running Sync covers (see `sync::State::building`): `scope` and its subpages, or
/// every page.
pub fn building_pages(doc: &Document, scope: Option<Option<u64>>) -> HashSet<NodeId> {
    match scope {
        None => HashSet::new(),
        Some(None) => doc.pages.iter().copied().collect(),
        Some(Some(page)) => descendants(doc, NodeId(page)).into_iter().chain([NodeId(page)]).collect(),
    }
}

/// `page`'s mode: what the page says it is.
pub fn mode(doc: &Document, page: NodeId) -> Mode {
    if is_exploration(doc, page) { Mode::Exploration } else { Mode::Current }
}

/// One row of the docked tree, and its pitch.
const ROW_H: f32 = 30.0;
/// Each level sits this much further in.
const INDENT: f32 = 16.0;
/// The mode icon before each name.
const ICON: f32 = 14.0;
/// An activity mark at the right of a busy page's row, and the pitch of each: × and the name
/// move in by one pitch per mark.
const ACTIVITY: f32 = 18.0;
const ACTIVITY_GAP: f32 = 24.0;

/// The tree's state plus the orphan-cleanup clock.
#[derive(Default)]
pub struct State {
    /// Asked to show the tree (⌘P): the sidebar opens on the next frame.
    pub open: bool,
    pub just_opened: bool,
    /// Pages whose children are folded away in the docked tree.
    pub collapsed: HashSet<NodeId>,
    last_cleanup: f64,
    /// The page whose name is being edited, and the text so far.
    pub(crate) renaming: Option<(NodeId, String)>,
    /// The rename field still has to take focus and select its whole text.
    select_name: bool,
    /// The rows as last drawn: each page and its rect.
    pub(crate) drawn: Vec<(NodeId, Rect)>,
    /// Pages just made with a first prompt, and when: the agent marks an exploration, and the
    /// tree says so once.
    pub(crate) new_pages: Vec<(NodeId, Option<f64>)>,
}

/// How long a new page is watched for the agent marking it an exploration (seconds).
const NEW_PAGE_WATCH: f64 = 30.0 * 60.0;

/// The rename field's selected text: a soft accent behind ink (Classic's own, the theme's
/// accent tint otherwise).
fn selection() -> Color32 {
    match crate::map_colors::theme() {
        crate::map_colors::Theme::Classic => Color32::from_rgb(0xF3, 0xDC, 0xD5),
        _ => crate::map_colors::get().accent_tint,
    }
}

/// The open row's fill: Classic's warm grey as designed, the dark skin's selection on Classic
/// Dark, the theme's selected teal otherwise.
fn current_fill(s: &Skin) -> Color32 {
    match crate::map_colors::theme() {
        crate::map_colors::Theme::Classic => Color32::from_rgb(0xEF, 0xEE, 0xEA),
        crate::map_colors::Theme::ClassicDark => s.selected,
        _ => crate::map_colors::get().selected,
    }
}

/// Start editing `page`'s name in its row.
pub fn start_rename(app: &mut FigApp, page: NodeId) {
    let Some(name) = app.session.doc().ok().and_then(|doc| doc.node(page).map(|node| node.name.clone())) else { return };
    app.page_tree.renaming = Some((page, name));
    app.page_tree.select_name = true;
}

/// Rename `page` to `text` (trimmed); an empty or unchanged name keeps the old one.
fn finish_rename(app: &mut FigApp, page: NodeId, text: &str) {
    let name = text.trim();
    let unchanged = app.session.doc().ok().and_then(|doc| doc.node(page).map(|node| node.name == name)).unwrap_or(true);
    if !name.is_empty() && !unchanged {
        app.run("page.rename", serde_json::json!({"id": page.0, "name": name}));
    }
}

/// How a rename ended: the name to rename to, or `None` when Esc cancelled.
type Renamed = Option<String>;

/// The name field in a row being renamed, over `field`. Returns how editing ended once Enter, a
/// click elsewhere or Esc ends it, `None` while it goes on.
fn rename_field(ui: &mut Ui, s: &Skin, field: Rect, page: NodeId, text: &mut String, select_name: &mut bool) -> Option<Renamed> {
    let id = Id::new(("page_tree_rename", page.0));
    // Esc is the map's: it ends the rename before the tree draws (`map_actions`). This catches
    // an Esc that reached the field anyway.
    let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let edit = egui::TextEdit::singleline(text)
        .id(id)
        .frame(egui::Frame::NONE)
        .margin(egui::Margin::symmetric(2, 0))
        .font(regular(13.0))
        .text_color(s.text)
        .vertical_align(egui::Align::Center)
        .desired_width(field.width());
    let resp = ui
        .scope(|ui| {
            let visuals = ui.visuals_mut();
            visuals.selection.bg_fill = selection();
            visuals.selection.stroke = Stroke::new(1.0, s.text);
            visuals.text_cursor.stroke = Stroke::new(1.0, s.text);
            ui.put(field, edit)
        })
        .inner;
    crate::a11y::text(&resp, "Page name", text);
    if *select_name {
        resp.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
            let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()));
            state.cursor.set_char_range(Some(all));
            state.store(ui.ctx(), id);
            *select_name = false;
        }
        ui.ctx().request_repaint();
        return None;
    }
    if escaped && (resp.has_focus() || resp.lost_focus()) {
        return Some(None);
    }
    if resp.lost_focus() {
        return Some(Some(text.clone()));
    }
    None
}

fn meta_id(doc: &Document, page: NodeId, key: &str) -> Option<NodeId> {
    doc.node(page)?.plugin_data.get(key)?.parse::<u64>().ok().and_then(|id| doc.node(NodeId(id)).map(|_| NodeId(id)))
}

/// The parent page, when the page is a subpage still attached to a live parent.
pub fn parent(doc: &Document, page: NodeId) -> Option<NodeId> {
    meta_id(doc, page, PARENT_KEY)
}

/// The component that owns a subpage.
pub fn source(doc: &Document, page: NodeId) -> Option<NodeId> {
    meta_id(doc, page, SOURCE_KEY)
}

/// Root pages, in document order.
pub fn roots(doc: &Document) -> Vec<NodeId> {
    doc.pages.iter().copied().filter(|page| parent(doc, *page).is_none()).collect()
}

/// Direct children of `page`, in document order.
pub fn children(doc: &Document, page: NodeId) -> Vec<NodeId> {
    doc.pages.iter().copied().filter(|child| parent(doc, *child) == Some(page)).collect()
}

/// The root of a page, following parent metadata.
pub fn root(doc: &Document, page: NodeId) -> NodeId {
    let mut current = page;
    let mut seen = Vec::new();
    while let Some(parent) = parent(doc, current) {
        if seen.contains(&parent) || parent == current {
            break;
        }
        seen.push(current);
        current = parent;
    }
    current
}

/// Root-to-page path, for the breadcrumb.
pub fn path(doc: &Document, page: NodeId) -> Vec<NodeId> {
    let mut out = vec![page];
    let mut current = page;
    while let Some(parent) = parent(doc, current) {
        if out.contains(&parent) {
            break;
        }
        out.push(parent);
        current = parent;
    }
    out.reverse();
    out
}

/// Depth from the root; a root is 0.
pub fn depth(doc: &Document, page: NodeId) -> usize {
    path(doc, page).len().saturating_sub(1)
}

/// All descendants of `page` (not including it), deepest first.
/// Every page below `page`, deepest first.
pub(crate) fn descendants(doc: &Document, page: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![page];
    let mut seen = Vec::new();
    while let Some(current) = stack.pop() {
        if seen.contains(&current) {
            continue;
        }
        seen.push(current);
        for child in children(doc, current) {
            out.push(child);
            stack.push(child);
        }
    }
    out.sort_by_key(|child| std::cmp::Reverse(depth(doc, *child)));
    out
}

fn page_name(doc: &Document, page: NodeId) -> String {
    doc.node(page).map(|node| node.name.clone()).unwrap_or_else(|| "Untitled".to_string())
}

/// Store the hierarchy metadata on a new subpage.
pub fn set_subpage(app: &mut FigApp, page: u64, parent_page: u64, source_node: u64) -> Result<(), String> {
    let params = serde_json::json!({
        "id": page,
        "pluginData": {
            PARENT_KEY: parent_page.to_string(),
            SOURCE_KEY: source_node.to_string(),
        },
    });
    app.session.execute("node.set", &params).map(|_| ()).map_err(|error| error.to_string())
}

/// Delete `page`, its descendants and any card links pointing at them. Deepest first.
pub fn delete_tree(app: &mut FigApp, page: NodeId) -> Result<usize, String> {
    let (pages, links) = {
        let doc = app.session.doc().map_err(|error| error.to_string())?;
        let mut pages = descendants(doc, page);
        pages.push(page);
        let ids: Vec<u64> = pages.iter().map(|id| id.0).collect();
        let mut links = Vec::new();
        for node in doc.nodes() {
            if node.plugin_data.get(crate::new_page::DETAIL_PAGE_KEY).and_then(|value| value.parse::<u64>().ok()).is_some_and(|id| ids.contains(&id))
            {
                links.push(node.id);
            }
        }
        (pages, links)
    };
    for node in links {
        let _ = app.session.execute("node.set", &serde_json::json!({"id": node, "pluginData": {crate::new_page::DETAIL_PAGE_KEY: null}}));
    }
    let mut deleted = 0;
    for page in pages {
        match app.session.execute("page.delete", &serde_json::json!({"id": page})) {
            Ok(_) => deleted += 1,
            Err(error) => return Err(format!("could not delete {page}: {error}")),
        }
    }
    Ok(deleted)
}

/// Remove subpages whose source card or parent page disappeared. Runs at most every two seconds.
pub fn maintain(app: &mut FigApp, ctx: &egui::Context) {
    if app.page_tree.last_cleanup > 0.0 && ctx.input(|input| input.time) - app.page_tree.last_cleanup < 2.0 {
        return;
    }
    app.page_tree.last_cleanup = ctx.input(|input| input.time);
    note_new_explorations(app, ctx.input(|input| input.time));
    let orphans: Vec<NodeId> = match app.session.doc() {
        Ok(doc) => doc
            .pages
            .iter()
            .copied()
            .filter(|page| {
                let has_parent = doc.node(*page).is_some_and(|node| node.plugin_data.contains_key(PARENT_KEY));
                if !has_parent {
                    return false;
                }
                let parent_gone = parent(doc, *page).is_none();
                let source_gone = doc
                    .node(*page)
                    .and_then(|node| node.plugin_data.get(SOURCE_KEY))
                    .and_then(|value| value.parse::<u64>().ok())
                    .is_some_and(|id| doc.node(NodeId(id)).is_none());
                parent_gone || source_gone
            })
            .collect(),
        Err(_) => return,
    };
    for page in orphans {
        let name = app.session.doc().ok().map(|doc| page_name(doc, page)).unwrap_or_default();
        if delete_tree(app, page).is_ok() {
            app.toast(format!("Removed subpage {name}; its component is gone"));
        }
    }
}

/// Page tree (`map.pageTree`): the tree lives in the sidebar, so asking for it shows the sidebar.
pub fn show(app: &mut FigApp, _ctx: &egui::Context) {
    if std::mem::take(&mut app.page_tree.open) {
        app.page_tree.just_opened = false;
        app.ws.pane_open = true;
    }
}

/// The docked tree's rows, top to bottom: each root, then its pages depth first in document
/// order, with their level; a folded page's descendants are left out.
pub fn rows(doc: &Document, collapsed: &HashSet<NodeId>) -> Vec<(NodeId, usize)> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut stack: Vec<(NodeId, usize)> = roots(doc).into_iter().rev().map(|page| (page, 0)).collect();
    while let Some((page, level)) = stack.pop() {
        if !seen.insert(page) {
            continue;
        }
        out.push((page, level));
        if !collapsed.contains(&page) {
            stack.extend(children(doc, page).into_iter().rev().map(|child| (child, level + 1)));
        }
    }
    out
}

/// One docked row as drawn.
struct Line {
    page: NodeId,
    level: usize,
    name: String,
    kids: usize,
    mode: Mode,
    activity: Vec<Activity>,
    folded: bool,
}

/// What a click in the tree asked for.
enum Ask {
    Open(NodeId),
    Fold(NodeId),
    Delete(NodeId),
    Rename(NodeId),
    /// Mark as: an exploration (`true`) or the state of the art.
    Mark(NodeId, bool),
    NewPage,
}

/// The docked tree in `rect`: `PAGES  11`, the rows, and `+ Page` under them.
pub fn docked(app: &mut FigApp, ui: &mut Ui, rect: Rect) {
    let tokens = Tokens::get(ui.ctx());
    let s = Skin::of(&tokens);
    let current_fill = current_fill(&s);
    let (lines, total) = match app.session.doc() {
        Ok(doc) => {
            let building = building_pages(doc, app.sync.building());
            let lines: Vec<Line> = rows(doc, &app.page_tree.collapsed)
                .into_iter()
                .map(|(page, level)| Line {
                    page,
                    level,
                    name: page_name(doc, page),
                    kids: children(doc, page).len(),
                    mode: mode(doc, page),
                    activity: activity(page, &building, app.compose.designing(page)),
                    folded: app.page_tree.collapsed.contains(&page),
                })
                .collect();
            (lines, doc.pages.len())
        }
        Err(_) => return,
    };
    let current = app.session.page().ok();
    // The section's name, and + for a new page at its right.
    let head = Rect::from_min_size(rect.min, vec2(rect.width(), crate::workspaces::SECTION_H));
    crate::workspaces::section(ui, &s, head, "Pages");
    let mut ask = None;
    let plus = Rect::from_center_size(pos2(head.max.x - 10.0 - 13.0, head.center().y), vec2(26.0, 26.0));
    let resp = ui.interact(plus, Id::new("page_tree_new_page"), Sense::click());
    crate::a11y::button(&resp, "New page");
    if resp.hovered() {
        ui.painter().rect_filled(plus, CornerRadius::same(6), s.soft);
    }
    crate::workspaces::plus(ui, plus.center(), if resp.hovered() { s.text } else { s.faint });
    if resp.on_hover_text("New page").clicked() {
        ask = Some(Ask::NewPage);
    }

    let list = Rect::from_min_max(pos2(rect.min.x, head.max.y), rect.max);
    let mut renaming = app.page_tree.renaming.take().filter(|(page, _)| lines.iter().any(|line| line.page == *page));
    let mut select_name = app.page_tree.select_name;
    let mut renamed: Option<(NodeId, Renamed)> = None;
    let mut drawn = Vec::with_capacity(lines.len());
    ui.scope_builder(egui::UiBuilder::new().max_rect(list).id_salt("page_tree_docked"), |ui| {
        ui.set_clip_rect(list.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical().id_salt("page_tree_docked_scroll").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for line in &lines {
                let editing = renaming.as_mut().filter(|(page, _)| *page == line.page).map(|(_, text)| text);
                let (a, rect, end) = row(ui, &s, tokens.dark, line, current == Some(line.page), current_fill, editing, &mut select_name);
                if let Some(a) = a {
                    ask = Some(a);
                }
                if let Some(end) = end {
                    renamed = Some((line.page, end));
                }
                drawn.push((line.page, rect));
            }
            ui.add_space(14.0);
        });
    });

    app.page_tree.drawn = drawn;
    app.page_tree.select_name = select_name;
    app.page_tree.renaming = renaming;
    if let Some((page, end)) = renamed {
        app.page_tree.renaming = None;
        if let Some(text) = end {
            finish_rename(app, page, &text);
        }
        ui.ctx().request_repaint();
    }
    match ask {
        Some(Ask::Open(page)) => {
            app.run("page.select", serde_json::json!({"id": page.0}));
        }
        Some(Ask::Rename(page)) => start_rename(app, page),
        Some(Ask::Fold(page)) => {
            if !app.page_tree.collapsed.remove(&page) {
                app.page_tree.collapsed.insert(page);
            }
        }
        Some(Ask::Delete(page)) => app.page_delete = Some(page),
        Some(Ask::Mark(page, exploration)) => set_mode(app, page, exploration),
        Some(Ask::NewPage) => app.new_page.start(total + 1),
        None => {}
    }
}

/// One page: the fold chevron for a page with children, then its mode icon and name indented by
/// level, and the depth cap at the right on a page at the last level (× to delete while hovered). The open page wears the selected fill. A row
/// being renamed (`editing`: the text so far) shows a field ringed in the accent instead of its
/// name, and says how the rename ended. Returns what was asked, the row's rect and that end.
fn row(
    ui: &mut Ui,
    s: &Skin,
    dark: bool,
    line: &Line,
    is_current: bool,
    current_fill: Color32,
    editing: Option<&mut String>,
    select_name: &mut bool,
) -> (Option<Ask>, Rect, Option<Renamed>) {
    let (outer, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let rect = Rect::from_min_max(pos2(outer.min.x + 8.0, outer.min.y), pos2(outer.max.x - 8.0, outer.max.y));
    crate::a11y::button(&response, &line.name);
    let x = rect.min.x + 22.0 + line.level as f32 * INDENT;
    let fold = (line.kids > 0).then(|| {
        let r = Rect::from_center_size(pos2(x - 11.0, rect.center().y), vec2(16.0, ROW_H));
        ui.interact(r, Id::new(("page_tree_fold", line.page.0)), Sense::click())
    });
    // A busy page keeps its activities at the right; × moves in beside them.
    let busy = !line.activity.is_empty();
    let marks = line.activity.len() as f32 * ACTIVITY_GAP;
    let delete_x = rect.max.x - 14.0 - marks;
    let delete = ui.interact(
        Rect::from_center_size(pos2(delete_x, rect.center().y), vec2(20.0, ROW_H - 6.0)),
        Id::new(("page_tree_delete", line.page.0)),
        Sense::click(),
    );
    let hot = response.hovered() || delete.hovered() || fold.as_ref().is_some_and(|f| f.hovered());
    if is_current {
        ui.painter().rect_filled(rect, CornerRadius::same(8), current_fill);
    } else if hot {
        ui.painter().rect_filled(rect, CornerRadius::same(8), s.soft);
    }
    if let Some(fold) = &fold {
        let ink = if fold.hovered() { s.text } else { s.muted };
        chevron(ui, pos2(x - 11.0, rect.center().y), !line.folded, ink);
    }
    // The mode icon where the name used to start; the name follows it.
    let icon = Rect::from_center_size(pos2(x + 4.0, rect.center().y), vec2(ICON, ICON));
    crate::icons::paint(ui.painter(), icon, line.mode.icon(), line.mode.ink(dark));
    let x = x + ICON;
    let room = (rect.max.x - 34.0 - marks - x).max(30.0);
    let mut end = None;
    let is_editing = editing.is_some();
    if let Some(text) = editing {
        let field = Rect::from_min_max(pos2(x - 4.0, rect.center().y - 11.0), pos2(rect.max.x - 6.0, rect.center().y + 11.0));
        ui.painter().rect_filled(field, CornerRadius::same(5), crate::map_colors::get().panel);
        ui.painter().rect_stroke(field, CornerRadius::same(5), Stroke::new(1.0, s.accent), egui::StrokeKind::Inside);
        end = rename_field(ui, s, field.shrink2(vec2(2.0, 1.0)), line.page, text, select_name);
    } else {
        let name = crate::widgets::elide_font(ui, &line.name, room, regular(13.0));
        ui.painter().text(pos2(x, rect.center().y), Align2::LEFT_CENTER, name, regular(13.0), s.text);
    }
    // The right column: the depth cap on a page at the last level.
    let trail = pos2(rect.max.x - 14.0, rect.center().y);
    for (i, activity) in line.activity.iter().enumerate().filter(|_| !is_editing) {
        let at = Rect::from_center_size(trail - vec2(i as f32 * ACTIVITY_GAP, 0.0), vec2(ACTIVITY, ACTIVITY));
        paint_activity(ui, s, at, activity);
        ui.interact(at, Id::new(("page_tree_activity", line.page.0, i)), Sense::hover()).on_hover_text(activity.tip(&line.name));
    }
    if is_editing {
        // The field covers the right column.
    } else if hot {
        let ink = if delete.hovered() { s.danger } else { s.faint };
        ui.painter().text(pos2(delete_x, rect.center().y), Align2::CENTER_CENTER, "×", regular(12.0), ink);
    } else if busy {
        // The activity holds the right column.
    } else if line.level + 1 >= MAX_LEVELS {
        ui.painter().text(trail, Align2::CENTER_CENTER, format!("L{}", line.level + 1), FontId::monospace(9.0), s.faint);
    }
    let delete = delete.on_hover_text(format!("Delete {}…", line.name));
    let mut ask = None;
    if is_editing {
        // Clicks belong to the field.
    } else if delete.clicked() {
        ask = Some(Ask::Delete(line.page));
    } else if fold.as_ref().is_some_and(|f| f.clicked()) {
        ask = Some(Ask::Fold(line.page));
    } else if response.double_clicked() {
        ask = Some(Ask::Rename(line.page));
    } else if response.clicked() {
        ask = Some(Ask::Open(line.page));
    }
    response.context_menu(|ui| {
        if ui.button("Open").clicked() {
            ask = Some(Ask::Open(line.page));
            ui.close();
        }
        if ui.button("Rename page").clicked() {
            ask = Some(Ask::Rename(line.page));
            ui.close();
        }
        ui.separator();
        ui.label(egui::RichText::new("MARK AS").font(FontId::monospace(8.0)).color(s.faint));
        let exploration = line.mode == Mode::Exploration;
        for (mode, label) in [(Mode::Current, "State of the art"), (Mode::Exploration, "Exploration")] {
            let checked = (mode == Mode::Exploration) == exploration;
            if mode_item(ui, s, dark, mode, label, checked) {
                ask = Some(Ask::Mark(line.page, mode == Mode::Exploration));
                ui.close();
            }
        }
        ui.separator();
        if ui.button(egui::RichText::new("Delete page…").color(s.accent)).clicked() {
            ask = Some(Ask::Delete(line.page));
            ui.close();
        }
    });
    (ask, outer, end)
}

/// A busy page's mark in `at`: the Burn disc being written (see [`crate::burn_disc`]), or an
/// agent's brush in a turning arc. Both move, so the tree repaints while one shows.
fn paint_activity(ui: &Ui, s: &Skin, at: Rect, activity: &Activity) {
    let time = ui.input(|i| i.time);
    let painter = ui.painter();
    match activity {
        Activity::Burning => {
            crate::burn_disc::paint(painter, at, crate::burn_disc::DiscState::Burning { t: time }, &crate::map_colors::get());
        }
        Activity::Designing(_) => {
            let t = time as f32;
            let c = at.center();
            let r = at.width() / 2.0;
            let ink = s.map_ink;
            let start = t * std::f32::consts::TAU * 0.8;
            let arc: Vec<egui::Pos2> = (0..=16).map(|i| start + i as f32 / 16.0 * 1.75).map(|a| c + vec2(a.cos(), a.sin()) * (r - 1.0)).collect();
            painter.add(egui::Shape::line(arc, Stroke::new(1.4, ink)));
            crate::icons::paint(painter, Rect::from_center_size(c, vec2(12.0, 12.0)), crate::icons::Icon::Brush, ink);
        }
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
}

/// The Mark as items' width, the page menu's.
const MENU_W: f32 = 210.0;

/// One Mark as item: the mode's icon, its name, and a check on the page's current mode.
fn mode_item(ui: &mut Ui, s: &Skin, dark: bool, mode: Mode, label: &str, checked: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(MENU_W, 26.0), Sense::click());
    crate::a11y::button(&resp, &format!("Mark as {label}"));
    if resp.hovered() || checked {
        ui.painter().rect_filled(rect, CornerRadius::same(6), s.soft);
    }
    let icon = Rect::from_center_size(pos2(rect.min.x + 14.0, rect.center().y), vec2(ICON, ICON));
    crate::icons::paint(ui.painter(), icon, mode.icon(), mode.ink(dark));
    let font = egui::TextStyle::Button.resolve(ui.style());
    ui.painter().text(pos2(rect.min.x + 30.0, rect.center().y), Align2::LEFT_CENTER, label, font, s.text);
    if checked {
        let tick = Rect::from_center_size(pos2(rect.max.x - 14.0, rect.center().y), vec2(12.0, 12.0));
        crate::icons::paint(ui.painter(), tick, crate::icons::Icon::Check, s.text);
    }
    resp.clicked() && !checked
}

/// The fold mark: ⌄ when open, › when folded.
fn chevron(ui: &Ui, c: egui::Pos2, open: bool, color: Color32) {
    let stroke = Stroke::new(1.1, color);
    let points = if open {
        [pos2(c.x - 3.0, c.y - 1.5), pos2(c.x, c.y + 1.5), pos2(c.x + 3.0, c.y - 1.5)]
    } else {
        [pos2(c.x - 1.5, c.y - 3.0), pos2(c.x + 1.5, c.y), pos2(c.x - 1.5, c.y + 3.0)]
    };
    ui.painter().line_segment([points[0], points[1]], stroke);
    ui.painter().line_segment([points[1], points[2]], stroke);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(doc: &mut Document, parent: NodeId, name: &str) -> NodeId {
        let page = doc.add_page(name);
        doc.node_mut(page).unwrap().plugin_data.insert(PARENT_KEY.into(), parent.0.to_string());
        page
    }

    #[test]
    fn the_docked_rows_go_depth_first_in_document_order() {
        let mut doc = Document::new("Map");
        let overview = doc.pages[0];
        let canvas = sub(&mut doc, overview, "Canvas and layers");
        let components = sub(&mut doc, overview, "Components");
        let paint = sub(&mut doc, canvas, "Paint");
        let checkout = doc.add_page("Checkout");
        let listed = rows(&doc, &HashSet::new());
        assert_eq!(listed, [(overview, 0), (canvas, 1), (paint, 2), (components, 1), (checkout, 0)]);
        let folded = rows(&doc, &HashSet::from([canvas]));
        assert_eq!(folded, [(overview, 0), (canvas, 1), (components, 1), (checkout, 0)]);
    }

    #[test]
    fn a_page_keeps_its_mode_and_shows_what_is_happening_to_it() {
        let mut doc = Document::new("Map");
        let overview = doc.pages[0];
        let canvas = sub(&mut doc, overview, "Canvas and layers");
        let paint = sub(&mut doc, canvas, "Paint");
        let checkout = doc.add_page("Checkout");
        doc.node_mut(checkout).unwrap().plugin_data.insert(MODE_KEY.into(), EXPLORATION.into());
        assert_eq!(mode(&doc, overview), Mode::Current);
        assert_eq!(mode(&doc, checkout), Mode::Exploration);
        // A page Sync burns the page and its subpages, and nothing else.
        let canvas_sync = building_pages(&doc, Some(Some(canvas.0)));
        assert_eq!(activity(canvas, &canvas_sync, None), [Activity::Burning]);
        assert_eq!(activity(paint, &canvas_sync, None), [Activity::Burning]);
        assert_eq!(activity(overview, &canvas_sync, None), []);
        // An agent at work on a page designs it; burned at the same time, it shows both.
        assert_eq!(activity(checkout, &canvas_sync, Some("Claude".into())), [Activity::Designing("Claude".into())]);
        assert_eq!(activity(paint, &canvas_sync, Some("Claude".into())), [Activity::Burning, Activity::Designing("Claude".into())]);
        assert_eq!(Activity::Designing("Claude".into()).tip("Strokes"), "Claude is designing Strokes");
        // An all-pages Sync burns every page.
        let all = building_pages(&doc, Some(None));
        assert!([overview, canvas, paint, checkout].iter().all(|page| activity(*page, &all, None) == [Activity::Burning]));
    }

    fn app_with_pages() -> (crate::layers_drag_tests::Ui, NodeId, NodeId, NodeId) {
        let mut session = figcraft_engine::Session::new();
        let second = session.execute("page.add", &serde_json::json!({"name": "Onboarding"})).unwrap()["id"].as_u64().unwrap();
        let third = session.execute("page.add", &serde_json::json!({"name": "Pricing"})).unwrap()["id"].as_u64().unwrap();
        let first = session.doc().unwrap().pages[0];
        let u = crate::layers_drag_tests::Ui::new(session);
        (u, first, NodeId(second), NodeId(third))
    }

    #[test]
    fn mark_as_sets_and_clears_a_pages_mode() {
        crate::with_map_mode(|| {
            let (mut u, first, _, _) = app_with_pages();
            set_mode(&mut u.app, first, true);
            assert!(is_exploration(u.app.session.doc().unwrap(), first));
            set_mode(&mut u.app, first, false);
            let doc = u.app.session.doc().unwrap();
            assert!(!is_exploration(doc, first));
            assert!(!doc.node(first).unwrap().plugin_data.contains_key(MODE_KEY), "the state of the art is unmarked");
        });
    }

    #[test]
    fn a_build_makes_the_explorations_it_covered_the_state_of_the_art() {
        crate::with_map_mode(|| {
            let (mut u, _, onboarding, pricing) = app_with_pages();
            set_mode(&mut u.app, onboarding, true);
            set_mode(&mut u.app, pricing, true);
            built(&mut u.app, Some(onboarding.0));
            let doc = u.app.session.doc().unwrap();
            assert!(!is_exploration(doc, onboarding));
            assert!(is_exploration(doc, pricing), "a page the Sync did not cover stays an exploration");
            assert_eq!(u.app.ui.toast.as_ref().map(|t| t.0.as_str()), Some("Built · Onboarding is now the state of the art"));
            assert_eq!(u.app.ui.toast_mark.and_then(|mark| mark.action), Some(crate::ToastAction::OpenSync));
            // A build with no exploration in it says nothing more.
            u.app.ui.toast = None;
            built(&mut u.app, Some(onboarding.0));
            assert!(u.app.ui.toast.is_none());
        });
    }

    #[test]
    fn a_new_page_its_agent_marks_an_exploration_is_announced_once() {
        crate::with_map_mode(|| {
            let (mut u, _, onboarding, _) = app_with_pages();
            watch_new(&mut u.app, onboarding);
            note_new_explorations(&mut u.app, 10.0);
            assert!(u.app.ui.toast.is_none(), "not marked yet");
            set_mode(&mut u.app, onboarding, true);
            note_new_explorations(&mut u.app, 12.0);
            assert_eq!(u.app.ui.toast.as_ref().map(|t| t.0.as_str()), Some("New page · Onboarding · the agent marked it Exploration"));
            assert!(u.app.page_tree.new_pages.is_empty());
            // Watched too long: forgotten without a word.
            u.app.ui.toast = None;
            watch_new(&mut u.app, onboarding);
            set_mode(&mut u.app, onboarding, false);
            note_new_explorations(&mut u.app, 0.0);
            note_new_explorations(&mut u.app, NEW_PAGE_WATCH + 1.0);
            assert!(u.app.page_tree.new_pages.is_empty());
            assert!(u.app.ui.toast.is_none());
        });
    }

    #[test]
    fn clicking_a_docked_page_opens_it() {
        crate::with_map_mode(|| {
            let mut session = figcraft_engine::Session::new();
            let second = session.execute("page.add", &serde_json::json!({"name": "Second"})).unwrap()["id"].as_u64().unwrap();
            let first = session.doc().unwrap().pages[0];
            session.execute("page.select", &serde_json::json!({"id": first.0})).unwrap();
            let mut u = crate::layers_drag_tests::Ui::new(session);
            u.app.onboarding.active = false;
            u.app.onboarding.decided = true;
            u.app.ui.home = false;
            u.app.ws.pane_open = true;
            u.frame(vec![]);
            assert_eq!(u.app.session.page().ok(), Some(first));
            // Walk down the sidebar's rows until one opens the second page.
            let mut opened = false;
            // Below the switcher head, which opens the switcher.
            for y in (124..400).step_by(10) {
                let p = egui::pos2(120.0, y as f32);
                u.button(p, true);
                u.button(p, false);
                if u.app.session.page().ok() == Some(NodeId(second)) {
                    opened = true;
                    break;
                }
            }
            assert!(opened, "a row of the docked tree opens its page");
            assert!(!u.app.new_page.open);
        });
    }
}
