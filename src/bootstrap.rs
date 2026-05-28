//! First-run global config bootstrap.
//!
//! On first run (via `bento check`, or implicitly the first time another
//! subcommand needs the global config), Bento writes `<XDG>/bento/config.toml`
//! containing every field with a baked-in default, written out and commented
//! with documentation. Users edit it once, then mostly leave it alone.
//!
//! Bento does **not** auto-update this file on future upgrades — once it
//! exists, the user owns it. `bento repair` (deferred) handles inserting any
//! new keys Bento has gained without disturbing user edits.
//!
//! The template below should round-trip: parsing it produces a [`Config`]
//! whose values exactly match [`crate::resolve::baked_defaults()`] when no
//! user layers are present. The `template_resolves_equivalent_to_defaults`
//! test enforces this.

use std::path::Path;

use crate::error::{Error, Result};

const GLOBAL_CONFIG_TEMPLATE: &str = r#"# Bento global config
#
# Your personal defaults: preferred CRF, container, audio bitrate, language,
# etc. Override per show in <show>/bento.toml, per file in
# <videofile>.bento.toml, or per invocation via CLI flags.
#
# Bento does NOT auto-update this file on future upgrades. Once it exists,
# it's yours; run `bento repair` to insert any new fields Bento has added
# without disturbing your edits.


# =============================================================================
# Output: container, file location, conflict resolution
# =============================================================================

[output]

# Output container.
#   "mp4" — maximizes Jellyfin direct-play compatibility on Pi clients.
#   "mkv" — supports more subtitle formats natively (e.g. soft ASS) but
#           narrows the compatible-client set.
container = "mp4"

# Where output files are written. "." writes alongside the source. Relative
# paths resolve against the source file's directory; absolute paths are
# absolute. Bento creates the directory if it doesn't exist.
destination = "."

# Copy chapter markers from source to output.
preserve_chapters = true

# What to do when the target output file already exists.
#   "warn"          — print a warning, leave existing in place (default)
#   "skip_silently" — leave existing, no warning
#   "overwrite"     — replace without warning
#   "fail"          — stop the whole run
on_existing = "warn"

# Embedded metadata varies per show — set in <show>/bento.toml:
#
#   [output.metadata]
#   show = "Cowboy Bebop"
#   season = 1
#   year = 1998

# Output filename templating varies per show — set in <show>/bento.toml.
# See [output.naming] in the design doc.


# =============================================================================
# Video: single video stream in, single video stream out
# =============================================================================

[video]

# Speed/quality tradeoff. Slower presets give marginally smaller files at the
# same CRF, but the gains shrink quickly (`veryslow` ≈ 8-10× the encode time
# of `medium` for ~5% smaller files).
#   ultrafast superfast veryfast faster fast medium slow slower veryslow placebo
preset = "medium"

# Black bar removal.
#   "none"                                            — no crop (default)
#   "auto"                                            — autodetect (unreliable on dark scenes)
#   { top = 60, bottom = 60, left = 0, right = 0 }    — explicit pixels
# Any side may be omitted from the explicit form (defaults to 0).
crop = "none"

# Deinterlacing — for content shot interlaced (sports, soap operas, pre-progressive
# broadcasts). DO NOT use on telecined content; use detelecine instead.
#   "none" (default), "auto", "yadif", "bwdif"
deinterlace = "none"

# Inverse telecine — for content broadcast at 30fps via 3:2 pulldown. Most
# pre-Blu-ray anime in NTSC regions falls here.
#   "none" (default), "auto"
detelecine = "none"

# Noise reduction. Generally avoid on clean modern sources; useful for old
# broadcast captures with analog noise.
#   "none" (default)
#   { filter = "nlmeans"|"hqdn3d", preset = "ultralight"|"light"|"medium"|"strong"|"stronger"|"verystrong" }
denoise = "none"

# Output resolution.
#   "original" (default)             — match source dimensions
#   { width = 1280, height = 720 }   — explicit
resolution = "original"

# When true, resolution settings that would enlarge the source are ignored.
# Keep true unless upscaling is genuinely intended (rare).
never_upscale = true

# Warn when resolved CRF and encoder.name look mismatched (the x264 and x265
# CRF scales are NOT interchangeable).
warn_crf_codec_mismatch = true

# --- Encoder choice and CRF/tune ---
#
# These three fields are coupled. Per the schema's leaf-merge rule, you can
# override a single field at a higher layer (e.g. `encoder = { crf = 22 }`
# in a directory config) — the others fall through to this layer.

[video.encoder]

# "x264" maximizes Pi-and-browser direct-play compatibility.
# "x265" produces ~30% smaller files but is ~3-5× slower at equivalent
# presets and narrows the direct-play client set.
name = "x264"

# Constant Rate Factor. Lower = higher quality, larger files.
# Transparent: ~18 for x264, ~20-22 for x265.
# The two scales are NOT interchangeable — change `crf` when changing `name`.
crf = 20

