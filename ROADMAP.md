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

### CLI surface
- `bento convert <path> [output_dir]` for both single-file and directory mode — DESIGN.md §CLI.
- `bento config <path>` resolves and prints config with per-layer provenance — DESIGN.md §CLI. (`render.rs`.)
- `--overwrite` / `-f` shorthand and `--on-existing={warn,skip-silently,overwrite,fail}` — DESIGN.md §CLI flags.

### Schema parsing (parsed and merged, even if not all are wired into the encoder yet)
- `[output]` — container, destination, preserve_chapters, on_existing, metadata block, naming block.
- `[video]` — encoder, crf, tune, preset, crop (scalar + table forms), deinterlace, detelecine, denoise, resolution, never_upscale, warn_crf_codec_mismatch.
- `[audio]` — section defaults + per-track fields (source, lang, title, default, forced, original, commentary, hearing_impaired, visual_impaired), normalize_mix, warn_no_default.
- `[subtitles]` — per-track routing, warn flags, filter spec, soft-track metadata fields.

### Pipeline
- Source probing via ffmpeg: stream enumeration and crop detection — DESIGN.md §Probe. (`pipeline/probe.rs`.)
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

---

## In progress

Features with substantive code in place but missing wiring, edge cases, or the last mile to be usable end-to-end.

### Configuration
- **Baked-in defaults layer** — DESIGN.md §Defaults. Defaults layer plumbing exists in `resolve.rs` but the actual default *values* aren't populated; resolution currently relies on a global config being present.
- **Global config bootstrap template** — DESIGN.md §Bootstrap. Template text in `bootstrap.rs` is comprehensive; **not yet invoked** by any command (waiting on `bento check`).
- **Required-field detection** — DESIGN.md §Validation. `audio.tracks` is checked; no general mechanism yet for other conditionally-required fields (e.g., naming template requiring metadata).

---

## Not started

Listed in rough priority order: MVP-completion items first, then UX/control flags, then deferred subcommands.

### CLI control & visibility flags
- `--verbose` / `--quiet` verbosity levels — DESIGN.md §CLI flags.
- `--no-warn-X` family + `--no-warnings` suppression flags — DESIGN.md §Warning suppression.
- `--dry-run` / `-n` plan-without-write mode — DESIGN.md §CLI flags.
- `--keep-intermediates` to preserve the temp dir — DESIGN.md §CLI flags.
- `--generate-config` to write a sidecar capturing CLI overrides — DESIGN.md §Sidecar generation.
- `--set KEY=VALUE` generic dotted-path overrides — DESIGN.md §CLI flags.

### Deferred subcommands
- **`bento check [-y]`** — ffmpeg presence + version detection, global config bootstrap, optional binary download — DESIGN.md §`bento check`. (`cli.rs:79` is a stub.)
- **`bento repair`** — insert missing fields into an existing global config — DESIGN.md §`bento repair`. (`cli.rs:98` is `unimplemented!`.)

---

## Backlog / nice-to-have

Anything explicitly deferred in DESIGN.md or surfaced as future work. Move items here from "Not started" if a session concludes they're out of scope for MVP.

- *(empty — populate as items get explicitly deprioritized)*

---

## Open questions / scope drift

Things in the code that don't cleanly map back to DESIGN.md, or design decisions that may have shifted. Resolve these before they accumulate.

*(no open questions currently)*

*Resolved 2026-05-19:*
- ~~ffmpeg-only vs HandBrakeCLI~~ — confirmed intentional. DESIGN.md updated (see §Background, "second note on lineage") to record the pivot to pure ffmpeg.
- ~~`subtitles.rs` scope mismatch~~ — actual surprise was that ASS parsing/serializing/filter/subtract/conversion had quietly landed while the module header still claimed Phase 5a (SRT-only). Header rewritten to match what's shipped; stale HandBrake `--srt-file` reference removed.

---

*Last updated: 2026-05-19.*
