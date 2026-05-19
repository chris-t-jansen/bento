//! `[output]` — packaging decisions: container, destination, metadata, naming, conflict handling.

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct Output {
    pub container: Option<Container>,
    pub destination: Option<String>,
    pub preserve_chapters: Option<bool>,
    pub on_existing: Option<OnExisting>,
    pub metadata: Option<Metadata>,
    pub naming: Option<Naming>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    Mp4,
    Mkv,
}

impl Container {
    /// ffmpeg muxer name for `-f <muxer>`.
    pub fn as_ffmpeg_muxer(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Mkv => "matroska",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OnExisting {
    Warn,
    SkipSilently,
    Overwrite,
    Fail,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct Metadata {
    pub show: Option<String>,
    pub season: Option<u32>,
    pub year: Option<i32>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct Naming {
    pub regex: Option<String>,
    pub template: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// The full `[output]` example from the design doc.
    #[test]
    fn parses_full_example() {
        let toml_str = r#"
[output]
container = "mp4"
destination = "encoded"
preserve_chapters = true
on_existing = "warn"
metadata = { show = "Cowboy Bebop", season = 1, year = 1998 }
naming = {
    regex = 'S(?P<s>\d+)E(?P<episode>\d+)',
    template = "{show} - S{s:02}E{episode:02}",
}
"#;
        let cfg = Config::from_toml_str(toml_str).expect("output example parses");
        let out = cfg.output;
        assert_eq!(out.container, Some(Container::Mp4));
        assert_eq!(out.destination.as_deref(), Some("encoded"));
        assert_eq!(out.preserve_chapters, Some(true));
        assert_eq!(out.on_existing, Some(OnExisting::Warn));

        let meta = out.metadata.expect("metadata present");
        assert_eq!(meta.show.as_deref(), Some("Cowboy Bebop"));
        assert_eq!(meta.season, Some(1));
        assert_eq!(meta.year, Some(1998));

        let naming = out.naming.expect("naming present");
        assert_eq!(
            naming.regex.as_deref(),
            Some(r"S(?P<s>\d+)E(?P<episode>\d+)")
        );
        assert_eq!(naming.template.as_deref(), Some("{show} - S{s:02}E{episode:02}"));
    }

    #[test]
    fn all_on_existing_variants_parse() {
        for (s, expected) in [
            ("warn", OnExisting::Warn),
            ("skip_silently", OnExisting::SkipSilently),
            ("overwrite", OnExisting::Overwrite),
            ("fail", OnExisting::Fail),
        ] {
            let toml_str = format!("[output]\non_existing = \"{}\"\n", s);
            let cfg = Config::from_toml_str(&toml_str).expect("on_existing parses");
            assert_eq!(cfg.output.on_existing, Some(expected));
        }
    }

    #[test]
    fn standard_table_form_equivalent_to_inline() {
        let inline = r#"
[output]
metadata = { show = "Cowboy Bebop", season = 1 }
"#;
        let standard = r#"
[output.metadata]
show = "Cowboy Bebop"
season = 1
"#;
        assert_eq!(
            Config::from_toml_str(inline).unwrap().output.metadata,
            Config::from_toml_str(standard).unwrap().output.metadata,
        );
    }
}
