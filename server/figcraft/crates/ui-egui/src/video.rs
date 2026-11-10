//! Video clips on the map canvas.
//!
//! A card tagged with `afterburn.video` plugin data ([`afterburn_video::Clip`]) shows its clip's
//! first frame as an ordinary image fill, so the page renders, exports and screenshots as ever.
//! This module adds what moves: a play button over the card, and, once pressed, the decoded
//! frames drawn over the card with a small control bar (pause, seek, time).
//!
//! The clip's file is not in the document. It comes from the worktree's `.afterburn/media/` when
//! the node runs on this machine, else from the node in chunks (`media.read`) into a local cache.
//! One worker thread per playing card fetches, decodes (OpenH264) and keeps time; the window only
//! uploads the newest frame as a texture. Nothing runs, and the window stays asleep, until a clip
//! is played; a clip that leaves the screen or its page pauses.
//!
//! The frames are an overlay on the page texture: while a clip shows a frame, layers stacked
//! above its card are covered inside the card's rectangle.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use afterburn_video::{Clip, MEDIA_DIR, Reader, VIDEO_KEY};
use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
use figcraft_doc::NodeId;
use serde_json::json;

use crate::FigApp;

/// How many bytes one `media.read` asks for.
const CHUNK: u64 = 2 * 1024 * 1024;
/// The narrowest card (on screen) that gets a control bar, and that keeps playing.
const MIN_BAR_W: f32 = 150.0;
const MIN_PLAY_W: f32 = 24.0;
const BAR_H: f32 = 30.0;

type Wake = Arc<dyn Fn() + Send + Sync>;

/// Where a clip's bytes come from.
#[derive(Clone, Debug)]
pub struct Source {
    /// The node's address and session, for `media.read`. `None`: local files only.
    pub node: Option<(String, String)>,
    /// The worktree, when it may be on this machine.
    pub worktree: Option<PathBuf>,
    /// Where fetched clips are kept.
    pub cache: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Fetching the file: bytes so far.
    Loading(u64),
    Ready,
    Failed(String),
}

enum Cmd {
    Play,
    Pause,
    Seek(u64),
}

struct Shared {
    status: Status,
    playing: bool,
    position_ms: u64,
    /// The newest decoded frame, until the window takes it.
    frame: Option<egui::ColorImage>,
}

struct Player {
    clip: Clip,
    shared: Arc<Mutex<Shared>>,
    tx: mpsc::Sender<Cmd>,
    texture: Option<egui::TextureHandle>,
    /// Where a drag of the seek bar is, until it is let go.
    scrub: Option<u64>,
}

/// What the control channel and the tests see of a player.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub status: Status,
    pub playing: bool,
    pub position_ms: u64,
    pub has_frame: bool,
}

#[derive(Default)]
pub struct Videos {
    players: HashMap<NodeId, Player>,
    /// Tests give the clips' folder here instead of a node.
    pub source_override: Option<Source>,
}

impl Videos {
    pub fn snapshot(&self, id: NodeId) -> Option<Snapshot> {
        let player = self.players.get(&id)?;
        let shared = lock(&player.shared);
        Some(Snapshot {
            status: shared.status.clone(),
            playing: shared.playing,
            position_ms: shared.position_ms,
            has_frame: player.texture.is_some() || shared.frame.is_some(),
        })
    }

    pub fn is_playing(&self) -> bool {
        self.players.values().any(|player| lock(&player.shared).playing)
    }

