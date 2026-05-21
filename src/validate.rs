//! Cross-field validation of a [`Resolved`] config.
//!
//! Validation runs after resolution and checks couplings the structural schema
//! can't enforce (CRF/codec scaling mismatch, `tune` valid for the resolved
//! encoder, exactly-one `default = true` per stream type, mutually exclusive
//! `filter` vs `subtract_track`, etc.).
//!
//! The pure-config subset lives here. Validation that requires source probing
//! (subtitle source format detection) or filesystem I/O (file-path source
//! existence, extension recognition) is deferred to the encode pipeline.
//!
//! Returns a list of [`ValidationIssue`]s sorted into [`Severity::Error`]
//! (which must be resolved before encoding) and [`Severity::Warning`]
//! (informational; suppressible per the warnings index).

use crate::config::*;
use crate::resolve::Resolved;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub severity: Severity,
    /// Dotted path for context (e.g. `"video.encoder.tune"`); empty if the issue
    /// spans multiple paths.
    pub path: String,
    pub message: String,
}

impl ValidationIssue {
    fn error(path: impl Into<String>, msg: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            path: path.into(),
            message: msg.into(),
        }
    }

    fn warning(path: impl Into<String>, msg: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            path: path.into(),
            message: msg.into(),
        }
    }
}

/// Run all pure-config validation checks on a resolved configuration.
pub fn validate(resolved: &Resolved) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let cfg = &resolved.config;

    validate_output(&cfg.output, &mut issues);
    validate_video(&cfg.video, &mut issues);
    validate_audio(&cfg.audio, &mut issues);
    validate_subtitles(&cfg.subtitles, cfg.output.container, &mut issues);

    issues
}

// =============================================================================
// Output
// =============================================================================

fn validate_output(output: &Output, issues: &mut Vec<ValidationIssue>) {
    let Some(naming) = &output.naming else { return };

    // Always validate regex syntax when the field is set.
    let compiled_regex = naming.regex.as_ref().and_then(|pattern| {
        match regex::Regex::new(pattern) {
            Ok(re) => Some(re),
            Err(e) => {
                issues.push(ValidationIssue::error(
                    "output.naming.regex",
                    format!("regex failed to compile: {e}"),
                ));
                None
            }
        }
    });

    let Some(template) = &naming.template else { return };

    // Named captures available from the compiled regex (empty if regex absent or invalid).
    let regex_captures: Vec<&str> = compiled_regex
        .as_ref()
        .map(|re| re.capture_names().flatten().collect())
        .unwrap_or_default();

    // Built-in variables always available regardless of config.
    const BUILTINS: &[&str] = &["source_basename", "source_dir"];
    let meta = output.metadata.as_ref();

    // Parse the template and verify each {varname} reference is resolvable.
    let var_re = regex::Regex::new(r"\{([A-Za-z_][A-Za-z0-9_]*)(?::[^}]*)?\}").unwrap();
    for cap in var_re.captures_iter(template) {
        let var_name: &str = &cap[1];

        if BUILTINS.contains(&var_name) {
            continue;
        }

        let from_metadata = match var_name {
            "show" => meta.and_then(|m| m.show.as_ref()).is_some(),
            "season" => meta.map(|m| m.season.is_some()).unwrap_or(false),
            "year" => meta.map(|m| m.year.is_some()).unwrap_or(false),
            _ => false,
        };
        if from_metadata {
            continue;
        }

        if regex_captures.contains(&var_name) {
            continue;
        }

        // Variable is not resolvable from any source — config error.
        let hint = match var_name {
            "show" => " — set output.metadata.show or add a regex capture named `show`",
            "season" => " — set output.metadata.season or add a regex capture named `season`",
            "year" => " — set output.metadata.year or add a regex capture named `year`",
            _ if naming.regex.is_some() => {
                " — not a built-in variable, metadata field, or named regex capture"
            }
            _ => {
                " — not a built-in variable or metadata field; \
                 add a regex capture or set the metadata field"
            }
        };
        issues.push(ValidationIssue::error(
            "output.naming.template",
            format!("template references undefined variable `{{{var_name}}}`{hint}"),
        ));
    }
}

