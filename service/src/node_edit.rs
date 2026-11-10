//! Video editing on the canvas: `edit.apply`, `media.frames`, `media.probe`.
//!
//! The edit is a file: `.afterburn/edits/<name>.json`, a cut list (clips with in and out points,
//! voice, captions, music). It is the only truth. An agent never moves timeline layers; it sends
//! a cut list to `edit.apply`, which
//!
//! 1. checks it (the gates: sources exist, cuts fit their sources, the length hits the target,
//!    transitions are declared, voice and captions fit), and refuses with every reason at once;
//! 2. renders it with ffmpeg into one clip, and probes the render (length, sound, black);
//! 3. writes the file, stores the render beside the map and redraws the edit's frame on the
//!    canvas: the playable take and a timeline picture. Layers keep their ids from one apply to
//!    the next (a comment stays on its clip), and the node refuses any other edit of them.
//!
//! An agent cannot watch or hear a clip. `media.frames` gives it a contact sheet of frames at
//! chosen times, and `media.probe` the numbers: length, loudness, silence, black.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use afterburn_video::{Clip, VIDEO_KEY};
use base64::Engine as _;
use figcraft_doc::color::{ImageScaleMode, Paint};
use figcraft_doc::{Document, NodeId};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::node_media::Stored;

/// Where edits live, relative to the worktree.
pub const EDITS_DIR: &str = ".afterburn/edits";
/// Plugin data on an edit's frame: the edit's name.
pub const EDIT_KEY: &str = "afterburn.edit";
/// Plugin data on a generated layer inside it: which part of the edit it draws.
pub const ITEM_KEY: &str = "afterburn.edit.item";

const MAX_CLIPS: usize = 200;
const MAX_SECONDS: f64 = 600.0;
/// Canvas units per second of the timeline picture.
const PPS: f64 = 40.0;
const LANE_X: f64 = 120.0;
const SHOT_COLORS: [&str; 6] = [
    "#3E4A57", "#557060", "#7A4A3C", "#4C5480", "#8A6A2C", "#5C5C5C",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub name: String,
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default = "default_fps")]
    pub fps: u32,
    /// The length the edit must have, within one frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_seconds: Option<f64>,
    pub clips: Vec<Cut>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub voice: Vec<Voice>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub captions: Vec<Caption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music: Option<Music>,
}

fn default_width() -> u32 {
    1280
}
fn default_height() -> u32 {
    720
}
fn default_fps() -> u32 {
    30
}

/// One shot: `source` from `in` to `out` seconds.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cut {
    pub id: String,
    /// A file in the worktree.
    pub source: String,
    #[serde(rename = "in")]
    pub start: f64,
    pub out: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Seconds this shot fades in over the end of the one before: the only overlap allowed.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub crossfade: f64,
}

fn is_zero(value: &f64) -> bool {
    *value == 0.0
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Voice {
    pub id: String,
    pub source: String,
    /// When it starts on the timeline.
    pub at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Caption {
    pub text: String,
    pub at: f64,
    pub until: f64,
    /// The voice segment it must sit inside.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Music {
    pub source: String,
    #[serde(default = "default_gain")]
    pub gain_db: f64,
}

fn default_gain() -> f64 {
    -20.0
}

/// A checked edit: sources resolved, every shot placed on the timeline.
pub struct Plan {
    pub edit: Edit,
    clips: Vec<PathBuf>,
    /// Where each shot starts on the timeline.
    pub starts: Vec<f64>,
    pub total: f64,
    voice: Vec<(PathBuf, f64)>,
    music: Option<PathBuf>,
}

fn tool(name: &str) -> Result<()> {
    let found = Command::new(name)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if found {
        Ok(())
    } else {
        Err(Error::new(format!(
            "{name} is not installed on the node's machine: video editing needs ffmpeg and ffprobe"
        )))
    }
}

fn slug(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// A source named in an edit, kept inside the worktree.
fn resolve(worktree: &Path, source: &str) -> std::result::Result<PathBuf, String> {
    let root = worktree
        .canonicalize()
        .map_err(|e| format!("worktree: {e}"))?;
    let path = root
        .join(source)
        .canonicalize()
        .map_err(|_| format!("{source}: no such file in the worktree"))?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err(format!("{source}: not a file inside the worktree"));
    }
    Ok(path)
}

/// A media file's length in seconds.
pub fn duration(path: &Path) -> std::result::Result<f64, String> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("ffprobe: {e}"))?;
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .ok_or_else(|| format!("{}: not a media file ffprobe can read", path.display()))
}

