+++
title = "Introduction"
description = "Bento documentation overview."
weight = 1
+++

Bento is a configuration-driven CLI for converting and re-encoding video files for [Jellyfin](https://jellyfin.org/). It wraps `ffmpeg` in a repeatable, per-show configuration model — write the right encoding settings for a show once, and every subsequent encode picks them up automatically.

## Documentation overview

**[Getting Started](/getting-started)** — Install Bento, follow a complete encoding walkthrough, and learn how to build a `bento.toml` for your own files.

**[Configuration](/configuration)** — Full reference for every setting across the four config sections: `[output]`, `[video]`, `[audio]`, and `[subtitles]`.

**[CLI Reference](/cli)** — Subcommands, flags, the `--set` override mechanism, and verbosity and warning controls.
