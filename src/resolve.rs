//! Configuration layer resolution.
//!
//! Resolution takes a stack of user layers (Global, Directory, PerFile, Cli) plus
//! the baked-in Defaults layer and folds them into a single resolved [`Config`]
//! with full per-leaf [`Provenance`] tracking.
//!
//! Rules (from DESIGN.md > Configuration Model > Layering):
//!
//! - **Scalar leaves**: highest layer that sets the field wins.
//! - **Scalar tables** (e.g. `[output.metadata]`, `video.encoder`): merge leaf-by-leaf.
//! - **Lists** (`audio.tracks`, `subtitles.tracks`): wholesale-replace; higher layer
//!   that sets the list shadows the lower layer's list entirely.
//! - **Sum-typed fields** (`crop`, `denoise`, `resolution`): if both layers agree
//!   on the form (e.g. both `Explicit`), leaf-merge inside that form; if they
//!   disagree, the higher form wins outright with no cross-form merging.
//! - **Section-level cascade** (within `[audio]`): section-level fields propagate
//!   into per-track fields where the track didn't set them. Applied after the
//!   cross-layer merge.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::config::*;

// =============================================================================
// Public types
// =============================================================================

/// One layer in the resolution chain. Variants are listed lowest to highest
/// precedence: `Defaults` < `Global` < `Directory` < `PerFile` < `Cli`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Layer {
    Defaults,
    Global(PathBuf),
    Directory(PathBuf),
    PerFile(PathBuf),
    Cli,
}

impl Layer {
    /// Short, human-readable name for the layer kind (used in summary output).
    pub fn kind(&self) -> &'static str {
        match self {
            Layer::Defaults => "defaults",
            Layer::Global(_) => "global",
            Layer::Directory(_) => "directory",
            Layer::PerFile(_) => "per-file",
            Layer::Cli => "cli",
        }
    }

    /// Display string with the source path where applicable.
    pub fn display(&self) -> String {
        match self {
            Layer::Defaults => "defaults (built-in)".to_string(),
            Layer::Global(p) => format!("global     {}", p.display()),
            Layer::Directory(p) => format!("directory  {}", p.display()),
            Layer::PerFile(p) => format!("per-file   {}", p.display()),
            Layer::Cli => "cli".to_string(),
        }
    }
}

/// Result of [`resolve`]: a fully-merged [`Config`] paired with per-leaf
/// provenance. Required fields with no baked-in default may still be `None` —
/// callers (e.g. `bento convert`) decide how to handle that.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub config: Config,
    pub provenance: Provenance,
}

/// Maps dotted leaf paths (e.g. `"video.encoder.crf"`) to the [`Layer`] that
/// provided the resolved value. Track-list elements are recorded as a single
/// path (e.g. `"audio.tracks"`) — list semantics are wholesale-replace.
#[derive(Debug, Clone, Default)]
pub struct Provenance {
    paths: BTreeMap<String, Layer>,
}

impl Provenance {
    pub fn layer_for(&self, path: &str) -> Option<&Layer> {
        self.paths.get(path)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Layer)> {
        self.paths.iter()
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// Number of resolved leaves attributed to each layer kind (for the
    /// default-mode summary line).
    pub fn count_by_kind(&self) -> BTreeMap<&'static str, usize> {
        let mut counts = BTreeMap::new();
        for layer in self.paths.values() {
            *counts.entry(layer.kind()).or_insert(0) += 1;
        }
        counts
    }
}

// =============================================================================
// Resolution entry point
// =============================================================================

/// Resolve a stack of user layers into a single [`Resolved`] config.
///
/// `user_layers` is ordered lowest to highest precedence (e.g.
/// `[Global, Directory, PerFile, Cli]`). Baked-in defaults are added internally
/// at the lowest precedence. Codec-dependent defaults (notably
/// `video.encoder.crf`) are computed from the partially-resolved encoder name.
pub fn resolve(user_layers: Vec<(Layer, Config)>) -> Resolved {
    let mut config = Config::default();
    let mut provenance = Provenance::default();

    // Step 1: merge user layers low-to-high. Each layer overlays the previous,
    // overwriting any field it explicitly sets.
    for (layer, layer_config) in &user_layers {
        record_set_paths(layer_config, layer, &mut provenance);
        config.merge_higher(layer_config.clone());
    }

    // Step 2: baked-in defaults at lowest precedence. Codec-aware: CRF default
    // depends on the resolved encoder name.
    let resolved_encoder_name = config.video.encoder.as_ref().and_then(|e| e.name);
    let defaults = baked_defaults(resolved_encoder_name);
    apply_defaults(&mut config, &mut provenance, &defaults);

    // Step 3: section-level cascade. Audio section-level fields populate per-track
    // fields where the track didn't override.
    apply_section_cascade(&mut config);

    Resolved { config, provenance }
}

