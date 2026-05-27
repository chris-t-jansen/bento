# Bento Roadmap

A living status document tracking what's implemented in Bento versus the target laid out in [`DESIGN.md`](DESIGN.md). Items are grouped by **status**, not by subsystem — if you want subsystem context, follow the DESIGN.md section reference on each item.

---

## How to update this roadmap

At the **end of each working session**, before the session closes:

1. **Promote items** that changed status this session:
   - Not Started → In Progress when work begins.
   - In Progress → Done when the feature is wired end-to-end *and* has at least minimal test coverage (or a manual verification note).
2. **Add new items** for any feature work that surfaced mid-session and wasn't already listed (cross-check against DESIGN.md so the roadmap stays anchored to the spec, not to scope drift).
3. **Keep each item's format consistent**: `- Feature name — DESIGN.md §reference. (Optional one-line note: what's left, blocker, or evidence.)`
4. **Don't delete completed items** — they stay in the Done section so the doc doubles as a record of progress.
5. **Re-check the "MVP happy path" callouts** at the top of each section — if an MVP-blocker has shipped, move the callout too.

If a feature in DESIGN.md no longer matches the implementation's direction, note it in **Open questions / scope drift** at the bottom rather than silently dropping it.

---

## Done

Foundation pieces that are working end-to-end. Most have at least light test coverage in `tests/convert.rs`.

### Configuration & resolution
- Layered config resolution: CLI > per-file sidecar > directory > global > defaults — DESIGN.md §Configuration. (`resolve.rs`, `layers.rs`.)
- Scalar leaf-level merge, list wholesale-replace, sum-type coalescing — DESIGN.md §Merge semantics.
- Section-level cascade of per-track defaults for audio/subtitles — DESIGN.md §Audio / §Subtitles. (Cascade resolved; not yet fully consumed by encoder args.)
- Cross-field validation: CRF/codec coupling, tune validity, `default = true` uniqueness, subtitle filter/subtract mutual exclusion — DESIGN.md §Validation. (`validate.rs`.)
- Validation issue reporting with severity, dotted path, and message — DESIGN.md §Errors & warnings.
- **Baked-in defaults layer** — DESIGN.md §Defaults. `baked_defaults()` in `resolve.rs` is fully populated; resolution falls through to built-in values correctly. (`resolve.rs`.)
- **Global config bootstrap template** — DESIGN.md §Bootstrap. Template in `bootstrap.rs` is comprehensive and is invoked by `bento check [-y]` via `ensure_global_config`. (`bootstrap.rs`, `layers.rs`, `cli.rs`.)
- **Required-field detection** — DESIGN.md §Validation. `validate_output` in `validate.rs` checks `output.naming.regex` syntax and validates that every `{varname}` in `output.naming.template` resolves to a built-in, metadata field, or named regex capture. Per-track `source` required checks for audio and subtitle tracks were already in place. (`validate.rs`.)
- **`--keep-intermediates`** — DESIGN.md §CLI flags. Per-run `TempDir` moved to `run_convert` level; each file carves a sanitized-basename subdir within it. Flag suppresses cleanup via `TempDir::keep()` and prints the preserved path. Dry-run is a silent no-op. 3 new integration tests. (`pipeline/mod.rs`, `cli.rs`.)
- **`--generate-config`** — DESIGN.md §Sidecar generation. Writes CLI overrides to a sidecar TOML file (per-file: `<file>.bento.toml`; directory: `<dir>/bento.toml`). Errors if no CLI overrides are present. Warns and skips without overwriting if sidecar already exists. Dry-run reports "Would write sidecar at:" with no filesystem effects. CLI overrides are now folded into a proper `Layer::Cli` in the resolution stack (replacing the old `apply_warn_overrides` post-resolution mutation), so provenance correctly attributes CLI-set fields. 6 new integration tests. (`pipeline/mod.rs`, `cli.rs`, `error.rs`.)
- **`--set KEY=VALUE`** — DESIGN.md §Override semantics. Generic dotted-path CLI override. VALUE is parsed as a strict TOML scalar (bool, int, or quoted string); bare strings, tables, and arrays are rejected with specific errors. `audio.tracks` and `subtitles.tracks` are explicitly blocked with a sidecar-pointing error message. `--set` values flow into `Layer::Cli` alongside the dedicated flags and are captured by `--generate-config`. Dedicated flags (`--on-existing`, `--no-warn-*`) overwrite conflicting `--set` values for the same field. 5 new error variants, `src/set_override.rs` with 17 unit tests, 7 new integration tests. (`src/set_override.rs`, `pipeline/mod.rs`, `cli.rs`, `error.rs`.)

### CLI surface
- `bento convert <path> [output_dir]` for both single-file and directory mode — DESIGN.md §CLI.
- `bento config <path>` resolves and prints config with per-layer provenance — DESIGN.md §CLI. (`render.rs`.)
- `bento check [-y]` — DESIGN.md §`bento check`. Verifies global config (bootstrap if missing, prompt or `-y` auto-confirm) and detects `ffmpeg`/`ffprobe` on PATH with version-band checking (below minimum → warn loudly; same major as tested or between → silent; above tested major → note). Exits non-zero if either binary is missing. (`cli.rs`, `ffmpeg.rs`, `layers.rs`.)
- `--no-warn-X` family + `--no-warnings` — DESIGN.md §Warning suppression. All 8 flags wired: `--no-warn-multiple-burns`, `--no-warn-burn-metadata`, `--no-warn-ass-to-srt`, `--no-warn-no-default` (suppresses both audio and subtitles), `--no-warn-crf-codec-mismatch`, `--no-warn-missing` and `--no-warn-redundant` (placeholders for unimplemented warnings), and `--no-warnings` bulk flag. CLI flags override resolved config `warn_*` fields via `apply_warn_overrides` called after resolution and before validation. (`cli.rs`, `pipeline/mod.rs`; 6 new integration tests.)
- `--overwrite` / `-f` shorthand and `--on-existing={warn,skip-silently,overwrite,fail}` — DESIGN.md §CLI flags.
- `--verbose` / `-v` and `--quiet` / `-q` verbosity flags — DESIGN.md §CLI flags. (`cli.rs`, `verbosity.rs`.)
- `--dry-run` / `-n` plan-without-encode mode — DESIGN.md §CLI flags. Resolves config, probes sources, and prints the per-file encode plan with no filesystem effects; summary shows "N files would be processed. M errors." with a `bento config` discovery footer. (`cli.rs`, `pipeline/mod.rs`, `pipeline/ffmpeg_args.rs`.)

### Schema parsing (parsed and merged, even if not all are wired into the encoder yet)
- `[output]` — container, destination, preserve_chapters, on_existing, metadata block, naming block.
- `[video]` — encoder, crf, tune, preset, crop (scalar + table forms), deinterlace, detelecine, denoise, resolution, never_upscale, warn_crf_codec_mismatch.
- `[audio]` — section defaults + per-track fields (source, lang, title, default, forced, original, commentary, hearing_impaired, visual_impaired), normalize_mix, warn_no_default.
- `[subtitles]` — per-track routing, warn flags, filter spec, soft-track metadata fields.

### Pipeline
- Source probing via ffmpeg: stream enumeration, duration extraction, and crop detection — DESIGN.md §Probe. (`pipeline/probe.rs`.)
- Real-time progress feedback during encode: spinner (unknown duration) or progress bar (known duration) with per-file status lines (✓/–/✗ with color); multi-line unified display (filename, config-layer summary, and bar/elapsed as a single visual entry); unified pre-encode header listing files (same format for single-file and directory mode); blank-line spacing between files and before batch summary; uses `indicatif` + `console`. (`progress.rs`, `pipeline/mod.rs`.)
- **`--dry-run` / `-n`** — resolve config, probe source files, and print the per-file encode plan (subtitle derivation, video params, audio copy-vs-transcode per track, mux destination) with no filesystem effects: no encodes, no output directories created, no temp files written. Header changes to "Dry-run for N files"; summary shows "N files would be processed. M errors."; discovery footer ("Run `bento config ...`") shown unless `--quiet`; under `--verbose`, also prints the ffmpeg command line with subtitle args omitted. (`cli.rs`, `pipeline/mod.rs`, `pipeline/ffmpeg_args.rs`.)
- Per-file error handling with batch-continue + end-of-batch summary — DESIGN.md §Batch behavior. (`pipeline/mod.rs`.)
- Audio copy-vs-transcode decision tree — DESIGN.md §Audio actions. (`pipeline/ffmpeg_args.rs`.)
- ffmpeg arg construction for video encoder, preset, tune, CRF, crop (pixels), deinterlace, detelecine, denoise, resolution/scale — DESIGN.md §Video. (`pipeline/ffmpeg_args.rs`.)
- Audio and subtitle per-track metadata (`-metadata:s:`) and disposition (`-disposition:`) flags — DESIGN.md §Audio / §Subtitles. (`pipeline/ffmpeg_args.rs`.)
- Soft subtitle mux with codec selection (`-c:s:`) — DESIGN.md §Mux. (`pipeline/ffmpeg_args.rs`.)
- Burn subtitle rendering via `subtitles=` libass filtergraph filter — DESIGN.md §Burn. (`pipeline/ffmpeg_args.rs`.)
- Subtitle extraction from source MKVs via ffmpeg (`-map 0:s:N`) — DESIGN.md §Pipeline. (`pipeline/subtitle_prep.rs`.)
- Subtitle derivation prep stage: `filter`, `subtract`, `ass_to_srt` wired end-to-end — DESIGN.md §Subtitle derivations. (`pipeline/subtitle_prep.rs`.)
- SRT parse/serialize/`subtract_by_timestamp` and ASS parse/serialize/`subtract_ass_by_timestamp`/`filter_ass`/`ass_to_srt` — DESIGN.md §Subtitle derivations. (`subtitles.rs`.)
- Output filename naming: `naming.regex` capture + `naming.template` expansion with format specifiers; `episode`/`ep` capture auto-embeds episode metadata (`tves` for MP4, `PART_NUMBER` for MKV) — DESIGN.md §[output] §Naming. (`pipeline/naming.rs`.)
- External subtitle tracks (`mux = "external"`): sidecar `.srt`/`.ass` files written next to the output video with Jellyfin-compatible filenames; `on_existing` policy applied per sidecar; duplicate sidecar name detection at validation time; external ASS correctly exempt from the MP4 soft-mux restriction — DESIGN.md §Subtitles > External subtitle tracks. (`pipeline/subtitle_prep.rs`, `validate.rs`.)
- **`bento repair`** — DESIGN.md §`bento repair`. Structural comparison of user's global config against baked defaults to detect missing fields; text-based surgical insertion preserving all existing content and comments (doc comments from the bootstrap template appended with `# (added by bento repair)` marker); corrupt-config path offers full regeneration; `--yes` flag for non-interactive use. `run_repair_at` is the path-explicit entry point used by integration tests. (`src/repair.rs`, `tests/repair.rs`; 22 unit + 7 integration tests.)
- **`bento probe <path>` subcommand** — DESIGN.md §CLI. Displays stream info from a video file in Bento-native terms: friendly codec names, resolution, framerate, 1-based type-relative track numbers for audio and subtitles (matching `source =` in config), language codes, channel layout, bitrate, and track titles. Section headers are colored (red/green/blue as a nod to RGB) and track numbers are magenta so users can copy them straight into `bento.toml`. Extended `VideoStreamInfo`, `AudioStreamInfo`, and `SubtitleStreamInfo` in `pipeline/probe.rs` to carry the additional fields; `probe_source_streams` now parses codec, framerate, channel layout, title/language tags, and falls back to the `BPS` stream tag for bitrate (needed for MKV files, which rarely carry `bit_rate` in stream headers). Column widths for language, codec, and channel layout are computed across all tracks so every column aligns. Footer hint reminds users how track numbers map to `source =`. (`src/probe.rs`, `src/pipeline/probe.rs`; 8 unit tests + 1 render integration test.)
- **Refactor `pipeline::run_convert_directory` and `pipeline::run_convert_file` to reduce argument counts.** Both functions' 8 and 10 positional args were bundled into a private `ConvertContext<'a>` struct (holding `cli_config`, `output_dir_override`, `dry_run`, `verbosity`, `warn_flags`, `temp_root`). Both functions now take `(…, ctx: &ConvertContext<'_>, out: &mut dyn Write)` — 3 and 5 args respectively. `#[allow(clippy::too_many_arguments)]` and TODO markers removed. (`src/pipeline/mod.rs`; all 45 integration tests still pass.)

---

## In progress

*(nothing currently in progress)*

---

## Not started

- **Reconsider `normalize_mix` scope and add a sanity warning.** Currently section-only (`Audio` struct in `src/config/audio.rs`; no field on `AudioTrack`). Two things to think through: (1) it arguably should be per-track-overridable like `encoder`/`bitrate`/`mixdown`, since downmix normalization is a per-track concern; (2) it only does anything on surround→stereo downmixes, so setting `normalize_mix = true` when no track targets `mixdown = "stereo"` is likely a mistake worth warning about. Needs more design thought before implementing. Surfaced 2026-05-26 during docs work.
- **Document `bento probe` in the docs site.** The docs currently have no mention of `bento probe`. Should cover: what it does, example output, and an explicit call-out that the track numbers printed are the same values to use for `source =` in `[audio]` and `[subtitles]` config. Good candidate for a new page under the CLI reference section, or as a subsection of the existing `bento check` / `bento config` page if one exists. Surfaced 2026-05-26.

---

## Backlog / nice-to-have

Anything explicitly deferred in DESIGN.md or surfaced as future work. Move items here from "Not started" if a session concludes they're out of scope for MVP.

*(nothing currently in backlog)*

---

## Open questions / scope drift

Things in the code that don't cleanly map back to DESIGN.md, or design decisions that may have shifted. Resolve these before they accumulate.

*(no open questions currently)*

*Resolved 2026-05-19:*
- ~~ffmpeg-only vs HandBrakeCLI~~ — confirmed intentional. DESIGN.md updated (see §Background, "second note on lineage") to record the pivot to pure ffmpeg.
- ~~`subtitles.rs` scope mismatch~~ — actual surprise was that ASS parsing/serializing/filter/subtract/conversion had quietly landed while the module header still claimed Phase 5a (SRT-only). Header rewritten to match what's shipped; stale HandBrake `--srt-file` reference removed.

---

*Last updated: 2026-05-27. `ConvertContext` refactor complete; moved to Done.*
