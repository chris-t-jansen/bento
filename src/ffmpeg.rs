//! ffmpeg/ffprobe detection and version checking for `bento check`.

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Oldest ffmpeg version Bento is known to work with.
pub const MINIMUM: Version = Version { major: 4, minor: 4, patch: 0 };

/// The ffmpeg version Bento is regularly tested against. Detections with a
/// higher major version trigger an informational warning.
pub const TESTED: Version = Version { major: 6, minor: 1, patch: 0 };

// ---------------------------------------------------------------------------
// Version type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl Version {
    /// Parse from a token like `"6.1.1"`, `"4.4.2-0ubuntu0.22.04.1"`, or `"6.1"`.
    pub fn parse(s: &str) -> Option<Self> {
        // Take only the leading digits-and-dots portion so package suffixes
        // (e.g. "-0ubuntu0.22.04.1") are ignored.
        let numeric: String = s
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let mut parts = numeric.splitn(3, '.');
        let major: u32 = parts.next()?.parse().ok()?;
        let minor: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        Some(Version { major, minor, patch })
    }

    /// Classify this version against the pinned constants.
    pub fn band(self) -> VersionBand {
        if self < MINIMUM {
            VersionBand::BelowMinimum
        } else if self.major > TESTED.major {
            VersionBand::AboveTestedMajor
        } else {
            VersionBand::Ok
        }
    }
}

/// The result of comparing a detected version against the pinned constants.
#[derive(Debug, PartialEq, Eq)]
pub enum VersionBand {
    /// Version is below [`MINIMUM`]: warn loudly.
    BelowMinimum,
    /// Version is in the supported range (between minimum and tested, inclusive
    /// of the tested major). Silent.
    Ok,
    /// Version's major is above the tested major: warn, may have breaking changes.
    AboveTestedMajor,
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Detection result for a single binary.
#[derive(Debug)]
pub struct BinDetection {
    pub name: &'static str,
    pub path: Option<PathBuf>,
    pub version: Option<Version>,
}

/// Try to locate `name` on `PATH` and parse its version string.
///
/// Returns `None` if the binary is not found at all. If it is found but the
/// version line cannot be parsed, returns `Some` with `version: None`.
pub fn detect(name: &'static str) -> Option<BinDetection> {
    let version = query_version(name)?; // returns None only when binary is absent
    let path = find_path(name);
    Some(BinDetection { name, path, version })
}

/// Run `<name> -version` and parse the version token from the first output line.
///
/// Returns `None` if the binary does not exist (`NotFound` IO error). Any
/// other failure (non-zero exit, parse error) returns `Some(None)` — the
/// binary ran but we couldn't read the version.
fn query_version(name: &str) -> Option<Option<Version>> {
    let result = Command::new(name)
        .arg("-version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    match result {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => return Some(None),
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // First line: "ffmpeg version 6.1.1 Copyright ..."
            let version = stdout
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(2))
                .and_then(Version::parse);
            Some(version)
        }
    }
}

/// Find the resolved filesystem path of `name` by searching `PATH`.
fn find_path(name: &str) -> Option<PathBuf> {
    which::which(name).ok()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parse_plain() {
        assert_eq!(
            Version::parse("6.1.1"),
            Some(Version { major: 6, minor: 1, patch: 1 })
        );
    }

    #[test]
    fn version_parse_ubuntu_suffix() {
        // e.g. "4.4.2-0ubuntu0.22.04.1"
        assert_eq!(
            Version::parse("4.4.2-0ubuntu0.22.04.1"),
            Some(Version { major: 4, minor: 4, patch: 2 })
        );
    }

    #[test]
    fn version_parse_no_patch() {
        assert_eq!(
            Version::parse("6.1"),
            Some(Version { major: 6, minor: 1, patch: 0 })
        );
    }

    #[test]
    fn version_parse_dev_build_n_prefix() {
        // "N-xxxxx-gxxxxxxxx" — major = 0 because "N" doesn't parse as a digit
        assert_eq!(Version::parse("N-111705-g4f6b3ad"), None);
    }

    #[test]
    fn version_band_below_minimum() {
        let v = Version { major: 3, minor: 4, patch: 0 };
        assert_eq!(v.band(), VersionBand::BelowMinimum);
    }

    #[test]
    fn version_band_at_minimum() {
        assert_eq!(MINIMUM.band(), VersionBand::Ok);
    }

    #[test]
    fn version_band_same_major_as_tested() {
        let v = Version { major: TESTED.major, minor: 99, patch: 0 };
        assert_eq!(v.band(), VersionBand::Ok);
    }

    #[test]
    fn version_band_above_tested_major() {
        let v = Version { major: TESTED.major + 1, minor: 0, patch: 0 };
        assert_eq!(v.band(), VersionBand::AboveTestedMajor);
    }

    #[test]
    fn version_display() {
        let v = Version { major: 6, minor: 1, patch: 1 };
        assert_eq!(v.to_string(), "6.1.1");
    }

    #[test]
    fn version_ordering() {
        assert!(
            Version { major: 5, minor: 0, patch: 0 } > Version { major: 4, minor: 4, patch: 0 }
        );
        assert!(
            Version { major: 4, minor: 5, patch: 0 } > Version { major: 4, minor: 4, patch: 0 }
        );
        assert!(
            Version { major: 4, minor: 4, patch: 1 } > Version { major: 4, minor: 4, patch: 0 }
        );
    }
}
