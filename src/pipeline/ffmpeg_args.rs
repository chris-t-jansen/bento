//! ffmpeg argument construction for the main encode pass.

use std::path::Path;

use crate::config::{
    Audio, AudioTrack, Config, Container, Crop, CropMode, DeinterlaceMode, Denoise, DenoiseFilter,
    DenoisePreset, DetelecineMode, Mixdown, Resolution, Tune,
};
use crate::pipeline::probe::{AudioStreamInfo, SourceProbe};
use crate::pipeline::subtitle_prep::{PreparedSubtitle, SourceFormat};

/// Audio track disposition: stream-copy vs. re-encode.
#[derive(Debug, PartialEq)]
pub enum AudioAction {
    Copy,
    Transcode {
        encoder: String,
        bitrate_kbps: u32,
        channels: u32,
        use_dpl2: bool,
    },
}

/// Decide whether to copy or transcode one audio track.
///
/// Transcoding is required when any of these hold:
/// - `normalize_mix` is true (loudnorm requires a filter pass)
/// - `force_bitrate` is true
/// - `force_mixdown` is true
/// - source codec doesn't match the target encoder
/// - source has more channels than the target mixdown
/// - source bitrate exceeds the target
pub fn decide_audio_action(
    track: &AudioTrack,
    section: &Audio,
    source: &AudioStreamInfo,
    normalize_mix: bool,
) -> AudioAction {
    let encoder = track
        .encoder
        .as_deref()
        .or(section.encoder.as_deref())
        .unwrap_or("aac");
    let target_bitrate = track.bitrate.or(section.bitrate).unwrap_or(192);
    let force_bitrate = track
        .force_bitrate
        .or(section.force_bitrate)
        .unwrap_or(false);
    let force_mixdown = track
        .force_mixdown
        .or(section.force_mixdown)
        .unwrap_or(false);
    let mixdown = track.mixdown.or(section.mixdown).unwrap_or(Mixdown::Stereo);
    let target_channels = mixdown_to_channels(mixdown);
    let use_dpl2 = matches!(mixdown, Mixdown::Dpl2);

    if normalize_mix || force_bitrate || force_mixdown {
        return AudioAction::Transcode {
            encoder: audio_encoder_to_ffmpeg(encoder).to_string(),
            bitrate_kbps: target_bitrate,
            channels: target_channels,
            use_dpl2,
        };
    }

    let codec_ok = source.codec == encoder;
    let channels_ok = source.channels <= target_channels;
    let bitrate_ok = source
        .bitrate_kbps
        .map(|src| src <= target_bitrate)
        .unwrap_or(true);

    if codec_ok && channels_ok && bitrate_ok {
        AudioAction::Copy
    } else {
        AudioAction::Transcode {
            encoder: audio_encoder_to_ffmpeg(encoder).to_string(),
            bitrate_kbps: target_bitrate,
            channels: target_channels,
            use_dpl2,
        }
    }
}

