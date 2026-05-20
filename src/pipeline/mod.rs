//! Encode pipeline: probe, prepare, and invoke ffmpeg.

pub mod ffmpeg_args;
pub mod naming;
pub mod probe;
pub mod subtitle_prep;

use std::io::Write;
use std::path::{Path, PathBuf};

use console::style;

use crate::config::{Config, Container, OnExisting};
use crate::error::{Error, Result};
use crate::layers::discover_layers;
use crate::progress::FileProgress;
use crate::resolve::resolve;
use crate::validate::{Severity, validate};
use crate::verbosity::Verbosity;
use ffmpeg_args::build_ffmpeg_args;
use naming::compute_output_stem;
use probe::{probe_cropdetect, probe_source_streams};
use subtitle_prep::{prepare_subtitles, write_external_sidecars};

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
    verbosity: Verbosity,
    out: &mut dyn Write,
) -> Result<()> {
    if !input.exists() {
        return Err(Error::PathNotFound(input.to_path_buf()));
    }
    if input.is_dir() {
        return run_convert_directory(
            input,
            output_dir_override,
            on_existing_override,
            verbosity,
            out,
        );
    }

    // Single-file: print a unified header (same style as the directory path),
    // then run, then always print the summary (per user request — it's the
    // "Bento is finished" signifier even for a single file). Environmental
    // errors (ffmpeg not found) skip the summary since it wouldn't be meaningful.
    let input_dir = input.parent().unwrap_or_else(|| Path::new("."));
    print_convert_header(input_dir, &[input.to_path_buf()], out)?;

    let result =
        run_convert_file(input, 1, 1, output_dir_override, on_existing_override, verbosity, out);

    // Build a one-element failed list (or empty on success) for the summary.
    match &result {
        Err(e) if matches!(e, Error::FfmpegNotFound) => return result,
        _ => {}
    }
    let failed: Vec<(PathBuf, String)> = match &result {
        Ok(()) => vec![],
        Err(e) => vec![(input.to_path_buf(), e.to_string())],
    };
    let succeeded = if failed.is_empty() { 1 } else { 0 };
    print_batch_summary(succeeded, &failed, out)?;
    result
}