// =============================================================================
// Provenance recording (via TOML serialization round-trip)
// =============================================================================

fn record_set_paths(config: &Config, layer: &Layer, provenance: &mut Provenance) {
    let value = toml::Value::try_from(config).expect("Config is always serializable");
    walk_leaves(&value, String::new(), &mut |path| {
        provenance.paths.insert(path.to_string(), layer.clone());
    });
}

fn apply_defaults(config: &mut Config, provenance: &mut Provenance, defaults: &Config) {
    // The codec-aware defaults are recorded for any path the user layers did not
    // already provide. `or_insert` preserves user-layer entries.
    let value = toml::Value::try_from(defaults).expect("Config is always serializable");
    walk_leaves(&value, String::new(), &mut |path| {
        provenance
            .paths
            .entry(path.to_string())
            .or_insert(Layer::Defaults);
    });

    // Then merge defaults into config at lowest priority (fills in unset fields
    // without overwriting any user-set value).
    let mut combined = defaults.clone();
    combined.merge_higher(config.clone());
    *config = combined;
}

fn walk_leaves(value: &toml::Value, prefix: String, on_leaf: &mut impl FnMut(&str)) {
    match value {
        toml::Value::Table(table) => {
            for (k, v) in table {
                let path = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{}.{}", prefix, k)
                };
                walk_leaves(v, path, on_leaf);
            }
        }
        // Arrays (incl. arrays-of-tables) are leaves: they wholesale-replace and
        // share one provenance entry per the schema's list rule.
        _ => on_leaf(&prefix),
    }
}

// =============================================================================
// Section-level cascade
// =============================================================================

fn apply_section_cascade(config: &mut Config) {
    // Audio: section-level fields populate per-track fields where unset.
    let section_encoder = config.audio.encoder.clone();
    let section_bitrate = config.audio.bitrate;
    let section_mixdown = config.audio.mixdown;
    let section_force_bitrate = config.audio.force_bitrate;
    let section_force_mixdown = config.audio.force_mixdown;

    if let Some(tracks) = config.audio.tracks.as_mut() {
        for track in tracks {
            if track.encoder.is_none() {
                track.encoder = section_encoder.clone();
            }
            if track.bitrate.is_none() {
                track.bitrate = section_bitrate;
            }
            if track.mixdown.is_none() {
                track.mixdown = section_mixdown;
            }
            if track.force_bitrate.is_none() {
                track.force_bitrate = section_force_bitrate;
            }
            if track.force_mixdown.is_none() {
                track.force_mixdown = section_force_mixdown;
            }
        }
    }
    // Subtitles section has no per-track cascade — section-level fields are
    // warning toggles only, not per-track concepts (DESIGN.md > [subtitles]).
}

// =============================================================================
// Baked-in defaults (codec-aware)
// =============================================================================