/// Build the complete ffmpeg argument list for the main encode.
///
/// Soft subtitle files are added as additional inputs (indices 1…N after the
/// main video at index 0). Burn subtitle files are wired into the `-vf`
/// filtergraph via `subtitles=` entries. The caller is responsible for
/// running the returned args against the `ffmpeg` binary.
pub fn build_ffmpeg_args(
    input: &Path,
    output: &Path,
    config: &Config,
    probe: &SourceProbe,
    prepared_subs: &[PreparedSubtitle],
    crop_params: Option<&str>,
    episode: Option<i64>,
) -> Vec<String> {
    let container = config.output.container.unwrap_or(Container::Mp4);
    let normalize_mix = config.audio.normalize_mix.unwrap_or(false);

    let mut args: Vec<String> = Vec::new();

    args.push("-y".into());

    // --- Inputs ---------------------------------------------------------------
    args.push("-i".into());
    args.push(input.display().to_string());

    let soft_subs: Vec<&PreparedSubtitle> = prepared_subs
        .iter()
        .filter(|s| matches!(s, PreparedSubtitle::Soft { .. }))
        .collect();

    for sub in &soft_subs {
        if let PreparedSubtitle::Soft { file, .. } = sub {
            args.push("-i".into());
            args.push(file.display().to_string());
        }
    }

    // --- Stream maps ----------------------------------------------------------
    args.push("-map".into());
    args.push("0:v:0".into());

    if let Some(tracks) = &config.audio.tracks {
        for track in tracks {
            let src = track.source.expect("validated") as usize;
            // source is 1-based; ffmpeg stream selectors are 0-based.
            args.push("-map".into());
            args.push(format!("0:a:{}", src.saturating_sub(1)));
        }
    }

    // Soft subtitle inputs start at index 1 (main input is 0).
    for (i, _) in soft_subs.iter().enumerate() {
        args.push("-map".into());
        args.push(format!("{}:s:0", i + 1));
    }

    // --- Video codec ----------------------------------------------------------
    if let Some(encoder) = &config.video.encoder {
        let codec_name = encoder.name.map(|n| n.as_ffmpeg()).unwrap_or("libx264");
        args.push("-c:v".into());
        args.push(codec_name.into());

        if let Some(crf) = encoder.crf {
            args.push("-crf".into());
            args.push(crf.to_string());
        }

        if let Some(tune) = encoder.tune {
            if !matches!(tune, Tune::None) {
                args.push("-tune".into());
                args.push(tune.to_string());
            }
        }
    } else {
        args.push("-c:v".into());
        args.push("libx264".into());
    }

    if let Some(preset) = config.video.preset {
        args.push("-preset".into());
        args.push(preset.to_string());
    }

    // Chapters
    if config.output.preserve_chapters.unwrap_or(true) {
        args.push("-map_chapters".into());
        args.push("0".into());
    } else {
        args.push("-map_chapters".into());
        args.push("-1".into());
    }

    // --- Video filter chain ---------------------------------------------------
    let vf = build_video_filters(config, probe, prepared_subs, crop_params);
    if !vf.is_empty() {
        args.push("-vf".into());
        args.push(vf);
    }

    // --- Audio codec and filters per track ------------------------------------
    if let Some(tracks) = &config.audio.tracks {
        for (i, track) in tracks.iter().enumerate() {
            let source_idx = track.source.expect("validated").saturating_sub(1) as usize;
            let stream_info = probe.audio.get(source_idx);

            let action = stream_info
                .map(|src| decide_audio_action(track, &config.audio, src, normalize_mix))
                .unwrap_or_else(|| {
                    let mixdown = track
                        .mixdown
                        .or(config.audio.mixdown)
                        .unwrap_or(Mixdown::Stereo);
                    AudioAction::Transcode {
                        encoder: audio_encoder_to_ffmpeg(
                            track
                                .encoder
                                .as_deref()
                                .or(config.audio.encoder.as_deref())
                                .unwrap_or("aac"),
                        )
                        .to_string(),
                        bitrate_kbps: track.bitrate.or(config.audio.bitrate).unwrap_or(192),
                        channels: mixdown_to_channels(mixdown),
                        use_dpl2: matches!(mixdown, Mixdown::Dpl2),
                    }
                });

            match &action {
                AudioAction::Copy => {
                    args.push(format!("-c:a:{}", i));
                    args.push("copy".into());
                }
                AudioAction::Transcode {
                    encoder,
                    bitrate_kbps,
                    channels,
                    use_dpl2,
                } => {
                    args.push(format!("-c:a:{}", i));
                    args.push(encoder.clone());
                    args.push(format!("-b:a:{}", i));
                    args.push(format!("{}k", bitrate_kbps));
                    args.push(format!("-ac:{}", i));
                    args.push(channels.to_string());

                    let mut filters: Vec<&str> = Vec::new();
                    if normalize_mix {
                        filters.push("loudnorm=I=-16:TP=-1.5:LRA=11");
                    }
                    if *use_dpl2 {
                        filters.push("aresample=matrix_encoding=dplii");
                    }
                    if !filters.is_empty() {
                        args.push(format!("-filter:a:{}", i));
                        args.push(filters.join(","));
                    }
                }
            }

            // Track metadata
            if let Some(lang) = &track.lang {
                args.push(format!("-metadata:s:a:{}", i));
                args.push(format!("language={}", lang));
            }
            if let Some(title) = &track.title {
                args.push(format!("-metadata:s:a:{}", i));
                args.push(format!("title={}", title));
            }

            let mut disposition = Vec::new();
            if track.default == Some(true) {
                disposition.push("default");
            }
            if track.forced == Some(true) {
                disposition.push("forced");
            }
            if track.commentary == Some(true) {
                disposition.push("comment");
            }
            if track.hearing_impaired == Some(true) {
                disposition.push("hearing_impaired");
            }
            if track.visual_impaired == Some(true) {
                disposition.push("visual_impaired");
            }
            if track.original == Some(true) {
                disposition.push("original");
            }
            if !disposition.is_empty() {
                args.push(format!("-disposition:a:{}", i));
                args.push(disposition.join("+"));
            }
        }
    }

    // --- Subtitle codec, metadata, and disposition ----------------------------
    for (i, sub) in soft_subs.iter().enumerate() {
        if let PreparedSubtitle::Soft {
            format,
            lang,
            title,
            is_default,
            is_forced,
            is_commentary,
            is_hearing_impaired,
            file: _,
        } = sub
        {
            let codec = soft_subtitle_codec(*format, container);
            args.push(format!("-c:s:{}", i));
            args.push(codec.to_string());

            if let Some(l) = lang {
                args.push(format!("-metadata:s:s:{}", i));
                args.push(format!("language={}", l));
            }
            if let Some(t) = title {
                args.push(format!("-metadata:s:s:{}", i));
                args.push(format!("title={}", t));
            }

            let mut disposition = Vec::new();
            if *is_default {
                disposition.push("default");
            }
            if *is_forced {
                disposition.push("forced");
            }
            if *is_commentary {
                disposition.push("comment");
            }
            if *is_hearing_impaired {
                disposition.push("hearing_impaired");
            }
            if !disposition.is_empty() {
                args.push(format!("-disposition:s:{}", i));
                args.push(disposition.join("+"));
            }
        }
    }

    // Output metadata (show/season/year)
    if let Some(meta) = &config.output.metadata {
        if let Some(show) = &meta.show {
            args.push("-metadata".into());
            args.push(format!("show={}", show));
        }
        if let Some(year) = &meta.year {
            args.push("-metadata".into());
            args.push(format!("date={}", year));
        }
    }

    // Episode number derived from naming regex ("episode" or "ep" capture).
    // MP4 uses the iTunes `tves` atom; MKV uses the Matroska PART_NUMBER tag.
    if let Some(ep) = episode {
        let key = match container {
            Container::Mp4 => "tves",
            Container::Mkv => "PART_NUMBER",
        };
        args.push("-metadata".into());
        args.push(format!("{}={}", key, ep));
    }

    // Container
    args.push("-f".into());
    args.push(container.as_ffmpeg_muxer().into());

    args.push(output.display().to_string());

    args
}

