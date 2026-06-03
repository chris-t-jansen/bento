use std::process::ExitCode;

use console::style;

fn main() -> ExitCode {
    match bento::cli::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{} {}", style("error:").red().bold(), e);
            ExitCode::FAILURE
        }
    }
}