/// Construct the baked-in defaults Config. CRF is selected per the resolved
/// encoder name (x264 → 20, x265 → 22, per DESIGN.md > [video] > encoder).
fn baked_defaults(resolved_encoder_name: Option<EncoderName>) -> Config {
    let crf = match resolved_encoder_name.unwrap_or(EncoderName::X264) {
        EncoderName::X264 => 20,
        EncoderName::X265 => 22,
    };

    Config {
        output: Output {
            container: Some(Container::Mp4),
            destination: Some(".".to_string()),
            preserve_chapters: Some(true),
            on_existing: Some(OnExisting::Warn),
            metadata: None,
            naming: None,
        },
        video: Video {
            encoder: Some(Encoder {
                name: Some(EncoderName::X264),
                crf: Some(crf),
                tune: Some(Tune::Animation),
            }),
            preset: Some(Preset::Medium),
            crop: Some(Crop::Mode(CropMode::None)),
            deinterlace: Some(DeinterlaceMode::None),
            detelecine: Some(DetelecineMode::None),
            denoise: Some(Denoise::Off(DenoiseOff::None)),
            resolution: Some(Resolution::Mode(ResolutionMode::Original)),
            never_upscale: Some(true),
            warn_crf_codec_mismatch: Some(true),
        },
        audio: Audio {
            encoder: Some("aac".to_string()),
            bitrate: Some(192),
            mixdown: Some(Mixdown::Stereo),
            force_bitrate: Some(false),
            force_mixdown: Some(true),
            normalize_mix: Some(true),
            warn_no_default: Some(true),
            tracks: None,
        },
        subtitles: Subtitles {
            warn_multiple_burns: Some(true),
            warn_burn_metadata: Some(true),
            warn_no_default: Some(true),
            warn_ass_to_srt: Some(true),
            tracks: None,
        },
    }
}

// =============================================================================
// Merge trait + per-struct impls
// =============================================================================

/// Overlay one layer on top of another. `self` is the lower layer (mutated in
/// place); `higher` provides values that overwrite or fill into `self`.
trait Merge {
    fn merge_higher(&mut self, higher: Self);
}

impl Merge for Config {
    fn merge_higher(&mut self, higher: Self) {
        self.output.merge_higher(higher.output);
        self.video.merge_higher(higher.video);
        self.audio.merge_higher(higher.audio);
        self.subtitles.merge_higher(higher.subtitles);
    }
}

impl Merge for Output {
    fn merge_higher(&mut self, higher: Self) {
        replace_if_some(&mut self.container, higher.container);
        replace_if_some(&mut self.destination, higher.destination);
        replace_if_some(&mut self.preserve_chapters, higher.preserve_chapters);
        replace_if_some(&mut self.on_existing, higher.on_existing);
        merge_inner(&mut self.metadata, higher.metadata);
        merge_inner(&mut self.naming, higher.naming);
    }
}

impl Merge for Metadata {
    fn merge_higher(&mut self, higher: Self) {
        replace_if_some(&mut self.show, higher.show);
        replace_if_some(&mut self.season, higher.season);
        replace_if_some(&mut self.year, higher.year);
    }
}

impl Merge for Naming {
    fn merge_higher(&mut self, higher: Self) {
        replace_if_some(&mut self.regex, higher.regex);
        replace_if_some(&mut self.template, higher.template);
    }
}

impl Merge for Video {
    fn merge_higher(&mut self, higher: Self) {
        merge_inner(&mut self.encoder, higher.encoder);
        replace_if_some(&mut self.preset, higher.preset);
        merge_inner(&mut self.crop, higher.crop);
        replace_if_some(&mut self.deinterlace, higher.deinterlace);
        replace_if_some(&mut self.detelecine, higher.detelecine);
        merge_inner(&mut self.denoise, higher.denoise);
        merge_inner(&mut self.resolution, higher.resolution);
        replace_if_some(&mut self.never_upscale, higher.never_upscale);
        replace_if_some(
            &mut self.warn_crf_codec_mismatch,
            higher.warn_crf_codec_mismatch,
        );
    }
}

impl Merge for Encoder {
    fn merge_higher(&mut self, higher: Self) {
        replace_if_some(&mut self.name, higher.name);
        replace_if_some(&mut self.crf, higher.crf);
        replace_if_some(&mut self.tune, higher.tune);
    }
}

impl Merge for Crop {
    fn merge_higher(&mut self, higher: Self) {
        match (&mut *self, higher) {
            (Crop::Explicit(s), Crop::Explicit(h)) => s.merge_higher(h),
            (_, h) => *self = h,
        }
    }
}

impl Merge for CropPixels {
    fn merge_higher(&mut self, higher: Self) {
        replace_if_some(&mut self.top, higher.top);
        replace_if_some(&mut self.bottom, higher.bottom);
        replace_if_some(&mut self.left, higher.left);
        replace_if_some(&mut self.right, higher.right);
    }
}