/// The gates. Every reason an edit is refused, at once, so one retry can fix them all.
pub fn check(worktree: &Path, edit: Edit) -> std::result::Result<Plan, Vec<String>> {
    let mut errors = Vec::new();
    let frame = 1.0 / f64::from(edit.fps.max(1));
    if !slug(&edit.name, 40) {
        errors.push("name: 1 to 40 of a-z, 0-9, - and _".to_string());
    }
    if !(160..=3840).contains(&edit.width)
        || !(90..=2160).contains(&edit.height)
        || !edit.width.is_multiple_of(2)
        || !edit.height.is_multiple_of(2)
    {
        errors.push("width x height: even, from 160x90 to 3840x2160".to_string());
    }
    if !(1..=60).contains(&edit.fps) {
        errors.push("fps: 1 to 60".to_string());
    }
    if edit.clips.is_empty() || edit.clips.len() > MAX_CLIPS {
        errors.push(format!("clips: 1 to {MAX_CLIPS}"));
    }
    let mut seen = BTreeSet::new();
    let mut clips = Vec::new();
    let mut starts = Vec::new();
    let mut total = 0.0f64;
    let mut previous = 0.0f64;
    for (index, cut) in edit.clips.iter().enumerate() {
        let at = format!("clips[{index}] ({})", cut.id);
        if !slug(&cut.id, 40) || !seen.insert(cut.id.clone()) {
            errors.push(format!(
                "{at}: id must be unique, 1 to 40 of a-z, 0-9, - and _"
            ));
        }
        let numbers = [cut.start, cut.out, cut.crossfade];
        if numbers.iter().any(|n| !n.is_finite() || *n < 0.0) || cut.out - cut.start < 2.0 * frame {
            errors.push(format!(
                "{at}: needs 0 <= in < out (at least two frames) and crossfade >= 0"
            ));
            continue;
        }
        let length = cut.out - cut.start;
        match resolve(worktree, &cut.source) {
            Ok(path) => {
                match duration(&path) {
                    Ok(source) if cut.out > source + frame => errors.push(format!(
                        "{at}: out {:.3}s is past the end of {} ({source:.3}s)",
                        cut.out, cut.source
                    )),
                    Ok(_) => {}
                    Err(e) => errors.push(format!("{at}: {e}")),
                }
                clips.push(path);
            }
            Err(e) => errors.push(format!("{at}: {e}")),
        }
        // A crossfade is the one declared overlap; anything else is end to start.
        if index == 0 && cut.crossfade > 0.0 {
            errors.push(format!("{at}: the first shot has nothing to fade from"));
        } else if cut.crossfade > 0.0 && (cut.crossfade >= length || cut.crossfade >= previous) {
            errors.push(format!(
                "{at}: crossfade {:.2}s must be shorter than both shots",
                cut.crossfade
            ));
        }
        let start = if index == 0 {
            0.0
        } else {
            (total - cut.crossfade).max(0.0)
        };
        starts.push(start);
        total = start + length;
        previous = length;
    }
    if total > MAX_SECONDS {
        errors.push(format!("the edit is {total:.1}s; at most {MAX_SECONDS}s"));
    }
    if let Some(target) = edit.target_seconds {
        if (total - target).abs() > frame + 1e-6 {
            errors.push(format!(
                "length: the shots add up to {total:.3}s; the target is {target:.3}s within one frame ({frame:.3}s)"
            ));
        }
    }
    // Voice: each segment inside the edit, none over another.
    let mut voice = Vec::new();
    let mut spans: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    let mut voice_end = 0.0f64;
    for (index, segment) in edit.voice.iter().enumerate() {
        let at = format!("voice[{index}] ({})", segment.id);
        if !slug(&segment.id, 40) || spans.contains_key(&segment.id) {
            errors.push(format!(
                "{at}: id must be unique, 1 to 40 of a-z, 0-9, - and _"
            ));
        }
        if !segment.at.is_finite() || segment.at < 0.0 {
            errors.push(format!("{at}: at must be 0 or more"));
            continue;
        }
        match resolve(worktree, &segment.source).and_then(|path| duration(&path).map(|d| (path, d)))
        {
            Ok((path, length)) => {
                if segment.at + length > total + frame {
                    errors.push(format!(
                        "{at}: runs to {:.2}s, past the end of the edit ({total:.2}s)",
                        segment.at + length
                    ));
                }
                if segment.at < voice_end - 1e-6 {
                    errors.push(format!(
                        "{at}: starts at {:.2}s while the voice before runs to {voice_end:.2}s",
                        segment.at
                    ));
                }
                voice_end = segment.at + length;
                spans.insert(segment.id.clone(), (segment.at, segment.at + length));
                voice.push((path, length));
            }
            Err(e) => errors.push(format!("{at}: {e}")),
        }
    }
    let mut caption_end = 0.0f64;
    for (index, caption) in edit.captions.iter().enumerate() {
        let at = format!("captions[{index}]");
        if caption.text.trim().is_empty() || caption.text.chars().count() > 200 {
            errors.push(format!("{at}: text of 1 to 200 characters"));
        }
        if !(caption.at.is_finite() && caption.until.is_finite())
            || caption.at < 0.0
            || caption.until <= caption.at
            || caption.until > total + frame
        {
            errors.push(format!("{at}: needs 0 <= at < until <= {total:.2}s"));
            continue;
        }
        if caption.at < caption_end - 1e-6 {
            errors.push(format!("{at}: starts before the caption before it ends"));
        }
        caption_end = caption.until;
        if let Some(id) = &caption.voice {
            match spans.get(id) {
                Some((from, to)) if caption.at >= from - 0.05 && caption.until <= to + 0.05 => {}
                Some((from, to)) => errors.push(format!(
                    "{at}: {:.2}-{:.2}s is outside its voice {id} ({from:.2}-{to:.2}s)",
                    caption.at, caption.until
                )),
                None => errors.push(format!("{at}: no voice segment {id}")),
            }
        }
    }
    let mut music = None;
    if let Some(bed) = &edit.music {
        if !bed.gain_db.is_finite() || !(-60.0..=6.0).contains(&bed.gain_db) {
            errors.push("music: gain_db from -60 to 6".to_string());
        }
        match resolve(worktree, &bed.source).and_then(|path| duration(&path).map(|_| path)) {
            Ok(path) => music = Some(path),
            Err(e) => errors.push(format!("music: {e}")),
        }
    }
    if errors.is_empty() {
        Ok(Plan {
            edit,
            clips,
            starts,
            total,
            voice,
            music,
        })
    } else {
        Err(errors)
    }
}

