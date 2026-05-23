//! `[subtitles]` — list of output tracks, each independently specifying derivation
//! and presentation. No section-level cascade defaults (subtitles' section-level
//! fields are warning toggles only, not per-track concepts).

use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct Subtitles {
    pub warn_multiple_burns: Option<bool>,
    pub warn_burn_metadata: Option<bool>,
    pub warn_no_default: Option<bool>,
    pub warn_ass_to_srt: Option<bool>,
    pub tracks: Option<Vec<SubtitleTrack>>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct SubtitleTrack {
    // Routing
    pub source: Option<TrackRef>,
    pub format: Option<SubtitleFormat>,
    pub mux: Option<SubtitleMux>,

    // Derivation (mutually exclusive — validation enforces "at most one")
    pub filter: Option<SubtitleFilter>,
    pub subtract_track: Option<TrackRef>,

    // Soft-track metadata (no effect on burn tracks; warn_burn_metadata flags misuse)
    pub lang: Option<String>,
    pub title: Option<String>,
    pub default: Option<bool>,
    pub forced: Option<bool>,
    pub commentary: Option<bool>,
    pub hearing_impaired: Option<bool>,
}

/// Either a track index in the source MKV or a path to an external subtitle file.
/// Paths are resolved relative to the config file containing them (Phase 2).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum TrackRef {
    Index(u32),
    Path(String),
}

