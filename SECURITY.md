# Security Policy

## Supported versions

Bento is a small project and only the latest release on `main` is supported. If you're running an older version, please update before reporting a security issue.

## Reporting a vulnerability

**Please do not open a public GitHub issue for security vulnerabilities.**

Instead, report them privately using GitHub's [private vulnerability reporting](https://github.com/chris-t-jansen/bento/security/advisories/new). This lets us discuss and fix the issue before it becomes public.

When reporting, please include:

- A description of the issue and the impact you believe it has
- Steps to reproduce, including a minimal config or command line if applicable
- The version of Bento and FFmpeg you observed it on
- Your OS and version

You can expect an initial response within roughly one week. If the issue is confirmed, we'll work on a fix and coordinate disclosure with you.

## Scope

Things that are in scope:

- Arbitrary command execution or shell injection via crafted configs, filenames, or CLI arguments
- Path traversal that escapes the directories Bento is configured to operate on
- Crashes or hangs triggered by malicious config files (denial of service)
- Sensitive information disclosure (e.g. logging credentials, leaking absolute paths in unintended ways)

Things that are generally out of scope:

- Vulnerabilities in FFmpeg itself — please report those to the [FFmpeg project](https://ffmpeg.org/security.html)
- Issues that require the user to deliberately run a malicious Bento binary
- Bugs that only affect output quality or correctness without a security impact (file those as regular issues)
