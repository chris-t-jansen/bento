+++
title = "Video"
description = "Encoder choice, CRF, presets, and source preprocessing — crop, deinterlace, detelecine, denoise, resize."
weight = 3

[extra]
toc_strip_signatures = true
+++

The `[video]` section governs the single video stream. Unlike `[audio]` and `[subtitles]`, there is no track list — one video stream in, one video stream out. All fields are optional and resolve from the layer cascade.

```
[video]
# Warnings
warn_crf_codec_mismatch = <bool>

# Encoding
encoder = {
    name = "x264" | "x265"
    crf  = <integer>
    tune = "film" | "animation" | "grain" | "stillimage" | "psnr" | "ssim" | "fastdecode" | "zerolatency" | "none"
}
preset = "ultrafast" | "superfast" | "veryfast" | "faster" | "fast" | "medium" | "slow" | "slower" | "veryslow" | "placebo"

# Source preprocessing
crop          = "none" | "auto" | { top = <integer>, bottom = <integer>, left = <integer>, right = <integer> }
deinterlace   = "none" | "yadif" | "bwdif" | "auto"
detelecine    = "none" | "auto"
denoise       = "none" | {
    filter = "nlmeans" | "hqdn3d"
    preset = "ultralight" | "light" | "medium" | "strong" | "stronger" | "verystrong"
}
resolution    = "original" | { width = <integer>, height = <integer> }
never_upscale = <bool>
```

## Fields

### `warn_crf_codec_mismatch = <bool>` {#warn_crf_codec_mismatch}

Warn when the resolved `crf` looks scaled for the other codec — an x264 encode with `crf ≥ 24`, or an x265 encode with `crf ≤ 19`. Default `true`.

Because the x264 and x265 CRF scales aren't interchangeable, an inherited `crf` can silently produce the wrong quality/size tradeoff — e.g. setting `encoder = { name = "x265" }` at the directory level while a low-layer `crf = 18` (an x264-transparent value) falls through. The warning catches this common cascade mistake and suggests the codec the value was likely written for.

### `encoder = <inline table>` {#encoder}

Groups the encoder and its dependent settings. Individual fields resolve independently through the layer cascade — `encoder = { crf = 22 }` in a directory config overrides only `crf`, leaving `name` and `tune` to fall through from lower layers.

- **`name = <option>`**
    - The video codec: `"x264"` (default) or `"x265"`. x264 maximizes Raspberry Pi and browser direct-play compatibility. x265 produces roughly 30% smaller files at equivalent perceptual quality, but encodes 3–5× slower at equivalent presets and narrows the direct-play client set (Pi 4+ only, modern browsers with HEVC support only).
- **`crf = <integer>`**
    - Constant Rate Factor — Bento's only rate-control mode. Default `20` for x264, `22` for x265. Lower values produce higher quality and larger files. "Transparent" quality is around CRF 18 for x264 and CRF 20–22 for x265. The two scales are not interchangeable (x265 CRF 28 ≈ x264 CRF 23 perceptually), which is why the defaults differ. See [`warn_crf_codec_mismatch`](#warn_crf_codec_mismatch) for the cascade safety check.
- **`tune = <option>`**
    - Psychovisual optimization. Default `"animation"` (appropriate for most anime sources). Accepted values depend on `name`: **x264** allows `film`, `animation`, `grain`, `stillimage`, `psnr`, `ssim`, `fastdecode`, `zerolatency`, `none`; **x265** allows `animation`, `grain`, `psnr`, `ssim`, `fastdecode`, `zerolatency`, `none`. Mismatched combinations (e.g. `tune = "film"` with `name = "x265"`) are caught at validation time.

### `preset = <option>` {#preset}

Speed/quality tradeoff. Default `"medium"`. Accepted values, fastest to slowest: `ultrafast`, `superfast`, `veryfast`, `faster`, `fast`, `medium`, `slow`, `slower`, `veryslow`, `placebo`.

