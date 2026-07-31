use crate::core::{Error, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::{Path, PathBuf};
use tracing::warn;

/// The allowlist must stay wide enough for names that are legitimately not bare words:
/// npm `@scope`, github `owner/repo`, versioned `pkg:1.2+build`.
static PACKAGE_NAME_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-zA-Z0-9._@:+/-]+$").unwrap());

/// The same allowlist plus the characters a Windows package identifier is built from.
///
/// `winget list` reports 185 of 278 names on a stock box as `ARP\Machine\X64\...` or
/// `MSIX\...`, and the ARP rows for MSI installers are GUIDs in braces. Those are the
/// identifiers `winget install` and `winget uninstall` take, so they are the names LiNix has to
/// be able to carry (V.113).
static WINDOWS_IDENTIFIER_NAME_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-zA-Z0-9._@:+/\\{}-]+$").unwrap());

/// Shell metacharacters, minus the three a Windows package identifier is made of.
///
/// Safe because **no package-manager command is ever a shell string** — every one is argv, and
/// that is the property the executor's own tests exist to keep. This list is defence in depth
/// against a name reaching a shell that does not exist; it is not what stands between a crafted
/// name and a command line.
static SHELL_INJECTION_REGEX_WINDOWS_ID: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[;&|><`$\(\)\[\]\*\?\!]").unwrap());

/// Shell metacharacters blocked to prevent command injection.
static SHELL_INJECTION_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[;&|><`$\(\)\[\]\{\}\*\?\!\\]").unwrap());

/// Sensitive system paths that LiNix is prohibited from accessing.
static FORBIDDEN_PATHS: &[&str] = &[
    "/etc/shadow",
    "/etc/sudoers",
    "/etc/passwd",
    "/etc/gshadow",
    "C:\\Windows\\System32\\config\\SAM",
    "C:\\Windows\\System32\\config\\SECURITY",
];

