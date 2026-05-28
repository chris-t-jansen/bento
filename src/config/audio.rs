//! `[audio]` — section-level defaults that cascade as per-track defaults, plus a
//! list of output tracks. `normalize_downmix` cascades like the other per-track
//! defaults; the remaining section-only fields (`warn_no_default`,
//! `warn_unnormalized_downmix`, `tracks`) do not cascade.

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct Audio {
    // Cascade as per-track defaults
    pub encoder: Option<String>,
    pub bitrate: Option<u32>,
    pub mixdown: Option<Mixdown>,
    pub force_bitrate: Option<bool>,
    pub force_mixdown: Option<bool>,
    // `normalize_mix` is the pre-1.1 name. The alias keeps old configs parsing;
    // `bento repair` upgrades the key in-place via the rename table in repair.rs
    // (keep the two in sync when adding future renames).
    #[serde(alias = "normalize_mix")]
    pub normalize_downmix: Option<bool>,

    // Section-only — do not cascade
    pub warn_no_default: Option<bool>,
    pub warn_unnormalized_downmix: Option<bool>,
    pub tracks: Option<Vec<AudioTrack>>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum Mixdown {
    #[serde(rename = "stereo")]
    Stereo,
    #[serde(rename = "5point1")]
    FivePointOne,
    #[serde(rename = "mono")]
    Mono,
    #[serde(rename = "dpl2")]
    Dpl2,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct AudioTrack {
    pub source: Option<u32>,
    pub lang: Option<String>,
    pub title: Option<String>,
    pub default: Option<bool>,
    pub forced: Option<bool>,
    pub original: Option<bool>,
    pub commentary: Option<bool>,
    pub hearing_impaired: Option<bool>,
    pub visual_impaired: Option<bool>,
    // Per-track overrides of section-level defaults
    pub encoder: Option<String>,
    pub bitrate: Option<u32>,
    pub mixdown: Option<Mixdown>,
    pub force_bitrate: Option<bool>,
    pub force_mixdown: Option<bool>,
    pub normalize_downmix: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// Global-config `[audio]` example from the design doc — defaults only, no tracks.
    #[test]
    fn parses_global_example() {
        let toml_str = r#"
[audio]
encoder = "aac"
bitrate = 192
mixdown = "stereo"
force_bitrate = false
force_mixdown = true
normalize_downmix = true
"#;
        let a = Config::from_toml_str(toml_str).unwrap().audio;
        assert_eq!(a.encoder.as_deref(), Some("aac"));
        assert_eq!(a.bitrate, Some(192));
        assert_eq!(a.mixdown, Some(Mixdown::Stereo));
        assert_eq!(a.force_bitrate, Some(false));
        assert_eq!(a.force_mixdown, Some(true));
        assert_eq!(a.normalize_downmix, Some(true));
        assert!(a.tracks.is_none());
    }

    /// `normalize_mix` is the pre-1.1 name and must still parse via the serde
    /// alias, landing in the `normalize_downmix` field.
    #[test]
    fn normalize_mix_alias_parses_into_normalize_downmix() {
        let a = Config::from_toml_str("[audio]\nnormalize_mix = false\n")
            .unwrap()
            .audio;
        assert_eq!(a.normalize_downmix, Some(false));
    }

    /// `normalize_downmix` is overridable per-track.
    #[test]
    fn normalize_downmix_per_track_override_parses() {
        let toml_str = r#"
[audio]
normalize_downmix = true
tracks = [
    { source = 1, lang = "jpn" },
    { source = 2, lang = "eng", normalize_downmix = false },
]
"#;
        let a = Config::from_toml_str(toml_str).unwrap().audio;
        assert_eq!(a.normalize_downmix, Some(true));
        let tracks = a.tracks.expect("tracks present");
        assert_eq!(tracks[0].normalize_downmix, None);
        assert_eq!(tracks[1].normalize_downmix, Some(false));
    }

    /// Directory-config `[audio]` example — three tracks with sparse overrides.
    #[test]
    fn parses_directory_example() {
        let toml_str = r#"
[audio]
tracks = [
    { source = 1, lang = "jpn", title = "Japanese", default = true, original = true },
    { source = 2, lang = "eng", title = "English Dub" },
    { source = 3, lang = "eng", title = "Director's Commentary", commentary = true, bitrate = 96 },
]
"#;
        let a = Config::from_toml_str(toml_str).unwrap().audio;
        let tracks = a.tracks.expect("tracks present");
        assert_eq!(tracks.len(), 3);

        assert_eq!(tracks[0].source, Some(1));
        assert_eq!(tracks[0].lang.as_deref(), Some("jpn"));
        assert_eq!(tracks[0].default, Some(true));
        assert_eq!(tracks[0].original, Some(true));

        assert_eq!(tracks[1].source, Some(2));
        assert_eq!(tracks[1].title.as_deref(), Some("English Dub"));
        assert_eq!(tracks[1].default, None);

        assert_eq!(tracks[2].source, Some(3));
        assert_eq!(tracks[2].commentary, Some(true));
        assert_eq!(tracks[2].bitrate, Some(96));
    }

    #[test]
    fn parses_all_mixdown_values() {
        for (s, expected) in [
            ("stereo", Mixdown::Stereo),
            ("5point1", Mixdown::FivePointOne),
            ("mono", Mixdown::Mono),
            ("dpl2", Mixdown::Dpl2),
        ] {
            let toml_str = format!("[audio]\nmixdown = \"{}\"\n", s);
            let a = Config::from_toml_str(&toml_str).expect("mixdown parses");
            assert_eq!(a.audio.mixdown, Some(expected));
        }
    }
}
