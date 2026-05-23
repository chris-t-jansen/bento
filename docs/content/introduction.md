+++
title = "Introduction"
description = "What Bento is, what it does, and who it's for."
weight = 1
+++

Bento is a configuration-driven CLI for converting and re-encoding video files, primarily designed for anime hosted in [Jellyfin](https://jellyfin.org/). It wraps `ffmpeg` in a repeatable, per-show configuration model so you stop re-deriving the right flags every time.

## The problem it solves

The right `ffmpeg` settings for a given show are stable across episodes but vary significantly between shows — different source containers, audio layouts, subtitle styles, and preprocessing needs. A CLI tool driven entirely by flags forces you to remember (or re-discover) the right combination for each show every time you run it.

Bento moves those settings into a `bento.toml` file that lives alongside the show. You write it once; subsequent encodes pick it up automatically. Settings that are the same across all your shows (preferred encoder, CRF, audio codec) go in a global config you write once and forget.

## What Bento does

Under the hood, Bento drives `ffmpeg` to:

- **Transcode video** to H.264 or H.265 using CRF rate control, with configurable presets and psychovisual tuning.
- **Preprocess** source video as needed: crop black bars, deinterlace, apply inverse telecine (IVTC), denoise, or resize.
- **Mix and re-encode audio tracks** (AAC, Opus, FLAC) with configurable channel mixdown and optional loudness normalization for surround-to-stereo downmixes.
- **Process subtitle tracks**: soft-mux into the container, burn onto the video stream via libass, or write Jellyfin-style external sidecar files. Supports ASS style filtering, timestamp-based track subtraction, and lossy ASS → SRT conversion.
- **Embed container metadata** (show, season, episode, year) derived from filename regex captures, compatible with Jellyfin's scraping.

## Default profile

The built-in defaults target **Jellyfin direct-play on a Raspberry Pi**: H.264 video, AAC stereo audio, signs burned in and spoken dialogue soft-muxed as SRT, MP4 container. Every default is overridable per-show, per-file, or at the command line.

## What Bento is not

Bento is a focused tool for the "encode a season of anime once, store it forever" workflow. It has no GUI, no queue manager, no hardware encoding support, and no watch mode. If you're looking for a general-purpose transcoder with a point-and-click interface, [HandBrake](https://handbrake.fr/) or [StaxRip](https://github.com/staxrip/staxrip) are better fits.

## Why "Bento"?

Each output file is a small, self-contained, neatly-packaged box — video, audio tracks, subtitle tracks, metadata — assembled to a recipe and ready to serve.