fn font() -> Option<&'static str> {
    [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
    ]
    .into_iter()
    .find(|path| Path::new(path).is_file())
}

/// Render the plan into `target`: Constrained Baseline H.264 and AAC, the format the app plays.
/// `scratch` holds the caption text files while ffmpeg runs.
pub fn render(plan: &Plan, scratch: &Path, target: &Path) -> Result<()> {
    let edit = &plan.edit;
    let (w, h, fps) = (edit.width, edit.height, edit.fps);
    let mut command = Command::new("ffmpeg");
    command.args(["-nostdin", "-loglevel", "error", "-y"]);
    for path in &plan.clips {
        command.arg("-i").arg(path);
    }
    for (path, _) in &plan.voice {
        command.arg("-i").arg(path);
    }
    if let Some(path) = &plan.music {
        command.args(["-stream_loop", "-1", "-i"]).arg(path);
    }
    let mut graph = Vec::new();
    for (index, cut) in edit.clips.iter().enumerate() {
        graph.push(format!(
            "[{index}:v]trim=start={:.4}:end={:.4},setpts=PTS-STARTPTS,fps={fps},scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=0x101010,setsar=1,format=yuv420p[s{index}]",
            cut.start, cut.out
        ));
    }
    let mut video = "s0".to_string();
    for (index, cut) in edit.clips.iter().enumerate().skip(1) {
        let next = format!("j{index}");
        if cut.crossfade > 0.0 {
            graph.push(format!(
                "[{video}][s{index}]xfade=transition=fade:duration={:.4}:offset={:.4}[{next}]",
                cut.crossfade,
                plan.starts.get(index).copied().unwrap_or(0.0)
            ));
        } else {
            graph.push(format!("[{video}][s{index}]concat=n=2:v=1:a=0[{next}]"));
        }
        video = next;
    }
    for (index, caption) in edit.captions.iter().enumerate() {
        let file = scratch.join(format!("caption-{index}.txt"));
        std::fs::write(&file, caption.text.trim())?;
        // The path goes through the filter parser: escape what it reads as syntax.
        let file = file
            .display()
            .to_string()
            .replace('\\', "/")
            .replace(':', "\\:")
            .replace('\'', "\\'");
        let face = font().map(|f| format!("fontfile={f}:")).unwrap_or_default();
        let next = format!("c{index}");
        graph.push(format!(
            "[{video}]drawtext={face}textfile='{file}':enable='between(t,{:.3},{:.3})':x=(w-text_w)/2:y=h-text_h-{}:fontsize={}:fontcolor=white:box=1:boxcolor=black@0.6:boxborderw={}[{next}]",
            caption.at,
            caption.until,
            h / 12,
            h / 22,
            h / 60
        ));
        video = next;
    }
    // Sound: silence the length of the edit, the voice segments at their times, the music under.
    graph.push(format!(
        "anullsrc=r=48000:cl=stereo,atrim=0:{:.4}[quiet]",
        plan.total
    ));
    let mut mix = vec!["[quiet]".to_string()];
    let base = edit.clips.len();
    for (index, segment) in edit.voice.iter().enumerate() {
        graph.push(format!(
            "[{}:a]aresample=48000,aformat=channel_layouts=stereo,adelay={}:all=1[v{index}]",
            base + index,
            (segment.at * 1000.0).round() as u64
        ));
        mix.push(format!("[v{index}]"));
    }
    if let Some(bed) = &edit.music {
        let fade = (plan.total - 1.5).max(0.0);
        graph.push(format!(
            "[{}:a]aresample=48000,aformat=channel_layouts=stereo,volume={:.2}dB,atrim=0:{:.4},afade=t=out:st={fade:.4}:d=1.5[bed]",
            base + edit.voice.len(),
            bed.gain_db,
            plan.total
        ));
        mix.push("[bed]".to_string());
    }
    graph.push(format!(
        "{}amix=inputs={}:normalize=0:duration=first[sound]",
        mix.concat(),
        mix.len()
    ));
    let output = command
        .arg("-filter_complex")
        .arg(graph.join(";"))
        .args([
            "-map",
            &format!("[{video}]"),
            "-map",
            "[sound]",
            "-t",
            &format!("{:.4}", plan.total),
        ])
        .args([
            "-c:v",
            "libx264",
            "-profile:v",
            "baseline",
            "-pix_fmt",
            "yuv420p",
            "-crf",
            "23",
        ])
        .args([
            "-g",
            &fps.to_string(),
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ])
        .arg(target)
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        let said = said.lines().last().unwrap_or("no message");
        return Err(Error::new(format!(
            "ffmpeg could not render the edit: {said}"
        )));
    }
    Ok(())
}

/// What a machine can tell about a clip without watching it.
#[derive(Clone, Debug, Serialize)]
pub struct Probe {
    pub seconds: f64,
    pub audio: bool,
    /// Integrated loudness (EBU R128), when there is sound.
    pub loudness_lufs: Option<f64>,
    /// Seconds of silence (under -50 dB for half a second or more).
    pub silence_seconds: f64,
    /// Seconds of black picture.
    pub black_seconds: f64,
}

