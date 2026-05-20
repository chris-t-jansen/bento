//! Subtitle track preparation: extraction, derivation, and format routing.
//!
//! Produces [`PreparedSubtitle`] values consumed by `pipeline/ffmpeg_args.rs`
//! to assemble the final ffmpeg invocation.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::config::{OnExisting, SubtitleFilter, SubtitleFormat, SubtitleMux, Subtitles, SubtitleTrack, TrackRef};
use crate::error::{Error, Result};
use crate::pipeline::probe::SourceProbe;
use crate::resolve::{Layer, Provenance};

/// Resolved subtitle track ready for the ffmpeg invocation.
#[derive(Debug)]
pub enum PreparedSubtitle {
    /// Soft-muxed track. `file` is an SRT or ASS file on disk; `format`
    /// tells `ffmpeg_args` which codec to select.
    Soft {
        file: PathBuf,
        format: SourceFormat,
        lang: Option<String>,
        title: Option<String>,
        is_default: bool,
        is_forced: bool,
        is_commentary: bool,
        is_hearing_impaired: bool,
    },
    /// Burn-in track. `file` is an ASS or SRT file on disk fed to ffmpeg's
    /// `subtitles=` libass filter. Multiple burn tracks are supported via
    /// chained filter entries.
    Burn { file: PathBuf },
    /// External (sidecar) track. `file` is the derived subtitle in a temp
    /// directory; `write_external_sidecars` copies it to the output directory
    /// with a Jellyfin-compatible filename after the encode completes.
    /// `commentary` is intentionally absent — Jellyfin's external filename
    /// convention has no commentary flag.
    External {
        file: PathBuf,
        format: SourceFormat,
        lang: Option<String>,
        title: Option<String>,
        is_default: bool,
        is_forced: bool,
        is_hearing_impaired: bool,
    },
}

/// Whether a subtitle source is in ASS/SSA or SRT format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    Srt,
    Ass,
}

