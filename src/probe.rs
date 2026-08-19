//! `bento probe` — inspect a video file's streams in Bento-native terms.

use std::io::Write;
use std::path::Path;

use console::style;

use crate::error::{Error, Result};
use crate::pipeline::probe::{
    AudioStreamInfo, Dispositions, SourceProbe, SubtitleStreamInfo, probe_source_streams,
};

pub fn run_probe(path: &Path, verbose: bool, out: &mut dyn Write) -> Result<()> {
    if !path.exists() {
        return Err(Error::PathNotFound(path.to_path_buf()));
    }
    if path.is_dir() {
        return Err(Error::PathIsDirectory(path.to_path_buf()));
    }
    let probe = probe_source_streams(path)?;
    render(&probe, path, verbose, out).map_err(crate::io_render_err)
}

fn render(
    probe: &SourceProbe,
    path: &Path,
    verbose: bool,
    out: &mut dyn Write,
) -> std::io::Result<()> {
    let filename = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();

    // ── Header ───────────────────────────────────────────────────────────────
    writeln!(out)?;
    if let Some(secs) = probe.duration_secs {
        writeln!(
            out,
            "  {}   {}",
            style(&*filename).bold(),
            style(format_duration(secs)).dim()
        )?;
    } else {
        writeln!(out, "  {}", style(&*filename).bold())?;
    }
    writeln!(out)?;

    // ── Video ────────────────────────────────────────────────────────────────
    let v = &probe.video;
    if !v.codec.is_empty() || v.width > 0 {
        writeln!(out, "  {}", style("Video").red().bold())?;
        write!(out, "    {}", friendly_video_codec(&v.codec))?;
        if v.width > 0 && v.height > 0 {
            write!(
                out,
                "   {}",
                style(format!("{} × {}", v.width, v.height)).dim()
            )?;
        }
        if let Some(ref fps_str) = v.r_frame_rate {
            write!(
                out,
                "   {}",
                style(format!("{} fps", format_framerate(fps_str))).dim()
            )?;
        }
        writeln!(out)?;
        writeln!(out)?;
    }

    // ── Audio ────────────────────────────────────────────────────────────────
    let audio_flag_w;
    if !probe.audio.is_empty() {
        let count = probe.audio.len();
        writeln!(
            out,
            "  {}   {}",
            style("Audio").green().bold(),
            style(format!("{count} {}", plural(count))).dim()
        )?;

        let lang_w = lang_col_width(probe.audio.iter().map(|a| a.language.as_deref()));
        let codec_w = probe
            .audio
            .iter()
            .map(|a| friendly_audio_codec(&a.codec).len())
            .max()
            .unwrap_or(0);
        let audio_flags: Vec<String> = probe
            .audio
            .iter()
            .map(|a| audio_flag_str(&a.dispositions, verbose))
            .collect();
        audio_flag_w = audio_flags.iter().map(|s| s.len()).max().unwrap_or(0);
        let layout_w = probe
            .audio
            .iter()
            .map(|a| format_audio_layout(a.channels, a.channel_layout.as_deref()).len())
            .max()
            .unwrap_or(0);

        for (idx, (track, flags)) in probe.audio.iter().zip(audio_flags.iter()).enumerate() {
            write_audio_row(
                out,
                idx + 1,
                track,
                flags,
                lang_w,
                codec_w,
                audio_flag_w,
                layout_w,
                verbose,
            )?;
        }
        writeln!(out)?;
    } else {
        audio_flag_w = 0;
    }

    // ── Subtitles ────────────────────────────────────────────────────────────
    let sub_flag_w;
    if !probe.subtitles.is_empty() {
        let count = probe.subtitles.len();
        writeln!(
            out,
            "  {}   {}",
            style("Subtitles").blue().bold(),
            style(format!("{count} {}", plural(count))).dim()
        )?;

        let lang_w = lang_col_width(probe.subtitles.iter().map(|s| s.language.as_deref()));
        let codec_w = probe
            .subtitles
            .iter()
            .map(|s| friendly_sub_codec(&s.codec).len())
            .max()
            .unwrap_or(0);
        let sub_flags: Vec<String> = probe
            .subtitles
            .iter()
            .map(|s| sub_flag_str(&s.dispositions, verbose))
            .collect();
        sub_flag_w = sub_flags.iter().map(|s| s.len()).max().unwrap_or(0);

        for (idx, (track, flags)) in probe.subtitles.iter().zip(sub_flags.iter()).enumerate() {
            write_subtitle_row(out, idx + 1, track, flags, lang_w, codec_w, sub_flag_w)?;
        }
        writeln!(out)?;
    } else {
        sub_flag_w = 0;
    }

    // ── Footer ───────────────────────────────────────────────────────────────
    if !probe.audio.is_empty() || !probe.subtitles.is_empty() {
        writeln!(
            out,
            "  {}",
            style("(Track numbers correspond to source = N in your bento.toml.)").dim()
        )?;
        // Show the flag legend in compact mode when at least one flag is set.
        if !verbose && (audio_flag_w > 0 || sub_flag_w > 0) {
            writeln!(
                out,
                "  {}",
                style(
                    "(Flags: D=default  F=forced  O=original  C=commentary  \
                     HI=hearing-impaired  VI=visual-impaired)"
                )
                .dim()
            )?;
        }
        writeln!(out)?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_audio_row(
    out: &mut dyn Write,
    num: usize,
    track: &AudioStreamInfo,
    flag_str: &str,
    lang_w: usize,
    codec_w: usize,
    flag_w: usize,
    layout_w: usize,
    verbose: bool,
) -> std::io::Result<()> {
    // Track number — the key value users copy into source =
    write!(out, "    {}", style(format!("{num:>2}")).magenta().bold())?;

    // Language code, dimmed when absent
    let lang = track.language.as_deref().unwrap_or("---");
    if track.language.is_some() {
        write!(out, "   {lang:<lang_w$}")?;
    } else {
        write!(out, "   {}", style(format!("{lang:<lang_w$}")).dim())?;
    }

    // Friendly codec name
    let codec = friendly_audio_codec(&track.codec);
    write!(out, "   {codec:<codec_w$}")?;

    // Disposition flags — only when at least one track in this section has flags
    if flag_w > 0 {
        write!(out, "   {flag_str:<flag_w$}")?;
    }

    // Channel layout — technical detail, dimmed; padded so bitrate column aligns
    let layout = format!(
        "{:<layout_w$}",
        format_audio_layout(track.channels, track.channel_layout.as_deref())
    );
    write!(out, "   {}", style(&layout).dim())?;

    // Bitrate — optional, dimmed
    if let Some(kbps) = track.bitrate_kbps {
        write!(out, "   {}", style(format!("{kbps} kbps")).dim())?;
    }

    // Sample rate — verbose only
    if verbose {
        if let Some(hz) = track.sample_rate {
            write!(out, "   {}", style(format!("{hz} Hz")).dim())?;
        }
    }

    // Human-readable title — the most useful identification info
    if let Some(ref title) = track.title {
        write!(out, "   \"{title}\"")?;
    }

    writeln!(out)
}

fn write_subtitle_row(
    out: &mut dyn Write,
    num: usize,
    track: &SubtitleStreamInfo,
    flag_str: &str,
    lang_w: usize,
    codec_w: usize,
    flag_w: usize,
) -> std::io::Result<()> {
    write!(out, "    {}", style(format!("{num:>2}")).magenta().bold())?;

    let lang = track.language.as_deref().unwrap_or("---");
    if track.language.is_some() {
        write!(out, "   {lang:<lang_w$}")?;
    } else {
        write!(out, "   {}", style(format!("{lang:<lang_w$}")).dim())?;
    }

    let codec = friendly_sub_codec(&track.codec);
    write!(out, "   {codec:<codec_w$}")?;

    // Disposition flags — only when at least one track in this section has flags
    if flag_w > 0 {
        write!(out, "   {flag_str:<flag_w$}")?;
    }

    if let Some(ref title) = track.title {
        write!(out, "   \"{title}\"")?;
    }

    writeln!(out)
}

// ── Disposition flag strings ──────────────────────────────────────────────────

/// Compact or verbose flag string for an audio track.
///
/// Audio exposes all six disposition types; the caller is responsible for not
/// calling this on subtitle rows (which omit `O` and `VI`).
fn audio_flag_str(d: &Dispositions, verbose: bool) -> String {
    if verbose {
        let mut names: Vec<&str> = Vec::new();
        if d.default {
            names.push("default");
        }
        if d.forced {
            names.push("forced");
        }
        if d.original {
            names.push("original");
        }
        if d.commentary {
            names.push("commentary");
        }
        if d.hearing_impaired {
            names.push("hearing-impaired");
        }
        if d.visual_impaired {
            names.push("visual-impaired");
        }
        names.join(", ")
    } else {
        let mut flags: Vec<&str> = Vec::new();
        if d.default {
            flags.push("D");
        }
        if d.forced {
            flags.push("F");
        }
        if d.original {
            flags.push("O");
        }
        if d.commentary {
            flags.push("C");
        }
        if d.hearing_impaired {
            flags.push("HI");
        }
        if d.visual_impaired {
            flags.push("VI");
        }
        flags.join(",")
    }
}

/// Compact or verbose flag string for a subtitle track.
///
/// Subtitles omit `O` (original, audio-only) and `VI` (visual-impaired, audio-only).
fn sub_flag_str(d: &Dispositions, verbose: bool) -> String {
    if verbose {
        let mut names: Vec<&str> = Vec::new();
        if d.default {
            names.push("default");
        }
        if d.forced {
            names.push("forced");
        }
        if d.commentary {
            names.push("commentary");
        }
        if d.hearing_impaired {
            names.push("hearing-impaired");
        }
        names.join(", ")
    } else {
        let mut flags: Vec<&str> = Vec::new();
        if d.default {
            flags.push("D");
        }
        if d.forced {
            flags.push("F");
        }
        if d.commentary {
            flags.push("C");
        }
        if d.hearing_impaired {
            flags.push("HI");
        }
        flags.join(",")
    }
}

// ── Column helpers ────────────────────────────────────────────────────────────

fn lang_col_width<'a>(langs: impl Iterator<Item = Option<&'a str>>) -> usize {
    langs
        .map(|l| l.unwrap_or("---").len())
        .max()
        .unwrap_or(3)
        .max(3)
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "track" } else { "tracks" }
}

