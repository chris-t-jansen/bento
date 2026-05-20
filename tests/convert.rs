//! Integration tests for the bento convert and config pipelines.
//!
//! These tests exercise the public API at the crate boundary without mocking
//! any filesystem I/O. They do NOT call ffmpeg; they test configuration
//! validation, output-path resolution, and error surface.

use std::path::{Path, PathBuf};

use bento::{
    error::Error,
    pipeline::run_convert,
    render::run_config,
    verbosity::Verbosity,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "bento-test-{}-{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn write(&self, name: &str, content: &str) -> PathBuf {
        let p = self.path.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn buf() -> Vec<u8> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// run_config — config subcommand
// ---------------------------------------------------------------------------

#[test]
fn run_config_errors_on_missing_path() {
    let mut out = buf();
    let result = run_config(Path::new("/nonexistent/file.mkv"), &mut out);
    assert!(matches!(result, Err(Error::PathNotFound(_))));
}

#[test]
fn run_config_succeeds_on_valid_config() {
    let dir = TestDir::new("cfg_valid");
    let video = dir.write("episode01.mkv", "");
    let _cfg = dir.write(
        "bento.toml",
        r#"
[output]
container = "mkv"

[audio]
tracks = [{ source = 1, lang = "jpn" }]
"#,
    );
    let mut out = buf();
    let result = run_config(&video, &mut out);
    assert!(result.is_ok(), "unexpected error: {:?}", result);
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("bento config for"));
    assert!(text.contains("Validation:"));
}

#[test]
fn run_config_reports_error_on_invalid_config() {
    let dir = TestDir::new("cfg_invalid");
    let video = dir.write("episode01.mkv", "");
    dir.write(
        "bento.toml",
        r#"
[audio]
tracks = [{ source = 0, lang = "jpn" }]
"#,
    );
    let mut out = buf();
    let result = run_config(&video, &mut out);
    assert!(
        matches!(result, Err(Error::ConfigInvalid { .. })),
        "expected ConfigInvalid, got: {:?}",
        result
    );
}

#[test]
fn run_config_on_directory_shows_each_file() {
    let dir = TestDir::new("cfg_dir");
    dir.write("ep01.mkv", "");
    dir.write("ep02.mkv", "");
    dir.write(
        "bento.toml",
        r#"
[output]
container = "mkv"

[audio]
tracks = [{ source = 1 }]
"#,
    );
    let mut out = buf();
    let result = run_config(&dir.path, &mut out);
    assert!(result.is_ok(), "unexpected error: {:?}", result);
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("ep01.mkv"));
    assert!(text.contains("ep02.mkv"));
}

#[test]
fn run_config_empty_directory_reports_no_files() {
    let dir = TestDir::new("cfg_empty_dir");
    let mut out = buf();
    let result = run_config(&dir.path, &mut out);
    assert!(result.is_ok());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("No video files found"));
}

// ---------------------------------------------------------------------------
// ASS+MP4 validation error
// ---------------------------------------------------------------------------

#[test]
fn run_config_errors_on_soft_ass_in_mp4() {
    let dir = TestDir::new("cfg_ass_mp4");
    let video = dir.write("episode01.mkv", "");
    dir.write(
        "bento.toml",
        r#"
[output]
container = "mp4"

[audio]
tracks = [{ source = 1 }]

[subtitles]
tracks = [{ source = 1, format = "ass", mux = "soft" }]
"#,
    );
    let mut out = buf();
    let result = run_config(&video, &mut out);
    assert!(
        matches!(result, Err(Error::ConfigInvalid { .. })),
        "expected ConfigInvalid for ASS+MP4, got: {:?}",
        result
    );
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("not supported in MP4"),
        "expected ASS+MP4 error in output:\n{}",
        text
    );
}

#[test]
fn run_config_allows_soft_ass_in_mkv() {
    let dir = TestDir::new("cfg_ass_mkv");
    let video = dir.write("episode01.mkv", "");
    dir.write(
        "bento.toml",
        r#"
[output]
container = "mkv"

[audio]
tracks = [{ source = 1 }]

[subtitles]
warn_no_default = false
tracks = [{ source = 1, format = "ass", mux = "soft" }]
"#,
    );
    let mut out = buf();
    let result = run_config(&video, &mut out);
    assert!(result.is_ok(), "ASS soft in MKV should be valid: {:?}", result);
}

// ---------------------------------------------------------------------------
// run_convert — missing ffmpeg short-circuits early
// ---------------------------------------------------------------------------

#[test]
fn run_convert_errors_on_missing_input() {
    let mut out = buf();
    let result = run_convert(
        Path::new("/nonexistent/episode.mkv"),
        None,
        None,
        Verbosity::Default,
        &mut out,
    );
    assert!(matches!(result, Err(Error::PathNotFound(_))));
}

#[test]
fn run_convert_requires_audio_tracks() {
    let dir = TestDir::new("conv_no_audio");
    let video = dir.write("episode01.mkv", "");
    dir.write(
        "bento.toml",
        r#"
[output]
container = "mkv"
"#,
    );
    let mut out = buf();
    let result = run_convert(&video, None, None, Verbosity::Default, &mut out);
    assert!(
        matches!(result, Err(Error::RequiredFieldMissing { ref field, .. }) if field == "audio.tracks"),
        "expected RequiredFieldMissing(audio.tracks), got: {:?}",
        result
    );
}

#[test]
fn run_convert_on_existing_fail_returns_output_exists() {
    let dir = TestDir::new("conv_on_existing_fail");
    let video = dir.write("episode01.mkv", "");
    // Pre-create the output file so on_existing=fail triggers.
    dir.write("episode01.mp4", "");
    dir.write(
        "bento.toml",
        r#"
[output]
container = "mp4"
on_existing = "fail"

[audio]
tracks = [{ source = 1 }]
"#,
    );
    let mut out = buf();
    let result = run_convert(&video, None, None, Verbosity::Default, &mut out);
    assert!(
        matches!(result, Err(Error::OutputExists { .. })),
        "expected OutputExists, got: {:?}",
        result
    );
}

#[test]
fn run_convert_on_existing_skip_silently_returns_ok() {
    let dir = TestDir::new("conv_skip_silently");
    let video = dir.write("episode01.mkv", "");
    dir.write("episode01.mp4", "");
    dir.write(
        "bento.toml",
        r#"
[output]
container = "mp4"
on_existing = "skip_silently"

[audio]
tracks = [{ source = 1 }]
"#,
    );
    let mut out = buf();
    let result = run_convert(&video, None, None, Verbosity::Default, &mut out);
    assert!(result.is_ok(), "skip_silently should return Ok: {:?}", result);
}

#[test]
fn run_convert_on_existing_warn_skips_with_message() {
    let dir = TestDir::new("conv_warn_existing");
    let video = dir.write("episode01.mkv", "");
    dir.write("episode01.mp4", "");
    dir.write(
        "bento.toml",
        r#"
[output]
container = "mp4"
on_existing = "warn"

[audio]
tracks = [{ source = 1 }]
"#,
    );
    let mut out = buf();
    let result = run_convert(&video, None, None, Verbosity::Default, &mut out);
    assert!(result.is_ok(), "warn mode should skip and return Ok: {:?}", result);
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("warning: output exists"),
        "expected skip warning:\n{}",
        text
    );
}