// =============================================================================
// Video
// =============================================================================

fn validate_video(video: &Video, issues: &mut Vec<ValidationIssue>) {
    let Some(enc) = &video.encoder else { return };

    // Tune validity per encoder (x265 doesn't support film/stillimage).
    if let (Some(name), Some(tune)) = (enc.name, enc.tune) {
        if matches!((name, tune), (EncoderName::X265, Tune::Film | Tune::Stillimage)) {
            issues.push(ValidationIssue::error(
                "video.encoder.tune",
                format!(
                    "tune={:?} is not valid for encoder.name=x265. \
                     x265 supports: animation, grain, psnr, ssim, fastdecode, zerolatency, none.",
                    tune_str(tune),
                ),
            ));
        }
    }

    // CRF/codec coupling — suppressible by warn_crf_codec_mismatch.
    if video.warn_crf_codec_mismatch.unwrap_or(true) {
        if let (Some(name), Some(crf)) = (enc.name, enc.crf) {
            let mismatch = match name {
                EncoderName::X264 if crf >= 24 => Some(format!(
                    "encoder.crf={} is x265-typical (≥24); resolved encoder.name=x264 \
                     uses a different scale (transparent ≈18). \
                     Did you mean encoder.name=\"x265\"?",
                    crf
                )),
                EncoderName::X265 if crf <= 19 => Some(format!(
                    "encoder.crf={} is x264-typical (≤19); resolved encoder.name=x265 \
                     uses a different scale (transparent ≈20-22). \
                     Did you mean encoder.name=\"x264\"?",
                    crf
                )),
                _ => None,
            };
            if let Some(msg) = mismatch {
                issues.push(ValidationIssue::warning("video.encoder.crf", msg));
            }
        }
    }
}

fn tune_str(t: Tune) -> &'static str {
    match t {
        Tune::Film => "film",
        Tune::Animation => "animation",
        Tune::Grain => "grain",
        Tune::Stillimage => "stillimage",
        Tune::Psnr => "psnr",
        Tune::Ssim => "ssim",
        Tune::Fastdecode => "fastdecode",
        Tune::Zerolatency => "zerolatency",
        Tune::None => "none",
    }
}

// =============================================================================
// Audio
// =============================================================================

fn validate_audio(audio: &Audio, issues: &mut Vec<ValidationIssue>) {
    let Some(tracks) = &audio.tracks else { return };

    // Per-track required field: source. And `source >= 1` since track indices
    // are 1-based throughout Bento's surface (matching HandBrake / mkvmerge /
    // VLC user-facing tooling).
    for (i, track) in tracks.iter().enumerate() {
        match track.source {
            None => {
                issues.push(ValidationIssue::error(
                    format!("audio.tracks[{}]", i),
                    "track is missing required field `source` (input track index, 1-based)",
                ));
            }
            Some(0) => {
                issues.push(ValidationIssue::error(
                    format!("audio.tracks[{}].source", i),
                    "`source` must be ≥ 1 — track indices are 1-based; \
                     the first audio track is `source = 1`",
                ));
            }
            Some(_) => {}
        }
    }

    // default uniqueness — hard error.
    let default_indices: Vec<usize> = tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.default == Some(true))
        .map(|(i, _)| i)
        .collect();

    if default_indices.len() > 1 {
        issues.push(ValidationIssue::error(
            "audio.tracks",
            format!(
                "multiple audio tracks have default=true (indices: {:?}); \
                 at most one allowed per stream type.",
                default_indices
            ),
        ));
    } else if default_indices.is_empty() && audio.warn_no_default.unwrap_or(true) {
        issues.push(ValidationIssue::warning(
            "audio.tracks",
            "no audio track has default=true; player will fall back to its own \
             track-selection logic, which may not match user expectation.",
        ));
    }
}