fn build_video_filters(
    config: &Config,
    probe: &SourceProbe,
    prepared_subs: &[PreparedSubtitle],
    crop_params: Option<&str>,
) -> String {
    let mut filters: Vec<String> = Vec::new();

    // Crop
    match &config.video.crop {
        Some(Crop::Mode(CropMode::None)) => {}
        Some(Crop::Mode(CropMode::Auto)) => {
            if let Some(params) = crop_params {
                filters.push(format!("crop={}", params));
            }
        }
        Some(Crop::Explicit(p)) => {
            // ffmpeg crop=out_w:out_h:x:y; CropPixels are edge insets.
            let left = p.left.unwrap_or(0);
            let right = p.right.unwrap_or(0);
            let top = p.top.unwrap_or(0);
            let bottom = p.bottom.unwrap_or(0);
            let out_w = probe.video.width.saturating_sub(left + right);
            let out_h = probe.video.height.saturating_sub(top + bottom);
            filters.push(format!("crop={}:{}:{}:{}", out_w, out_h, left, top));
        }
        None => {}
    }

    // Deinterlace
    match config.video.deinterlace {
        Some(DeinterlaceMode::None) | None => {}
        Some(DeinterlaceMode::Auto) | Some(DeinterlaceMode::Yadif) => {
            filters.push("yadif".into());
        }
        Some(DeinterlaceMode::Bwdif) => {
            filters.push("bwdif".into());
        }
    }

    // Detelecine (IVTC: fieldmatch + decimate)
    if matches!(config.video.detelecine, Some(DetelecineMode::Auto)) {
        filters.push("fieldmatch".into());
        filters.push("decimate".into());
    }

    // Denoise
    if let Some(Denoise::Active(c)) = &config.video.denoise {
        let filter_str = match c.filter {
            Some(DenoiseFilter::Nlmeans) => {
                let params = nlmeans_params(c.preset.unwrap_or(DenoisePreset::Medium));
                format!("nlmeans={}", params)
            }
            Some(DenoiseFilter::Hqdn3d) => {
                let params = hqdn3d_params(c.preset.unwrap_or(DenoisePreset::Medium));
                format!("hqdn3d={}", params)
            }
            None => String::new(),
        };
        if !filter_str.is_empty() {
            filters.push(filter_str);
        }
    }

    // Scale / resolution
    if let Some(resolution) = &config.video.resolution {
        if let Resolution::Explicit(r) = resolution {
            let never_upscale = config.video.never_upscale.unwrap_or(true);
            match (r.width, r.height) {
                (Some(w), Some(h)) if never_upscale => {
                    filters.push(format!(
                        "scale={}:{}:force_original_aspect_ratio=decrease",
                        w, h
                    ));
                }
                (Some(w), Some(h)) => {
                    filters.push(format!("scale={}:{}", w, h));
                }
                (Some(w), None) if never_upscale => {
                    filters.push(format!("scale='min({},iw)':-2", w));
                }
                (Some(w), None) => {
                    filters.push(format!("scale={}:-2", w));
                }
                (None, Some(h)) if never_upscale => {
                    filters.push(format!("scale=-2:'min({},ih)'", h));
                }
                (None, Some(h)) => {
                    filters.push(format!("scale=-2:{}", h));
                }
                (None, None) => {}
            }
        }
    }

    // Burn subtitles — always last in the chain (rendered onto processed frame)
    for sub in prepared_subs {
        if let PreparedSubtitle::Burn { file } = sub {
            filters.push(format!("subtitles={}", escape_filtergraph_path(file)));
        }
    }

    filters.join(",")
}

