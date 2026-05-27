//! Encode pipeline: probe, prepare, and invoke ffmpeg.

pub mod ffmpeg_args;
pub mod naming;
pub mod probe;
pub mod subtitle_prep;

use std::io::Write;
use std::path::{Path, PathBuf};

use console::style;

use crate::config::{
    Config, Container, Crop, CropMode, DeinterlaceMode, Denoise, DetelecineMode, OnExisting,
    SubtitleMux, SubtitleTrack, TrackRef,
};
use crate::error::{Error, Result};
use crate::layers::discover_layers;
use crate::progress::FileProgress;
use crate::resolve::resolve;
use crate::validate::{Severity, validate};
use crate::verbosity::Verbosity;
use ffmpeg_args::{AudioAction, build_ffmpeg_args, decide_audio_action};
use naming::compute_output_stem;
use probe::{probe_cropdetect, probe_source_streams};
use subtitle_prep::{prepare_subtitles, write_external_sidecars};

/// CLI-level warning suppressions, applied on top of the resolved config's
/// `warn_*` fields.  Constructed from `--no-warn-X` / `--no-warnings` flags
/// and passed into `run_convert`.  `Default` is all-false (nothing suppressed).
#[derive(Debug, Default, Clone, Copy)]
pub struct WarnFlags {
    pub no_warn_multiple_burns: bool,
    pub no_warn_burn_metadata: bool,
    pub no_warn_ass_to_srt: bool,
    /// Suppresses both `[audio].warn_no_default` and `[subtitles].warn_no_default`.
    pub no_warn_no_default: bool,
    pub no_warn_crf_codec_mismatch: bool,
    pub no_warn_missing: bool,
    pub no_warn_redundant: bool,
    /// Bulk flag: equivalent to setting every individual flag above.
    pub no_warnings: bool,
}

/// All CLI-level options for a `bento convert` invocation. Passed as a single
/// argument to [`run_convert`] so that adding new flags does not change the
/// call signature.
#[derive(Debug, Default)]
pub struct ConvertOptions {
    pub output_dir_override: Option<std::path::PathBuf>,
    pub on_existing_override: Option<OnExisting>,
    pub generate_config: bool,
    pub dry_run: bool,
    pub verbosity: Verbosity,
    pub warn_flags: WarnFlags,
    pub keep_intermediates: bool,
    pub set_overrides: Vec<String>,
}

/// Per-run context shared across all file-level pipeline calls. Bundles the
/// run-level settings so `run_convert_file` and `run_convert_directory` don't
/// need long positional argument lists.
#[derive(Clone, Copy)]
struct ConvertContext<'a> {
    cli_config: &'a Config,
    output_dir_override: Option<&'a Path>,
    dry_run: bool,
    verbosity: Verbosity,
    warn_flags: WarnFlags,
    /// `None` iff `dry_run` (no temp dir is created for dry runs).
    temp_root: Option<&'a Path>,
}

pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "m4v", "avi", "mov", "webm", "ts", "m2ts", "wmv",
];

pub fn is_video_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| VIDEO_EXTENSIONS.iter().any(|v| v.eq_ignore_ascii_case(e)))
        .unwrap_or(false)
}

