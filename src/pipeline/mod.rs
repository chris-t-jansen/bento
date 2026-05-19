//! Encode pipeline: probe, prepare, and invoke ffmpeg.

pub mod ffmpeg_args;
pub mod probe;
pub mod subtitle_prep;

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::config::{Config, Container, OnExisting};
use crate::error::{Error, Result};
use crate::layers::discover_layers;
use crate::resolve::resolve;
use crate::validate::{Severity, validate};
use ffmpeg_args::build_ffmpeg_args;
use probe::{probe_cropdetect, probe_source_streams};
use subtitle_prep::prepare_subtitles;

pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "m4v", "avi", "mov", "webm", "ts", "m2ts", "wmv",
];

pub fn is_video_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| VIDEO_EXTENSIONS.iter().any(|v| v.eq_ignore_ascii_case(e)))
        .unwrap_or(false)
}

pub fn run_convert(
    input: &Path,
    output_dir_override: Option<&Path>,
    on_existing_override: Option<OnExisting>,
    out: &mut dyn Write,
) -> Result<()> {
    if !input.exists() {
        return Err(Error::PathNotFound(input.to_path_buf()));
    }
    if input.is_dir() {
        return run_convert_directory(input, output_dir_override, on_existing_override, out);
    }
    run_convert_file(input, output_dir_override, on_existing_override, out)
}

fn run_convert_directory(
    input_dir: &Path,
    output_dir_override: Option<&Path>,
    on_existing_override: Option<OnExisting>,
    out: &mut dyn Write,
) -> Result<()> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(input_dir)
        .map_err(|e| Error::Io {
            path: input_dir.to_path_buf(),
            source: e,
        })?
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_video_extension(p))
        .collect();
    files.sort();

    if files.is_empty() {
        writeln!(out, "No video files found in {}.", input_dir.display())
            .map_err(crate::io_render_err)?;
        return Ok(());
    }

    let mut succeeded: Vec<PathBuf> = Vec::new();
    let mut failed: Vec<(PathBuf, String)> = Vec::new();

    for (idx, file) in files.iter().enumerate() {
        if idx > 0 {
            writeln!(out).map_err(crate::io_render_err)?;
        }
        match run_convert_file(file, output_dir_override, on_existing_override, out) {
            Ok(()) => succeeded.push(file.clone()),
            Err(e) => {
                if matches!(e, Error::FfmpegNotFound) {
                    return Err(e);
                }
                let msg = e.to_string();
                writeln!(out, "[error] {}: {}", file.display(), msg)
                    .map_err(crate::io_render_err)?;
                failed.push((file.clone(), msg));
            }
        }
    }

    writeln!(out).map_err(crate::io_render_err)?;
    writeln!(
        out,
        "{} succeeded, {} failed{}",
        succeeded.len(),
        failed.len(),
        if failed.is_empty() { "." } else { ":" }
    )
    .map_err(crate::io_render_err)?;
    for (path, err) in &failed {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        writeln!(out, "  {}: {}", name, err).map_err(crate::io_render_err)?;
    }

    if !failed.is_empty() {
        return Err(Error::BatchFailed {
            count: failed.len(),
        });
    }
    Ok(())
}

fn run_convert_file(
    input: &Path,
    output_dir_override: Option<&Path>,
    on_existing_override: Option<OnExisting>,
    out: &mut dyn Write,
) -> Result<()> {
    let layers = discover_layers(input, out)?;
    let resolved = resolve(layers);
    let issues = validate(&resolved);

    let error_count = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .count();
    let warning_count = issues
        .iter()
        .filter(|i| i.severity == Severity::Warning)
        .count();
    for issue in &issues {
        let label = match issue.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        writeln!(out, "[{}] {}: {}", label, issue.path, issue.message)
            .map_err(crate::io_render_err)?;
    }
    if error_count > 0 {
        return Err(Error::ConfigInvalid {
            path: input.to_path_buf(),
            errors: error_count,
            warnings: warning_count,
        });
    }

    resolved
        .config
        .audio
        .tracks
        .as_ref()
        .ok_or_else(|| Error::RequiredFieldMissing {
            field: "audio.tracks".to_string(),
        })?;

    let output_path = compute_output_path(input, &resolved.config, output_dir_override)?;

    let on_existing = on_existing_override
        .or(resolved.config.output.on_existing)
        .unwrap_or(OnExisting::Warn);
    if output_path.exists() {
        match on_existing {
            OnExisting::Warn => {
                writeln!(
                    out,
                    "warning: output exists, skipping: {}",
                    output_path.display()
                )
                .map_err(crate::io_render_err)?;
                return Ok(());
            }
            OnExisting::SkipSilently => return Ok(()),
            OnExisting::Overwrite => {}
            OnExisting::Fail => {
                return Err(Error::OutputExists { path: output_path });
            }
        }
    }

    // Probe source streams once; shared by subtitle prep and audio decisions.
    let probe = probe_source_streams(input)?;

    // Run cropdetect pre-pass if the config requests automatic crop detection.
    let crop_params: Option<String> = match &resolved.config.video.crop {
        Some(crate::config::Crop::Mode(crate::config::CropMode::Auto)) => {
            probe_cropdetect(input)?
        }
        _ => None,
    };

    let temp_dir = tempfile::tempdir().map_err(|e| Error::Io {
        path: PathBuf::from("<tempdir>"),
        source: e,
    })?;

    let prepared_subs = prepare_subtitles(
        input,
        &resolved.config.subtitles,
        &resolved.provenance,
        &probe,
        temp_dir.path(),
        out,
    )?;

    let args = build_ffmpeg_args(
        input,
        &output_path,
        &resolved.config,
        &probe,
        &prepared_subs,
        crop_params.as_deref(),
    );

    writeln!(
        out,
        "Converting {} → {}",
        input.display(),
        output_path.display()
    )
    .map_err(crate::io_render_err)?;

    let status = std::process::Command::new("ffmpeg")
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
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
        return Err(Error::FfmpegEncodeFailed {
            status: status.code().unwrap_or(-1),
            input: input.to_path_buf(),
        });
    }

    writeln!(out, "Done. Output at {}", output_path.display())
        .map_err(crate::io_render_err)?;
    Ok(())
}

fn compute_output_path(
    input: &Path,
    config: &Config,
    output_dir_override: Option<&Path>,
) -> Result<PathBuf> {
    let container = config.output.container.unwrap_or(Container::Mp4);
    let extension = match container {
        Container::Mp4 => "mp4",
        Container::Mkv => "mkv",
    };

    let stem = input
        .file_stem()
        .ok_or_else(|| Error::PathNotFound(input.to_path_buf()))?;
    let mut output_name = stem.to_owned();
    output_name.push(".");
    output_name.push(extension);

    let destination = if let Some(override_path) = output_dir_override {
        override_path.to_path_buf()
    } else {
        let dest_str = config.output.destination.as_deref().unwrap_or(".");
        let dest_path = Path::new(dest_str);
        if dest_path.is_absolute() {
            dest_path.to_path_buf()
        } else {
            input.parent().unwrap_or(Path::new(".")).join(dest_path)
        }
    };

    if !destination.exists() {
        std::fs::create_dir_all(&destination).map_err(|e| Error::Io {
            path: destination.clone(),
            source: e,
        })?;
    }

    Ok(destination.join(output_name))
}
