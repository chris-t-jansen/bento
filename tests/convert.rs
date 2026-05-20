//! Integration tests for the bento convert and config pipelines.
//!
//! These tests exercise the public API at the crate boundary without mocking
//! any filesystem I/O. They do NOT call ffmpeg; they test configuration
//! validation, output-path resolution, and error surface.

use std::path::{Path, PathBuf};

use bento::{
    error::Error,
    pipeline::{WarnFlags, run_convert},
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
        false,
        Verbosity::Default,
        WarnFlags::default(),
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
    let result = run_convert(
        &video,
        None,
        None,
        false,
        Verbosity::Default,
        WarnFlags::default(),
        &mut out,
    );
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
    let result = run_convert(
        &video,
        None,
        None,
        false,
        Verbosity::Default,
        WarnFlags::default(),
        &mut out,
    );
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
    let result = run_convert(
        &video,
        None,
        None,
        false,
        Verbosity::Default,
        WarnFlags::default(),
        &mut out,
    );
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
    let result = run_convert(
        &video,
        None,
        None,
        false,
        Verbosity::Default,
        WarnFlags::default(),
        &mut out,
    );
    assert!(result.is_ok(), "warn mode should skip and return Ok: {:?}", result);
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("warning: output exists"),
        "expected skip warning:\n{}",
        text
    );
}

// ---------------------------------------------------------------------------
// run_convert --dry-run
// ---------------------------------------------------------------------------

#[test]
fn dry_run_header_says_dry_run_for() {
    let dir = TestDir::new("dryrun_header");
    let video = dir.write("episode01.mkv", "");
    dir.write(
        "bento.toml",
        r#"
[output]
container = "mkv"
on_existing = "warn"

[audio]
tracks = [{ source = 1, lang = "jpn", title = "Japanese", default = true }]
"#,
    );
    let mut out = buf();
    let result = run_convert(
        &video,
        None,
        None,
        true,
        Verbosity::Default,
        WarnFlags::default(),
        &mut out,
    );
    let text = String::from_utf8(out).unwrap();
    match result {
        Err(Error::FfmpegNotFound) => return,
        _ => {}
    }
    assert!(
        text.contains("Dry-run for"),
        "header should say 'Dry-run for', got:\n{}",
        text
    );
    assert!(
        !text.contains("Converting"),
        "header should NOT say 'Converting' in dry-run, got:\n{}",
        text
    );
}

#[test]
fn dry_run_config_error_shows_summary_with_error_count() {
    let dir = TestDir::new("dryrun_config_err");
    let video = dir.write("episode01.mkv", "");
    dir.write(
        "bento.toml",
        r#"
[output]
container = "mkv"
"#,
    );
    let mut out = buf();
    let result = run_convert(
        &video,
        None,
        None,
        true,
        Verbosity::Default,
        WarnFlags::default(),
        &mut out,
    );
    let text = String::from_utf8(out).unwrap();
    assert!(result.is_err(), "config error should propagate: {:?}", result);
    assert!(
        text.contains("would be processed"),
        "dry-run summary missing 'would be processed':\n{}",
        text
    );
    assert!(
        text.contains("error"),
        "dry-run summary should mention error count:\n{}",
        text
    );
}

#[test]
fn dry_run_does_not_create_output_directory() {
    let dir = TestDir::new("dryrun_no_mkdir");
    let video = dir.write("episode01.mkv", "");
    dir.write(
        "bento.toml",
        r#"
[output]
container = "mp4"
destination = "encoded"

[audio]
tracks = [{ source = 1, lang = "jpn", default = true }]
"#,
    );
    let encoded_dir = dir.path.join("encoded");
    assert!(!encoded_dir.exists(), "encoded/ should not exist before run");

    let mut out = buf();
    let result = run_convert(
        &video,
        None,
        None,
        true,
        Verbosity::Default,
        WarnFlags::default(),
        &mut out,
    );
    match result {
        Err(Error::FfmpegNotFound) => return,
        _ => {}
    }

    assert!(
        !encoded_dir.exists(),
        "dry-run must NOT create the destination directory, but encoded/ was created"
    );
}