// ── Formatters ────────────────────────────────────────────────────────────────

fn format_duration(secs: f64) -> String {
    let total = secs as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn format_framerate(r: &str) -> String {
    let Some((num_s, den_s)) = r.split_once('/') else {
        return r.to_string();
    };
    let (Ok(num), Ok(den)) = (num_s.trim().parse::<f64>(), den_s.trim().parse::<f64>()) else {
        return r.to_string();
    };
    if den == 0.0 {
        return r.to_string();
    }
    let fps = num / den;
    // 3 decimal places, trailing zeros trimmed: 25.000 → "25", 23.976239... → "23.976"
    let s = format!("{fps:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn format_audio_layout(channels: u32, layout: Option<&str>) -> String {
    let name = layout
        .and_then(|l| match l {
            "stereo" => Some("stereo"),
            "mono" => Some("mono"),
            s if s.starts_with("5.1") => Some("5.1"),
            s if s.starts_with("7.1") => Some("7.1"),
            s if s.starts_with("6.1") => Some("6.1"),
            s if s.starts_with("7.0") => Some("7.0"),
            "quad" => Some("4.0"),
            _ => None,
        })
        .or(match channels {
            1 => Some("mono"),
            2 => Some("stereo"),
            6 => Some("5.1"),
            8 => Some("7.1"),
            _ => None,
        });

    match name {
        // mono/stereo are self-describing; no need to add the channel count
        Some("mono") | Some("stereo") => name.unwrap().to_string(),
        Some(n) => format!("{n} ({channels} ch)"),
        None => format!("{channels} ch"),
    }
}

// ── Codec friendly names ──────────────────────────────────────────────────────

fn friendly_video_codec(codec: &str) -> &str {
    match codec {
        "h264" => "H.264",
        "hevc" => "H.265 (HEVC)",
        "av1" => "AV1",
        "vp9" => "VP9",
        "vp8" => "VP8",
        "mpeg2video" => "MPEG-2",
        "mpeg4" => "MPEG-4",
        "theora" => "Theora",
        "prores" => "ProRes",
        "dnxhd" => "DNxHD",
        other => other,
    }
}

fn friendly_audio_codec(codec: &str) -> &str {
    match codec {
        "aac" => "AAC",
        "ac3" => "AC3",
        "eac3" => "E-AC3",
        "dts" => "DTS",
        "truehd" | "mlp" => "TrueHD",
        "flac" => "FLAC",
        "mp3" => "MP3",
        "opus" => "Opus",
        "vorbis" => "Vorbis",
        "alac" => "ALAC",
        "pcm_s16le" | "pcm_s16be" => "PCM 16-bit",
        "pcm_s24le" | "pcm_s24be" => "PCM 24-bit",
        "pcm_s32le" | "pcm_s32be" => "PCM 32-bit",
        other => other,
    }
}

fn friendly_sub_codec(codec: &str) -> &str {
    match codec {
        "ass" | "ssa" => "ASS",
        "subrip" | "srt" => "SRT",
        "mov_text" => "Timed Text",
        "hdmv_pgs_subtitle" => "PGS",
        "dvd_subtitle" => "DVD Bitmap",
        "webvtt" => "WebVTT",
        "microdvd" => "MicroDVD",
        other => other,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::probe::{
        AudioStreamInfo, SourceProbe, SubtitleStreamInfo, VideoStreamInfo,
    };

    #[test]
    fn format_duration_under_one_hour() {
        assert_eq!(format_duration(0.0), "0:00");
        assert_eq!(format_duration(59.0), "0:59");
        assert_eq!(format_duration(60.0), "1:00");
        assert_eq!(format_duration(2537.0), "42:17");
    }

    #[test]
    fn format_duration_over_one_hour() {
        assert_eq!(format_duration(3600.0), "1:00:00");
        assert_eq!(format_duration(3661.0), "1:01:01");
        assert_eq!(format_duration(7384.0), "2:03:04");
    }

    #[test]
    fn format_framerate_common_values() {
        assert_eq!(format_framerate("24000/1001"), "23.976");
        assert_eq!(format_framerate("30000/1001"), "29.97");
        assert_eq!(format_framerate("25/1"), "25");
        assert_eq!(format_framerate("30/1"), "30");
        assert_eq!(format_framerate("60/1"), "60");
        assert_eq!(format_framerate("24/1"), "24");
    }

    #[test]
    fn format_framerate_bad_input_passthrough() {
        assert_eq!(format_framerate("0/0"), "0/0");
        assert_eq!(format_framerate("notarate"), "notarate");
    }

    #[test]
    fn format_audio_layout_named() {
        assert_eq!(format_audio_layout(2, Some("stereo")), "stereo");
        assert_eq!(format_audio_layout(1, Some("mono")), "mono");
        assert_eq!(format_audio_layout(6, Some("5.1")), "5.1 (6 ch)");
        assert_eq!(format_audio_layout(6, Some("5.1(side)")), "5.1 (6 ch)");
        assert_eq!(format_audio_layout(8, Some("7.1")), "7.1 (8 ch)");
    }

    #[test]
    fn format_audio_layout_derived_from_channels() {
        assert_eq!(format_audio_layout(2, None), "stereo");
        assert_eq!(format_audio_layout(6, None), "5.1 (6 ch)");
        assert_eq!(format_audio_layout(4, None), "4 ch");
    }

    #[test]
    fn render_produces_expected_sections() {
        let probe = SourceProbe {
            video: VideoStreamInfo {
                codec: "h264".to_string(),
                width: 1920,
                height: 1080,
                r_frame_rate: Some("24000/1001".to_string()),
                ..Default::default()
            },
            audio: vec![
                AudioStreamInfo {
                    codec: "dts".to_string(),
                    channels: 6,
                    channel_layout: Some("5.1".to_string()),
                    bitrate_kbps: Some(1509),
                    language: Some("eng".to_string()),
                    title: Some("English DTS".to_string()),
                    ..Default::default()
                },
                AudioStreamInfo {
                    codec: "aac".to_string(),
                    channels: 2,
                    channel_layout: Some("stereo".to_string()),
                    bitrate_kbps: Some(192),
                    language: Some("jpn".to_string()),
                    ..Default::default()
                },
            ],
            subtitles: vec![SubtitleStreamInfo {
                codec: "subrip".to_string(),
                language: Some("eng".to_string()),
                title: Some("Signs & Songs".to_string()),
                ..Default::default()
            }],
            duration_secs: Some(2537.0),
        };

        let mut buf = Vec::<u8>::new();
        let path = std::path::Path::new("episode01.mkv");
        render(&probe, path, false, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();

        assert!(output.contains("episode01.mkv"), "filename in header");
        assert!(output.contains("42:17"), "duration in header");
        assert!(output.contains("H.264"), "friendly video codec");
        assert!(output.contains("1920 × 1080"), "resolution");
        assert!(output.contains("23.976 fps"), "framerate");
        assert!(output.contains("Audio"), "audio section header");
        assert!(output.contains("2 tracks"), "audio track count");
        assert!(output.contains("DTS"), "friendly audio codec");
        assert!(output.contains("\"English DTS\""), "audio title");
        assert!(output.contains("Subtitles"), "subtitles section header");
        assert!(
            output.contains("SRT"),
            "friendly sub codec — subrip maps to SRT"
        );
        assert!(output.contains("\"Signs & Songs\""), "subtitle title");
        assert!(output.contains("source = N"), "footer hint");
    }
}