Slower presets test more encoding options for better compression at the same quality. The gains diminish quickly at the slow end — `veryslow` over `medium` is roughly 8–10× the encode time for ~5% smaller files. For a large library, `medium` is a sensible starting point; raise it for archival encodes where you want the best compression.

### `crop = "none" | "auto" | <inline table>` {#crop}

Remove black bars. Default `"none"` — Bento does not crop unless explicitly requested.

- **`"none"`** — No crop.
- **`"auto"`** — Autodetect via ffmpeg's `cropdetect` filter on a sample of frames. Convenient but unreliable on dark scenes and content with intermittent letterboxing.
- **`<inline table>`** — Explicit pixel values `{ top, bottom, left, right }`, each `<integer>`. Any side may be omitted; absent sides default to 0.

```toml
crop = { top = 60, bottom = 60 }          # letterbox only
crop = { left = 138, right = 138 }        # pillarbox only
crop = { top = 138, bottom = 138, left = 0, right = 0 }
```

### `deinterlace = <option>` {#deinterlace}

For sources where the content itself was captured interlaced (sports, soap operas, pre-progressive broadcasts). Default `"none"`. See [Deinterlace vs. detelecine](#deinterlace-vs-detelecine) for choosing between the two operations.

- **`"none"`** — No deinterlacing.
- **`"yadif"`** — ffmpeg's standard deinterlacer.
- **`"bwdif"`** — Motion-adaptive deinterlacer; generally better on mixed-content sources.
- **`"auto"`** — Bento chooses (currently maps to `yadif`).

### `detelecine = <option>` {#detelecine}

Inverse telecine (IVTC), for content that was 24fps but broadcast at 30fps via 3:2 pulldown. Most pre-Blu-ray anime broadcast in NTSC regions falls into this category. Default `"none"`. See [Deinterlace vs. detelecine](#deinterlace-vs-detelecine).

- **`"none"`** — No IVTC.
- **`"auto"`** — Apply IVTC to recover the original 24fps cadence.

### `denoise = "none" | <inline table>` {#denoise}

Noise reduction. Generally avoid on clean modern sources; useful for old broadcast captures with analog noise. Default `"none"`.

- **`"none"`** — No denoise.
- **`<inline table>`** — `{ filter, preset }`:
    - **`filter = <option>`** — `"nlmeans"` (Non-Local Means — higher quality, slower) or `"hqdn3d"` (High-Quality 3D denoiser — faster, less aggressive).
    - **`preset = <option>`** — `ultralight`, `light`, `medium`, `strong`, `stronger`, or `verystrong`.

### `resolution = "original" | <inline table>` {#resolution}

Output resolution. Default `"original"`.

- **`"original"`** — Match source dimensions.
- **`<inline table>`** — Explicit dimensions `{ width, height }` in pixels, each `<integer>`.

### `never_upscale = <bool>` {#never_upscale}

When `true` (default), `resolution` settings that would enlarge the source are ignored, leaving the source dimensions untouched. Set to `false` only if upscaling is genuinely intended.

## Behavior

### Deinterlace vs. detelecine

Applying the wrong operation corrupts the file. Use the wrong one and you'll either destroy the 24fps cadence (deinterlacing telecined content) or make things worse (de-telecining genuinely interlaced content).

- Pause the source during motion. Combing on **every frame** → interlaced (use `deinterlace`). Combing on **roughly 2 of every 5 frames** in a regular pattern → telecined (use `detelecine`).
- A 29.97fps source could be either. After successful IVTC, the recovered framerate should be 23.976fps.
- **Era heuristic for anime:** pre-Blu-ray broadcast is usually telecined. DVD-era is often telecined (sometimes hard-telecined). Blu-ray-era anime is usually already progressive 23.976fps and needs no preprocessing.

## Examples

**Global config:**
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

**Directory config for an old DVD-era release:**
```toml
[video]
detelecine = "auto"
crop = { top = 60, bottom = 60 }
```

This inherits all encoding parameters from the global config and applies IVTC plus letterbox crop to every file in the directory.