/// Sum the `key: <seconds>` values ffmpeg's detectors print.
fn sum_after(text: &str, key: &str) -> f64 {
    text.match_indices(key)
        .filter_map(|(at, _)| {
            text.get(at + key.len()..)?
                .split_whitespace()
                .next()?
                .parse::<f64>()
                .ok()
        })
        .filter(|value| value.is_finite())
        .fold(0.0, |sum, value| sum + value)
}

pub fn probe(path: &Path) -> Result<Probe> {
    let seconds = duration(path).map_err(Error::new)?;
    let streams = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a",
            "-show_entries",
            "stream=index",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .output()?;
    let audio = !String::from_utf8_lossy(&streams.stdout).trim().is_empty();
    let mut command = Command::new("ffmpeg");
    command.args(["-nostdin", "-hide_banner", "-i"]).arg(path);
    command.args(["-vf", "blackdetect=d=0.1:pix_th=0.10"]);
    if audio {
        command.args(["-af", "silencedetect=noise=-50dB:d=0.5,ebur128"]);
    }
    let output = command
        .args(["-f", "null", "-"])
        .stdin(Stdio::null())
        .output()?;
    let said = String::from_utf8_lossy(&output.stderr);
    // The loudness summary is the last `I: <n> LUFS` ffmpeg prints.
    let loudness_lufs = said
        .rfind("Integrated loudness:")
        .and_then(|at| said.get(at..))
        .and_then(|rest| rest.split("I:").nth(1))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite());
    Ok(Probe {
        seconds,
        audio,
        loudness_lufs,
        silence_seconds: sum_after(&said, "silence_duration:"),
        black_seconds: sum_after(&said, "black_duration:"),
    })
}