/// Extract, derive, and route all configured subtitle tracks into
/// [`PreparedSubtitle`] values. Multiple burn tracks are allowed; soft ASS
/// tracks are supported (validated at the container level in `validate.rs`).
pub fn prepare_subtitles(
    input: &Path,
    subtitles: &Subtitles,
    provenance: &Provenance,
    probe: &SourceProbe,
    tempdir: &Path,
    out: &mut dyn Write,
) -> Result<Vec<PreparedSubtitle>> {
    let Some(tracks) = &subtitles.tracks else {
        return Ok(Vec::new());
    };
    if tracks.is_empty() {
        return Ok(Vec::new());
    }

    let config_dir: Option<PathBuf> = match provenance.layer_for("subtitles.tracks") {
        Some(Layer::Global(p)) | Some(Layer::Directory(p)) | Some(Layer::PerFile(p)) => {
            p.parent().map(Path::to_path_buf)
        }
        _ => None,
    };

    let mut prepared = Vec::new();

    for (i, track) in tracks.iter().enumerate() {
        let out_format = track.format.unwrap_or(SubtitleFormat::Srt);
        let mux = track.mux.unwrap_or(SubtitleMux::Soft);
        let source = track.source.as_ref().expect("validated by validate.rs");

        if mux == SubtitleMux::Burn {
            let file = prepare_burn_subtitle_file(
                input,
                track,
                source,
                config_dir.as_deref(),
                probe,
                tempdir,
                i,
            )?;
            prepared.push(PreparedSubtitle::Burn { file });
            continue;
        }

        // Soft and External share the same derivation logic; they diverge only
        // in which PreparedSubtitle variant is produced at the end.
        let src_fmt = detect_source_format(input, source, probe)?;

        if track.filter.is_some() && src_fmt == SourceFormat::Srt {
            writeln!(
                out,
                "warning: subtitles.tracks[{}]: `filter` requires an ASS source; \
                 the resolved source is SRT. Track will be skipped.",
                i
            )
            .map_err(crate::io_render_err)?;
            continue;
        }

        if out_format == SubtitleFormat::Ass && src_fmt == SourceFormat::Srt {
            writeln!(
                out,
                "warning: subtitles.tracks[{}]: format=\"ass\" requires an ASS source; \
                 the resolved source is SRT. Track will be skipped.",
                i
            )
            .map_err(crate::io_render_err)?;
            continue;
        }

        let (file, fmt) = match (src_fmt, out_format) {
            (SourceFormat::Srt, SubtitleFormat::Srt) => {
                let srt = derive_srt_from_srt_source(
                    input,
                    track,
                    source,
                    config_dir.as_deref(),
                    tempdir,
                    i,
                )?;
                let path = tempdir.join(format!("track{}-derived.srt", i));
                write_to_disk(&path, crate::subtitles::serialize_srt(&srt))?;
                (path, SourceFormat::Srt)
            }
            (SourceFormat::Ass, SubtitleFormat::Srt) => {
                if subtitles.warn_ass_to_srt.unwrap_or(true) {
                    writeln!(
                        out,
                        "warning: subtitles.tracks[{}]: lossy ASS→SRT conversion \
                         (styling, positioning, fonts, and effects are stripped \
                         to plain text). Suppress with \
                         [subtitles].warn_ass_to_srt = false.",
                        i
                    )
                    .map_err(crate::io_render_err)?;
                }
                let srt = derive_srt_from_ass_source(
                    input,
                    track,
                    source,
                    config_dir.as_deref(),
                    tempdir,
                    i,
                )?;
                let path = tempdir.join(format!("track{}-derived.srt", i));
                write_to_disk(&path, crate::subtitles::serialize_srt(&srt))?;
                (path, SourceFormat::Srt)
            }
            (SourceFormat::Ass, SubtitleFormat::Ass) => {
                let path = derive_soft_ass(
                    input,
                    track,
                    source,
                    config_dir.as_deref(),
                    tempdir,
                    i,
                )?;
                (path, SourceFormat::Ass)
            }
            (SourceFormat::Srt, SubtitleFormat::Ass) => {
                unreachable!("SRT+ASS combination is filtered out above")
            }
        };

        match mux {
            SubtitleMux::Soft => prepared.push(PreparedSubtitle::Soft {
                file,
                format: fmt,
                lang: track.lang.clone(),
                title: track.title.clone(),
                is_default: track.default == Some(true),
                is_forced: track.forced == Some(true),
                is_commentary: track.commentary == Some(true),
                is_hearing_impaired: track.hearing_impaired == Some(true),
            }),
            SubtitleMux::External => prepared.push(PreparedSubtitle::External {
                file,
                format: fmt,
                lang: track.lang.clone(),
                title: track.title.clone(),
                is_default: track.default == Some(true),
                is_forced: track.forced == Some(true),
                is_hearing_impaired: track.hearing_impaired == Some(true),
            }),
            SubtitleMux::Burn => unreachable!("handled above"),
        }
    }

    Ok(prepared)
}

