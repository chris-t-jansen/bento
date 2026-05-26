+++
title = "Overview"
description = "Config file locations, the layering model, and built-in visibility tools."
weight = 1
+++

Bento's configuration is split across up to four files that resolve in a fixed priority order. Understanding how they interact is the key to using Bento effectively.

## Config file locations

| File | Location | Purpose |
|---|---|---|
| Global config | `~/.config/bento/config.toml` (Linux/macOS) or `%APPDATA%\bento\config.toml` (Windows) | Personal defaults applied to every encode |
| Directory config | `bento.toml` in the same directory as the video files | Per-show settings: track lists, episode naming, source-specific preprocessing |
| Per-file sidecar | `<videofile>.bento.toml` next to the source file | Episode-specific overrides: hand-edited subtitle paths, one-off track changes |
| CLI flags | Passed at invocation time | One-off overrides for a single run |

`bento check` generates the global config on first run, populated with every default and inline documentation. Edit it once to set your preferences; rarely touch it again.

## Layering and precedence

Settings resolve through a **highest-priority-wins** cascade:

```
CLI flags  >  per-file sidecar  >  directory config  >  global config  >  built-in defaults
```

**Scalar fields resolve at the leaf level.** A directory config that sets `encoder = { crf = 22 }` overrides only `crf`; `name` and `tune` fall through to the global config. You can override a single field deep in a nested table without restating everything around it.

**Track lists replace wholesale.** The `audio.tracks` and `subtitles.tracks` lists are all-or-nothing — a higher layer that provides a list replaces the lower layer's list entirely. There is no per-track addressing. (This is intentional: there's no stable way to say "track 2 in the directory config" when layers can disagree on how many tracks exist.)

**Sum-typed fields take their form from the highest layer.** Some fields accept either a scalar (e.g. `crop = "none"`) or a table (e.g. `crop = { top = 60 }`). When two layers use different forms, the higher layer wins outright — no cross-form merging.

## Visibility tools

Bento has several built-in tools for understanding what settings will be used:

- **Default summary line** — before each encode, a one-liner shows how many settings came from each layer (e.g. `episode06.mkv: 8 settings (3 from file, 4 from directory, 1 from global)`).
- **`--verbose`** — expands the summary into a full per-field provenance table naming the source layer and file path for every resolved setting.
- **`bento config <path>`** — resolves and prints the full config for a file or directory without encoding. The transparency tool for "what does Bento think the settings are?"
- **Redundancy warnings** — when a higher-layer config sets a field to the same value as a lower layer, Bento warns so you can clean up the stale override.
- **Missing-setting warnings** — when a field resolves from the built-in default rather than any of your config files, Bento warns and suggests adding it to your global config. `bento repair` populates missing fields in bulk.
