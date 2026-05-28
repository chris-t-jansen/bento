+++
title = "Flags"
description = "Complete reference for verbosity, warning suppression, and output-collision flags."
weight = 6
+++

## Verbosity

`--verbose` / `-v` and `--quiet` / `-q` are mutually exclusive.

| Level | Errors | Warnings | Per-file output | Encoder progress | Provenance / commands |
|---|---|---|---|---|---|
| `--quiet` / `-q` | Yes | Yes | None | None | None |
| (default) | Yes | Yes | Layer-count summary line | Brief in-place line (TTY) or one line per update | None |
| `--verbose` / `-v` | Yes | Yes | Full per-field provenance table | Full `ffmpeg` output | `ffmpeg` command lines per file |

**Warnings are independent of verbosity.** `-q` does not suppress warnings — use `--no-warn-*` for that. Conflating the two would mean scripted runs with `-q` could silently miss real anomalies.

**TTY detection.** Default-mode progress uses carriage-return in-place updates on a TTY, falling back to one-line-per-update otherwise (so `bento convert ./ > log.txt` produces a readable log).

**Run summary always shown.** The end-of-batch `8 succeeded, 2 failed` line is shown in all modes including `-q`.

## Warning suppression

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

`--no-warn-no-default` suppresses both the audio and subtitle variant in one shot. The CLI is coarser than the config by design — a one-off suppression rarely needs per-section precision.

There is no `--warn-X` form. All warnings default on; to re-enable one disabled in config, use `--set audio.warn_no_default=true` (or the relevant dotted path).

`--no-warnings` is deliberately not mirrored as a config field. Persistent "suppress everything forever" would mask real problems silently; the flag keeps the choice visible per-invocation.

## Output-collision flags

These override `[output].on_existing` for a single run.

### `--overwrite` / `-f`

Shorthand for `--on-existing=overwrite`. The most common case — "rerun with new settings, replace the old outputs."

### `--on-existing=VALUE`

Full override. Values (kebab-case at the CLI, snake_case in config):

| CLI value | Config value | Behavior |
|---|---|---|
| `warn` | `"warn"` | Print a warning, leave existing file, continue. |
| `skip-silently` | `"skip_silently"` | Leave existing file silently, continue. |
| `overwrite` | `"overwrite"` | Replace without warning. |
| `fail` | `"fail"` | Abort the run. |

Passing both `--overwrite`/`-f` and `--on-existing=VALUE` is a CLI error, even if the values would agree.

## `--set KEY=VALUE`

Override any scalar config field by dotted path.

```sh
bento convert ./ --set video.encoder.crf=18
bento convert ./ --set output.container="mkv"
bento convert ./ --set audio.bitrate=256 --set video.preset="slow"
```

`VALUE` is parsed as a TOML scalar. Quoting rules:

| Type | Example |
|---|---|
| Integer | `--set video.encoder.crf=18` |
| Boolean | `--set output.preserve_chapters=false` |
| String | `--set output.container="mkv"` (quotes required for strings) |

Track lists (`audio.tracks`, `subtitles.tracks`) are not addressable. Use a `<videofile>.bento.toml` sidecar for per-file track changes.
