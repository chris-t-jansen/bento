+++
title = "CLI Reference"
description = "Bento's four subcommands and their flags."
sort_by = "weight"
weight = 5
+++

Bento has four subcommands. The config is the primary interface — the CLI exists to drive it, not replace it.

| Subcommand | Description |
|---|---|
| [`bento convert`](@/cli/convert.md) | Run the conversion pipeline against a file or directory. |
| [`bento config`](@/cli/config.md) | Print the fully resolved config with per-field provenance. Does not encode. |
| [`bento check`](@/cli/check.md) | Verify external dependencies and the global config. |
| [`bento repair`](@/cli/repair.md) | Populate missing fields in the global config with current defaults. |

## Override mechanism

CLI flags are the highest-precedence configuration layer, above all config files.

**Dedicated flags** clobber a specific config value. The full set is documented per subcommand.

**`--set KEY=VALUE`** is the generic override mechanism. `KEY` is a dotted path into the config schema (e.g. `video.encoder.crf`, `audio.bitrate`). `VALUE` is parsed as a TOML scalar — `true`, `42`, `"quoted string"` — bare unquoted strings are not accepted.

Resolves at the leaf level, consistent with the general cascade rule: `--set video.encoder.crf=22` overrides only `crf`; `name` and `tune` fall through to lower layers.

Track lists (`audio.tracks`, `subtitles.tracks`) are not addressable via `--set`. Use a `<videofile>.bento.toml` sidecar for per-file track changes.

See [Flags](@/cli/flags.md) for the complete flag reference.