fn soft_subtitle_codec(format: SourceFormat, container: Container) -> &'static str {
    match (format, container) {
        (SourceFormat::Srt, Container::Mp4) => "mov_text",
        (SourceFormat::Srt, Container::Mkv) => "subrip",
        (SourceFormat::Ass, Container::Mkv) => "ass",
        // ASS in MP4 is rejected at validate time; this branch is unreachable.
        (SourceFormat::Ass, Container::Mp4) => "mov_text",
    }
}

pub fn mixdown_to_channels(mixdown: Mixdown) -> u32 {
    match mixdown {
        Mixdown::Mono => 1,
        Mixdown::Stereo | Mixdown::Dpl2 => 2,
        Mixdown::FivePointOne => 6,
    }
}

pub fn audio_encoder_to_ffmpeg(name: &str) -> &str {
    match name {
        "aac" => "aac",
        "opus" => "libopus",
        "flac" => "flac",
        "mp3" => "libmp3lame",
        "vorbis" => "libvorbis",
        "ac3" => "ac3",
        "eac3" => "eac3",
        other => other,
    }
}

fn nlmeans_params(preset: DenoisePreset) -> &'static str {
    match preset {
        DenoisePreset::Ultralight => "1.0:7:7:3:3",
        DenoisePreset::Light => "3.0:7:7:5:5",
        DenoisePreset::Medium => "8.0:7:7:5:5",
        DenoisePreset::Strong => "12.0:7:7:5:5",
        DenoisePreset::Stronger => "16.0:7:7:7:7",
        DenoisePreset::Verystrong => "20.0:7:7:9:9",
    }
}

fn hqdn3d_params(preset: DenoisePreset) -> &'static str {
    match preset {
        DenoisePreset::Ultralight => "1:0.7:2:1.5",
        DenoisePreset::Light => "2:1.5:3:2.5",
        DenoisePreset::Medium => "3:2:6:4.5",
        DenoisePreset::Strong => "7:7:7:5",
        DenoisePreset::Stronger => "10:10:10:8",
        DenoisePreset::Verystrong => "15:15:15:12",
    }
}

