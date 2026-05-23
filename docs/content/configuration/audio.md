+++
title = "[audio]"
description = "Codec, bitrate, mixdown, loudness normalization, and per-track configuration."
weight = 3
+++

The `[audio]` section configures all audio output. It has two levels: section-level fields that act as per-track defaults, and a `tracks` list that describes each output track. Settings at the section level cascade to every track that doesn't override them individually.

## Section-level fields (per-track defaults)

These fields apply to every track in the `tracks` list unless overridden per-track.

### `encoder`

Target codec for audio output. Default `"aac"`.

Common values: `"aac"`, `"opus"`, `"flac"`. There is no explicit `"copy"` value — Bento decides whether to copy or transcode based on the copy/transcode rules below.

### `bitrate`

Target bitrate in kbps for transcoded output. Default `192`. Ignored when Bento copies the stream rather than transcoding.

### `mixdown`

Output channel layout. Default `"stereo"`.

| Value | Description |
|---|---|
| `"stereo"` | 2-channel stereo |
| `"5point1"` | 5.1 surround |
| `"mono"` | Mono |
| `"dpl2"` | Dolby Pro Logic II (matrix-encoded stereo) |

### `force_mixdown`

When `true` (default), transcode if the source channel layout differs from `mixdown`. Setting to `false` makes `mixdown` advisory — it applies if transcoding happens for other reasons, but does not itself trigger a transcode.

### `force_bitrate`

When `true`, transcode if the source bitrate exceeds the target `bitrate`. Default `false`.

Source bitrate below target never triggers a transcode regardless of this flag — there is no benefit to re-encoding lossy audio at a higher bitrate.

## Section-only fields

These apply at the section level and do not cascade to individual tracks.

### `normalize_mix`

Apply ffmpeg's `loudnorm` filter to surround-to-stereo downmixes. Default `true`. Addresses the quiet-dialogue artifact common in anime surround sources downmixed to stereo.

### `warn_no_default`

Warn if no track in the list is marked `default = true`. Default `true`. Without a default track, Jellyfin falls back to its own track-selection logic, which may not match your preference.

## Per-track fields

Each entry in `tracks` describes one output audio track.

| Field | Required | Description |
|---|---|---|
| `source` | Yes | Source audio track index in the input file. |
| `lang` | No | ISO 639 language code (e.g. `"jpn"`, `"eng"`). Strongly recommended for Jellyfin track selection. |
| `title` | No | User-facing label (e.g. `"Japanese"`, `"Director's Commentary"`). |
| `default` | No | Auto-play disposition. At most one track may set this; multiple `default = true` is a hard error. |
| `forced` | No | Play this track even when audio is set to a different language. |
| `original` | No | Marks the original-language track. |
| `commentary` | No | Marks the track as commentary. |
| `hearing_impaired` | No | Marks a dialogue-emphasized mix for hard-of-hearing listeners. |
| `visual_impaired` | No | Marks an audio description track for blind or low-vision viewers. |
| `encoder` | No | Override section-level `encoder` for this track. |
| `bitrate` | No | Override section-level `bitrate` for this track. |
| `mixdown` | No | Override section-level `mixdown` for this track. |
| `force_bitrate` | No | Override section-level `force_bitrate` for this track. |
| `force_mixdown` | No | Override section-level `force_mixdown` for this track. |

Only `default` is uniqueness-enforced. The other dispositions (`forced`, `original`, `commentary`, `hearing_impaired`, `visual_impaired`) are category flags — multiple tracks may set them.

## Copy vs. transcode

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
