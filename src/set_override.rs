//! Parsing and application of `--set KEY=VALUE` dotted-path CLI overrides.
//!
//! Each `--set` argument has the form `dotted.path=TOML_scalar` where the value
//! is a TOML scalar (bool, integer, or quoted string — tables and arrays are
//! rejected). Unknown paths and type mismatches are caught by the `Config`
//! schema's `deny_unknown_fields` and serde type coercions.

use crate::config::Config;
use crate::error::{Error, Result};

/// Build a [`Config`] from a list of `KEY=VALUE` override strings.
///
/// Returns a `Config` with only the overridden fields set (all others remain
/// `None`), suitable for insertion as the `Layer::Cli` entry in the resolution
/// stack alongside other CLI overrides.
pub fn build_set_config(set_args: &[String]) -> Result<Config> {
    if set_args.is_empty() {
        return Ok(Config::default());
    }

    let mut root: toml::map::Map<String, toml::Value> = toml::map::Map::new();

    for s in set_args {
        let (key, value) = parse_one(s)?;
        insert_dotted(&mut root, &key, value, s)?;
    }

    let toml_str = toml::to_string(&toml::Value::Table(root))
        .expect("freshly built TOML map always serializes");
    Config::from_toml_str(&toml_str).map_err(|e| Error::SetOverrideSchema {
        reason: e.to_string(),
    })
}

fn parse_one(s: &str) -> Result<(String, toml::Value)> {
    let eq_pos = s.find('=').ok_or_else(|| Error::SetOverrideSyntax {
        input: s.to_string(),
        reason: "missing `=`; expected `KEY=VALUE` (e.g. `video.encoder.crf=22`)".to_string(),
    })?;

    let key = s[..eq_pos].trim();
    let value_str = &s[eq_pos + 1..];

    if key.is_empty() {
        return Err(Error::SetOverrideSyntax {
            input: s.to_string(),
            reason: "key is empty; expected a dotted config path before `=`".to_string(),
        });
    }

    for segment in key.split('.') {
        if segment.is_empty() {
            return Err(Error::SetOverrideSyntax {
                input: s.to_string(),
                reason: format!(
                    "key `{}` contains an empty segment (consecutive or trailing dots?)",
                    key
                ),
            });
        }
    }

    // Reject list paths with a targeted error before attempting TOML parse.
    if key == "audio.tracks"
        || key.starts_with("audio.tracks.")
        || key == "subtitles.tracks"
        || key.starts_with("subtitles.tracks.")
    {
        return Err(Error::SetOverrideListPath {
            key: key.to_string(),
        });
    }

    // Parse value_str as a TOML value via the `sentinel = <value>` trick.
    let toml_snippet = format!("sentinel = {}", value_str);
    let table: toml::Value = toml::from_str(&toml_snippet).map_err(|_| {
        let hint = quoting_hint(value_str);
        Error::SetOverrideValue {
            key: key.to_string(),
            value: value_str.to_string(),
            hint: hint.to_string(),
        }
    })?;

    let value = match table {
        toml::Value::Table(mut t) => t.remove("sentinel").expect("sentinel key always present"),
        _ => unreachable!("toml::from_str on a key=value string always yields a Table"),
    };

    if value.is_table() || value.is_array() {
        return Err(Error::SetOverrideNotScalar {
            key: key.to_string(),
            value: value_str.to_string(),
        });
    }

    Ok((key.to_string(), value))
}

/// When a value fails to parse as TOML, guess whether it looks like a bare
/// string that should be quoted and return a hint if so.
fn quoting_hint(value_str: &str) -> &'static str {
    if !value_str.is_empty()
        && !value_str.starts_with('"')
        && !value_str.starts_with('\'')
        && !value_str.starts_with('[')
        && !value_str.starts_with('{')
        && value_str.parse::<i64>().is_err()
        && value_str.parse::<f64>().is_err()
        && value_str != "true"
        && value_str != "false"
    {
        "\n  hint: bare strings are not valid TOML; quote the value: --set KEY=\"value\""
    } else {
        ""
    }
}