pub fn run_convert(input: &Path, out: &mut dyn Write, opts: ConvertOptions) -> Result<()> {
    let ConvertOptions {
        output_dir_override,
        on_existing_override,
        generate_config,
        dry_run,
        verbosity,
        warn_flags,
        keep_intermediates,
        set_overrides,
    } = opts;
    let output_dir_override = output_dir_override.as_deref();

    if !input.exists() {
        return Err(Error::PathNotFound(input.to_path_buf()));
    }

    // Build the CLI layer config from explicit CLI overrides. This is used both
    // for --generate-config sidecar writing and for adding to the resolution stack.
    let cli_config = build_cli_config(on_existing_override, warn_flags, &set_overrides)?;
    let cli_config_is_empty = cli_config == Config::default();

    // --generate-config requires at least one override to write.
    if generate_config && cli_config_is_empty {
        return Err(Error::GenerateConfigNoOverrides);
    }

    // Compute the sidecar path once (file → <file>.bento.toml, dir → <dir>/bento.toml).
    let sidecar_path: Option<PathBuf> = if generate_config {
        Some(if input.is_dir() {
            input.join("bento.toml")
        } else {
            crate::layers::sidecar_path(input)
        })
    } else {
        None
    };

    // Handle sidecar write (or dry-run announcement) before encoding starts.
    if let Some(ref sp) = sidecar_path {
        if dry_run {
            writeln!(out, "Would write sidecar at: {}", sp.display())
                .map_err(crate::io_render_err)?;
            writeln!(out).map_err(crate::io_render_err)?;
        } else if sp.exists() {
            if !warn_flags.no_warnings {
                writeln!(
                    out,
                    "warning: --generate-config: sidecar already exists, not overwriting: {}",
                    sp.display()
                )
                .map_err(crate::io_render_err)?;
            }
        } else {
            write_sidecar(&cli_config, sp)?;
            writeln!(out, "Wrote config to: {}", sp.display()).map_err(crate::io_render_err)?;
        }
    }

    // Per-run temp dir — held here so it outlives all file processing.
    // Dry-run produces no intermediate files, so no dir is needed.
    let temp_dir: Option<tempfile::TempDir> = if dry_run {
        None
    } else {
        Some(tempfile::tempdir().map_err(|e| Error::Io {
            path: PathBuf::from("<tempdir>"),
            source: e,
        })?)
    };
    let temp_root: Option<&Path> = temp_dir.as_ref().map(|d| d.path());

    let ctx = ConvertContext {
        cli_config: &cli_config,
        output_dir_override,
        dry_run,
        verbosity,
        warn_flags,
        temp_root,
    };

    let run_result = if input.is_dir() {
        run_convert_directory(input, &ctx, out)
    } else {
        // Single-file: print a unified header, run, then always print the summary.
        let input_dir = input.parent().unwrap_or_else(|| Path::new("."));
        print_convert_header(input_dir, &[input.to_path_buf()], dry_run, out)?;

        let result = run_convert_file(input, 1, 1, &ctx, out);

        // FfmpegNotFound is an environmental error — skip the per-run summary
        // since every remaining file would fail the same way.
        let is_env_error = matches!(&result, Err(Error::FfmpegNotFound));
        if !is_env_error {
            if dry_run {
                let error_count = if result.is_err() { 1 } else { 0 };
                print_dry_run_summary(input, 1, error_count, verbosity, out)?;
            } else {
                let failed: Vec<(PathBuf, String)> = match &result {
                    Ok(()) => vec![],
                    Err(e) => vec![(input.to_path_buf(), e.to_string())],
                };
                let succeeded = if failed.is_empty() { 1 } else { 0 };
                print_batch_summary(succeeded, &failed, out)?;
            }
        }
        result
    };

    // Suppress temp dir cleanup when --keep-intermediates is set.
    // Dry-run is always a silent no-op (temp_dir is None).
    if keep_intermediates {
        if let Some(dir) = temp_dir {
            let path = dir.keep();
            writeln!(out, "\nIntermediate files preserved at: {}", path.display())
                .map_err(crate::io_render_err)?;
        }
    }
    // temp_dir drops here if !keep_intermediates → auto-cleanup

    run_result
}

