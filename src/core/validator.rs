use crate::core::{Error, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::{Path, PathBuf};
use tracing::warn;

/// The allowlist must stay wide enough for names that are legitimately not bare words:
/// npm `@scope`, github `owner/repo`, versioned `pkg:1.2+build`.
static PACKAGE_NAME_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-zA-Z0-9._@:+/-]+$").unwrap());

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
        matches!(backend, "link" | "web" | "github" | "appimage")
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

        if !PACKAGE_NAME_REGEX.is_match(name) {
            return Err(Error::Validation(format!(
                "Invalid characters in package name: {}",
                printable(name)
            )));
        }

        if SHELL_INJECTION_REGEX.is_match(name) {
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
}
