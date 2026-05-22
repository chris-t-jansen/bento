# Bento — Design Document

A configuration-driven CLI tool for converting and re-encoding video files for hosting via Jellyfin. Designed primarily for anime, with escape hatches for other content.

This document captures the design decisions made before implementation begins. It is the source of truth for the project's intended shape; details not yet decided are called out explicitly.

---

## Background and Motivation

The project began as `convert.sh`, a Bash pipeline orchestrating `ffmpeg` and `HandBrakeCLI` to produce Jellyfin-friendly MP4 files. The script works, but its interface is configured per-invocation through a long list of CLI flags. In practice, settings vary heavily across source releases but are stable within a given show — meaning the natural unit of configuration is *per-show*, not *per-invocation*. The current design forces the user to re-derive (or remember) the right flag combination every time.

Bento exists to fix that mismatch. Per-show settings live in a config file alongside the show; the CLI becomes a thin runner over that config. One-offs and overrides remain possible via CLI flags.

A secondary motivation is escaping macOS's bundled Bash 3.2, which has driven a number of awkward workarounds in the current script (array-emptiness guards, `set +e` dances around piped commands, etc.). Rust eliminates these entirely.

A note on lineage: while `convert.sh` is the practical antecedent, Bento's schema is being designed **domain-up** rather than ported field-for-field. The script accreted features per show as new edge cases appeared; transferring that shape directly would bake its technical debt into the new tool. Where the script's pipeline shape still makes sense (extract → derive → transcode → mux), it's preserved; where its rigid two-track / one-spoken-output model was too narrow, it's been generalized.

A second note on lineage — on the encoder backend: early Bento prototypes invoked `HandBrakeCLI` for transcode, mirroring `convert.sh`'s split between ffmpeg and HandBrake. That split has since been retired. Bento now drives ffmpeg directly for every encode, mux, and filter operation. HandBrake's CLI surface imposed enough constraints (limited filter composition, opaque error reporting, awkward subtitle handling) that the ffmpeg-native equivalents — `bwdif` for motion-adaptive deinterlacing, `loudnorm` for surround-to-stereo dialogue normalization, `nlmeans` / `hqdn3d` for denoise — were a net win once identified. References elsewhere in this doc reflect the ffmpeg-only model; the HandBrake mention here exists to record the pivot, not the current architecture.

---

## Identity

