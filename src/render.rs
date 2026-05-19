//! `bento config` — resolve and print config with full provenance.

use std::fs;
use std::io::Write;
use std::path::Path;

use crate::config::*;
use crate::error::{Error, Result};
use crate::layers::{discover_layers, sidecar_path};
use crate::resolve::{Layer, Resolved, resolve};
use crate::validate::{Severity, ValidationIssue, validate};

pub fn run_config(target: &Path, out: &mut dyn Write) -> Result<()> {
    if !target.exists() {
        return Err(Error::PathNotFound(target.to_path_buf()));
    }
    if target.is_dir() {
        return run_config_directory(target, out);
    }
    run_config_file(target, out)
}

fn run_config_file(target: &Path, out: &mut dyn Write) -> Result<()> {
    let layers = discover_layers(target, out)?;
    let resolved = resolve(layers.clone());
    let issues = validate(&resolved);
    render(target, &layers, &resolved, &issues, out).map_err(crate::io_render_err)?;

    let error_count = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .count();
    let warning_count = issues
        .iter()
        .filter(|i| i.severity == Severity::Warning)
        .count();
    if error_count > 0 {
        return Err(Error::ConfigInvalid {
            path: target.to_path_buf(),
            errors: error_count,
            warnings: warning_count,
        });
    }
    Ok(())
}

fn run_config_directory(target: &Path, out: &mut dyn Write) -> Result<()> {
    let mut files: Vec<std::path::PathBuf> = fs::read_dir(target)
        .map_err(|e| Error::Io {
            path: target.to_path_buf(),
            source: e,
        })?
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && crate::pipeline::is_video_extension(p))
        .collect();
    files.sort();

    if files.is_empty() {
        writeln!(out, "No video files found in {}.", target.display())
            .map_err(crate::io_render_err)?;
        return Ok(());
    }

    let mut total_errors = 0;
    for (idx, file) in files.iter().enumerate() {
        if idx > 0 {
            writeln!(out, "\n{}\n", "─".repeat(80)).map_err(crate::io_render_err)?;
        }
        let layers = discover_layers(file, out)?;
        let resolved = resolve(layers.clone());
        let issues = validate(&resolved);
        render(file, &layers, &resolved, &issues, out).map_err(crate::io_render_err)?;
        total_errors += issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .count();
    }

    if total_errors > 0 {
        return Err(Error::BatchFailed {
            count: total_errors,
        });
    }
    Ok(())
}