fn run_convert_directory(
    input_dir: &Path,
    ctx: &ConvertContext<'_>,
    out: &mut dyn Write,
) -> Result<()> {
    let ConvertContext {
        dry_run, verbosity, ..
    } = *ctx;
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

    print_convert_header(input_dir, &files, dry_run, out)?;

    let file_count = files.len();
    let mut succeeded: Vec<PathBuf> = Vec::new();
    let mut failed: Vec<(PathBuf, String)> = Vec::new();

    for (idx, file) in files.iter().enumerate() {
        match run_convert_file(file, idx + 1, file_count, ctx, out) {
            Ok(()) => succeeded.push(file.clone()),
            Err(e) => {
                if matches!(e, Error::FfmpegNotFound) {
                    return Err(e);
                }
                let msg = e.to_string();
                if !dry_run {
                    writeln!(out, "[error] {}: {}", file.display(), msg)
                        .map_err(crate::io_render_err)?;
                }
                failed.push((file.clone(), msg));
            }
        }
    }

    if dry_run {
        print_dry_run_summary(input_dir, file_count, failed.len(), verbosity, out)?;
    } else {
        print_batch_summary(succeeded.len(), &failed, out)?;
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
    file_idx: usize,
    file_count: usize,
    ctx: &ConvertContext<'_>,
    out: &mut dyn Write,
) -> Result<()> {
    let ConvertContext {
        cli_config,
        output_dir_override,
        dry_run,
        verbosity,
        warn_flags,
        temp_root,
    } = *ctx;
    use crate::resolve::Layer;

    // input_name is used in the layer-count summary and throughout; compute early.
    let input_name = input
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| input.display().to_string());

    // --- Config resolution and validation ------------------------------------
    // Add the CLI layer at the top of the stack if any CLI overrides were set.
    let mut layers = discover_layers(input, out)?;

    // Redundancy check on config-file layers before the CLI layer is added.
    // CLI flags are one-off overrides; redundancy is meaningful only between
    // persistent config files the user can edit.
    if !warn_flags.no_warnings && !warn_flags.no_warn_redundant {
        for (path, lower_layer, higher_layer) in detect_redundancies(&layers) {
            writeln!(
                out,
                "warning: redundant override: `{}` in {} is already set to \
                 the same value in {}; the setting in the {} config can be removed.",
                path,
                layer_inline(&higher_layer),
                layer_inline(&lower_layer),
                higher_layer.kind(),
            )
            .map_err(crate::io_render_err)?;
        }
    }

    if cli_config != &Config::default() {
        layers.push((Layer::Cli, cli_config.clone()));
    }
    let resolved = resolve(layers);

    // Missing-setting check: fields that fell all the way through to baked-in
    // defaults without being set in any user config layer.
    if !warn_flags.no_warnings && !warn_flags.no_warn_missing {
        let default_paths: Vec<&String> = resolved
            .provenance
            .iter()
            .filter(|(_, layer)| **layer == crate::resolve::Layer::Defaults)
            .map(|(path, _)| path)
            .collect();
        if !default_paths.is_empty() {
            writeln!(
                out,
                "warning: {} setting{} resolved from baked-in defaults \
                 (not present in any config file):",
                default_paths.len(),
                if default_paths.len() == 1 { "" } else { "s" },
            )
            .map_err(crate::io_render_err)?;
            let cfg_val =
                toml::Value::try_from(&resolved.config).expect("Config is always serializable");
            for path in &default_paths {
                let val_str = get_toml_at_path(&cfg_val, path)
                    .map(|v| format!(" = {}", toml_value_display(v)))
                    .unwrap_or_default();
                writeln!(out, "  {}{}", path, val_str).map_err(crate::io_render_err)?;
            }
            writeln!(
                out,
                "  Run `bento repair` to add these to your global config, \
                 or pass --no-warn-missing to suppress.",
            )
            .map_err(crate::io_render_err)?;
        }
    }

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
        let cli_n = counts.get("cli").copied().unwrap_or(0);
        let per_file_n = counts.get("per-file").copied().unwrap_or(0);
        let directory_n = counts.get("directory").copied().unwrap_or(0);
        let global_n = counts.get("global").copied().unwrap_or(0);
        let defaults_n = counts.get("defaults").copied().unwrap_or(0);
        if cli_n > 0 {
            format!(
                "{} settings ({} from cli, {} from file, {} from directory, {} from global, {} from baked-in defaults)",
                total, cli_n, per_file_n, directory_n, global_n, defaults_n,
            )
        } else {
            format!(
                "{} settings ({} from file, {} from directory, {} from global, {} from baked-in defaults)",
                total, per_file_n, directory_n, global_n, defaults_n,
            )
        }
    } else {
        String::new()
    };

    // --- Output path and on_existing resolution ------------------------------
    // In dry-run mode we compute the path but skip creating the destination
    // directory — no filesystem effects allowed.
    let (output_path, episode_number) =
        compute_output_path(input, &resolved.config, output_dir_override, dry_run)?;

    // on_existing is resolved through the full layer stack (CLI layer included).
    let on_existing = resolved
        .config
        .output
        .on_existing
        .unwrap_or(OnExisting::Warn);

    let output_name = output_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| output_path.display().to_string());

    // Dry-run: probe the source (needed for copy-vs-transcode decisions), print
    // the plan, then return.  We never touch the output path or create a temp dir.
    if dry_run {
        let probe = probe_source_streams(input)?;
        let output_exists = output_path.exists();
        print_dry_run_plan(
            &input_name,
            &output_path,
            output_exists,
            on_existing,
            &resolved.config,
            &probe,
            episode_number,
            verbosity,
            out,
        )?;
        return Ok(());
    }

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
        Some(crate::config::Crop::Mode(crate::config::CropMode::Auto)) => probe_cropdetect(input)?,
        _ => None,
    };

    // --- Subtitle preparation ------------------------------------------------
    // Carve a per-file subdir within the per-run temp dir (guaranteed Some here
    // since we only reach this point when !dry_run).
    let file_temp = {
        let root = temp_root.expect("temp_root is Some when !dry_run");
        let subdir = root.join(sanitize_basename(input));
        std::fs::create_dir_all(&subdir).map_err(|e| Error::Io {
            path: subdir.clone(),
            source: e,
        })?;
        subdir
    };

    let prepared_subs = match prepare_subtitles(
        input,
        &resolved.config.subtitles,
        &resolved.provenance,
        &probe,
        &file_temp,
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

/// Build a Config representing only the fields explicitly set by CLI flags.
/// This becomes the `Layer::Cli` entry added to the resolution stack, and is
/// also what `--generate-config` serializes to the sidecar file.
///
/// `--set KEY=VALUE` overrides are applied first; dedicated flags (e.g.
/// `--on-existing`, `--no-warn-*`) are written on top so they win when both
/// target the same field.
fn build_cli_config(
    on_existing_override: Option<OnExisting>,
    flags: WarnFlags,
    set_overrides: &[String],
) -> Result<Config> {
    // Start from --set overrides (empty Config when none are given).
    let mut config = crate::set_override::build_set_config(set_overrides)?;

    // Dedicated CLI flags overwrite any conflicting --set values for the same fields.
    let suppress_all = flags.no_warnings;

    if let Some(oe) = on_existing_override {
        config.output.on_existing = Some(oe);
    }
    if suppress_all || flags.no_warn_multiple_burns {
        config.subtitles.warn_multiple_burns = Some(false);
    }
    if suppress_all || flags.no_warn_burn_metadata {
        config.subtitles.warn_burn_metadata = Some(false);
    }
    if suppress_all || flags.no_warn_ass_to_srt {
        config.subtitles.warn_ass_to_srt = Some(false);
    }
    if suppress_all || flags.no_warn_no_default {
        config.audio.warn_no_default = Some(false);
        config.subtitles.warn_no_default = Some(false);
    }
    if suppress_all || flags.no_warn_crf_codec_mismatch {
        config.video.warn_crf_codec_mismatch = Some(false);
    }
    // no_warn_missing and no_warn_redundant are accepted but have no Config
    // counterparts; they are CLI-only no-ops.

    Ok(config)
}

/// Write the CLI-override config to a sidecar file, stripping empty sections
/// so only the fields that were actually set appear in the output.
fn write_sidecar(cli_config: &Config, path: &Path) -> Result<()> {
    use std::io::Write as _;

    let value = toml::Value::try_from(cli_config).expect("Config is always serializable");
    let toml_str = match clean_empty_tables(value) {
        Some(cleaned) => toml::to_string_pretty(&cleaned).expect("cleaned Value always serializes"),
        None => String::new(),
    };

    // Ensure the parent directory exists (e.g. when writing a dir-level bento.toml
    // into a directory that hasn't been created yet — rare but possible).
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| Error::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }
    }

    let mut file = std::fs::File::create(path).map_err(|e| Error::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    writeln!(file, "# Generated by `bento convert --generate-config`.").map_err(|e| Error::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    writeln!(
        file,
        "# Contains only the settings overridden via CLI flags."
    )
    .map_err(|e| Error::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    if !toml_str.is_empty() {
        writeln!(file, "{}", toml_str.trim_end()).map_err(|e| Error::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
    }
    Ok(())
}

/// Recursively remove empty tables from a `toml::Value`, returning `None` if
/// the value reduces to nothing. Ensures `--generate-config` output only
/// contains sections that actually have content.
fn clean_empty_tables(value: toml::Value) -> Option<toml::Value> {
    match value {
        toml::Value::Table(table) => {
            let cleaned: toml::map::Map<String, toml::Value> = table
                .into_iter()
                .filter_map(|(k, v)| clean_empty_tables(v).map(|v| (k, v)))
                .collect();
            if cleaned.is_empty() {
                None
            } else {
                Some(toml::Value::Table(cleaned))
            }
        }
        other => Some(other),
    }
}

fn run_ffmpeg_encode(
    base_args: &[String],
    input: &Path,
    progress: &FileProgress,
    verbosity: Verbosity,
) -> Result<()> {
    use std::io::BufRead;
    use std::process::Stdio;

    // Build full arg list: -loglevel and -progress flags prepended to base_args.
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
    dry_run: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let verb = if dry_run { "Dry-run for" } else { "Converting" };
    writeln!(
        out,
        "{} {} {}",
        style(verb).bold(),
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

/// Print the dry-run plan for one file: what Bento would do if `--dry-run`
/// were not passed.  Probes are already done by the caller; no files are
/// written here.
#[allow(clippy::too_many_arguments)]
fn print_dry_run_plan(
    input_name: &str,
    output_path: &Path,
    output_exists: bool,
    on_existing: OnExisting,
    config: &Config,
    probe: &probe::SourceProbe,
    episode_number: Option<i64>,
    verbosity: Verbosity,
    out: &mut dyn Write,
) -> Result<()> {
    use std::collections::BTreeSet;

    use crate::config::{EncoderName, Resolution, SubtitleFormat, Tune};

    writeln!(out, "{}:", input_name).map_err(crate::io_render_err)?;

    // --- Subtitles -----------------------------------------------------------
    if let Some(tracks) = &config.subtitles.tracks {
        if !tracks.is_empty() {
            // Unique source-MKV track indices that need extraction.
            let mut extract: BTreeSet<u32> = BTreeSet::new();
            for t in tracks {
                if let Some(TrackRef::Index(i)) = &t.source {
                    extract.insert(*i);
                }
                if let Some(TrackRef::Index(i)) = &t.subtract_track {
                    extract.insert(*i);
                }
            }
            if !extract.is_empty() {
                let idx_str = extract
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                writeln!(
                    out,
                    "  Would extract subtitle track{} {} from source.",
                    if extract.len() == 1 { "" } else { "s" },
                    idx_str,
                )
                .map_err(crate::io_render_err)?;
            }

            let soft_ext: Vec<&SubtitleTrack> = tracks
                .iter()
                .filter(|t| matches!(t.mux, Some(SubtitleMux::Soft) | Some(SubtitleMux::External)))
                .collect();
            let burns: Vec<&SubtitleTrack> = tracks
                .iter()
                .filter(|t| matches!(t.mux, Some(SubtitleMux::Burn)))
                .collect();

            if !soft_ext.is_empty() {
                let n = soft_ext.len();
                let any_ext = soft_ext
                    .iter()
                    .any(|t| matches!(t.mux, Some(SubtitleMux::External)));
                let any_soft = soft_ext
                    .iter()
                    .any(|t| matches!(t.mux, Some(SubtitleMux::Soft)));
                let kind = match (any_soft, any_ext) {
                    (true, true) => "soft/external subtitle",
                    (false, true) => "external subtitle",
                    _ => "soft subtitle",
                };
                writeln!(
                    out,
                    "  Would derive {} {} track{}:",
                    n,
                    kind,
                    if n == 1 { "" } else { "s" },
                )
                .map_err(crate::io_render_err)?;
                for t in &soft_ext {
                    let label = sub_track_label(t);
                    let deriv = sub_derivation(t);
                    let fmt = match t.format {
                        Some(SubtitleFormat::Srt) => "srt",
                        Some(SubtitleFormat::Ass) => "ass",
                        None => "auto",
                    };
                    let mut flags: Vec<&str> = Vec::new();
                    if t.default == Some(true) {
                        flags.push("default=true");
                    }
                    if t.forced == Some(true) {
                        flags.push("forced=true");
                    }
                    if t.hearing_impaired == Some(true) {
                        flags.push("sdh");
                    }
                    if t.commentary == Some(true) {
                        flags.push("commentary");
                    }
                    let flag_str = if flags.is_empty() {
                        String::new()
                    } else {
                        format!(", {}", flags.join(", "))
                    };
                    let ext_tag = if matches!(t.mux, Some(SubtitleMux::External)) {
                        " [external]"
                    } else {
                        ""
                    };
                    writeln!(
                        out,
                        "    {}: {}, format={}{}{}",
                        label, deriv, fmt, flag_str, ext_tag,
                    )
                    .map_err(crate::io_render_err)?;
                }
            }

            if !burns.is_empty() {
                let n = burns.len();
                writeln!(
                    out,
                    "  Would burn {} subtitle track{}:",
                    n,
                    if n == 1 { "" } else { "s" },
                )
                .map_err(crate::io_render_err)?;
                for t in &burns {
                    let deriv = sub_derivation(t);
                    let fmt_suffix = match t.format {
                        Some(SubtitleFormat::Ass) => " (ass)",
                        _ => "",
                    };
                    writeln!(out, "    {}{} onto video.", deriv, fmt_suffix)
                        .map_err(crate::io_render_err)?;
                }
            }
        }
    }

    // --- Video ---------------------------------------------------------------
    {
        let enc = config.video.encoder.as_ref();
        let name = enc
            .and_then(|e| e.name)
            .map(|n| match n {
                EncoderName::X264 => "x264",
                EncoderName::X265 => "x265",
            })
            .unwrap_or("x264");
        let crf = enc.and_then(|e| e.crf).unwrap_or(20);
        let tune_str = enc
            .and_then(|e| e.tune)
            .filter(|t| !matches!(t, Tune::None))
            .map(|t| format!(" tune={}", t))
            .unwrap_or_default();
        let preset_str = config
            .video
            .preset
            .map(|p| format!(" preset={}", p))
            .unwrap_or_default();

        let mut pre: Vec<String> = Vec::new();
        match &config.video.crop {
            Some(Crop::Mode(CropMode::None)) | None => {}
            Some(Crop::Mode(CropMode::Auto)) => pre.push("crop=auto".into()),
            Some(Crop::Explicit(p)) => {
                let mut parts: Vec<String> = Vec::new();
                if let Some(v) = p.top {
                    parts.push(format!("top:{}", v));
                }
                if let Some(v) = p.bottom {
                    parts.push(format!("bottom:{}", v));
                }
                if let Some(v) = p.left {
                    parts.push(format!("left:{}", v));
                }
                if let Some(v) = p.right {
                    parts.push(format!("right:{}", v));
                }
                pre.push(format!("crop={}", parts.join(",")));
            }
        }
        match config.video.deinterlace {
            Some(DeinterlaceMode::None) | None => {}
            Some(DeinterlaceMode::Auto) | Some(DeinterlaceMode::Yadif) => {
                pre.push("deinterlace=yadif".into());
            }
            Some(DeinterlaceMode::Bwdif) => pre.push("deinterlace=bwdif".into()),
        }
        if matches!(config.video.detelecine, Some(DetelecineMode::Auto)) {
            pre.push("detelecine=auto".into());
        }
        if let Some(Denoise::Active(c)) = &config.video.denoise {
            use crate::config::{DenoiseFilter, DenoisePreset};
            let filter = match c.filter {
                Some(DenoiseFilter::Nlmeans) => "nlmeans",
                Some(DenoiseFilter::Hqdn3d) => "hqdn3d",
                None => "auto",
            };
            let preset = match c.preset.unwrap_or(DenoisePreset::Medium) {
                DenoisePreset::Ultralight => ":ultralight",
                DenoisePreset::Light => ":light",
                DenoisePreset::Medium => ":medium",
                DenoisePreset::Strong => ":strong",
                DenoisePreset::Stronger => ":stronger",
                DenoisePreset::Verystrong => ":verystrong",
            };
            pre.push(format!("denoise={}{}", filter, preset));
        }
        if let Some(Resolution::Explicit(r)) = &config.video.resolution {
            match (r.width, r.height) {
                (Some(w), Some(h)) => pre.push(format!("scale={}x{}", w, h)),
                (Some(w), None) => pre.push(format!("width={}", w)),
                (None, Some(h)) => pre.push(format!("height={}", h)),
                (None, None) => {}
            }
        }
        let pre_str = if pre.is_empty() {
            "no preprocessing".to_string()
        } else {
            pre.join(", ")
        };
        writeln!(
            out,
            "  Would transcode video: {} crf={}{}{}, {}.",
            name, crf, tune_str, preset_str, pre_str,
        )
        .map_err(crate::io_render_err)?;
    }

    // --- Audio ---------------------------------------------------------------
    if let Some(tracks) = &config.audio.tracks {
        let normalize_mix = config.audio.normalize_mix.unwrap_or(false);
        let actions: Vec<AudioAction> = tracks
            .iter()
            .map(|t| {
                let src_idx = t.source.unwrap_or(1).saturating_sub(1) as usize;
                probe
                    .audio
                    .get(src_idx)
                    .map(|src| decide_audio_action(t, &config.audio, src, normalize_mix))
                    .unwrap_or_else(|| {
                        let enc = t
                            .encoder
                            .as_deref()
                            .or(config.audio.encoder.as_deref())
                            .unwrap_or("aac");
                        let mixdown = t
                            .mixdown
                            .or(config.audio.mixdown)
                            .unwrap_or(crate::config::Mixdown::Stereo);
                        AudioAction::Transcode {
                            encoder: ffmpeg_args::audio_encoder_to_ffmpeg(enc).to_string(),
                            bitrate_kbps: t.bitrate.or(config.audio.bitrate).unwrap_or(192),
                            channels: ffmpeg_args::mixdown_to_channels(mixdown),
                            use_dpl2: matches!(mixdown, crate::config::Mixdown::Dpl2),
                        }
                    })
            })
            .collect();

        let copy_n = actions.iter().filter(|a| **a == AudioAction::Copy).count();
        let trans_n = tracks.len() - copy_n;
        let heading = if trans_n == 0 {
            format!(
                "Would copy {} audio track{}:",
                tracks.len(),
                if tracks.len() == 1 { "" } else { "s" }
            )
        } else if copy_n == 0 {
            format!(
                "Would transcode {} audio track{}:",
                tracks.len(),
                if tracks.len() == 1 { "" } else { "s" }
            )
        } else {
            format!(
                "Would process {} audio track{}:",
                tracks.len(),
                if tracks.len() == 1 { "" } else { "s" }
            )
        };
        writeln!(out, "  {}", heading).map_err(crate::io_render_err)?;

        for (t, action) in tracks.iter().zip(actions.iter()) {
            let label = audio_track_label(t);
            match action {
                AudioAction::Copy => {
                    let src_idx = t.source.unwrap_or(1).saturating_sub(1) as usize;
                    let src_desc = probe
                        .audio
                        .get(src_idx)
                        .map(|s| format!(" ({} {})", s.codec, channels_label(s.channels)))
                        .unwrap_or_default();
                    writeln!(out, "    {}: copy{}.", label, src_desc)
                        .map_err(crate::io_render_err)?;
                }
                AudioAction::Transcode {
                    encoder,
                    bitrate_kbps,
                    channels,
                    ..
                } => {
                    writeln!(
                        out,
                        "    {}: transcode → {} {}k {}.",
                        label,
                        encoder,
                        bitrate_kbps,
                        channels_label(*channels),
                    )
                    .map_err(crate::io_render_err)?;
                }
            }
        }
    }

    // --- Mux destination -----------------------------------------------------
    let exists_note = if output_exists {
        match on_existing {
            OnExisting::Warn => " (warning: output exists, would skip)",
            OnExisting::SkipSilently => " (output exists, would skip silently)",
            OnExisting::Overwrite => " (output exists, would overwrite)",
            OnExisting::Fail => " (output exists, would fail)",
        }
    } else {
        ""
    };
    writeln!(
        out,
        "  Would mux to: {}{}.",
        output_path.display(),
        exists_note
    )
    .map_err(crate::io_render_err)?;

    // --- Verbose: show the ffmpeg command line --------------------------------
    // Subtitles are omitted (their temp-dir paths aren't known at dry-run time).
    if verbosity == Verbosity::Verbose {
        let args = build_ffmpeg_args(
            &PathBuf::from(input_name),
            output_path,
            config,
            probe,
            &[],
            None,
            episode_number,
        );
        writeln!(out, "  ffmpeg {} (subtitle args omitted)", args.join(" "))
            .map_err(crate::io_render_err)?;
    }

    writeln!(out).map_err(crate::io_render_err)?; // blank line between files
    Ok(())
}

/// Dry-run end-of-batch summary (replaces the normal `print_batch_summary`).
fn print_dry_run_summary(
    input: &Path,
    file_count: usize,
    error_count: usize,
    verbosity: Verbosity,
    out: &mut dyn Write,
) -> Result<()> {
    writeln!(out, "{}", "─".repeat(50)).map_err(crate::io_render_err)?;
    let err_str = match error_count {
        0 => "0 errors".to_string(),
        1 => "1 error".to_string(),
        n => format!("{} errors", n),
    };
    writeln!(
        out,
        "  {} {} would be processed. {}.",
        file_count,
        if file_count == 1 { "file" } else { "files" },
        err_str,
    )
    .map_err(crate::io_render_err)?;
    if verbosity != Verbosity::Quiet {
        writeln!(
            out,
            "Run `bento config {}` to see where each setting resolved from.",
            input.display(),
        )
        .map_err(crate::io_render_err)?;
    }
    Ok(())
}

// --- Dry-run plan helpers ---------------------------------------------------

fn sub_track_label(t: &SubtitleTrack) -> String {
    match (&t.title, &t.lang) {
        (Some(title), Some(lang)) => format!("\"{}\" ({})", title, lang),
        (Some(title), None) => format!("\"{}\"", title),
        (None, Some(lang)) => format!("({})", lang),
        (None, None) => sub_ref_short(&t.source),
    }
}

fn sub_derivation(t: &SubtitleTrack) -> String {
    use crate::config::FilterMode;
    let src = sub_ref_short(&t.source);
    if let Some(sub_ref) = &t.subtract_track {
        return format!(
            "{} minus {} timestamps",
            src,
            sub_ref_short(&Some(sub_ref.clone()))
        );
    }
    if let Some(f) = &t.filter {
        let mode = match f.mode {
            Some(FilterMode::Retain) => "retained",
            Some(FilterMode::Remove) => "removed",
            None => "filtered",
        };
        let criterion = if let Some(s) = &f.style {
            format!("style \"{}\" {}", s, mode)
        } else if let Some(font) = &f.font {
            format!("font \"{}\" {}", font, mode)
        } else if let Some(sz) = &f.size {
            format!("size {} {}", sz, mode)
        } else {
            mode.to_string()
        };
        return format!("{} ({})", src, criterion);
    }
    src
}

fn sub_ref_short(r: &Option<TrackRef>) -> String {
    match r {
        Some(TrackRef::Index(i)) => format!("track {}", i),
        Some(TrackRef::Path(p)) => {
            let name = std::path::Path::new(p)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.clone());
            format!("file \"{}\"", name)
        }
        None => "?".to_string(),
    }
}

fn audio_track_label(t: &crate::config::AudioTrack) -> String {
    match (&t.title, &t.lang) {
        (Some(title), Some(lang)) => format!("\"{}\" ({})", title, lang),
        (Some(title), None) => format!("\"{}\"", title),
        (None, Some(lang)) => format!("({})", lang),
        (None, None) => format!("track {}", t.source.unwrap_or(0)),
    }
}

fn channels_label(n: u32) -> &'static str {
    match n {
        1 => "mono",
        2 => "stereo",
        6 => "5.1",
        _ => "?ch",
    }
}

/// Produce a filesystem-safe directory name from an input file's stem.
/// Non-alphanumeric characters (except `-` and `_`) become `_`.
fn sanitize_basename(input: &Path) -> String {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    stem.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

// =============================================================================
// Warning helpers
// =============================================================================

/// Concise inline representation of a layer for use in warning messages.
fn layer_inline(layer: &crate::resolve::Layer) -> String {
    use crate::resolve::Layer;
    match layer {
        Layer::Defaults => "built-in defaults".to_string(),
        Layer::Global(p) => format!("global config ({})", p.display()),
        Layer::Directory(p) => format!("directory config ({})", p.display()),
        Layer::PerFile(p) => format!("per-file config ({})", p.display()),
        Layer::Cli => "CLI flags".to_string(),
    }
}

/// Collect all leaf paths and their TOML values from a serialized config.
/// Arrays are treated as atomic leaves per the wholesale-replace rule.
fn collect_toml_leaves(
    value: &toml::Value,
    prefix: &str,
) -> std::collections::HashMap<String, toml::Value> {
    let mut map = std::collections::HashMap::new();
    if let toml::Value::Table(table) = value {
        for (k, v) in table {
            let path = if prefix.is_empty() {
                k.clone()
            } else {
                format!("{}.{}", prefix, k)
            };
            if let toml::Value::Table(_) = v {
                map.extend(collect_toml_leaves(v, &path));
            } else {
                map.insert(path, v.clone());
            }
        }
    }
    map
}

/// Detect redundant overrides: fields where a higher-precedence config-file
/// layer sets the same value already present in a lower-precedence layer.
/// Returns `(field_path, lower_layer, higher_layer)` for each redundancy.
/// CLI and Defaults layers are excluded — redundancy is meaningful only
/// between persistent config files the user can edit.
fn detect_redundancies(
    layers: &[(crate::resolve::Layer, Config)],
) -> Vec<(String, crate::resolve::Layer, crate::resolve::Layer)> {
    use crate::resolve::Layer;

    let file_layers: Vec<(&Layer, std::collections::HashMap<String, toml::Value>)> = layers
        .iter()
        .filter(|(layer, _)| !matches!(layer, Layer::Cli | Layer::Defaults))
        .map(|(layer, cfg)| {
            let value = toml::Value::try_from(cfg).expect("Config is always serializable");
            let leaves = collect_toml_leaves(&value, "");
            (layer, leaves)
        })
        .collect();

    let mut redundancies = Vec::new();
    for i in 0..file_layers.len() {
        let (lower_layer, ref lower_leaves) = file_layers[i];
        for (higher_layer, higher_leaves) in &file_layers[(i + 1)..] {
            for (path, higher_val) in higher_leaves {
                if let Some(lower_val) = lower_leaves.get(path) {
                    if higher_val == lower_val {
                        redundancies.push((
                            path.clone(),
                            (*lower_layer).clone(),
                            (*higher_layer).clone(),
                        ));
                    }
                }
            }
        }
    }

    redundancies
}

/// Look up a dotted path in a TOML value tree.
fn get_toml_at_path<'a>(root: &'a toml::Value, path: &str) -> Option<&'a toml::Value> {
    let mut cur = root;
    for part in path.split('.') {
        if let toml::Value::Table(table) = cur {
            cur = table.get(part)?;
        } else {
            return None;
        }
    }
    Some(cur)
}

/// Short display for a TOML value (arrays and tables summarized).
fn toml_value_display(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => format!("{:?}", s),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Array(a) => {
            format!("[{} item{}]", a.len(), if a.len() == 1 { "" } else { "s" })
        }
        toml::Value::Table(_) => "{…}".to_string(),
        toml::Value::Datetime(d) => d.to_string(),
    }
}

fn compute_output_path(
    input: &Path,
    config: &Config,
    output_dir_override: Option<&Path>,
    dry_run: bool,
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

    if !dry_run && !destination.exists() {
        std::fs::create_dir_all(&destination).map_err(|e| Error::Io {
            path: destination.clone(),
            source: e,
        })?;
    }

    Ok((destination.join(output_name), episode_number))
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::path::PathBuf;

    fn dir_layer(path: &str) -> crate::resolve::Layer {
        crate::resolve::Layer::Directory(PathBuf::from(path))
    }

    fn per_file_layer(path: &str) -> crate::resolve::Layer {
        crate::resolve::Layer::PerFile(PathBuf::from(path))
    }

    fn parse(s: &str) -> Config {
        Config::from_toml_str(s).unwrap()
    }

    #[test]
    fn detect_redundancies_finds_same_scalar_in_higher_layer() {
        let layers = vec![
            (
                dir_layer("/show/bento.toml"),
                parse("[audio]\nbitrate = 192\n"),
            ),
            (
                per_file_layer("/show/ep01.mkv.bento.toml"),
                parse("[audio]\nbitrate = 192\n"),
            ),
        ];
        let r = detect_redundancies(&layers);
        assert!(
            r.iter().any(|(p, _, _)| p == "audio.bitrate"),
            "expected audio.bitrate flagged as redundant: {:?}",
            r
        );
    }

    #[test]
    fn detect_redundancies_ignores_different_values() {
        let layers = vec![
            (
                dir_layer("/show/bento.toml"),
                parse("[audio]\nbitrate = 192\n"),
            ),
            (
                per_file_layer("/show/ep01.mkv.bento.toml"),
                parse("[audio]\nbitrate = 128\n"),
            ),
        ];
        let r = detect_redundancies(&layers);
        assert!(
            !r.iter().any(|(p, _, _)| p == "audio.bitrate"),
            "different values must not be flagged: {:?}",
            r
        );
    }

    #[test]
    fn detect_redundancies_ignores_cli_layer() {
        let layers = vec![
            (
                dir_layer("/show/bento.toml"),
                parse("[audio]\nbitrate = 192\n"),
            ),
            (
                crate::resolve::Layer::Cli,
                parse("[audio]\nbitrate = 192\n"),
            ),
        ];
        let r = detect_redundancies(&layers);
        assert!(
            r.is_empty(),
            "CLI layer must not be included in redundancy checks: {:?}",
            r
        );
    }

    #[test]
    fn detect_redundancies_flags_identical_list() {
        let tracks = "[audio]\ntracks = [{ source = 1, lang = \"jpn\", default = true }]\n";
        let layers = vec![
            (dir_layer("/show/bento.toml"), parse(tracks)),
            (per_file_layer("/show/ep01.mkv.bento.toml"), parse(tracks)),
        ];
        let r = detect_redundancies(&layers);
        assert!(
            r.iter().any(|(p, _, _)| p == "audio.tracks"),
            "identical track list should be flagged: {:?}",
            r
        );
    }

    #[test]
    fn detect_redundancies_does_not_flag_different_list() {
        let lower = "[audio]\ntracks = [{ source = 1, lang = \"jpn\", default = true }]\n";
        let higher = "[audio]\ntracks = [{ source = 2, lang = \"eng\", default = true }]\n";
        let layers = vec![
            (dir_layer("/show/bento.toml"), parse(lower)),
            (per_file_layer("/show/ep01.mkv.bento.toml"), parse(higher)),
        ];
        let r = detect_redundancies(&layers);
        assert!(
            !r.iter().any(|(p, _, _)| p == "audio.tracks"),
            "different track lists must not be flagged: {:?}",
            r
        );
    }

    #[test]
    fn get_toml_at_path_looks_up_nested_key() {
        let cfg = parse("[video]\nencoder = { name = \"x264\", crf = 20 }\n");
        let val = toml::Value::try_from(&cfg).unwrap();
        let crf = get_toml_at_path(&val, "video.encoder.crf");
        assert_eq!(crf, Some(&toml::Value::Integer(20)));
    }

    #[test]
    fn get_toml_at_path_returns_none_for_missing_key() {
        let cfg = parse("[video]\npreset = \"medium\"\n");
        let val = toml::Value::try_from(&cfg).unwrap();
        assert!(get_toml_at_path(&val, "video.encoder.crf").is_none());
    }

    #[test]
    fn toml_value_display_formats_primitives() {
        assert_eq!(toml_value_display(&toml::Value::Integer(42)), "42");
        assert_eq!(
            toml_value_display(&toml::Value::String("mp4".into())),
            "\"mp4\""
        );
        assert_eq!(toml_value_display(&toml::Value::Boolean(true)), "true");
        assert_eq!(
            toml_value_display(&toml::Value::Array(vec![
                toml::Value::Integer(1),
                toml::Value::Integer(2),
            ])),
            "[2 items]"
        );
    }
}