    /// Play, or pause, the card's clip.
    pub fn toggle(&mut self, id: NodeId, clip: &Clip, source: Result<Source, String>, wake: Wake) {
        if let Some(player) = self.players.get(&id) {
            let shared = lock(&player.shared);
            let (failed, playing) = (matches!(shared.status, Status::Failed(_)), shared.playing);
            drop(shared);
            if !failed && player.clip == *clip {
                let _ = player.tx.send(if playing { Cmd::Pause } else { Cmd::Play });
                return;
            }
        }
        // First play (or a retry after a failure): start the card's worker.
        let shared = Arc::new(Mutex::new(Shared { status: Status::Loading(0), playing: true, position_ms: 0, frame: None }));
        let (tx, rx) = mpsc::channel();
        match source {
            Ok(source) => {
                let (clip, state) = (clip.clone(), shared.clone());
                let spawned = std::thread::Builder::new().name("afterburn-video".into()).spawn(move || worker(&clip, &source, &state, &rx, &wake));
                if let Err(e) = spawned {
                    fail(&shared, format!("The player could not start: {e}"));
                }
            }
            Err(e) => fail(&shared, e),
        }
        self.players.insert(id, Player { clip: clip.clone(), shared, tx, texture: None, scrub: None });
    }

    pub fn pause(&mut self, id: NodeId) {
        if let Some(player) = self.players.get(&id) {
            let _ = player.tx.send(Cmd::Pause);
        }
    }

    pub fn seek(&mut self, id: NodeId, time_ms: u64) {
        if let Some(player) = self.players.get(&id) {
            let _ = player.tx.send(Cmd::Seek(time_ms.min(player.clip.duration_ms)));
        }
    }
}

fn lock(shared: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn fail(shared: &Mutex<Shared>, message: String) {
    let mut shared = lock(shared);
    shared.status = Status::Failed(message);
    shared.playing = false;
}

// ---------------------------------------------------------------------------------------------
// The worker: fetch, decode, keep time.

/// The clip's file on this machine: the worktree's own copy, the cache, or fetched from the node.
fn local_file(clip: &Clip, source: &Source, shared: &Mutex<Shared>, wake: &Wake) -> Result<PathBuf, String> {
    let complete = |path: &Path| std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() == clip.size);
    if let Some(worktree) = &source.worktree {
        let path = worktree.join(MEDIA_DIR).join(&clip.file);
        if complete(&path) {
            return Ok(path);
        }
    }
    let cached = source.cache.join(&clip.file);
    if complete(&cached) {
        return Ok(cached);
    }
    let Some((address, session)) = &source.node else {
        return Err(format!("The clip {} is not in {MEDIA_DIR}.", clip.file));
    };
    std::fs::create_dir_all(&source.cache).map_err(|e| format!("The clip cache cannot be written: {e}"))?;
    let partial = source.cache.join(format!(".{}.part", clip.file));
    let mut out = std::fs::File::create(&partial).map_err(|e| format!("The clip cache cannot be written: {e}"))?;
    let mut got = 0u64;
    while got < clip.size {
        let request = json!({"method": "media.read", "session": session, "file": clip.file, "offset": got, "length": CHUNK});
        let reply = crate::node::rpc(address, request, Duration::from_secs(60))?;
        if reply.get("ok") != Some(&serde_json::Value::Bool(true)) {
            return Err(reply.get("error").and_then(|e| e.as_str()).unwrap_or("The node refused the clip.").to_string());
        }
        use base64::Engine as _;
        let data = reply.get("data").and_then(|d| d.as_str()).ok_or("The node sent no clip data.")?;
        let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| "The node sent unreadable clip data.")?;
        if bytes.is_empty() || reply.get("size").and_then(|s| s.as_u64()) != Some(clip.size) {
            return Err("The clip on the node is not the one this card names.".into());
        }
        std::io::Write::write_all(&mut out, &bytes).map_err(|e| format!("The clip cache cannot be written: {e}"))?;
        got += bytes.len() as u64;
        lock(shared).status = Status::Loading(got);
        wake();
    }
    drop(out);
    std::fs::rename(&partial, &cached).map_err(|e| format!("The clip cache cannot be written: {e}"))?;
    Ok(cached)
}

