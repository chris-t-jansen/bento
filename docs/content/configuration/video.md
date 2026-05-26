+++
title = "Video"
description = "Encoder choice, CRF, presets, and source preprocessing — crop, deinterlace, detelecine, denoise, resize."
weight = 3
+++

The `[video]` section governs the single video stream. Unlike `[audio]` and `[subtitles]`, there is no track list — one video stream in, one video stream out. All fields are optional and resolve from the layer cascade.

```
[video]
encoder = {
    name = "x264" | "x265"
    crf  = <integer>
    tune = "animation" | "film" | "grain" | "stillimage" | "psnr" | "ssim" | "fastdecode" | "zerolatency" | "none"
}
preset      = "ultrafast" | "superfast" | "veryfast" | "faster" | "fast" | "medium" | "slow" | "slower" | "veryslow" | "placebo"
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

## Encoding

### `encoder`

An inline table grouping the encoder and its dependent settings.

| Field | Values | Default |
|---|---|---|
| `name` | `"x264"`, `"x265"` | `"x264"` |
| `crf` | integer | `20` for x264, `22` for x265 |
| `tune` | see below | `"animation"` |

**`name`:** x264 maximizes Raspberry Pi and browser direct-play compatibility. x265 produces roughly 30% smaller files at equivalent perceptual quality, but encodes 3–5× slower at equivalent presets and narrows the direct-play client set (Pi 4+ only, modern browsers with HEVC support only).

**`crf`:** Constant Rate Factor — Bento's only rate-control mode. Lower values produce higher quality and larger files. "Transparent" quality is around CRF 18 for x264 and CRF 20–22 for x265. The two scales are not interchangeable (x265 CRF 28 ≈ x264 CRF 23 perceptually), which is why the defaults differ. If you write `encoder = { name = "x265" }` at directory level and inherit `crf` from below, Bento warns if the inherited value looks scaled for the other codec.

**`tune`:** Psychovisual optimization. Accepted values depend on `name`:

| `name` | Accepted `tune` values |
|---|---|
| `x264` | `film`, `animation`, `grain`, `stillimage`, `psnr`, `ssim`, `fastdecode`, `zerolatency`, `none` |
| `x265` | `animation`, `grain`, `psnr`, `ssim`, `fastdecode`, `zerolatency`, `none` |

Mismatched combinations (e.g. `tune = "film"` with `name = "x265"`) are caught at validation time. The default `"animation"` is appropriate for most anime sources.

**Leaf-level resolution:** Individual fields within `encoder` resolve independently. `encoder = { crf = 22 }` in a directory config overrides only `crf`; `name` and `tune` fall through to the global config.

### `preset`

Speed/quality tradeoff. Accepted values: `ultrafast`, `superfast`, `veryfast`, `faster`, `fast`, `medium`, `slow`, `slower`, `veryslow`, `placebo`. Default `"medium"`.

Slower presets test more encoding options for better compression at the same quality. The gains diminish quickly at the slow end — `veryslow` over `medium` is roughly 8–10× the encode time for ~5% smaller files. For a large library, `medium` is a sensible starting point; raise it for archival encodes where you want the best compression.

## Source preprocessing

All preprocessing fields default to `"none"`. Bento does not modify the source video unless explicitly requested.

### `crop`

Remove black bars.

| Value | Description |
|---|---|
| `"none"` (default) | No crop. |
| `"auto"` | Autodetect via ffmpeg's `cropdetect` filter on a sample of frames. Convenient but unreliable on dark scenes and content with intermittent letterboxing. |
| `{ top, bottom, left, right }` | Explicit pixel values. Any side may be omitted; absent sides default to 0. |

```toml
crop = { top = 60, bottom = 60 }          # letterbox only
crop = { left = 138, right = 138 }        # pillarbox only
crop = { top = 138, bottom = 138, left = 0, right = 0 }
```

### `deinterlace`

For sources where the content itself was captured interlaced (sports, soap operas, pre-progressive broadcasts).

| Value | Description |
|---|---|
| `"none"` (default) | No deinterlacing. |
| `"yadif"` | ffmpeg's standard deinterlacer. |
| `"bwdif"` | Motion-adaptive deinterlacer; generally better on mixed-content sources. |
| `"auto"` | Bento chooses (currently maps to `yadif`). |

### `detelecine`

Inverse telecine (IVTC), for content that was 24fps but broadcast at 30fps via 3:2 pulldown. Most pre-Blu-ray anime broadcast in NTSC regions falls into this category.

| Value | Description |
|---|---|
| `"none"` (default) | No IVTC. |
| `"auto"` | Apply IVTC to recover the original 24fps cadence. |

**Deinterlace vs. detelecine — critical distinction.** Applying the wrong operation corrupts the file. Use the wrong one and you'll either destroy the 24fps cadence (deinterlacing telecined content) or make things worse (de-telecining genuinely interlaced content).

- Pause the source during motion. Combing on **every frame** → interlaced (use `deinterlace`). Combing on **roughly 2 of every 5 frames** in a regular pattern → telecined (use `detelecine`).
- A 29.97fps source could be either. After successful IVTC, the recovered framerate should be 23.976fps.
- **Era heuristic for anime:** pre-Blu-ray broadcast is usually telecined. DVD-era is often telecined (sometimes hard-telecined). Blu-ray-era anime is usually already progressive 23.976fps and needs no preprocessing.

### `denoise`

Noise reduction. Generally avoid on clean modern sources; useful for old broadcast captures with analog noise.

| Value | Description |
|---|---|
| `"none"` (default) | No denoise. |
| `{ filter = "nlmeans", preset = "..." }` | Non-Local Means filter — higher quality, slower. |
| `{ filter = "hqdn3d", preset = "..." }` | High-Quality 3D denoiser — faster, less aggressive. |

Both filters accept the same presets: `ultralight`, `light`, `medium`, `strong`, `stronger`, `verystrong`.

### `resolution`

Output resolution.

| Value | Description |
|---|---|
| `"original"` (default) | Match source dimensions. |
| `{ width, height }` | Explicit dimensions in pixels. |

### `never_upscale`

When `true` (default), `resolution` settings that would enlarge the source are ignored, leaving the source dimensions untouched. Set to `false` only if upscaling is genuinely intended.

## Example

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