#[test]
fn dry_run_summary_footer_suppressed_in_quiet_mode() {
    let dir = TestDir::new("dryrun_quiet");
    let video = dir.write("episode01.mkv", "");
    dir.write("bento.toml", "[output]\ncontainer = \"mkv\"\n");
    let mut out = buf();
    let _ = run_convert(
        &video,
        None,
        None,
        true,
        Verbosity::Quiet,
        WarnFlags::default(),
        &mut out,
    );
    let text = String::from_utf8(out).unwrap();
    assert!(
        !text.contains("bento config"),
        "quiet mode should suppress the footer hint:\n{}",
        text
    );
}

#[test]
fn dry_run_summary_footer_shown_in_default_mode() {
    let dir = TestDir::new("dryrun_footer");
    let video = dir.write("episode01.mkv", "");
    dir.write("bento.toml", "[output]\ncontainer = \"mkv\"\n");
    let mut out = buf();
    let _ = run_convert(
        &video,
        None,
        None,
        true,
        Verbosity::Default,
        WarnFlags::default(),
        &mut out,
    );
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("bento config"),
        "default mode should show the footer hint:\n{}",
        text
    );
}

#[test]
fn dry_run_plan_contains_expected_sections() {
    let dir = TestDir::new("dryrun_plan");
    let video = dir.write("episode01.mkv", "");
    dir.write(
        "bento.toml",
        r#"
[output]
container = "mp4"

[video]
encoder = { name = "x264", crf = 20, tune = "animation" }
preset = "medium"

[audio]
tracks = [
    { source = 1, lang = "jpn", title = "Japanese", default = true },
    { source = 2, lang = "eng", title = "English Dub" },
]

[subtitles]
tracks = [
    { source = 1, format = "srt", mux = "soft", subtract_track = 2, lang = "eng", title = "English", default = true },
    { source = 2, format = "ass", mux = "burn" },
]
"#,
    );
    let mut out = buf();
    let result = run_convert(
        &video,
        None,
        None,
        true,
        Verbosity::Default,
        WarnFlags::default(),
        &mut out,
    );
    let text = String::from_utf8(out).unwrap();
    match result {
        Err(Error::FfmpegNotFound) | Err(Error::FfprobeFailed { .. }) => return,
        _ => {}
    }
    assert!(text.contains("Would extract subtitle track"), "missing subtitle extraction line:\n{}", text);
    assert!(text.contains("Would derive"), "missing subtitle derivation line:\n{}", text);
    assert!(text.contains("Would burn"), "missing burn subtitle line:\n{}", text);
    assert!(text.contains("Would transcode video: x264 crf=20"), "missing video plan:\n{}", text);
    assert!(text.contains("Would"), "missing audio plan:\n{}", text);
    assert!(text.contains("Would mux to:"), "missing mux destination line:\n{}", text);
    assert!(text.contains("would be processed"), "missing dry-run summary:\n{}", text);
}

// ---------------------------------------------------------------------------
// --no-warn-X / --no-warnings suppression flags
//
// Each test uses a config that fires the warning under test plus a hard
// validation error (two audio `default=true`) or missing `audio.tracks` to
// stop execution before ffmpeg is needed. Warnings are emitted before any
// error check, so they appear in the output even when the run exits early.
// ---------------------------------------------------------------------------

/// Config that triggers `warn_multiple_burns`. Omits `audio.tracks` so the
/// run exits with `RequiredFieldMissing` (no ffmpeg involved).
fn multiple_burns_toml() -> &'static str {
    r#"
[subtitles]
tracks = [
    { source = 1, format = "ass", mux = "burn" },
    { source = 2, format = "ass", mux = "burn" },
    { source = 3, format = "srt", mux = "soft", default = true },
]
"#
}

/// Config that triggers `warn_burn_metadata`. Omits `audio.tracks`.
fn burn_metadata_toml() -> &'static str {
    r#"
[subtitles]
tracks = [
    { source = 1, format = "ass", mux = "burn", lang = "eng" },
    { source = 2, format = "srt", mux = "soft", default = true },
]
"#
}

