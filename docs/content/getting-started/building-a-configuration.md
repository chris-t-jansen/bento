+++
title = "Building a Configuration"
description = "How to inspect your source files and write a bento.toml for your own shows."
weight = 4
+++

[Your First Encode](@/getting-started/encoding-example.md) used a fixed scenario with prescribed settings. This page covers how to build a `bento.toml` for your own files: inspect the source, identify what you have, and translate that into config.

## Inspect the source file

Start by seeing what's in your source file with `bento probe`:

```sh
bento probe episode01.mkv
```

Example output from a typical anime MKV:

```
  episode01.mkv   23:40

  Video
    H.264   1920 × 1080   23.976 fps

  Audio   2 tracks
     1   jpn   AAC   stereo   "Japanese"
     2   eng   AAC   stereo   "English"

  Subtitles   2 tracks
     1   eng   ASS   "Full Subtitles"
     2   eng   ASS   "Signs"

  (Track numbers correspond to source = N in your bento.toml.)
```

The numbers shown in the left column are exactly what you write for `source =` in your `[audio]` and `[subtitles]` config. Audio and subtitle tracks are numbered independently, both starting at 1.

Run `bento probe` on a few different episodes before writing the config — stream order occasionally varies between episodes in the same release. See the [`bento probe` reference](@/cli/probe.md) for details on what each column means.

## Video: do you need preprocessing?

Most modern Blu-ray rips need no preprocessing. Start with nothing and add only what the source requires.

**Black bars (crop):** If the source has letterbox or pillarbox bars, set `crop`. You can measure bars by pausing at a non-dark frame, or use `crop = "auto"` to have ffmpeg detect them from a sample of frames — convenient but unreliable on dark scenes. See the [`[video]` reference](@/configuration/video.md) for explicit pixel syntax.

**Interlacing vs. telecine:** Pause the source during fast motion and look for combing artifacts — horizontal lines of alternating rows blending together. If combing appears on *every* frame, the source is interlaced — use `deinterlace`. If combing appears on roughly 2 of every 5 frames in a regular alternating pattern, the source was telecined — use `detelecine`. These are mutually exclusive. Applying the wrong one corrupts the output. The [`[video]` reference](@/configuration/video.md) covers the distinction in detail.

**Era heuristic for anime:** Blu-ray-era anime is almost always progressive 23.976fps and needs no preprocessing. Pre-Blu-ray broadcast and older DVDs are often telecined.

If your source needs none of this, omit the `[video]` section from your `bento.toml` entirely and let everything fall through to the global config.

## Audio: identify and configure tracks

From the `bento probe` output, note how many audio streams there are, their codecs and channel layouts, and which is the original-language track.

For each output audio track you want, write an entry in the `tracks` list with the `source` index, language code, and a title:

```toml
[audio]
tracks = [
    { source = 1, lang = "jpn", title = "Japanese", default = true, original = true },
    { source = 2, lang = "eng", title = "English Dub" },
]
```

Bento decides automatically whether to copy or re-encode each stream. If a track already matches your configured codec and channel layout, Bento copies it without touching quality. If it doesn't match — DTS 5.1 source with a stereo AAC target, for example — Bento transcodes it and applies loudness normalization for the surround-to-stereo downmix. See the [`[audio]` reference](@/configuration/audio.md) for the copy-vs.-transcode decision rules.

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

Converting ASS to SRT (`format = "srt"`) is lossy — styling and positioning are stripped. This is fine for spoken dialogue but not for styled signs. Burn tracks always use ASS internally regardless of the `format` field. See the [`[subtitles]` reference](@/configuration/subtitles.md) for the full set of options.

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

`naming` is optional. Without it, output filenames mirror source filenames with the extension changed to match `container`. With it, `regex` extracts named captures from each source filename (without extension), and `template` uses them to build the output filename. If the regex fails to match a file, that file errors at encode time. See the [`[output]` reference](@/configuration/output.md) for the full template variable list.

## Validate before encoding

Once the config is written, use Bento's built-in tools before committing to a full run:

```sh
# Show every resolved setting and which layer it came from
bento config ./

# Probe source files and print the full encode plan without writing anything
bento convert --dry-run ./
```

`bento config` is fast — it doesn't touch the source files — and catches layer misconfigurations and missing fields immediately. `--dry-run` goes further: it probes each source file and makes copy-vs.-transcode decisions, so it catches track index errors that would only surface mid-encode. On a 20-file batch, a dry-run takes seconds; catching a wrong `source` index before a multi-hour run is worth it.
