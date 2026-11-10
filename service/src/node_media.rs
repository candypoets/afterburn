//! Video clips on the canvas: `media.add` and `media.read`.
//!
//! A clip is a file in the worktree's `.afterburn/media/`, named by its SHA-256. The map holds
//! only a card filled with the clip's first frame and tagged with `afterburn.video` plugin data;
//! the app fetches the file from the node in chunks (`media.read`), so a clip never has to fit
//! one RPC frame and never rides inside `map.figcraft`.
//!
//! The app's decoder plays Constrained Baseline H.264. `media.add` decodes the whole clip once as
//! the proof; a clip that fails is converted with ffmpeg when ffmpeg is installed, else refused.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use afterburn_video::{Checked, Clip, MAX_CLIP_BYTES, MEDIA_DIR, VIDEO_KEY};
use base64::Engine as _;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// The most bytes one `media.read` returns: far under the 16 MiB RPC frame once base64-encoded.
pub const MAX_CHUNK: u64 = 4 * 1024 * 1024;

/// The widest card `media.add` makes when no width is given.
const DEFAULT_WIDTH: f64 = 640.0;

/// A clip ready to place: its file is in the worktree's media directory.
pub struct Stored {
    pub clip: Clip,
    pub poster_jpeg: Vec<u8>,
}

fn media_dir(worktree: &Path) -> PathBuf {
    worktree.join(MEDIA_DIR)
}

fn ffmpeg() -> Option<PathBuf> {
    let found = Command::new("ffmpeg")
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    found.then(|| PathBuf::from("ffmpeg"))
}

/// Convert any clip ffmpeg reads into what the app plays: Constrained Baseline H.264 (no
/// B-frames), 4:2:0, at most 1920 wide, AAC audio when there is audio.
fn convert(ffmpeg: &Path, source: &Path, target: &Path) -> Result<()> {
    let output = Command::new(ffmpeg)
        .args(["-nostdin", "-loglevel", "error", "-y", "-i"])
        .arg(source)
        .args([
            "-map", "0:v:0", "-map", "0:a:0?", "-c:v", "libx264", "-profile:v", "baseline",
            "-pix_fmt", "yuv420p", "-vf", "scale='min(1920,iw)':-2", "-c:a", "aac", "-movflags",
            "+faststart", "-f", "mp4",
        ])
        .arg(target)
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        let said = said.lines().last().unwrap_or("no message");
        return Err(Error::new(format!("ffmpeg could not convert the clip: {said}")));
    }
    Ok(())
}