fn worker(clip: &Clip, source: &Source, shared: &Mutex<Shared>, rx: &mpsc::Receiver<Cmd>, wake: &Wake) {
    let opened = local_file(clip, source, shared, wake).and_then(|path| Reader::open(&path).map(|reader| (reader, path)));
    let (mut reader, path) = match opened {
        Ok(opened) => opened,
        Err(e) => {
            fail(shared, e);
            wake();
            return;
        }
    };
    let audio = audio::Audio::open(&path, clip.audio);
    lock(shared).status = Status::Ready;
    let duration = reader.duration_ms();
    // The clock: `base` ms of the clip were at `origin`.
    let mut playing = true;
    let mut base = 0u64;
    let mut origin = Instant::now();
    audio.play();
    let stop = |e: String| {
        fail(shared, e);
        wake();
    };
    loop {
        // Paused: sleep until told otherwise. Playing: sleep until the next frame is due.
        let wait = if playing {
            let now = base + origin.elapsed().as_millis() as u64;
            match reader.next_time_ms() {
                Some(next) if next > now => Some(Duration::from_millis(next - now)),
                Some(_) => Some(Duration::ZERO),
                None => {
                    // The end: stay on the last frame.
                    playing = false;
                    audio.pause();
                    let mut state = lock(shared);
                    state.playing = false;
                    state.position_ms = duration;
                    drop(state);
                    wake();
                    None
                }
            }
        } else {
            None
        };
        let cmd = match wait {
            Some(wait) if wait.is_zero() => rx.try_recv().ok(),
            Some(wait) => match rx.recv_timeout(wait) {
                Ok(cmd) => Some(cmd),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            },
            None => match rx.recv() {
                Ok(cmd) => Some(cmd),
                Err(_) => return,
            },
        };
        match cmd {
            Some(Cmd::Pause) => {
                if playing {
                    base += origin.elapsed().as_millis() as u64;
                }
                playing = false;
                audio.pause();
                lock(shared).playing = false;
                wake();
                continue;
            }
            Some(Cmd::Play) => {
                if reader.next_time_ms().is_none() {
                    // Played to the end: again from the start.
                    if let Err(e) = reader.seek(0) {
                        return stop(e);
                    }
                    base = 0;
                    audio.seek(0);
                }
                playing = true;
                origin = Instant::now();
                audio.play();
                lock(shared).playing = true;
                wake();
                continue;
            }
            Some(Cmd::Seek(to)) => {
                // Back to the key frame before, then on to the frame asked for.
                let mut shown = None;
                let result = reader.seek(to).and_then(|_| {
                    while let Some(frame) = reader.next_frame()? {
                        let reached = frame.time_ms >= to;
                        shown = Some(frame);
                        if reached {
                            break;
                        }
                    }
                    Ok(())
                });
                if let Err(e) = result {
                    return stop(e);
                }
                base = to;
                origin = Instant::now();
                audio.seek(to);
                let mut state = lock(shared);
                state.position_ms = to;
                if let Some(frame) = shown {
                    state.frame = Some(image(&frame));
                }
                drop(state);
                wake();
                continue;
            }
            None => {}
        }
        if !playing {
            continue;
        }
        let now = base + origin.elapsed().as_millis() as u64;
        if reader.next_time_ms().is_none_or(|next| next > now) {
            continue;
        }
        match reader.next_frame() {
            Ok(Some(frame)) => {
                // Running late: decode on without showing this one.
                if reader.next_time_ms().is_some_and(|next| next <= now) {
                    continue;
                }
                let mut state = lock(shared);
                state.position_ms = frame.time_ms;
                state.frame = Some(image(&frame));
                drop(state);
                wake();
            }
            Ok(None) => {}
            Err(e) => return stop(e),
        }
    }
}

fn image(frame: &afterburn_video::Frame) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied([frame.width as usize, frame.height as usize], &frame.rgba)
}

/// The clip's sound. Without the `video-audio` feature (or without a sound device) clips are
/// silent and everything else works the same.
mod audio {
    use std::path::Path;

    #[cfg(feature = "video-audio")]
    pub struct Audio {
        // The stream must live as long as the sink plays.
        inner: Option<(rodio::OutputStream, rodio::Sink)>,
    }

