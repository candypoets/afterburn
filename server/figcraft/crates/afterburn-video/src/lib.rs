//! Video clips on the Afterburn canvas.
//!
//! A clip is an MP4 file with one H.264 video track (and, optionally, AAC audio) kept beside the
//! map in `.afterburn/media/`. The card that shows it is an ordinary layer filled with the clip's
//! first frame (the poster) and tagged with [`VIDEO_KEY`] plugin data ([`Clip`]). The node makes
//! the poster and checks that the clip plays ([`check`]); the app decodes it for playback
//! ([`Reader`]).
//!
//! The decoder is OpenH264: it plays Constrained Baseline streams without B-frames reliably, so
//! the node normalises other clips to that with ffmpeg where ffmpeg is installed.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The plugin-data key on a card that shows a clip. Its value is a [`Clip`] as JSON.
pub const VIDEO_KEY: &str = "afterburn.video";

/// Where clips live, relative to the worktree.
pub const MEDIA_DIR: &str = ".afterburn/media";

/// The largest clip the node takes (bytes).
pub const MAX_CLIP_BYTES: u64 = 512 * 1024 * 1024;

/// The largest frame side decoded.
const MAX_SIDE: u32 = 4096;

/// What a card knows about its clip.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    /// File name inside [`MEDIA_DIR`]: `<sha256 prefix>.mp4`.
    pub file: String,
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
    /// The file's size in bytes.
    pub size: u64,
    /// Whether the file has an audio track.
    #[serde(default)]
    pub audio: bool,
}

impl Clip {
    pub fn parse(value: &str) -> Option<Clip> {
        let clip: Clip = serde_json::from_str(value).ok()?;
        (valid_file_name(&clip.file) && clip.width > 0 && clip.height > 0).then_some(clip)
    }
}

/// A clip's file name is a lowercase hex digest and `.mp4`: never a path.
pub fn valid_file_name(name: &str) -> bool {
    match name.strip_suffix(".mp4") {
        Some(stem) => (16..=64).contains(&stem.len()) && stem.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        None => false,
    }
}

/// One decoded frame, RGBA8, `width * height * 4` bytes.
pub struct Frame {
    pub index: usize,
    /// Presentation time in milliseconds.
    pub time_ms: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

struct Sample {
    time_ms: u64,
    sync: bool,
}

/// A clip open for decoding. Frames come in presentation order; [`Reader::seek`] goes back to the
/// key frame at or before a time.
pub struct Reader {
    mp4: mp4::Mp4Reader<BufReader<File>>,
    track: u32,
    header: Vec<u8>,
    samples: Vec<Sample>,
    decoder: openh264::decoder::Decoder,
    /// The next sample to decode (0-based).
    next: usize,
    width: u32,
    height: u32,
    duration_ms: u64,
    audio: bool,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn decoder(header: &[u8]) -> Result<openh264::decoder::Decoder, String> {
    let mut decoder = openh264::decoder::Decoder::new().map_err(err)?;
    // Parameter sets alone give no picture; a stream the decoder refuses fails here.
    decoder.decode(header).map_err(|e| format!("The video stream cannot be decoded: {e}"))?;
    Ok(decoder)
}

impl Reader {
    pub fn open(path: &Path) -> Result<Reader, String> {
        let file = File::open(path).map_err(err)?;
        let size = file.metadata().map_err(err)?.len();
        let mp4 = mp4::Mp4Reader::read_header(BufReader::new(file), size).map_err(|e| format!("Not an MP4 file: {e}"))?;
        let mut video = None;
        let mut audio = false;
        for (id, track) in mp4.tracks() {
            match track.track_type() {
                Ok(mp4::TrackType::Video) if video.is_none() && matches!(track.media_type(), Ok(mp4::MediaType::H264)) => video = Some(*id),
                Ok(mp4::TrackType::Audio) => audio = true,
                _ => {}
            }
        }
        let id = video.ok_or("The file has no H.264 video track.")?;
        let track = mp4.tracks().get(&id).ok_or("The file has no H.264 video track.")?;
        let (width, height) = (u32::from(track.width()), u32::from(track.height()));
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(format!("Unsupported video size {width}x{height}."));
        }
        let mut header = Vec::new();
        for set in [track.sequence_parameter_set().map_err(err)?, track.picture_parameter_set().map_err(err)?] {
            header.extend_from_slice(&[0, 0, 0, 1]);
            header.extend_from_slice(set);
        }
        let timescale = u64::from(track.timescale()).max(1);
        let count = track.sample_count();
        let duration_ms = u64::try_from(track.duration().as_millis()).unwrap_or(u64::MAX);
        let mut reader =
            Reader { decoder: decoder(&header)?, mp4, track: id, header, samples: Vec::new(), next: 0, width, height, duration_ms, audio };
        // Sample times, without reading the frames themselves.
        let mut samples = Vec::with_capacity(count as usize);
        for index in 1..=count {
            let sample = reader.mp4.read_sample(id, index).map_err(err)?.ok_or("The video track is truncated.")?;
            let ticks = (i128::from(sample.start_time) + i128::from(sample.rendering_offset)).max(0) as u128;
            samples.push(Sample { time_ms: u64::try_from(ticks * 1000 / u128::from(timescale)).unwrap_or(u64::MAX), sync: sample.is_sync });
        }
        if samples.is_empty() {
            return Err("The video track is empty.".into());
        }
        if let Some(first) = samples.first_mut() {
            first.sync = true;
        }
        reader.samples = samples;
        Ok(reader)
    }

    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn duration_ms(&self) -> u64 {
        self.duration_ms.max(self.samples.last().map_or(0, |s| s.time_ms))
    }
    pub fn frames(&self) -> usize {
        self.samples.len()
    }
    pub fn has_audio(&self) -> bool {
        self.audio
    }
    /// The presentation time of the next frame [`Reader::next_frame`] gives, or `None` at the end.
    pub fn next_time_ms(&self) -> Option<u64> {
        self.samples.get(self.next).map(|s| s.time_ms)
    }