/// Prepare the subtitle file for a burn track. Always produces a file on disk
/// that ffmpeg's `subtitles=` filter can read (ASS or SRT). Int sources
/// without derivation are extracted; path sources without derivation are used
/// directly. Any derivation (filter/subtract) first loads the source as ASS,
/// applies the operation in-format, then writes a derived ASS file.
pub fn prepare_burn_subtitle_file(
    input: &Path,
    track: &SubtitleTrack,
    source: &TrackRef,
    config_dir: Option<&Path>,
    probe: &SourceProbe,
    tempdir: &Path,
    track_idx: usize,
) -> Result<PathBuf> {
    let needs_derivation = track.filter.is_some() || track.subtract_track.is_some();

    if needs_derivation {
        let source_ass = load_ass_source(input, source, config_dir, tempdir, track_idx)?;
        let derived_ass = apply_ass_derivation(
            input,
            source_ass,
            &track.filter,
            &track.subtract_track,
            config_dir,
            tempdir,
            track_idx,
        )?;
        let path = tempdir.join(format!("track{}-burn-derived.ass", track_idx));
        write_to_disk(&path, crate::subtitles::serialize_ass(&derived_ass))?;
        Ok(path)
    } else {
        match source {
            TrackRef::Index(idx) => {
                let src_fmt = match probe.subtitle_codec(*idx) {
                    Some(c) if c == "ass" || c == "ssa" => SourceFormat::Ass,
                    _ => SourceFormat::Srt,
                };
                let ext = if src_fmt == SourceFormat::Ass { "ass" } else { "srt" };
                let extracted = tempdir.join(format!("track{}-burn-source.{}", track_idx, ext));
                extract_subtitle_track(input, *idx, &extracted)?;
                Ok(extracted)
            }
            TrackRef::Path(p) => {
                let path = resolve_subtitle_path(p, config_dir);
                if !path.exists() {
                    return Err(Error::PathNotFound(path));
                }
                Ok(path)
            }
        }
    }
}

/// Infer subtitle source format from a file path's extension.
pub fn infer_format_from_path(path: &str) -> Option<SourceFormat> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".srt") {
        Some(SourceFormat::Srt)
    } else if lower.ends_with(".ass") || lower.ends_with(".ssa") {
        Some(SourceFormat::Ass)
    } else {
        None
    }
}

/// Resolve a path-typed subtitle source. Absolute paths pass through;
/// relative paths resolve against the config file that defined them.
pub fn resolve_subtitle_path(track_path: &str, config_dir: Option<&Path>) -> PathBuf {
    let p = Path::new(track_path);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    match config_dir {
        Some(dir) => dir.join(p),
        None => p.to_path_buf(),
    }
}

// --- Private helpers ---------------------------------------------------------

fn detect_source_format(_input: &Path, source: &TrackRef, probe: &SourceProbe) -> Result<SourceFormat> {
    match source {
        TrackRef::Index(idx) => Ok(match probe.subtitle_codec(*idx) {
            Some(c) if c == "ass" || c == "ssa" => SourceFormat::Ass,
            _ => SourceFormat::Srt,
        }),
        TrackRef::Path(p) => infer_format_from_path(p).ok_or_else(|| Error::SubtitleParse {
            path: PathBuf::from(p.as_str()),
            line: 0,
            message: "unrecognized subtitle file extension; expected .srt, .ass, or .ssa"
                .to_string(),
        }),
    }
}

fn derive_srt_from_srt_source(
    input: &Path,
    track: &SubtitleTrack,
    source: &TrackRef,
    config_dir: Option<&Path>,
    tempdir: &Path,
    track_idx: usize,
) -> Result<crate::subtitles::Srt> {
    let source_srt = load_srt_source(input, source, config_dir, tempdir, track_idx)?;

    if let Some(subtrahend_ref) = &track.subtract_track {
        let subtrahend = load_srt_source(input, subtrahend_ref, config_dir, tempdir, track_idx)?;
        Ok(crate::subtitles::subtract_by_timestamp(&source_srt, &subtrahend))
    } else {
        Ok(source_srt)
    }
}

fn derive_srt_from_ass_source(
    input: &Path,
    track: &SubtitleTrack,
    source: &TrackRef,
    config_dir: Option<&Path>,
    tempdir: &Path,
    track_idx: usize,
) -> Result<crate::subtitles::Srt> {
    let source_ass = load_ass_source(input, source, config_dir, tempdir, track_idx)?;
    let derived_ass = if let Some(subtrahend_ref) = &track.subtract_track {
        let subtrahend = load_ass_source(input, subtrahend_ref, config_dir, tempdir, track_idx)?;
        crate::subtitles::subtract_ass_by_timestamp(&source_ass, &subtrahend)
    } else if let Some(filter) = &track.filter {
        crate::subtitles::filter_ass(&source_ass, filter)
    } else {
        source_ass
    };
    Ok(crate::subtitles::ass_to_srt(&derived_ass))
}