fn run_convert_directory(
    input_dir: &Path,
    output_dir_override: Option<&Path>,
    on_existing_override: Option<OnExisting>,
    verbosity: Verbosity,
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

    // Header: show what Bento is about to process before any encode starts.
    print_convert_header(input_dir, &files, out)?;

    let file_count = files.len();
    let mut succeeded: Vec<PathBuf> = Vec::new();
    let mut failed: Vec<(PathBuf, String)> = Vec::new();

    for (idx, file) in files.iter().enumerate() {
        match run_convert_file(
            file,
            idx + 1,
            file_count,
            output_dir_override,
            on_existing_override,
            verbosity,
            out,
        ) {
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

    print_batch_summary(succeeded.len(), &failed, out)?;

    if !failed.is_empty() {
        return Err(Error::BatchFailed {
            count: failed.len(),
        });
    }
    Ok(())
}

fn run_convert_file(
    input: &Path,
    file_idx: usize,
    file_count: usize,
    output_dir_override: Option<&Path>,
    on_existing_override: Option<OnExisting>,
    verbosity: Verbosity,
    out: &mut dyn Write,
) -> Result<()> {
    // input_name is used in the layer-count summary and throughout; compute early.
    let input_name = input
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| input.display().to_string());

    // --- Config resolution and validation ------------------------------------
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

    // --- Per-file layer-count summary (DESIGN.md §Visibility mechanisms) -----
    // Computed for Default mode; embedded inside the FileProgress display rather
    // than printed as a standalone line. Quiet and Verbose modes receive "".
    let config_summary = if verbosity == Verbosity::Default {
        let counts = resolved.provenance.count_by_kind();
        let total = resolved.provenance.len();
        let per_file_n = counts.get("per-file").copied().unwrap_or(0);
        let directory_n = counts.get("directory").copied().unwrap_or(0);
        let global_n = counts.get("global").copied().unwrap_or(0);
        let defaults_n = counts.get("defaults").copied().unwrap_or(0);
        format!(
            "{} settings ({} from file, {} from directory, {} from global, {} from baked-in defaults)",
            total, per_file_n, directory_n, global_n, defaults_n,
        )
    } else {
        String::new()
    };

    // --- Output path and on_existing resolution ------------------------------
    let (output_path, episode_number) =
        compute_output_path(input, &resolved.config, output_dir_override)?;

    let on_existing = on_existing_override
        .or(resolved.config.output.on_existing)
        .unwrap_or(OnExisting::Warn);

    let output_name = output_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| output_path.display().to_string());

    if output_path.exists() {
        match on_existing {
            OnExisting::Warn => {
                writeln!(
                    out,
                    "warning: output exists, skipping: {}",
                    output_path.display()
                )
                .map_err(crate::io_render_err)?;
                // Create a progress display just to emit the skip line.
                let progress = FileProgress::new(
                    &input_name,
                    &output_name,
                    file_idx,
                    file_count,
                    None,
                    &config_summary,
                    verbosity,
                );
                progress.finish_skip("output exists");
                return Ok(());
            }
            OnExisting::SkipSilently => return Ok(()),
            OnExisting::Overwrite => {}
            OnExisting::Fail => {
                return Err(Error::OutputExists { path: output_path });
            }
        }
    }

    // --- Probe source (gets duration for progress bar) -----------------------
    let probe = probe_source_streams(input)?;

    // --- Create progress display ---------------------------------------------
    let progress = FileProgress::new(
        &input_name,
        &output_name,
        file_idx,
        file_count,
        probe.duration_secs,
        &config_summary,
        verbosity,
    );

    // --- Cropdetect pre-pass (if needed) -------------------------------------
    let crop_params: Option<String> = match &resolved.config.video.crop {
        Some(crate::config::Crop::Mode(crate::config::CropMode::Auto)) => {
            probe_cropdetect(input)?
        }
        _ => None,
    };

    // --- Subtitle preparation ------------------------------------------------
    let temp_dir = tempfile::tempdir().map_err(|e| Error::Io {
        path: PathBuf::from("<tempdir>"),
        source: e,
    })?;

    let prepared_subs = match prepare_subtitles(
        input,
        &resolved.config.subtitles,
        &resolved.provenance,
        &probe,
        temp_dir.path(),
        out,
    ) {
        Ok(subs) => subs,
        Err(e) => {
            progress.finish_err();
            return Err(e);
        }
    };

    // --- Build ffmpeg args ---------------------------------------------------
    let base_args = build_ffmpeg_args(
        input,
        &output_path,
        &resolved.config,
        &probe,
        &prepared_subs,
        crop_params.as_deref(),
        episode_number,
    );

    // --- Verbose: print the ffmpeg command line ------------------------------
    if verbosity == Verbosity::Verbose {
        writeln!(out, "ffmpeg {}", base_args.join(" ")).map_err(crate::io_render_err)?;
    }

    // --- Run encode ----------------------------------------------------------
    match run_ffmpeg_encode(&base_args, input, &progress, verbosity) {
        Ok(()) => {}
        Err(e) => {
            progress.finish_err();
            return Err(e);
        }
    }

    // --- Write external subtitle sidecars ------------------------------------
    if let Err(e) = write_external_sidecars(&prepared_subs, &output_path, on_existing, out) {
        progress.finish_err();
        return Err(e);
    }

    progress.finish_ok();
    Ok(())
}

