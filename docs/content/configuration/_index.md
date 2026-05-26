+++
title = "Configuration"
description = "Bento's configuration model — file locations, layering, and section-by-section reference."
sort_by = "weight"
weight = 4
+++

Bento is configured through layered TOML files — global, directory, and per-file sidecar — plus CLI overrides, each sharing the same `[output]`, `[video]`, `[audio]`, and `[subtitles]` sections. Start with the **Overview** to see how config files are located and how their settings layer together; the remaining pages are the field-by-field reference for each section.

- [Overview](@/configuration/overview.md) — Config file locations, the layering model, and built-in visibility tools.
- [Output](@/configuration/output.md) — Container format, destination, filename templating, conflict resolution, and metadata.
- [Video](@/configuration/video.md) — Encoder, CRF, preset, and source preprocessing.
- [Audio](@/configuration/audio.md) — Codec, bitrate, mixdown, normalization, and per-track configuration.
- [Subtitles](@/configuration/subtitles.md) — Soft mux, burn-in, external sidecars, ASS filtering, and track subtraction.