- **Tool name:** Bento
- **Binary name (on user's `PATH`):** `bento`
- **Crate name on crates.io:** TBD — `bento`, `bento-box`, and `bento-cli` are taken; `bento-bin` is a leading candidate. The crate name and binary name are independent (`Cargo.toml` can declare any crate name while `[[bin]]` names the executable `bento`).
- **Config filename (directory-level):** `bento.toml`
- **Config filename (per-file):** `<videofile>.bento.toml` (sidecar pattern)
- **Config filename (global):** `config.toml` under the OS-appropriate XDG path (`~/.config/bento/` on Linux/macOS, `%APPDATA%\bento\` on Windows). The `directories` crate handles per-OS resolution.

---

## Tech Stack

- **Language:** Rust
- **Config format:** TOML 1.1 (uses multi-line inline tables, which were added in 1.1)
- **Key crates (anticipated):**
  - `serde` + `toml` (≥1.1.x with `+spec-1.1.0` support) — config parsing and (eventually) generation
  - `clap` (derive macros) — CLI parsing and `--help` output
  - `directories` — XDG-compliant config paths across platforms
  - **ASS and SRT parsing:** hand-rolled in `crate::subtitles`. Earlier plan was to use `oxideav-ass` (with `ass-core` as fallback), but the operations Bento actually needs — parse, filter by style, subtract by timestamp, lossy ASS→SRT, canonical serialize — turned out small enough that a dependency wasn't a clear win, and avoiding it sidestepped pinning to an early-stage external (`oxideav-ass` was v0.0.3 at the time of the original write-up). The hand-rolled implementations live alongside their tests in one module.

---

## Distribution

Bento is distributed via crates.io. Users run `cargo install <crate>` (final crate name TBD). The compiled binary is small; no large assets are bundled into the crate.

A pre-built binary distribution channel (e.g. `cargo-binstall`, GitHub release archives, Homebrew) may be considered later, but is deliberately out of scope for now: requiring users to install a separate tool just to install Bento is friction Bento should avoid unless a compelling reason emerges.

### External dependencies (`ffmpeg`)

Bento depends on `ffmpeg` (and its bundled tools, including `ffprobe`) for all encoding, decoding, probing, and filter operations. It is *not* bundled into the crate. Reasoning:

- crates.io has a per-crate size limit (10 MB default) that makes binary bundling impractical.
- `cargo install` builds from source, so bundled binaries would require build-script gymnastics.

Bento expects `ffmpeg` to be available on the user's `PATH`. The `bento check` subcommand verifies presence and version; if `ffmpeg` is missing, Bento prints platform-appropriate install hints (Homebrew on macOS, `apt`/`dnf` on Linux, the ffmpeg.org download page elsewhere) and exits non-zero. Bento does **not** download `ffmpeg` itself.

The first-run-download pattern (as used by Playwright, esbuild, and similar tools) was considered and deferred. Auto-downloading raises a cluster of design questions Bento doesn't yet have answers to: which build to fetch (static vs. shared, which fork), where to cache it (XDG data dir? per-user?), how to update it, and how to surface trust/provenance for a binary the tool fetched on the user's behalf. The install-hint route punts those questions to the platform package manager, which has already answered them. Revisit if the friction of "go install ffmpeg" turns out to be a real adoption barrier.

### Version checking

When external binaries are detected, Bento compares their versions against:

- A **pinned minimum version** (oldest known to work).
- A **tested-against version** (what CI builds against).

Warning bands:

| Detected version | Behavior |
|---|---|
| Below pinned minimum | Warn loudly |
| Between minimum and tested | Silent |
| Same major as tested | Silent |
| Major version above tested | Warn (potentially breaking under SemVer) |

Version checking runs on `bento check`. It does *not* run on every encode invocation, to avoid startup latency.

---

## Configuration Model

### Layering

Configuration resolves through a **linear chain** with highest-priority-wins precedence per field:

```
CLI flags  >  per-file config  >  directory config  >  global config  >  built-in defaults
```

- Every settable field is `Option<T>` at every layer until final resolution.
- **Leaf-level resolution.** Each scalar field's resolved value comes from the highest-precedence layer that sets it. For table-typed fields (e.g. `[video.encoder]`, `[output.metadata]`), individual fields within the table resolve independently — `encoder = { crf = 22 }` at directory level overrides only `crf`, leaving `name` and `tune` to fall through to lower layers. Inline tables and standard tables are equivalent for resolution purposes; the syntactic choice is presentation only.
- **Lists replace wholesale.** Array-of-table fields (`audio.tracks`, `subtitles.tracks`) cannot decompose into stable leaf paths the way scalar tables can — there's no consistent address for "the second track" across layers, since layers can disagree on how many tracks exist. So a higher layer that provides a list replaces the lower layer's list entirely.
- **Sum-typed fields take their form from the highest layer that sets them.** Some fields accept either a scalar (e.g. `crop = "none"`) or a table (e.g. `crop = { top = 60 }`). When layers disagree on which form is used, the highest layer wins outright, fully shadowing lower layers — there is no merging across forms. Within a single form, normal leaf-level resolution applies.
- There is no conditional logic or cross-config referencing. The resolution algorithm is a fold over the layer chain.
- Every field is overridable at every layer. Users who put inappropriate settings in inappropriate layers (e.g. `tracks` in the global config) are trusted to know what they're doing.
- **Cross-field validation.** Some fields are coupled in ways structural rules can't enforce — e.g. `encoder.crf` values are scaled differently for `encoder.name = "x264"` vs `"x265"`. These couplings are enforced at validation time after resolution, with errors or warnings identifying the suspected mistake. Validation rules are documented alongside the relevant schema sections.

### Section-level cascade for per-track fields

Some sections (`[audio]`, `[subtitles]`) contain a list of tracks plus section-level fields that act as **per-track defaults**. This is a second, smaller cascade nested inside the layer cascade: the section-level value applies to every track that doesn't set the field explicitly; an explicit per-track value wins.

This pattern is what lets the global config carry sensible defaults (e.g. `encoder = "aac"`, `bitrate = 192`) without ever needing to know about specific tracks, while directory configs supply the track list and individual tracks override defaults only where they need to.

### Global config bootstrap

On first run, Bento generates `<XDG>/bento/config.toml` containing every default setting written out and commented with documentation. Users edit this file once to set their personal defaults (preferred CRF, container, audio bitrate, sub language, etc.) and rarely touch it again.

Bento does *not* auto-update the global config on future upgrades. Once it exists, the user owns it.

### Required fields

Some fields have built-in defaults and are always resolvable. Others (e.g. track indices for a specific source) have no sensible default. If a required field cannot be resolved through the layer chain, Bento errors out with a message identifying the missing field and exits.

### Visibility mechanisms

To prevent silent drift between layers, five mechanisms are built in:

1. **Default-mode summary line** per file: a one-liner before each encode showing how many settings came from each layer (e.g. `episode06.mkv: 8 settings (3 from file, 4 from directory, 1 from global, 0 from baked-in defaults)`). Cheap, quiet, continuously informative. A nonzero count from baked-in defaults triggers the missing-setting warning described below.

2. **`--verbose` flag**: expands the summary into a full per-field provenance table, naming the source layer (and file path, where applicable) for every resolved setting.

3. **Redundancy warnings (on by default):** when a higher-precedence layer sets a field to the *same value* a lower-precedence layer already sets, Bento warns and names both files so the stale override can be deleted. The same warning fires when a list-typed field (`audio.tracks`, `subtitles.tracks`) is redeclared at a higher layer with content fully identical to a lower layer's — a true no-op redeclaration. (Lists redeclared with any field changed are *not* flagged: per the lists-replace-wholesale rule, the user has no choice but to redeclare the whole list to change one field, and warning about the inevitable would be a slap in the face.) A `--no-warn-redundant` flag disables both forms. A real override (different value) is not flagged.

4. **`config` subcommand.** Resolves all configs in a directory (or for a single file) without encoding and prints the resolved settings for each file with full provenance. Useful for catching drift after editing a directory config.

5. **Missing-setting warnings (on by default):** when a field's resolved value comes from the baked-in default layer rather than any user-facing config (global, directory, per-file, or CLI), Bento warns. The warning lists each missing field and its baked-in value, and recommends adding it to the global config. Suppressed by `--no-warn-missing`. Rationale: the global config is *expected* to contain every default after first-run bootstrap, so missing fields are anomalous — either the user removed them, or Bento was upgraded and gained new fields not yet present in the user's config. The `repair` subcommand populates missing fields in the global config in bulk. This warning applies only to fields that have baked-in defaults; truly required fields with no default (e.g. `audio.tracks`) error out at config-resolution time per the Required fields rule.

### Warnings policy

Warnings fall into two classes, with different toggle semantics:

**Config-implication warnings** fire because of what the user's configuration implies. They fire on every encode in perpetuity unless the configuration changes or the warning is suppressed. Each gets a section-level boolean field (default `true`), with a parallel `--no-warn-X` CLI flag. CLI-only suppression here would be annoying — the user would have to flip the flag on every run forever for an intentional configuration.

**Runtime warnings** fire because of something specific to a single encode — source-file anomalies, fallbacks Bento took because something didn't match expectations, drift between configuration layers. These do *not* get config toggles; suppression is CLI-only via `--no-warn-X` flags. Persistent suppression would mean "I don't want to know when something is wrong about this run," which contradicts the visibility-first design.

**Consistency principle.** Where `[audio]` and `[subtitles]` share track-level semantics (disposition fields, the track-list shape), their warnings and errors mirror each other: multiple `default = true` is a hard error in both; no `default = true` triggers `warn_no_default` in both. Other dispositions (`forced`, `commentary`, `hearing_impaired`, etc.) allow multiples in both sections — they're category flags, not singletons. Concepts unique to one section (burn rendering and format conversion in subtitles; mixdown in audio) carry section-specific warnings and that asymmetry is fine.

**Warnings index.** All warnings Bento emits:

| Warning | Class | Toggle |
|---|---|---|
| Multiple `mux = "burn"` subtitle tracks | config-implication | `[subtitles].warn_multiple_burns` |
| Burn subtitle track with soft-only metadata fields | config-implication | `[subtitles].warn_burn_metadata` |
| Lossy ASS→SRT format conversion | config-implication | `[subtitles].warn_ass_to_srt` |
| No subtitle track marked `default = true` | config-implication | `[subtitles].warn_no_default` |
| No audio track marked `default = true` | config-implication | `[audio].warn_no_default` |
| `encoder.crf` value suspicious for resolved `encoder.name` | config-implication | `[video].warn_crf_codec_mismatch` |
| Field resolved from baked-in default rather than user config | runtime | `--no-warn-missing` (CLI-only) |
| Higher-precedence layer sets a field to the same value as a lower layer | runtime | `--no-warn-redundant` (CLI-only) |
| Sidecar config already exists when `--generate-config` is passed | runtime | none (intentional exception) |

The "sidecar exists" warning is the documented exception to the runtime-warning suppression rule: it has no dedicated `--no-warn-X` flag. It fires only when the user has explicitly opted in via `--generate-config`, so a flag to suppress a warning about your own flag not working would be silly. The bulk `--no-warnings` flag (see CLI Surface) does suppress it, however — that flag is the explicit "silence everything for this run" affordance, and the per-warning-flag exception logic doesn't extend to bulk suppression.

CLI flag names for config-implication warnings are settled in the Warning suppression section under CLI Surface; the config field names above are stable.

---

## Configuration Schema

This section defines the shape and semantics of the config sections. Field names and section structure are stable; details deferred to implementation are called out inline.

### `[output]`

Everything determining the output file that isn't a stream encoding decision: container format, file location and name, embedded metadata, conflict resolution. The encoding sections (`[audio]`, `[subtitles]`, `[video]`) handle stream content; `[output]` handles the file itself.

**Fields:**

- `container` — `"mp4"` (default) or `"mkv"`. MP4 maximizes Jellyfin direct-play compatibility on Pi clients. MKV supports more subtitle formats natively (e.g. soft ASS) but at the cost of client compatibility. Lives here rather than in `[video]` because it's a packaging decision affecting subtitle muxing and audio codec compatibility, not just video.
- `destination` — where output files are written. Default `"."` (alongside the source). Relative paths resolve against *the source file's directory*, not the working directory — `destination = "encoded"` puts each output in an `encoded/` subdir next to its input. Absolute paths are absolute. Bento creates the directory if it doesn't exist.
- `preserve_chapters` — `true` (default) or `false`. When `true`, chapter markers in the source are copied to the output.
- `on_existing` — what to do when the target output file already exists. One of:
  - `"warn"` (default) — print a warning, leave the existing output in place, move on to the next file. Loud `skip_silently`.
  - `"skip_silently"` — leave the existing output in place silently, move on.
  - `"overwrite"` — replace the existing output without warning.
  - `"fail"` — stop the entire run.
- `metadata` — inline table of tags embedded in the output container. Fixed schema; only the fields below are recognized. All optional; absent fields aren't written. Containers Bento supports (MP4, MKV) all map these cleanly. For richer tagging needs, post-process with mkvpropedit or AtomicParsley — Bento's metadata is the lowest-common-denominator set Jellyfin actually uses to corroborate filename-based scraping.
  - `show` — series title (string)
  - `season` — season number (integer)
  - `year` — release year (integer)
- `naming` — inline table controlling output filenames. Optional; if absent, output filenames mirror source filenames with the extension changed to match `container`. Fields:
  - `regex` — a regular expression matched against each source filename (without extension). Named captures `(?P<name>...)` become available as template variables. If `regex` is set and fails to match a given source file, that file errors at encode time. Both `(?P<name>...)` and `(?<name>...)` syntaxes are accepted.
  - `template` — the output filename (without extension). References variables in `{name}` form. Format specifiers like `{name:02}` (zero-padded integer width 2) work on integer-typed values: metadata fields typed as numbers, and regex captures that parse cleanly as integers. Referencing an undefined variable is a config error.
  
  Available template variables:
  - Any non-table field from `metadata` (e.g. `{show}`, `{season}`, `{year}`).
  - `{source_basename}` — the source filename without extension.
  - `{source_dir}` — the name of the source's containing directory.
  - Any named capture from `regex`.

**Auto-derived metadata: episode numbers.** When `naming.regex` includes a capture named `episode` or `ep`, Bento embeds the captured value as the per-file episode-number tag (MP4 `tves`, MKV `PART_NUMBER` at episode target). This is how per-file metadata is supplied without per-file config: one regex extracts the right episode number from each filename in the directory. If both `episode` and `ep` are present, `episode` wins. Captures with any other name (`e`, `num`, etc.) are template-only and are not embedded.

**Example:**

```toml
[output]
container = "mp4"
destination = "encoded"
preserve_chapters = true
on_existing = "warn"
metadata = { show = "Cowboy Bebop", season = 1, year = 1998 }
naming = {
    regex = 'S(?P<s>\d+)E(?P<episode>\d+)',
    template = "{show} - S{s:02}E{episode:02}",
}
```

A source file `Cowboy Bebop S01E06 [BD 1080p].mkv` produces an output file `Cowboy Bebop - S01E06.mp4`, written to `<source-dir>/encoded/`, with embedded tags for show, season, year, and episode 6.

### `[video]`

A single section governing the video stream. Unlike `[audio]` and `[subtitles]`, video has no track list — there is one video stream in, one video stream out. All fields are section-level.

**Core encoding:**

- `encoder` — inline table grouping encoder choice with the fields whose meaning depends on it.
  - `name` — `"x264"` or `"x265"`. Default `"x264"`. x264 maximizes Pi-and-browser direct-play compatibility; x265 produces ~30% smaller files at equivalent perceptual quality but encodes 3-5× slower at equivalent presets and narrows the direct-play client set (Pi 4+ only, modern browsers only).
  - `crf` — Constant Rate Factor: Bento's only rate-control mode. Lower values produce higher quality and larger files. "Transparent" output is around 18 for x264 and 20-22 for x265. **Codec-dependent default:** `20` when `name = "x264"`, `22` when `name = "x265"`. The two scales are not interchangeable — x265 CRF 28 produces roughly the same perceptual quality as x264 CRF 23 — so the default shifts with `name` rather than asking the user to know the conversion. CBR and 2-pass rate control are out of scope; CRF is the right answer for an encode-once-store-forever workflow.
  - `tune` — psychovisual optimization. Default `"animation"`. Accepted values depend on `name`:
    - `name = "x264"`: `film`, `animation`, `grain`, `stillimage`, `psnr`, `ssim`, `fastdecode`, `zerolatency`, `none`.
    - `name = "x265"`: `animation`, `grain`, `psnr`, `ssim`, `fastdecode`, `zerolatency`, `none`.
    Mismatched combinations (e.g. `tune = "film"` with `name = "x265"`) are config errors caught at validation time. Bento normalizes the spelling internally; users always write `fastdecode` and `zerolatency` regardless of encoder.

  Per the schema's leaf-level resolution rule, individual fields within `encoder` resolve from the highest layer that sets them: writing `encoder = { crf = 22 }` at directory level overrides only `crf`, leaving `name` and `tune` to fall through to the global config (or, failing that, baked-in defaults). This makes single-field overrides ergonomic but introduces a coupling hazard — a user who writes `encoder = { name = "x265" }` at directory level inherits `crf` from below, which is likely scaled for x264. The hazard is caught by **cross-field validation**: at config-resolution time, Bento checks that the resolved `crf` is in a reasonable range for the resolved `name` (an x264-typical CRF ≤19 paired with x265, or an x265-typical CRF ≥24 paired with x264) and warns identifying the likely error. Suppressed by `[video].warn_crf_codec_mismatch = false` or `--no-warn-crf-codec-mismatch`. Validation catches the actual bug — incompatible codec/CRF combination — rather than the structural proxy of "encoder fields didn't all come from the same layer."
- `preset` — speed/quality tradeoff. Accepted values: `ultrafast`, `superfast`, `veryfast`, `faster`, `fast`, `medium`, `slow`, `slower`, `veryslow`, `placebo`. Default `"medium"`. Slower presets test more encoding options for better compression at the same quality, but the gains shrink quickly: `veryslow` over `medium` is roughly 8-10× the encode time for ~5% smaller files at the same CRF. `placebo` doubles encode time over `veryslow` for negligible gain and is widely considered impractical. Users who care about every byte will set this to `slow` or `veryslow`; `medium` is the friendlier default for first runs against a large library. Preset names are identical between x264 and x265, so `preset` stays a top-level field rather than moving into the encoder table.

**Source preprocessing:** Defaults to `"none"` for every preprocessing field. Bento does not modify the source video unless explicitly requested — consistent with the "video in = video out" principle.

- `crop` — black bar removal.
  - `crop = "none"` (default) — no crop.
  - `crop = "auto"` — autodetect via ffmpeg's `cropdetect` filter on a sample of frames. Convenient but unreliable on dark scenes (over-crops) and on content with intermittent letterboxing.
  - `crop = { top = 138, bottom = 138, left = 0, right = 0 }` — explicit pixel values. Any side may be omitted; absent sides default to 0. So `crop = { left = 138, right = 138 }` is a valid pillarbox-only crop, and `crop = { top = 60, bottom = 60 }` is a valid letterbox-only crop.
- `deinterlace` — for sources where the *content itself* was shot interlaced (sports, soap operas, pre-progressive broadcasts).
  - `"none"` (default), `"auto"`, `"yadif"`, or `"bwdif"`.
  - `yadif` is ffmpeg's standard deinterlacer. `bwdif` (Bob Weaver Deinterlacing Filter) is ffmpeg's motion-adaptive option and generally behaves better on mixed-content sources — the closest functional analog to HandBrake's old `decomb` filter, which earlier Bento prototypes exposed. `auto` lets Bento pick (currently maps to `yadif`).
- `detelecine` — inverse telecine (IVTC), for content that was *24fps but broadcast at 30fps via 3:2 pulldown*. Most pre-Blu-ray anime broadcast in NTSC regions falls into this category.
  - `"none"` (default) or `"auto"`.
- `denoise` — noise reduction. Generally avoid on clean modern sources; useful for old broadcast captures with analog noise.
  - `denoise = "none"` (default).
  - `denoise = { filter = "nlmeans", preset = "light" }` — Non-Local Means filter (higher quality, slower).
  - `denoise = { filter = "hqdn3d", preset = "medium" }` — High-Quality 3D denoiser (faster, less aggressive).
  - Each filter accepts presets `ultralight`, `light`, `medium`, `strong`, `stronger`, `verystrong`. Bento maps these to the underlying ffmpeg filter's parameter set (filter strength, spatial/temporal weighting); see ffmpeg's `nlmeans` and `hqdn3d` documentation for the exact picture impact at each level.
- `resolution` — output resolution.
  - `resolution = "original"` (default) — match source dimensions.
  - `resolution = { width = 1280, height = 720 }` — explicit dimensions.
  - Preset forms like `"720p"` were considered and rejected: the height-anchored interpretation interacts poorly with non-standard aspect ratios (1024×576, 720×480, anamorphic anime sources), and resolving those interactions would require schema-level rules that are clearer expressed by writing the dimensions out.
- `never_upscale` — when `true` (default), `resolution` settings that would enlarge the source are ignored, leaving the source dimensions untouched. Safety net for global configs that set a target resolution applied across mixed-resolution sources. Set to `false` only if upscaling is genuinely intended (rare; usually only for output normalization when downstream tooling requires uniform dimensions).

**Choosing between deinterlace and detelecine:** Critical distinction; applying the wrong operation corrupts the file. Deinterlacing telecined content destroys the 24fps cadence and produces visible judder; inverse-telecining true interlaced content either no-ops or makes things worse.

- Pause the source during motion. Combing on every frame → interlaced (use `deinterlace`). Combing on roughly 2 of every 5 frames in a regular pattern → telecined (use `detelecine`).
- Source framerate of 29.97fps could be either. After successful IVTC, the recovered framerate should be 23.976fps (the original film cadence). If you can't reach 23.976 cleanly, the content wasn't telecined.
- Era heuristic for anime: pre-Blu-ray broadcast anime is usually telecined (24fps animation broadcast at 30fps NTSC). DVD-era is often telecined, sometimes hard-telecined (pattern baked into the encoding but still recoverable by IVTC). Blu-ray-era anime is usually already progressive 23.976fps with no preprocessing needed.
- When in doubt, `auto` for the suspected operation does a reasonable job for the common cases.

**Example: global config:**

```toml
[video]
encoder = { name = "x264", crf = 20, tune = "animation" }
preset = "medium"
crop = "none"
deinterlace = "none"
detelecine = "none"
denoise = "none"
resolution = "original"
never_upscale = true
```

**Example: directory config for an old DVD-era anime release:**

```toml
[video]
detelecine = "auto"
crop = { top = 60, bottom = 60 }
```

The directory config inherits all the encoding parameters (the full `encoder` table, `preset`) from the global config. It applies inverse telecine to recover the 24fps source cadence and crops the 60-pixel letterbox bars. Resolution stays at source dimensions.

### `[audio]`

A single section governing all audio output. Contains section-level defaults and a list of output tracks.

**Section-level fields that cascade as per-track defaults:**

- `encoder` — target codec for audio output: `"aac"`, `"opus"`, `"flac"`, etc. Default `"aac"`. There is no explicit "copy" value; see "Copy vs. transcode" below.
- `bitrate` — target bitrate in kbps for transcoded output. Default `192`. Ignored when Bento copies the source stream rather than transcoding.
- `mixdown` — `"stereo"`, `"5point1"`, `"mono"`, or `"dpl2"`. Default `"stereo"`.
- `force_bitrate` — when `true`, transcode if source bitrate exceeds the target `bitrate`. Default `false` (bitrate alone never triggers transcoding).
- `force_mixdown` — when `true`, transcode if source channel layout differs from `mixdown`. Default `true` (a configured `mixdown` is treated as a requirement).

**Section-only fields (do not cascade — not per-track concepts):**

- `normalize_mix` — apply ffmpeg's `loudnorm` filter to combat quiet-dialogue artifacts in surround-to-stereo downmixes. Default `true` (anime dialogue is the use case).
- `warn_no_default` — when `true` (default), Bento warns if no track in the list is marked `default = true`. Without a default, Jellyfin (or whichever player) falls back to its own track-selection logic, which may not match user expectation. Suppressed by setting to `false`.

**Per-track fields:**

- `source` (required) — source audio track index in the input file.
- `lang` — ISO 639 language code. Required for sane Jellyfin track-selection behavior.
- `title` — user-facing label ("Japanese", "English Dub", "Director's Commentary").
- `default` — auto-play disposition. At most one track per stream type may set this; multiple `default = true` tracks is a hard config validation error. If no track sets it, see `warn_no_default`.
- `forced` — "play this track even when audio is set to a different language" disposition; for tracks like a primarily-English dub where certain scenes need a specific track switched in (foreign-language dialogue, song lyrics in original language).
- `original` — marks the original-language track (e.g. Japanese for an anime). Useful for Jellyfin's track-selection logic when the user prefers original-language audio.
- `commentary` — marks the track as commentary (director's commentary, cast commentary). Surfaced in Jellyfin's UI as such.
- `hearing_impaired` — disposition for dialogue-emphasized mixes intended for hard-of-hearing listeners. Rare on audio (the role is more often filled by SDH subtitles), but valid in both MP4 and MKV.
- `visual_impaired` — audio description track for blind or low-vision viewers. Standard MKV `flag-visual-impaired` and the MP4 equivalent.
- `encoder`, `bitrate`, `mixdown`, `force_bitrate`, `force_mixdown` — override section-level defaults.

**Multiple-disposition behavior:** Only `default` is uniqueness-enforced. The other dispositions (`forced`, `original`, `commentary`, `hearing_impaired`, `visual_impaired`) are category flags, not singletons — multiple tracks may set them. Multilingual content, multi-commentary releases, and audio descriptions in multiple languages all legitimately produce multi-flagged outputs.

**Copy vs. transcode:**

Bento decides automatically whether to copy the source audio stream or re-encode it. Three dimensions can independently trigger transcoding:

- **Codec mismatch** — always triggers transcoding. If the source codec doesn't match `encoder`, the stream must be re-encoded.
- **Mixdown mismatch** — triggers transcoding when `force_mixdown = true` (default) and the source channel layout differs from the configured `mixdown`. Setting `force_mixdown = false` makes `mixdown` advisory: it applies if transcoding happens for other reasons, but does not itself force a transcode.
- **Bitrate exceeds target** — triggers transcoding when `force_bitrate = true` and the source bitrate is higher than the configured `bitrate`. Defaults to `false`; with the default, `bitrate` is a target *for transcoding only* and never triggers a transcode on its own. Source bitrate *below* target never triggers a transcode regardless of this flag — there is no benefit to re-encoding lossy audio at a higher bitrate.

If any trigger fires, Bento transcodes using the configured `encoder`, `bitrate`, and `mixdown`. If none fire, Bento copies the source stream.

The asymmetric defaults reflect a real asymmetry in user intent. A user who writes `mixdown = "stereo"` is usually expressing a hard requirement ("I want stereo output") rather than a preference contingent on transcoding happening for other reasons; defaulting `force_mixdown` to `true` preserves that meaning. By contrast, `bitrate = 192` is more often a target ceiling than a hard requirement — re-encoding 320kbps AAC to 192kbps AAC saves ~12 MB per 24-minute episode at the cost of lossy-to-lossy quality degradation, a poor trade by default. Users who do want the storage cap can opt in via `force_bitrate = true`.

**Example: global config (defaults only, no tracks):**

```toml
[audio]
encoder = "aac"
bitrate = 192
mixdown = "stereo"
force_bitrate = false
force_mixdown = true
normalize_mix = true
```

**Example: directory config (track list, sparing overrides):**

```toml
[audio]
tracks = [
    { source = 1, lang = "jpn", title = "Japanese", default = true, original = true },
    { source = 2, lang = "eng", title = "English Dub" },
    { source = 3, lang = "eng", title = "Director's Commentary", commentary = true, bitrate = 96 },
]
```

The directory config inherits `encoder`, `bitrate`, `mixdown`, `force_bitrate`, `force_mixdown`, and `normalize_mix` from the global config. The commentary track overrides `bitrate` for itself only; the other two tracks use the section default. With the default flags (`force_mixdown = true`, `force_bitrate = false`), source AAC tracks already in stereo are copied directly regardless of source bitrate; AAC tracks in 5.1 are transcoded down to stereo; non-AAC tracks are transcoded to AAC.

### `[subtitles]`

A single section governing all subtitle output. Each track in the list independently specifies how it's derived from the source and how it appears in the output. This generalizes the original script's rigid "burn signs + soft spoken" pair into an arbitrary list of tracks, each with its own derivation.

**Section-only fields (do not cascade — not per-track concepts):**

- `warn_multiple_burns` — when `true` (default), Bento warns if more than one track has `mux = "burn"`. Multiple burn layers are supported (foreign-language hardsubs alongside signs is a legitimate case) and rendered in declaration order, but the configuration is also a common misfire — the warning catches the unintended case while letting the deliberate case suppress the noise by setting the field to `false`. The CLI flag `--no-warn-multiple-burns` is equivalent.
- `warn_burn_metadata` — when `true` (default), Bento warns if any burn track has soft-track metadata fields set (`lang`, `title`, `default`, `forced`, `commentary`, `hearing_impaired`). Burn tracks are rendered as pixels and have no metadata channel in the output, so these fields have no effect — the warning catches likely-unintended configurations (e.g. a track that was switched from soft to burn but still carries leftover metadata). Suppressed by setting to `false` or via `--no-warn-burn-metadata`.
- `warn_no_default` — when `true` (default), Bento warns if no track in the list is marked `default = true`. Without a default, Jellyfin (or whichever player) falls back to its own track-selection logic, which may not match user expectation. Suppressed by setting to `false`. Symmetric with `[audio].warn_no_default`.
- `warn_ass_to_srt` — when `true` (default), Bento warns when a track is processed in ASS but emitted as SRT (`format = "srt"` against an ASS source, with or without a `filter` operation). The conversion is lossy — styling, positioning, fonts, and effects are stripped to plain text — but it's a legitimate operation for extracting plain dialogue from a styled track. Suppressed by setting to `false`.

**Per-track fields — routing:**

- `source` (required) — either an **integer** (input subtitle track index in the source MKV) or a **string** (path to an external subtitle file). Multiple output tracks can share a source.
- `format` — output format: `"srt"` or `"ass"`. For soft tracks, this is the encoding of the muxed stream. For burn tracks, this is the data passed to the libass renderer (which in turn determines whether styling is preserved).
- `mux` — `"soft"`, `"burn"`, or `"external"`. Soft tracks are muxed into the output container; burn tracks are rendered onto the video as pixels via libass; external tracks are written as sidecar files next to the output video for player-side discovery, targeting Jellyfin's external-track feature. See "External subtitle tracks" below.

**File-path sources:** When `source` is a string, Bento reads the subtitle data from that file rather than extracting it from the input MKV. This supports workflows where users hand-edit a track (fixing typos, rewording, retiming) and want the encode to use the edited version.

- **Path resolution.** Paths are resolved relative to the config file that contains them, not relative to CWD or the input video. Drop your edited `.srt` next to the `bento.toml` that references it and the reference works regardless of where Bento is invoked from. (Absolute paths are fine and are used as-is.)
- **Format detection.** By extension: `.srt` → SRT, `.ass` / `.ssa` → ASS. Unknown extensions are a hard error at config-validation time — no content-sniffing fallback, since silent guessing on subtitle formats is the kind of thing that fails once and is never debugged.
- **Validation.** Missing files, unreadable files, and a `filter` block applied to an SRT source are all caught at config-validation time, before any encoding starts.

**Per-track fields — derivation (mutually exclusive):**

A track may have **at most one** of the following derivation operations. Either or neither is fine; both is a config error.

- `filter` — keep or drop dialogue events based on ASS style attributes.
  - `style` — match by ASS style name (e.g. `"Main"`, `"Signs"`).
  - `font` — match by font name.
  - `size` — match by font size.
  - Multiple match keys are AND-ed.
  - `mode` — `"retain"` keeps only matching events; `"remove"` drops them.
  - Valid only when input is ASS.
- `subtract_track` — drop events from this track whose `(start, end)` timestamps exactly match an event in another track. Value is either an **integer** (track index in the source MKV) or a **string** (file path), with the same semantics and path-resolution rules as `source`. Matching is on the `(start, end)` pair as a unit; partial overlaps and timestamp shifts are not matched. The intended use is the canonical "full minus signs = dialogue-only" case, where the signs track is a literal subset of the full track with identical timing.

**Per-track fields — soft-track metadata:**

- `lang` — ISO 639 language code.
- `title` — user-facing label.
- `default` — auto-display disposition. At most one track per stream type may set this; multiple `default = true` tracks is a hard config validation error. If no track sets it, see `warn_no_default`.
- `forced` — "show this even when subs are off" disposition; for forced-narrative tracks (foreign-language scenes in an otherwise native-language work), distinct from signs.
- `commentary` — marks the track as commentary (director's running notes, production notes). Surfaced in Jellyfin's UI as such.
- `hearing_impaired` — SDH/CC disposition. Surfaced in Jellyfin's UI as "SDH"/"CC".

**Multiple-disposition behavior:** Only `default` is uniqueness-enforced. The other dispositions (`forced`, `commentary`, `hearing_impaired`) are category flags, not singletons — multiple tracks may set them. SDH in multiple languages, separate forced tracks for different scene types, and multi-commentary releases all legitimately produce multi-flagged outputs.

These fields apply to soft and external tracks. On burn tracks, they have no output effect (burn tracks are pixels, not muxed streams) and Bento warns at config/encode time per `warn_burn_metadata`. On external tracks, `lang`, `title`, `default`, `forced`, and `hearing_impaired` are mapped onto Jellyfin's sidecar filename tokens (see "External subtitle tracks" below); `commentary` has no representation in the filename convention and is dropped.

**External subtitle tracks (`mux = "external"`):**

Writes the derived track as a sidecar file next to the output video rather than into the container or onto the video stream. Targets Jellyfin's [external subtitle and audio tracks](https://jellyfin.org/docs/general/server/media/shows#external-subtitles-and-audio-tracks) feature, in which the player reads sidecar `.srt`/`.ass` files from disk and presents them in the track-selection UI alongside container streams.

The motivation is that external tracks are easier to edit, re-time, or replace after the encode — no remux required — and they sidestep the codec-compatibility constraints of MP4 (which only supports a limited subtitle codec set natively). Soft mux remains the default; external is opt-in for users who specifically want player-side files.

**Filename construction.** Bento builds the sidecar filename from the resolved output video's basename and the track's existing metadata, following Jellyfin's `<basename>.<title?>.<lang?>.<flags?>.<ext>` pattern:

- **Basename** — the output video's basename (per `[output].destination` and `[output].naming`).
- **`title`** — included verbatim if set; omitted if not.
- **`lang`** — included as the resolved ISO 639 code if set; omitted if not.
- **Flags** — derived from per-track booleans: `default = true` → `default`, `forced = true` → `forced`, `hearing_impaired = true` → `sdh`. `sdh` is chosen over Jellyfin's other accepted synonyms (`cc`, `hi`) to sidestep the `hi`-as-Hindi ambiguity called out in Jellyfin's docs.
- **Extension** — `.srt` or `.ass`, matching the resolved `format`.

For example, a track with `lang = "eng"`, `title = "English"`, `default = true`, `format = "srt"` against output `episode06.mp4` produces `episode06.English.eng.default.srt`.

**Commentary disposition is dropped.** Jellyfin's filename convention has no commentary flag, and Bento doesn't synthesize one — users who want the track labeled as commentary should set `title = "English Commentary"` (or similar) directly. The `commentary` boolean remains valid and effectful on soft tracks; only external ignores it.

**Format conversion**, `filter`, and `subtract_track` all apply to external tracks identically to soft. `warn_ass_to_srt` still fires when `format = "srt"` is requested against an ASS source.

**Overwrite policy and uniqueness.** Sidecars are written into the same directory as the output video. The `[output].on_existing` policy applies to collisions with files left over from previous runs the same way it applies to the main output file. Within a single run, sidecar filename uniqueness is verified at config-validation time alongside the existing `default = true` uniqueness check; two external tracks that would resolve to the same filename is a hard config error.

**Format model:**

Three logically distinct formats are involved in subtitle processing. Bento exposes only one of them as config:

1. **Input format** — auto-detected from the source MKV. No config needed.
2. **Processing format** — implicit. If a `filter` is configured, the input must be ASS (Bento validates this) and processing happens in ASS. Otherwise, a simpler representation is used as needed.
3. **Output format** — controlled by the `format` field. If processing was in ASS but `format = "srt"`, conversion happens at the end. This is lossy — styling is lost — and Bento warns at encode time per `warn_ass_to_srt`, but does not refuse, since extracting plain dialogue from a styled track is a legitimate use case.

**Example: typical anime episode (two-track source).** The classic case — the source MKV ships a full dialogue+signs track (track 1) and a signs-only track (track 2). Bento produces a burned signs track plus a soft spoken-only track derived by subtracting signs from the full track:

```toml
[subtitles]
tracks = [
    {
        source = 1,
        format = "srt",
        mux = "soft",
        subtract_track = 2,
        lang = "eng",
        title = "English",
        default = true,
    },
    {
        source = 2,
        format = "ass",
        mux = "burn",
    },
]
```

**Example: single-track source with style-based split.** A fansub release ships one ASS track containing both dialogue and signs, distinguished by ASS style name. Bento splits it into a burned signs track and a soft dialogue track derived from the same input via complementary filters:

```toml
[subtitles]
tracks = [
    {
        source = 1,
        format = "srt",
        mux = "soft",
        filter = { style = "Main", mode = "retain" },
        lang = "eng",
        title = "English",
        default = true,
    },
    {
        source = 1,
        format = "ass",
        mux = "burn",
        filter = { style = "Main", mode = "remove" },
    },
]
```

Both tracks derive from `source = 1`; the `retain` filter keeps dialogue for soft mux, and the complementary `remove` filter keeps everything else (signs) for burn-in. The schema makes this case as natural as the two-track-source case.

**Example: multiple soft sub tracks with mixed dispositions.**

```toml
[subtitles]
tracks = [
    {
        source = 1,
        format = "srt",
        mux = "soft",
        filter = { style = "Main", mode = "retain" },
        lang = "eng",
        title = "English (Official)",
        default = true,
    },
    {
        source = 2,
        format = "srt",
        mux = "soft",
        subtract_track = 3,
        lang = "eng",
        title = "English (Fansub, SDH)",
        hearing_impaired = true,
    },
    {
        source = 1,
        format = "ass",
        mux = "burn",
        filter = { style = "Main", mode = "remove" },
    },
]
```

**Example: hand-edited dialogue track.** A typo in the source dialogue track has been fixed by hand and saved as `episode06.dialogue.srt` next to the `bento.toml`. The signs track is still extracted from the MKV:

```toml
[subtitles]
tracks = [
    {
        source = "episode06.dialogue.srt",
        format = "srt",
        mux = "soft",
        lang = "eng",
        title = "English",
        default = true,
    },
    {
        source = 2,
        format = "ass",
        mux = "burn",
    },
]
```

The hand-edited file is read directly with no derivation applied (no `filter`, no `subtract_track`); it goes through format conversion if needed and then to soft mux. Because per-file edits are typical in this workflow, the natural place for this kind of override is in a `<videofile>.bento.toml` sidecar config, not the directory-level config.

**Example: external sidecar tracks for Jellyfin direct access.** Same two-track source as the canonical case, but Bento writes the dialogue track as a sidecar `.srt` next to the output video rather than muxing it. Signs stay burned in.

```toml
[subtitles]
tracks = [
    {
        source = 1,
        format = "srt",
        mux = "external",
        subtract_track = 2,
        lang = "eng",
        title = "English",
        default = true,
    },
    {
        source = 2,
        format = "ass",
        mux = "burn",
    },
]
```

If the output video resolves to `episode06.mp4`, Bento writes `episode06.English.eng.default.srt` next to it. Jellyfin picks the file up automatically and exposes it in the track-selection UI alongside any container-internal streams.

### Runtime concerns are CLI-only

`[pipeline]` was considered and dropped. Items that originally lived there split into two groups:

- **Output-file decisions** (skip-if-output-exists) moved into `[output]` as `on_existing`. They're sticky preferences about the output artifact.
- **Per-invocation behavior** (dry-run, log verbosity, intermediate file retention) is CLI-flag-only. These aren't preferences a user has across encodes — they're switches flipped for a single run, typically for debugging. Putting them in config encourages stale state ("why is dry-run still on?") and adds layering rules to settings that don't benefit from layering. The CLI surface (`--dry-run`, `--verbose`/`--quiet`, `--keep-intermediates`, etc.) is documented under CLI Surface > Run-behavior flags / Verbosity.

---

## Pipeline (generalized from `convert.sh`, ported to Rust)

The conversion pipeline is no longer a fixed sequence of "extract two specific tracks, diff, transcode, mux one specific spoken track back." Generalized to match the schema:

1. **Extract** every input track referenced by `source` in the audio and subtitle config.
2. **Derive** each output subtitle track per its config: apply `filter` or `subtract_track` as specified, convert format if necessary.
3. **Transcode** with ffmpeg per the `[video]` and `[audio]` config (multi-track audio supported; video preset/tune/quality/etc.). Burn-mux'd subtitle tracks are rendered into the video stream via libass at this stage.
4. **Mux** all soft subtitle tracks back into the output container with their declared dispositions. Tracks with `mux = "external"` are written as sidecar files next to the output video at this stage rather than muxed into the container; the filename follows Jellyfin's external-track convention (see `[subtitles]` > "External subtitle tracks").

### Subtitle logic: Python → Rust

The current script embeds Python heredocs for SRT parsing and diffing. These are being ported to Rust using `oxideav-ass` (or fallback) for ASS handling. Reasoning:

- The logic is small (SRT parsing is straightforward; the derivation operations are well-bounded).
- Porting eliminates the `python3` runtime dependency.
- Rust's string handling and explicit error paths suit this kind of code well.
- A self-contained Rust binary plus two external tools is a much cleaner distribution story than Rust + Python interpreter + Python script + two external tools.

### Encoding targets and rationale

The schema defaults — H.264 video, AAC stereo audio, signs-burned-and-spoken-soft subtitles, MP4 container — are tuned for one goal: *Jellyfin direct-play on a Raspberry Pi*. The Pi has limited CPU available for playback-time transcoding, so the encoding pipeline produces files no Pi-side transcoding is needed for. Every default trades encoding-side cost (which happens on a beefier machine where it doesn't matter) for playback-side cheapness (which happens on the Pi where it matters a lot).

The schema does not enforce these defaults — any combination is allowed — but they are what anime users targeting Jellyfin on Pi will typically reach for. Users targeting different clients (Apple TV with a real receiver, dedicated HTPCs, browsers with HEVC support) will find different defaults make more sense and can override accordingly.

The Jellyfin codec support reference is at <https://jellyfin.org/docs/general/clients/codec-support/>.

---

## CLI Surface

The CLI is a thin override layer over the config; the four subcommands cover everything Bento does. The full surface — subcommand structure, override mechanism, `--generate-config`, run-behavior flags, verbosity, warning suppression, and output-existence handling — is settled. A small set of per-subcommand niceties (e.g. `--output FILE` for `bento config` to redirect output to a file) remains to be designed during implementation.

### Subcommands

#### `convert`

Usage: `bento convert <path> [output_dir]`

The primary command — runs the full conversion pipeline.

- If `<path>` is a file, runs the conversion for that file using the resolved config (file-specific sidecar, directory `bento.toml`, global config, baked-in defaults, in cascade order).
- If `<path>` is a directory, runs the conversion for every valid video file in that directory (non-recursive), resolving configs per file.
- The optional `[output_dir]` overrides `[output].destination` for this run.

**Path resolution.** Both positional arguments resolve relative to the current working directory, in contrast with `[output].destination` in config files (which resolves relative to the source file's directory). The mismatch is intentional: CLI args are typed at the shell against tab-completion and shell idioms, while config field values travel with the configs and source files they describe.

**Valid video files.** In directory mode, Bento picks up files by extension whitelist: `.mkv`, `.mp4`, `.m4v`, `.avi`, `.mov`, `.webm`, `.ts`, `.m2ts`, `.wmv`. The whitelist is hardcoded; users with non-standard extensions can invoke `bento convert <specific_file>` directly. No content-probing is done up front — malformed files surface their errors when ffmpeg reaches them, via the per-file error path described below. Sidecar configs (`<videofile>.bento.toml`) and external subtitle files (`.srt`, `.ass`, `.ssa`) are excluded automatically by virtue of not matching the whitelist; no special-case logic needed.

**Batch failure semantics.** Files in a directory are processed sequentially. Per-file errors (config validation failure, source unreadable, ffmpeg non-zero exit) are logged and the batch continues with the next file. Environmental errors that would affect every remaining file (missing external binary, disk full, root output dir unwritable) abort the run. At the end of the batch, Bento prints a summary:

```
8 succeeded, 2 failed:
  episode06.mkv: ffmpeg exited non-zero (see log above)
  episode11.mkv: required field audio.tracks not resolvable
```

Exit code is non-zero if any file failed. The summary is always shown regardless of verbosity — if you ran a batch, you almost always want to know whether it worked. The continue-by-default behavior matches user expectation for a "kick off the encode and walk away" workflow; a `--fail-fast` flag could be added later if real demand surfaces.

#### `config`

Usage: `bento config <path>`

Resolves the full config for `<path>` and prints the result with full provenance — every resolved field annotated with the layer (and file path) it came from. Does not encode anything. The transparency tool for "what does Bento think the settings are for this file/show?"

If `<path>` is a directory, resolves and prints for every valid video file in that directory.

#### `check`

Usage: `bento check [-y]`

Verifies that Bento's external dependencies are present and usable: `ffmpeg` (with version check against the pinned minimum and tested version) and the global `config.toml`. If `ffmpeg` is missing, Bento prints platform-appropriate install hints and exits non-zero. If the global config is missing, Bento offers via prompt to generate it. This is the explicit re-entry point after the user has accidentally broken their global config, upgraded `ffmpeg` to an unsupported version, etc.

The `-y` / `--yes` flag auto-confirms the global-config generate prompt. In non-interactive contexts (no TTY) without `-y`, `check` errors instead of hanging — matching the behavior of `apt`, `dnf`, etc.

#### `repair`

Usage: `bento repair`

Repairs the global `config.toml`. Scans the existing global config for keys expected to be present per the current schema (i.e. fields with baked-in defaults), prints which keys are missing, and offers to insert them at their default values with documentation comments — closing the loop on the missing-setting warnings (see Visibility mechanisms).

If the global config is corrupt or unparseable, `repair` warns and offers to regenerate it from scratch.

### Override semantics

CLI flags act as the highest-precedence configuration layer, above all config files. Override semantics follow two rules:

- **Dedicated flags clobber wholesale.** A dedicated flag that maps to a top-level config key replaces the resolved value at that path. The flag set is small by design (the config is the primary interface); dedicated flags exist for common one-offs that benefit from short forms or convenience. The full list (`--overwrite`/`-f`, `--on-existing`, the `--no-warn-X` family) is documented in the sections that follow.
- **`--set KEY=VALUE [KEY=VALUE...]`** — the generic override mechanism. Accepts one or more `KEY=VALUE` pairs in a single flag invocation. `KEY` is a dotted path into the config schema (e.g. `video.encoder.crf`, `audio.bitrate`, `output.metadata.show`). `VALUE` is parsed as a TOML scalar (`true`, `42`, `"quoted string"`, etc.).
  - Resolves at the leaf level, consistent with the schema's general resolution rule. `--set video.encoder.crf=22` overrides only `crf` in the encoder table; `name` and `tune` fall through to lower layers.
  - **Lists are not addressable via `--set`.** Track lists (`audio.tracks`, `subtitles.tracks`) cannot be modified per-track via CLI; if the path resolves to a list, Bento errors with a message pointing to the `<videofile>.bento.toml` sidecar pattern as the right tool for per-file track tweaks.
  - **Strict TOML value parsing.** The string after `=` is parsed as a TOML scalar: `true` is a bool, `42` is an int, `"text"` is a quoted string, bare strings are not accepted. Parse errors get specific error messages identifying the likely fix (typically, "did you mean to quote this string?"). Friendliness lives in the error messages, not the parser.

### `--generate-config`

A flag on `bento convert`. Writes a config file capturing the difference between resolved-with-CLI-overrides and resolved-without-CLI-overrides — i.e. it captures only what the CLI flags changed, not the full resolved config.

- For a file target, writes `<videofile>.bento.toml` next to the source.
- For a directory target, writes `<dir>/bento.toml`.
- The conversion still runs; `--generate-config` is an additive side effect, not a mode switch.
- If no CLI overrides were passed, errors before the conversion starts: there's nothing to write.
- If the target sidecar/config already exists, **warns and continues** without overwriting. The conversion runs to completion regardless. To regenerate, delete the existing file and re-run.

The "warns and continues" behavior reflects a general principle: flag failures shouldn't fail the run when the flag's effect is auxiliary to the primary operation. `--generate-config` is auxiliary; `convert` is primary.

### Run-behavior flags

Two flags control whether and how a `convert` invocation actually runs.

#### `--dry-run` / `-n`

Resolves config, runs cross-field validation, and prints the *plan* — what Bento would do if `--dry-run` weren't passed. Exits without filesystem effects: no encodes, no temp dirs created, no `--generate-config` writes, no output files touched.

The plan is rendered as per-file prose, not a machine-parseable structure (the audience is a human deciding "is this what I want?"):

```
$ bento convert --dry-run ./
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
episode02.mkv:
  ...
2 files would be processed. 0 config errors.
Run `bento config <path>` to see where each setting resolved from.
```

The discovery footer pointing at `bento config` is shown by default; suppressed under `--quiet`.

The unique value of `--dry-run` over `bento config` is source-aware decisions: copy-vs-transcode per audio track depends on probing the source file's actual codec/channels/bitrate, which `bento config` deliberately doesn't do. `--dry-run` answers "what *would* happen"; `bento config` answers "what settings will be used and where do they come from."

Validation errors surface during dry-run and contribute to a non-zero exit code — catching unresolvable required fields, suspected codec/CRF mismatches, etc. before committing to a multi-hour encode is the point of the flag.

Under `--generate-config`, `--dry-run` reports "Would write sidecar at <path>" but does not write — preserving the no-filesystem-effects contract. Under `-v`, `--dry-run` additionally prints the actual ffmpeg command lines that would run.

#### `--keep-intermediates`

Bento extracts subtitle tracks, derives subtitle outputs (filtering, subtraction, format conversion), and possibly produces other transient artifacts during the pipeline. By default these intermediates live in a system temp directory and are cleaned up automatically at the end of the run (success, error, or panic) via Rust's `tempfile` crate's drop semantics.

Layout: per-run subdirectory with per-file sub-subdirectories.

```
/tmp/bento-<random>/         # per-run, via tempfile::TempDir
  ├── episode01/             # per source file (sanitized basename)
  │   ├── track-1-extracted.ass
  │   ├── signs-derived.ass
  │   └── ...
  ├── episode02/
  │   └── ...
```

System temp is the prevailing idiom (`make`, `cargo`, `git`, ffmpeg's `-passlogfile`), inherits the OS's cleanup policies, and respects `$TMPDIR` for users who need a roomier mount. No config knob; the env var is the universal escape hatch.

The `--keep-intermediates` flag suppresses the cleanup. At the end of the run, Bento prints the path:

```
Intermediate files preserved at: /tmp/bento-aBcDef
```

No short form (debug-tier flag, used rarely enough that long form is fine). Under `--dry-run`, `--keep-intermediates` is a silent no-op (dry-run produces nothing to keep). On batch partial success, the temp dir is preserved as-is — both successful and failed files' intermediates remain inspectable.

### Verbosity

Three discrete levels. `--verbose` and `--quiet` are mutually exclusive; passing both is a CLI error. No stacked levels (`-vv`, `-qq`) — three is enough for the natural tiers of useful detail, and richer levels can be added later via `clap`'s `ArgAction::Count` without reshaping the design.

| Level | Errors | Warnings | Per-file output | Encoder progress | Provenance/commands |
|---|---|---|---|---|---|
| `--quiet` / `-q` | yes | yes | none | none | none |
| (default) | yes | yes | layer-count summary line | brief Bento-styled line, in-place if TTY | none |
| `--verbose` / `-v` | yes | yes | full provenance table | full ffmpeg passthrough | command lines per file |

A few things worth pinning:

- **Warnings are decoupled from verbosity.** `-q` does not suppress warnings — that's what the `--no-warn-*` family is for (see Warning suppression). `-q` controls routine narration; warnings flag specific anomalies and have their own off-switches. Conflating them would mean a user running scripted with `-q` could silently miss a real warning.
- **TTY detection.** The default-mode progress line uses carriage-return updates if stdout is a TTY, falling back to one-line-per-update otherwise (so `bento convert ./ > log.txt` produces a sensible log instead of `\r`-mangled garbage).
- **Run summary always shown.** The end-of-batch `8 succeeded, 2 failed` line is shown in all modes including `-q`. If you ran a batch, you almost always want to know whether it worked.
- **Verbose replaces, not augments, the layer-count summary.** Under `-v`, each file gets the full provenance table per the existing visibility-mechanism design; the layer-count line is omitted because it's already a summary of the table being shown.

### Warning suppression

Each warning in the warnings index has a corresponding `--no-warn-X` CLI flag, mirroring the `warn_X` config-field suffix. Plus a bulk `--no-warnings` flag that suppresses everything in one shot.

| Class | Config field | CLI flag |
|---|---|---|
| config-implication | `[subtitles].warn_multiple_burns` | `--no-warn-multiple-burns` |
| config-implication | `[subtitles].warn_burn_metadata` | `--no-warn-burn-metadata` |
| config-implication | `[subtitles].warn_ass_to_srt` | `--no-warn-ass-to-srt` |
| config-implication | `[audio].warn_no_default` & `[subtitles].warn_no_default` | `--no-warn-no-default` |
| config-implication | `[video].warn_crf_codec_mismatch` | `--no-warn-crf-codec-mismatch` |
| runtime | (resolved-from-default) | `--no-warn-missing` |
| runtime | (redundant-override; single-field or whole-list) | `--no-warn-redundant` |
| bulk | n/a | `--no-warnings` |

A few specific properties:

- **The `warn_no_default` collision.** `[audio].warn_no_default` and `[subtitles].warn_no_default` are separately addressable in config but share a single CLI flag (`--no-warn-no-default`) that suppresses both. Coarser CLI than config is the standard idiom — config is for sticky preferences with full granularity, CLI flags are one-off interventions where less precision is fine. A user who hits one warning is plausibly hitting both (shared root cause: dispositions forgotten while writing the config).
- **No positive `--warn-X` forms.** All warnings default to on; the only direction users normally flip is off. For the rare case of re-enabling a warning that's been disabled in config, the generic `--set audio.warn_no_default=true` mechanism works.
- **CLI-only for the bulk flag.** `--no-warnings` is deliberately not mirrored as a config field. Persistent "shut up about everything forever" is state a user would set once and forget, masking real issues silently. Forcing the per-run choice keeps the noise-vs-signal tradeoff visible each invocation.
- **The bulk flag *does* suppress the sidecar-exists warning.** That warning has no dedicated `--no-warn-X` flag (per the visibility design's documented exception), but `--no-warnings` is the explicit "silence everything for this run" affordance and the per-warning-flag exception logic doesn't extend to bulk suppression.

Composition: multiple `--no-warn-X` flags compose freely; `--no-warnings` is equivalent to passing every individual flag. For config-implication warnings, CLI takes precedence over config field per the general layering rule. Verbosity (`-q`/`-v`) is independent — it does not affect warnings either way.

**Implementation note for later.** Within a single batch, one warning often fires repeatedly across files (e.g., a directory of 12 episodes all hitting `warn_no_default` because the directory config never set a default). Worth deduplicating: present as one summary line per warning type with a count and affected-file list, instead of 12 identical multi-line warnings. Implementation detail, not a design decision.

### Output-existence flag

CLI mirror of `[output].on_existing`, plus a shorthand for the most common case.

- **`--on-existing=VALUE`** — full mirror, where VALUE is one of `warn`, `skip-silently`, `overwrite`, `fail`. Takes precedence over the config field per the general layering rule.
- **`--overwrite`** / **`-f`** — shorthand equivalent to `--on-existing=overwrite`. The "rerun this batch with new settings, blow away the old outputs" case comes up often enough to deserve a dedicated form, and `-f` is well-established (matching `cp -f`, `rm -f`) with `--force` as the dominant interpretation. The only check Bento has to "force through" is the existence check, so the meaning is unambiguous in context.

The shorthand pair (`--overwrite`, `-f`) does not have parallel forms for the other three values. Adding `--skip-existing` and `--fail-on-existing` was considered and skipped: those cases are less common, and adding them would invite the "why these but not the fourth?" question. The canonical `--on-existing=VALUE` covers them.

**Mutual exclusion.** Passing both `--overwrite`/`-f` and `--on-existing=VALUE` is a CLI error regardless of whether the values would agree. Consistency over micro-optimization: any combination of the two flag families is rejected, no exception for tautological cases.

**CLI value casing.** CLI uses kebab-case (`skip-silently`) where the config field uses snake_case (`skip_silently`); the value parser maps between them. This matches CLI idiom — strict consistency with the config's casing would require both `--on_existing=skip_silently` (snake_case in the flag name itself) which is wildly non-idiomatic, and Rust's `clap` would resist it. The generic `--set output.on_existing="skip_silently"` mechanism still uses the snake_case form because it's setting a raw TOML value, so users who reach for `--set` will see the snake form there; users who reach for the dedicated flag will see the kebab form. Small surface inconsistency; acceptable.

**Composition.** Orthogonal to `--generate-config` (different artifacts: video output vs. sidecar config). Under `--dry-run`, the planned action is reported per resolved value ("Would overwrite episode01.mp4", "Would skip episode01.mp4 (already exists)") with no actual writes.

### Pending design

Per-subcommand niceties to be designed during implementation:

- `--output FILE` for `bento config` to redirect the resolved-config output to a file rather than stdout.
- Any analogous redirects for `bento check` / `bento repair` if their interactive output benefits from logging.

These are minor surface and don't warrant a dedicated design pass; ad-hoc resolution during implementation is fine.

---

## Open Questions (deferred)

- **Wrong-directory protection.** Pointing `bento convert` at the wrong directory — most concerningly, a previous run's output directory — can cause silent re-encoding of already-finished files: wasted time and irrecoverable quality loss with no way to tell once-encoded from twice-encoded files after the fact. Candidates considered: per-output metadata stamps with input-side detection (most targeted, but interacts poorly with same-container iterative-settings workflows where users *want* to re-encode their previous outputs); confirmation prompts for directory mode (heavyweight, trains users into reflexive `-y`); pre-flight summary lines (loud but not blocking); input-side extension filters (orthogonal — solves a different mistake). No clear winner; revisit when more user-facing data is in.
- **Per-subcommand niceties.** `--output FILE` for `bento config` and any analogous output-redirection flags. Minor surface, ad-hoc during implementation.
- **Crate name on crates.io.** Pending availability check and personal preference.
- **Exact pinned-minimum version for `ffmpeg`.** Determined empirically.
- **Behavior of the default summary line** — whether it's truly default-on, or behind a flag. Currently planned as default-on.
- **Pre-built binary distribution.** Deliberately deferred; revisit only if a compelling reason emerges.
- **External audio tracks.** Jellyfin's external-track feature also supports sidecar audio (`.mp3`, `.aac`, `.dts`, etc.) using the same filename convention as external subtitles. Adding `mux = "external"` to `[audio]` would be symmetric with the subtitle case (already in the schema for `[subtitles]`) and could reuse the same filename construction. Deferred from v1: the canonical anime use case is well-served by muxed audio, and the surrounding design questions (transcode targets for external streams, source-stream copy semantics, interaction with `force_bitrate` / `force_mixdown`) didn't have clear answers under the same time budget. Revisit if a real use case emerges; the schema delta is small.
- **Resolved-config data shape.** Internally, every settable field is `Option<T>` at every layer until resolution; after resolution, fields-with-defaults could be guaranteed non-None and represented as `T`, pushing the "is this set?" question into the type system and removing `.unwrap_or(default)` calls scattered through the encoder builder. Truly required no-default fields stay `Option<T>` until the missing-field check fires. The cost is a parallel `ResolvedConfig` type maintained alongside the layer-level `Config`. Deferred until the encoder builder stabilizes — the call-site shape will be clearer then, and the `.unwrap_or` pattern is annoying but not broken in the meantime.

---

## Out of Scope (for now)

- Bundling `ffmpeg` into the crate. Explicitly rejected; users install `ffmpeg` themselves via their platform package manager, and `bento check` surfaces install hints if it's missing.
- Hardware-accelerated encoding (NVENC, QuickSync, VideoToolbox). Not relevant to current goals; could be added later.
- Recursive directory traversal. Current script is non-recursive; no reason to change that yet.
- A daemon/watch mode. Out of scope.
- **Chained subtitle derivations** — deriving track B from already-derived track A. The current schema only allows references to source tracks, not to other derived tracks. Not in v1; possible future feature.
- **Content-based subtitle subtraction.** The current `subtract_track` operation matches events by `(start, end)` timestamp pair only. A content-based mode (drop events whose text appears in another track) was considered and dropped from v1: the canonical use case is timestamp-based, the design questions around content matching (strip ASS override tags? normalize whitespace? case-fold? handle SRT vs ASS differences?) lacked clear answers, and no current workflow demands it. If a real use case emerges, the natural shape is a sum-typed `subtract_track` field — `subtract_track = 3` stays valid for the timestamp case, and `subtract_track = { from = 3, mode = "content" }` would be the content form, fitting the existing scalar-or-table pattern (e.g. `crop`).
- **HDR → SDR tone mapping and color-space handling.** Increasingly relevant as HDR anime ships, but well-understood territory that can be added later without schema disruption.
- **Parallel batch encoding.** Encoding multiple files in a directory in parallel rather than sequentially. The video encoders Bento drives (libx264, libx265) are already multithreaded *within* a single encode, so the wins require either underused cores or I/O-bound workloads — and the design questions around log interleaving, memory pressure with two concurrent encodes, and partial-failure semantics deserve their own pass. Additive when wanted (a `--parallel N` flag layered over the existing batch loop).

---

## Next Step

The design is settled. Schema, CLI surface (subcommands, override mechanism, `--generate-config`, run-behavior flags, verbosity, warning suppression, output-existence handling), pipeline shape, and distribution model are all in place. Open Questions tracks the items deliberately deferred; Out of Scope tracks what's intentionally not being built.

Implementation order and architecture are intentionally not captured in this document — they have different stability profiles than design and belong in a separate plan / issue tracker / `ARCHITECTURE.md` once the code starts to take shape.
