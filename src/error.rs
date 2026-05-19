use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse {path}:\n{source}")]
    Toml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("could not determine config directory for the current platform")]
    NoConfigDir,

    #[error("path does not exist: {0}")]
    PathNotFound(PathBuf),

    #[error("{errors} config error(s) in {path} (see output above)")]
    ConfigInvalid {
        path: PathBuf,
        errors: usize,
        warnings: usize,
    },

    #[error("{count} file(s) failed in batch (see output above)")]
    BatchFailed { count: usize },

    #[error("non-interactive context (no TTY); rerun with --yes to auto-confirm")]
    NotInteractive,

    #[error("required field `{field}` is not set in any config layer; configure it in the global, directory, or per-file config")]
    RequiredFieldMissing { field: String },

    #[error("output file already exists: {path}")]
    OutputExists { path: PathBuf },

    #[error(
        "ffmpeg not found on PATH. Install it (e.g. `brew install ffmpeg` on macOS, \
         `apt install ffmpeg` on Debian/Ubuntu) and try again."
    )]
    FfmpegNotFound,

    #[error("ffprobe failed (status {status}): {context}")]
    FfprobeFailed { status: i32, context: String },

    #[error("ffmpeg failed extracting subtitle track {track} from {input} (status {status})")]
    FfmpegExtractFailed {
        status: i32,
        track: u32,
        input: PathBuf,
    },

    #[error("ffmpeg encode failed for {input} (status {status})")]
    FfmpegEncodeFailed { status: i32, input: PathBuf },

    #[error("failed to parse SRT file {path} at line {line}: {message}")]
    SubtitleParse {
        path: PathBuf,
        line: usize,
        message: String,
    },
}

pub type Result<T> = std::result::Result<T, Error>;
