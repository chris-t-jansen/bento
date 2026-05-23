+++
title = "bento repair"
description = "Populate missing fields in the global config with current defaults."
weight = 4
+++

```
bento repair
```

Repairs the global `config.toml`. Scans the existing global config for fields that are expected to be present (fields with built-in defaults), prints which ones are missing, and offers to insert them at their default values with documentation comments.

## When to run

After upgrading Bento, new config fields may exist that aren't in your global config yet. These trigger missing-setting warnings during `bento convert` (since the values are resolving from the built-in default layer rather than your config). Run `bento repair` to populate them in bulk and silence the warnings.

## What it does

1. Reads the existing global config.
2. Identifies fields with built-in defaults that aren't present.
3. Prints the list of missing fields and their default values.
4. Prompts to insert them (or skips on confirmation refusal).

If the global config is corrupt or unparseable, `repair` warns and offers to regenerate it from scratch.

## Relationship to `warn_missing`

The missing-setting warning during `bento convert` is the signal that `bento repair` is needed. The warning fires per-field, per-encode, until the field is added to your config. `repair` closes the loop in bulk.