/// The gates on the render itself.
pub fn check_render(plan: &Plan, probe: &Probe) -> std::result::Result<(), Vec<String>> {
    let mut errors = Vec::new();
    let frame = 1.0 / f64::from(plan.edit.fps.max(1));
    if (probe.seconds - plan.total).abs() > 3.0 * frame {
        errors.push(format!(
            "the render is {:.3}s; the edit is {:.3}s",
            probe.seconds, plan.total
        ));
    }
    if !probe.audio {
        errors.push("the render has no sound track".to_string());
    }
    if probe.black_seconds > plan.total * 0.5 {
        errors.push(format!(
            "{:.1}s of the render's {:.1}s is black",
            probe.black_seconds, plan.total
        ));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Check, render and store an edit. Nothing is written when a gate refuses it.
pub fn build(worktree: &Path, raw: &Value) -> Result<(Plan, Stored, Probe)> {
    tool("ffmpeg")?;
    tool("ffprobe")?;
    let edit: Edit = serde_json::from_value(raw.clone())
        .map_err(|e| Error::new(format!("edit.apply: the edit is not a cut list: {e}")))?;
    let refused =
        |errors: Vec<String>| Error::new(format!("edit refused:\n- {}", errors.join("\n- ")));
    let plan = check(worktree, edit).map_err(refused)?;
    let dir = worktree.join(EDITS_DIR);
    std::fs::create_dir_all(&dir)?;
    let scratch = dir.join(format!(".render-{}-{}", plan.edit.name, std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch)?;
    let take = scratch.join("take.mp4");
    let built: Result<(Stored, Probe)> = (|| {
        render(&plan, &scratch, &take)?;
        let probe = probe(&take)?;
        check_render(&plan, &probe).map_err(refused)?;
        let stored = crate::node_media::store(worktree, &take)?;
        Ok((stored, probe))
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    let (stored, probe) = built?;
    // Only an edit that passed every gate becomes the file.
    let file = dir.join(format!("{}.json", plan.edit.name));
    let partial = dir.join(format!(".{}.json.part", plan.edit.name));
    std::fs::write(
        &partial,
        format!("{}\n", serde_json::to_string_pretty(&plan.edit)?),
    )?;
    std::fs::rename(&partial, &file)?;
    Ok((plan, stored, probe))
}

// ---------------------------------------------------------------------------------------------
// The canvas: the edit's frame, drawn from the plan.

fn engine_error(e: figcraft_engine::EngineError) -> Error {
    Error::new(e.to_string())
}

/// The frame of the edit named `name`, if the map has one.
fn frame_of(doc: &Document, name: &str) -> Option<NodeId> {
    doc.pages
        .iter()
        .flat_map(|page| doc.subtree(*page))
        .find(|id| {
            doc.node(*id)
                .and_then(|node| node.plugin_data.get(EDIT_KEY))
                .map(String::as_str)
                == Some(name)
        })
}

/// Whether the map already has the frame of the edit named `name`.
pub fn has_frame(doc: &Document, name: &str) -> bool {
    frame_of(doc, name).is_some()
}

/// A layer of the last apply: where it hangs and what kind it is.
struct Old {
    id: u64,
    parent: u64,
    kind: &'static str,
}

struct Draw<'a> {
    engine: &'a mut figcraft_engine::Session,
    root: u64,
    /// The layers of the last apply, by what they draw.
    old: BTreeMap<String, Old>,
}

/// A box of the timeline: a frame, so the map selects it as one card, with its words inside.
struct Block<'a> {
    key: &'a str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    fill: &'a str,
}

impl Draw<'_> {
    /// Make or update the layer that draws `key` under `parent`; it keeps its id across applies.
    /// A layer of another kind or under another parent (an older layout) is replaced.
    #[allow(clippy::too_many_arguments)]
    fn item(
        &mut self,
        parent: u64,
        key: &str,
        kind: &'static str,
        x: f64,
        y: f64,
        size: Option<(f64, f64)>,
        mut props: Value,
    ) -> Result<u64> {
        if let Some((w, h)) = size {
            props["width"] = json!(w.max(1.0));
            props["height"] = json!(h.max(1.0));
        }
        let same = self
            .old
            .get(key)
            .is_some_and(|old| old.kind == kind && old.parent == parent);
        if same {
            if let Some(old) = self.old.remove(key) {
                props["ids"] = json!([old.id]);
                props["x"] = json!(x);
                props["y"] = json!(y);
                self.engine
                    .execute("node.set", &props)
                    .map_err(engine_error)?;
                return Ok(old.id);
            }
        }
        let mut params =
            json!({"type": kind, "parent": parent, "name": key, "x": x, "y": y, "select": false});
        for field in ["width", "height"] {
            if let Some(value) = props.get(field).cloned() {
                params[field] = value;
            }
        }
        if kind == "text" {
            params["text"] = props["characters"].take();
        }
        if let Some(object) = props.as_object_mut() {
            object.remove("characters");
            object.remove("width");
            object.remove("height");
        }
        props["pluginData"] = json!({ITEM_KEY: key});
        params["props"] = props;
        let made = self
            .engine
            .execute("node.create", &params)
            .map_err(engine_error)?;
        made["id"]
            .as_u64()
            .ok_or_else(|| Error::new("node.create returned no id"))
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        parent: u64,
        key: &str,
        x: f64,
        y: f64,
        text: &str,
        size: f64,
        color: &str,
        mono: bool,
    ) -> Result<u64> {
        let family = if mono { "JetBrains Mono" } else { "Inter" };
        self.item(
            parent,
            key,
            "text",
            x,
            y,
            None,
            json!({"characters": text, "fontFamily": family, "fontSize": size, "fill": color}),
        )
    }

    /// Words that wrap inside `width`: all of them show.
    #[allow(clippy::too_many_arguments)]
    fn wrapped(
        &mut self,
        parent: u64,
        key: &str,
        x: f64,
        y: f64,
        width: f64,
        text: &str,
        color: &str,
    ) -> Result<u64> {
        self.item(
            parent,
            key,
            "text",
            x,
            y,
            None,
            json!({
                "characters": text, "fontFamily": "Inter", "fontSize": WORDS_SIZE, "fill": color,
                "width": width.max(8.0), "textAutoResize": "height",
            }),
        )
    }

    fn rect(&mut self, key: &str, x: f64, y: f64, w: f64, h: f64, fill: &str) -> Result<u64> {
        self.item(
            self.root,
            key,
            "rectangle",
            x,
            y,
            Some((w, h)),
            json!({"fill": fill, "cornerRadius": 4.0}),
        )
    }

    fn block(&mut self, block: &Block<'_>) -> Result<u64> {
        self.item(
            self.root,
            block.key,
            "frame",
            block.x,
            block.y,
            Some((block.w, block.h)),
            json!({"fill": block.fill, "cornerRadius": 4.0, "clip": true}),
        )
    }
}

const WORDS_SIZE: f64 = 9.0;

/// How tall `text` stands when wrapped inside `width`, with room to spare: a block must hold all
/// of its words.
fn words_height(text: &str, width: f64) -> f64 {
    let per_line = ((width - 10.0) / (WORDS_SIZE * 0.6)).floor().max(1.0);
    // Wrapping breaks at words, so lines run short of the box: count each word's own length.
    let mut lines = 1.0f64;
    let mut used = 0.0f64;
    for word in text.split_whitespace() {
        let length = word.chars().count() as f64;
        if used > 0.0 && used + 1.0 + length > per_line {
            lines += 1.0 + (length / per_line).floor();
            used = length % per_line;
        } else {
            used += length + if used > 0.0 { 1.0 } else { 0.0 };
        }
    }
    lines * (WORDS_SIZE * 1.35).ceil() + 10.0
}

/// Draw the edit on the canvas: the take (playable) and the timeline picture under it. The first
/// apply needs `parent` (a page or card id) and takes `x`, `y`; later ones redraw in place.
///
/// Every shot, voice segment, caption and the music is a frame of its own with its words inside,
/// so the map selects each as one card and a comment lands on it.
pub fn place(
    engine: &mut figcraft_engine::Session,
    plan: &Plan,
    stored: &Stored,
    probe: &Probe,
    req: &Value,
) -> Result<Value> {
    let edit = &plan.edit;
    let width = (LANE_X + plan.total * PPS + 32.0).max(720.0);
    let take_w = 480.0f64;
    let take_h = (take_w * f64::from(edit.height) / f64::from(edit.width.max(1))).round();
    // Lanes are as tall as their tallest block: all the words of a voice line or caption show.
    let voice_words: Vec<&str> = edit
        .voice
        .iter()
        .map(|segment| segment.text.as_deref().unwrap_or(&segment.id))
        .collect();
    let voice_h = voice_words
        .iter()
        .enumerate()
        .map(|(index, words)| {
            words_height(
                words,
                plan.voice.get(index).map_or(0.0, |v| v.1) * PPS - 1.0,
            )
        })
        .fold(28.0, f64::max);
    let caption_h = edit
        .captions
        .iter()
        .map(|caption| words_height(&caption.text, (caption.until - caption.at) * PPS - 1.0))
        .fold(28.0, f64::max);
    let heights = [44.0, voice_h, caption_h, 28.0];
    let lanes_y = 84.0 + take_h + 40.0;
    let lane = |index: usize| lanes_y + heights.iter().take(index).map(|h| h + 10.0).sum::<f64>();
    let ruler = lane(4);
    let height = ruler + 64.0;
    let existing = frame_of(engine.doc().map_err(engine_error)?, &edit.name);
    let root = match existing {
        Some(id) => {
            engine
                .execute(
                    "node.set",
                    &json!({"ids": [id.0], "width": width, "height": height}),
                )
                .map_err(engine_error)?;
            id.0
        }
        None => {
            let parent = crate::node_media::parent(req).map_err(|_| {
                Error::new("edit.apply needs parent the first time: the id of the page or card to draw the edit on")
            })?;
            let made = engine
                .execute(
                    "node.create",
                    &json!({
                        "type": "frame", "parent": parent, "name": format!("Edit {}", edit.name),
                        "x": req.get("x").and_then(Value::as_f64).unwrap_or(0.0),
                        "y": req.get("y").and_then(Value::as_f64).unwrap_or(0.0),
                        "width": width, "height": height, "select": false,
                        "props": {"fill": "#FFFFFF", "stroke": "#E3E2DF", "strokeWeight": 1, "cornerRadius": 10, "pluginData": {EDIT_KEY: edit.name}},
                    }),
                )
                .map_err(engine_error)?;
            made["id"]
                .as_u64()
                .ok_or_else(|| Error::new("node.create returned no id"))?
        }
    };
    let doc = engine.doc().map_err(engine_error)?;
    let old: BTreeMap<String, Old> = doc
        .subtree(NodeId(root))
        .into_iter()
        .filter_map(|id| {
            let node = doc.node(id)?;
            let kind = match node.kind {
                figcraft_doc::NodeKind::Frame => "frame",
                figcraft_doc::NodeKind::Rectangle => "rectangle",
                _ if node.kind.text().is_some() => "text",
                _ => "other",
            };
            Some((
                node.plugin_data.get(ITEM_KEY)?.clone(),
                Old {
                    id: id.0,
                    parent: node.parent?.0,
                    kind,
                },
            ))
        })
        .collect();
    let mut draw = Draw { engine, root, old };
    let (ink, grey, red) = ("#1A1A1A", "#6E6C68", "#B5452F");
    draw.text(
        root,
        "eyebrow",
        32.0,
        25.0,
        &format!("EDIT · .afterburn/edits/{}.json", edit.name),
        10.0,
        red,
        true,
    )?;
    draw.text(
        root,
        "title",
        32.0,
        46.0,
        &format!(
            "{} · {:.1}s · {}×{} · {} shots",
            edit.name,
            plan.total,
            edit.width,
            edit.height,
            edit.clips.len()
        ),
        20.0,
        ink,
        false,
    )?;
    // The take: the render, playable in place. Same layer every apply; its picture and clip change.
    let take = draw.item(
        root,
        "take",
        "rectangle",
        32.0,
        84.0,
        Some((take_w, take_h)),
        json!({"fill": "#101010", "cornerRadius": 0.0}),
    )?;
    let numbers = format!(
        "length    {:.2}s\nloudness  {}\nsilence   {:.1}s\nblack     {:.1}s",
        probe.seconds,
        probe
            .loudness_lufs
            .map_or("no sound".to_string(), |l| format!("{l:.1} LUFS")),
        probe.silence_seconds,
        probe.black_seconds
    );
    draw.text(
        root,
        "probe label",
        32.0 + take_w + 24.0,
        84.0,
        "THE RENDER, MEASURED",
        10.0,
        grey,
        true,
    )?;
    draw.text(
        root,
        "probe",
        32.0 + take_w + 24.0,
        104.0,
        &numbers,
        11.0,
        "#507A62",
        true,
    )?;
    // The lanes.
    for (index, label) in ["SHOTS", "VOICE", "CAPTIONS", "MUSIC"].iter().enumerate() {
        let tall = heights.get(index).copied().unwrap_or(28.0);
        draw.text(
            root,
            &format!("lane {label}"),
            32.0,
            lane(index) + tall / 2.0 - 6.0,
            label,
            10.0,
            grey,
            true,
        )?;
        draw.rect(
            &format!("lane bg {label}"),
            LANE_X,
            lane(index) - 4.0,
            plan.total * PPS,
            tall + 8.0,
            "#F4F3F0",
        )?;
    }
    for (index, cut) in edit.clips.iter().enumerate() {
        let x = LANE_X + plan.starts.get(index).copied().unwrap_or(0.0) * PPS;
        let w = (cut.out - cut.start) * PPS - 1.0;
        let color = SHOT_COLORS
            .get(index % SHOT_COLORS.len())
            .copied()
            .unwrap_or("#5C5C5C");
        let key = format!("shot {}", cut.id);
        let shot = draw.block(&Block {
            key: &key,
            x,
            y: lane(0),
            w,
            h: 44.0,
            fill: color,
        })?;
        draw.text(
            shot,
            &format!("shot title {}", cut.id),
            6.0,
            6.0,
            cut.title.as_deref().unwrap_or(&cut.id),
            11.0,
            "#FFFFFF",
            false,
        )?;
        draw.text(
            shot,
            &format!("shot length {}", cut.id),
            6.0,
            25.0,
            &format!(
                "{:.1}s · {:.1}–{:.1}",
                cut.out - cut.start,
                cut.start,
                cut.out
            ),
            9.0,
            "#FFFFFFB0",
            true,
        )?;
    }
    for (index, segment) in edit.voice.iter().enumerate() {
        let length = plan.voice.get(index).map_or(0.0, |v| v.1);
        let (x, w) = (LANE_X + segment.at * PPS, length * PPS - 1.0);
        let key = format!("voice {}", segment.id);
        let block = draw.block(&Block {
            key: &key,
            x,
            y: lane(1),
            w,
            h: voice_h,
            fill: "#CBDCD1",
        })?;
        let words = voice_words.get(index).copied().unwrap_or("");
        draw.wrapped(
            block,
            &format!("voice text {}", segment.id),
            5.0,
            5.0,
            w - 10.0,
            words,
            "#33513F",
        )?;
    }
    for (index, caption) in edit.captions.iter().enumerate() {
        let (x, w) = (
            LANE_X + caption.at * PPS,
            (caption.until - caption.at) * PPS - 1.0,
        );
        let key = format!("caption {index}");
        let block = draw.block(&Block {
            key: &key,
            x,
            y: lane(2),
            w,
            h: caption_h,
            fill: "#E3E2DF",
        })?;
        draw.wrapped(
            block,
            &format!("caption text {index}"),
            5.0,
            5.0,
            w - 10.0,
            &caption.text,
            ink,
        )?;
    }
    if let Some(bed) = &edit.music {
        let block = draw.block(&Block {
            key: "music",
            x: LANE_X,
            y: lane(3),
            w: plan.total * PPS,
            h: 28.0,
            fill: "#B9B0D6",
        })?;
        draw.text(
            block,
            "music text",
            8.0,
            8.0,
            &format!("{} · {:.0} dB", bed.source, bed.gain_db),
            9.0,
            "#2F2850",
            true,
        )?;
    }
    let step = if plan.total > 120.0 {
        30
    } else if plan.total > 40.0 {
        10
    } else {
        5
    };
    for second in (0..=plan.total.floor() as u64).step_by(step) {
        let x = LANE_X + second as f64 * PPS;
        draw.item(
            root,
            &format!("tick {second}"),
            "rectangle",
            x,
            ruler,
            Some((1.0, 6.0)),
            json!({"fill": "#B9B7B2", "cornerRadius": 0.0}),
        )?;
        draw.text(
            root,
            &format!("tick text {second}"),
            x - 4.0,
            ruler + 9.0,
            &format!("{second}s"),
            9.0,
            grey,
            true,
        )?;
    }
    draw.text(
        root,
        "foot",
        32.0,
        ruler + 34.0,
        "Generated by edit.apply from the file above. These layers are locked: change the cut list.",
        10.0,
        grey,
        true,
    )?;
    // What the last apply drew and this one does not goes away (children leave with their box).
    let gone: BTreeSet<u64> = draw.old.values().map(|old| old.id).collect();
    let tops: Vec<u64> = draw
        .old
        .values()
        .filter(|old| !gone.contains(&old.parent))
        .map(|old| old.id)
        .collect();
    if !tops.is_empty() {
        draw.engine
            .execute("edit.delete", &json!({"ids": tops}))
            .map_err(engine_error)?;
    }
    // The take's picture and clip: set on the document, the layer stays.
    let doc = engine.doc_mut().map_err(engine_error)?;
    let (hash, _, _) = figcraft_format::add_image(doc, stored.poster_jpeg.clone())
        .map_err(|e| Error::new(e.to_string()))?;
    let tag = serde_json::to_string(&stored.clip)?;
    let node = doc
        .get_mut(NodeId(take))
        .map_err(|e| Error::new(e.to_string()))?;
    node.fills = vec![Paint::image(hash, ImageScaleMode::Fill)];
    node.plugin_data.insert(VIDEO_KEY.to_string(), tag);
    Ok(json!({
        "frame": root, "take": take, "video": stored.clip, "seconds": plan.total, "probe": probe,
        "file": format!("{EDITS_DIR}/{}.json", edit.name),
    }))
}

/// Renders no card names any more: takes of edits cost a file each, so only the kept one stays.
pub fn drop_unused_takes(worktree: &Path, doc: &Document, was: Option<&str>) {
    let Some(was) = was.filter(|name| afterburn_video::valid_file_name(name)) else {
        return;
    };
    let used = doc
        .pages
        .iter()
        .flat_map(|page| doc.subtree(*page))
        .any(|id| {
            doc.node(id)
                .and_then(|node| node.plugin_data.get(VIDEO_KEY))
                .and_then(|tag| Clip::parse(tag))
                .is_some_and(|clip| clip.file == was)
        });
    if !used {
        let _ = std::fs::remove_file(worktree.join(afterburn_video::MEDIA_DIR).join(was));
    }
}

/// The take an edit's frame shows now.
pub fn current_take(doc: &Document, name: &str) -> Option<String> {
    let root = frame_of(doc, name)?;
    doc.node(root)?.children.iter().find_map(|id| {
        let node = doc.node(*id)?;
        (node.plugin_data.get(ITEM_KEY).map(String::as_str) == Some("take"))
            .then(|| Clip::parse(node.plugin_data.get(VIDEO_KEY)?))?
            .map(|clip| clip.file)
    })
}

/// The lock: a layer `edit.apply` generated is changed only by `edit.apply`. Returns the reason
/// when `params` (of any command, batches included) names such a layer or puts one inside.
pub fn locked(doc: &Document, command: &str, params: &Value) -> Option<String> {
    if command == "command.batch" {
        return params.get("commands")?.as_array()?.iter().find_map(|step| {
            locked(
                doc,
                step.get("command")?.as_str()?,
                step.get("params").unwrap_or(&Value::Null),
            )
        });
    }
    if command.starts_with("select.") {
        return None;
    }
    // The edit whose frame holds `id` (a layer at any depth inside it; not the frame itself).
    let edit_above = |id: u64| {
        let mut at = doc.node(NodeId(id))?.parent;
        for _ in 0..64 {
            let node = doc.node(at?)?;
            if let Some(name) = node.plugin_data.get(EDIT_KEY) {
                return Some(name.clone());
            }
            at = node.parent;
        }
        None
    };
    let mut ids: Vec<u64> = params
        .get("ids")
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_u64).collect())
        .unwrap_or_default();
    ids.extend(params.get("id").and_then(Value::as_u64));
    if let Some(name) = ids.into_iter().find_map(edit_above) {
        return Some(format!(
            "this layer is generated from {EDITS_DIR}/{name}.json and is locked: change the cut list and call edit.apply"
        ));
    }
    // Nothing is added by hand inside an edit's frame either.
    let parent = params.get("parent").and_then(Value::as_u64)?;
    let name = doc
        .node(NodeId(parent))?
        .plugin_data
        .get(EDIT_KEY)
        .cloned()
        .or_else(|| edit_above(parent))?;
    Some(format!(
        "the frame of edit {name} holds only what edit.apply draws: put your layer beside it"
    ))
}