// Custom Deserialize so wrong-type inputs produce a clear "expected an
// integer or a string" error rather than serde's default untagged-enum
// `data did not match any variant` message. Negative or oversized integers
// are rejected with a specific message naming the value.
impl<'de> Deserialize<'de> for TrackRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = TrackRef;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("either an integer (input track index) or a string (file path)")
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<TrackRef, E> {
                if v < 0 {
                    return Err(E::custom(format!(
                        "track index must be non-negative, got {}",
                        v
                    )));
                }
                u32::try_from(v)
                    .map(TrackRef::Index)
                    .map_err(|_| E::custom(format!("track index {} doesn't fit in a u32", v)))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<TrackRef, E> {
                u32::try_from(v)
                    .map(TrackRef::Index)
                    .map_err(|_| E::custom(format!("track index {} doesn't fit in a u32", v)))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<TrackRef, E> {
                Ok(TrackRef::Path(v.to_string()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<TrackRef, E> {
                Ok(TrackRef::Path(v))
            }
        }
        deserializer.deserialize_any(V)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleFormat {
    Srt,
    Ass,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleMux {
    Soft,
    Burn,
    External,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct SubtitleFilter {
    pub style: Option<String>,
    pub font: Option<String>,
    pub size: Option<u32>,
    pub mode: Option<FilterMode>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FilterMode {
    Retain,
    Remove,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// Two-track source: full dialogue+signs minus signs-only = soft spoken;
    /// signs-only burned in.
    #[test]
    fn parses_two_track_source_example() {
        let toml_str = r#"
[subtitles]
tracks = [
    {
        source = 1,
        format = "srt",
        mux = "soft",
        subtract_track = 2,
        lang = "eng",
        title = "English",
        default = true,
    },
    {
        source = 2,
        format = "ass",
        mux = "burn",
    },
]
"#;
        let s = Config::from_toml_str(toml_str).unwrap().subtitles;
        let tracks = s.tracks.expect("tracks present");
        assert_eq!(tracks.len(), 2);

        assert_eq!(tracks[0].source, Some(TrackRef::Index(1)));
        assert_eq!(tracks[0].format, Some(SubtitleFormat::Srt));
        assert_eq!(tracks[0].mux, Some(SubtitleMux::Soft));
        assert_eq!(tracks[0].subtract_track, Some(TrackRef::Index(2)));
        assert_eq!(tracks[0].default, Some(true));

        assert_eq!(tracks[1].source, Some(TrackRef::Index(2)));
        assert_eq!(tracks[1].format, Some(SubtitleFormat::Ass));
        assert_eq!(tracks[1].mux, Some(SubtitleMux::Burn));
    }

    /// Single-track source split via complementary style filters.
    #[test]
    fn parses_style_split_example() {
        let toml_str = r#"
[subtitles]
tracks = [
    {
        source = 1,
        format = "srt",
        mux = "soft",
        filter = { style = "Main", mode = "retain" },
        lang = "eng",
        title = "English",
        default = true,
    },
    {
        source = 1,
        format = "ass",
        mux = "burn",
        filter = { style = "Main", mode = "remove" },
    },
]
"#;
        let s = Config::from_toml_str(toml_str).unwrap().subtitles;
        let tracks = s.tracks.expect("tracks present");
        assert_eq!(tracks.len(), 2);

        let f0 = tracks[0].filter.as_ref().expect("filter present");
        assert_eq!(f0.style.as_deref(), Some("Main"));
        assert_eq!(f0.mode, Some(FilterMode::Retain));

        let f1 = tracks[1].filter.as_ref().expect("filter present");
        assert_eq!(f1.mode, Some(FilterMode::Remove));
    }

    /// Hand-edited dialogue track via file-path source.
    #[test]
    fn parses_file_path_source_example() {
        let toml_str = r#"
[subtitles]
tracks = [
    {
        source = "episode06.dialogue.srt",
        format = "srt",
        mux = "soft",
        lang = "eng",
        title = "English",
        default = true,
    },
    {
        source = 2,
        format = "ass",
        mux = "burn",
    },
]
"#;
        let s = Config::from_toml_str(toml_str).unwrap().subtitles;
        let tracks = s.tracks.expect("tracks present");
        assert_eq!(
            tracks[0].source,
            Some(TrackRef::Path("episode06.dialogue.srt".to_string()))
        );
        assert_eq!(tracks[1].source, Some(TrackRef::Index(2)));
    }

    /// Mixed-disposition example — multiple soft sub tracks.
    #[test]
    fn parses_mixed_dispositions_example() {
        let toml_str = r#"
[subtitles]
tracks = [
    {
        source = 1,
        format = "srt",
        mux = "soft",
        filter = { style = "Main", mode = "retain" },
        lang = "eng",
        title = "English (Official)",
        default = true,
    },
    {
        source = 2,
        format = "srt",
        mux = "soft",
        subtract_track = 3,
        lang = "eng",
        title = "English (Fansub, SDH)",
        hearing_impaired = true,
    },
    {
        source = 1,
        format = "ass",
        mux = "burn",
        filter = { style = "Main", mode = "remove" },
    },
]
"#;
        let s = Config::from_toml_str(toml_str).unwrap().subtitles;
        let tracks = s.tracks.expect("tracks present");
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[1].hearing_impaired, Some(true));
        assert_eq!(tracks[1].subtract_track, Some(TrackRef::Index(3)));
    }

    #[test]
    fn parses_subtract_track_as_path() {
        let toml_str = r#"
[subtitles]
tracks = [
    { source = 1, format = "srt", mux = "soft", subtract_track = "signs.ass" },
]
"#;
        let s = Config::from_toml_str(toml_str).unwrap().subtitles;
        let tracks = s.tracks.unwrap();
        assert_eq!(
            tracks[0].subtract_track,
            Some(TrackRef::Path("signs.ass".to_string()))
        );
    }

    #[test]
    fn warn_toggles_parse() {
        let toml_str = r#"
[subtitles]
warn_multiple_burns = false
warn_burn_metadata = false
warn_no_default = false
warn_ass_to_srt = false
"#;
        let s = Config::from_toml_str(toml_str).unwrap().subtitles;
        assert_eq!(s.warn_multiple_burns, Some(false));
        assert_eq!(s.warn_burn_metadata, Some(false));
        assert_eq!(s.warn_no_default, Some(false));
        assert_eq!(s.warn_ass_to_srt, Some(false));
    }

    // --- TrackRef error message quality (Phase 6b) -----------------------

    #[test]
    fn trackref_wrong_type_says_int_or_string() {
        let toml_str = r#"
[subtitles]
tracks = [{ source = true, format = "srt", mux = "soft" }]
"#;
        let err = Config::from_toml_str(toml_str).unwrap_err().to_string();
        assert!(err.contains("integer"), "got: {}", err);
        assert!(err.contains("string"), "got: {}", err);
        assert!(
            !err.contains("data did not match any variant"),
            "got: {}",
            err
        );
    }

    #[test]
    fn trackref_negative_integer_rejected_with_clear_message() {
        let toml_str = r#"
[subtitles]
tracks = [{ source = -1, format = "srt", mux = "soft" }]
"#;
        let err = Config::from_toml_str(toml_str).unwrap_err().to_string();
        assert!(err.contains("non-negative"), "got: {}", err);
        assert!(err.contains("-1"), "got: {}", err);
    }

    #[test]
    fn trackref_int_and_path_both_still_parse() {
        let toml_str = r#"
[subtitles]
tracks = [
    { source = 1, format = "srt", mux = "soft", default = true },
    { source = "external.srt", format = "srt", mux = "soft" },
]
"#;
        let s = Config::from_toml_str(toml_str).unwrap().subtitles;
        let tracks = s.tracks.unwrap();
        assert_eq!(tracks[0].source, Some(TrackRef::Index(1)));
        assert_eq!(
            tracks[1].source,
            Some(TrackRef::Path("external.srt".to_string()))
        );
    }
}
