//! Output filename stem computation from `[output].naming` config.
//!
//! Called once per file before the encode. Returns the stem (no extension) to
//! use for the output file, plus an optional episode number to embed as
//! container metadata (`tves` for MP4, `PART_NUMBER` for MKV).

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::config::Config;
use crate::error::{Error, Result};

static TEMPLATE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{([A-Za-z_][A-Za-z0-9_]*)(?::([^}]*))?\}").unwrap());

/// Compute the output filename stem and, if the naming regex contains a capture
/// named `episode` or `ep`, the episode number for container metadata embedding.
///
/// Returns `(stem, episode)` where `stem` is the output filename without
/// extension. When `[output].naming` is not configured (or has no `template`),
/// returns the source file's own stem unchanged and `None` for episode.
pub fn compute_output_stem(input: &Path, config: &Config) -> Result<(String, Option<i64>)> {
    let source_basename = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    let Some(naming) = &config.output.naming else {
        return Ok((source_basename, None));
    };
    let Some(template) = &naming.template else {
        return Ok((source_basename, None));
    };

    let mut vars: HashMap<String, TemplateVar> = HashMap::new();

    // Always-available built-in variables.
    vars.insert(
        "source_basename".into(),
        TemplateVar::Str(source_basename.clone()),
    );
    if let Some(dir_name) = input
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
    {
        vars.insert("source_dir".into(), TemplateVar::Str(dir_name));
    }

    // Metadata fields from [output.metadata].
    if let Some(meta) = &config.output.metadata {
        if let Some(show) = &meta.show {
            vars.insert("show".into(), TemplateVar::Str(show.clone()));
        }
        if let Some(season) = meta.season {
            vars.insert("season".into(), TemplateVar::Int(season as i64));
        }
        if let Some(year) = meta.year {
            vars.insert("year".into(), TemplateVar::Int(year as i64));
        }
    }

    // Named captures from [output.naming.regex], if set.
    if let Some(pattern) = &naming.regex {
        let re = Regex::new(pattern).map_err(|e| Error::NamingRegexInvalid {
            pattern: pattern.clone(),
            reason: e.to_string(),
        })?;
        let caps = re
            .captures(&source_basename)
            .ok_or_else(|| Error::NamingRegexNoMatch {
                pattern: pattern.clone(),
                filename: source_basename.clone(),
            })?;
        for name in re.capture_names().flatten() {
            if let Some(m) = caps.name(name) {
                let s = m.as_str().to_string();
                let var = if let Ok(n) = s.parse::<i64>() {
                    TemplateVar::Int(n)
                } else {
                    TemplateVar::Str(s)
                };
                vars.insert(name.to_string(), var);
            }
        }
    }

    // "episode" wins over "ep" per DESIGN.md.
    let episode = vars
        .get("episode")
        .and_then(int_value)
        .or_else(|| vars.get("ep").and_then(int_value));

    let stem = expand_template(template, &vars)?;
    Ok((stem, episode))
}

// =============================================================================
// Template expansion
// =============================================================================

enum TemplateVar {
    Str(String),
    Int(i64),
}

fn int_value(v: &TemplateVar) -> Option<i64> {
    if let TemplateVar::Int(n) = v {
        Some(*n)
    } else {
        None
    }
}

fn expand_template(template: &str, vars: &HashMap<String, TemplateVar>) -> Result<String> {
    let mut result = String::with_capacity(template.len() + 16);
    let mut last_end = 0;

    for cap in TEMPLATE_RE.captures_iter(template) {
        let whole = cap.get(0).unwrap();
        result.push_str(&template[last_end..whole.start()]);
        last_end = whole.end();

        let var_name = &cap[1];
        let fmt_spec = cap.get(2).map(|m| m.as_str());

        let var = vars
            .get(var_name)
            .ok_or_else(|| Error::NamingUndefinedVar {
                var: var_name.to_string(),
            })?;
        result.push_str(&render_var(var, var_name, fmt_spec)?);
    }

    result.push_str(&template[last_end..]);
    Ok(result)
}

