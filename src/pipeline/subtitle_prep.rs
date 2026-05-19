//! Subtitle track preparation: extraction, derivation, and format routing.
//!
//! Produces [`PreparedSubtitle`] values consumed by `pipeline/ffmpeg_args.rs`
//! to assemble the final ffmpeg invocation.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::config::{SubtitleFilter, SubtitleFormat, SubtitleMux, Subtitles, SubtitleTrack, TrackRef};
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

        // Soft-mux path.
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

        prepared.push(PreparedSubtitle::Soft {
            file,
            format: fmt,
            lang: track.lang.clone(),
            title: track.title.clone(),
            is_default: track.default == Some(true),
            is_forced: track.forced == Some(true),
            is_commentary: track.commentary == Some(true),
            is_hearing_impaired: track.hearing_impaired == Some(true),
        });
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

#[cfg(test)]
mod tests {
    use super::*;

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
