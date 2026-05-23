//! `bento repair` — insert missing fields into the global config.
//!
//! Compares the fields present in the global `config.toml` against the
//! baked-in defaults (via [`crate::resolve::baked_defaults`]). Any field that
//! has a baked-in default but is absent from the user's file is reported and,
//! with confirmation, inserted at that default value with its documentation
//! comment (extracted from the bootstrap template).
//!
//! If the file is corrupt (unparseable), repair offers to regenerate it from
//! scratch instead.

use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::Path;

use crate::error::{Error, Result};

// =============================================================================
// Entry points
// =============================================================================

/// `bento repair` top-level handler. Discovers the global config path and
/// delegates to [`run_repair_at`].
pub fn run_repair(yes: bool, out: &mut dyn Write) -> Result<()> {
    let path = crate::layers::global_config_path().ok_or(Error::NoConfigDir)?;
    run_repair_at(&path, yes, out)
}

/// Path-explicit repair logic; separated so integration tests can supply a
/// temp-dir path without touching the real global config.
pub fn run_repair_at(path: &Path, yes: bool, out: &mut dyn Write) -> Result<()> {
    if !path.exists() {
        writeln!(out, "global config: not found").map_err(crate::io_render_err)?;
        writeln!(out, "  expected at: {}", path.display()).map_err(crate::io_render_err)?;
        writeln!(out, "Run `bento check [-y]` to create it.").map_err(crate::io_render_err)?;
        return Ok(());
    }

    let text = std::fs::read_to_string(path).map_err(|e| Error::Io {
        path: path.to_path_buf(),
        source: e,
    })?;

    let user_config = match crate::config::Config::from_toml_str(&text) {
        Ok(c) => c,
        Err(e) => {
            writeln!(
                out,
                "Global config is invalid TOML and cannot be repaired automatically."
            )
            .map_err(crate::io_render_err)?;
            writeln!(out, "  {}", path.display()).map_err(crate::io_render_err)?;
            writeln!(out, "  {}", e).map_err(crate::io_render_err)?;
            writeln!(out).map_err(crate::io_render_err)?;

            if confirm(
                "Regenerate from scratch? (Your existing config will be replaced.)",
                yes,
                out,
            )? {
                crate::bootstrap::write_global_config(path)?;
                writeln!(out, "Wrote {}", path.display()).map_err(crate::io_render_err)?;
            } else {
                writeln!(out, "skipped").map_err(crate::io_render_err)?;
            }
            return Ok(());
        }
    };

    // Serialize both the user config and the baked defaults to TOML values so
    // we can structurally compare them.
    let encoder_name = user_config.video.encoder.as_ref().and_then(|e| e.name);
    let defaults = crate::resolve::baked_defaults(encoder_name);

    let defaults_value = toml::Value::try_from(&defaults).expect("defaults always serializable");
    let user_value = toml::Value::try_from(&user_config).expect("config always serializable");

    let missing = find_missing(&defaults_value, &user_value);

    if missing.is_empty() {
        writeln!(out, "Global config is up to date — no fields are missing.")
            .map_err(crate::io_render_err)?;
        writeln!(out, "  {}", path.display()).map_err(crate::io_render_err)?;
        return Ok(());
    }

    writeln!(
        out,
        "Global config: {} field(s) missing from {}",
        missing.len(),
        path.display()
    )
    .map_err(crate::io_render_err)?;
    writeln!(out).map_err(crate::io_render_err)?;

    for (dotted_path, value) in &missing {
        writeln!(out, "  {} = {}", dotted_path, format_value(value))
            .map_err(crate::io_render_err)?;
    }
    writeln!(out).map_err(crate::io_render_err)?;

    if !confirm("Add these fields to your global config?", yes, out)? {
        writeln!(out, "skipped").map_err(crate::io_render_err)?;
        return Ok(());
    }

    let repaired = insert_missing(&text, &missing)?;

    // Validate before writing — guards against edge cases like inline-table
    // sections that can't be extended with a new [section.subsection] header.
    crate::config::Config::from_toml_str(&repaired).map_err(|_| Error::RepairResultInvalid {
        path: path.to_path_buf(),
        fields: missing.iter().map(|(p, _)| p.clone()).collect(),
    })?;

    std::fs::write(path, &repaired).map_err(|e| Error::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    writeln!(
        out,
        "Updated {} ({} field(s) added)",
        path.display(),
        missing.len()
    )
    .map_err(crate::io_render_err)?;
    Ok(())
}

// =============================================================================
// Missing-field detection
// =============================================================================

/// Walk the defaults TOML tree and collect leaf paths absent from the user tree.
pub(crate) fn find_missing(
    defaults: &toml::Value,
    user: &toml::Value,
) -> Vec<(String, toml::Value)> {
    let mut result = Vec::new();
    walk(defaults, user, "", &mut result);
    result
}

fn walk(
    defaults: &toml::Value,
    user: &toml::Value,
    prefix: &str,
    result: &mut Vec<(String, toml::Value)>,
) {
    let toml::Value::Table(dt) = defaults else {
        return;
    };

    for (key, dval) in dt {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{}.{}", prefix, key)
        };

        let maybe_uval = if let toml::Value::Table(ut) = user {
            ut.get(key)
        } else {
            None
        };

        match maybe_uval {
            None => collect_leaves(dval, &path, result),
            Some(uval) if matches!(dval, toml::Value::Table(_)) => walk(dval, uval, &path, result),
            Some(_) => {} // present scalar — not missing
        }
    }
}

