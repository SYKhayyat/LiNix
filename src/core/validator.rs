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

/// Destructive patterns blocked at the command layer.
static DESTRUCTIVE_PATTERNS: &[&str] = &[
    "rm -rf /",
    "dd if=",
    "mkfs",
    ":(){ :|:& };:",
    "chmod -R 777",
    "chown -R",
    "> /dev/sda",
    "mv / /dev/null",
];

static TRUSTED_BIN_PATHS: &[&str] = &[
    "/usr/bin",
    "/bin",
    "/usr/sbin",
    "/sbin",
    "/usr/local/bin",
    "C:\\Windows\\System32",
    "C:\\Program Files",
    "C:\\Program Files (x86)",
];

/// Sensitive system paths that LiNix is prohibited from accessing.
static FORBIDDEN_PATHS: &[&str] = &[
    "/etc/shadow",
    "/etc/sudoers",
    "/etc/passwd",
    "/etc/gshadow",
    "C:\\Windows\\System32\\config\\SAM",
    "C:\\Windows\\System32\\config\\SECURITY",
];

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
                name
            )));
        }
        // A leading path separator normally signals an absolute-path injection attempt — but
        // for a path/URL-oriented backend (e.g. `link`, whose name IS a path) it is valid.
        if !Self::is_path_oriented_backend(backend)
            && (name.starts_with('/') || name.starts_with('\\'))
        {
            return Err(Error::Validation(format!(
                "Path traversal detected in name: {}",
                name
            )));
        }

        if !PACKAGE_NAME_REGEX.is_match(name) {
            return Err(Error::Validation(format!(
                "Invalid characters in package name: {}",
                name
            )));
        }

        if SHELL_INJECTION_REGEX.is_match(name) {
            return Err(Error::Validation(
                "Shell injection characters detected".into(),
            ));
        }

        Ok(())
    }

    pub fn validate_command(cmd: &str, args: &[&str]) -> Result<()> {
        let full = format!("{} {}", cmd, args.join(" ")).to_lowercase();

        for pattern in DESTRUCTIVE_PATTERNS {
            if full.contains(pattern) {
                return Err(Error::Validation(format!(
                    "Destructive command blocked: {}",
                    full
                )));
            }
        }

        if SHELL_INJECTION_REGEX.is_match(&full) {
            return Err(Error::Validation("Command injection detected".into()));
        }

        let cmd_path = Path::new(cmd);
        if cmd_path.is_absolute() {
            // Canonicalize before the prefix check: a symlink or `..` in an untrusted
            // location would otherwise present a trusted-looking prefix.
            let canonical_cmd = cmd_path
                .canonicalize()
                .unwrap_or_else(|_| cmd_path.to_path_buf());
            let is_trusted = TRUSTED_BIN_PATHS.iter().any(|trusted| {
                let t_path = Path::new(trusted);
                canonical_cmd.starts_with(t_path)
            });

            if !is_trusted {
                return Err(Error::Validation(format!(
                    "Untrusted binary origin: {}",
                    cmd
                )));
            }
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

        for forbidden in FORBIDDEN_PATHS {
            let f_path = Path::new(forbidden);
            if canonical.starts_with(f_path) {
                warn!(
                    "Security Block: Attempted access to forbidden path: {:?}",
                    canonical
                );
                return Err(Error::Permission(format!("Access Denied: {}", forbidden)));
            }
        }

        Ok(canonical)
    }

    pub fn validate_backend_name(name: &str) -> Result<()> {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Err(Error::Validation(format!(
                "Invalid backend identifier: {}",
                name
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
