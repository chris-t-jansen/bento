# Contributing to Bento

Thanks for your interest in contributing! Bento is a small, configuration-driven CLI, and contributions of all sizes are welcome — bug reports, documentation improvements, tests, and code.

Before you start, please read [DESIGN.md](DESIGN.md). It explains the architectural decisions and trade-offs behind Bento, and PRs that don't fit the design will be hard to merge without revisiting it first.

## A note on AI-assisted development

As called out in the [README](README.md), this project is AI-generated, and ongoing maintenance is likely to continue using AI assistance. You are welcome to use AI tools when contributing, but:

- You are responsible for everything in your PR. Read it, test it, and stand behind it.
- Don't paste AI output verbatim into issues or PR descriptions without reviewing it.
- Prefer small, focused PRs — they're easier to review whether a human or an AI wrote them.

## Development setup

Bento is a standard Cargo project. You'll need:

- A recent stable Rust toolchain (install via [rustup](https://rustup.rs/))
- [FFmpeg](https://ffmpeg.org/) installed and on your `PATH` (only required to actually *run* Bento — the test suite does not invoke FFmpeg)

Clone and build:

```sh
git clone https://github.com/chris-t-jansen/bento.git
cd bento
cargo build
```

## The dev loop

Before opening a PR, please run all of the following locally:

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs the same checks, so getting them green locally first will save a round trip.

To run a single test:

```sh
cargo test --test convert -- some_test_name
```

## Pull requests

- **One concern per PR.** If you find yourself writing "and also..." in the PR description, consider splitting it.
- **Write a clear PR description.** What changed, why, and any trade-offs. If it fixes an issue, link it (`Fixes #123`).
- **Add or update tests.** Most of Bento's logic is unit- or integration-testable without touching FFmpeg; see the existing tests in `tests/` for patterns.
- **Update documentation** if you change user-visible behavior (README, DESIGN.md, or `--help` text).
- **Keep the commit history reasonable.** Squash WIP commits before merge; meaningful intermediate commits are fine to keep.

## Reporting bugs

Use the [bug report issue template](.github/ISSUE_TEMPLATE/bug_report.yml). The most useful bug reports include:

- Your OS and `bento --version`
- Your FFmpeg version (`ffmpeg -version` first line is enough)
- The config file (or a minimal reproduction of it)
- The exact command you ran
- What you expected vs. what happened

## Security issues

Please don't open public issues for security vulnerabilities. See [SECURITY.md](SECURITY.md) for how to report them privately.

## License

By contributing, you agree that your contributions will be dual-licensed under the MIT and Apache 2.0 licenses, matching the rest of the project.
