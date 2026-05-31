+++
title = "config"
weight = 2
+++

```
bento config <path>
```

Resolves the full config for `<path>` and prints the result with full provenance — every resolved field annotated with the layer it came from and the file path where applicable. Does not encode anything.

## Arguments

| Argument | Description |
|---|---|
| `<path>` | Path to a video file or a directory. For a directory, the config is resolved and printed for every valid video file it contains. |

## Description

For each file, every resolved setting is shown alongside its source:

```
episode01.mkv:
  output.container       = "mp4"         [global: ~/.config/bento/config.toml]
  output.destination     = "encoded"     [directory: ./bento.toml]
  video.encoder.name     = "x264"        [global: ~/.config/bento/config.toml]
  video.encoder.crf      = 20            [global: ~/.config/bento/config.toml]
  audio.tracks           = [...]         [directory: ./bento.toml]
  subtitles.tracks       = [...]         [directory: ./bento.toml]
  ...
```

## Comparison with `--dry-run`

`bento config` answers: **what settings will be used, and where did they come from?**

`bento convert --dry-run` answers: **what will actually happen?**

The key difference is that `--dry-run` probes the source files to make decisions that depend on them — copy vs. transcode per audio track requires knowing the source codec and bitrate. `bento config` deliberately doesn't probe sources, so its output is stable and fast regardless of the source files' state.

Use `bento config` to audit your config layer setup. Use `--dry-run` when you want to see the full encode plan before committing.

## See also

- [`convert`](@/cli/convert.md) — `--dry-run` shows the full encode plan, including source-dependent decisions.
- [`probe`](@/cli/probe.md) — inspect the source file's actual streams.
- [Configuration overview](@/configuration/overview.md) — the layering model `config` makes visible.
