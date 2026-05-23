+++
title = "Installation"
description = "How to install Bento and its external dependencies."
weight = 2
+++

## Prerequisites

Bento requires `ffmpeg` (including `ffprobe`) available on your `PATH`. It does not bundle or download `ffmpeg` itself.

| Platform | Recommended method |
|---|---|
| macOS | `brew install ffmpeg` |
| Ubuntu / Debian | `sudo apt install ffmpeg` |
| Fedora / RHEL | `sudo dnf install ffmpeg` |
| Windows / other | [ffmpeg.org/download](https://ffmpeg.org/download.html) |

## Install Bento

Bento is distributed as source. You'll need [Rust](https://rustup.rs/) installed.

```sh
git clone https://github.com/chris-t-jansen/bento.git
cd bento
cargo build --release
```

The compiled binary lands at `target/release/bento`. Copy it somewhere on your `PATH`:

```sh
# Linux / macOS
cp target/release/bento ~/.local/bin/

# or wherever your PATH includes
```

## Verify the install

```sh
bento check
```

This confirms `ffmpeg` is present and at a supported version, and generates your global config at `~/.config/bento/config.toml` (Linux/macOS) or `%APPDATA%\bento\config.toml` (Windows) if it doesn't exist yet. If `ffmpeg` is missing, Bento prints platform-appropriate install hints.

See [`bento check`](/cli/check) for the full reference.