/// Escape a path for use inside an ffmpeg filtergraph option value.
/// Colons and backslashes are the characters that require escaping in this
/// context (colons separate filter option key=value pairs; backslashes are
/// the escape character).
pub fn escape_filtergraph_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace(':', "\\:")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AudioTrack;
    use crate::pipeline::probe::AudioStreamInfo;

    fn aac_stream(bitrate_kbps: Option<u32>, channels: u32) -> AudioStreamInfo {
        AudioStreamInfo {
            codec: "aac".to_string(),
            channels,
            bitrate_kbps,
            sample_rate: Some(48000),
            language: None,
        }
    }

    #[test]
    fn copy_when_codec_and_channels_match() {
        let track = AudioTrack::default();
        let section = Audio {
            encoder: Some("aac".to_string()),
            bitrate: Some(192),
            ..Default::default()
        };
        let src = aac_stream(Some(192), 2);
        assert_eq!(
            decide_audio_action(&track, &section, &src, false),
            AudioAction::Copy
        );
    }

    #[test]
    fn transcode_on_codec_mismatch() {
        let track = AudioTrack::default();
        let section = Audio {
            encoder: Some("aac".to_string()),
            bitrate: Some(192),
            ..Default::default()
        };
        let src = aac_stream(Some(192), 2);
        let src_ac3 = AudioStreamInfo {
            codec: "ac3".to_string(),
            ..src
        };
        assert!(matches!(
            decide_audio_action(&track, &section, &src_ac3, false),
            AudioAction::Transcode { .. }
        ));
    }

    #[test]
    fn transcode_when_normalize_mix_is_true() {
        let track = AudioTrack::default();
        let section = Audio {
            encoder: Some("aac".to_string()),
            bitrate: Some(192),
            ..Default::default()
        };
        let src = aac_stream(Some(192), 2);
        assert!(matches!(
            decide_audio_action(&track, &section, &src, true),
            AudioAction::Transcode { .. }
        ));
    }

    #[test]
    fn transcode_on_channel_downmix_needed() {
        let track = AudioTrack::default();
        let section = Audio {
            encoder: Some("aac".to_string()),
            bitrate: Some(192),
            mixdown: Some(Mixdown::Stereo),
            ..Default::default()
        };
        let src = aac_stream(Some(128), 6); // 5.1 source → downmix to stereo
        assert!(matches!(
            decide_audio_action(&track, &section, &src, false),
            AudioAction::Transcode { .. }
        ));
    }

    #[test]
    fn transcode_when_force_bitrate() {
        let track = AudioTrack {
            force_bitrate: Some(true),
            ..Default::default()
        };
        let section = Audio {
            encoder: Some("aac".to_string()),
            bitrate: Some(192),
            ..Default::default()
        };
        let src = aac_stream(Some(192), 2);
        assert!(matches!(
            decide_audio_action(&track, &section, &src, false),
            AudioAction::Transcode { .. }
        ));
    }

    #[test]
    fn escape_filtergraph_path_escapes_colons_and_backslashes() {
        use std::path::PathBuf;
        let path = PathBuf::from("/tmp/bento/track0-burn.ass");
        assert_eq!(escape_filtergraph_path(&path), "/tmp/bento/track0-burn.ass");

        // Windows-style path with backslash and colon
        let win_path = PathBuf::from("C:\\Users\\foo\\track.ass");
        let escaped = escape_filtergraph_path(&win_path);
        assert!(
            escaped.contains("\\\\")
                || escaped.contains("\\:")
                || !escaped.contains(':')
                || cfg!(windows)
        );
    }

    #[test]
    fn audio_encoder_ffmpeg_names() {
        assert_eq!(audio_encoder_to_ffmpeg("aac"), "aac");
        assert_eq!(audio_encoder_to_ffmpeg("opus"), "libopus");
        assert_eq!(audio_encoder_to_ffmpeg("mp3"), "libmp3lame");
        assert_eq!(audio_encoder_to_ffmpeg("flac"), "flac");
        assert_eq!(audio_encoder_to_ffmpeg("vorbis"), "libvorbis");
        assert_eq!(audio_encoder_to_ffmpeg("ac3"), "ac3");
        assert_eq!(audio_encoder_to_ffmpeg("eac3"), "eac3");
        assert_eq!(audio_encoder_to_ffmpeg("custom"), "custom");
    }

    #[test]
    fn build_video_filters_chains_denoise_and_deinterlace() {
        use crate::config::{DenoiseConfig, DenoiseFilter, DenoisePreset, Video};
        use crate::pipeline::probe::{SourceProbe, VideoStreamInfo};
        let config = crate::config::Config {
            video: Video {
                deinterlace: Some(DeinterlaceMode::Yadif),
                denoise: Some(Denoise::Active(DenoiseConfig {
                    filter: Some(DenoiseFilter::Hqdn3d),
                    preset: Some(DenoisePreset::Medium),
                })),
                ..Default::default()
            },
            ..Default::default()
        };
        let probe = SourceProbe {
            video: VideoStreamInfo {
                width: 1920,
                height: 1080,
            },
            audio: Vec::new(),
            subtitles: Vec::new(),
            duration_secs: None,
        };
        let vf = build_video_filters(&config, &probe, &[], None);
        assert_eq!(vf, "yadif,hqdn3d=3:2:6:4.5");
    }
}
