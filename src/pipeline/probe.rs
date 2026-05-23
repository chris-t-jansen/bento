//! Source-file probing via ffprobe and ffmpeg cropdetect.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Parsed stream info from a source file.
#[derive(Debug, Default)]
pub struct SourceProbe {
    pub video: VideoStreamInfo,
    pub audio: Vec<AudioStreamInfo>,
    pub subtitles: Vec<SubtitleStreamInfo>,
    /// Total media duration in seconds, if ffprobe can determine it.
    pub duration_secs: Option<f64>,
}

#[derive(Debug, Default)]
pub struct VideoStreamInfo {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Default)]
pub struct AudioStreamInfo {
    /// Codec name as reported by ffprobe (e.g. "aac", "ac3", "dts", "flac").
    pub codec: String,
    pub channels: u32,
    /// Source bitrate in kbps, if ffprobe can determine it.
    pub bitrate_kbps: Option<u32>,
    pub sample_rate: Option<u32>,
    pub language: Option<String>,
}

#[derive(Debug, Default)]
pub struct SubtitleStreamInfo {
    /// Codec name as reported by ffprobe (e.g. "ass", "subrip").
    pub codec: String,
}

impl SourceProbe {
    /// Look up the subtitle codec for a 1-based track index.
    pub fn subtitle_codec(&self, one_based: u32) -> Option<&str> {
        let zero = one_based.saturating_sub(1) as usize;
        self.subtitles.get(zero).map(|s| s.codec.as_str())
    }
}

/// Run one `ffprobe -show_streams -show_format -of json` call and return parsed stream info.
pub fn probe_source_streams(input: &Path) -> Result<SourceProbe> {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
            &input.display().to_string(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::FfmpegNotFound
            } else {
                Error::Io {
                    path: PathBuf::from("ffprobe"),
                    source: e,
                }
            }
        })?;

    if !output.status.success() {
        return Err(Error::FfprobeFailed {
            status: output.status.code().unwrap_or(-1),
            context: format!("probing streams of {}", input.display()),
        });
    }

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| Error::FfprobeFailed {
            status: 0,
            context: format!("parsing ffprobe output for {}: {}", input.display(), e),
        })?;

    let streams = json["streams"].as_array().cloned().unwrap_or_default();

    let mut video = VideoStreamInfo::default();
    let mut video_found = false;
    let mut audio: Vec<AudioStreamInfo> = Vec::new();
    let mut subtitles: Vec<SubtitleStreamInfo> = Vec::new();

    for stream in &streams {
        let codec_type = stream["codec_type"].as_str().unwrap_or("");
        match codec_type {
            "video" if !video_found => {
                video_found = true;
                video = VideoStreamInfo {
                    width: stream["width"].as_u64().unwrap_or(0) as u32,
                    height: stream["height"].as_u64().unwrap_or(0) as u32,
                };
            }
            "audio" => {
                audio.push(AudioStreamInfo {
                    codec: stream["codec_name"].as_str().unwrap_or("").to_string(),
                    channels: stream["channels"].as_u64().unwrap_or(2) as u32,
                    bitrate_kbps: stream["bit_rate"]
                        .as_str()
                        .and_then(|s| s.parse::<u64>().ok())
                        .map(|bps| (bps / 1000) as u32),
                    sample_rate: stream["sample_rate"]
                        .as_str()
                        .and_then(|s| s.parse::<u32>().ok()),
                    language: stream["tags"]["language"].as_str().map(str::to_string),
                });
            }
            "subtitle" => {
                subtitles.push(SubtitleStreamInfo {
                    codec: stream["codec_name"].as_str().unwrap_or("").to_string(),
                });
            }
            _ => {}
        }
    }

    let duration_secs = json["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|&d| d > 0.0);

    Ok(SourceProbe {
        video,
        audio,
        subtitles,
        duration_secs,
    })
}

/// Run a short ffmpeg cropdetect pass (first 10 s) and return the crop
/// parameters as `"W:H:X:Y"`, or `None` if nothing was detected.
pub fn probe_cropdetect(input: &Path) -> Result<Option<String>> {
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-t",
            "10",
            "-i",
            &input.display().to_string(),
            "-vf",
            "cropdetect=24:16:0",
            "-an",
            "-sn",
            "-f",
            "null",
            "-",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::FfmpegNotFound
            } else {
                Error::Io {
                    path: PathBuf::from("ffmpeg"),
                    source: e,
                }
            }
        })?;

    // cropdetect emits lines like:
    //   [Parsed_cropdetect_0 @ ...] ... crop=1920:800:0:140
    // We want the last such line's crop= value.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let crop = stderr
        .lines()
        .filter_map(|line| {
            let pos = line.rfind("crop=")?;
            let rest = &line[pos + 5..]; // skip "crop="
            let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
            let value = rest[..end].trim();
            if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            }
        })
        .next_back();

    Ok(crop)
}

#[cfg(test)]
mod tests {

    #[test]
    fn parse_cropdetect_extracts_last_crop_value() {
        // Simulate cropdetect stderr output
        let stderr = "\
[Parsed_cropdetect_0 @ 0xabc] x1:0 x2:1919 y1:140 y2:939 w:1920 h:800 x:0 y:140 t:2.0 crop=1920:800:0:140\n\
[Parsed_cropdetect_0 @ 0xabc] x1:0 x2:1919 y1:138 y2:937 w:1920 h:800 x:0 y:138 t:4.0 crop=1920:800:0:138\n";

        let crop = stderr
            .lines()
            .filter_map(|line| {
                let pos = line.rfind("crop=")?;
                let rest = &line[pos + 5..];
                let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
                let value = rest[..end].trim();
                if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                }
            })
            .next_back();

        assert_eq!(crop, Some("1920:800:0:138".to_string()));
    }
}
