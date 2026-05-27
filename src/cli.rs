//! CLI surface: argument parsing and subcommand dispatch.

use std::io::Write;
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::config::OnExisting;
use crate::error::{Error, Result};
use crate::pipeline::{ConvertOptions, WarnFlags};
use crate::verbosity::Verbosity;

#[derive(Parser)]
#[command(
    name = "bento",
    version,
    about = "Configuration-driven video re-encoding for Jellyfin"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// CLI-side mirror of [`OnExisting`] using clap's default kebab-case
/// representation (`skip-silently` rather than the config's snake_case
/// `skip_silently`). This isolates the CLI surface from the config layer so
/// the config crate doesn't need a clap dependency.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OnExistingArg {
    Warn,
    SkipSilently,
    Overwrite,
    Fail,
}

impl From<OnExistingArg> for OnExisting {
    fn from(arg: OnExistingArg) -> Self {
        match arg {
            OnExistingArg::Warn => OnExisting::Warn,
            OnExistingArg::SkipSilently => OnExisting::SkipSilently,
            OnExistingArg::Overwrite => OnExisting::Overwrite,
            OnExistingArg::Fail => OnExisting::Fail,
        }
    }
}

#[derive(Subcommand)]
pub enum Command {
    /// Run the conversion pipeline on a file or directory.
    Convert {
        path: PathBuf,
        output_dir: Option<PathBuf>,

        /// Resolve config and print what would happen without encoding anything.
        /// Probes source files for copy-vs-transcode decisions. No files are
        /// written or created.
        #[arg(short = 'n', long = "dry-run")]
        dry_run: bool,

        /// Write the CLI overrides to a sidecar config file instead of passing
        /// them on every run. For a file target writes `<file>.bento.toml`; for
        /// a directory target writes `<dir>/bento.toml`. The encode still runs.
        /// Errors if no CLI overrides are present (nothing to write). Warns and
        /// skips (without overwriting) if the target file already exists.
        #[arg(long = "generate-config")]
        generate_config: bool,

        /// Keep the temp directory after the run completes instead of deleting it.
        /// Prints the preserved path at the end. Silent no-op under --dry-run.
        #[arg(long = "keep-intermediates")]
        keep_intermediates: bool,

        /// Overwrite existing output files. Shorthand for
        /// `--on-existing=overwrite`. Mutually exclusive with `--on-existing`.
        #[arg(short = 'f', long = "overwrite", conflicts_with = "on_existing")]
        overwrite: bool,

        /// Override the resolved `[output].on_existing` for this run.
        /// Values: `warn`, `skip-silently`, `overwrite`, `fail`.
        #[arg(long = "on-existing", value_name = "VALUE")]
        on_existing: Option<OnExistingArg>,

        /// Enable verbose output: print the ffmpeg command line before each
        /// encode and pass ffmpeg's full output through to the terminal.
        #[arg(short = 'v', long = "verbose", conflicts_with = "quiet")]
        verbose: bool,

        /// Suppress per-file progress; show only the end-of-batch summary and
        /// any errors.
        #[arg(short = 'q', long = "quiet", conflicts_with = "verbose")]
        quiet: bool,

        // --- Warning suppression --------------------------------------------
        /// Suppress all warnings for this run. Equivalent to passing every
        /// individual --no-warn-* flag.
        #[arg(long = "no-warnings")]
        no_warnings: bool,

        /// Suppress the "multiple burn subtitle tracks" warning.
        #[arg(long = "no-warn-multiple-burns")]
        no_warn_multiple_burns: bool,

        /// Suppress the "burn track has soft-only metadata" warning.
        #[arg(long = "no-warn-burn-metadata")]
        no_warn_burn_metadata: bool,

        /// Suppress the "lossy ASS→SRT conversion" warning.
        #[arg(long = "no-warn-ass-to-srt")]
        no_warn_ass_to_srt: bool,

        /// Suppress the "no default track" warning for audio and subtitles.
        #[arg(long = "no-warn-no-default")]
        no_warn_no_default: bool,

        /// Suppress the "CRF value suspicious for encoder" warning.
        #[arg(long = "no-warn-crf-codec-mismatch")]
        no_warn_crf_codec_mismatch: bool,

        /// Suppress the "field resolved from baked-in default" warning.
        /// (Warning not yet emitted; flag accepted for forward compatibility.)
        #[arg(long = "no-warn-missing")]
        no_warn_missing: bool,

        /// Suppress the "redundant config override" warning.
        /// (Warning not yet emitted; flag accepted for forward compatibility.)
        #[arg(long = "no-warn-redundant")]
        no_warn_redundant: bool,

        /// Override a config field for this run. KEY is a dotted path into the
        /// schema (e.g. `video.encoder.crf`); VALUE is a TOML scalar (`true`,
        /// `42`, `"quoted string"`). Bare strings are not accepted. May be
        /// repeated. Track lists (`audio.tracks`, `subtitles.tracks`) are not
        /// addressable via --set; use a sidecar config instead.
        #[arg(long = "set", value_name = "KEY=VALUE", num_args = 1)]
        set: Vec<String>,
    },
    /// Resolve and print the full config for a file or directory, with provenance.
    Config { path: PathBuf },
    /// Verify external dependencies and the global config.
    Check {
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },
    /// Repair the global config by inserting missing keys at their defaults.
    Repair {
        /// Auto-confirm all prompts (non-interactive / scripted use).
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },
    /// Show the streams in a video file — track numbers, codecs, and titles —
    /// formatted so you can copy source = N straight into your bento.toml.
    Probe { path: PathBuf },
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let mut stdout = std::io::stdout().lock();