/// Render untrusted text for a message a terminal will draw.
///
/// A refusal about invisible characters that reprints them is worse than no refusal: U+202E
/// reverses everything after it as it renders, so the message can be made to read as its own
/// opposite, and an ANSI escape recolours or erases the lines around it. Manifests arrive from
/// shared configs, not only from the user's own hand.
///
/// Everything outside printable ASCII-and-ordinary-Unicode is named by codepoint instead of
/// emitted. Ordinary non-ASCII stays as itself — a package name in Cyrillic should read as one
/// — so the rule is drawn at *what the character does to the display*, not at what alphabet it
/// belongs to: C0/C1 controls, the bidi overrides and embeddings, the invisible formatting
/// characters, and the line/paragraph separators.
pub fn printable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let dangerous = c.is_control()
            // Bidi overrides, embeddings and isolates — the trojan-source family.
            || ('\u{202A}'..='\u{202E}').contains(&c)
            || ('\u{2066}'..='\u{2069}').contains(&c)
            // Zero-width and other invisible formatting.
            || matches!(c, '\u{200B}'..='\u{200F}' | '\u{FEFF}' | '\u{00AD}')
            // Line and paragraph separators, which break a message across lines.
            || matches!(c, '\u{2028}' | '\u{2029}');
        if dangerous {
            out.push_str(&format!("<U+{:04X}>", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

pub struct Validator;

impl Validator {
    /// Backends whose "name" is legitimately a filesystem path (`link`) or a URL / owner-repo
    /// (`web`, `github`, `appimage`). For these the "looks like an absolute path" guard — a
    /// leading `/` or `\` — would wrongly reject valid input (e.g. `link:/home/me/.vimrc`).
    /// They still get every other check: `..` traversal, the character allowlist, and
    /// shell-injection blocking. Only the leading-separator rule is lifted for them.
    fn is_path_oriented_backend(backend: &str) -> bool {
        // `btrfs` was missing until 2026-07-30, and it is the member whose name is *most*
        // literally a path: `btrfs:/mnt/data/vol` installs by running
        // `btrfs subvolume create /mnt/data/vol`. No declaration of it could be written, and
        // nothing noticed because no harness had a btrfs filesystem to install into.
        //
        // Not `lvm` or `zfs`: `lvm:vg0/data` and `zfs:tank/data` carry a separator and never a
        // leading one, so the strict rule is correct for them and widening it would buy nothing.
        matches!(backend, "link" | "web" | "github" | "appimage" | "btrfs")
    }

    /// Backends whose manager's own identifiers carry a path separator or braces.
    ///
    /// `winget` is the whole list, and a list rather than a rule because those characters are
    /// worth refusing everywhere else: no second manager on any platform names things this way.
    /// `..` stays forbidden for it, exactly as for everything else.
    fn names_carry_windows_identifiers(backend: &str) -> bool {
        backend == "winget"
    }

    /// Validates package names against injection and traversal, with no knowledge of the
    /// backend — the strict rule (a leading path separator is rejected). Prefer
    /// [`Validator::validate_package_name_for`] when the backend is known.
    pub fn validate_package_name(name: &str) -> Result<()> {
        Self::validate_package_name_for(name, "")
    }

    /// Validates a package name for a specific backend. Identical to
    /// [`Validator::validate_package_name`] except that the "absolute path" guard (a leading
    /// `/` or `\`) is lifted for the path/URL-oriented backends (see
    /// [`Validator::is_path_oriented_backend`]), whose names are legitimately paths/URLs.
    /// Directory traversal (`..`), the character allowlist, and shell-injection blocking
    /// always apply, for every backend.
    pub fn validate_package_name_for(name: &str, backend: &str) -> Result<()> {
        if name.is_empty() {
            return Err(Error::Validation("Empty package name".into()));
        }
        if name.len() > 256 {
            return Err(Error::Validation("Name too long".into()));
        }

        // Directory traversal is ALWAYS forbidden, regardless of backend.
        if name.contains("..") {
            return Err(Error::Validation(format!(
                "Path traversal detected in name: {}",
                printable(name)
            )));
        }
        // A leading path separator normally signals an absolute-path injection attempt — but
        // for a path/URL-oriented backend (e.g. `link`, whose name IS a path) it is valid.
        if !Self::is_path_oriented_backend(backend)
            && (name.starts_with('/') || name.starts_with('\\'))
        {
            return Err(Error::Validation(format!(
                "Path traversal detected in name: {}",
                printable(name)
            )));
        }

        // A manager that prints a name must be able to be handed it back (V.113). `winget`'s
        // identifiers carry backslashes and braces; the grammar was taught to accept them and
        // this check was not, so `adopt` wrote rows that then failed to parse and wedged the
        // model — measured on the native sweep at `adopted.txt:78`.
        let (allowed, injection) = if Self::names_carry_windows_identifiers(backend) {
            (
                &*WINDOWS_IDENTIFIER_NAME_REGEX,
                &*SHELL_INJECTION_REGEX_WINDOWS_ID,
            )
        } else {
            (&*PACKAGE_NAME_REGEX, &*SHELL_INJECTION_REGEX)
        };

        if !allowed.is_match(name) {
            return Err(Error::Validation(format!(
                "Invalid characters in package name: {}",
                printable(name)
            )));
        }

        if injection.is_match(name) {
            return Err(Error::Validation(
                "Shell injection characters detected".into(),
            ));
        }

        Ok(())
    }

    /// Forbidden zones are matched as path prefixes on the *resolved* path, never as
    /// substrings: a substring test both misses `/etc/../etc/shadow` and rejects innocent
    /// names that merely contain a forbidden one. A path that does not exist is returned
    /// unresolved and unchecked — callers must not treat this as proof it is allowed.
    pub async fn validate_path(path: &Path) -> Result<PathBuf> {
        if !tokio::fs::try_exists(path).await.unwrap_or(false) {
            return Ok(path.to_path_buf());
        }

        let path_owned = path.to_path_buf();
        let canonical = tokio::task::spawn_blocking(move || path_owned.canonicalize())
            .await
            .map_err(|e| Error::Other(e.to_string()))?
            .map_err(|e| Error::Validation(format!("Path resolution failed: {}", e)))?;

        Self::refuse_forbidden(&canonical)?;
        Ok(canonical)
    }

    /// [`Validator::validate_path`] for a caller that cannot await — the `vars` standard
    /// library's `read_file`, which runs inside Rhai.
    pub fn validate_path_sync(path: &Path) -> Result<PathBuf> {
        if !path.exists() {
            return Ok(path.to_path_buf());
        }
        let canonical = path
            .canonicalize()
            .map_err(|e| Error::Validation(format!("Path resolution failed: {}", e)))?;
        Self::refuse_forbidden(&canonical)?;
        Ok(canonical)
    }

    fn refuse_forbidden(canonical: &Path) -> Result<()> {
        for forbidden in FORBIDDEN_PATHS {
            if canonical.starts_with(Path::new(forbidden)) {
                warn!(
                    "Security Block: Attempted access to forbidden path: {:?}",
                    canonical
                );
                return Err(Error::Permission(format!("Access Denied: {}", forbidden)));
            }
        }
        Ok(())
    }

    pub fn validate_backend_name(name: &str) -> Result<()> {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Err(Error::Validation(format!(
                "Invalid backend identifier: {}",
                printable(name)
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_validation_blocks_absolute_paths_for_normal_backends() {
        // A leading slash on an ordinary package name is an injection attempt.
        assert!(Validator::validate_package_name("/etc/passwd").is_err());
        assert!(Validator::validate_package_name_for("/etc/passwd", "apt").is_err());
        assert!(Validator::validate_package_name_for("ripgrep", "apt").is_ok());
        // github owner/repo and web URLs (no leading slash) pass either way.
        assert!(Validator::validate_package_name_for("BurntSushi/ripgrep", "github").is_ok());
    }

    #[test]
    fn path_oriented_backends_allow_absolute_paths_but_never_traversal() {
        // `link` legitimately names a filesystem path — an absolute path is allowed.
        assert!(Validator::validate_package_name_for("/home/me/.vimrc", "link").is_ok());
        assert!(Validator::validate_package_name_for("/tmp/linix-link-src", "link").is_ok());
        // …but `..` traversal is STILL blocked, for every backend including path-oriented ones.
        assert!(Validator::validate_package_name_for("/home/../etc/shadow", "link").is_err());
        assert!(Validator::validate_package_name_for("../secrets", "link").is_err());
        // …and shell-injection characters are still blocked (backslash, $(), etc.).
        assert!(Validator::validate_package_name_for("/tmp/$(rm -rf)", "link").is_err());
    }

    /// `btrfs:` names a subvolume by its filesystem path — `btrfs subvolume create <path>` is
    /// the whole install — and it was absent from the path-oriented list until 2026-07-30. So
    /// the one backend whose name is *literally* a filesystem path was the one the list forgot,
    /// and no declaration of it could be written:
    ///
    /// ```text
    /// $ linix -y install btrfs:/mnt/data/vol
    /// Error: Validation error: Path traversal detected in name: /mnt/data/vol
    /// ```
    ///
    /// Found by the first privileged container run in the project's history, because until
    /// there was a real btrfs filesystem to install into, nothing ever tried.
    ///
    /// The family is every backend whose name may begin with a separator. `lvm:vg/lv`,
    /// `zfs:pool/dataset` and `setting:SCHEMA/KEY` all carry a separator and never a leading
    /// one, so the strict rule is right for them and they are asserted here to keep it that way.
    #[test]
    fn a_backend_whose_name_is_a_path_may_say_so_and_the_others_may_not() {
        for good in [
            "/mnt/data/vol",
            "/mnt/linix-btrfs/canary",
            "/.snapshots/root",
        ] {
            assert!(
                Validator::validate_package_name_for(good, "btrfs").is_ok(),
                "`btrfs:{good}` is a subvolume path and the install runs `subvolume create` on it"
            );
        }
        // The bans that make widening the list narrow: traversal, injection, and the allowlist.
        assert!(Validator::validate_package_name_for("/mnt/../etc/shadow", "btrfs").is_err());
        assert!(Validator::validate_package_name_for("/mnt/$(id)", "btrfs").is_err());

        // The siblings that must NOT be widened — each names a path-shaped thing that is not a
        // filesystem path, so a leading separator is still an injection attempt.
        for (name, backend) in [
            ("/vg0/data", "lvm"),
            ("/tank/data", "zfs"),
            ("/org.gnome.desktop/idle-delay", "setting"),
        ] {
            assert!(
                Validator::validate_package_name_for(name, backend).is_err(),
                "`{backend}:` names {name} with no leading separator; allowing one would widen \
                 the guard for nothing"
            );
        }
    }

    /// `winget`'s own identifiers, and the four things widening the allowlist must NOT do.
    ///
    /// Found by running the native sweep: the grammar was taught to accept a backslash in a
    /// name (G-2) and this validator was not, so `adopt` wrote 340 winget rows it believed it
    /// could write and the next command could not parse the file — `adopted.txt:78`, a wedged
    /// model, which is E1's class arriving through the other door.
    #[test]
    fn winget_identifiers_are_names_and_the_widening_stops_there() {
        for name in [
            r"ARP\Machine\X64\{8BD2A40D-67A6-45F5-877D-6D9D04C9D5A2}",
            r"ARP\Machine\X86\ILST_30_2_1",
            r"MSIX\Microsoft.AV1VideoExtension_2.0.24.0_x64__8wekyb3d8bbwe",
            "7zip.7zip",
        ] {
            assert!(
                Validator::validate_package_name_for(name, "winget").is_ok(),
                "winget prints `{name}` and cannot be handed it back"
            );
        }

        // 1. Only winget. Every other backend keeps the strict allowlist.
        assert!(
            Validator::validate_package_name_for(r"ARP\Machine\X64\thing", "cargo").is_err(),
            "the widening leaked to a backend whose manager never prints such a name"
        );
        // 2. Traversal is still forbidden, for winget as for everything else.
        assert!(
            Validator::validate_package_name_for(r"ARP\..\..\Windows\System32", "winget").is_err(),
            "`..` must stay refused whatever else the name may carry"
        );
        // 3. The shell metacharacters that are NOT part of a Windows identifier stay blocked.
        for hostile in [
            r"ARP\Machine; rm -rf /",
            r"ARP\Machine`whoami`",
            r"ARP\Machine$(id)",
            r"ARP\Machine|cat",
        ] {
            assert!(
                Validator::validate_package_name_for(hostile, "winget").is_err(),
                "`{hostile}` is not an identifier, it is a command line"
            );
        }
        // 4. And the ordinary names every other backend depends on still pass.
        for (name, backend) in [
            ("@angular/cli", "npm"),
            ("serde_json", "cargo"),
            ("sharkdp/fd", "github"),
        ] {
            assert!(
                Validator::validate_package_name_for(name, backend).is_ok(),
                "`{name}` stopped being a legal {backend} name"
            );
        }
    }
}
