+++
title = "Your First Encode"
description = "A complete walkthrough from source files to finished output."
weight = 3
+++

This page walks through a complete encode from start to finish using a specific example so there's something concrete to follow. Everything is prescribed. For the general process of inspecting your own source files and building a config from scratch, continue to [Building a Configuration](@/getting-started/building-a-configuration.md).

## The scenario

For this example, imagine a directory of MKV files named like `Cowboy Bebop S01E01 [BD 1080p].mkv` — a Blu-ray rip with a standard two-track subtitle layout. The source streams are:

- **Video:** H.264, 1080p, progressive — no preprocessing needed.
- **Audio track 1:** Japanese, AAC stereo.
- **Audio track 2:** English dub, AAC stereo.
- **Subtitle track 1:** English ASS — the full track, combining dialogue and signs.
- **Subtitle track 2:** English ASS — signs and titles only.

Your files will have different names and very likely a different stream layout. That's expected — the goal here is to see how the pieces fit together, not to follow this example literally.

The goal: MP4 files in an `encoded/` subdirectory, named `Cowboy Bebop - S01E01.mp4`. Both audio tracks, Japanese as default. A soft SRT of the spoken dialogue (derived by removing sign timestamps from the full track). Signs burned onto the video.

## 1. Check your environment

```sh
bento check
```

Confirms `ffmpeg` is present and generates your global config at `~/.config/bento/config.toml` (Linux/macOS) or `%APPDATA%\bento\config.toml` (Windows) if it doesn't exist yet. Open that file and set your personal defaults — encoder, CRF, audio codec. The built-in defaults are reasonable and will work for this example as-is. You'll rarely need to edit the global config again after the first setup.

## 2. Write the directory config

Create `bento.toml` in the source directory, alongside the MKV files. Here's what goes in each section:

```toml
[output]
container = "mp4"
destination = "encoded"
metadata = { show = "Cowboy Bebop", season = 1, year = 1998 }
naming = {
    regex = 'S(?<s>\d+)E(?<episode>\d+)',
    template = "{show} - S{s:02}E{episode:02}",
}
```

`[output]` writes MP4 files to an `encoded/` subdirectory and embeds show, season, and year as container metadata. The `naming` block extracts values from each source filename using the regex and uses them to build the output filename. The `metadata` values (`show`, `season`, `year`) and the `naming` pattern are specific to this show and its filename convention — you'll replace these with your own show details and a regex that matches your filenames.

```toml
[audio]
tracks = [
    { source = 1, lang = "jpn", title = "Japanese", default = true, original = true },
    { source = 2, lang = "eng", title = "English Dub" },
]
```

`[audio]` keeps both tracks. `source = 1` and `source = 2` refer to the first and second audio streams in the file. The source indices, language codes, and titles all come from inspecting the source file — yours will differ. Both tracks here are already AAC stereo, which matches the global config defaults, so Bento copies them rather than re-encoding.

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

`[subtitles]` derives a spoken-dialogue SRT by removing any events whose timestamps exactly match an event in the signs track (`subtract_track = 2`), then soft-muxes it into the container. The signs track is burned onto the video stream via libass. This two-track layout is common for Blu-ray releases, but your source may be structured differently — [Building a Configuration](@/getting-started/building-a-configuration.md) covers the other common patterns.

Video encoding settings — encoder, CRF, preset — are not set here and fall through to the global config.

The full `bento.toml`:

```toml
[output]
container = "mp4"
destination = "encoded"
metadata = { show = "Cowboy Bebop", season = 1, year = 1998 }
naming = {
    regex = 'S(?<s>\d+)E(?<episode>\d+)',
    template = "{show} - S{s:02}E{episode:02}",
}

[audio]
tracks = [
    { source = 1, lang = "jpn", title = "Japanese", default = true, original = true },
    { source = 2, lang = "eng", title = "English Dub" },
]

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

## 3. Check what Bento resolved

Before encoding, verify the resolved config:

```sh
bento config ./
```

This prints every resolved setting for each file, annotated with which layer it came from. If a field you expected from the global config looks wrong, or a track index seems off, this is where you catch it.

## 4. Preview the encode plan

```sh
bento convert --dry-run ./
```

`--dry-run` probes the source files and prints the full encode plan — audio copy-vs.-transcode decisions, the subtitle derivation chain, each output path — without writing anything. For a multi-file batch, a quick scan before a long run is worth it.

## 5. Encode

```sh
bento convert ./
```

Bento processes the files sequentially, showing progress per file. When the batch finishes, it prints a summary listing how many files succeeded and any that failed. Output lands in `encoded/` next to your source files.
