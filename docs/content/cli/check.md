+++
title = "bento check"
description = "Verify external dependencies and the global config."
weight = 3
+++

```
bento check [-y]
```

Verifies that Bento's external dependencies are present and usable, and that the global config exists.

## What it checks

**`ffmpeg` and `ffprobe`:** Confirms both are on `PATH` and checks their versions against:

- A pinned minimum version (oldest known to work with Bento).
- The tested-against version (what Bento's CI builds against).

| Detected version | Result |
|---|---|
| Below pinned minimum | Warning |
| Between minimum and tested | Silent pass |
| Same major as tested | Silent pass |
| Major version above tested | Warning (potentially breaking under SemVer) |

If `ffmpeg` is missing entirely, Bento prints platform-appropriate install hints and exits non-zero.

**Global config:** Confirms `~/.config/bento/config.toml` (or the platform equivalent) exists and is parseable. If it's missing, Bento offers to generate it.

## Flags

### `-y` / `--yes`

Auto-confirm the global config generate prompt. In non-interactive contexts (no TTY) without `-y`, `check` errors rather than hanging.

## When to run

- **After installing Bento** — generates the global config and confirms `ffmpeg` is set up.
- **After upgrading `ffmpeg`** — confirms the new version is compatible.
- **After accidentally breaking the global config** — re-run to validate and offer to regenerate.

Version checking does not run on every encode invocation — only on `bento check` — to avoid startup latency.