fn derive_soft_ass(
    input: &Path,
    track: &SubtitleTrack,
    source: &TrackRef,
    config_dir: Option<&Path>,
    tempdir: &Path,
    track_idx: usize,
) -> Result<PathBuf> {
    let source_ass = load_ass_source(input, source, config_dir, tempdir, track_idx)?;
    let derived_ass = apply_ass_derivation(
        input,
        source_ass,
        &track.filter,
        &track.subtract_track,
        config_dir,
        tempdir,
        track_idx,
    )?;
    let path = tempdir.join(format!("track{}-derived.ass", track_idx));
    write_to_disk(&path, crate::subtitles::serialize_ass(&derived_ass))?;
    Ok(path)
}

/// Apply one optional derivation (filter xor subtract) to an ASS document.
/// Validation enforces at most one derivation per track; when neither is set
/// the source is returned unchanged.
fn apply_ass_derivation(
    input: &Path,
    source: crate::subtitles::Ass,
    filter: &Option<SubtitleFilter>,
    subtract_track: &Option<TrackRef>,
    config_dir: Option<&Path>,
    tempdir: &Path,
    track_idx: usize,
) -> Result<crate::subtitles::Ass> {
    if let Some(f) = filter {
        Ok(crate::subtitles::filter_ass(&source, f))
    } else if let Some(subtrahend_ref) = subtract_track {
        let subtrahend = load_ass_source(input, subtrahend_ref, config_dir, tempdir, track_idx)?;
        Ok(crate::subtitles::subtract_ass_by_timestamp(&source, &subtrahend))
    } else {
        Ok(source)
    }
}

fn load_srt_source(
    input: &Path,
    source: &TrackRef,
    config_dir: Option<&Path>,
    tempdir: &Path,
    track_idx: usize,
) -> Result<crate::subtitles::Srt> {
    match source {
        TrackRef::Index(idx) => {
            let extracted = tempdir.join(format!("track{}-source-{}.srt", track_idx, idx));
            extract_subtitle_track(input, *idx, &extracted)?;
            parse_srt_file(&extracted)
        }
        TrackRef::Path(p) => {
            let path = resolve_subtitle_path(p, config_dir);
            if !path.exists() {
                return Err(Error::PathNotFound(path));
            }
            parse_srt_file(&path)
        }
    }
}

fn load_ass_source(
    input: &Path,
    source: &TrackRef,
    config_dir: Option<&Path>,
    tempdir: &Path,
    track_idx: usize,
) -> Result<crate::subtitles::Ass> {
    match source {
        TrackRef::Index(idx) => {
            let extracted = tempdir.join(format!("track{}-source-{}.ass", track_idx, idx));
            extract_subtitle_track(input, *idx, &extracted)?;
            parse_ass_file(&extracted)
        }
        TrackRef::Path(p) => {
            let path = resolve_subtitle_path(p, config_dir);
            if !path.exists() {
                return Err(Error::PathNotFound(path));
            }
            parse_ass_file(&path)
        }
    }
}

fn extract_subtitle_track(input: &Path, track_index: u32, output: &Path) -> Result<()> {
    let zero_based = track_index.saturating_sub(1);
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-i",
            &input.display().to_string(),
            "-map",
            &format!("0:s:{}", zero_based),
            "-c:s",
            "copy",
            &output.display().to_string(),
        ])
        .status()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::FfmpegNotFound
            } else {
                Error::Io {
                    path: PathBuf::from("ffmpeg"),
                    source: e,
                }
            }
        })?;

    if !status.success() {
        return Err(Error::FfmpegExtractFailed {
            status: status.code().unwrap_or(-1),
            track: track_index,
            input: input.to_path_buf(),
        });
    }
    Ok(())
}