fn run_ffmpeg_encode(
    base_args: &[String],
    input: &Path,
    progress: &FileProgress,
    verbosity: Verbosity,
) -> Result<()> {
    use std::io::BufRead;
    use std::process::Stdio;

    // Build full arg list with verbosity-appropriate global flags prepended.
    // base_args starts with "-y -i input ..." (no -loglevel, removed from ffmpeg_args.rs).
    let full_args: Vec<String> = match verbosity {
        Verbosity::Quiet => {
            let mut a = vec!["-loglevel".into(), "error".into()];
            a.extend_from_slice(base_args);
            a
        }
        Verbosity::Default => {
            // -loglevel error suppresses noise; -progress pipe:2 writes key=value to
            // stderr (bypasses loglevel); -nostats suppresses the human-readable stats line.
            let mut a = vec![
                "-loglevel".into(),
                "error".into(),
                "-progress".into(),
                "pipe:2".into(),
                "-nostats".into(),
            ];
            a.extend_from_slice(base_args);
            a
        }
        Verbosity::Verbose => {
            // No loglevel flag: use ffmpeg's default (info), which includes the stats line.
            base_args.to_vec()
        }
    };

    match verbosity {
        Verbosity::Quiet => {
            let status = std::process::Command::new("ffmpeg")
                .args(&full_args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(map_ffmpeg_spawn_err)?;
            if !status.success() {
                return Err(Error::FfmpegEncodeFailed {
                    status: status.code().unwrap_or(-1),
                    input: input.to_path_buf(),
                });
            }
        }
        Verbosity::Default => {
            let mut child = std::process::Command::new("ffmpeg")
                .args(&full_args)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(map_ffmpeg_spawn_err)?;

            let stderr = child.stderr.take().expect("stderr was piped");

            // Clone the indicatif ProgressBar (if any) into the reader thread.
            // ProgressBar::clone() shares the same underlying state — updating the
            // clone advances the bar in the main thread's display.
            let pb_for_thread = progress.bar_clone();

            let reader = std::thread::spawn(move || {
                let mut error_lines: Vec<String> = Vec::new();
                let reader = std::io::BufReader::new(stderr);
                for line in reader.lines() {
                    let line = line.unwrap_or_default();
                    if let Some(update) = crate::progress::parse_progress_line(&line) {
                        if let Some(us) = update.out_time_us {
                            if let Some(pb) = &pb_for_thread {
                                pb.set_position(us / 1000); // µs → ms
                            }
                        }
                    } else if !line.trim().is_empty() {
                        error_lines.push(line);
                    }
                }
                error_lines
            });

            let status = child.wait().map_err(|e| Error::Io {
                path: PathBuf::from("ffmpeg"),
                source: e,
            })?;
            let error_lines = reader.join().expect("reader thread panicked");

            if !status.success() {
                for line in &error_lines {
                    progress.println(line);
                }
                return Err(Error::FfmpegEncodeFailed {
                    status: status.code().unwrap_or(-1),
                    input: input.to_path_buf(),
                });
            }
        }
        Verbosity::Verbose => {
            // Inherit stderr so ffmpeg's output (including \r-updated stats) goes
            // directly to the user's terminal unchanged.
            let status = std::process::Command::new("ffmpeg")
                .args(&full_args)
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .status()
                .map_err(map_ffmpeg_spawn_err)?;
            if !status.success() {
                return Err(Error::FfmpegEncodeFailed {
                    status: status.code().unwrap_or(-1),
                    input: input.to_path_buf(),
                });
            }
        }
    }
    Ok(())
}

fn map_ffmpeg_spawn_err(e: std::io::Error) -> Error {
    if e.kind() == std::io::ErrorKind::NotFound {
        Error::FfmpegNotFound
    } else {
        Error::Io {
            path: PathBuf::from("ffmpeg"),
            source: e,
        }
    }
}

/// Print the pre-encode header listing the files Bento is about to process.
/// Used by both the single-file and directory paths so the output is uniform.
fn print_convert_header(
    input_dir: &Path,
    files: &[PathBuf],
    out: &mut dyn Write,
) -> Result<()> {
    writeln!(
        out,
        "{} {} {}",
        style("Converting").bold(),
        files.len(),
        style(format!(
            "file{} in {}:",
            if files.len() == 1 { "" } else { "s" },
            input_dir.display()
        ))
        .dim(),
    )
    .map_err(crate::io_render_err)?;
    for file in files {
        let name = file
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.display().to_string());
        writeln!(out, "  {}", name).map_err(crate::io_render_err)?;
    }
    writeln!(out).map_err(crate::io_render_err)?; // blank line before first file
    Ok(())
}

/// Print the end-of-batch summary separator and counts. Called from both the
/// directory path (multiple files) and the single-file path in `run_convert`
/// so the experience is consistent regardless of how many files were processed.
fn print_batch_summary(
    succeeded: usize,
    failed: &[(PathBuf, String)],
    out: &mut dyn Write,
) -> Result<()> {
    writeln!(out, "{}", "─".repeat(50)).map_err(crate::io_render_err)?;
    writeln!(
        out,
        "  {} succeeded · {} failed{}",
        succeeded,
        failed.len(),
        if failed.is_empty() { "" } else { ":" }
    )
    .map_err(crate::io_render_err)?;
    for (path, err) in failed {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        writeln!(out, "    ✗ {}: {}", name, err).map_err(crate::io_render_err)?;
    }
    Ok(())
}

fn compute_output_path(
    input: &Path,
    config: &Config,
    output_dir_override: Option<&Path>,
) -> Result<(PathBuf, Option<i64>)> {
    let container = config.output.container.unwrap_or(Container::Mp4);
    let extension = match container {
        Container::Mp4 => "mp4",
        Container::Mkv => "mkv",
    };

    let (stem, episode_number) = compute_output_stem(input, config)?;
    let output_name = format!("{}.{}", stem, extension);

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

    Ok((destination.join(output_name), episode_number))
}
