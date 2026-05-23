+++
title = "Getting Started"
description = "From zero to your first encode in a few steps."
weight = 3
+++

## 1. Check your environment

```sh
bento check
```

This verifies `ffmpeg` is present, checks its version, and generates your global config at `~/.config/bento/config.toml` if it doesn't exist yet. Open that file and set your personal defaults — preferred encoder, CRF, audio bitrate, etc. You'll rarely need to touch it again.

## 2. Find your track indices

Before writing a config, you need to know which tracks in your source file are audio and which are subtitles. `ffprobe` (bundled with `ffmpeg`) can tell you:

```sh
ffprobe -v quiet -print_format json -show_streams episode01.mkv | \
  grep -E '"index"|"codec_type"|"codec_name"|"tags"' | head -60
```

Or for a quicker summary:

```sh
ffprobe -v error -show_entries stream=index,codec_type,codec_name \
  -of default=noprint_wrappers=1 episode01.mkv
```

Note the `index` values for the audio and subtitle streams you want.

## 3. Drop a `bento.toml` next to your show

Create `bento.toml` in the same directory as your video files. The minimum useful config sets the audio and subtitle track lists (these have no universal default) and some output options:

```toml
[output]
container = "mp4"
destination = "encoded"
metadata = { show = "Cowboy Bebop", season = 1, year = 1998 }
naming = {
    regex = 'S(?P<s>\d+)E(?P<episode>\d+)',
    template = "{show} - S{s:02}E{episode:02}",
}

[audio]
tracks = [
    { source = 1, lang = "jpn", title = "Japanese", default = true, original = true },
    { source = 2, lang = "eng", title = "English Dub" },
]

[subtitles]
tracks = [
    { source = 1, format = "srt", mux = "soft", subtract_track = 2, lang = "eng", title = "English", default = true },
    { source = 2, format = "ass", mux = "burn" },
]
```

This config:

- Outputs MP4 files to an `encoded/` subdirectory.
- Extracts the show/season/year from the `metadata` table and the episode number from each filename.
- Keeps the Japanese audio track (as default) and the English dub.
- Produces a spoken-dialogue SRT track by subtracting the signs track (track 2) from the full subtitle track (track 1), and burns the signs in on the video.

Video encoding settings (encoder, CRF, preset, etc.) fall through to your global config defaults.

## 4. Preview the plan

Before committing to a full encode, check what Bento resolved:

```sh
# Show resolved settings for every file in the directory, with per-field provenance
bento config ./

# Show the full encode plan — what would actually happen, including copy vs. transcode decisions
bento convert --dry-run ./
```

`bento config` tells you where each setting came from (global config, directory config, built-in default). `--dry-run` goes further and probes the source files to make copy-vs.-transcode decisions, then prints the full plan without touching the filesystem.

## 5. Run

```sh
bento convert ./
```

Bento processes files sequentially and prints a summary when the batch is complete. Output files land in `encoded/` next to your source files (per `destination = "encoded"` above).

## What's next

- Browse the [Configuration reference](/configuration) for all available settings.
- See the [CLI reference](/cli) for flags, subcommands, and the `--set` override mechanism.
- Use `bento repair` if you upgrade Bento and want to populate newly-added config fields into your global config.