/// Config that triggers audio `warn_no_default`. Uses two subtitle
/// `default=true` to produce a hard error that stops the run without ffmpeg.
fn audio_no_default_toml() -> &'static str {
    r#"
[audio]
tracks = [{ source = 1, lang = "jpn" }]

[subtitles]
tracks = [
    { source = 1, format = "srt", mux = "soft", default = true },
    { source = 2, format = "srt", mux = "soft", default = true },
]
"#
}

/// Config that triggers subtitle `warn_no_default`. Uses two audio
/// `default=true` to produce a hard error that stops the run without ffmpeg.
fn subtitle_no_default_toml() -> &'static str {
    r#"
[audio]
tracks = [
    { source = 1, lang = "jpn", default = true },
    { source = 2, lang = "eng", default = true },
]

[subtitles]
tracks = [{ source = 1, format = "srt", mux = "soft" }]
"#
}

/// Config that triggers `warn_crf_codec_mismatch` (x264 with x265-typical
/// CRF ≥ 24). Uses two audio `default=true` as a hard stop.
fn crf_mismatch_toml() -> &'static str {
    r#"
[video]
encoder = { name = "x264", crf = 26 }

[audio]
tracks = [
    { source = 1, default = true },
    { source = 2, default = true },
]
"#
}

#[test]
fn no_warn_multiple_burns_suppresses_warning() {
    let dir = TestDir::new("warn_burns_suppress");
    let video = dir.write("episode01.mkv", "");
    dir.write("bento.toml", multiple_burns_toml());

    // Without suppression: warning is present.
    let mut out = buf();
    let _ = run_convert(&video, None, None, false, Verbosity::Default, WarnFlags::default(), &mut out);
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("subtitle tracks have mux=\"burn\""),
        "expected multiple-burns warning in output:\n{}",
        text
    );

    // With suppression: warning is absent.
    let mut out = buf();
    let _ = run_convert(
        &video,
        None,
        None,
        false,
        Verbosity::Default,
        WarnFlags { no_warn_multiple_burns: true, ..WarnFlags::default() },
        &mut out,
    );
    let text = String::from_utf8(out).unwrap();
    assert!(
        !text.contains("subtitle tracks have mux=\"burn\""),
        "multiple-burns warning should be suppressed:\n{}",
        text
    );
}

#[test]
fn no_warn_burn_metadata_suppresses_warning() {
    let dir = TestDir::new("warn_burn_meta_suppress");
    let video = dir.write("episode01.mkv", "");
    dir.write("bento.toml", burn_metadata_toml());

    let mut out = buf();
    let _ = run_convert(&video, None, None, false, Verbosity::Default, WarnFlags::default(), &mut out);
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("burn subtitle track has soft-only metadata"),
        "expected burn-metadata warning:\n{}",
        text
    );

    let mut out = buf();
    let _ = run_convert(
        &video,
        None,
        None,
        false,
        Verbosity::Default,
        WarnFlags { no_warn_burn_metadata: true, ..WarnFlags::default() },
        &mut out,
    );
    let text = String::from_utf8(out).unwrap();
    assert!(
        !text.contains("burn subtitle track has soft-only metadata"),
        "burn-metadata warning should be suppressed:\n{}",
        text
    );
}

#[test]
fn no_warn_no_default_suppresses_audio_warning() {
    let dir = TestDir::new("warn_audio_default_suppress");
    let video = dir.write("episode01.mkv", "");
    dir.write("bento.toml", audio_no_default_toml());

    let mut out = buf();
    let _ = run_convert(&video, None, None, false, Verbosity::Default, WarnFlags::default(), &mut out);
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("no audio track has default=true"),
        "expected audio no-default warning:\n{}",
        text
    );

    let mut out = buf();
    let _ = run_convert(
        &video,
        None,
        None,
        false,
        Verbosity::Default,
        WarnFlags { no_warn_no_default: true, ..WarnFlags::default() },
        &mut out,
    );
    let text = String::from_utf8(out).unwrap();
    assert!(
        !text.contains("no audio track has default=true"),
        "audio no-default warning should be suppressed:\n{}",
        text
    );
}

