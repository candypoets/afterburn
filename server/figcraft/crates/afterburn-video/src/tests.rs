use super::*;

/// A four-second 320x180 test clip (ffmpeg's `testsrc2`, Constrained Baseline, with a sine tone),
/// made for these tests: synthetic content.
fn clip() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/testsrc.mp4")
}

#[test]
fn a_clip_decodes_in_order_and_seeks_to_a_key_frame() {
    let mut reader = Reader::open(&clip()).unwrap();
    assert_eq!((reader.width(), reader.height()), (320, 180));
    assert!(reader.has_audio());
    assert!((3900..=4100).contains(&reader.duration_ms()), "{}", reader.duration_ms());
    let mut last = None;
    let mut count = 0;
    while let Some(frame) = reader.next_frame().unwrap() {
        assert_eq!(frame.rgba.len(), 320 * 180 * 4);
        assert!(last.is_none_or(|t| frame.time_ms > t), "frames come in time order");
        last = Some(frame.time_ms);
        count += 1;
    }
    assert_eq!(count, reader.frames());
    let key = reader.seek(2500).unwrap();
    assert!(key <= 2500);
    let frame = reader.next_frame().unwrap().unwrap();
    assert_eq!(frame.time_ms, key);
}

#[test]
fn check_gives_a_poster() {
    let checked = check(&clip()).unwrap();
    assert_eq!((checked.width, checked.height), (320, 180));
    assert!(checked.audio);
    let poster = image::load_from_memory(&checked.poster_jpeg).unwrap();
    assert_eq!((poster.width(), poster.height()), (320, 180));
}

#[test]
fn junk_is_refused_not_a_crash() {
    let dir = std::env::temp_dir().join(format!("afterburn-video-junk-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("junk.mp4");
    std::fs::write(&path, b"not a video at all").unwrap();
    assert!(Reader::open(&path).is_err());
    let mut bytes = std::fs::read(clip()).unwrap();
    bytes.truncate(bytes.len() / 2);
    std::fs::write(&path, &bytes).unwrap();
    assert!(check(&path).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn clip_names_are_never_paths() {
    assert!(valid_file_name("0123456789abcdef.mp4"));
    for bad in ["../secret.mp4", "a/b.mp4", "0123456789ABCDEF.mp4", "short.mp4", "0123456789abcdef.mov", ""] {
        assert!(!valid_file_name(bad), "{bad}");
    }
    assert!(Clip::parse(r#"{"file":"../x.mp4","width":1,"height":1,"duration_ms":1,"size":1}"#).is_none());
}
