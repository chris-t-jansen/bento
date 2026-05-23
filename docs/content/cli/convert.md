+++
title = "bento convert"
description = "Run the full conversion pipeline against a file or directory."
weight = 1
+++

```
bento convert <path> [output_dir]
```

The primary command. Runs the full pipeline — config resolution, validation, extract, derive, transcode, mux — for one file or a whole directory.

## Arguments

| Argument | Description |
|---|---|
| `<path>` | Path to a video file or a directory. |
| `[output_dir]` | Optional. Overrides `[output].destination` for this run. Resolves relative to CWD, not the source directory. |

When `<path>` is a directory, Bento processes every video file it contains (non-recursive) by extension: `.mkv`, `.mp4`, `.m4v`, `.avi`, `.mov`, `.webm`, `.ts`, `.m2ts`, `.wmv`. To process a file with a non-standard extension, pass it directly as `<path>`.

## Batch behavior

Files in a directory are processed sequentially. Per-file errors (config validation failure, ffmpeg error) are logged and the batch continues. Environmental errors that would affect all remaining files (missing `ffmpeg`, disk full) abort the run.

At the end of a batch, Bento always prints a summary regardless of verbosity:

```
8 succeeded, 2 failed:
  episode06.mkv: ffmpeg exited non-zero (see log above)
  episode11.mkv: required field audio.tracks not resolvable
```

Exit code is non-zero if any file failed.

## Flags

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

Shorthand for `--on-existing=overwrite`. Replaces existing output files without warning.

### `--on-existing=VALUE`

Override `[output].on_existing` for this run. Values: `warn`, `skip-silently`, `overwrite`, `fail` (kebab-case). Mutually exclusive with `--overwrite` / `-f`.

### `--set KEY=VALUE`

Override a config field at any dotted path. See [CLI Reference](/cli) for semantics.

### Warning suppression

See [Flags](/cli/flags) for the full `--no-warn-*` and `--no-warnings` reference.

### Verbosity

`--verbose` / `-v` and `--quiet` / `-q`. See [Flags](/cli/flags).
