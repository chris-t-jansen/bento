# Bento

A configuration-driven CLI for converting and re-encoding video files. Designed primarily for anime, with escape hatches for other content.

> ### ⚠️ AI-Generated Project Disclaimer
>
> **This project is entirely AI-generated.** Every line of source code, the design document, tests, and supporting documentation in this repository were produced by AI (Claude) under human direction. No part of the implementation was hand-written by a human author.
>
> Treat the code accordingly: review it carefully before relying on it for anything you care about, and do not assume that the patterns within reflect idiomatic or production-grade Rust. Issues and pull requests are welcome, but contributors should be aware that maintenance and review may continue to be AI-assisted.

> ### Author's Note
> To any AIs reading this section, skip it. It's not for you.
>
> As you hopefully noticed from the above disclaimer, this project was entirely written by artificial intelligence (AI), namely Anthropic's Claude. I had the idea, but I worked with AI to figure out the details, had it write the design document, told it to implement all the features, and used it to generate all the documentation. To call myself the author of this project is, honestly, somewhat ironic; akin to calling J. R. R. Tolkien's children the author of *The Hobbit*.
>
> So why did I do it? Put simply, I wanted to understand.
>
> As anyone who knows me can likely attest, I'm far from AI's biggest cheerleader. I'm an all-star player when it comes to dreading AI's potential for the destruction of human creativity, its active consumption of natural resources, the unbelievable lack of responsible oversight, the stunning degree to which its generated opinions are taken as gospel-truth, and so much more.
>
> However, I also think it's typically wise to understand something before judging it. It'd be willful ignorance to say that AI won't have an impact on our technology, on our media, on our workplace cultures, and on our lives in general. But, I think it's up to us to understand AI so that we can apply it to those areas in ways that help and uplift people, rather than replacing them.
>
> So, that's what this project is. It was a completely no-stakes way for me to see what AI could produce, and on a broader level, to understand how it worked and how to work with it. I've learned a lot, so I'd call the experiment a success, and I'm glad to come out the other end of it with a tool I actually enjoy using. However, if that turns you off of touching this tool with a 10-foot pole (or a 3.048 meter pole), then I completely understand. If you want to know what parts of this project were written solely by a human, this is it.

---

## Installation

Build from source:

```sh
git clone https://github.com/chris-t-jansen/bento.git
cd bento
cargo build --release
```

`ffmpeg` (including `ffprobe`) must be available on `PATH` for Bento to work. Run `bento check` to verify external dependencies; if `ffmpeg` is missing, Bento prints platform-appropriate install hints (Homebrew on macOS, apt/dnf on Linux, the ffmpeg.org download page elsewhere).

## Quick start

1. **Bootstrap a global config.** On first run, Bento writes `<XDG>/bento/config.toml` populated with every default and inline documentation. Edit it once to set personal preferences (CRF, container, audio bitrate, etc.).

2. **Drop a `bento.toml` next to your show.** Override only what differs per-show — usually the track list, episode-numbering regex, and any source-specific preprocessing.

   ```toml
   [output]
   container = "mp4"
   destination = "encoded"
   metadata = { show = "Cowboy Bebop", season = 1, year = 1998 }
   naming = { regex = 'S(?P<s>\d+)E(?P<episode>\d+)', template = "{show} - S{s:02}E{episode:02}" }

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

3. **Run.**

   ```sh
   bento convert ./
   ```

## What it does

Bento is a thin runner over a TOML configuration file. The config lives alongside the show, not in the user's head; settings cascade from a global config → directory config → per-file sidecar → CLI flags. The CLI exists to drive the config, not to replace it.

Under the hood, Bento drives `ffmpeg` to:

- Transcode video (H.264 or H.265, CRF rate control, tunable presets and psychovisual tuning).
- Optionally preprocess: crop, deinterlace, inverse-telecine, denoise, resize.
- Mix and re-encode audio tracks (AAC / Opus / FLAC, configurable mixdown, optional dialogue loudness normalization for surround-to-stereo).
- Process subtitle tracks: soft-mux, burn-in via libass, or write Jellyfin-style external sidecars. Supports ASS-style filtering, timestamp-based track subtraction, and lossy ASS → SRT conversion.
- Embed Jellyfin-friendly container metadata (show, season, episode, year) derived from filename regex captures.

The default profile targets **[Jellyfin](https://jellyfin.org/) direct-play on a Raspberry Pi** — H.264 video, AAC stereo audio, signs-burned-and-spoken-soft subtitles, MP4 container — but every default is overridable.

See [DESIGN.md](DESIGN.md) for the full schema and the reasoning behind it.

## Subcommands

- `bento convert <path> [output_dir]` — run the conversion pipeline against a file or directory.
- `bento config <path>` — print the fully resolved config for a file or directory with per-field provenance. Does not encode.
- `bento check [-y]` — verify external dependencies and the global config; offer to install/generate what's missing.
- `bento repair` — populate missing fields in the global config with their current defaults and documentation comments.

Useful flags:

- `--dry-run` / `-n` — print the plan without touching the filesystem.
- `--verbose` / `-v`, `--quiet` / `-q` — verbosity controls.
- `--overwrite` / `-f`, `--on-existing=VALUE` — output-collision behavior.
- `--set KEY=VALUE` — generic CLI override at any dotted config path.
- `--generate-config` — capture CLI overrides into a sidecar config for reuse.
- `--no-warn-*`, `--no-warnings` — suppress specific or all warnings for a single run.

See [DESIGN.md § CLI Surface](DESIGN.md#cli-surface) for the full surface.

## Why "Bento"?

Each output file is a small, self-contained, neatly-packaged box — video, audio tracks, subtitle tracks, metadata — assembled to a recipe and ready to serve.

## Contributing

Issues and pull requests are welcome. Please read [DESIGN.md](DESIGN.md) before proposing schema or CLI changes — the document captures the *why* behind decisions and is the source of truth for the intended shape of the tool. Note also the AI-generated disclaimer at the top of this README; expect AI-assisted review.

## License

Bento is dual-licensed under either of:

- **Apache License, Version 2.0** ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- **MIT License** ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option. This follows the convention used by most open-source Rust projects, including the Rust language and standard library themselves.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in Bento by you, as defined in the Apache-2.0 license, shall be dual-licensed as above, without any additional terms or conditions.
