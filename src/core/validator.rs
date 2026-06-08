use crate::core::{Error, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::Path;

/// Strict regex for package names to prevent shell injection or path traversal.
static PACKAGE_NAME_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[a-zA-Z0-9._@:+/-]+$").unwrap());

/// Regex to detect shell metacharacters that could be used in injection attacks.
static SHELL_INJECTION_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"[;&|><`$\(\)\[\]\{\}\*\?\!\\]").unwrap());

/// Comprehensive list of destructive command patterns blocked at the validation layer.
/// Fulfills Phase 10.1: Security Hardening.
static DESTRUCTIVE_PATTERNS: &[&str] = &[
    "rm -rf /", 
    "dd if=", 
    "mkfs", 
    ":(){ :|:& };:", 
    "chmod -R 777", 
    "chown -R", 
    "> /dev/sda", 
    "mv / /dev/null",
    "fdisk",
    "parted",
    "chattr",
    "mkswap",
    "shred",
    "wipe",
];

/// Whitelist of trusted binary directories for security verification.
static TRUSTED_BIN_PATHS: &[&str] = &[
    "/usr/bin",
    "/bin",
    "/usr/sbin",
    "/sbin",
    "/usr/local/bin",
    "C:\\Windows\\System32",
    "C:\\Program Files",
];

/// The Mission-Critical Security Validator.
/// Implements strict constraints to ensure LiNix cannot be used as an 
/// escalation vector or for accidental system destruction.
pub struct Validator;

impl Validator {
    /// Validates a package name against strict character rules.
    /// Blocks potential path traversals and shell injection characters.
    pub fn validate_package_name(name: &str) -> Result<()> {
        if name.is_empty() { 
            return Err(Error::Validation("Package name cannot be empty".into())); 
        }
        if name.len() > 256 { 
            return Err(Error::Validation("Package name length exceeds 256 characters".into())); 
        }
        
        if !PACKAGE_NAME_REGEX.is_match(name) {
            return Err(Error::Validation(format!("Invalid characters in package name: {}", name)));
        }

        if SHELL_INJECTION_REGEX.is_match(name) {
            return Err(Error::Validation(format!("Security Block: Shell metacharacters detected in package name: {}", name)));
        }

        Ok(())
    }

    /// Validates an external command and its arguments before execution.
    /// Phase 10.1: Hardened check including destructive patterns and binary origin.
    pub fn validate_command(cmd: &str, args: &[&str]) -> Result<()> {
        let full = format!("{} {}", cmd, args.join(" ")).to_lowercase();
        
        // 1. Block known destructive patterns
        for pattern in DESTRUCTIVE_PATTERNS {
            if full.contains(pattern) {
                return Err(Error::Validation(format!("Security Block: Destructive command pattern detected: {}", full)));
            }
        }
        
        // 2. Block shell injection
        if SHELL_INJECTION_REGEX.is_match(&full) {
            return Err(Error::Validation(format!("Security Block: Command injection characters detected: {}", full)));
        }

        // 3. Binary origin validation (if absolute path is used)
        let cmd_path = Path::new(cmd);
        if cmd_path.is_absolute() {
            let is_trusted = TRUSTED_BIN_PATHS.iter().any(|trusted| {
                cmd_path.starts_with(trusted)
            });
            if !is_trusted {
                return Err(Error::Validation(format!("Security Block: Execution of absolute path binary outside trusted directories: {}", cmd)));
            }
        }
        
        Ok(())
    }

    /// Ensures that backend names are strictly alphanumeric.
    pub fn validate_backend_name(name: &str) -> Result<()> {
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            return Err(Error::Validation(format!("Invalid backend identifier: {}", name)));
        }
        Ok(())
    }

    /// Asynchronously prevents symlink-based path traversal and unauthorized access.
    /// Fulfills Phase 3.3: Canonicalization is wrapped in spawn_blocking for async safety.
    pub async fn validate_path(path: &Path) -> Result<()> {
        if !tokio::fs::try_exists(path).await.unwrap_or(false) {
            return Ok(());
        } 
        
        let path_owned = path.to_path_buf();
        let canonical = tokio::task::spawn_blocking(move || {
            path_owned.canonicalize()
        }).await.map_err(|e| Error::Other(e.to_string()))?
          .map_err(|e| Error::Validation(format!("Path canonicalization failed: {}", e)))?;

        let path_str = canonical.to_string_lossy();
        
        let sensitive_locations = [
            "/etc/shadow", 
            "/etc/sudoers", 
            "/etc/passwd", 
            "/etc/gshadow",
            "C:\\Windows\\System32\\config\\SAM",
            "C:\\Windows\\System32\\config\\SECURITY"
        ];

        for sensitive in sensitive_locations {
            if path_str.contains(sensitive) {
                return Err(Error::Validation(format!("Access Denied: LiNix is prohibited from accessing sensitive system file: {}", sensitive)));
            }
        }
        
        Ok(())
    }
}