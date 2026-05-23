//! `[video]` — single video stream in, single video stream out. No track list.

use std::fmt;

use serde::de::value::{MapAccessDeserializer, StrDeserializer};
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt::Display;

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct Video {
    pub encoder: Option<Encoder>,
    pub preset: Option<Preset>,
    pub crop: Option<Crop>,
    pub deinterlace: Option<DeinterlaceMode>,
    pub detelecine: Option<DetelecineMode>,
    pub denoise: Option<Denoise>,
    pub resolution: Option<Resolution>,
    pub never_upscale: Option<bool>,
    pub warn_crf_codec_mismatch: Option<bool>,
}

// --- Encoder ----------------------------------------------------------------

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct Encoder {
    pub name: Option<EncoderName>,
    pub crf: Option<u32>,
    pub tune: Option<Tune>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EncoderName {
    X264,
    X265,
}

impl EncoderName {
    pub fn as_ffmpeg(self) -> &'static str {
        match self {
            EncoderName::X264 => "libx264",
            EncoderName::X265 => "libx265",
        }
    }
}

/// Psychovisual tune. The set is the union over x264 + x265; cross-field
/// validation enforces that the resolved tune is valid for the resolved encoder.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Tune {
    Film,
    Animation,
    Grain,
    Stillimage,
    Psnr,
    Ssim,
    Fastdecode,
    Zerolatency,
    None,
}

impl Display for Tune {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Tune::Film => "film",
            Tune::Animation => "animation",
            Tune::Grain => "grain",
            Tune::Stillimage => "stillimage",
            Tune::Psnr => "psnr",
            Tune::Ssim => "ssim",
            Tune::Fastdecode => "fastdecode",
            Tune::Zerolatency => "zerolatency",
            Tune::None => "none",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Ultrafast,
    Superfast,
    Veryfast,
    Faster,
    Fast,
    Medium,
    Slow,
    Slower,
    Veryslow,
    Placebo,
}

impl Display for Preset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Preset::Ultrafast => "ultrafast",
            Preset::Superfast => "superfast",
            Preset::Veryfast => "veryfast",
            Preset::Faster => "faster",
            Preset::Fast => "fast",
            Preset::Medium => "medium",
            Preset::Slow => "slow",
            Preset::Slower => "slower",
            Preset::Veryslow => "veryslow",
            Preset::Placebo => "placebo",
        };
        f.write_str(s)
    }
}

// --- Crop -------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Crop {
    Mode(CropMode),
    Explicit(CropPixels),
}