    match cli.command {
        Command::Config { path } => crate::render::run_config(&path, &mut stdout),
        Command::Check { yes } => run_check(yes, &mut stdout),
        Command::Convert {
            path,
            output_dir,
            dry_run,
            generate_config,
            keep_intermediates,
            overwrite,
            on_existing,
            verbose,
            quiet,
            no_warnings,
            no_warn_multiple_burns,
            no_warn_burn_metadata,
            no_warn_ass_to_srt,
            no_warn_no_default,
            no_warn_crf_codec_mismatch,
            no_warn_missing,
            no_warn_redundant,
            set,
        } => {
            let on_existing_override = if overwrite {
                Some(OnExisting::Overwrite)
            } else {
                on_existing.map(OnExisting::from)
            };
            let verbosity = if verbose {
                Verbosity::Verbose
            } else if quiet {
                Verbosity::Quiet
            } else {
                Verbosity::Default
            };
            let warn_flags = WarnFlags {
                no_warnings,
                no_warn_multiple_burns,
                no_warn_burn_metadata,
                no_warn_ass_to_srt,
                no_warn_no_default,
                no_warn_crf_codec_mismatch,
                no_warn_missing,
                no_warn_redundant,
            };
            crate::pipeline::run_convert(
                &path,
                &mut stdout,
                ConvertOptions {
                    output_dir_override: output_dir,
                    on_existing_override,
                    generate_config,
                    dry_run,
                    verbosity,
                    warn_flags,
                    keep_intermediates,
                    set_overrides: set,
                },
            )
        }
        Command::Repair { yes } => crate::repair::run_repair(yes, &mut stdout),
        Command::Probe { path } => crate::probe::run_probe(&path, &mut stdout),
    }
}

pub fn run_check(yes: bool, out: &mut dyn Write) -> Result<()> {
    let path = crate::layers::global_config_path().ok_or(Error::NoConfigDir)?;
    crate::layers::ensure_global_config(&path, yes, out)?;

    writeln!(out).map_err(crate::io_render_err)?;

    let mut failures = 0usize;
    for name in &["ffmpeg", "ffprobe"] {
        if !check_binary(name, out)? {
            failures += 1;
        }
        writeln!(out).map_err(crate::io_render_err)?;
    }

    if failures > 0 {
        return Err(Error::CheckFailed { count: failures });
    }
    Ok(())
}

fn check_binary(name: &'static str, out: &mut dyn Write) -> Result<bool> {
    use crate::ffmpeg::{VersionBand, detect};

    match detect(name) {
        None => {
            writeln!(out, "{name}: not found").map_err(crate::io_render_err)?;
            write_install_hint(name, out)?;
            Ok(false)
        }
        Some(bin) => {
            let ver_str = bin.version.map(|v| format!(" ({v})")).unwrap_or_default();

            let band = bin.version.map(|v| v.band());

            match band {
                Some(VersionBand::BelowMinimum) => {
                    writeln!(
                        out,
                        "{name}: warning — version{ver_str} is below the required minimum ({min})",
                        min = crate::ffmpeg::MINIMUM
                    )
                    .map_err(crate::io_render_err)?;
                    if let Some(p) = &bin.path {
                        writeln!(out, "  {}", p.display()).map_err(crate::io_render_err)?;
                    }
                    writeln!(
                        out,
                        "  Bento may not work correctly. Please upgrade to {name} {} or later.",
                        crate::ffmpeg::MINIMUM
                    )
                    .map_err(crate::io_render_err)?;
                }
                Some(VersionBand::AboveTestedMajor) => {
                    writeln!(out, "{name}: ok{ver_str}").map_err(crate::io_render_err)?;
                    if let Some(p) = &bin.path {
                        writeln!(out, "  {}", p.display()).map_err(crate::io_render_err)?;
                    }
                    writeln!(out, "  note: version is above the tested release ({major}.x); behavior may differ", major = crate::ffmpeg::TESTED.major)
                        .map_err(crate::io_render_err)?;
                }
                Some(VersionBand::Ok) | None => {
                    writeln!(out, "{name}: ok{ver_str}").map_err(crate::io_render_err)?;
                    if let Some(p) = &bin.path {
                        writeln!(out, "  {}", p.display()).map_err(crate::io_render_err)?;
                    }
                }
            }
            Ok(true)
        }
    }
}

fn write_install_hint(name: &str, out: &mut dyn Write) -> Result<()> {
    writeln!(out, "  Install {name} to use Bento:").map_err(crate::io_render_err)?;
    #[cfg(target_os = "macos")]
    writeln!(out, "    brew install ffmpeg").map_err(crate::io_render_err)?;
    #[cfg(target_os = "linux")]
    {
        writeln!(out, "    apt install ffmpeg   (Debian/Ubuntu)").map_err(crate::io_render_err)?;
        writeln!(out, "    dnf install ffmpeg   (Fedora/RHEL)").map_err(crate::io_render_err)?;
    }
    writeln!(out, "    https://ffmpeg.org/download.html").map_err(crate::io_render_err)?;
    Ok(())
}
