//! Mindmap's Actions palette (⇧⌘P; ⌘K too): every map action in one searchable list. With an
//! empty query, recently used actions come first, then every action by group; typing filters by
//! name, group and scope, best match first. Rows show their shortcut as key caps, toggles their
//! current state, and actions that cannot run now stay listed, dimmed, with the reason. When no
//! action matches, the palette offers to search the map's text instead.
//!
//! The rows come from `map_action_catalog`; running one goes through
//! [`crate::menus::activate_map_action`], the guard every map surface shares.

use egui::{Align2, Color32, CornerRadius, FontId, Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::FigApp;
use crate::map_action_catalog::{self as catalog, ACTIONS, Action};
use crate::theme::{Tokens, regular};

/// The palette card's width (narrower windows shrink it).
const WIDTH: f32 = 520.0;
const SEARCH_H: f32 = 44.0;
const ROW_H: f32 = 30.0;
const FOOTER_H: f32 = 30.0;
/// Space between the window's top content edge and the palette.
const TOP: f32 = 16.0;
/// The header of the recently used group.
pub(crate) const RECENTLY_USED: &str = "Recently used";
/// Footer hints when there are rows to choose from.
const HINTS: &str = "↑↓ choose   ↵ run   esc close";

/// The map's warm palette, as the composer and Sync review use it; dark mode keeps the tokens'
/// surfaces.
struct Pal {
    panel: Color32,
    border: Color32,
    text: Color32,
    muted: Color32,
    faint: Color32,
    teal: Color32,
    warm: Color32,
    hover: Color32,
    cap: Color32,
    scrim: Color32,
}

impl Pal {
    fn of(t: &Tokens) -> Pal {
        if t.dark {
            Pal {
                panel: t.panel,
                border: t.border,
                text: t.text,
                muted: t.text_secondary,
                faint: t.text_tertiary,
                teal: Color32::from_rgb(0x7F, 0xBD, 0xB5),
                warm: Color32::from_rgba_unmultiplied(0xD9, 0x7A, 0x5F, 34),
                hover: t.hover,
                cap: t.field,
                scrim: Color32::from_black_alpha(60),
            }
        } else {
            Pal {
                panel: Color32::WHITE,
                border: Color32::from_rgb(0xE3, 0xE2, 0xDF),
                text: Color32::from_rgb(0x1A, 0x1A, 0x1A),
                muted: Color32::from_rgb(0x6E, 0x6C, 0x68),
                faint: Color32::from_rgb(0x96, 0x93, 0x8D),
                teal: Color32::from_rgb(0x2F, 0x6F, 0x6A),
                warm: Color32::from_rgb(0xF1, 0xEC, 0xE7),
                hover: Color32::from_rgb(0xF8, 0xF6, 0xF3),
                cap: Color32::from_rgb(0xF6, 0xF5, 0xF2),
                scrim: Color32::from_rgba_unmultiplied(0x1A, 0x1A, 0x1A, 0x0F),
            }
        }
    }
}

/// The palette's rows for `query`, by section: a section without a title is the ranked search
/// result. Every catalog action appears at most once; an empty result means nothing matched.
pub(crate) fn sections(app: &FigApp, query: &str) -> Vec<(Option<&'static str>, Vec<&'static Action>)> {
    let platform = crate::shortcuts::platform();
    if !query.trim().is_empty() {
        let hits = catalog::ranked(query, platform);
        return if hits.is_empty() { Vec::new() } else { vec![(None, hits)] };
    }
    let recent: Vec<&'static Action> = app
        .session
        .prefs
        .recent_actions
        .iter()
        .filter_map(|key| key.strip_prefix("command:"))
        .filter_map(catalog::find)
        .take(figcraft_engine::MAX_RECENT_ACTIONS)
        .collect();
    let mut out: Vec<(Option<&'static str>, Vec<&'static Action>)> = Vec::new();
    if !recent.is_empty() {
        out.push((Some(RECENTLY_USED), recent.clone()));
    }
    for action in ACTIONS.iter().filter(|action| !recent.iter().any(|r| r.id == action.id)) {
        if let Some((_, rows)) = out.iter_mut().skip(usize::from(!recent.is_empty())).find(|(group, _)| *group == Some(action.group)) {
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

/// The no-match row and footer, quoting the query.
pub(crate) fn search_map_label(query: &str) -> String {
    format!("Search this map for “{}”", query.trim())
}

pub(crate) fn no_match_label(query: &str) -> String {
    format!("No action named “{}”.   ↵ search the map", query.trim())
}

/// Run the no-match row: close the palette and open map search with the query typed. Declines
/// (palette stays open) when map search cannot run.
pub(crate) fn search_map(app: &mut FigApp, ctx: &egui::Context, query: &str) -> bool {
    let Some(action) = catalog::find("map.search") else { return false };
    if !catalog::enabled(app, action) {
        return false;
    }
    app.ui.palette = None;
    if let Some(focus) = ctx.memory(|m| m.focused()) {
        ctx.memory_mut(|m| m.surrender_focus(focus));
    }
    let opened = crate::map_navigation::open_search(app, ctx, query);
    ctx.request_repaint();
    opened
}

/// What a click or Enter chose this frame.
enum Pick {
    Action(&'static str),
    SearchMap,
}

/// Paint `text` as a key cap ending at `right`, vertically centred on `cy`; returns its left edge.
fn keycap(ui: &Ui, pal: &Pal, text: &str, right: f32, cy: f32) -> f32 {
    let galley = ui.painter().layout_no_wrap(text.to_string(), regular(10.5), pal.muted);
    let w = (galley.size().x + 12.0).max(22.0);
    let r = Rect::from_min_size(pos2(right - w, cy - 9.0), vec2(w, 18.0));
    ui.painter().rect_filled(r, CornerRadius::same(4), pal.cap);
    ui.painter().rect_stroke(r, CornerRadius::same(4), Stroke::new(1.0, pal.border), StrokeKind::Inside);
    ui.painter().galley(pos2(r.center().x - galley.size().x / 2.0, cy - galley.size().y / 2.0), galley, pal.muted);
    r.min.x
}

/// One row: the label at the left; at the right the key caps and, before them, a toggle's state
/// or a disabled action's reason. Returns the row's response.
fn row(ui: &mut Ui, pal: &Pal, label: &str, keys: &[String], note: Option<(&str, Color32)>, enabled: bool, highlighted: bool) -> egui::Response {
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::click());
    let pill = r.shrink2(vec2(6.0, 0.0));
    if highlighted {
        ui.painter().rect_filled(pill, CornerRadius::same(6), pal.warm);
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(pill, CornerRadius::same(6), pal.hover);
    }
    let cy = r.center().y;
    let mut right = pill.max.x - 8.0;
    for (i, k) in keys.iter().enumerate().rev() {
        right = keycap(ui, pal, k, right, cy) - if i > 0 { 4.0 } else { 0.0 };
    }
    if !keys.is_empty() {
        right -= 10.0;
    }
    if let Some((text, color)) = note {
        let galley = ui.painter().layout_no_wrap(text.to_string(), regular(10.5), color);
        right -= galley.size().x;
        ui.painter().galley(pos2(right, cy - galley.size().y / 2.0), galley, color);
        right -= 12.0;
    }
    let x = pill.min.x + 8.0;
    let shown = crate::widgets::elide_font(ui, label, (right - x).max(20.0), regular(12.5));
    ui.painter().text(pos2(x, cy), Align2::LEFT_CENTER, shown, regular(12.5), if enabled { pal.text } else { pal.faint });
    resp
}

fn divider(ui: &mut Ui, pal: &Pal) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::ZERO, pal.border);
}

/// A row's key cap: its primary shortcut (Help also lists aliases); none for palette-only actions.
fn caps(action: &Action, platform: figcraft_engine::keymap::Platform) -> Vec<String> {
    let keys = action.keys(platform);
    if keys.is_empty() { Vec::new() } else { vec![figcraft_engine::keymap::display(keys, platform)] }
}

/// Draw the palette and handle its keys and clicks. Escape and a click outside close it.
pub(crate) fn show(app: &mut FigApp, ctx: &egui::Context) {
    let Some(mut query) = app.ui.palette.clone() else { return };
    let pal = Pal::of(&Tokens::get(ctx));
    let platform = crate::shortcuts::platform();
    let (step, enter, escape) = ctx.input_mut(|i| {
        (
            i.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) as i64
                - i.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) as i64,
            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
        )
    });
    let pressed = ctx.input(|i| i.pointer.any_pressed());
    let armed = std::mem::replace(&mut app.ui.palette_armed, false);
    let screen = ctx.content_rect();
    // The header stays clear of the scrim, as in the design; elsewhere (home) the whole window dims.
    let header = if app.ui.home || app.session.ds().is_ok_and(|ds| ds.view.ui_hidden) {
        0.0
    } else if app.ui.mac_inset() {
        crate::MAC_BAND
    } else {
        44.0
    };
    let scrim = Rect::from_min_max(pos2(screen.min.x, screen.min.y + header), screen.max);
    let width = WIDTH.min(screen.width() - 32.0).max(240.0);
    let search_id = Id::new("map_action_search");
    let mut picked = None;
    let area = egui::Area::new(Id::new("palette"))
        .anchor(Align2::CENTER_TOP, vec2(0.0, header + TOP))
        .order(egui::Order::Foreground)
        .constrain(true)
        .show(ctx, |ui| {
            // Painted first on the palette's own layer, so it dims what is below and never the card.
            ui.ctx().layer_painter(ui.layer_id()).rect_filled(scrim, CornerRadius::ZERO, pal.scrim);
            egui::Frame::NONE
                .fill(pal.panel)
                .stroke(Stroke::new(1.0, pal.border))
                .corner_radius(CornerRadius::same(10))
                .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(26) })
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    // Search row: the prompt, the field and the palette's own shortcut.
                    let (bar, _) = ui.allocate_exact_size(vec2(width, SEARCH_H), Sense::hover());
                    ui.painter().text(pos2(bar.min.x + 16.0, bar.center().y), Align2::LEFT_CENTER, "›", regular(16.0), pal.faint);
                    let own = catalog::find("map.actions").map(|a| caps(a, platform)).unwrap_or_default();
                    let mut field_right = bar.max.x - 12.0;
                    for k in &own {
                        field_right = keycap(ui, &pal, k, field_right, bar.center().y) - 8.0;
                    }
                    let field = Rect::from_min_max(pos2(bar.min.x + 32.0, bar.center().y - 10.0), pos2(field_right, bar.center().y + 10.0));
                    let response = ui.put(
                        field,
                        egui::TextEdit::singleline(&mut query)
                            .id(search_id)
                            .frame(egui::Frame::NONE)
                            .margin(egui::Margin::ZERO)
                            .font(regular(14.0))
                            .text_color(pal.text)
                            .hint_text(egui::RichText::new("Run an action…").font(regular(14.0)).color(pal.faint))
                            .desired_width(field.width()),
                    );
                    response.request_focus();
                    let sections = sections(app, &query);
                    let flat: Vec<&'static Action> = sections.iter().flat_map(|(_, rows)| rows.iter().copied()).collect();
                    if response.changed() {
                        app.ui.palette_hi = flat.iter().position(|a| catalog::enabled(app, a)).unwrap_or(0);
                    }
                    let count = flat.len().max(usize::from(!query.trim().is_empty()));
                    let hi = (app.ui.palette_hi as i64 + step).clamp(0, count.saturating_sub(1) as i64) as usize;
                    app.ui.palette_hi = hi;
                    divider(ui, &pal);
                    if flat.is_empty() && !query.trim().is_empty() {
                        ui.add_space(6.0);
                        let search = catalog::find("map.search");
                        let enabled = search.is_some_and(|a| catalog::enabled(app, a));
                        let reason = search.and_then(|a| catalog::disabled_reason(app, a));
                        let keys = search.map(|a| caps(a, platform)).unwrap_or_default();
                        let r = row(ui, &pal, &search_map_label(&query), &keys, reason.map(|r| (r, pal.faint)), enabled, true);
                        if enabled && (r.clicked() || enter) {
                            picked = Some(Pick::SearchMap);
                        }
                        ui.add_space(6.0);
                    } else {
                        let max_h = (screen.height() - header - TOP - SEARCH_H - FOOTER_H - 48.0).clamp(ROW_H * 3.0, 420.0);
                        egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
                            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                            ui.add_space(6.0);
                            let mut index = 0;
                            for (title, rows) in &sections {
                                if let Some(title) = title {
                                    if index > 0 {
                                        ui.add_space(6.0);
                                    }
                                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
                                    ui.painter().text(
                                        pos2(r.min.x + 14.0, r.center().y),
                                        Align2::LEFT_CENTER,
                                        title.to_uppercase(),
                                        FontId::monospace(9.0),
                                        pal.faint,
                                    );
                                }
                                for action in rows {
                                    let reason = catalog::disabled_reason(app, action);
                                    let note = match reason {
                                        Some(reason) => Some((reason, pal.faint)),
                                        None => catalog::toggle_state(app, action).map(|state| (state, pal.teal)),
                                    };
                                    let resp = row(ui, &pal, action.label, &caps(action, platform), note, reason.is_none(), index == hi);
                                    if reason.is_none() && (resp.clicked() || (enter && index == hi)) {
                                        picked = Some(Pick::Action(action.id));
                                    }
                                    if step != 0 && index == hi {
                                        resp.scroll_to_me(None);
                                    }
                                    index += 1;
                                }
                            }
                            ui.add_space(6.0);
                        });
                    }
                    divider(ui, &pal);
                    let (foot, _) = ui.allocate_exact_size(vec2(width, FOOTER_H), Sense::hover());
                    let hints = if flat.is_empty() && !query.trim().is_empty() { no_match_label(&query) } else { HINTS.to_string() };
                    let counted = count_label(&query, flat.len());
                    let count_w = ui.painter().layout_no_wrap(counted.clone(), regular(10.0), pal.faint).size().x;
                    let hints = crate::widgets::elide_font(ui, &hints, (foot.width() - count_w - 44.0).max(20.0), regular(10.0));
                    ui.painter().text(pos2(foot.min.x + 14.0, foot.center().y), Align2::LEFT_CENTER, hints, regular(10.0), pal.faint);
                    ui.painter().text(pos2(foot.max.x - 14.0, foot.center().y), Align2::RIGHT_CENTER, counted, regular(10.0), pal.faint);
                });
        });
    let on_toggle = app.ui.actions_rect.is_some_and(|rect| ctx.input(|i| i.pointer.hover_pos().is_some_and(|p| rect.contains(p))));
    let outside = pressed && !armed && !on_toggle && !area.response.contains_pointer();
    if escape || outside {
        app.ui.palette = None;
        ctx.memory_mut(|m| m.surrender_focus(search_id));
    } else if let Some(pick) = picked {
        match pick {
            Pick::Action(id) => {
                crate::menus::activate_map_action(app, ctx, id);
            }
            Pick::SearchMap => {
                search_map(app, ctx, &query);
            }
        }
    } else {
        app.ui.palette = Some(query);
    }
}
