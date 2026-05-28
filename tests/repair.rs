//! Integration tests for `bento repair`.
//!
//! These call [`bento::repair::run_repair_at`] with a temp-dir path rather
//! than touching the real global config.

use std::path::PathBuf;

use bento::repair::run_repair_at;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "bento-repair-integ-{}-{}",
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

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.path.join(name)).unwrap()
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn run(path: &std::path::Path, yes: bool) -> (String, bento::error::Result<()>) {
    let mut buf: Vec<u8> = Vec::new();
    let r = run_repair_at(path, yes, &mut buf);
    (String::from_utf8(buf).unwrap(), r)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn noop_on_complete_config() {
    let dir = TestDir::new("noop");
    let cfg = dir.write(
        "config.toml",
        bento::bootstrap::generate_global_config_text(),
    );
    let (out, r) = run(&cfg, false);
    r.unwrap();
    assert!(out.contains("up to date"), "got: {}", out);
}

#[test]
fn reports_missing_field() {
    let dir = TestDir::new("reports");
    let modified =
        bento::bootstrap::generate_global_config_text().replace("normalize_downmix = true\n", "");
    let cfg = dir.write("config.toml", &modified);

    // yes=false in a non-TTY context → NotInteractive (confirm never reached
    // when there are missing fields), so we check the output before that.
    let (out, _) = run(&cfg, false);
    assert!(
        out.contains("audio.normalize_downmix"),
        "missing field listed; got: {}",
        out
    );
}

#[test]
fn inserts_missing_field_with_yes() {
    let dir = TestDir::new("inserts");
    let modified =
        bento::bootstrap::generate_global_config_text().replace("normalize_downmix = true\n", "");
    dir.write("config.toml", &modified);
    let cfg = dir.path.join("config.toml");

    let (out, r) = run(&cfg, true);
    r.unwrap();
    assert!(out.contains("field(s) added"), "confirmed; got: {}", out);

    let repaired = dir.read("config.toml");
    assert!(
        repaired.contains("normalize_downmix = true"),
        "field inserted"
    );
    bento::config::Config::from_toml_str(&repaired).expect("repaired config parses");
}

#[test]
fn missing_file_suggests_check() {
    let dir = TestDir::new("nofile");
    let cfg = dir.path.join("config.toml");
    let (out, r) = run(&cfg, false);
    r.unwrap();
    assert!(out.contains("not found"), "got: {}", out);
    assert!(out.contains("bento check"), "got: {}", out);
}

#[test]
fn corrupt_config_with_yes_regenerates() {
    let dir = TestDir::new("corrupt");
    dir.write("config.toml", "this = is = not = valid toml !!!");
    let cfg = dir.path.join("config.toml");

    let (out, r) = run(&cfg, true);
    r.unwrap();
    assert!(out.contains("invalid TOML"), "got: {}", out);
    assert!(out.contains("Wrote"), "got: {}", out);

    let written = dir.read("config.toml");
    bento::config::Config::from_toml_str(&written).expect("regenerated config parses");
}

#[test]
fn multiple_missing_fields_all_inserted() {
    let dir = TestDir::new("multi");
    let modified = bento::bootstrap::generate_global_config_text()
        .replace("normalize_downmix = true\n", "")
        .replace("preserve_chapters = true\n", "");
    dir.write("config.toml", &modified);
    let cfg = dir.path.join("config.toml");

    let (out, r) = run(&cfg, true);
    r.unwrap();
    assert!(out.contains("field(s) added"));

    let repaired = dir.read("config.toml");
    assert!(
        repaired.contains("normalize_downmix = true"),
        "normalize_downmix inserted"
    );
    assert!(
        repaired.contains("preserve_chapters = true"),
        "preserve_chapters inserted"
    );
    bento::config::Config::from_toml_str(&repaired).expect("repaired config parses");
}

#[test]
fn inserted_fields_carry_repair_marker_comment() {
    let dir = TestDir::new("comments");
    let modified =
        bento::bootstrap::generate_global_config_text().replace("normalize_downmix = true\n", "");
    dir.write("config.toml", &modified);
    let cfg = dir.path.join("config.toml");

    run(&cfg, true).1.unwrap();
    let repaired = dir.read("config.toml");
    assert!(
        repaired.contains("# (added by bento repair)"),
        "repair marker present; got:\n{}",
        repaired
    );
}