// =============================================================================
// Subtitles
// =============================================================================

fn validate_subtitles(subs: &Subtitles, container: Option<Container>, issues: &mut Vec<ValidationIssue>) {
    let Some(tracks) = &subs.tracks else { return };
    let effective_container = container.unwrap_or(Container::Mp4);

    // default uniqueness — hard error.
    let default_indices: Vec<usize> = tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.default == Some(true))
        .map(|(i, _)| i)
        .collect();

    if default_indices.len() > 1 {
        issues.push(ValidationIssue::error(
            "subtitles.tracks",
            format!(
                "multiple subtitle tracks have default=true (indices: {:?}); \
                 at most one allowed per stream type.",
                default_indices
            ),
        ));
    } else if default_indices.is_empty() && subs.warn_no_default.unwrap_or(true) {
        issues.push(ValidationIssue::warning(
            "subtitles.tracks",
            "no subtitle track has default=true; player will fall back to its own \
             track-selection logic, which may not match user expectation.",
        ));
    }

    // Per-track checks.
    let mut burn_count = 0;
    for (i, track) in tracks.iter().enumerate() {
        let path = format!("subtitles.tracks[{}]", i);

        // Required: source. And `source >= 1` for int-typed sources, since
        // track indices are 1-based throughout Bento (matching HandBrake /
        // mkvmerge user-facing tooling). Path-typed sources are unaffected.
        match &track.source {
            None => {
                issues.push(ValidationIssue::error(
                    &path,
                    "track is missing required field `source` (input track index or file path, 1-based)",
                ));
            }
            Some(TrackRef::Index(0)) => {
                issues.push(ValidationIssue::error(
                    format!("{}.source", path),
                    "`source` must be ≥ 1 — track indices are 1-based; \
                     the first subtitle track is `source = 1`",
                ));
            }
            Some(_) => {}
        }

        // Repeat-subtract: subtract_track is also 1-based when integer-typed.
        if let Some(TrackRef::Index(0)) = &track.subtract_track {
            issues.push(ValidationIssue::error(
                format!("{}.subtract_track", path),
                "`subtract_track` must be ≥ 1 — track indices are 1-based",
            ));
        }

        // filter xor subtract_track — at most one derivation per track.
        if track.filter.is_some() && track.subtract_track.is_some() {
            issues.push(ValidationIssue::error(
                &path,
                "track has both `filter` and `subtract_track` set; \
                 at most one derivation operation per track.",
            ));
        }

        // ASS soft-mux is only supported in MKV containers; MP4 has no ASS
        // codec (mov_text is SRT-only). External tracks are sidecar files and
        // are not affected by the container codec restriction.
        if track.format == Some(SubtitleFormat::Ass)
            && matches!(track.mux, None | Some(SubtitleMux::Soft))
            && effective_container == Container::Mp4
        {
            issues.push(ValidationIssue::error(
                &path,
                "format=\"ass\" mux=\"soft\" is not supported in MP4 containers \
                 (MP4 uses mov_text, an SRT-only codec). Use container=\"mkv\", \
                 change format to \"srt\", or use mux=\"external\".",
            ));
        }

        // Burn-track-specific checks.
        if track.mux == Some(SubtitleMux::Burn) {
            burn_count += 1;

            if subs.warn_burn_metadata.unwrap_or(true) && has_soft_metadata(track) {
                issues.push(ValidationIssue::warning(
                    &path,
                    "burn subtitle track has soft-only metadata fields set \
                     (lang/title/default/forced/commentary/hearing_impaired); \
                     burn tracks are pixels and have no metadata channel.",
                ));
            }
        }
    }

    // Multiple burn tracks — config-implication warning.
    if burn_count > 1 && subs.warn_multiple_burns.unwrap_or(true) {
        issues.push(ValidationIssue::warning(
            "subtitles.tracks",
            format!(
                "{} subtitle tracks have mux=\"burn\"; multiple burn layers are \
                 supported but often a misfire. If intentional, set \
                 [subtitles].warn_multiple_burns = false to suppress.",
                burn_count
            ),
        ));
    }

    // External tracks must produce unique sidecar filenames within this config.
    // The filename is <output_stem>.<key>, so uniqueness reduces to key uniqueness.
    validate_external_sidecar_uniqueness(tracks, issues);
}

