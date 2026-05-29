+++
title = "CLI Reference"
description = "Bento's five subcommands and their flags."
sort_by = "weight"
weight = 5
+++

Bento has five subcommands. The config is the primary interface — the CLI exists to drive it, not replace it.

| Subcommand | Description |
|---|---|
| [`bento convert`](@/cli/convert.md) | Run the conversion pipeline against a file or directory. |
| [`bento config`](@/cli/config.md) | Print the fully resolved config with per-field provenance. Does not encode. |
| [`bento check`](@/cli/check.md) | Verify external dependencies and the global config. |
| [`bento repair`](@/cli/repair.md) | Populate missing fields in the global config with current defaults. |
| [`bento probe`](@/cli/probe.md) | Inspect a video file's streams and identify track numbers for use in config. |
