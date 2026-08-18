//! Config-layer discovery and loading.

use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use console::style;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::resolve::Layer;

pub fn discover_layers(target: &Path, out: &mut dyn Write) -> Result<Vec<(Layer, Config)>> {
    let mut layers = Vec::new();

    if let Some(global_path) = global_config_path() {
        if global_path.exists() {
            let cfg = read_config_file(&global_path, out)?;
            layers.push((Layer::Global(global_path), cfg));
        }
    }

    let target_dir = target.parent().unwrap_or_else(|| Path::new("."));
    let dir_config = target_dir.join("bento.toml");
    if dir_config.exists() {
        let cfg = read_config_file(&dir_config, out)?;
        layers.push((Layer::Directory(dir_config), cfg));
    }

    let sidecar = sidecar_path(target);
    if sidecar.exists() {
        let cfg = read_config_file(&sidecar, out)?;
        layers.push((Layer::PerFile(sidecar), cfg));
    }

    Ok(layers)
}

pub fn sidecar_path(target: &Path) -> PathBuf {
    let mut name: OsString = target.as_os_str().to_owned();
    name.push(".bento.toml");
    PathBuf::from(name)
}

pub fn read_config_file(path: &Path, out: &mut dyn Write) -> Result<Config> {
    let text = fs::read_to_string(path).map_err(|e| Error::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    if text_is_blank(&text) {
        writeln!(
            out,
            "{} {} is blank; this layer contributes nothing to resolution. \
             Did you mean to add content?",
            style("warning:").yellow().bold(),
            path.display()
        )
        .map_err(crate::io_render_err)?;
    }
    toml::from_str::<Config>(&text).map_err(|e| Error::Toml {
        path: path.to_path_buf(),
        source: e,
    })
}

pub fn text_is_blank(text: &str) -> bool {
    text.strip_prefix('\u{FEFF}')
        .unwrap_or(text)
        .trim()
        .is_empty()
}

/// XDG-style global config path: `~/.config/bento/config.toml` on Linux/macOS,
/// `%APPDATA%\bento\config.toml` on Windows.
pub fn global_config_path() -> Option<PathBuf> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(config_home.join("bento").join("config.toml"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("bento").join("config.toml"))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

pub fn ensure_global_config(
    path: &Path,
    yes: bool,
    confirmer: &mut dyn Confirmer,
    out: &mut dyn Write,
) -> Result<()> {
    if path.exists() {
        read_config_file(path, out)?;
        writeln!(out, "  {}  global config: ok", style("✓").green().bold())
            .map_err(crate::io_render_err)?;
        writeln!(out, "     {}", style(path.display()).dim()).map_err(crate::io_render_err)?;
        return Ok(());
    }

    writeln!(
        out,
        "  {}  global config: not found",
        style("✗").red().bold()
    )
    .map_err(crate::io_render_err)?;
    writeln!(out, "     expected at: {}", style(path.display()).dim())
        .map_err(crate::io_render_err)?;

    let should_create = if yes {
        true
    } else {
        confirmer.confirm("Generate now?")?
    };

    if should_create {
        crate::bootstrap::write_global_config(path)?;
        writeln!(out, "     wrote {}", path.display()).map_err(crate::io_render_err)?;
    } else {
        writeln!(out, "     skipped").map_err(crate::io_render_err)?;
    }
    Ok(())
}

/// Supplies yes/no answers for confirmation prompts. Exists so callers
/// (`repair`, `check`) don't have to depend on the real process stdin being,
/// or not being, a terminal — [`Terminal`] does that for the CLI, while tests
/// can supply a fixed answer instead of exercising the real prompt.
pub trait Confirmer {
    fn confirm(&mut self, question: &str) -> Result<bool>;
}

/// Prompts the real terminal. Refuses with [`Error::NotInteractive`] when
/// stdin isn't a TTY (piped input, CI, a test harness) rather than blocking
/// on a read that will never be answered.
pub struct Terminal;

impl Confirmer for Terminal {
    fn confirm(&mut self, question: &str) -> Result<bool> {
        if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            confirm_via_stdin(question)
        } else {
            Err(Error::NotInteractive)
        }
    }
}

pub(crate) fn confirm_via_stdin(question: &str) -> Result<bool> {
    use std::io::{BufRead, Write as _};
    let mut stdout = std::io::stdout();
    let _ = write!(stdout, "{} [y/N] ", question);
    let _ = stdout.flush();

    let stdin = std::io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line).map_err(|e| Error::Io {
        path: PathBuf::from("<stdin>"),
        source: e,
    })?;
    let trimmed = line.trim();
    Ok(trimmed.eq_ignore_ascii_case("y") || trimmed.eq_ignore_ascii_case("yes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "bento-layers-test-{}-{}",
                std::process::id(),
                name
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn write(&self, name: &str, content: &str) -> PathBuf {
            let p = self.path.join(name);
            std::fs::write(&p, content).unwrap();
            p
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// A confirmer that never touches real stdin. None of these tests need
    /// an actual answer (they either pass `yes = true` or hit a return path
    /// before confirmation), so any use would indicate a test change that
    /// should also decide what the answer ought to be.
    struct Unreachable;

    impl Confirmer for Unreachable {
        fn confirm(&mut self, question: &str) -> Result<bool> {
            panic!("test unexpectedly prompted for confirmation: {question}");
        }
    }

    /// Mimics a non-interactive shell (piped input, CI): any confirmation
    /// attempt fails immediately instead of blocking on real stdin.
    struct NonInteractive;

    impl Confirmer for NonInteractive {
        fn confirm(&mut self, _question: &str) -> Result<bool> {
            Err(Error::NotInteractive)
        }
    }

    /// Returns a fixed answer, as if a real prompt had been answered.
    struct Fixed(bool);

    impl Confirmer for Fixed {
        fn confirm(&mut self, _question: &str) -> Result<bool> {
            Ok(self.0)
        }
    }

    #[test]
    fn resolve_subtitle_path_uses_config_dir_for_relative() {
        let config_dir = PathBuf::from("/show/season1");
        // Test that relative paths are resolved against a config dir
        let p = Path::new("edited.srt");
        let resolved = if p.is_absolute() {
            p.to_path_buf()
        } else {
            config_dir.join(p)
        };
        assert_eq!(resolved, PathBuf::from("/show/season1/edited.srt"));
    }

    #[test]
    fn text_is_blank_detects_empty() {
        assert!(text_is_blank(""));
        assert!(text_is_blank("   \n  \t\n\n"));
        assert!(text_is_blank("\u{FEFF}"));
        assert!(!text_is_blank("# comment"));
        assert!(!text_is_blank("[audio]\n"));
    }

    #[test]
    fn ensure_global_config_missing_non_interactive_errors_instead_of_prompting() {
        let dir = TestDir::new("check_non_interactive");
        let path = dir.path.join("config.toml");
        assert!(!path.exists());

        let mut buf = Vec::new();
        let result = ensure_global_config(&path, false, &mut NonInteractive, &mut buf);
        assert!(
            matches!(result, Err(Error::NotInteractive)),
            "got: {:?}",
            result
        );
        assert!(!path.exists());
    }

    #[test]
    fn ensure_global_config_confirmer_declines_skips_creation() {
        let dir = TestDir::new("check_declines");
        let path = dir.path.join("config.toml");
        assert!(!path.exists());

        let mut buf = Vec::new();
        ensure_global_config(&path, false, &mut Fixed(false), &mut buf).expect("should not error");
        let out = String::from_utf8(buf).unwrap();

        assert!(!path.exists());
        assert!(out.contains("skipped"), "got: {}", out);
    }

    #[test]
    fn ensure_global_config_confirmer_accepts_creates_missing() {
        let dir = TestDir::new("check_accepts");
        let path = dir.path.join("config.toml");
        assert!(!path.exists());

        let mut buf = Vec::new();
        ensure_global_config(&path, false, &mut Fixed(true), &mut buf).expect("should write");
        let out = String::from_utf8(buf).unwrap();

        assert!(path.exists());
        assert!(out.contains("wrote"), "got: {}", out);

        let written = std::fs::read_to_string(&path).unwrap();
        Config::from_toml_str(&written).expect("written config parses");
    }

    #[test]
    fn ensure_global_config_yes_writes_missing() {
        let dir = TestDir::new("check_writes");
        let path = dir.path.join("config.toml");
        assert!(!path.exists());

        let mut buf = Vec::new();
        ensure_global_config(&path, true, &mut Unreachable, &mut buf).expect("should write");
        let out = String::from_utf8(buf).unwrap();

        assert!(path.exists());
        assert!(out.contains("not found"));
        assert!(out.contains("wrote"));

        let written = std::fs::read_to_string(&path).unwrap();
        Config::from_toml_str(&written).expect("written config parses");
    }

    #[test]
    fn ensure_global_config_yes_creates_parent_directory() {
        let dir = TestDir::new("check_parent_dir");
        let path = dir.path.join("nested").join("subdir").join("config.toml");
        assert!(!path.parent().unwrap().exists());

        let mut buf = Vec::new();
        ensure_global_config(&path, true, &mut Unreachable, &mut buf).expect("should write");
        assert!(path.exists());
    }

    #[test]
    fn ensure_global_config_existing_valid_reports_ok() {
        let dir = TestDir::new("check_existing_ok");
        let path = dir.write("config.toml", "[output]\ncontainer = \"mp4\"\n");

        let mut buf = Vec::new();
        ensure_global_config(&path, false, &mut Unreachable, &mut buf).expect("should report ok");
        let out = String::from_utf8(buf).unwrap();

        assert!(out.contains("global config: ok"));
        assert!(!out.contains("wrote"));
    }

    #[test]
    fn ensure_global_config_broken_returns_parse_error() {
        let dir = TestDir::new("check_broken");
        let path = dir.write("config.toml", "this is = not = valid toml");

        let mut buf = Vec::new();
        let result = ensure_global_config(&path, false, &mut Unreachable, &mut buf);
        assert!(
            matches!(result, Err(crate::error::Error::Toml { .. })),
            "got: {:?}",
            result
        );
    }

    #[test]
    fn ensure_global_config_yes_does_not_overwrite_existing() {
        let dir = TestDir::new("check_no_overwrite");
        let original = "[output]\ncontainer = \"mkv\"\n";
        let path = dir.write("config.toml", original);

        let mut buf = Vec::new();
        ensure_global_config(&path, true, &mut Unreachable, &mut buf).expect("ok");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }
}
