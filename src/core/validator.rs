use crate::core::{Error, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::{Path, PathBuf};
use tracing::warn;

/// Strict regex for package names.
/// Allows alphanumeric, dots, underscores, @ scopes, and slashes for github.
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

/// Bug 6: Expanded trusted paths for Windows and Linux.
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
    /// Validates package names against injection and traversal.
    pub fn validate_package_name(name: &str) -> Result<()> {
        if name.is_empty() {
            return Err(Error::Validation("Empty package name".into()));
        }
        if name.len() > 256 {
            return Err(Error::Validation("Name too long".into()));
        }

        // Bug 2: Explicitly block directory traversal
        if name.contains("..") || name.starts_with('/') || name.starts_with('\\') {
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

    /// Hardened command validation.
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
            // Bug 6: Use canonical path matching for binary origins
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

    /// Bug 2: Strict Canonical Path Validation.
    /// Replaces substring "contains" checks with prefix matching on resolved paths.
    pub async fn validate_path(path: &Path) -> Result<PathBuf> {
        if !tokio::fs::try_exists(path).await.unwrap_or(false) {
            return Ok(path.to_path_buf());
        }

        let path_owned = path.to_path_buf();
        // Resolve symlinks and ".."
        let canonical = tokio::task::spawn_blocking(move || path_owned.canonicalize())
            .await
            .map_err(|e| Error::Other(e.to_string()))?
            .map_err(|e| Error::Validation(format!("Path resolution failed: {}", e)))?;

        // Use strict prefix matching against forbidden zones
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

    /// Ensures backend names are alphanumeric safely.
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