    #[cfg(feature = "video-audio")]
    impl Audio {
        pub fn open(path: &Path, has_audio: bool) -> Audio {
            let open = || {
                if !has_audio {
                    return None;
                }
                let mut stream = rodio::OutputStreamBuilder::open_default_stream().ok()?;
                stream.log_on_drop(false);
                let source = rodio::Decoder::try_from(std::fs::File::open(path).ok()?).ok()?;
                let sink = rodio::Sink::connect_new(stream.mixer());
                sink.pause();
                sink.append(source);
                Some((stream, sink))
            };
            Audio { inner: open() }
        }
        pub fn play(&self) {
            if let Some((_, sink)) = &self.inner {
                sink.play();
            }
        }
        pub fn pause(&self) {
            if let Some((_, sink)) = &self.inner {
                sink.pause();
            }
        }
        pub fn seek(&self, time_ms: u64) {
            if let Some((_, sink)) = &self.inner {
                let _ = sink.try_seek(std::time::Duration::from_millis(time_ms));
            }
        }
    }

    #[cfg(not(feature = "video-audio"))]
    pub struct Audio;

    #[cfg(not(feature = "video-audio"))]
    impl Audio {
        pub fn open(_: &Path, _: bool) -> Audio {
            Audio
        }
        pub fn play(&self) {}
        pub fn pause(&self) {}
        pub fn seek(&self, _: u64) {}
    }
}

// ---------------------------------------------------------------------------------------------
// The canvas.

/// Where fetched clips are kept on this machine.
fn cache_dir() -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = home().map(|home| home.join("Library/Caches"));
    #[cfg(not(target_os = "macos"))]
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| home().map(|home| home.join(".cache")));
    base.unwrap_or_else(std::env::temp_dir).join("Afterburn").join("media")
}

fn source(app: &FigApp) -> Result<Source, String> {
    if let Some(source) = &app.ui.videos.source_override {
        return Ok(source.clone());
    }
    let node = match crate::workspaces::active_node(app) {
        Ok(node) => node,
        // A map opened straight from its file plays the clips beside it.
        Err(offline) => {
            let beside = app.session.ds().ok().and_then(|ds| ds.path.clone()).map(PathBuf::from).and_then(|file| {
                let dir = file.parent()?;
                (dir.file_name()? == ".afterburn").then(|| dir.parent().map(Path::to_path_buf))?
            });
            return match beside {
                Some(worktree) => Ok(Source { node: None, worktree: Some(worktree), cache: cache_dir() }),
                None => Err(offline),
            };
        }
    };
    Ok(Source { worktree: (!node.path.is_empty()).then(|| PathBuf::from(&node.path)), node: Some((node.address, node.session)), cache: cache_dir() })
}

/// The cards of the page in front that show a clip, with their rectangle on screen.
pub fn cards(app: &FigApp) -> Vec<(NodeId, Clip, Rect)> {
    let Ok(ds) = app.session.ds() else { return Vec::new() };
    let doc = &ds.doc;
    doc.subtree(ds.page)
        .into_iter()
        .filter_map(|id| {
            let clip = Clip::parse(doc.node(id)?.plugin_data.get(VIDEO_KEY)?)?;
            let quad = crate::canvas::node_quad(app, doc, id)?;
            let rect = Rect::from_points(&quad);
            rect.is_finite().then_some((id, clip, rect))
        })
        .collect()
}

fn clock(ms: u64) -> String {
    let secs = ms / 1000;
    format!("{}:{:02}", secs / 60, secs % 60)
}

fn play_shape(center: Pos2, size: f32, ink: Color32) -> Shape {
    // A triangle pointing right, nudged so it looks centred.
    let (w, h) = (size * 0.9, size);
    let left = center.x - w / 2.0 + size * 0.08;
    Shape::convex_polygon(vec![pos2(left, center.y - h / 2.0), pos2(left + w, center.y), pos2(left, center.y + h / 2.0)], ink, Stroke::NONE)
}

fn pause_shape(painter: &egui::Painter, center: Pos2, size: f32, ink: Color32) {
    let (bar, gap) = (size * 0.3, size * 0.2);
    for x in [center.x - gap / 2.0 - bar, center.x + gap / 2.0] {
        painter.rect_filled(Rect::from_min_size(pos2(x, center.y - size / 2.0), vec2(bar, size)), CornerRadius::same(1), ink);
    }
}

