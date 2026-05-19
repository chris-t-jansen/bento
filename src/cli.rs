//! CLI surface: argument parsing and subcommand dispatch.

use std::io::Write;
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::config::OnExisting;
use crate::error::{Error, Result};

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

        /// Overwrite existing output files. Shorthand for
        /// `--on-existing=overwrite`. Mutually exclusive with `--on-existing`.
        #[arg(short = 'f', long = "overwrite", conflicts_with = "on_existing")]
        overwrite: bool,

        /// Override the resolved `[output].on_existing` for this run.
        /// Values: `warn`, `skip-silently`, `overwrite`, `fail`.
        #[arg(long = "on-existing", value_name = "VALUE")]
        on_existing: Option<OnExistingArg>,
    },
    /// Resolve and print the full config for a file or directory, with provenance.
    Config { path: PathBuf },
    /// Verify external dependencies and the global config.
    Check {
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },
    /// Repair the global config by inserting missing keys at their defaults.
    Repair,
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
            overwrite,
            on_existing,
        } => {
            let on_existing_override = if overwrite {
                Some(OnExisting::Overwrite)
            } else {
                on_existing.map(OnExisting::from)
            };
            crate::pipeline::run_convert(
                &path,
                output_dir.as_deref(),
                on_existing_override,
                &mut stdout,
            )
        }
        Command::Repair => unimplemented!("repair (deferred)"),
    }
}

pub fn run_check(yes: bool, out: &mut dyn Write) -> Result<()> {
    let path = crate::layers::global_config_path().ok_or(Error::NoConfigDir)?;
    crate::layers::ensure_global_config(&path, yes, out)?;

    writeln!(out).map_err(crate::io_render_err)?;
    writeln!(
        out,
        "(External binary check for ffmpeg will be added in a later release.)"
    )
    .map_err(crate::io_render_err)?;
    Ok(())
}