#[test]
fn no_warn_no_default_suppresses_subtitle_warning() {
    let dir = TestDir::new("warn_sub_default_suppress");
    let video = dir.write("episode01.mkv", "");
    dir.write("bento.toml", subtitle_no_default_toml());

    let mut out = buf();
    let _ = run_convert(&video, None, None, false, Verbosity::Default, WarnFlags::default(), &mut out);
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("no subtitle track has default=true"),
        "expected subtitle no-default warning:\n{}",
        text
    );

    let mut out = buf();
    let _ = run_convert(
        &video,
        None,
        None,
        false,
        Verbosity::Default,
        WarnFlags { no_warn_no_default: true, ..WarnFlags::default() },
        &mut out,
    );
    let text = String::from_utf8(out).unwrap();
    assert!(
        !text.contains("no subtitle track has default=true"),
        "subtitle no-default warning should be suppressed:\n{}",
        text
    );
}

#[test]
fn no_warn_crf_codec_mismatch_suppresses_warning() {
    let dir = TestDir::new("warn_crf_suppress");
    let video = dir.write("episode01.mkv", "");
    dir.write("bento.toml", crf_mismatch_toml());

    let mut out = buf();
    let _ = run_convert(&video, None, None, false, Verbosity::Default, WarnFlags::default(), &mut out);
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("encoder.crf"),
        "expected CRF mismatch warning:\n{}",
        text
    );

    let mut out = buf();
    let _ = run_convert(
        &video,
        None,
        None,
        false,
        Verbosity::Default,
        WarnFlags { no_warn_crf_codec_mismatch: true, ..WarnFlags::default() },
        &mut out,
    );
    let text = String::from_utf8(out).unwrap();
    assert!(
        !text.contains("encoder.crf=26 is x265-typical"),
        "CRF mismatch warning should be suppressed:\n{}",
        text
    );
}

#[test]
fn no_warnings_suppresses_all_config_implication_warnings() {
    let dir = TestDir::new("no_warnings_bulk");
    let video = dir.write("episode01.mkv", "");
    // Config generates three distinct warnings:
    //   - warn_crf_codec_mismatch  (x264 + crf=26)
    //   - warn_multiple_burns      (two burn tracks)
    //   - warn_burn_metadata       (burn track with lang)
    // Plus a hard audio error (two defaults) so the run exits without ffmpeg.
    dir.write(
        "bento.toml",
        r#"
[video]
encoder = { name = "x264", crf = 26 }

[audio]
tracks = [
    { source = 1, default = true },
    { source = 2, default = true },
]

[subtitles]
tracks = [
    { source = 1, format = "ass", mux = "burn", lang = "eng" },
    { source = 2, format = "ass", mux = "burn" },
    { source = 3, format = "srt", mux = "soft", default = true },
]
"#,
    );

    // Without suppression: all three warnings appear.
    let mut out = buf();
    let _ = run_convert(&video, None, None, false, Verbosity::Default, WarnFlags::default(), &mut out);
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("encoder.crf=26"), "CRF warning expected:\n{}", text);
    assert!(text.contains("subtitle tracks have mux=\"burn\""), "multiple-burns warning expected:\n{}", text);
    assert!(text.contains("burn subtitle track has soft-only metadata"), "burn-metadata warning expected:\n{}", text);

    // With --no-warnings: all warnings suppressed; hard error still present.
    let mut out = buf();
    let _ = run_convert(
        &video,
        None,
        None,
        false,
        Verbosity::Default,
        WarnFlags { no_warnings: true, ..WarnFlags::default() },
        &mut out,
    );
    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains("encoder.crf=26 is x265-typical"), "CRF warning should be suppressed:\n{}", text);
    assert!(!text.contains("subtitle tracks have mux=\"burn\""), "multiple-burns warning should be suppressed:\n{}", text);
    assert!(!text.contains("burn subtitle track has soft-only metadata"), "burn-metadata warning should be suppressed:\n{}", text);
    // The hard error (two audio defaults) is still present.
    assert!(text.contains("multiple audio tracks have default=true"), "hard error must still appear:\n{}", text);
}
