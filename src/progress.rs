//! Per-file visual progress display for `bento convert`.

use std::time::Duration;

use console::style;
use indicatif::{ProgressBar, ProgressStyle};

use crate::verbosity::Verbosity;

/// Update parsed from one line of ffmpeg's `-progress pipe:2` output.
pub struct ProgressUpdate {
    pub out_time_us: Option<u64>,
    pub is_end: bool,
}

/// Parse one key=value line from ffmpeg's `-progress pipe:2` output.
/// Returns `Some` only for lines we care about.
pub fn parse_progress_line(line: &str) -> Option<ProgressUpdate> {
    if let Some(val) = line.strip_prefix("out_time_us=") {
        let us = val.trim().parse::<i64>().unwrap_or(0).max(0) as u64;
        return Some(ProgressUpdate {
            out_time_us: Some(us),
            is_end: false,
        });
    }
    if line.trim_start().starts_with("progress=") {
        let is_end = line.contains("progress=end");
        return Some(ProgressUpdate {
            out_time_us: None,
            is_end,
        });
    }
    None
}

enum Inner {
    Bar(ProgressBar),
    Plain,
    Silent,
}

/// Manages the per-file visual display during encoding.
pub struct FileProgress {
    inner: Inner,
    /// Verbosity level — controls whether finish_* methods emit output.
    verbosity: Verbosity,
    /// Label shown in status lines: `"input → output"`.
    label: String,
    /// Counter string: `"[N/M]"`.
    counter: String,
    /// Wall-clock start time for elapsed formatting.
    start: std::time::Instant,
}

impl FileProgress {
    /// Create a new progress display.
    ///
    /// - `input_name`: basename of the source file.
    /// - `output_name`: basename of the output file.
    /// - `file_idx`: 1-based index of this file.
    /// - `file_count`: total number of files in the batch.
    /// - `duration_secs`: total media duration, if known (drives progress bar vs. spinner).
    /// - `config_summary`: one-line config provenance summary, e.g. `"14 settings (…)"`.
    ///   Pass `""` to omit the second line.
    /// - `verbosity`: controls display mode.
    pub fn new(
        input_name: &str,
        output_name: &str,
        file_idx: usize,
        file_count: usize,
        duration_secs: Option<f64>,
        config_summary: &str,
        verbosity: Verbosity,
    ) -> Self {
        let label = format!("{} → {}", input_name, output_name);
        let counter = format!("[{}/{}]", file_idx, file_count);
        let start = std::time::Instant::now();

        // Build the indicatif {msg}: the label (bold) with the config summary
        // on a second line (dim), so both lines travel together as a unit that
        // the bar/spinner template extends with a third `|- …` line.
        let msg = if config_summary.is_empty() {
            format!("{}", style(&label).bold())
        } else {
            format!(
                "{}\n   |- {}",
                style(&label).bold(),
                style(config_summary).dim(),
            )
        };

        let inner = match verbosity {
            Verbosity::Quiet | Verbosity::Verbose => Inner::Silent,
            Verbosity::Default => {
                if console::Term::stderr().is_term() {
                    let pb = build_progress_bar(duration_secs);
                    pb.set_prefix(counter.clone());
                    pb.set_message(msg);
                    pb.enable_steady_tick(Duration::from_millis(80));
                    Inner::Bar(pb)
                } else {
                    eprintln!("{} {}", counter, label);
                    if !config_summary.is_empty() {
                        eprintln!("   |- {}", config_summary);
                    }
                    Inner::Plain
                }
            }
        };

        Self {
            inner,
            verbosity,
            label,
            counter,
            start,
        }
    }

    /// Advance the progress bar position from an ffmpeg `out_time_us` value.
    pub fn update_time(&self, out_time_us: u64) {
        if let Inner::Bar(pb) = &self.inner {
            pb.set_position(out_time_us / 1000); // µs → ms
        }
    }

    /// Print a line above the bar (for warnings emitted during encode).
    pub fn println(&self, msg: &str) {
        match &self.inner {
            Inner::Bar(pb) => pb.println(msg),
            Inner::Plain => eprintln!("{}", msg),
            Inner::Silent => {}
        }
    }

    /// Consuming finish — clears the bar and prints a green success line.
    /// Suppressed in Quiet mode. Emits a trailing blank line to visually
    /// separate this file from the next (or from the batch summary separator).
    pub fn finish_ok(self) {
        if let Inner::Bar(pb) = &self.inner {
            pb.finish_and_clear();
        }
        if self.verbosity != Verbosity::Quiet {
            let elapsed = format_elapsed(self.start.elapsed());
            eprintln!(
                " {} {} {}  (done in {})",
                style("✓").green().bold(),
                style(&self.counter).dim(),
                self.label,
                style(elapsed).dim(),
            );
            eprintln!();
        }
    }

    /// Consuming finish — clears the bar and prints a yellow skip line.
    /// Suppressed in Quiet mode.
    pub fn finish_skip(self, reason: &str) {
        if let Inner::Bar(pb) = &self.inner {
            pb.finish_and_clear();
        }
        if self.verbosity != Verbosity::Quiet {
            eprintln!(
                " {} {} {}  (skipped — {})",
                style("–").yellow().bold(),
                style(&self.counter).dim(),
                self.label,
                reason,
            );
            eprintln!();
        }
    }

    /// Consuming finish — clears the bar and prints a red error line.
    /// Suppressed in Quiet mode.
    pub fn finish_err(self) {
        if let Inner::Bar(pb) = &self.inner {
            pb.finish_and_clear();
        }
        if self.verbosity != Verbosity::Quiet {
            eprintln!(
                " {} {} {}",
                style("✗").red().bold(),
                style(&self.counter).dim(),
                self.label,
            );
            eprintln!();
        }
    }

    /// Returns a clone of the inner `ProgressBar` (if any) for use in a
    /// background thread. The clone shares state with the original — advancing
    /// the clone advances the bar on screen.
    pub fn bar_clone(&self) -> Option<ProgressBar> {
        match &self.inner {
            Inner::Bar(pb) => Some(pb.clone()),
            _ => None,
        }
    }
}

fn build_progress_bar(duration_secs: Option<f64>) -> ProgressBar {
    if let Some(dur) = duration_secs.filter(|&d| d > 0.0) {
        // Determinate progress bar (total in milliseconds).
        // The template has two lines: {msg} already contains the label + config
        // summary (with an embedded \n), and the template appends the bar line.
        let total_ms = (dur * 1000.0).ceil() as u64;
        let pb = ProgressBar::new(total_ms);
        let style = ProgressStyle::with_template(
            " {spinner:.cyan} {prefix:.dim} {msg}\n   |- [{bar:28.green/black.dim}]  {percent:>3}%  ETA {eta:.dim}",
        )
        .unwrap()
        .progress_chars("██░");
        pb.set_style(style);
        pb
    } else {
        // Indeterminate spinner.
        let pb = ProgressBar::new_spinner();
        let style = ProgressStyle::with_template(
            " {spinner:.cyan} {prefix:.dim} {msg}\n   |- {elapsed_precise:.dim}",
        )
        .unwrap()
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", ""]);
        pb.set_style(style);
        pb
    }
}

fn format_elapsed(d: Duration) -> String {
    let total_secs = d.as_secs();
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    if hours > 0 {
        format!("{}:{:02}:{:02}", hours, mins, secs)
    } else {
        format!("{}:{:02}", mins, secs)
    }
}
