+++
title = "Building a Configuration"
description = "How to inspect your source files and write a bento.toml for your own shows."
weight = 4
+++

[Your First Encode](/getting-started/encoding-example) used a fixed scenario with prescribed settings. This page covers how to build a `bento.toml` for your own files: inspect the source, identify what you have, and translate that into config.

## Inspect the source file

Start by seeing what's in your source file. `ffprobe` (bundled with `ffmpeg`) gives you the stream layout:

```sh
ffprobe -v error \
  -show_entries stream=index,codec_type,codec_name,tags=language,tags=title \
  -of default=noprint_wrappers=1 \
  episode01.mkv
```

Example output from a typical anime MKV:

```
index=0
codec_type=video
codec_name=h264
index=1
codec_type=audio
codec_name=aac
tag:language=jpn
tag:TITLE=Japanese
index=2
codec_type=audio
codec_name=aac
tag:language=eng
tag:TITLE=English
index=3
codec_type=subtitle
codec_name=ass
tag:language=eng
tag:TITLE=Full Subtitles
index=4
codec_type=subtitle
codec_name=ass
tag:language=eng
tag:TITLE=Signs
```

**A note on track numbering.** Bento's `source` field is **1-based and type-relative** — it counts streams of a given type in order, starting from 1. The first audio stream is `source = 1`, the second is `source = 2`, and so on. Subtitle streams are numbered the same way, independently. The overall `index` values from ffprobe (0, 1, 2, 3, 4...) are not what you write in `source`.

For the output above:
- `source = 1` (audio) → Japanese, index 1
- `source = 2` (audio) → English, index 2
- `source = 1` (subtitles) → Full Subtitles, index 3
- `source = 2` (subtitles) → Signs, index 4

Run the same command on a few different episodes before writing the config — stream order occasionally varies between episodes in the same release.

## Video: do you need preprocessing?

Most modern Blu-ray rips need no preprocessing. Start with nothing and add only what the source requires.

**Black bars (crop):** If the source has letterbox or pillarbox bars, set `crop`. You can measure bars by pausing at a non-dark frame, or use `crop = "auto"` to have ffmpeg detect them from a sample of frames — convenient but unreliable on dark scenes. See the [`[video]` reference](/configuration/video) for explicit pixel syntax.

**Interlacing vs. telecine:** Pause the source during fast motion and look for combing artifacts — horizontal lines of alternating rows blending together. If combing appears on *every* frame, the source is interlaced — use `deinterlace`. If combing appears on roughly 2 of every 5 frames in a regular alternating pattern, the source was telecined — use `detelecine`. These are mutually exclusive. Applying the wrong one corrupts the output. The [`[video]` reference](/configuration/video) covers the distinction in detail.

**Era heuristic for anime:** Blu-ray-era anime is almost always progressive 23.976fps and needs no preprocessing. Pre-Blu-ray broadcast and older DVDs are often telecined.

If your source needs none of this, omit the `[video]` section from your `bento.toml` entirely and let everything fall through to the global config.

## Audio: identify and configure tracks

From the ffprobe output, note how many audio streams there are, their codecs and channel counts, and which is the original-language track.

For each output audio track you want, write an entry in the `tracks` list with the `source` index, language code, and a title:

```toml
[audio]
tracks = [
    { source = 1, lang = "jpn", title = "Japanese", default = true, original = true },
    { source = 2, lang = "eng", title = "English Dub" },
]
```

Bento decides automatically whether to copy or re-encode each stream. If a track already matches your configured codec and channel layout, Bento copies it without touching quality. If it doesn't match — DTS 5.1 source with a stereo AAC target, for example — Bento transcodes it and applies loudness normalization for the surround-to-stereo downmix. See the [`[audio]` reference](/configuration/audio) for the copy-vs.-transcode decision rules.

Mark exactly one track `default = true`. Without a default track, Jellyfin falls back to its own selection logic.

## Subtitles: identify tracks and choose a strategy

The right subtitle configuration depends on what your source ships. The common patterns:

**Separate full and signs tracks.** The most common Blu-ray layout: one ASS track has all dialogue and signs combined, a second has signs only. Derive spoken dialogue by subtracting the signs track:

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
    { source = 2, format = "ass", mux = "burn" },
]
```

**Style-separated single track.** Some fansub releases ship one ASS track where dialogue and signs are distinguished by ASS style name. Use `filter` to split it:

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

Both tracks derive from the same source. `retain` keeps dialogue for soft-mux; `remove` keeps everything else (signs) for burn-in.

**No subtitles.** Omit the `[subtitles]` section entirely.

Converting ASS to SRT (`format = "srt"`) is lossy — styling and positioning are stripped. This is fine for spoken dialogue but not for styled signs. Burn tracks always use ASS internally regardless of the `format` field. See the [`[subtitles]` reference](/configuration/subtitles) for the full set of options.

## Output: container, naming, destination

```toml
[output]
container = "mp4"
destination = "encoded"
metadata = { show = "My Show", season = 1, year = 2005 }
naming = {
    regex = 'S(?<s>\d+)E(?<episode>\d+)',
    template = "{show} - S{s:02}E{episode:02}",
}
```

`container = "mp4"` maximizes Jellyfin direct-play compatibility. Use `"mkv"` if you need to soft-mux ASS subtitles natively, but MP4 with burned signs and a soft SRT covers the common case and plays on the widest range of clients.

`destination` resolves relative to the source directory. `"encoded"` puts output in an `encoded/` subdirectory next to the source files; Bento creates it if it doesn't exist.

`naming` is optional. Without it, output filenames mirror source filenames with the extension changed to match `container`. With it, `regex` extracts named captures from each source filename (without extension), and `template` uses them to build the output filename. If the regex fails to match a file, that file errors at encode time. See the [`[output]` reference](/configuration/output) for the full template variable list.

## Validate before encoding

Once the config is written, use Bento's built-in tools before committing to a full run:

```sh
# Show every resolved setting and which layer it came from
bento config ./

# Probe source files and print the full encode plan without writing anything
bento convert --dry-run ./
```

`bento config` is fast — it doesn't touch the source files — and catches layer misconfigurations and missing fields immediately. `--dry-run` goes further: it probes each source file and makes copy-vs.-transcode decisions, so it catches track index errors that would only surface mid-encode. On a 20-file batch, a dry-run takes seconds; catching a wrong `source` index before a multi-hour run is worth it.