/// Play, or pause, the card's clip (the play button, and the control channel's `ui.video`).
pub fn toggle(app: &mut FigApp, ctx: &egui::Context, id: NodeId, clip: &Clip) {
    let wake_ctx = ctx.clone();
    let wake: Wake = Arc::new(move || wake_ctx.request_repaint());
    let from = source(app);
    app.ui.videos.toggle(id, clip, from, wake);
    ctx.request_repaint();
}

/// Draw the clips' frames and controls over their cards, and take their clicks. Called after the
/// page texture is drawn.
pub fn canvas(app: &mut FigApp, ui: &mut Ui, clip_rect: Rect) {
    let cards = cards(app);
    // A card that is gone (deleted, or on another page) stops: dropping its player ends its worker.
    app.ui.videos.players.retain(|id, player| cards.iter().any(|(card, clip, _)| card == id && *clip == player.clip));
    if cards.is_empty() {
        return;
    }
    let ctx = ui.ctx().clone();
    let painter = ui.painter_at(clip_rect);
    let white = Color32::WHITE;
    let shade = Color32::from_black_alpha(150);
    for (id, clip, rect) in cards {
        let visible = rect.intersects(clip_rect) && rect.width() >= MIN_PLAY_W;
        // Take the worker's newest frame; read where the clip is.
        let mut state = None;
        if let Some(player) = app.ui.videos.players.get_mut(&id) {
            let mut shared = lock(&player.shared);
            let frame = shared.frame.take();
            state = Some((shared.status.clone(), shared.playing, shared.position_ms));
            drop(shared);
            if let Some(frame) = frame {
                match &mut player.texture {
                    Some(texture) => texture.set(frame, egui::TextureOptions::LINEAR),
                    None => player.texture = Some(ctx.load_texture(format!("afterburn-video-{}", id.0), frame, egui::TextureOptions::LINEAR)),
                }
            }
            if !visible && state.as_ref().is_some_and(|s| s.1) {
                // Off screen (or too small to see): stop spending on it.
                let _ = player.tx.send(Cmd::Pause);
            }
        }
        if !visible {
            continue;
        }
        let player = app.ui.videos.players.get(&id);
        if let Some(texture) = player.and_then(|p| p.texture.as_ref()) {
            // The frame covers the card as its poster does: cropped about the middle, never
            // stretched, when the card is not the clip's shape (a film card as long as its cut).
            let [tw, th] = texture.size();
            let (frame, card) = (tw as f32 / (th as f32).max(1.0), rect.width() / rect.height().max(1.0));
            let (u, v) = if card > frame { (1.0, frame / card) } else { (card / frame, 1.0) };
            let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(u, v));
            painter.image(texture.id(), rect, uv, white);
        }
        let scrub = player.and_then(|p| p.scrub);
        let (status, playing, position) = state.unwrap_or((Status::Ready, false, 0));
        let hover = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| rect.contains(p));
        let mut toggle = false;
        let mut seek = None;
        let mut scrub_to = scrub;
        let mut scrub_end = false;
        let failed = match &status {
            Status::Failed(message) => Some(message.clone()),
            _ => None,
        };

        // The button in the middle: play (or retry) while the clip stands still.
        let loading = matches!(status, Status::Loading(_)) && failed.is_none();
        if !playing || loading {
            let size = (rect.width().min(rect.height()) * 0.24).clamp(18.0, 72.0);
            let badge = Rect::from_center_size(rect.center(), vec2(size, size));
            let response =
                ui.interact(badge.expand(4.0), ui.id().with(("video_play", id)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            let fill = if response.hovered() { Color32::from_black_alpha(200) } else { shade };
            painter.circle_filled(badge.center(), size / 2.0, fill);
            painter.circle_stroke(badge.center(), size / 2.0, Stroke::new((size * 0.04).max(1.0), white));
            if let Status::Loading(got) = status {
                // Fetching: the ring fills as the file arrives.
                let part = (got as f32 / clip.size.max(1) as f32).clamp(0.0, 1.0);
                let steps = 48;
                let points: Vec<Pos2> = (0..=((steps as f32 * part) as usize))
                    .map(|i| {
                        let angle = std::f32::consts::TAU * i as f32 / steps as f32 - std::f32::consts::FRAC_PI_2;
                        badge.center() + vec2(angle.cos(), angle.sin()) * (size / 2.0 - size * 0.12)
                    })
                    .collect();
                painter.add(Shape::line(points, Stroke::new((size * 0.08).max(1.5), white)));
            } else {
                painter.add(play_shape(badge.center(), size * 0.4, white));
                if response.clicked() {
                    toggle = true;
                }
                response.on_hover_text(if failed.is_some() { "Try again" } else { "Play" });
            }
        }
        if let Some(message) = &failed {
            let at = pos2(rect.center().x, rect.bottom() - 10.0);
            let galley = painter.layout(message.clone(), FontId::proportional(11.0), white, (rect.width() - 16.0).max(40.0));
            let back = Align2::CENTER_BOTTOM.anchor_size(at, galley.size()).expand(4.0);
            painter.rect_filled(back, CornerRadius::same(4), Color32::from_rgba_unmultiplied(160, 30, 30, 230));
            painter.galley(back.min + vec2(4.0, 4.0), galley, white);
        }

        // The bar along the bottom: pause or play, where the clip is, and a track to seek on.
        let started = player.is_some() && failed.is_none() && !loading;
        if started && rect.width() >= MIN_BAR_W && (playing || hover || scrub.is_some()) {
            let bar = Rect::from_min_max(pos2(rect.left(), rect.bottom() - BAR_H), rect.max);
            painter.rect_filled(bar, CornerRadius::ZERO, shade);
            let button = Rect::from_min_size(bar.min, vec2(BAR_H, BAR_H));
            let response = ui.interact(button, ui.id().with(("video_toggle", id)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            if playing {
                pause_shape(&painter, button.center(), 11.0, white);
            } else {
                painter.add(play_shape(button.center(), 11.0, white));
            }
            if response.clicked() {
                toggle = true;
            }
            let duration = clip.duration_ms.max(1);
            let at = scrub.unwrap_or(position).min(duration);
            let label = format!("{} / {}", clock(at), clock(duration));
            let galley = painter.layout_no_wrap(label, FontId::monospace(11.0), white);
            let text_w = galley.size().x;
            painter.galley(pos2(bar.right() - text_w - 10.0, bar.center().y - galley.size().y / 2.0), galley, white);
            let track = Rect::from_min_max(pos2(button.right() + 4.0, bar.center().y - 2.0), pos2(bar.right() - text_w - 22.0, bar.center().y + 2.0));
            if track.width() > 20.0 {
                let hit = track.expand2(vec2(4.0, 10.0));
                let response =
                    ui.interact(hit, ui.id().with(("video_seek", id)), Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::PointingHand);
                painter.rect_filled(track, CornerRadius::same(2), Color32::from_white_alpha(70));
                let x = track.left() + track.width() * (at as f32 / duration as f32);
                painter.rect_filled(Rect::from_min_max(track.min, pos2(x, track.bottom())), CornerRadius::same(2), white);
                painter.circle_filled(pos2(x, track.center().y), if response.hovered() || response.dragged() { 6.0 } else { 4.0 }, white);
                let time_at = |p: Pos2| (((p.x - track.left()) / track.width()).clamp(0.0, 1.0) * duration as f32) as u64;
                if let Some(p) = response.interact_pointer_pos() {
                    if response.dragged() {
                        scrub_to = Some(time_at(p));
                    } else if response.clicked() {
                        seek = Some(time_at(p));
                    }
                }
                if response.drag_stopped() {
                    seek = scrub_to;
                    scrub_end = true;
                }
            }
        }

        if let Some(player) = app.ui.videos.players.get_mut(&id) {
            player.scrub = if scrub_end { None } else { scrub_to };
        }
        if let Some(to) = seek {
            app.ui.videos.seek(id, to);
        }
        if toggle {
            self::toggle(app, &ctx, id, &clip);
        }
    }
}
