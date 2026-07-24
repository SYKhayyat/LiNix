//! The closed format vocabulary, and how a filename is mapped into it.

use std::fmt;

/// A downloadable artifact shape. The vocabulary is closed: an unrecognised name is an error
/// that names the legal set, never a passthrough string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Format {
    Deb,
    Rpm,
    AppImage,
    Tarball,
    Zip,
    Exe,
    Msi,
    Pkg,
    Dmg,
    /// An unarchived executable: matched this machine and carries no recognised extension.
    Binary,
}

impl Format {
    pub const ALL: [Format; 10] = [
        Format::Deb,
        Format::Rpm,
        Format::AppImage,
        Format::Tarball,
        Format::Zip,
        Format::Exe,
        Format::Msi,
        Format::Pkg,
        Format::Dmg,
        Format::Binary,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Format::Deb => "deb",
            Format::Rpm => "rpm",
            Format::AppImage => "appimage",
            Format::Tarball => "tarball",
            Format::Zip => "zip",
            Format::Exe => "exe",
            Format::Msi => "msi",
            Format::Pkg => "pkg",
            Format::Dmg => "dmg",
            Format::Binary => "binary",
        }
    }

    /// The one place the vocabulary is read from, so the parser and the error message cannot
    /// disagree about what is legal.
    pub fn parse(name: &str) -> Result<Format, UnknownFormat> {
        Format::ALL
            .into_iter()
            .find(|f| f.as_str() == name)
            .ok_or_else(|| UnknownFormat {
                given: name.to_string(),
            })
    }

    pub fn vocabulary() -> String {
        Format::ALL
            .iter()
            .map(|f| f.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Whether this format installs itself into a system package database rather than being
    /// unpacked by LiNix. These are the artifacts a second package manager can then upgrade.
    pub fn is_system_package(self) -> bool {
        matches!(
            self,
            Format::Deb | Format::Rpm | Format::Pkg | Format::Msi | Format::Exe
        )
    }

    /// Whether LiNix must unpack this before anything is executable.
    pub fn is_archive(self) -> bool {
        matches!(self, Format::Tarball | Format::Zip)
    }

    /// Longest suffix wins, so `.tar.gz` is a tarball rather than an unknown `.gz`.
    fn suffixes(self) -> &'static [&'static str] {
        match self {
            Format::Deb => &[".deb"],
            Format::Rpm => &[".rpm"],
            Format::AppImage => &[".appimage"],
            Format::Tarball => &[".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst", ".tgz", ".txz"],
            Format::Zip => &[".zip"],
            Format::Exe => &[".exe"],
            Format::Msi => &[".msi"],
            Format::Pkg => &[".pkg"],
            Format::Dmg => &[".dmg"],
            Format::Binary => &[],
        }
    }

    /// `None` means "no recognised extension", which is what makes a filename a `Binary`
    /// candidate — but only after the platform filter has agreed the file runs here, so this
    /// function deliberately cannot answer `Some(Binary)`.
    pub fn of_filename(name: &str) -> Option<Format> {
        let lower = name.to_lowercase();
        Format::ALL
            .iter()
            .flat_map(|f| f.suffixes().iter().map(move |s| (*f, *s)))
            .filter(|(_, suffix)| lower.ends_with(suffix))
            .max_by_key(|(_, suffix)| suffix.len())
            .map(|(f, _)| f)
    }

    /// Detached signatures and checksum files are assets of a release but never a thing to
    /// install; they would otherwise win the `Binary` fallback on name length alone.
    pub fn is_metadata_filename(name: &str) -> bool {
        const METADATA_SUFFIXES: [&str; 8] = [
            ".sha256", ".sha512", ".sha1", ".md5", ".asc", ".sig", ".pem", ".sbom",
        ];
        let lower = name.to_lowercase();
        if METADATA_SUFFIXES.iter().any(|s| lower.ends_with(s)) {
            return true;
        }
        // A digest list, by any of the spellings releases actually use. rclone ships `MD5SUMS`,
        // `SHA1SUMS` and `SHA256SUMS` in one release, and none of them carries an extension —
        // which used to make each a candidate executable (D2). Matching the family rather than
        // three literals is what stops the fourth spelling arriving unnoticed.
        let stem = lower.strip_suffix(".txt").unwrap_or(&lower);
        stem.ends_with("sums") || stem.ends_with("sum")
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownFormat {
    pub given: String,
}

impl fmt::Display for UnknownFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown format '{}'. Legal formats: {}",
            self.given,
            Format::vocabulary()
        )
    }
}

impl std::error::Error for UnknownFormat {}

/// An ordered preference. First match wins; a later entry is a fallback, never an addition.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FormatOrder(Vec<Format>);

impl FormatOrder {
    pub fn new(formats: Vec<Format>) -> Self {
        let mut seen = Vec::with_capacity(formats.len());
        for f in formats {
            if !seen.contains(&f) {
                seen.push(f);
            }
        }
        FormatOrder(seen)
    }