impl Merge for Denoise {
    fn merge_higher(&mut self, higher: Self) {
        match (&mut *self, higher) {
            (Denoise::Active(s), Denoise::Active(h)) => s.merge_higher(h),
            (_, h) => *self = h,
        }
    }
}

impl Merge for DenoiseConfig {
    fn merge_higher(&mut self, higher: Self) {
        replace_if_some(&mut self.filter, higher.filter);
        replace_if_some(&mut self.preset, higher.preset);
    }
}

impl Merge for Resolution {
    fn merge_higher(&mut self, higher: Self) {
        match (&mut *self, higher) {
            (Resolution::Explicit(s), Resolution::Explicit(h)) => s.merge_higher(h),
            (_, h) => *self = h,
        }
    }
}

impl Merge for ResolutionExplicit {
    fn merge_higher(&mut self, higher: Self) {
        replace_if_some(&mut self.width, higher.width);
        replace_if_some(&mut self.height, higher.height);
    }
}

impl Merge for Audio {
    fn merge_higher(&mut self, higher: Self) {
        replace_if_some(&mut self.encoder, higher.encoder);
        replace_if_some(&mut self.bitrate, higher.bitrate);
        replace_if_some(&mut self.mixdown, higher.mixdown);
        replace_if_some(&mut self.force_bitrate, higher.force_bitrate);
        replace_if_some(&mut self.force_mixdown, higher.force_mixdown);
        replace_if_some(&mut self.normalize_mix, higher.normalize_mix);
        replace_if_some(&mut self.warn_no_default, higher.warn_no_default);
        // Lists wholesale-replace: any higher-layer list shadows the lower list.
        replace_if_some(&mut self.tracks, higher.tracks);
    }
}

impl Merge for Subtitles {
    fn merge_higher(&mut self, higher: Self) {
        replace_if_some(&mut self.warn_multiple_burns, higher.warn_multiple_burns);
        replace_if_some(&mut self.warn_burn_metadata, higher.warn_burn_metadata);
        replace_if_some(&mut self.warn_no_default, higher.warn_no_default);
        replace_if_some(&mut self.warn_ass_to_srt, higher.warn_ass_to_srt);
        replace_if_some(&mut self.tracks, higher.tracks);
    }
}

fn replace_if_some<T>(self_opt: &mut Option<T>, higher: Option<T>) {
    if higher.is_some() {
        *self_opt = higher;
    }
}

