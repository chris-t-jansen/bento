+++
title = "Audio"
description = "Codec, bitrate, mixdown, loudness normalization, and per-track configuration."
weight = 4

[extra]
toc_strip_signatures = true
+++

The `[audio]` section configures all audio output. It has section-level fields and a `tracks` list describing each output track. Several section-level fields — `encoder`, `bitrate`, `mixdown`, `force_bitrate`, `force_mixdown` — also act as per-track defaults: every track inherits them unless it overrides them individually.

```
[audio]
# Warnings
warn_no_default  = <bool>

# Section-only fields
normalize_mix    = <bool>

# All-tracks fields
encoder          = "aac" | "opus" | "flac"
bitrate          = <integer>
mixdown          = "stereo" | "5point1" | "mono" | "dpl2"
force_mixdown    = <bool>
force_bitrate    = <bool>

# Track fields
tracks = [
    {
        # Required fields
        source           = <integer>

        # Optional fields
        lang             = <string>
        title            = <string>
        default          = <bool>
        forced           = <bool>
        original         = <bool>
        commentary       = <bool>
        hearing_impaired = <bool>
        visual_impaired  = <bool>

        # Per-track override fields
        encoder          = "aac" | "opus" | "flac"
        bitrate          = <integer>
        mixdown          = "stereo" | "5point1" | "mono" | "dpl2"
        force_bitrate    = <bool>
        force_mixdown    = <bool>
    },
    ...
]
```

## Fields

### `warn_no_default = <bool>` {#warn_no_default}

Warn if no track in the list is marked `default = true`. Default `true`. Without a default track, Jellyfin falls back to its own track-selection logic, which may not match your preference. Applies at the section level only — it cannot be set per-track.

### `normalize_mix = <bool>` {#normalize_mix}

Apply ffmpeg's `loudnorm` filter to surround-to-stereo downmixes. Default `true`. Addresses the quiet-dialogue artifact common in anime surround sources downmixed to stereo. Applies at the section level only — it cannot be set per-track.

### `encoder = <option>` {#encoder}

Sets the target codec for audio output across all tracks. Default `"aac"`. Can be overridden on a per-track basis.

Common values: `"aac"`, `"opus"`, `"flac"`. There is no explicit `"copy"` value — Bento decides whether to copy or transcode based on the rules in [Copy vs. transcode](#copy-vs-transcode).

### `bitrate = <integer>` {#bitrate}

Sets the target bitrate in kbps for transcoded output across all tracks. Default `192`. Can be overridden on a per-track basis. Ignored when Bento copies the stream rather than transcoding.

### `mixdown = <option>` {#mixdown}

Sets the output channel layout across all tracks. Default `"stereo"`. Can be overridden on a per-track basis.

- **`"stereo"`** — 2-channel stereo
- **`"5point1"`** — 5.1 surround
- **`"mono"`** — mono
- **`"dpl2"`** — Dolby Pro Logic II matrix-encoded stereo. Encodes a surround mix into a 2-channel signal that a DPL II decoder can expand back to surround. Useful for players or receivers with Dolby Pro Logic II decoding. Output is 2-channel; ffmpeg applies `aresample=matrix_encoding=dplii` during the transcode.

### `force_mixdown = <bool>` {#force_mixdown}

When `true` (default), transcode if the source channel layout differs from `mixdown`. Setting to `false` makes `mixdown` advisory — it applies if transcoding happens for other reasons, but does not itself trigger a transcode. Can be overridden on a per-track basis.

### `force_bitrate = <bool>` {#force_bitrate}

When `true`, transcode if the source bitrate exceeds the target `bitrate`. Default `false`. Can be overridden on a per-track basis.

Source bitrate below target never triggers a transcode regardless of this flag — there is no benefit to re-encoding lossy audio at a higher bitrate.

### `tracks = [ <track>, ... ]` {#tracks}

A list of output audio tracks. Each entry describes one track.

- **`source = <integer>`**
    - Source audio track index in the input file. 1-based and type-relative — it counts audio streams only, so the first audio track is `source = 1` regardless of its overall stream index. Required.
- **`lang = <string>`**
    - ISO 639 language code (e.g. `"jpn"`, `"eng"`). Strongly recommended for Jellyfin track selection.
- **`title = <string>`**
    - User-facing label (e.g. `"Japanese"`, `"Director's Commentary"`).
- **`default = <bool>`**
    - Auto-play disposition. At most one track may set this; multiple `default = true` is a hard error.
- **`forced = <bool>`**
    - Play this track even when audio is set to a different language.
- **`original = <bool>`**
    - Marks the original-language track.
- **`commentary = <bool>`**
    - Marks the track as commentary.
- **`hearing_impaired = <bool>`**
    - Marks a dialogue-emphasized mix for hard-of-hearing listeners.
- **`visual_impaired = <bool>`**
    - Marks an audio description track for blind or low-vision viewers.
- **`encoder = <option>`**
    - Overrides the section-level setting for this track.
- **`bitrate = <integer>`**
    - Overrides the section-level setting for this track.
- **`mixdown = <option>`**
    - Overrides the section-level setting for this track.
- **`force_bitrate = <bool>`**
    - Overrides the section-level setting for this track.
- **`force_mixdown = <bool>`**
    - Overrides the section-level setting for this track.

Only `default` is uniqueness-enforced. The other dispositions (`forced`, `original`, `commentary`, `hearing_impaired`, `visual_impaired`) are category flags — multiple tracks may set them.

## Behavior

### Copy vs. transcode

Bento decides automatically whether to copy or re-encode each source audio stream. Three conditions can independently trigger transcoding:

1. **Codec mismatch** — always transcodes. If the source codec doesn't match `encoder`, the stream must be re-encoded.
2. **Mixdown mismatch** — transcodes when `force_mixdown = true` (default) and the source channel layout differs from `mixdown`.
3. **Bitrate exceeds target** — transcodes when `force_bitrate = true` and the source bitrate is higher than `bitrate`.

If any condition fires, Bento transcodes using the configured `encoder`, `bitrate`, and `mixdown`. If none fire, Bento copies the stream.

The asymmetric defaults reflect a real asymmetry in intent: `mixdown = "stereo"` usually expresses a hard requirement ("I want stereo output"), while `bitrate = 192` is usually a ceiling rather than a trigger — re-encoding 320kbps AAC to 192kbps saves a modest amount of space at the cost of lossy-to-lossy quality degradation. Users who want the bitrate cap can opt in with `force_bitrate = true`.

## Examples

**Global config (defaults only, no tracks):**
```toml
[audio]
encoder = "aac"
bitrate = 192
mixdown = "stereo"
force_bitrate = false
force_mixdown = true
normalize_mix = true
```

**Directory config (track list with section defaults inherited):**
```toml
[audio]
tracks = [
    { source = 1, lang = "jpn", title = "Japanese", default = true, original = true },
    { source = 2, lang = "eng", title = "English Dub" },
    { source = 3, lang = "eng", title = "Director's Commentary", commentary = true, bitrate = 96 },
]
```

`encoder`, `bitrate`, `mixdown`, `force_bitrate`, `force_mixdown`, and `normalize_mix` are inherited from the global config. The commentary track overrides `bitrate` for itself only.