    /// Decode the next frame. `Ok(None)` at the end of the clip.
    pub fn next_frame(&mut self) -> Result<Option<Frame>, String> {
        loop {
            let index = self.next;
            let Some(sample) = self.samples.get(index) else { return Ok(None) };
            let time_ms = sample.time_ms;
            let data = self.mp4.read_sample(self.track, index as u32 + 1).map_err(err)?.ok_or("The video track is truncated.")?;
            self.next += 1;
            let Some(nal) = annex_b(&data.bytes) else { return Err("A video sample is malformed.".into()) };
            let decoded = self.decoder.decode(&nal).map_err(|e| format!("Frame {} cannot be decoded: {e}", index + 1))?;
            // A decoder that holds a frame back gives it with a later sample.
            let Some(yuv) = decoded else { continue };
            use openh264::formats::YUVSource;
            let (w, h) = yuv.dimensions();
            let mut rgba = vec![0u8; w.saturating_mul(h).saturating_mul(4)];
            yuv.write_rgba8(&mut rgba);
            return Ok(Some(Frame { index, time_ms, width: w as u32, height: h as u32, rgba }));
        }
    }

    /// Continue from the key frame at or before `time_ms`. Returns that key frame's time: decode
    /// on from there to reach the exact frame.
    pub fn seek(&mut self, time_ms: u64) -> Result<u64, String> {
        let mut key = 0;
        for (index, sample) in self.samples.iter().enumerate() {
            if sample.time_ms > time_ms {
                break;
            }
            if sample.sync {
                key = index;
            }
        }
        self.decoder = decoder(&self.header)?;
        self.next = key;
        Ok(self.samples.get(key).map_or(0, |s| s.time_ms))
    }
}

/// An MP4 sample holds length-prefixed NAL units; the decoder takes start codes.
fn annex_b(sample: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(sample.len() + 16);
    let mut at = 0;
    while at < sample.len() {
        let len = u32::from_be_bytes(sample.get(at..at + 4)?.try_into().ok()?) as usize;
        at += 4;
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(sample.get(at..at.checked_add(len)?)?);
        at += len;
    }
    Some(out)
}

/// What [`check`] learned about a clip that plays.
pub struct Checked {
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
    pub audio: bool,
    /// The first frame as a JPEG: the card's poster.
    pub poster_jpeg: Vec<u8>,
}

/// Decode a whole clip once: the proof that the app can play it. Gives its poster.
pub fn check(path: &Path) -> Result<Checked, String> {
    let mut reader = Reader::open(path)?;
    let first = reader.next_frame()?.ok_or("The clip has no frame.")?;
    let (width, height) = (first.width, first.height);
    let mut decoded = 1;
    while reader.next_frame()?.is_some() {
        decoded += 1;
    }
    if decoded != reader.frames() {
        return Err(format!("Only {decoded} of {} frames decode.", reader.frames()));
    }
    let rgb: Vec<u8> = first.rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let mut poster_jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut poster_jpeg, 85)
        .encode(&rgb, width, height, image::ExtendedColorType::Rgb8)
        .map_err(err)?;
    Ok(Checked { width, height, duration_ms: reader.duration_ms(), audio: reader.has_audio(), poster_jpeg })
}

#[cfg(test)]
mod tests;