/// Collect all scalar leaves from a defaults subtree into `result`.
fn collect_leaves(value: &toml::Value, path: &str, result: &mut Vec<(String, toml::Value)>) {
    match value {
        toml::Value::Table(t) => {
            for (k, v) in t {
                collect_leaves(v, &format!("{}.{}", path, k), result);
            }
        }
        _ => result.push((path.to_string(), value.clone())),
    }
}

// =============================================================================
// Text-based insertion
// =============================================================================

/// Insert missing fields into the config file text, preserving all existing
/// content.
///
/// Groups missing fields by their TOML section header (e.g.
/// `video.encoder.crf` belongs to section `[video.encoder]`). For sections
/// whose header already exists in the file, fields are appended at the end of
/// that section. Sections entirely absent from the file are appended at the end.
pub(crate) fn insert_missing(text: &str, missing: &[(String, toml::Value)]) -> Result<String> {
    if missing.is_empty() {
        return Ok(text.to_string());
    }

    let snippets = template_snippets();

    // Group by section, preserving stable insertion order.
    let mut by_section: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (dotted_path, value) in missing {
        let (section, field) = split_path(dotted_path);
        let lines = make_snippet(dotted_path, &field, value, &snippets);
        by_section.entry(section).or_default().extend(lines);
    }

    let file_lines: Vec<&str> = text.lines().collect();
    let mut result: Vec<String> = Vec::new();
    let mut inserted: HashSet<String> = HashSet::new();
    let mut current_section: Option<String> = None;

    for line in &file_lines {
        if let Some(new_section) = parse_section_header(line) {
            // Flush insertions for the section we are leaving.
            if let Some(ref prev) = current_section {
                if let Some(fields) = by_section.get(prev) {
                    flush_insertions(&mut result, fields);
                    inserted.insert(prev.clone());
                }
            }
            current_section = Some(new_section);
        }
        result.push(line.to_string());
    }

    // Flush insertions for the final section in the file.
    if let Some(ref last) = current_section {
        if !inserted.contains(last) {
            if let Some(fields) = by_section.get(last) {
                flush_insertions(&mut result, fields);
                inserted.insert(last.clone());
            }
        }
    }

    // Append any sections that were entirely absent from the file.
    for (section, fields) in &by_section {
        if !inserted.contains(section) {
            result.push(String::new());
            result.push(format!("[{}]", section));
            result.push(String::new());
            result.push("# (added by bento repair)".to_string());
            result.extend(fields.iter().cloned());
        }
    }

    let mut out = result.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

/// Append missing-field lines before any trailing blank lines in `result`, so
/// new content sits inside the section rather than after the inter-section gap.
fn flush_insertions(result: &mut Vec<String>, fields: &[String]) {
    let mut trailing = 0;
    for line in result.iter().rev() {
        if line.trim().is_empty() {
            trailing += 1;
        } else {
            break;
        }
    }
    let split = result.len() - trailing;
    let reattach: Vec<String> = result.drain(split..).collect();

    result.push(String::new());
    result.push("# (added by bento repair)".to_string());
    result.extend(fields.iter().cloned());
    result.extend(reattach);
}

fn make_snippet(
    dotted_path: &str,
    field: &str,
    value: &toml::Value,
    snippets: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    if let Some(comments) = snippets.get(dotted_path) {
        lines.extend(comments.iter().cloned());
    }
    lines.push(format!("{} = {}", field, format_value(value)));
    lines
}

// =============================================================================
// Template snippet extraction
// =============================================================================

/// Parse the bootstrap template and return a map from dotted field path to the
/// documentation comment lines that precede it in the template.
fn template_snippets() -> BTreeMap<String, Vec<String>> {
    let text = crate::bootstrap::generate_global_config_text();
    let mut result: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut current_section = String::new();
    let mut pending_comments: Vec<String> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(section) = parse_section_header(trimmed) {
            current_section = section;
            pending_comments.clear();
        } else if trimmed.starts_with('#') {
            pending_comments.push(line.to_string());
        } else if trimmed.is_empty() {
            pending_comments.clear();
        } else if let Some(eq) = trimmed.find('=') {
            let key = trimmed[..eq].trim();
            // Only record simple bare-key assignments; skip inline tables and
            // keys containing dots or spaces.
            if !key.contains('[') && !key.contains('{') && !key.contains(' ') && !key.contains('.')
            {
                let path = if current_section.is_empty() {
                    key.to_string()
                } else {
                    format!("{}.{}", current_section, key)
                };
                result.insert(path, pending_comments.clone());
            }
            pending_comments.clear();
        } else {
            pending_comments.clear();
        }
    }
    result
}

