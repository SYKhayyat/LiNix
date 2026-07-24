//! Whether an asset filename can run on this machine.
//!
//! This filter runs before `formats` is consulted, so a preference only ever orders artifacts
//! that already execute here. There is no user-facing arch or os option: a declaration that
//! could request an artifact this machine cannot run has no use case.

/// A filename names an OS or an architecture, or it names neither. Absent evidence is not
/// evidence of mismatch — a release that ships one portable `tool.tar.gz` names nothing, and
/// rejecting it would leave the user with no asset at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Match {
    /// The filename names this machine.
    Ours,
    /// The filename names a different machine.
    Foreign,
    /// The filename is silent on the question.
    Silent,
}

impl Match {
    /// Within one axis, naming us wins over also naming someone else: a macOS universal asset
    /// names both arches and runs here regardless.
    fn or_stronger(self, other: Match) -> Match {
        match (self, other) {
            (Match::Ours, _) | (_, Match::Ours) => Match::Ours,
            (Match::Foreign, _) | (_, Match::Foreign) => Match::Foreign,
            _ => Match::Silent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub os: String,
    pub arch: String,
}

/// Each row is one canonical target and every spelling releases use for it. A row is matched
/// as a whole: any alias hitting means the filename names *that* target.
const OS_ALIASES: &[(&str, &[&str])] = &[
    ("linux", &["linux"]),
    ("macos", &["macos", "darwin", "apple", "osx", "mac"]),
    ("windows", &["windows", "win", "win32", "win64"]),
    ("freebsd", &["freebsd"]),
    ("netbsd", &["netbsd"]),
    ("openbsd", &["openbsd"]),
    ("android", &["android"]),
    ("ios", &["ios"]),
    ("solaris", &["solaris", "illumos"]),
];

const ARCH_ALIASES: &[(&str, &[&str])] = &[
    ("x86_64", &["x86_64", "x86-64", "amd64", "x64"]),
    ("aarch64", &["aarch64", "arm64", "armv8"]),
    ("x86", &["i686", "i386", "x86", "386", "ia32"]),
    ("arm", &["armv7", "armv7l", "armv6", "armhf", "armel", "arm"]),
    ("riscv64", &["riscv64", "riscv"]),
    ("powerpc64", &["ppc64le", "ppc64", "powerpc64"]),
    ("s390x", &["s390x"]),
    ("loongarch64", &["loongarch64", "loong64"]),
];

impl Platform {
    pub fn current() -> Self {
        Platform {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        }
    }

    pub fn new(os: impl Into<String>, arch: impl Into<String>) -> Self {
        Platform {
            os: os.into(),
            arch: arch.into(),
        }
    }

    /// A `Foreign` on *either* axis rejects — unlike within an axis, naming the right OS does
    /// not excuse naming the wrong architecture. `Silent` on both accepts.
    pub fn accepts(&self, filename: &str) -> bool {
        let lower = filename.to_lowercase();
        classify(&lower, OS_ALIASES, &self.os) != Match::Foreign
            && classify(&lower, ARCH_ALIASES, &self.arch) != Match::Foreign
    }

    /// Whether the filename actually names this machine, as opposed to merely not contradicting
    /// it. Two otherwise equal candidates are not equally good when one says `linux-x86_64`.
    pub fn specificity(&self, filename: &str) -> u8 {
        let lower = filename.to_lowercase();
        let os = classify(&lower, OS_ALIASES, &self.os) == Match::Ours;
        let arch = classify(&lower, ARCH_ALIASES, &self.arch) == Match::Ours;
        u8::from(os) + u8::from(arch)
    }
}

fn classify(lower: &str, table: &[(&str, &[&str])], ours: &str) -> Match {
    let mut hits: Vec<(usize, usize, &str)> = Vec::new();
    for (canonical, aliases) in table {
        for alias in *aliases {
            for start in token_positions(lower, alias) {
                hits.push((start, alias.len(), canonical));
            }
        }
    }

    // `x86` sits inside `x86_64` and `_` reads as a token boundary, so a shorter alias starting
    // where a longer one does is not a second target — it is the same text, misread.
    let mut result = Match::Silent;
    for (start, len, canonical) in &hits {
        let dominated = hits
            .iter()
            .any(|(other_start, other_len, _)| other_start == start && other_len > len);
        if dominated {
            continue;
        }
        let hit = if canonical_matches(canonical, ours) {
            Match::Ours
        } else {
            Match::Foreign
        };
        result = result.or_stronger(hit);
    }
    result
}

/// `std::env::consts::ARCH` reports `powerpc64` for both endiannesses, so the table's one row
/// covers both spellings.
fn canonical_matches(canonical: &str, ours: &str) -> bool {
    canonical == ours || (canonical == "powerpc64" && ours.starts_with("powerpc64"))
}

/// Every offset where `needle` appears bounded by non-alphanumeric characters, so `arm` does
/// not match inside `armv7`. The alias may itself contain `_` or `-`.
///
/// **A run of digits closing the word is part of the boundary** (D2, checked against real
/// releases): `linux64`, `win64`, `mac64` and `darwin64` are all shipped spellings, and
/// requiring a non-alphanumeric after `linux` made `jq-linux64` — which is in jq's actual
/// release — read as naming no OS at all. On Windows that made it an executable candidate.
/// The digits must *end* the word: `linux6x` is not a claim about Linux, and the leading
/// boundary is unchanged, so `386` still does not match inside `i386`.
fn token_positions(haystack: &str, needle: &str) -> Vec<usize> {
    let bytes = haystack.as_bytes();
    let mut found = Vec::new();
    let mut from = 0;
    while from < haystack.len() {
        let Some(offset) = haystack[from..].find(needle) else {
            break;
        };
        let start = from + offset;
        let end = start + needle.len();
        let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        if before_ok && ends_word(bytes, end, needle) {
            found.push(start);
        }
        from = start + 1;
    }
    found
}

/// Whether the alias ending at `end` closes a word — directly, or across trailing digits when
/// the alias itself ends in a letter.
fn ends_word(bytes: &[u8], end: usize, needle: &str) -> bool {
    if end == bytes.len() || !bytes[end].is_ascii_alphanumeric() {
        return true;
    }
    if !needle.ends_with(|c: char| c.is_ascii_alphabetic()) {
        return false;
    }
    let mut i = end;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    i > end && (i == bytes.len() || !bytes[i].is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linux64() -> Platform {
        Platform::new("linux", "x86_64")
    }

    #[test]
    fn a_matching_triple_is_accepted() {
        assert!(linux64().accepts("fd-v10.2.0-x86_64-unknown-linux-gnu.tar.gz"));
        assert!(linux64().accepts("fd_10.2.0_amd64.deb"));
    }

    #[test]
    fn a_foreign_os_is_rejected() {
        assert!(!linux64().accepts("fd-v10.2.0-x86_64-pc-windows-msvc.zip"));
        assert!(!linux64().accepts("fd-v10.2.0-x86_64-apple-darwin.tar.gz"));
    }

    #[test]
    fn a_foreign_arch_is_rejected() {
        assert!(!linux64().accepts("fd_10.2.0_arm64.deb"));
        assert!(!linux64().accepts("fd-v10.2.0-i686-unknown-linux-gnu.tar.gz"));
    }

    #[test]
    fn silence_on_both_axes_is_accepted() {
        assert!(linux64().accepts("fd.tar.gz"));
        assert!(linux64().accepts("fd"));
    }

    #[test]
    fn arm_does_not_match_inside_armv7() {
        let arm64 = Platform::new("linux", "aarch64");
        assert!(!arm64.accepts("tool-linux-armv7.tar.gz"));
        assert!(arm64.accepts("tool-linux-arm64.tar.gz"));
    }

    #[test]
    fn x86_does_not_match_inside_x86_64() {
        let x86 = Platform::new("linux", "x86");
        assert!(!x86.accepts("tool-linux-x86_64.tar.gz"));
        assert!(x86.accepts("tool-linux-i686.tar.gz"));
    }

    #[test]
    fn x86_64_is_not_confused_by_the_32_bit_row() {
        assert!(linux64().accepts("tool-linux-x86_64.tar.gz"));
        assert!(!linux64().accepts("tool-linux-x86.tar.gz"));
    }

    #[test]
    fn win_does_not_match_inside_a_longer_word() {
        assert!(linux64().accepts("winnowing-tool-linux.tar.gz"));
    }

    #[test]
    fn specificity_ranks_an_explicit_target_above_a_silent_one() {
        assert_eq!(linux64().specificity("fd-linux-x86_64.tar.gz"), 2);
        assert_eq!(linux64().specificity("fd-linux.tar.gz"), 1);
        assert_eq!(linux64().specificity("fd.tar.gz"), 0);
    }

    #[test]
    fn a_universal_macos_asset_naming_both_arches_is_not_foreign() {
        let mac = Platform::new("macos", "aarch64");
        assert!(mac.accepts("tool-macos-universal.dmg"));
        assert!(mac.accepts("tool-macos-x86_64-arm64.dmg"));
    }

    #[test]
    fn the_right_os_does_not_excuse_the_wrong_arch() {
        assert!(!linux64().accepts("tool-linux-arm64.tar.gz"));
    }
}
