//! Bento configuration schema.
//!
//! Every field is `Option<T>`: each layer of the cascade (CLI, per-file, directory,
//! global, baked-in defaults) declares only the fields it sets. Resolution is a
//! separate phase (see Phase 2 — `crate::resolve`).
//!
//! Schema parsing only enforces *shape* (structure, types, no unknown fields).
//! Cross-field validation (CRF/codec coupling, `default` uniqueness, mutually
//! exclusive `filter`/`subtract_track`, etc.) is a separate pass that runs after
//! resolution.

mod audio;
mod output;
mod subtitles;
mod video;

pub use audio::*;
pub use output::*;
pub use subtitles::*;
pub use video::*;

use serde::{Deserialize, Serialize};

/// Root config object — the parsed contents of one config file (any layer).
#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct Config {
    #[serde(default)]
    pub output: Output,
    #[serde(default)]
    pub video: Video,
    #[serde(default)]
    pub audio: Audio,
    #[serde(default)]
    pub subtitles: Subtitles,
}

impl Config {
    pub fn from_toml_str(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A multi-section config combining elements from several design-doc examples
    /// — verifies the top-level Config wires all four sections cleanly.
    #[test]
    fn parses_combined_config() {
        let toml_str = r#"
[output]
container = "mp4"
destination = "encoded"
preserve_chapters = true
on_existing = "warn"
metadata = { show = "Cowboy Bebop", season = 1, year = 1998 }

[video]
encoder = { name = "x264", crf = 20, tune = "animation" }
preset = "medium"

[audio]
encoder = "aac"
bitrate = 192
mixdown = "stereo"
tracks = [
    { source = 1, lang = "jpn", title = "Japanese", default = true, original = true },
    { source = 2, lang = "eng", title = "English Dub" },
]

[subtitles]
tracks = [
    { source = 1, format = "srt", mux = "soft", subtract_track = 2, lang = "eng", title = "English", default = true },
    { source = 2, format = "ass", mux = "burn" },
]
"#;
        let cfg: Config = Config::from_toml_str(toml_str).expect("config should parse");
        assert_eq!(cfg.output.container, Some(Container::Mp4));
        assert_eq!(cfg.video.preset, Some(Preset::Medium));
        assert_eq!(cfg.audio.bitrate, Some(192));
        assert_eq!(cfg.subtitles.tracks.as_ref().map(Vec::len), Some(2));
    }

    #[test]
    fn empty_input_yields_default() {
        let cfg: Config = Config::from_toml_str("").expect("empty config parses");
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn unknown_top_level_section_rejected() {
        let toml_str = r#"
[pipeline]
foo = "bar"
"#;
        Config::from_toml_str(toml_str).expect_err("unknown sections rejected");
    }

    #[test]
    fn unknown_field_rejected() {
        let toml_str = r#"
[output]
typo_here = "mp4"
"#;
        Config::from_toml_str(toml_str).expect_err("unknown fields rejected");
    }
}