fn render(
    target: &Path,
    layers: &[(Layer, Config)],
    resolved: &Resolved,
    issues: &[ValidationIssue],
    out: &mut dyn Write,
) -> std::io::Result<()> {
    writeln!(out, "bento config for {}", target.display())?;
    writeln!(out)?;

    writeln!(out, "Layers loaded (lowest → highest precedence):")?;
    writeln!(out, "  {}", Layer::Defaults.display())?;
    for (layer, _) in layers {
        writeln!(out, "  {}", layer.display())?;
    }
    let sidecar = sidecar_path(target);
    if !layers.iter().any(|(l, _)| matches!(l, Layer::PerFile(_))) {
        writeln!(
            out,
            "  per-file   (none — would be at {})",
            sidecar.display()
        )?;
    }
    writeln!(out)?;

    writeln!(out, "Resolved settings:")?;
    let value =
        toml::Value::try_from(&resolved.config).expect("resolved config is always serializable");
    let mut leaves: Vec<(String, toml::Value)> = Vec::new();
    collect_leaves(&value, String::new(), &mut leaves);
    leaves.sort_by(|a, b| a.0.cmp(&b.0));

    let key_width = leaves.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    for (path, val) in &leaves {
        let layer_label = resolved
            .provenance
            .layer_for(path)
            .map(|l| l.kind())
            .unwrap_or("?");
        writeln!(
            out,
            "  {:<width$}  = {:<24}  [{}]",
            path,
            format_scalar(val),
            layer_label,
            width = key_width,
        )?;
    }
    writeln!(out)?;

    if let Some(tracks) = &resolved.config.audio.tracks {
        let layer = resolved
            .provenance
            .layer_for("audio.tracks")
            .map(|l| l.kind())
            .unwrap_or("?");
        writeln!(out, "audio.tracks ({}) [{}]:", tracks.len(), layer)?;
        for (i, t) in tracks.iter().enumerate() {
            writeln!(out, "  [{}] {}", i, format_audio_track(t))?;
        }
        writeln!(out)?;
    }
    if let Some(tracks) = &resolved.config.subtitles.tracks {
        let layer = resolved
            .provenance
            .layer_for("subtitles.tracks")
            .map(|l| l.kind())
            .unwrap_or("?");
        writeln!(out, "subtitles.tracks ({}) [{}]:", tracks.len(), layer)?;
        for (i, t) in tracks.iter().enumerate() {
            writeln!(out, "  [{}] {}", i, format_subtitle_track(t))?;
        }
        writeln!(out)?;
    }

    let errors: Vec<&ValidationIssue> = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .collect();
    let warnings: Vec<&ValidationIssue> = issues
        .iter()
        .filter(|i| i.severity == Severity::Warning)
        .collect();

    writeln!(out, "Validation:")?;
    if errors.is_empty() && warnings.is_empty() {
        writeln!(out, "  ok (0 errors, 0 warnings)")?;
    } else {
        if !errors.is_empty() {
            writeln!(out, "  {} error(s):", errors.len())?;
            for e in &errors {
                writeln!(out, "    [error]   {}: {}", e.path, e.message)?;
            }
        }
        if !warnings.is_empty() {
            writeln!(out, "  {} warning(s):", warnings.len())?;
            for w in &warnings {
                writeln!(out, "    [warning] {}: {}", w.path, w.message)?;
            }
        }
    }

    Ok(())
}

fn collect_leaves(value: &toml::Value, prefix: String, out: &mut Vec<(String, toml::Value)>) {
    match value {
        toml::Value::Table(t) => {
            for (k, v) in t {
                let path = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{}.{}", prefix, k)
                };
                collect_leaves(v, path, out);
            }
        }
        _ => out.push((prefix, value.clone())),
    }
}

fn format_scalar(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => format!("{:?}", s),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Datetime(d) => d.to_string(),
        toml::Value::Array(a) => format!("[{} items]", a.len()),
        toml::Value::Table(_) => "(table)".to_string(),
    }
}

fn format_track<T: serde::Serialize>(t: &T, field_order: &[&str]) -> String {
    let value = toml::Value::try_from(t).expect("track is serializable");
    let toml::Value::Table(table) = value else {
        return String::new();
    };
    let mut parts = Vec::new();
    for &key in field_order {
        if let Some(v) = table.get(key) {
            parts.push(format!("{}={}", key, format_scalar_inline(v)));
        }
    }
    parts.join(" ")
}

fn format_scalar_inline(v: &toml::Value) -> String {
    match v {
        toml::Value::Table(t) => {
            let inner: Vec<String> = t
                .iter()
                .map(|(k, vv)| format!("{}={}", k, format_scalar_inline(vv)))
                .collect();
            format!("{{{}}}", inner.join(", "))
        }
        _ => format_scalar(v),
    }
}

fn format_audio_track(t: &AudioTrack) -> String {
    const ORDER: &[&str] = &[
        "source",
        "lang",
        "title",
        "default",
        "forced",
        "original",
        "commentary",
        "hearing_impaired",
        "visual_impaired",
        "encoder",
        "bitrate",
        "mixdown",
        "force_bitrate",
        "force_mixdown",
    ];
    format_track(t, ORDER)
}

fn format_subtitle_track(t: &SubtitleTrack) -> String {
    const ORDER: &[&str] = &[
        "source",
        "format",
        "mux",
        "subtract_track",
        "filter",
        "lang",
        "title",
        "default",
        "forced",
        "commentary",
        "hearing_impaired",
    ];
    format_track(t, ORDER)
}