// Custom Deserialize so that typos inside the explicit-pixels table variant
// produce clear errors (e.g. `unknown field 'tup', expected one of 'top',
// 'bottom', 'left', 'right'`) rather than serde's default untagged-enum
// `data did not match any variant` message.
impl<'de> Deserialize<'de> for Crop {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Crop;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str(r#"either a string ("none" or "auto") or a table with optional top/bottom/left/right pixel values"#)
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Crop, E> {
                CropMode::deserialize(StrDeserializer::<E>::new(v)).map(Crop::Mode)
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Crop, E> {
                self.visit_str(&v)
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Crop, A::Error> {
                CropPixels::deserialize(MapAccessDeserializer::new(map)).map(Crop::Explicit)
            }
        }
        deserializer.deserialize_any(V)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CropMode {
    None,
    Auto,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CropPixels {
    pub top: Option<u32>,
    pub bottom: Option<u32>,
    pub left: Option<u32>,
    pub right: Option<u32>,
}

// --- Deinterlace / detelecine ----------------------------------------------

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeinterlaceMode {
    None,
    Auto,
    Yadif,
    /// Motion-adaptive deinterlacer (ffmpeg `bwdif`). Replaces HandBrake's
    /// `decomb` — closest behavioral analog on mixed-content sources.
    #[serde(rename = "bwdif")]
    Bwdif,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DetelecineMode {
    None,
    Auto,
}

// --- Denoise ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Denoise {
    Off(DenoiseOff),
    Active(DenoiseConfig),
}

impl<'de> Deserialize<'de> for Denoise {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Denoise;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str(r#"either "none" or a table { filter, preset }"#)
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Denoise, E> {
                DenoiseOff::deserialize(StrDeserializer::<E>::new(v)).map(Denoise::Off)
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Denoise, E> {
                self.visit_str(&v)
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Denoise, A::Error> {
                DenoiseConfig::deserialize(MapAccessDeserializer::new(map)).map(Denoise::Active)
            }
        }
        deserializer.deserialize_any(V)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DenoiseOff {
    None,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct DenoiseConfig {
    pub filter: Option<DenoiseFilter>,
    pub preset: Option<DenoisePreset>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DenoiseFilter {
    Nlmeans,
    Hqdn3d,
}

impl DenoiseFilter {
    pub fn as_ffmpeg(self) -> &'static str {
        match self {
            DenoiseFilter::Nlmeans => "nlmeans",
            DenoiseFilter::Hqdn3d => "hqdn3d",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DenoisePreset {
    Ultralight,
    Light,
    Medium,
    Strong,
    Stronger,
    Verystrong,
}

// --- Resolution -------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Resolution {
    Mode(ResolutionMode),
    Explicit(ResolutionExplicit),
}

impl<'de> Deserialize<'de> for Resolution {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Resolution;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str(r#"either "original" or a table { width, height }"#)
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Resolution, E> {
                ResolutionMode::deserialize(StrDeserializer::<E>::new(v)).map(Resolution::Mode)
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Resolution, E> {
                self.visit_str(&v)
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Resolution, A::Error> {
                ResolutionExplicit::deserialize(MapAccessDeserializer::new(map))
                    .map(Resolution::Explicit)
            }
        }
        deserializer.deserialize_any(V)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionMode {
    Original,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ResolutionExplicit {
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// Global-config `[video]` example from the design doc.
    #[test]
    fn parses_global_example() {
        let toml_str = r#"
[video]
encoder = { name = "x264", crf = 20, tune = "animation" }
preset = "medium"
crop = "none"
deinterlace = "none"
detelecine = "none"
denoise = "none"
resolution = "original"
never_upscale = true
"#;
        let v = Config::from_toml_str(toml_str).unwrap().video;
        let enc = v.encoder.expect("encoder present");
        assert_eq!(enc.name, Some(EncoderName::X264));
        assert_eq!(enc.crf, Some(20));
        assert_eq!(enc.tune, Some(Tune::Animation));
        assert_eq!(v.preset, Some(Preset::Medium));
        assert_eq!(v.crop, Some(Crop::Mode(CropMode::None)));
        assert_eq!(v.deinterlace, Some(DeinterlaceMode::None));
        assert_eq!(v.detelecine, Some(DetelecineMode::None));
        assert_eq!(v.denoise, Some(Denoise::Off(DenoiseOff::None)));
        assert_eq!(
            v.resolution,
            Some(Resolution::Mode(ResolutionMode::Original))
        );
        assert_eq!(v.never_upscale, Some(true));
    }

    /// Directory-config example: explicit crop + auto detelecine, sparse override.
    #[test]
    fn parses_directory_example() {
        let toml_str = r#"
[video]
detelecine = "auto"
crop = { top = 60, bottom = 60 }
"#;
        let v = Config::from_toml_str(toml_str).unwrap().video;
        assert_eq!(v.detelecine, Some(DetelecineMode::Auto));
        match v.crop.expect("crop present") {
            Crop::Explicit(px) => {
                assert_eq!(px.top, Some(60));
                assert_eq!(px.bottom, Some(60));
                assert_eq!(px.left, None);
                assert_eq!(px.right, None);
            }
            other => panic!("expected explicit crop, got {:?}", other),
        }
    }

    #[test]
    fn parses_bwdif_deinterlace() {
        let toml_str = "[video]\ndeinterlace = \"bwdif\"\n";
        let v = Config::from_toml_str(toml_str).unwrap().video;
        assert_eq!(v.deinterlace, Some(DeinterlaceMode::Bwdif));
    }

    #[test]
    fn parses_partial_encoder_override() {
        // Per the leaf-level resolution rule: only `crf` is set here; `name`/`tune`
        // fall through at resolution time.
        let toml_str = r#"
[video]
encoder = { crf = 22 }
"#;
        let v = Config::from_toml_str(toml_str).unwrap().video;
        let enc = v.encoder.expect("encoder present");
        assert_eq!(enc.name, None);
        assert_eq!(enc.crf, Some(22));
        assert_eq!(enc.tune, None);
    }

    #[test]
    fn parses_denoise_active() {
        let toml_str = r#"
[video]
denoise = { filter = "nlmeans", preset = "light" }
"#;
        let v = Config::from_toml_str(toml_str).unwrap().video;
        match v.denoise.expect("denoise present") {
            Denoise::Active(c) => {
                assert_eq!(c.filter, Some(DenoiseFilter::Nlmeans));
                assert_eq!(c.preset, Some(DenoisePreset::Light));
            }
            other => panic!("expected active denoise, got {:?}", other),
        }
    }

    #[test]
    fn parses_resolution_explicit() {
        let toml_str = r#"
[video]
resolution = { width = 1280, height = 720 }
"#;
        let v = Config::from_toml_str(toml_str).unwrap().video;
        match v.resolution.expect("resolution present") {
            Resolution::Explicit(r) => {
                assert_eq!(r.width, Some(1280));
                assert_eq!(r.height, Some(720));
            }
            other => panic!("expected explicit resolution, got {:?}", other),
        }
    }

    #[test]
    fn rejects_unknown_tune() {
        let toml_str = r#"
[video]
encoder = { tune = "bogus" }
"#;
        Config::from_toml_str(toml_str).expect_err("unknown tune rejected");
    }

    // --- Untagged-enum error message quality -----------------------------------
    //
    // Before custom Deserialize impls, typos inside the table-form variants
    // of Crop/Denoise/Resolution surfaced as the unhelpful `data did not
    // match any variant of untagged enum X`. These tests pin the better
    // behavior: the inner struct's `deny_unknown_fields` error propagates,
    // naming the typo'd field and the valid field set.

    #[test]
    fn crop_table_typo_names_field_and_valid_set() {
        let toml_str = r#"
[video]
crop = { tup = 60, bottom = 60 }
"#;
        let err = Config::from_toml_str(toml_str).unwrap_err().to_string();
        assert!(err.contains("unknown field `tup`"), "got: {}", err);
        assert!(err.contains("`top`"), "got: {}", err);
        assert!(err.contains("`bottom`"), "got: {}", err);
        assert!(
            !err.contains("data did not match any variant"),
            "got: {}",
            err
        );
    }

    #[test]
    fn denoise_table_typo_names_field_and_valid_set() {
        let toml_str = r#"
[video]
denoise = { filter = "nlmeans", preset = "light", strenght = 5 }
"#;
        let err = Config::from_toml_str(toml_str).unwrap_err().to_string();
        assert!(err.contains("unknown field `strenght`"), "got: {}", err);
        assert!(err.contains("`filter`"), "got: {}", err);
        assert!(err.contains("`preset`"), "got: {}", err);
    }

    #[test]
    fn resolution_table_typo_names_field_and_valid_set() {
        let toml_str = r#"
[video]
resolution = { width = 1280, hight = 720 }
"#;
        let err = Config::from_toml_str(toml_str).unwrap_err().to_string();
        assert!(err.contains("unknown field `hight`"), "got: {}", err);
        assert!(err.contains("`width`"), "got: {}", err);
        assert!(err.contains("`height`"), "got: {}", err);
    }

    #[test]
    fn crop_string_typo_names_valid_modes() {
        let toml_str = r#"
[video]
crop = "automatic"
"#;
        let err = Config::from_toml_str(toml_str).unwrap_err().to_string();
        // CropMode's derive(Deserialize) reports unknown variant — the
        // visitor forwards string inputs to it directly.
        assert!(err.contains("automatic"), "got: {}", err);
        assert!(err.contains("none") || err.contains("auto"), "got: {}", err);
    }
}
