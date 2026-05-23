+++
title = "[subtitles]"
description = "Soft mux, burn-in, external sidecars, ASS style filtering, and track subtraction."
weight = 4
+++

The `[subtitles]` section configures all subtitle output. Each track in the `tracks` list independently specifies its source, how it's derived, and how it appears in the output. Unlike `[audio]`, there are no section-level per-track defaults — every track is fully self-contained.

## Section-only fields

### `warn_multiple_burns`

Warn if more than one track has `mux = "burn"`. Default `true`. Multiple burn layers are supported (e.g. foreign-language hardsubs alongside signs), but the configuration is also a common mistake. Set to `false` to suppress if intentional.

### `warn_burn_metadata`

Warn if any burn track has soft-track metadata fields set (`lang`, `title`, `default`, etc.). Default `true`. Burn tracks are rendered as pixels and have no metadata channel in the output, so these fields have no effect — the warning catches leftover metadata from a track that was switched from soft to burn.

### `warn_no_default`

Warn if no track in the list is marked `default = true`. Default `true`. Symmetric with `[audio].warn_no_default`.

### `warn_ass_to_srt`

Warn when a track is processed as ASS but emitted as SRT. Default `true`. The conversion is lossy — styling, positioning, and effects are stripped to plain text. Legitimate but worth flagging. Set to `false` to suppress if intentional.

## Per-track fields

### Routing

| Field | Required | Description |
|---|---|---|
| `source` | Yes | Source track index (integer) in the input file, or path to an external subtitle file (string). Multiple output tracks can share a source. |
| `format` | No | Output format: `"srt"` or `"ass"`. For soft tracks, this is the muxed stream format. If the input is ASS and `format = "srt"`, the conversion is lossy — styling is stripped. |
| `mux` | No | `"soft"` (mux into container), `"burn"` (render onto video via libass), or `"external"` (write as a sidecar file). |

**File-path sources:** When `source` is a string, Bento reads the subtitle data from that file instead of extracting it from the source video. Paths resolve relative to the config file that contains them, not CWD. Supported extensions: `.srt`, `.ass`, `.ssa`.

### Derivation (mutually exclusive — at most one per track)

| Field | Description |
|---|---|
| `filter` | Keep or drop dialogue events based on ASS style attributes. Valid only when the input is ASS. |
| `subtract_track` | Drop events whose `(start, end)` timestamps exactly match an event in another track. Value is a track index (integer) or file path (string). The "full minus signs = dialogue-only" case. |

**`filter` fields:**

| Field | Description |
|---|---|
| `style` | Match by ASS style name (e.g. `"Main"`, `"Signs"`). |
| `font` | Match by font name. |
| `size` | Match by font size. |
| `mode` | `"retain"` keeps only matching events; `"remove"` drops them. Multiple match keys are AND-ed. |

### Soft-track metadata

These fields apply to soft and external tracks. On burn tracks, they have no effect (burn tracks are pixels, not muxed streams) and trigger `warn_burn_metadata`.

| Field | Description |
|---|---|
| `lang` | ISO 639 language code. |
| `title` | User-facing label. |
| `default` | Auto-display disposition. At most one track may set this; multiple `default = true` is a hard error. |
| `forced` | Show this track even when subtitles are off — for forced-narrative content (foreign-language scenes, on-screen text). |
| `commentary` | Marks the track as commentary. |
| `hearing_impaired` | SDH/CC disposition. |

Only `default` is uniqueness-enforced. The other dispositions are category flags — multiple tracks may set them.

## External subtitle tracks (`mux = "external"`)

External tracks are written as sidecar files next to the output video, targeting Jellyfin's [external subtitle](https://jellyfin.org/docs/general/server/media/shows#external-subtitles-and-audio-tracks) feature. Useful for tracks you want to be able to edit or replace without remuxing, or for formats that MP4 can't carry natively as soft tracks.

**Filename format:** `<output_basename>.<title?>.<lang?>.<flags?>.<ext>`

- `title` — included verbatim if set.
- `lang` — included as the ISO 639 code if set.
- Flags: `default = true` → `default`, `forced = true` → `forced`, `hearing_impaired = true` → `sdh`.
- Extension: `.srt` or `.ass`.

A track with `lang = "eng"`, `title = "English"`, `default = true`, `format = "srt"` against output `episode06.mp4` produces `episode06.English.eng.default.srt`.

The `[output].on_existing` policy applies to sidecar collisions. Sidecar filename uniqueness is validated at config time — two external tracks resolving to the same filename is a hard config error.

## Examples

**Two-track source: signs separate from full dialogue.**
The source ships a full dialogue+signs track (track 1) and a signs-only track (track 2). Bento burns the signs and produces a soft spoken-only SRT by subtracting track 2 from track 1:

```toml
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
```

**Single-track source with style-based split.**
A fansub release ships one ASS track containing both dialogue and signs, distinguished by ASS style name. Bento splits it using complementary filters:

```toml
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
```

Both tracks derive from the same source. `retain` keeps dialogue for soft mux; `remove` keeps everything else (signs) for burn-in.

**Hand-edited dialogue track.**
A typo in the source was fixed by hand and saved as `episode06.dialogue.srt` next to the `bento.toml`. The signs track is still extracted from the MKV:

```toml
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
```

This kind of per-file override is best placed in a `<videofile>.bento.toml` sidecar rather than the directory config.

**External sidecar track for Jellyfin direct access.**
Same two-track source, but the dialogue track is written next to the output video rather than muxed in:

```toml
[subtitles]
tracks = [
    {
        source = 1,
        format = "srt",
        mux = "external",
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
```

If the output resolves to `episode06.mp4`, Bento writes `episode06.English.eng.default.srt` alongside it.
