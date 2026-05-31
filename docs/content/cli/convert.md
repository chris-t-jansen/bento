+++
title = "convert"
weight = 1
+++

```
bento convert [options] <path> [output_dir]
```

The primary command. Runs the full pipeline — config resolution, validation, extract, derive, transcode, mux — for one file or a whole directory.

## Arguments

| Argument | Description |
|---|---|
| `<path>` | Path to a video file or a directory. |
| `[output_dir]` | Optional. Overrides `[output].destination` for this run. Resolves relative to CWD, not the source directory. |

When `<path>` is a directory, Bento processes every video file it contains (non-recursive) by extension: `.mkv`, `.mp4`, `.m4v`, `.avi`, `.mov`, `.webm`, `.ts`, `.m2ts`, `.wmv`. To process a file with a non-standard extension, pass it directly as `<path>`.

## Description

Files in a directory are processed sequentially. Per-file errors (config validation failure, ffmpeg error) are logged and the batch continues. Environmental errors that would affect all remaining files (missing `ffmpeg`, disk full) abort the run.

At the end of a batch, Bento always prints a summary regardless of verbosity:

```
8 succeeded, 2 failed:
  episode06.mkv: ffmpeg exited non-zero (see log above)
  episode11.mkv: required field audio.tracks not resolvable
```

Exit code is non-zero if any file failed.

## Options

### `--dry-run` / `-n`

Resolve config, validate, and print the plan — what Bento would do — without touching the filesystem. No encodes, no temp files, no `--generate-config` writes.

The plan shows per-file decisions including copy vs. transcode per audio track (which requires probing the source file):

```
episode01.mkv:
  Would extract subtitle tracks 1, 2 from source.
  Would derive 1 soft subtitle track:
    "English" (eng): track 1 minus track 2 timestamps, format=srt, default=true.
  Would burn 1 subtitle track:
    track 2 (ass) onto video.
  Would transcode video: x264 crf=20 tune=animation preset=medium, no preprocessing.
  Would copy 2 audio tracks:
    "Japanese" (jpn): copy (source aac stereo matches target).
    "English Dub" (eng): copy.
  Would mux to: ./encoded/episode01.mp4.
```

Validation errors surface during dry-run and contribute to a non-zero exit code — the point of the flag is catching problems before a multi-hour encode.

With `-v`, dry-run also prints the actual `ffmpeg` command lines that would run.

### `--generate-config`

After resolving the full config (including any CLI overrides), write a config file capturing only what the CLI flags changed:

- For a file target: writes `<videofile>.bento.toml` next to the source.
- For a directory target: writes `<dir>/bento.toml`.

The conversion still runs — `--generate-config` is an additive side effect, not a mode switch. If no CLI overrides were passed, errors before encoding starts (nothing to write). If the target file already exists, warns and continues without overwriting (delete the file and re-run to regenerate).

### `--keep-intermediates`

Preserve temp files (extracted subtitle tracks, derived subtitle outputs) after the run. By default they are cleaned up automatically. At the end of the run, Bento prints the path:

```
Intermediate files preserved at: /tmp/bento-aBcDef
```

Under `--dry-run`, this flag is a no-op (dry-run produces nothing to keep).

### `--overwrite` / `-f`

Shorthand for `--on-existing=overwrite` — the most common case, "rerun with new settings, replace the old outputs." Mutually exclusive with `--on-existing`; passing both is a CLI error, even if the values would agree.

### `--on-existing=VALUE` {#on-existing}

Override the resolved `[output].on_existing` for this run. CLI flags are the highest-precedence configuration layer, above all config files.

| CLI value | Config value | Behavior |
|---|---|---|
| `warn` | `"warn"` | Print a warning, leave the existing file, continue. |
| `skip-silently` | `"skip_silently"` | Leave the existing file silently, continue. |
| `overwrite` | `"overwrite"` | Replace without warning. |
| `fail` | `"fail"` | Abort the run. |

Values are kebab-case at the CLI, snake_case in config.

### `--set KEY=VALUE` {#set}

The generic override mechanism: set any scalar config field by dotted path. `KEY` is a path into the config schema (e.g. `video.encoder.crf`, `audio.bitrate`); `VALUE` is parsed as a TOML scalar. May be repeated.