fn merge_inner<T: Merge>(self_opt: &mut Option<T>, higher: Option<T>) {
    match (self_opt.as_mut(), higher) {
        (Some(s), Some(h)) => s.merge_higher(h),
        (None, h @ Some(_)) => *self_opt = h,
        _ => {}
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn parse(s: &str) -> Config {
        Config::from_toml_str(s).expect("config parses")
    }

    fn global() -> Layer {
        Layer::Global(PathBuf::from("/global/config.toml"))
    }
    fn directory() -> Layer {
        Layer::Directory(PathBuf::from("/show/bento.toml"))
    }
    fn per_file() -> Layer {
        Layer::PerFile(PathBuf::from("/show/episode06.mkv.bento.toml"))
    }

    // --- Defaults -----------------------------------------------------------

    #[test]
    fn defaults_only_resolves_to_baked_values() {
        let r = resolve(vec![]);
        assert_eq!(r.config.output.container, Some(Container::Mp4));
        assert_eq!(r.config.output.destination.as_deref(), Some("."));
        assert_eq!(r.config.output.preserve_chapters, Some(true));
        assert_eq!(r.config.output.on_existing, Some(OnExisting::Warn));

        let enc = r.config.video.encoder.expect("encoder defaulted");
        assert_eq!(enc.name, Some(EncoderName::X264));
        assert_eq!(enc.crf, Some(20));
        assert_eq!(enc.tune, Some(Tune::Animation));
        assert_eq!(r.config.video.preset, Some(Preset::Medium));
        assert_eq!(r.config.video.never_upscale, Some(true));

        assert_eq!(r.config.audio.encoder.as_deref(), Some("aac"));
        assert_eq!(r.config.audio.bitrate, Some(192));
        assert_eq!(r.config.audio.mixdown, Some(Mixdown::Stereo));
        assert_eq!(r.config.audio.tracks, None);
    }

    #[test]
    fn defaults_provenance_is_defaults() {
        let r = resolve(vec![]);
        assert_eq!(
            r.provenance.layer_for("output.container"),
            Some(&Layer::Defaults)
        );
        assert_eq!(
            r.provenance.layer_for("video.encoder.crf"),
            Some(&Layer::Defaults)
        );
    }

    // --- Codec-aware CRF default --------------------------------------------

    #[test]
    fn x265_gets_crf_22_when_user_only_sets_name() {
        let user = parse(
            r#"
[video]
encoder = { name = "x265" }
"#,
        );
        let r = resolve(vec![(directory(), user)]);
        let enc = r.config.video.encoder.expect("encoder present");
        assert_eq!(enc.name, Some(EncoderName::X265));
        assert_eq!(enc.crf, Some(22), "CRF should default to 22 for x265");
        // Provenance: name from directory, crf from defaults.
        assert_eq!(
            r.provenance.layer_for("video.encoder.name"),
            Some(&directory())
        );
        assert_eq!(
            r.provenance.layer_for("video.encoder.crf"),
            Some(&Layer::Defaults)
        );
    }

    #[test]
    fn x264_gets_crf_20_default() {
        let user = parse(
            r#"
[video]
encoder = { name = "x264" }
"#,
        );
        let r = resolve(vec![(directory(), user)]);
        assert_eq!(r.config.video.encoder.unwrap().crf, Some(20));
    }

    // --- Scalar leaf merge --------------------------------------------------

    #[test]
    fn higher_layer_wins_for_scalar() {
        let g = parse(
            r#"
[output]
container = "mp4"
"#,
        );
        let d = parse(
            r#"
[output]
container = "mkv"
"#,
        );
        let r = resolve(vec![(global(), g), (directory(), d)]);
        assert_eq!(r.config.output.container, Some(Container::Mkv));
        assert_eq!(
            r.provenance.layer_for("output.container"),
            Some(&directory())
        );
    }

    #[test]
    fn nested_table_leaf_merge_independent_fields() {
        // Global sets full encoder; directory only sets crf. After merge:
        // name, tune from global; crf from directory.
        let g = parse(
            r#"
[video]
encoder = { name = "x264", crf = 18, tune = "film" }
"#,
        );
        let d = parse(
            r#"
[video]
encoder = { crf = 22 }
"#,
        );
        let r = resolve(vec![(global(), g), (directory(), d)]);
        let enc = r.config.video.encoder.unwrap();
        assert_eq!(enc.name, Some(EncoderName::X264));
        assert_eq!(enc.crf, Some(22));
        assert_eq!(enc.tune, Some(Tune::Film));
        assert_eq!(
            r.provenance.layer_for("video.encoder.name"),
            Some(&global())
        );
        assert_eq!(
            r.provenance.layer_for("video.encoder.crf"),
            Some(&directory())
        );
        assert_eq!(
            r.provenance.layer_for("video.encoder.tune"),
            Some(&global())
        );
    }

    // --- Sum-typed merge ----------------------------------------------------

    #[test]
    fn sum_typed_within_form_leaf_merges() {
        // Both layers use Crop::Explicit form — leaf merge expected.
        let g = parse(
            r#"
[video]
crop = { left = 138, right = 138 }
"#,
        );
        let d = parse(
            r#"
[video]
crop = { top = 60, bottom = 60 }
"#,
        );
        let r = resolve(vec![(global(), g), (directory(), d)]);
        match r.config.video.crop.unwrap() {
            Crop::Explicit(p) => {
                assert_eq!(p.top, Some(60));
                assert_eq!(p.bottom, Some(60));
                assert_eq!(p.left, Some(138));
                assert_eq!(p.right, Some(138));
            }
            other => panic!("expected explicit crop, got {:?}", other),
        }
    }

    #[test]
    fn sum_typed_form_disagreement_higher_wins_wholesale() {
        // Global is Explicit; directory is Mode("none") — directory wins,
        // explicit pixels are not merged in.
        let g = parse(
            r#"
[video]
crop = { top = 60, bottom = 60 }
"#,
        );
        let d = parse(
            r#"
[video]
crop = "none"
"#,
        );
        let r = resolve(vec![(global(), g), (directory(), d)]);
        assert_eq!(r.config.video.crop, Some(Crop::Mode(CropMode::None)));
    }

    // --- List wholesale-replace --------------------------------------------

    #[test]
    fn list_wholesale_replaces() {
        let g = parse(
            r#"
[audio]
tracks = [
    { source = 1, lang = "jpn" },
    { source = 2, lang = "eng" },
]
"#,
        );
        let d = parse(
            r#"
[audio]
tracks = [
    { source = 5, lang = "fra" },
]
"#,
        );
        let r = resolve(vec![(global(), g), (directory(), d)]);
        let tracks = r.config.audio.tracks.unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].source, Some(5));
        assert_eq!(tracks[0].lang.as_deref(), Some("fra"));
        assert_eq!(r.provenance.layer_for("audio.tracks"), Some(&directory()));
    }

    // --- Section-level cascade ---------------------------------------------

    #[test]
    fn audio_section_cascade_fills_per_track_defaults() {
        // Global supplies section-level encoder/bitrate/mixdown defaults;
        // directory supplies a track list with sparse overrides. After section
        // cascade, each track has all five cascadable fields set.
        let g = parse(
            r#"
[audio]
encoder = "aac"
bitrate = 192
mixdown = "stereo"
"#,
        );
        let d = parse(
            r#"
[audio]
tracks = [
    { source = 1, lang = "jpn", title = "Japanese" },
    { source = 3, lang = "eng", title = "Commentary", commentary = true, bitrate = 96 },
]
"#,
        );
        let r = resolve(vec![(global(), g), (directory(), d)]);
        let tracks = r.config.audio.tracks.unwrap();
        // Track 0: all from section default.
        assert_eq!(tracks[0].encoder.as_deref(), Some("aac"));
        assert_eq!(tracks[0].bitrate, Some(192));
        assert_eq!(tracks[0].mixdown, Some(Mixdown::Stereo));
        // Track 1: bitrate from track override; rest from section default.
        assert_eq!(tracks[1].encoder.as_deref(), Some("aac"));
        assert_eq!(tracks[1].bitrate, Some(96));
        assert_eq!(tracks[1].mixdown, Some(Mixdown::Stereo));
    }

    // --- Provenance summary ------------------------------------------------

    #[test]
    fn provenance_count_by_kind() {
        let d = parse(
            r#"
[output]
destination = "encoded"
metadata = { show = "Cowboy Bebop", season = 1 }
"#,
        );
        let r = resolve(vec![(directory(), d)]);
        let counts = r.provenance.count_by_kind();
        // 3 paths from directory: destination, metadata.show, metadata.season.
        assert_eq!(counts.get("directory"), Some(&3));
        // Everything else from defaults.
        assert!(counts.get("defaults").copied().unwrap_or(0) > 0);
    }

    // --- Layer ordering ----------------------------------------------------

    #[test]
    fn cli_overrides_per_file_overrides_directory_overrides_global() {
        let g = parse(
            r#"[video]
preset = "slow"
"#,
        );
        let d = parse(
            r#"[video]
preset = "medium"
"#,
        );
        let p = parse(
            r#"[video]
preset = "fast"
"#,
        );
        let cli = parse(
            r#"[video]
preset = "veryfast"
"#,
        );
        let r = resolve(vec![
            (global(), g),
            (directory(), d),
            (per_file(), p),
            (Layer::Cli, cli),
        ]);
        assert_eq!(r.config.video.preset, Some(Preset::Veryfast));
        assert_eq!(r.provenance.layer_for("video.preset"), Some(&Layer::Cli));
    }

    // --- Empty layers ------------------------------------------------------

    #[test]
    fn empty_user_layer_does_not_overwrite_or_record_provenance() {
        let g = parse(
            r#"
[video]
preset = "slow"
"#,
        );
        let empty = Config::default();
        let r = resolve(vec![(global(), g), (directory(), empty)]);
        assert_eq!(r.config.video.preset, Some(Preset::Slow));
        assert_eq!(r.provenance.layer_for("video.preset"), Some(&global()));
    }
}
