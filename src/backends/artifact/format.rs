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
///
/// `user_specified` records whether this order came from a `@formats=` the user wrote or was
/// detected from the machine. It changes the tie-break, and only the tie-break (D2): a written
/// order is an instruction and wins outright, while a detected one yields to an asset that
/// names the exact machine — because a detected default is LiNix's guess about file *type*, and
/// a file naming your os and arch is a better-evidenced guess than one naming neither.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FormatOrder {
    order: Vec<Format>,
    user_specified: bool,
}

impl FormatOrder {
    pub fn new(formats: Vec<Format>) -> Self {
        let mut seen = Vec::with_capacity(formats.len());
        for f in formats {
            if !seen.contains(&f) {
                seen.push(f);
            }
        }
        FormatOrder {
            order: seen,
            user_specified: false,
        }
    }

    /// The same order, marked as something the user asked for by name. Called where a
    /// `@formats=` value is read.
    pub fn as_user_specified(mut self) -> Self {
        self.user_specified = true;
        self
    }

    /// Whether this order was written by the user rather than detected. The tie-break reads
    /// this and nothing else reads it.
    pub fn is_user_specified(&self) -> bool {
        self.user_specified
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
        self.order.is_empty()
    }

    pub fn as_slice(&self) -> &[Format] {
        &self.order
    }

    /// The version of the built-in default order (D11).
    ///
    /// The default is *detected*, not configured, so a LiNix upgrade that changes it would
    /// silently install a different artifact on a machine with no `@formats=` line — a `tarball`
    /// today, a `deb` after the upgrade. This constant is the promise that such a move is
    /// visible: **bump it whenever `detected_default` changes what it returns for any host**, and
    /// the changelog says so. A lock protects an existing install; this protects the person
    /// reading the changelog before a fresh one.
    pub const DEFAULT_ORDER_VERSION: u32 = 1;

    /// Derived from detected facts rather than configured, so a fresh repo installs the right
    /// artifact with no `formats` line anywhere.
    ///
    /// Its output is versioned by [`DEFAULT_ORDER_VERSION`] — changing what this returns is a
    /// visible event, not a silent one.
    pub fn detected_default(os: &str, family: Option<&str>) -> Self {
        let tail = [Format::AppImage, Format::Tarball, Format::Binary];
        let head: &[Format] = match (os, family) {
            ("windows", _) => return FormatOrder::new(vec![Format::Exe, Format::Msi, Format::Zip]),
            // `zip` is in the macOS order because gh, rclone and starship — checked against
            // their real releases — ship their macOS build as one, and without it those
            // packages resolved to nothing on a Mac (D2).
            ("macos", _) => {
                return FormatOrder::new(vec![
                    Format::Dmg,
                    Format::Pkg,
                    Format::Tarball,
                    Format::Zip,
                    Format::Binary,
                ])
            }
            (_, Some("debian")) => &[Format::Deb],
            (_, Some("fedora")) | (_, Some("rhel")) | (_, Some("suse")) => &[Format::Rpm],
            _ => &[],
        };
        FormatOrder::new(head.iter().copied().chain(tail).collect())
    }

    /// Rank of `format` in this order; `None` means the user did not ask for it, which is a
    /// rejection rather than a low score.
    pub fn rank(&self, format: Format) -> Option<usize> {
        self.order.iter().position(|f| *f == format)
    }

    /// Narrow the order to what a backend can actually install, keeping the order intact.
    /// A default order that names a format the backend cannot handle should fall through to
    /// the next entry, which is what "a later entry is a fallback" already means.
    pub fn retaining(&self, keep: impl Fn(Format) -> bool) -> Self {
        FormatOrder {
            order: self.order.iter().copied().filter(|f| keep(*f)).collect(),
            user_specified: self.user_specified,
        }
    }

    /// What was dropped by `retaining`, so an empty result can say why.
    pub fn rejected_by(&self, keep: impl Fn(Format) -> bool) -> Vec<Format> {
        self.order.iter().copied().filter(|f| !keep(*f)).collect()
    }
}

impl fmt::Display for FormatOrder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<_> = self.order.iter().map(|f| f.as_str()).collect();
        f.write_str(&names.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D11: the default order is versioned, and this test is the tripwire. It pins the exact
    /// order for the three host shapes to `DEFAULT_ORDER_VERSION`. Changing what
    /// `detected_default` returns fails this test until the constant is bumped — which is the
    /// point: a change to the default is a deliberate, visible event, not a silent one an
    /// upgrade slips in.
    #[test]
    fn the_default_order_is_pinned_to_its_version() {
        assert_eq!(FormatOrder::DEFAULT_ORDER_VERSION, 1, "bump the version WITH the order");

        let debian = FormatOrder::detected_default("linux", Some("debian"));
        assert_eq!(
            debian.as_slice(),
            &[Format::Deb, Format::AppImage, Format::Tarball, Format::Binary]
        );
        let windows = FormatOrder::detected_default("windows", None);
        assert_eq!(windows.as_slice(), &[Format::Exe, Format::Msi, Format::Zip]);
        let macos = FormatOrder::detected_default("macos", None);
        assert_eq!(
            macos.as_slice(),
            &[Format::Dmg, Format::Pkg, Format::Tarball, Format::Zip, Format::Binary]
        );
    }

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
    fn the_macos_default_includes_zip() {
        // gh, rclone and starship all ship their macOS build as a .zip; without it those
        // packages resolve to nothing on a Mac (D2).
        let order = FormatOrder::detected_default("macos", None);
        assert!(order.as_slice().contains(&Format::Zip), "{:?}", order.as_slice());
        // dmg and pkg still lead — a native installer beats an archive to unpack.
        assert_eq!(order.as_slice().first(), Some(&Format::Dmg));
    }

    #[test]
    fn a_detected_order_is_not_user_specified_but_a_marked_one_is() {
        assert!(!FormatOrder::detected_default("linux", Some("debian")).is_user_specified());
        assert!(FormatOrder::new(vec![Format::Deb]).as_user_specified().is_user_specified());
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