```sh
bento convert ./ --set video.encoder.crf=18
bento convert ./ --set output.container="mkv"
bento convert ./ --set audio.bitrate=256 --set video.preset="slow"
```

| Type | Example |
|---|---|
| Integer | `--set video.encoder.crf=18` |
| Boolean | `--set output.preserve_chapters=false` |
| String | `--set output.container="mkv"` (quotes required for strings) |

Bare unquoted strings are not accepted. Overrides resolve at the leaf level, consistent with the config cascade: `--set video.encoder.crf=22` overrides only `crf`, while `name` and `tune` fall through to lower layers.

Track lists (`audio.tracks`, `subtitles.tracks`) are not addressable via `--set`. Use a `<videofile>.bento.toml` sidecar for per-file track changes.

### Warning suppression {#warning-suppression}

Each warning has a corresponding `--no-warn-X` flag. A bulk `--no-warnings` flag suppresses everything in one shot.

| Warning | Config field | CLI flag |
|---|---|---|
| Multiple `mux = "burn"` subtitle tracks | `[subtitles].warn_multiple_burns` | `--no-warn-multiple-burns` |
| Burn track with soft-track metadata fields | `[subtitles].warn_burn_metadata` | `--no-warn-burn-metadata` |
| Lossy ASS → SRT conversion | `[subtitles].warn_ass_to_srt` | `--no-warn-ass-to-srt` |
| No subtitle or audio track marked `default = true` | `[audio].warn_no_default` / `[subtitles].warn_no_default` | `--no-warn-no-default` |
| CRF value suspicious for resolved encoder | `[video].warn_crf_codec_mismatch` | `--no-warn-crf-codec-mismatch` |
| Surround downmix runs without loudness normalization | `[audio].warn_unnormalized_downmix` | `--no-warn-unnormalized-downmix` |
| Field resolved from built-in default instead of user config | (runtime, no config field) | `--no-warn-missing` |
| Higher-precedence layer sets a field to the same value as a lower layer (also a per-track `normalize_downmix = true` that has no effect) | (runtime, no config field) | `--no-warn-redundant` |
| All of the above | — | `--no-warnings` |

`--no-warn-no-default` suppresses both the audio and subtitle variant in one shot — the CLI is coarser than the config by design, since a one-off suppression rarely needs per-section precision.

There is no `--warn-X` form. All warnings default on; to re-enable one disabled in config, use `--set audio.warn_no_default=true` (or the relevant dotted path). `--no-warnings` is deliberately not mirrored as a config field: a persistent "suppress everything forever" would mask real problems silently, so the flag keeps the choice visible per-invocation.

**Warnings are independent of verbosity.** `-q` does not suppress warnings — use `--no-warn-*` for that. Conflating the two would mean scripted runs with `-q` could silently miss real anomalies.

### Verbosity {#verbosity}

`--verbose` / `-v` and `--quiet` / `-q` are mutually exclusive.

| Level | Errors | Warnings | Per-file output | Encoder progress | Provenance / commands |
|---|---|---|---|---|---|
| `--quiet` / `-q` | Yes | Yes | None | None | None |
| (default) | Yes | Yes | Layer-count summary line | Brief in-place line (TTY) or one line per update | None |
| `--verbose` / `-v` | Yes | Yes | Full per-field provenance table | Full `ffmpeg` output | `ffmpeg` command lines per file |

**TTY detection.** Default-mode progress uses carriage-return in-place updates on a TTY, falling back to one-line-per-update otherwise (so `bento convert ./ > log.txt` produces a readable log).

**Run summary always shown.** The end-of-batch `8 succeeded, 2 failed` line is shown in all modes, including `-q`.

## Examples

```sh
bento convert episode01.mkv                          # one file, using the resolved config
bento convert ./                                     # every video in the current directory
bento convert ./ ~/encoded                           # override the output destination
bento convert ./ --dry-run                           # preview the plan without encoding
bento convert ep.mkv --set video.encoder.crf=18 -f   # one-off override, replace outputs
```

## See also

- [`config`](@/cli/config.md) — see the resolved config and where each value came from.
- [`probe`](@/cli/probe.md) — find the track numbers to reference in your config.
- [Configuration overview](@/configuration/overview.md) — how the config layers resolve.