fn render_var(var: &TemplateVar, name: &str, spec: Option<&str>) -> Result<String> {
    let Some(spec) = spec else {
        return Ok(match var {
            TemplateVar::Str(s) => s.clone(),
            TemplateVar::Int(n) => n.to_string(),
        });
    };

    // Format specs require an integer value.
    let n = match var {
        TemplateVar::Int(n) => *n,
        TemplateVar::Str(_) => {
            return Err(Error::NamingFormatOnString {
                var: name.to_string(),
                spec: spec.to_string(),
            });
        }
    };

    // Recognized spec: "0<width>" → zero-padded to <width> digits.
    if let Some(width_str) = spec.strip_prefix('0') {
        if let Ok(width) = width_str.parse::<usize>() {
            if width > 0 {
                return Ok(format!("{n:0width$}"));
            }
        }
    }

    Err(Error::NamingUnknownFormatSpec {
        var: name.to_string(),
        spec: spec.to_string(),
    })
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Metadata, Naming, Output};
    use std::path::PathBuf;

    fn make_config(naming: Option<Naming>, meta: Option<Metadata>) -> Config {
        Config {
            output: Output {
                naming,
                metadata: meta,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    // Helper: input path with just a filename (no real directory).
    fn inp(filename: &str) -> PathBuf {
        PathBuf::from(filename)
    }

    #[test]
    fn no_naming_config_returns_source_stem() {
        let cfg = make_config(None, None);
        let (stem, ep) = compute_output_stem(&inp("episode01.mkv"), &cfg).unwrap();
        assert_eq!(stem, "episode01");
        assert_eq!(ep, None);
    }

    #[test]
    fn template_only_no_regex_expands_metadata() {
        let cfg = make_config(
            Some(Naming {
                regex: None,
                template: Some("{show} - ep{source_basename}".into()),
            }),
            Some(Metadata {
                show: Some("Bebop".into()),
                season: None,
                year: None,
            }),
        );
        let (stem, ep) = compute_output_stem(&inp("01.mkv"), &cfg).unwrap();
        assert_eq!(stem, "Bebop - ep01");
        assert_eq!(ep, None);
    }

    #[test]
    fn design_doc_cowboy_bebop_example() {
        // From DESIGN.md §[output] example:
        // regex = 'S(?P<s>\d+)E(?P<episode>\d+)'
        // template = "{show} - S{s:02}E{episode:02}"
        // source: "Cowboy Bebop S01E06 [BD 1080p].mkv"
        // expected stem: "Cowboy Bebop - S01E06"
        let cfg = make_config(
            Some(Naming {
                regex: Some(r"S(?P<s>\d+)E(?P<episode>\d+)".into()),
                template: Some("{show} - S{s:02}E{episode:02}".into()),
            }),
            Some(Metadata {
                show: Some("Cowboy Bebop".into()),
                season: Some(1),
                year: Some(1998),
            }),
        );
        let (stem, ep) =
            compute_output_stem(&inp("Cowboy Bebop S01E06 [BD 1080p].mkv"), &cfg).unwrap();
        assert_eq!(stem, "Cowboy Bebop - S01E06");
        assert_eq!(ep, Some(6));
    }

    #[test]
    fn episode_capture_sets_episode_number() {
        let cfg = make_config(
            Some(Naming {
                regex: Some(r"E(?P<episode>\d+)".into()),
                template: Some("out-{episode}".into()),
            }),
            None,
        );
        let (stem, ep) = compute_output_stem(&inp("show_E12.mkv"), &cfg).unwrap();
        assert_eq!(stem, "out-12");
        assert_eq!(ep, Some(12));
    }

    #[test]
    fn ep_capture_used_when_no_episode() {
        let cfg = make_config(
            Some(Naming {
                regex: Some(r"E(?P<ep>\d+)".into()),
                template: Some("out-{ep}".into()),
            }),
            None,
        );
        let (_, ep) = compute_output_stem(&inp("show_E05.mkv"), &cfg).unwrap();
        assert_eq!(ep, Some(5));
    }

    #[test]
    fn episode_wins_over_ep() {
        let cfg = make_config(
            Some(Naming {
                regex: Some(r"E(?P<ep>\d+)x(?P<episode>\d+)".into()),
                template: Some("{episode}".into()),
            }),
            None,
        );
        let (_, ep) = compute_output_stem(&inp("show_E99x03.mkv"), &cfg).unwrap();
        assert_eq!(ep, Some(3)); // episode=3 wins over ep=99
    }

    #[test]
    fn zero_pad_format_spec() {
        let cfg = make_config(
            Some(Naming {
                regex: Some(r"E(?P<ep>\d+)".into()),
                template: Some("ep{ep:03}".into()),
            }),
            None,
        );
        let (stem, _) = compute_output_stem(&inp("show_E7.mkv"), &cfg).unwrap();
        assert_eq!(stem, "ep007");
    }

    #[test]
    fn regex_no_match_is_error() {
        let cfg = make_config(
            Some(Naming {
                regex: Some(r"S\d+E\d+".into()),
                template: Some("out".into()),
            }),
            None,
        );
        let err = compute_output_stem(&inp("random_name.mkv"), &cfg).unwrap_err();
        assert!(matches!(err, Error::NamingRegexNoMatch { .. }), "{err}");
    }

    #[test]
    fn undefined_variable_is_error() {
        let cfg = make_config(
            Some(Naming {
                regex: None,
                template: Some("{nonexistent}".into()),
            }),
            None,
        );
        let err = compute_output_stem(&inp("file.mkv"), &cfg).unwrap_err();
        assert!(matches!(err, Error::NamingUndefinedVar { .. }), "{err}");
    }

    #[test]
    fn format_spec_on_string_is_error() {
        let cfg = make_config(
            Some(Naming {
                regex: None,
                template: Some("{show:02}".into()),
            }),
            Some(Metadata {
                show: Some("Bebop".into()),
                season: None,
                year: None,
            }),
        );
        let err = compute_output_stem(&inp("file.mkv"), &cfg).unwrap_err();
        assert!(matches!(err, Error::NamingFormatOnString { .. }), "{err}");
    }

    #[test]
    fn source_basename_and_source_dir_available() {
        let cfg = make_config(
            Some(Naming {
                regex: None,
                template: Some("{source_dir}/{source_basename}".into()),
            }),
            None,
        );
        let input = PathBuf::from("/anime/Bebop/S01E06.mkv");
        let (stem, _) = compute_output_stem(&input, &cfg).unwrap();
        assert_eq!(stem, "Bebop/S01E06");
    }

    #[test]
    fn season_as_integer_zero_padded() {
        let cfg = make_config(
            Some(Naming {
                regex: None,
                template: Some("S{season:02}".into()),
            }),
            Some(Metadata {
                show: None,
                season: Some(2),
                year: None,
            }),
        );
        let (stem, _) = compute_output_stem(&inp("ep.mkv"), &cfg).unwrap();
        assert_eq!(stem, "S02");
    }
}