// ---------------------------------------------------------------------------------------------
// Frames: what an agent can look at.

/// The clip a request names: `file` (a clip's file name) or `node` (a card that shows one).
pub fn clip_path(worktree: &Path, doc: &Document, req: &Value) -> Result<PathBuf> {
    let file = match req.get("node").and_then(Value::as_u64) {
        Some(id) => doc
            .node(NodeId(id))
            .and_then(|node| node.plugin_data.get(VIDEO_KEY))
            .and_then(|tag| Clip::parse(tag))
            .map(|clip| clip.file)
            .ok_or_else(|| Error::new(format!("layer {id} shows no video clip")))?,
        None => req
            .get("file")
            .and_then(Value::as_str)
            .filter(|name| afterburn_video::valid_file_name(name))
            .map(str::to_string)
            .ok_or_else(|| {
                Error::new("pass node (a video card's id) or file (a clip's file name)")
            })?,
    };
    let path = worktree.join(afterburn_video::MEDIA_DIR).join(&file);
    if !path.is_file() {
        return Err(Error::new(format!(
            "the clip {file} is not in {}",
            afterburn_video::MEDIA_DIR
        )));
    }
    Ok(path)
}

/// A contact sheet: the clip's frames at `times` (seconds; default: `count` evenly spread, 6), in
/// reading order, three to a row.
pub fn frames(path: &Path, req: &Value) -> Result<Value> {
    let mut reader = afterburn_video::Reader::open(path).map_err(Error::new)?;
    let length = reader.duration_ms();
    let mut times: Vec<u64> = match req.get("times").and_then(Value::as_array) {
        Some(times) => times
            .iter()
            .filter_map(Value::as_f64)
            .filter(|t| t.is_finite() && *t >= 0.0)
            .map(|t| ((t * 1000.0) as u64).min(length))
            .collect(),
        None => {
            let count = req
                .get("count")
                .and_then(Value::as_u64)
                .unwrap_or(6)
                .clamp(1, 24);
            (0..count)
                .map(|i| length * (2 * i + 1) / (2 * count))
                .collect()
        }
    };
    times.truncate(24);
    if times.is_empty() {
        return Err(Error::new("media.frames: no usable time"));
    }
    let columns = times.len().min(3) as u32;
    let rows = (times.len() as u32).div_ceil(columns);
    let thumb_w = 384u32;
    let thumb_h = (thumb_w * reader.height() / reader.width().max(1)).max(1);
    let gap = 6u32;
    let mut sheet = image::RgbaImage::from_pixel(
        columns * thumb_w + (columns + 1) * gap,
        rows * thumb_h + (rows + 1) * gap,
        image::Rgba([24, 24, 24, 255]),
    );
    let mut shown = Vec::new();
    for (index, time) in times.iter().enumerate() {
        reader.seek(*time).map_err(Error::new)?;
        let mut frame = None;
        while let Some(next) = reader.next_frame().map_err(Error::new)? {
            let reached = next.time_ms >= *time;
            frame = Some(next);
            if reached {
                break;
            }
        }
        let Some(frame) = frame else { continue };
        shown.push(frame.time_ms as f64 / 1000.0);
        let Some(picture) = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)
        else {
            continue;
        };
        let thumb = image::imageops::resize(
            &picture,
            thumb_w,
            thumb_h,
            image::imageops::FilterType::Triangle,
        );
        let (column, row) = (index as u32 % columns, index as u32 / columns);
        image::imageops::overlay(
            &mut sheet,
            &thumb,
            i64::from(gap + column * (thumb_w + gap)),
            i64::from(gap + row * (thumb_h + gap)),
        );
    }
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(sheet)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| Error::new(e.to_string()))?;
    Ok(json!({
        "data": base64::engine::general_purpose::STANDARD.encode(&png),
        "seconds": shown,
        "columns": columns,
        "order": "left to right, top to bottom",
    }))
}

#[cfg(test)]
mod tests;