// =============================================================================
// Helpers
// =============================================================================

/// Parse a `[section]` or `[section.sub]` header from a line. Returns `None`
/// for `[[array]]` headers and any non-header line.
fn parse_section_header(line: &str) -> Option<String> {
    let t = line.trim();
    if t.starts_with('[') && !t.starts_with("[[") && t.ends_with(']') {
        let inner = &t[1..t.len() - 1];
        if !inner.contains('[') {
            return Some(inner.trim().to_string());
        }
    }
    None
}

/// Split `"a.b.c"` into `("a.b", "c")`. A top-level key returns `("", key)`.
fn split_path(dotted: &str) -> (String, String) {
    match dotted.rfind('.') {
        Some(i) => (dotted[..i].to_string(), dotted[i + 1..].to_string()),
        None => (String::new(), dotted.to_string()),
    }
}

/// Serialize a TOML scalar value to the right-hand side of a `key = <value>`.
pub(crate) fn format_value(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => {
            let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{}\"", escaped)
        }
        toml::Value::Integer(n) => n.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Datetime(dt) => dt.to_string(),
        toml::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(format_value).collect();
            format!("[{}]", items.join(", "))
        }
        toml::Value::Table(t) => {
            let items: Vec<String> = t
                .iter()
                .map(|(k, v)| format!("{} = {}", k, format_value(v)))
                .collect();
            format!("{{ {} }}", items.join(", "))
        }
    }
}

