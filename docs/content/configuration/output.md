+++
title = "Output"
description = "Container format, destination, filename templating, conflict resolution, and embedded metadata."
weight = 2

[extra]
toc_strip_signatures = true
+++

The `[output]` section controls everything about the output file that isn't a stream encoding decision: where files go, what they're named, what metadata is embedded, and what happens if the output already exists.

```
[output]
container         = "mp4" | "mkv"
destination       = <string>
preserve_chapters = <bool>
on_existing       = "warn" | "skip_silently" | "overwrite" | "fail"
metadata = {
    show   = <string>
    season = <integer>
    year   = <integer>
}
naming = {
    regex    = <string>
    template = <string>
}
```

## Fields

### `container = <option>` {#container}

The output container format. Default `"mp4"`.

- **`"mp4"`** — Maximizes Jellyfin direct-play compatibility, especially on Raspberry Pi clients.
- **`"mkv"`** — Supports more subtitle formats natively (e.g. soft ASS), but narrows the direct-play client set.

### `destination = <string>` {#destination}

Where output files are written. Default `"."` (alongside the source).

Relative paths resolve against **the source file's directory**, not the working directory. `destination = "encoded"` puts each output file in an `encoded/` subdirectory next to its input. Bento creates the directory if it doesn't exist. Absolute paths are used as-is.

### `preserve_chapters = <bool>` {#preserve_chapters}

Whether to copy chapter markers from the source to the output. Default `true`.

### `on_existing = <option>` {#on_existing}

What to do when the target output file already exists. Default `"warn"`.

- **`"warn"`** — Print a warning, leave the existing file in place, continue with the next file.
- **`"skip_silently"`** — Leave the existing file in place silently, continue.
- **`"overwrite"`** — Replace the existing file without warning.
- **`"fail"`** — Abort the entire run.

The `--overwrite` / `-f` CLI flag and `--on-existing=VALUE` override this per-run. See [`bento convert`](@/cli/convert.md#on-existing).

### `metadata = <inline table>` {#metadata}

Tags embedded in the output container. All fields are optional; absent fields are not written.

- **`show = <string>`**
    - Series title.
- **`season = <integer>`**
    - Season number.
- **`year = <integer>`**
    - Release year.

These map to standard container tags that Jellyfin uses to corroborate filename-based scraping. For richer tagging, post-process with `mkvpropedit` or `AtomicParsley`.

### `naming = <inline table>` {#naming}

Controls output filenames. If absent, output filenames mirror source filenames with the extension changed to match `container`.

- **`regex = <string>`**
    - A regular expression matched against each source filename (without extension). Named captures become template variables. If set and fails to match a file, that file errors at encode time.
- **`template = <string>`**
    - The output filename (without extension). References variables as `{name}`. Format specifiers like `{name:02}` (zero-padded integer) work on integer-typed values.

**Available template variables:**

- `{show}`, `{season}`, `{year}` — from the `metadata` table.
- `{source_basename}` — the source filename without extension.
- `{source_dir}` — the name of the source's containing directory.
- Any named capture from `regex`.

**Auto-derived episode numbers.** When `naming.regex` includes a capture named `episode` or `ep`, Bento embeds it as the episode number tag in the output container. Other capture names are template-only and are not embedded.

## Examples

```toml
[output]
container = "mp4"
destination = "encoded"
preserve_chapters = true
on_existing = "warn"
metadata = { show = "Cowboy Bebop", season = 1, year = 1998 }
naming = {
    regex = 'S(?<s>\d+)E(?<episode>\d+)',
    template = "{show} - S{s:02}E{episode:02}",
}
```

A source file `Cowboy Bebop S01E06 [BD 1080p].mkv` produces `Cowboy Bebop - S01E06.mp4` in an `encoded/` subdirectory, with embedded tags for show, season, year, and episode 6.
