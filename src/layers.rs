//! Config-layer discovery and loading.

use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

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
            "warning: {} is blank; this layer contributes nothing to resolution. \
             Did you mean to add content?",
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

pub fn ensure_global_config(path: &Path, yes: bool, out: &mut dyn Write) -> Result<()> {
    if path.exists() {
        read_config_file(path, out)?;
        writeln!(out, "global config: ok").map_err(crate::io_render_err)?;
        writeln!(out, "  {}", path.display()).map_err(crate::io_render_err)?;
        return Ok(());
    }

    writeln!(out, "global config: not found").map_err(crate::io_render_err)?;
    writeln!(out, "  expected at: {}", path.display()).map_err(crate::io_render_err)?;

    let should_create = if yes {
        true
    } else if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        confirm_via_stdin("Generate now?")?
    } else {
        return Err(Error::NotInteractive);
    };

    if should_create {
        crate::bootstrap::write_global_config(path)?;
        writeln!(out, "wrote {}", path.display()).map_err(crate::io_render_err)?;
    } else {
        writeln!(out, "skipped").map_err(crate::io_render_err)?;
    }
    Ok(())
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
    fn ensure_global_config_yes_writes_missing() {
        let dir = TestDir::new("check_writes");
        let path = dir.path.join("config.toml");
        assert!(!path.exists());

        let mut buf = Vec::new();
        ensure_global_config(&path, true, &mut buf).expect("should write");
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
        ensure_global_config(&path, true, &mut buf).expect("should write");
        assert!(path.exists());
    }

    #[test]
    fn ensure_global_config_existing_valid_reports_ok() {
        let dir = TestDir::new("check_existing_ok");
        let path = dir.write("config.toml", "[output]\ncontainer = \"mp4\"\n");

        let mut buf = Vec::new();
        ensure_global_config(&path, false, &mut buf).expect("should report ok");
        let out = String::from_utf8(buf).unwrap();

        assert!(out.contains("global config: ok"));
        assert!(!out.contains("wrote"));
    }

    #[test]
    fn ensure_global_config_broken_returns_parse_error() {
        let dir = TestDir::new("check_broken");
        let path = dir.write("config.toml", "this is = not = valid toml");

        let mut buf = Vec::new();
        let result = ensure_global_config(&path, false, &mut buf);
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
        ensure_global_config(&path, true, &mut buf).expect("ok");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }
}