/// Navigate into `table` along the dotted segments of `key`, creating
/// intermediate tables as needed, and set the leaf to `value`.
fn insert_dotted(
    table: &mut toml::map::Map<String, toml::Value>,
    key: &str,
    value: toml::Value,
    raw: &str,
) -> Result<()> {
    let segments: Vec<&str> = key.split('.').collect();
    let mut current = table;

    for segment in &segments[..segments.len() - 1] {
        let entry = current
            .entry((*segment).to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        match entry {
            toml::Value::Table(t) => current = t,
            _ => {
                return Err(Error::SetOverrideSyntax {
                    input: raw.to_string(),
                    reason: format!(
                        "key `{}` conflicts with a previous `--set` that set a \
                         non-table value at segment `{}`",
                        key, segment
                    ),
                });
            }
        }
    }

    let last = segments.last().expect("at least one segment");
    current.insert((*last).to_string(), value);
    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Container, EncoderName, OnExisting};

    fn s(lit: &str) -> String {
        lit.to_string()
    }

    #[test]
    fn integer_scalar() {
        let cfg = build_set_config(&[s("video.encoder.crf=22")]).unwrap();
        assert_eq!(cfg.video.encoder.unwrap().crf, Some(22));
    }

    #[test]
    fn bool_scalar() {
        let cfg = build_set_config(&[s("video.never_upscale=false")]).unwrap();
        assert_eq!(cfg.video.never_upscale, Some(false));
    }

    #[test]
    fn string_scalar() {
        let cfg = build_set_config(&[s(r#"output.container="mkv""#)]).unwrap();
        assert_eq!(cfg.output.container, Some(Container::Mkv));
    }

    #[test]
    fn multiple_overrides_same_subtable() {
        let cfg = build_set_config(&[s("video.encoder.crf=18"), s(r#"video.encoder.name="x265""#)])
            .unwrap();
        let enc = cfg.video.encoder.unwrap();
        assert_eq!(enc.crf, Some(18));
        assert_eq!(enc.name, Some(EncoderName::X265));
    }

    #[test]
    fn top_level_string_field() {
        let cfg = build_set_config(&[s(r#"output.on_existing="overwrite""#)]).unwrap();
        assert_eq!(cfg.output.on_existing, Some(OnExisting::Overwrite));
    }

    #[test]
    fn warn_flag_reenablement() {
        // The design doc calls this out as a key --set use case.
        let cfg = build_set_config(&[s("audio.warn_no_default=true")]).unwrap();
        assert_eq!(cfg.audio.warn_no_default, Some(true));
    }

    #[test]
    fn metadata_integer_field() {
        let cfg = build_set_config(&[s("output.metadata.season=2")]).unwrap();
        assert_eq!(cfg.output.metadata.unwrap().season, Some(2));
    }

    #[test]
    fn empty_input_returns_default() {
        let cfg = build_set_config(&[]).unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn error_no_equals() {
        let err = build_set_config(&[s("video.encoder.crf")]).unwrap_err();
        assert!(
            matches!(err, Error::SetOverrideSyntax { .. }),
            "expected SetOverrideSyntax, got: {}",
            err
        );
    }

    #[test]
    fn error_empty_key() {
        let err = build_set_config(&[s("=22")]).unwrap_err();
        assert!(
            matches!(err, Error::SetOverrideSyntax { .. }),
            "expected SetOverrideSyntax, got: {}",
            err
        );
    }

    #[test]
    fn error_consecutive_dots() {
        let err = build_set_config(&[s("video..crf=22")]).unwrap_err();
        assert!(
            matches!(err, Error::SetOverrideSyntax { .. }),
            "expected SetOverrideSyntax, got: {}",
            err
        );
    }

    #[test]
    fn error_list_path_audio_tracks() {
        let err = build_set_config(&[s("audio.tracks=[]")]).unwrap_err();
        assert!(
            matches!(err, Error::SetOverrideListPath { .. }),
            "expected SetOverrideListPath, got: {}",
            err
        );
    }

    #[test]
    fn error_list_path_subtitles_tracks() {
        let err = build_set_config(&[s("subtitles.tracks=[]")]).unwrap_err();
        assert!(
            matches!(err, Error::SetOverrideListPath { .. }),
            "expected SetOverrideListPath, got: {}",
            err
        );
    }

    #[test]
    fn error_bare_string_has_quoting_hint() {
        let err = build_set_config(&[s("video.encoder.name=animation")]).unwrap_err();
        assert!(
            matches!(err, Error::SetOverrideValue { .. }),
            "expected SetOverrideValue, got: {}",
            err
        );
        let msg = err.to_string();
        assert!(msg.contains("animation"), "value in message: {}", msg);
        assert!(msg.contains("hint"), "quoting hint in message: {}", msg);
    }

    #[test]
    fn error_table_value_rejected() {
        let err = build_set_config(&[s(r#"video.encoder={"name"="x265"}"#)]).unwrap_err();
        assert!(
            matches!(err, Error::SetOverrideNotScalar { .. }),
            "expected SetOverrideNotScalar, got: {}",
            err
        );
    }

    #[test]
    fn error_unknown_field_caught_by_schema() {
        let err = build_set_config(&[s("video.unknown_field=42")]).unwrap_err();
        assert!(
            matches!(err, Error::SetOverrideSchema { .. }),
            "expected SetOverrideSchema, got: {}",
            err
        );
    }

    #[test]
    fn error_wrong_type_caught_by_schema() {
        // `crf` expects an integer, not a bool.
        let err = build_set_config(&[s("video.encoder.crf=true")]).unwrap_err();
        assert!(
            matches!(err, Error::SetOverrideSchema { .. }),
            "expected SetOverrideSchema, got: {}",
            err
        );
    }

    #[test]
    fn audio_bitrate_integer() {
        let cfg = build_set_config(&[s("audio.bitrate=128")]).unwrap();
        assert_eq!(cfg.audio.bitrate, Some(128));
    }

    #[test]
    fn value_with_equals_sign_not_split_mid_value() {
        // Ensure the key is split on the *first* `=` only; a quoted string
        // value that happens to contain `=` must be parsed correctly.
        // Note: TOML double-quoted strings don't support `\d` as an escape;
        // use a value that is valid TOML and happens to contain a literal `=`.
        let cfg = build_set_config(&[s(r#"output.destination="path=with=equals""#)]).unwrap();
        assert_eq!(cfg.output.destination.as_deref(), Some("path=with=equals"));
    }
}
