+++
title = "bento probe"
description = "Inspect a video file's streams in Bento-native terms."
weight = 5
+++

```
bento probe <path>
```

Displays stream information for a video file — codec, resolution, framerate, language, channel layout, bitrate, and track titles — using the same terminology Bento uses internally. Designed to answer "what's in this file, and how do I reference it in my config?"

## What it shows

Output is divided into three sections: **Video**, **Audio**, and **Subtitles**.

**Video** shows the codec (e.g. `H.264`, `H.265 (HEVC)`), resolution, and framerate.

**Audio** lists every audio track with its track number, language code, codec, channel layout, bitrate (when available), and title (when present).

**Subtitles** lists every subtitle track with its track number, language code, format (`ASS`, `SRT`, `PGS`, etc.), and title (when present).

All track numbers are **1-based and type-relative** — audio tracks are numbered separately from subtitle tracks, starting at 1 each. These numbers map directly to `source =` in your `bento.toml`.

## Example

```
$ bento probe "Cowboy Bebop - S01E01 [BD 1080p].mkv"

  Cowboy Bebop - S01E01 [BD 1080p].mkv   24:32

  Video
    H.264   1920 × 1080   23.976 fps

  Audio   3 tracks
     1   jpn   FLAC   stereo              "Japanese (Lossless)"
     2   jpn   AAC    stereo   192 kbps   "Japanese"
     3   eng   AAC    stereo   192 kbps   "English Dub"

  Subtitles   2 tracks
     1   eng   ASS   "Full Subtitles"
     2   eng   ASS   "Signs & Songs"

  (Track numbers correspond to source = N in your bento.toml.)
```

## Track numbers and `source =`

The track numbers shown are the values to use for `source =` in your `[audio]` and `[subtitles]` config blocks. They count audio tracks and subtitle tracks independently — audio track 2 and subtitle track 2 are different streams, both referenced as `source = 2` in their respective sections.

For the example above, a typical directory config would look like:

```toml
[audio]
tracks = [
    { source = 2, lang = "jpn", title = "Japanese", default = true, original = true },
    { source = 3, lang = "eng", title = "English Dub" },
]

[subtitles]
tracks = [
    { source = 1, format = "srt", mux = "soft", subtract_track = 2,
      lang = "eng", title = "English", default = true },
    { source = 2, format = "ass", mux = "burn" },
]
```

## When to run

- **Before writing a directory config** — identify what tracks are available and what their source indices are.
- **When a source file behaves unexpectedly** — confirm the actual stream layout matches what your config expects.
- **When inheriting someone else's config** — verify the track numbers still match the files you're working with.