    pub fn parse_all<I, S>(names: I) -> Result<Self, UnknownFormat>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let parsed = names
            .into_iter()
            .map(|n| Format::parse(n.as_ref().trim()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(FormatOrder::new(parsed))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_slice(&self) -> &[Format] {
        &self.0
    }

    /// Derived from detected facts rather than configured, so a fresh repo installs the right
    /// artifact with no `formats` line anywhere.
    pub fn detected_default(os: &str, family: Option<&str>) -> Self {
        let tail = [Format::AppImage, Format::Tarball, Format::Binary];
        let head: &[Format] = match (os, family) {
            ("windows", _) => return FormatOrder::new(vec![Format::Exe, Format::Msi, Format::Zip]),
            ("macos", _) => return FormatOrder::new(vec![Format::Dmg, Format::Pkg, Format::Tarball, Format::Binary]),
            (_, Some("debian")) => &[Format::Deb],
            (_, Some("fedora")) | (_, Some("rhel")) | (_, Some("suse")) => &[Format::Rpm],
            _ => &[],
        };
        FormatOrder::new(head.iter().copied().chain(tail).collect())
    }

    /// Rank of `format` in this order; `None` means the user did not ask for it, which is a
    /// rejection rather than a low score.
    pub fn rank(&self, format: Format) -> Option<usize> {
        self.0.iter().position(|f| *f == format)
    }

    /// Narrow the order to what a backend can actually install, keeping the order intact.
    /// A default order that names a format the backend cannot handle should fall through to
    /// the next entry, which is what "a later entry is a fallback" already means.
    pub fn retaining(&self, keep: impl Fn(Format) -> bool) -> Self {
        FormatOrder(self.0.iter().copied().filter(|f| keep(*f)).collect())
    }

    /// What was dropped by `retaining`, so an empty result can say why.
    pub fn rejected_by(&self, keep: impl Fn(Format) -> bool) -> Vec<Format> {
        self.0.iter().copied().filter(|f| !keep(*f)).collect()
    }
}

impl fmt::Display for FormatOrder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<_> = self.0.iter().map(|f| f.as_str()).collect();
        f.write_str(&names.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tar_gz_is_a_tarball_not_an_unknown_gz() {
        assert_eq!(
            Format::of_filename("fd-v10.2.0-x86_64-unknown-linux-gnu.tar.gz"),
            Some(Format::Tarball)
        );
        assert_eq!(Format::of_filename("x.tgz"), Some(Format::Tarball));
        assert_eq!(Format::of_filename("x.tar.zst"), Some(Format::Tarball));
    }

    #[test]
    fn an_extensionless_asset_has_no_format() {
        assert_eq!(Format::of_filename("fd-linux"), None);
        assert_eq!(Format::of_filename("fd"), None);
    }

    #[test]
    fn detection_is_case_insensitive() {
        assert_eq!(Format::of_filename("Tool.AppImage"), Some(Format::AppImage));
        assert_eq!(Format::of_filename("TOOL.DEB"), Some(Format::Deb));
    }

    #[test]
    fn an_unknown_format_names_the_legal_set() {
        let err = Format::parse("snap").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("snap"));
        for f in Format::ALL {
            assert!(msg.contains(f.as_str()), "vocabulary missing {}", f);
        }
    }

    #[test]
    fn signatures_and_checksums_are_not_installable_assets() {
        assert!(Format::is_metadata_filename("fd_10.2.0_amd64.deb.sha256"));
        assert!(Format::is_metadata_filename("checksums.txt"));
        assert!(Format::is_metadata_filename("release.asc"));
        assert!(!Format::is_metadata_filename("fd_10.2.0_amd64.deb"));
        // The whole digest-list family, in the spellings real releases use — rclone ships the
        // first three in one release, and none has an extension to be recognised by (D2).
        for name in [
            "MD5SUMS",
            "SHA1SUMS",
            "SHA256SUMS",
            "sha512sums.txt",
            "sha256sum.txt",
        ] {
            assert!(Format::is_metadata_filename(name), "{}", name);
        }
        // And a package that merely ends in those letters is not one.
        assert!(!Format::is_metadata_filename("consums-1.0-linux-amd64.tar.gz"));
    }

    #[test]
    fn the_debian_default_prefers_deb_then_falls_back() {
        let order = FormatOrder::detected_default("linux", Some("debian"));
        assert_eq!(
            order.as_slice(),
            &[
                Format::Deb,
                Format::AppImage,
                Format::Tarball,
                Format::Binary
            ]
        );
    }

    #[test]
    fn an_unknown_family_still_has_a_working_default() {
        let order = FormatOrder::detected_default("linux", Some("alpine"));
        assert_eq!(
            order.as_slice(),
            &[Format::AppImage, Format::Tarball, Format::Binary]
        );
    }

    #[test]
    fn windows_never_falls_back_to_a_linux_artifact() {
        let order = FormatOrder::detected_default("windows", None);
        assert!(!order.as_slice().contains(&Format::AppImage));
        assert!(!order.as_slice().contains(&Format::Deb));
    }

    #[test]
    fn a_repeated_format_keeps_its_first_position() {
        let order = FormatOrder::new(vec![Format::Deb, Format::Tarball, Format::Deb]);
        assert_eq!(order.as_slice(), &[Format::Deb, Format::Tarball]);
    }

    #[test]
    fn rank_rejects_a_format_nobody_asked_for() {
        let order = FormatOrder::new(vec![Format::Deb, Format::Tarball]);
        assert_eq!(order.rank(Format::Deb), Some(0));
        assert_eq!(order.rank(Format::Tarball), Some(1));
        assert_eq!(order.rank(Format::Rpm), None);
    }
}