fn parse_ass_file(path: &Path) -> Result<crate::subtitles::Ass> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    crate::subtitles::parse_ass(&text).map_err(|e| Error::SubtitleParse {
        path: path.to_path_buf(),
        line: e.line,
        message: e.message,
    })
}

fn parse_srt_file(path: &Path) -> Result<crate::subtitles::Srt> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    crate::subtitles::parse_srt(&text).map_err(|e| Error::SubtitleParse {
        path: path.to_path_buf(),
        line: e.line,
        message: e.message,
    })
}

fn write_to_disk(path: &Path, content: String) -> Result<()> {
    std::fs::write(path, content).map_err(|e| Error::Io {
        path: path.to_path_buf(),
        source: e,
    })
}

// =============================================================================
// External sidecar writing
// =============================================================================

/// Build the Jellyfin-compatible sidecar filename for an external subtitle
/// track: `<output_stem>.<title?>.<lang?>.<flags?>.<ext>`.
///
/// Flags in order: `default`, `forced`, `sdh` (hearing-impaired).
/// Commentary is intentionally excluded — Jellyfin's filename convention has
/// no commentary flag.
pub fn build_sidecar_filename(
    output_stem: &str,
    format: SourceFormat,
    title: Option<&str>,
    lang: Option<&str>,
    is_default: bool,
    is_forced: bool,
    is_hearing_impaired: bool,
) -> String {
    let mut parts: Vec<&str> = vec![output_stem];
    if let Some(t) = title {
        parts.push(t);
    }
    if let Some(l) = lang {
        parts.push(l);
    }
    if is_default {
        parts.push("default");
    }
    if is_forced {
        parts.push("forced");
    }
    if is_hearing_impaired {
        parts.push("sdh");
    }
    let ext = match format {
        SourceFormat::Srt => "srt",
        SourceFormat::Ass => "ass",
    };
    format!("{}.{}", parts.join("."), ext)
}