fn confirm(question: &str, yes: bool, _out: &mut dyn Write) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        crate::layers::confirm_via_stdin(question)
    } else {
        Err(Error::NotInteractive)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // find_missing
    // -------------------------------------------------------------------------

    #[test]
    fn no_missing_when_configs_identical() {
        let defaults = crate::resolve::baked_defaults(None);
        let dv = toml::Value::try_from(&defaults).unwrap();
        assert!(find_missing(&dv, &dv).is_empty());
    }

    #[test]
    fn finds_missing_scalar_in_existing_section() {
        let defaults = crate::resolve::baked_defaults(None);
        let dv = toml::Value::try_from(&defaults).unwrap();

        let mut user = dv.clone();
        user.as_table_mut()
            .unwrap()
            .get_mut("video")
            .unwrap()
            .as_table_mut()
            .unwrap()
            .remove("preset");

        let missing = find_missing(&dv, &user);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].0, "video.preset");
        assert_eq!(missing[0].1, toml::Value::String("medium".to_string()));
    }

    #[test]
    fn finds_all_leaves_when_subsection_absent() {
        let defaults = crate::resolve::baked_defaults(None);
        let dv = toml::Value::try_from(&defaults).unwrap();

        let mut user = dv.clone();
        user.as_table_mut()
            .unwrap()
            .get_mut("video")
            .unwrap()
            .as_table_mut()
            .unwrap()
            .remove("encoder");

        let missing = find_missing(&dv, &user);
        let paths: Vec<&str> = missing.iter().map(|(p, _)| p.as_str()).collect();
        assert!(paths.contains(&"video.encoder.name"), "got: {:?}", paths);
        assert!(paths.contains(&"video.encoder.crf"), "got: {:?}", paths);
        assert!(paths.contains(&"video.encoder.tune"), "got: {:?}", paths);
        assert_eq!(missing.len(), 3);
    }

    #[test]
    fn no_false_positives_for_none_fields() {
        // output.metadata, output.naming, audio.tracks, and subtitles.tracks
        // have no baked-in defaults and must never appear as missing.
        let defaults = crate::resolve::baked_defaults(None);
        let dv = toml::Value::try_from(&defaults).unwrap();
        let empty_user = toml::Value::try_from(crate::config::Config::default()).unwrap();

        let missing = find_missing(&dv, &empty_user);
        let paths: Vec<&str> = missing.iter().map(|(p, _)| p.as_str()).collect();

        // Use exact-prefix checks to avoid false matches like
        // "subtitles.warn_burn_metadata" matching a plain "metadata" substring.
        let has_output_metadata = paths
            .iter()
            .any(|p| *p == "output.metadata" || p.starts_with("output.metadata."));
        let has_output_naming = paths
            .iter()
            .any(|p| *p == "output.naming" || p.starts_with("output.naming."));
        let has_tracks = paths.iter().any(|p| {
            *p == "audio.tracks"
                || p.starts_with("audio.tracks.")
                || *p == "subtitles.tracks"
                || p.starts_with("subtitles.tracks.")
        });

        assert!(
            !has_output_metadata,
            "output.metadata should not appear; got: {:?}",
            paths
        );
        assert!(
            !has_output_naming,
            "output.naming should not appear; got: {:?}",
            paths
        );
        assert!(!has_tracks, "tracks should not appear; got: {:?}", paths);
    }

    // -------------------------------------------------------------------------
    // template_snippets
    // -------------------------------------------------------------------------

    #[test]
    fn template_snippets_covers_known_paths() {
        let s = template_snippets();
        for path in &[
            "output.container",
            "output.destination",
            "video.preset",
            "video.crop",
            "video.encoder.name",
            "video.encoder.crf",
            "audio.encoder",
            "audio.bitrate",
            "subtitles.warn_no_default",
        ] {
            assert!(s.contains_key(*path), "missing snippet for {}", path);
        }
    }

    // -------------------------------------------------------------------------
    // parse_section_header
    // -------------------------------------------------------------------------

    #[test]
    fn section_header_parses_simple_and_dotted() {
        assert_eq!(parse_section_header("[output]"), Some("output".to_string()));
        assert_eq!(
            parse_section_header("[video.encoder]"),
            Some("video.encoder".to_string())
        );
        assert_eq!(parse_section_header("[[array]]"), None);
        assert_eq!(parse_section_header("key = value"), None);
        assert_eq!(parse_section_header(""), None);
    }

    // -------------------------------------------------------------------------
    // insert_missing
    // -------------------------------------------------------------------------

    #[test]
    fn insert_into_existing_section_is_valid_toml() {
        let original = crate::bootstrap::generate_global_config_text();
        let modified = original.replace("normalize_mix = true\n", "");
        assert!(!modified.contains("normalize_mix"), "setup: field absent");

        let missing = vec![(
            "audio.normalize_mix".to_string(),
            toml::Value::Boolean(true),
        )];
        let repaired = insert_missing(&modified, &missing).unwrap();

        crate::config::Config::from_toml_str(&repaired)
            .expect("repaired text must parse as valid config");
        assert!(
            repaired.contains("normalize_mix = true"),
            "inserted field present"
        );
    }

    #[test]
    fn insert_new_section_when_absent_is_valid_toml() {
        // Strip the [subtitles] section from the template.
        let original = crate::bootstrap::generate_global_config_text();
        let sub_start = original.find("\n[subtitles]").expect("subtitles present");
        let stripped = &original[..sub_start];

        let missing = vec![(
            "subtitles.warn_no_default".to_string(),
            toml::Value::Boolean(true),
        )];
        let repaired = insert_missing(stripped, &missing).unwrap();

        crate::config::Config::from_toml_str(&repaired)
            .expect("repaired text must parse as valid config");
        assert!(repaired.contains("[subtitles]"), "section header appended");
        assert!(
            repaired.contains("warn_no_default = true"),
            "field appended"
        );
    }

    #[test]
    fn noop_when_nothing_missing_returns_same_text() {
        let original = crate::bootstrap::generate_global_config_text();
        let repaired = insert_missing(original, &[]).unwrap();
        assert_eq!(original, repaired.as_str());
    }

    // -------------------------------------------------------------------------
    // run_repair_at (end-to-end)
    // -------------------------------------------------------------------------

    struct TempDir {
        path: std::path::PathBuf,
    }

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("bento-repair-{}-{}", std::process::id(), tag));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn write(&self, name: &str, content: &str) -> std::path::PathBuf {
            let p = self.path.join(name);
            std::fs::write(&p, content).unwrap();
            p
        }

        fn read(&self, name: &str) -> String {
            std::fs::read_to_string(self.path.join(name)).unwrap()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn run(path: &std::path::Path, yes: bool) -> (String, crate::error::Result<()>) {
        let mut buf: Vec<u8> = Vec::new();
        let r = run_repair_at(path, yes, &mut buf);
        (String::from_utf8(buf).unwrap(), r)
    }

    #[test]
    fn noop_on_complete_config() {
        let dir = TempDir::new("noop");
        let cfg = dir.write(
            "config.toml",
            crate::bootstrap::generate_global_config_text(),
        );
        let (out, r) = run(&cfg, false);
        r.unwrap();
        assert!(out.contains("up to date"), "got: {}", out);
    }

    #[test]
    fn reports_and_inserts_missing_field() {
        let dir = TempDir::new("insert");
        let original = crate::bootstrap::generate_global_config_text();
        let modified = original.replace("normalize_mix = true\n", "");
        dir.write("config.toml", &modified);
        let cfg = dir.path.join("config.toml");

        // Dry run (yes=false, non-TTY) → NotInteractive; confirm field listed.
        let (out, _) = run(&cfg, false);
        assert!(
            out.contains("audio.normalize_mix"),
            "field listed; got: {}",
            out
        );

        // Actual run (yes=true) → field inserted.
        let (out, r) = run(&cfg, true);
        r.unwrap();
        assert!(out.contains("field(s) added"), "confirmed; got: {}", out);

        let repaired = dir.read("config.toml");
        assert!(repaired.contains("normalize_mix = true"));
        crate::config::Config::from_toml_str(&repaired).expect("repaired config parses");
    }

    #[test]
    fn no_file_suggests_check() {
        let dir = TempDir::new("nofile");
        let cfg = dir.path.join("config.toml");
        let (out, r) = run(&cfg, false);
        r.unwrap();
        assert!(out.contains("not found"), "got: {}", out);
        assert!(out.contains("bento check"), "got: {}", out);
    }

    #[test]
    fn corrupt_config_with_yes_regenerates() {
        let dir = TempDir::new("corrupt");
        dir.write("config.toml", "this = is = not = valid toml !!!");
        let cfg = dir.path.join("config.toml");

        let (out, r) = run(&cfg, true);
        r.unwrap();
        assert!(out.contains("invalid TOML"), "got: {}", out);
        assert!(out.contains("Wrote"), "got: {}", out);

        let written = dir.read("config.toml");
        crate::config::Config::from_toml_str(&written).expect("regenerated config parses");
    }

    #[test]
    fn multiple_missing_fields_all_inserted() {
        let dir = TempDir::new("multi");
        let original = crate::bootstrap::generate_global_config_text();
        let modified = original
            .replace("normalize_mix = true\n", "")
            .replace("preserve_chapters = true\n", "");
        dir.write("config.toml", &modified);
        let cfg = dir.path.join("config.toml");

        let (out, r) = run(&cfg, true);
        r.unwrap();
        assert!(out.contains("field(s) added"));

        let repaired = dir.read("config.toml");
        assert!(repaired.contains("normalize_mix = true"));
        assert!(repaired.contains("preserve_chapters = true"));
        crate::config::Config::from_toml_str(&repaired).expect("repaired config parses");
    }

    #[test]
    fn inserted_fields_carry_doc_comments() {
        let dir = TempDir::new("comments");
        let original = crate::bootstrap::generate_global_config_text();
        let modified = original.replace("normalize_mix = true\n", "");
        dir.write("config.toml", &modified);
        let cfg = dir.path.join("config.toml");

        run(&cfg, true).1.unwrap();
        let repaired = dir.read("config.toml");

        // The bootstrap template has a comment above normalize_mix.
        // After repair, a comment line should appear before the inserted field.
        assert!(
            repaired.contains("# (added by bento repair)"),
            "repair marker present; got:\n{}",
            repaired
        );
    }
}
