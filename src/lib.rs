pub mod bootstrap;
pub mod cli;
pub mod config;
pub mod error;
pub mod ffmpeg;
pub mod layers;
pub mod pipeline;
pub mod progress;
pub mod render;
pub mod resolve;
pub mod set_override;
pub mod subtitles;
pub mod validate;
pub mod verbosity;

pub(crate) fn io_render_err(e: std::io::Error) -> crate::error::Error {
    crate::error::Error::Io {
        path: std::path::PathBuf::from("<output>"),
        source: e,
    }
}
