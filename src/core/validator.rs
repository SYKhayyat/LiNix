use crate::core::{Error, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::Path;

/// Strictly allowed characters in package names.
static PACKAGE_NAME_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-zA-Z0-9._@:+/-]+$").unwrap());

/// Point #4: Characters used for command chaining or injection.
static SHELL_INJECTION_REGEX: Lazy<Regex> = 
    Lazy::new(|| Regex::new(r"[;&|><`$\(\)\[\]\{\}\*\?\!\\]").unwrap());

/// Point #4: Blacklist of strings that indicate attempted system destruction.
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

pub struct Validator;

impl Validator {
    /// Validates a package name for backend-compatible characters and shell safety.
    pub fn validate_package_name(name: &str) -> Result<()> {
        if name.is_empty() {
            return Err(Error::Validation("Package name cannot be empty".into()));
        }

        if name.len() > 256 {
            return Err(Error::Validation("Package name exceeds 256 character limit".into()));
        }

        if !PACKAGE_NAME_REGEX.is_match(name) {
            return Err(Error::Validation(format!(
                "Invalid package name '{}': Contains forbidden characters.",
                name
            )));
        }

        if SHELL_INJECTION_REGEX.is_match(name) {
            return Err(Error::Validation(format!(
                "Security Alert: Blocked package name '{}' containing shell metacharacters.",
                name
            )));
        }

        Ok(())
    }

    /// Validates multiple names.
    pub fn validate_package_names(names: &[String]) -> Result<()> {
        for name in names {
            Self::validate_package_name(name)?;
        }
        Ok(())
    }

    /// Checks for dangerous CLI arguments or destructive patterns.
    pub fn validate_command(cmd: &str, args: &[&str]) -> Result<()> {
        let full_cmd = format!("{} {}", cmd, args.join(" "));
        let cmd_lower = full_cmd.to_lowercase();

        for pattern in DESTRUCTIVE_PATTERNS {
            if cmd_lower.contains(pattern) {
                return Err(Error::Validation(format!(
                    "Security Block: Destructive command detected and blocked: {}",
                    full_cmd
                )));
            }
        }

        if SHELL_INJECTION_REGEX.is_match(&full_cmd) {
            return Err(Error::Validation(format!(
                "Security Block: Command contains forbidden shell metacharacters: {}",
                full_cmd
            )));
        }

        Ok(())
    }

    /// Robust path validation using canonicalization to catch hidden traversal attempts.
    pub fn validate_path(path: &Path) -> Result<()> {
        if !path.exists() {
            return Err(Error::Validation(format!("Path does not exist: {}", path.display())));
        }

        // Resolves symlinks and removes ".." segments
        let canonical = path
            .canonicalize()
            .map_err(|e| Error::Validation(format!("Path security check failed: {}", e)))?;

        let path_str = canonical.to_string_lossy();

        // Prevent manipulation of core system authentication files
        if path_str.contains("/etc/shadow") || path_str.contains("/etc/sudoers") {
            return Err(Error::Validation("Access Denied: Unsafe path access attempt.".into()));
        }

        Ok(())
    }

    /// Validates the backend identifier.
    pub fn validate_backend_name(name: &str) -> Result<()> {
        if name.is_empty() {
            return Err(Error::Validation("Backend name cannot be empty".into()));
        }

        if !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            return Err(Error::Validation(format!(
                "Invalid backend name '{}': must be alphanumeric.",
                name
            )));
        }

        Ok(())
    }
}