# Psychovisual tune. Valid values depend on `name`:
#   x264: film animation grain stillimage psnr ssim fastdecode zerolatency none
#   x265: animation grain psnr ssim fastdecode zerolatency none
tune = "animation"


# =============================================================================
# Audio: section-level defaults cascade as per-track defaults
# =============================================================================

[audio]

# Target codec for transcoded audio output. There is no explicit "copy"
# value; Bento decides automatically per the source codec / channel layout
# / bitrate vs. the target.
encoder = "aac"

# Target bitrate in kbps for transcoded audio. Ignored when Bento copies
# the source stream.
bitrate = 192

# Channel layout: "stereo" "5point1" "mono" "dpl2"
mixdown = "stereo"

# When true, transcode if source bitrate exceeds `bitrate`. Default false:
# `bitrate` is a target ceiling for transcoding only and never triggers a
# transcode on its own. Re-encoding lossy audio at a higher bitrate is rarely
# worth it.
force_bitrate = false

# When true, transcode if source channel layout differs from `mixdown`.
# Default true: a configured `mixdown` is treated as a hard requirement.
force_mixdown = true

# Apply EBU R128 loudness normalization (loudnorm=I=-16:TP=-1.5:LRA=11) when an
# audio track is folded from surround down to fewer channels (5.1→stereo,
# 5.1→mono). This normalizes the *overall* program loudness of the downmix; it
# is not dialogue-specific, but on surround sources mastered for a different
# listening setup it usually keeps dialogue at a sensible level. Advisory: it
# never forces a transcode on its own, and does nothing unless a qualifying
# downmix actually happens. Overridable per audio track.
normalize_downmix = true

# Warn when no audio track is marked default = true.
warn_no_default = true

# Warn when a track is downmixed from surround but normalization is off
# (normalize_downmix = false) — dialogue may end up quiet.
warn_unnormalized_downmix = true

# Audio tracks (REQUIRED if you want audio output) — set per show in
# <show>/bento.toml:
#
#   [audio]
#   tracks = [
#       { source = 1, lang = "jpn", title = "Japanese", default = true, original = true },
#       { source = 2, lang = "eng", title = "English Dub" },
#   ]


# =============================================================================
# Subtitles: list of output tracks; each derived independently
# =============================================================================

[subtitles]

# Warn when more than one track has mux = "burn".
warn_multiple_burns = true

# Warn when a burn track has soft-only metadata fields (lang/title/etc.) set.
# Burn tracks are pixels and have no metadata channel in the output.
warn_burn_metadata = true

# Warn when no subtitle track is marked default = true.
warn_no_default = true

# Warn on lossy ASS→SRT format conversion (styling/positioning stripped).
warn_ass_to_srt = true

# Subtitle tracks — set per show in <show>/bento.toml:
#
#   [subtitles]
#   tracks = [
#       { source = 1, format = "srt", mux = "soft", subtract_track = 2,
#         lang = "eng", title = "English", default = true },
#       { source = 2, format = "ass", mux = "burn" },
#   ]
"#;

/// Returns the text of a fresh global config file with all baked-in defaults
/// written out and commented with documentation.
pub fn generate_global_config_text() -> &'static str {
    GLOBAL_CONFIG_TEMPLATE
}

/// Write the bootstrap text to `path`, creating the parent directory if needed.
pub fn write_global_config(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    std::fs::write(path, generate_global_config_text()).map_err(|e| Error::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::resolve::{Layer, resolve};
    use std::path::PathBuf;

    #[test]
    fn template_parses_as_valid_toml() {
        let _: Config = Config::from_toml_str(generate_global_config_text())
            .expect("bootstrap template should parse");
    }

    /// The bootstrap, used as the global layer, should produce the same
    /// resolved config as the no-user-layers case (which uses defaults
    /// internally). This catches drift between [`baked_defaults`] and the
    /// template — if you add a baked-in default and forget to update the
    /// template (or vice versa), this test breaks.
    #[test]
    fn template_resolves_equivalent_to_defaults() {
        let config_from_template = Config::from_toml_str(generate_global_config_text()).unwrap();
        let r_from_template = resolve(vec![(
            Layer::Global(PathBuf::from("/test/config.toml")),
            config_from_template,
        )]);
        let r_defaults_only = resolve(vec![]);

        let serialized_template = toml::to_string(&r_from_template.config).unwrap();
        let serialized_defaults = toml::to_string(&r_defaults_only.config).unwrap();
        assert_eq!(
            serialized_template, serialized_defaults,
            "bootstrap template drifted from baked_defaults():\n\
             template:\n{}\n\ndefaults:\n{}",
            serialized_template, serialized_defaults
        );
    }

    #[test]
    fn write_creates_file_and_parents() {
        let dir = std::env::temp_dir().join(format!("bento-bootstrap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("subdir").join("config.toml");

        write_global_config(&path).expect("write should succeed");
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, generate_global_config_text());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