fn sha256_hex(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Check (and, if needed, convert) `source`, then keep it in the worktree's media directory.
pub fn store(worktree: &Path, source: &Path) -> Result<Stored> {
    let size = std::fs::metadata(source)
        .map_err(|e| Error::new(format!("{}: {e}", source.display())))?
        .len();
    if size == 0 || size > MAX_CLIP_BYTES {
        return Err(Error::new(format!(
            "a clip must be 1 byte to {} MiB; this one is {size} bytes",
            MAX_CLIP_BYTES / (1024 * 1024)
        )));
    }
    let dir = media_dir(worktree);
    std::fs::create_dir_all(&dir)?;
    let scratch = dir.join(format!(".incoming-{}.mp4", std::process::id()));
    let result = store_in(&dir, source, &scratch);
    let _ = std::fs::remove_file(&scratch);
    result
}

fn store_in(dir: &Path, source: &Path, scratch: &Path) -> Result<Stored> {
    let (file, checked): (PathBuf, Checked) = match afterburn_video::check(source) {
        Ok(checked) => (source.to_path_buf(), checked),
        Err(direct) => {
            let Some(ffmpeg) = ffmpeg() else {
                return Err(Error::new(format!(
                    "this clip does not play as it is ({direct}) and ffmpeg is not installed on the node's machine to convert it. Give an MP4 with Constrained Baseline H.264 video, or install ffmpeg"
                )));
            };
            convert(&ffmpeg, source, scratch)?;
            let checked = afterburn_video::check(scratch)
                .map_err(|e| Error::new(format!("the converted clip does not play: {e}")))?;
            (scratch.to_path_buf(), checked)
        }
    };
    let digest = sha256_hex(&file)?;
    let name = format!("{}.mp4", digest.get(..32).unwrap_or(&digest));
    let target = dir.join(&name);
    let size = std::fs::metadata(&file)?.len();
    if std::fs::metadata(&target).map(|m| m.len()).ok() != Some(size) {
        // Never leave a half-written clip under its final name.
        let partial = dir.join(format!(".{name}.part"));
        std::fs::copy(&file, &partial)?;
        std::fs::rename(&partial, &target)?;
    }
    Ok(Stored {
        clip: Clip {
            file: name,
            width: checked.width,
            height: checked.height,
            duration_ms: checked.duration_ms,
            size,
            audio: checked.audio,
        },
        poster_jpeg: checked.poster_jpeg,
    })
}

/// The page or card a new video goes on. Checked before the clip is stored.
pub fn parent(req: &Value) -> Result<u64> {
    req.get("parent").and_then(Value::as_u64).ok_or_else(|| {
        Error::new("media.add needs parent: the id of the page or card to put the video on")
    })
}

/// Put a stored clip's card on the canvas: a rectangle filled with the poster, tagged with the
/// clip. `req` carries `parent` (a page or card id), `x`, `y` and optionally `width` and `name`.
pub fn place(engine: &mut figcraft_engine::Session, stored: &Stored, req: &Value) -> Result<Value> {
    let parent = parent(req)?;
    let clip = &stored.clip;
    let width = req
        .get("width")
        .and_then(Value::as_f64)
        .filter(|w| w.is_finite() && *w >= 16.0 && *w <= 8000.0)
        .unwrap_or_else(|| f64::from(clip.width).min(DEFAULT_WIDTH));
    let height = (width * f64::from(clip.height) / f64::from(clip.width.max(1))).round();
    let name = req
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Video");
    let engine_error = |e: figcraft_engine::EngineError| Error::new(e.to_string());
    let placed = engine
        .execute(
            "image.place",
            &json!({
                "data": base64::engine::general_purpose::STANDARD.encode(&stored.poster_jpeg),
                "parent": parent,
                "x": req.get("x").and_then(Value::as_f64).unwrap_or(0.0),
                "y": req.get("y").and_then(Value::as_f64).unwrap_or(0.0),
                "width": width,
                "height": height,
                "name": name,
            }),
        )
        .map_err(engine_error)?;
    let id = placed["id"].clone();
    let tag = serde_json::to_string(clip)?;
    engine
        .execute("node.set", &json!({"ids": [id], "pluginData": {VIDEO_KEY: tag}}))
        .map_err(engine_error)?;
    // A reply's `id` is the request's: the card is `node`.
    Ok(json!({"node": id, "video": clip, "width": width, "height": height}))
}

/// One chunk of a clip, for the app's cache.
pub fn read(worktree: &Path, req: &Value) -> Result<Value> {
    let file = req
        .get("file")
        .and_then(Value::as_str)
        .filter(|name| afterburn_video::valid_file_name(name))
        .ok_or_else(|| Error::new("media.read needs file: a clip's file name"))?;
    let offset = req.get("offset").and_then(Value::as_u64).unwrap_or(0);
    let length = req
        .get("length")
        .and_then(Value::as_u64)
        .unwrap_or(MAX_CHUNK)
        .clamp(1, MAX_CHUNK);
    let path = media_dir(worktree).join(file);
    let mut handle = std::fs::File::open(&path).map_err(|_| {
        Error::new(format!(
            "the clip {file} is not in {MEDIA_DIR} of this worktree: it was not committed or pulled with the map"
        ))
    })?;
    let size = handle.metadata()?.len();
    if offset > size {
        return Err(Error::new("media.read offset is past the end of the clip"));
    }
    handle.seek(SeekFrom::Start(offset))?;
    let mut data = Vec::new();
    handle.take(length).read_to_end(&mut data)?;
    Ok(json!({
        "file": file,
        "offset": offset,
        "size": size,
        "data": base64::engine::general_purpose::STANDARD.encode(&data),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../server/figcraft/crates/afterburn-video/testdata/testsrc.mp4")
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("afterburn-media-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_clip_is_stored_by_hash_and_read_back_in_chunks() {
        let worktree = scratch("store");
        let stored = store(&worktree, &sample()).unwrap();
        assert!(afterburn_video::valid_file_name(&stored.clip.file));
        assert_eq!((stored.clip.width, stored.clip.height), (320, 180));
        assert!(stored.clip.audio);
        // Stored again, it is the same file.
        assert_eq!(store(&worktree, &sample()).unwrap().clip, stored.clip);
        let original = std::fs::read(sample()).unwrap();
        assert_eq!(stored.clip.size, original.len() as u64);
        let mut got = Vec::new();
        while (got.len() as u64) < stored.clip.size {
            let reply = read(
                &worktree,
                &json!({"file": stored.clip.file, "offset": got.len(), "length": 30000}),
            )
            .unwrap();
            assert_eq!(reply["size"], stored.clip.size);
            let chunk = base64::engine::general_purpose::STANDARD
                .decode(reply["data"].as_str().unwrap())
                .unwrap();
            assert!(!chunk.is_empty() && chunk.len() <= 30000);
            got.extend(chunk);
        }
        assert_eq!(got, original);
        let _ = std::fs::remove_dir_all(worktree);
    }

    #[test]
    fn read_never_leaves_the_media_directory() {
        let worktree = scratch("escape");
        std::fs::write(worktree.join("secret.mp4"), b"secret").unwrap();
        for file in ["../secret.mp4", "/etc/passwd", "secret.mp4", ""] {
            assert!(read(&worktree, &json!({"file": file})).is_err(), "{file}");
        }
        let missing = read(&worktree, &json!({"file": "0123456789abcdef0123456789abcdef.mp4"}))
            .unwrap_err()
            .to_string();
        assert!(missing.contains("not in .afterburn/media"), "{missing}");
        let _ = std::fs::remove_dir_all(worktree);
    }

    #[test]
    fn junk_is_refused_and_leaves_nothing_behind() {
        let worktree = scratch("junk");
        let junk = worktree.join("junk.bin");
        std::fs::write(&junk, b"this is not a video").unwrap();
        assert!(store(&worktree, &junk).is_err());
        assert!(store(&worktree, &worktree.join("absent.mp4")).is_err());
        let left: Vec<_> = std::fs::read_dir(media_dir(&worktree))
            .map(|entries| entries.flatten().map(|e| e.file_name()).collect())
            .unwrap_or_default();
        assert!(left.is_empty(), "{left:?}");
        let _ = std::fs::remove_dir_all(worktree);
    }

    #[test]
    fn the_card_carries_the_poster_and_the_clip() {
        let worktree = scratch("place");
        let stored = store(&worktree, &sample()).unwrap();
        let mut engine = figcraft_engine::Session::new();
        let page = engine.page().unwrap().0;
        let placed = place(&mut engine, &stored, &json!({"parent": page, "x": 40, "y": 60, "name": "Demo"})).unwrap();
        let id = figcraft_doc::NodeId(placed["node"].as_u64().unwrap());
        let doc = engine.doc().unwrap();
        let node = doc.node(id).unwrap();
        assert_eq!(node.name, "Demo");
        assert_eq!((node.width, node.height), (320.0, 180.0));
        let clip = Clip::parse(node.plugin_data.get(VIDEO_KEY).unwrap()).unwrap();
        assert_eq!(clip, stored.clip);
        assert_eq!(doc.images.len(), 1, "the poster is the card's image fill");
        assert!(place(&mut engine, &stored, &json!({"x": 0})).is_err(), "a parent is required");
        let _ = std::fs::remove_dir_all(worktree);
    }
}