fn validate_external_sidecar_uniqueness(
    tracks: &[SubtitleTrack],
    issues: &mut Vec<ValidationIssue>,
) {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, track) in tracks.iter().enumerate() {
        if track.mux != Some(SubtitleMux::External) {
            continue;
        }
        let key = external_sidecar_key(track);
        if let Some(&prev) = seen.get(&key) {
            issues.push(ValidationIssue::error(
                format!("subtitles.tracks[{}]", i),
                format!(
                    "external subtitle track would produce the same sidecar filename as \
                     subtitles.tracks[{}] (both resolve to `<stem>.{}`); \
                     give them distinct title, lang, or disposition values.",
                    prev, key,
                ),
            ));
        } else {
            seen.insert(key, i);
        }
    }
}

/// The portion of the sidecar filename after the output stem:
/// `<title?>.<lang?>.<flags?>.<ext>`. Used for duplicate detection.
fn external_sidecar_key(track: &SubtitleTrack) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if let Some(t) = &track.title {
        parts.push(t.as_str());
    }
    if let Some(l) = &track.lang {
        parts.push(l.as_str());
    }
    if track.default == Some(true) {
        parts.push("default");
    }
    if track.forced == Some(true) {
        parts.push("forced");
    }
    if track.hearing_impaired == Some(true) {
        parts.push("sdh");
    }
    let ext = match track.format {
        Some(SubtitleFormat::Ass) => "ass",
        _ => "srt",
    };
    if parts.is_empty() {
        ext.to_string()
    } else {
        format!("{}.{}", parts.join("."), ext)
    }
}

