/// Output verbosity level for `bento convert`.
///
/// - `Quiet` (`-q`): errors, warnings, and the end-of-batch summary only.
/// - `Default`: errors, warnings, per-file layer-count summary, brief progress
///   display (spinner / progress bar while ffmpeg runs).
/// - `Verbose` (`-v`): errors, warnings, full ffmpeg output passthrough, and
///   the ffmpeg command line printed before each encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Verbosity {
    Quiet,
    #[default]
    Default,
    Verbose,
}