/// Write all `PreparedSubtitle::External` tracks as sidecar files next to the
/// output video. Called after the ffmpeg encode completes successfully.
///
/// The `[output].on_existing` policy is applied to each sidecar independently:
/// `warn` and `skip_silently` skip the sidecar write; `overwrite` replaces it;
/// `fail` returns an error for the whole file.
pub fn write_external_sidecars(
    prepared_subs: &[PreparedSubtitle],
    output_path: &Path,
    on_existing: OnExisting,
    out: &mut dyn Write,
) -> Result<()> {
    let output_stem = output_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let output_dir = output_path.parent().unwrap_or(Path::new("."));

    for sub in prepared_subs {
        let PreparedSubtitle::External {
            file,
            format,
            lang,
            title,
            is_default,
            is_forced,
            is_hearing_impaired,
        } = sub
        else {
            continue;
        };

        let sidecar_name = build_sidecar_filename(
            &output_stem,
            *format,
            title.as_deref(),
            lang.as_deref(),
            *is_default,
            *is_forced,
            *is_hearing_impaired,
        );
        let sidecar_path = output_dir.join(&sidecar_name);

        if sidecar_path.exists() {
            match on_existing {
                OnExisting::Warn => {
                    writeln!(
                        out,
                        "warning: external subtitle sidecar already exists, skipping: {}",
                        sidecar_path.display()
                    )
                    .map_err(crate::io_render_err)?;
                    continue;
                }
                OnExisting::SkipSilently => continue,
                OnExisting::Overwrite => {}
                OnExisting::Fail => return Err(Error::OutputExists { path: sidecar_path }),
            }
        }

        std::fs::copy(file, &sidecar_path).map_err(|e| Error::Io {
            path: sidecar_path.clone(),
            source: e,
        })?;
        writeln!(out, "External subtitle: {}", sidecar_path.display())
            .map_err(crate::io_render_err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- build_sidecar_filename ----------------------------------------------

    #[test]
    fn design_doc_example_filename() {
        // lang="eng", title="English", default=true, format=srt →
        // "episode06.English.eng.default.srt"
        let name = build_sidecar_filename("episode06", SourceFormat::Srt,
            Some("English"), Some("eng"), true, false, false);
        assert_eq!(name, "episode06.English.eng.default.srt");
    }

    #[test]
    fn sidecar_all_flags() {
        let name = build_sidecar_filename("ep01", SourceFormat::Ass,
            Some("Signs"), Some("jpn"), false, true, true);
        assert_eq!(name, "ep01.Signs.jpn.forced.sdh.ass");
    }

    #[test]
    fn sidecar_no_optional_fields() {
        // No title, no lang, no flags — minimal sidecar name.
        let name = build_sidecar_filename("ep01", SourceFormat::Srt,
            None, None, false, false, false);
        assert_eq!(name, "ep01.srt");
    }

    #[test]
    fn sidecar_sdh_not_hi_or_cc() {
        // DESIGN.md mandates "sdh" (not "hi" or "cc") to avoid hi-as-Hindi ambiguity.
        let name = build_sidecar_filename("ep01", SourceFormat::Srt,
            None, Some("eng"), false, false, true);
        assert_eq!(name, "ep01.eng.sdh.srt");
        assert!(!name.contains(".hi.") && !name.contains(".cc."));
    }

    #[test]
    fn sidecar_flag_order_default_forced_sdh() {
        // Flags must appear in declaration order: default, forced, sdh.
        let name = build_sidecar_filename("ep", SourceFormat::Srt,
            None, None, true, true, true);
        assert_eq!(name, "ep.default.forced.sdh.srt");
    }

    // --- write_external_sidecars ---------------------------------------------

    #[test]
    fn write_external_sidecars_creates_file() {
        use crate::config::OnExisting;
        use std::io::Cursor;
        use tempfile::tempdir;

        let src_dir = tempdir().unwrap();
        let out_dir = tempdir().unwrap();

        // Write a small temp subtitle file representing a derived track.
        let temp_sub = src_dir.path().join("track0-derived.srt");
        std::fs::write(&temp_sub, "1\n00:00:01,000 --> 00:00:02,000\nHello\n\n").unwrap();

        let prepared = vec![PreparedSubtitle::External {
            file: temp_sub,
            format: SourceFormat::Srt,
            lang: Some("eng".into()),
            title: Some("English".into()),
            is_default: true,
            is_forced: false,
            is_hearing_impaired: false,
        }];

        let output_path = out_dir.path().join("episode01.mp4");
        let mut log = Cursor::new(Vec::<u8>::new());
        write_external_sidecars(&prepared, &output_path, OnExisting::Warn, &mut log).unwrap();

        let expected = out_dir.path().join("episode01.English.eng.default.srt");
        assert!(expected.exists(), "sidecar file should have been created");
    }

    #[test]
    fn write_external_sidecars_warn_on_existing() {
        use crate::config::OnExisting;
        use std::io::Cursor;
        use tempfile::tempdir;

        let src_dir = tempdir().unwrap();
        let out_dir = tempdir().unwrap();

        let temp_sub = src_dir.path().join("track0-derived.srt");
        std::fs::write(&temp_sub, "content").unwrap();

        let sidecar = out_dir.path().join("ep.eng.srt");
        std::fs::write(&sidecar, "old content").unwrap();

        let prepared = vec![PreparedSubtitle::External {
            file: temp_sub,
            format: SourceFormat::Srt,
            lang: Some("eng".into()),
            title: None,
            is_default: false,
            is_forced: false,
            is_hearing_impaired: false,
        }];

        let output_path = out_dir.path().join("ep.mp4");
        let mut log = Cursor::new(Vec::<u8>::new());
        write_external_sidecars(&prepared, &output_path, OnExisting::Warn, &mut log).unwrap();

        // File should be untouched; log should contain a warning.
        assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), "old content");
        let log_str = String::from_utf8(log.into_inner()).unwrap();
        assert!(log_str.contains("warning"), "expected a warning in log: {}", log_str);
    }

    #[test]
    fn write_external_sidecars_overwrite_replaces_file() {
        use crate::config::OnExisting;
        use std::io::Cursor;
        use tempfile::tempdir;

        let src_dir = tempdir().unwrap();
        let out_dir = tempdir().unwrap();

        let temp_sub = src_dir.path().join("track0-derived.srt");
        std::fs::write(&temp_sub, "new content").unwrap();

        let sidecar = out_dir.path().join("ep.eng.srt");
        std::fs::write(&sidecar, "old content").unwrap();

        let prepared = vec![PreparedSubtitle::External {
            file: temp_sub,
            format: SourceFormat::Srt,
            lang: Some("eng".into()),
            title: None,
            is_default: false,
            is_forced: false,
            is_hearing_impaired: false,
        }];

        let output_path = out_dir.path().join("ep.mp4");
        let mut log = Cursor::new(Vec::<u8>::new());
        write_external_sidecars(&prepared, &output_path, OnExisting::Overwrite, &mut log).unwrap();

        assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), "new content");
    }

    #[test]
    fn write_external_sidecars_fail_returns_error() {
        use crate::config::OnExisting;
        use std::io::Cursor;
        use tempfile::tempdir;

        let src_dir = tempdir().unwrap();
        let out_dir = tempdir().unwrap();

        let temp_sub = src_dir.path().join("track0-derived.srt");
        std::fs::write(&temp_sub, "content").unwrap();

        let sidecar = out_dir.path().join("ep.eng.srt");
        std::fs::write(&sidecar, "existing").unwrap();

        let prepared = vec![PreparedSubtitle::External {
            file: temp_sub,
            format: SourceFormat::Srt,
            lang: Some("eng".into()),
            title: None,
            is_default: false,
            is_forced: false,
            is_hearing_impaired: false,
        }];

        let output_path = out_dir.path().join("ep.mp4");
        let mut log = Cursor::new(Vec::<u8>::new());
        let result = write_external_sidecars(&prepared, &output_path, OnExisting::Fail, &mut log);
        assert!(matches!(result, Err(crate::error::Error::OutputExists { .. })));
    }

    #[test]
    fn infer_format_recognizes_extensions() {
        assert_eq!(infer_format_from_path("foo.srt"), Some(SourceFormat::Srt));
        assert_eq!(infer_format_from_path("foo.SRT"), Some(SourceFormat::Srt));
        assert_eq!(infer_format_from_path("foo.ass"), Some(SourceFormat::Ass));
        assert_eq!(infer_format_from_path("foo.ssa"), Some(SourceFormat::Ass));
        assert_eq!(infer_format_from_path("foo.ASS"), Some(SourceFormat::Ass));
        assert_eq!(infer_format_from_path("foo.txt"), None);
        assert_eq!(infer_format_from_path("foo"), None);
    }

    #[test]
    fn resolve_subtitle_path_uses_config_dir_for_relative() {
        let config_dir = PathBuf::from("/show/season1");
        let resolved = resolve_subtitle_path("edited.srt", Some(&config_dir));
        assert_eq!(resolved, PathBuf::from("/show/season1/edited.srt"));
    }

    #[test]
    fn resolve_subtitle_path_passes_absolute_through() {
        let resolved = resolve_subtitle_path("/abs/path/file.srt", Some(&PathBuf::from("/show")));
        assert_eq!(resolved, PathBuf::from("/abs/path/file.srt"));
    }

    #[test]
    fn resolve_subtitle_path_falls_back_when_no_config_dir() {
        let resolved = resolve_subtitle_path("foo.srt", None);
        assert_eq!(resolved, PathBuf::from("foo.srt"));
    }
}