fn has_soft_metadata(track: &SubtitleTrack) -> bool {
    track.lang.is_some()
        || track.title.is_some()
        || track.default.is_some()
        || track.forced.is_some()
        || track.commentary.is_some()
        || track.hearing_impaired.is_some()
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::{Layer, resolve};
    use std::path::PathBuf;

    fn parse(s: &str) -> Config {
        Config::from_toml_str(s).unwrap()
    }

    fn directory() -> Layer {
        Layer::Directory(PathBuf::from("/show/bento.toml"))
    }

    fn check(toml_str: &str) -> Vec<ValidationIssue> {
        let r = resolve(vec![(directory(), parse(toml_str))]);
        validate(&r)
    }

    fn errors(issues: &[ValidationIssue]) -> Vec<&ValidationIssue> {
        issues.iter().filter(|i| i.severity == Severity::Error).collect()
    }

    fn warnings(issues: &[ValidationIssue]) -> Vec<&ValidationIssue> {
        issues.iter().filter(|i| i.severity == Severity::Warning).collect()
    }

    // --- Tune validity ------------------------------------------------------

    #[test]
    fn x265_with_film_tune_errors() {
        let issues = check(r#"
[video]
encoder = { name = "x265", crf = 22, tune = "film" }
"#);
        let errs = errors(&issues);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].path, "video.encoder.tune");
    }

    #[test]
    fn x265_with_stillimage_tune_errors() {
        let issues = check(r#"
[video]
encoder = { name = "x265", crf = 22, tune = "stillimage" }
"#);
        assert_eq!(errors(&issues).len(), 1);
    }

    #[test]
    fn x264_accepts_all_tunes() {
        for tune in ["film", "animation", "grain", "stillimage", "psnr", "ssim", "fastdecode", "zerolatency", "none"] {
            let issues = check(&format!(r#"
[video]
encoder = {{ name = "x264", crf = 20, tune = "{}" }}
"#, tune));
            assert!(errors(&issues).is_empty(), "x264 should accept tune={}", tune);
        }
    }

    #[test]
    fn x265_accepts_animation_grain_etc() {
        for tune in ["animation", "grain", "psnr", "ssim", "fastdecode", "zerolatency", "none"] {
            let issues = check(&format!(r#"
[video]
encoder = {{ name = "x265", crf = 22, tune = "{}" }}
"#, tune));
            assert!(errors(&issues).is_empty(), "x265 should accept tune={}", tune);
        }
    }

    // --- CRF/codec coupling -------------------------------------------------

    #[test]
    fn x265_with_low_crf_warns() {
        let issues = check(r#"
[video]
encoder = { name = "x265", crf = 18 }
"#);
        let warns = warnings(&issues);
        assert!(warns.iter().any(|w| w.path == "video.encoder.crf"));
    }

    #[test]
    fn x264_with_high_crf_warns() {
        let issues = check(r#"
[video]
encoder = { name = "x264", crf = 26 }
"#);
        let warns = warnings(&issues);
        assert!(warns.iter().any(|w| w.path == "video.encoder.crf"));
    }

    #[test]
    fn crf_warning_suppressed_by_config_field() {
        let issues = check(r#"
[video]
encoder = { name = "x265", crf = 18 }
warn_crf_codec_mismatch = false
"#);
        assert!(!warnings(&issues).iter().any(|w| w.path == "video.encoder.crf"));
    }

    #[test]
    fn matched_codec_crf_no_warning() {
        let issues = check(r#"
[video]
encoder = { name = "x265", crf = 22 }
"#);
        assert!(!warnings(&issues).iter().any(|w| w.path == "video.encoder.crf"));
    }

    // --- Audio default uniqueness/absence ----------------------------------

    #[test]
    fn audio_multiple_defaults_errors() {
        let issues = check(r#"
[audio]
tracks = [
    { source = 1, lang = "jpn", default = true },
    { source = 2, lang = "eng", default = true },
]
"#);
        let errs = errors(&issues);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].path, "audio.tracks");
    }

    #[test]
    fn audio_no_default_warns() {
        let issues = check(r#"
[audio]
tracks = [
    { source = 1, lang = "jpn" },
    { source = 2, lang = "eng" },
]
"#);
        let warns = warnings(&issues);
        assert!(warns.iter().any(|w| w.path == "audio.tracks"));
    }

    #[test]
    fn audio_no_default_warning_suppressed() {
        let issues = check(r#"
[audio]
warn_no_default = false
tracks = [{ source = 1, lang = "jpn" }]
"#);
        assert!(!warnings(&issues).iter().any(|w| w.path == "audio.tracks"));
    }

    #[test]
    fn audio_one_default_no_issue() {
        let issues = check(r#"
[audio]
tracks = [
    { source = 1, lang = "jpn", default = true },
    { source = 2, lang = "eng" },
]
"#);
        assert!(errors(&issues).is_empty());
        assert!(!warnings(&issues).iter().any(|w| w.path == "audio.tracks"));
    }

    // --- 1-based source index enforcement (Phase 6d) ----------------------

    #[test]
    fn audio_source_zero_errors_with_one_based_message() {
        let issues = check(r#"
[audio]
tracks = [
    { source = 0, lang = "jpn", default = true },
]
"#);
        let errs = errors(&issues);
        assert!(errs.iter().any(|e| e.path == "audio.tracks[0].source"
            && e.message.contains("1-based")));
    }

    #[test]
    fn subtitles_source_zero_errors_with_one_based_message() {
        let issues = check(r#"
[audio]
tracks = [{ source = 1, lang = "jpn", default = true }]

[subtitles]
tracks = [
    { source = 0, format = "srt", mux = "soft", default = true },
]
"#);
        let errs = errors(&issues);
        assert!(errs.iter().any(|e| e.path == "subtitles.tracks[0].source"
            && e.message.contains("1-based")));
    }

    #[test]
    fn subtitles_subtract_track_zero_errors() {
        let issues = check(r#"
[audio]
tracks = [{ source = 1, lang = "jpn", default = true }]

[subtitles]
tracks = [
    { source = 1, format = "srt", mux = "soft", subtract_track = 0, default = true },
]
"#);
        let errs = errors(&issues);
        assert!(errs.iter().any(|e| e.path == "subtitles.tracks[0].subtract_track"
            && e.message.contains("1-based")));
    }

    #[test]
    fn subtitles_path_typed_source_unaffected_by_range_check() {
        // Path sources don't have a range; only int sources need to be ≥ 1.
        let issues = check(r#"
[audio]
tracks = [{ source = 1, lang = "jpn", default = true }]

[subtitles]
tracks = [
    { source = "edited.srt", format = "srt", mux = "soft", default = true },
]
"#);
        // No source-range error.
        let errs = errors(&issues);
        assert!(!errs.iter().any(|e| e.message.contains("≥ 1")));
    }

    #[test]
    fn audio_source_one_no_range_error() {
        let issues = check(r#"
[audio]
tracks = [
    { source = 1, lang = "jpn", default = true },
    { source = 2, lang = "eng" },
]
"#);
        let errs = errors(&issues);
        assert!(!errs.iter().any(|e| e.message.contains("≥ 1")));
    }

    // --- Subtitles per-track checks ----------------------------------------

    #[test]
    fn subtitles_filter_and_subtract_both_errors() {
        let issues = check(r#"
[subtitles]
tracks = [
    {
        source = 1,
        format = "srt",
        mux = "soft",
        filter = { style = "Main", mode = "retain" },
        subtract_track = 2,
        default = true,
    },
]
"#);
        let errs = errors(&issues);
        assert!(errs.iter().any(|e| e.path == "subtitles.tracks[0]"));
    }

    #[test]
    fn subtitles_multiple_default_errors() {
        let issues = check(r#"
[subtitles]
tracks = [
    { source = 1, format = "srt", mux = "soft", default = true },
    { source = 2, format = "srt", mux = "soft", default = true },
]
"#);
        let errs = errors(&issues);
        assert!(errs.iter().any(|e| e.path == "subtitles.tracks"));
    }

    #[test]
    fn subtitles_multiple_burns_warns() {
        let issues = check(r#"
[subtitles]
tracks = [
    { source = 1, format = "ass", mux = "burn" },
    { source = 2, format = "ass", mux = "burn" },
    { source = 3, format = "srt", mux = "soft", default = true },
]
"#);
        let warns = warnings(&issues);
        assert!(warns.iter().any(|w| w.message.contains("subtitle tracks have mux=\"burn\"")));
    }

    #[test]
    fn subtitles_multiple_burns_suppressed() {
        let issues = check(r#"
[subtitles]
warn_multiple_burns = false
tracks = [
    { source = 1, format = "ass", mux = "burn" },
    { source = 2, format = "ass", mux = "burn" },
    { source = 3, format = "srt", mux = "soft", default = true },
]
"#);
        assert!(!warnings(&issues).iter().any(|w| w.message.contains("subtitle tracks have mux=\"burn\"")));
    }

    #[test]
    fn subtitles_burn_with_metadata_warns() {
        let issues = check(r#"
[subtitles]
tracks = [
    { source = 1, format = "ass", mux = "burn", lang = "eng", title = "Signs" },
    { source = 2, format = "srt", mux = "soft", default = true },
]
"#);
        let warns = warnings(&issues);
        assert!(warns.iter().any(|w| w.message.contains("burn subtitle track")));
    }

    #[test]
    fn subtitles_burn_metadata_suppressed() {
        let issues = check(r#"
[subtitles]
warn_burn_metadata = false
tracks = [
    { source = 1, format = "ass", mux = "burn", lang = "eng", title = "Signs" },
    { source = 2, format = "srt", mux = "soft", default = true },
]
"#);
        assert!(!warnings(&issues).iter().any(|w| w.message.contains("burn subtitle track")));
    }

    // --- External subtitle tracks ------------------------------------------

    #[test]
    fn external_ass_in_mp4_is_allowed() {
        // External tracks are sidecar files; they're not affected by the
        // container codec restriction that bars soft ASS in MP4.
        let issues = check(r#"
[output]
container = "mp4"

[subtitles]
tracks = [
    { source = 1, format = "ass", mux = "external", lang = "eng", default = true },
]
"#);
        assert!(
            !errors(&issues).iter().any(|e| e.message.contains("format=\"ass\"")),
            "external ASS in MP4 should not be an error: {:?}",
            errors(&issues)
        );
    }

    #[test]
    fn soft_ass_in_mp4_still_errors() {
        let issues = check(r#"
[output]
container = "mp4"

[subtitles]
tracks = [
    { source = 1, format = "ass", mux = "soft", lang = "eng", default = true },
]
"#);
        assert!(
            errors(&issues).iter().any(|e| e.message.contains("format=\"ass\"")),
            "soft ASS in MP4 should still error"
        );
    }

    #[test]
    fn external_duplicate_sidecar_names_errors() {
        let issues = check(r#"
[subtitles]
tracks = [
    { source = 1, format = "srt", mux = "external", lang = "eng", title = "English", default = true },
    { source = 2, format = "srt", mux = "external", lang = "eng", title = "English", default = true },
]
"#);
        let errs = errors(&issues);
        assert!(
            errs.iter().any(|e| e.message.contains("same sidecar filename")),
            "duplicate external names should error: {:?}", errs
        );
    }

    #[test]
    fn external_unique_sidecar_names_ok() {
        // Two external tracks that differ only by title → unique filenames.
        let issues = check(r#"
[subtitles]
tracks = [
    { source = 1, format = "srt", mux = "external", lang = "eng", title = "English", default = true },
    { source = 2, format = "srt", mux = "external", lang = "eng", title = "English SDH", hearing_impaired = true },
]
"#);
        assert!(
            !errors(&issues).iter().any(|e| e.message.contains("sidecar")),
            "unique external names should not error: {:?}", errors(&issues)
        );
    }

    #[test]
    fn external_tracks_parse_from_toml() {
        // Smoke-test that mux = "external" round-trips through the config parser.
        let cfg = crate::config::Config::from_toml_str(r#"
[subtitles]
tracks = [
    { source = 1, format = "srt", mux = "external", lang = "eng", default = true },
    { source = 2, format = "ass", mux = "burn" },
]
"#).unwrap();
        use crate::config::SubtitleMux;
        let tracks = cfg.subtitles.tracks.unwrap();
        assert_eq!(tracks[0].mux, Some(SubtitleMux::External));
        assert_eq!(tracks[1].mux, Some(SubtitleMux::Burn));
    }

    // --- Output / naming template validation --------------------------------

    #[test]
    fn naming_template_show_without_metadata_errors() {
        let issues = check(r#"
[output]
naming = { template = "{show} - ep{source_basename}" }
"#);
        let errs = errors(&issues);
        assert!(
            errs.iter().any(|e| e.path == "output.naming.template" && e.message.contains("{show}")),
            "expected undefined-variable error for {{show}}, got: {:?}", errs
        );
    }

    #[test]
    fn naming_template_show_with_metadata_ok() {
        let issues = check(r#"
[output]
metadata = { show = "Cowboy Bebop" }
naming = { template = "{show} - ep{source_basename}" }
"#);
        assert!(
            !errors(&issues).iter().any(|e| e.path == "output.naming.template"),
            "expected no naming error when metadata.show is set: {:?}", errors(&issues)
        );
    }

    #[test]
    fn naming_template_builtin_vars_ok() {
        let issues = check(r#"
[output]
naming = { template = "{source_dir}/{source_basename}" }
"#);
        assert!(
            !errors(&issues).iter().any(|e| e.path == "output.naming.template"),
            "built-in variables should not produce an error: {:?}", errors(&issues)
        );
    }

    #[test]
    fn naming_template_regex_capture_ok() {
        let issues = check(r#"
[output]
naming = {
    regex = 'S(?P<s>\d+)E(?P<episode>\d+)',
    template = "S{s:02}E{episode:02}",
}
"#);
        assert!(
            !errors(&issues).iter().any(|e| e.path == "output.naming.template"),
            "regex capture variables should not produce an error: {:?}", errors(&issues)
        );
    }

    #[test]
    fn naming_template_missing_regex_capture_errors() {
        // Template references {episode} but the regex only has capture {s}.
        let issues = check(r#"
[output]
naming = {
    regex = 'S(?P<s>\d+)',
    template = "S{s:02}E{episode:02}",
}
"#);
        let errs = errors(&issues);
        assert!(
            errs.iter().any(|e| e.path == "output.naming.template" && e.message.contains("{episode}")),
            "expected undefined-variable error for {{episode}}: {:?}", errs
        );
    }

    #[test]
    fn naming_template_unknown_var_no_regex_errors() {
        let issues = check(r#"
[output]
naming = { template = "{myvar}" }
"#);
        let errs = errors(&issues);
        assert!(
            errs.iter().any(|e| e.path == "output.naming.template" && e.message.contains("{myvar}")),
            "expected undefined-variable error for {{myvar}}: {:?}", errs
        );
    }

    #[test]
    fn naming_invalid_regex_errors() {
        let issues = check(r#"
[output]
naming = { regex = "[(invalid" }
"#);
        let errs = errors(&issues);
        assert!(
            errs.iter().any(|e| e.path == "output.naming.regex"),
            "expected regex compile error: {:?}", errs
        );
    }

    #[test]
    fn naming_template_show_via_regex_capture_overrides_absent_metadata() {
        // A regex capture named `show` satisfies {show} in the template even
        // when metadata.show is not set — captures are inserted after metadata
        // in naming.rs and can supply variables metadata doesn't.
        let issues = check(r#"
[output]
naming = {
    regex = '(?P<show>.+) S\d+E\d+',
    template = "{show} episode",
}
"#);
        assert!(
            !errors(&issues).iter().any(|e| e.path == "output.naming.template"),
            "regex capture `show` should satisfy {{show}} even without metadata.show: {:?}",
            errors(&issues)
        );
    }

    #[test]
    fn naming_multiple_undefined_vars_all_reported() {
        let issues = check(r#"
[output]
naming = { template = "{show} S{season:02}" }
"#);
        let errs: Vec<_> = errors(&issues)
            .into_iter()
            .filter(|e| e.path == "output.naming.template")
            .collect();
        assert_eq!(errs.len(), 2, "both {{show}} and {{season}} should error: {:?}", errs);
    }

    #[test]
    fn naming_season_and_year_from_metadata_ok() {
        let issues = check(r#"
[output]
metadata = { show = "Bebop", season = 1, year = 1998 }
naming = { template = "{show} S{season:02} ({year})" }
"#);
        assert!(
            !errors(&issues).iter().any(|e| e.path == "output.naming.template"),
            "all metadata vars set, should be clean: {:?}", errors(&issues)
        );
    }

    // --- Clean configs produce no errors -----------------------------------

    #[test]
    fn canonical_anime_episode_config_validates_clean() {
        let issues = check(r#"
[output]
container = "mp4"
metadata = { show = "Cowboy Bebop", season = 1, year = 1998 }

[video]
encoder = { name = "x264", crf = 20, tune = "animation" }

[audio]
tracks = [
    { source = 1, lang = "jpn", title = "Japanese", default = true, original = true },
    { source = 2, lang = "eng", title = "English Dub" },
]

[subtitles]
tracks = [
    { source = 1, format = "srt", mux = "soft", subtract_track = 2, lang = "eng", title = "English", default = true },
    { source = 2, format = "ass", mux = "burn" },
]
"#);
        assert!(errors(&issues).is_empty(), "expected no errors, got: {:?}", errors(&issues));
        assert!(warnings(&issues).is_empty(), "expected no warnings, got: {:?}", warnings(&issues));
    }
}
