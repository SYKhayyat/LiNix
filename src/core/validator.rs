use crate::core::{Error, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::Path;

static PACKAGE_NAME_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-zA-Z0-9._@:+/-]+$").unwrap());

static DANGEROUS_COMMANDS: &[&str] = &[
    "rm -rf /",
    "dd if=/dev/zero",
    "mkfs",
    ":(){ :|:& };:",
    "chmod -R 777 /",
];

/// Validates user input and system state
pub struct Validator;

impl Validator {
    /// Validate a package name
    pub fn validate_package_name(name: &str) -> Result<()> {
        if name.is_empty() {
            return Err(Error::Validation("Package name cannot be empty".into()));
        }

        if name.len() > 256 {
            return Err(Error::Validation("Package name too long".into()));
        }

        if !PACKAGE_NAME_REGEX.is_match(name) {
            return Err(Error::Validation(format!(
                "Invalid package name '{}': must contain only alphanumeric characters, dots, hyphens, underscores, @, :, +, or /",
                name
            )));
        }

        Ok(())
    }

    /// Validate a list of package names
    pub fn validate_package_names(names: &[String]) -> Result<()> {
        for name in names {
            Self::validate_package_name(name)?;
        }
        Ok(())
    }

    /// Check if a command appears dangerous
    pub fn is_dangerous_command(cmd: &str) -> bool {
        let cmd_lower = cmd.to_lowercase();
        DANGEROUS_COMMANDS
            .iter()
            .any(|dangerous| cmd_lower.contains(&dangerous.to_lowercase()))
    }

    /// Validate a command before execution
    pub fn validate_command(cmd: &str, args: &[&str]) -> Result<()> {
        let full_cmd = format!("{} {}", cmd, args.join(" "));

        if Self::is_dangerous_command(&full_cmd) {
            return Err(Error::Validation(format!(
                "Potentially dangerous command blocked: {}",
                full_cmd
            )));
        }

        Ok(())
    }

    /// Validate a file path
    pub fn validate_path(path: &Path) -> Result<()> {
        if !path.exists() {
            return Err(Error::Validation(format!(
                "Path does not exist: {}",
                path.display()
            )));
        }

        // Check for path traversal attempts
        let canonical = path
            .canonicalize()
            .map_err(|e| Error::Validation(format!("Cannot canonicalize path: {}", e)))?;

        let path_str = canonical.to_string_lossy();
        if path_str.contains("..") {
            return Err(Error::Validation("Path traversal detected".into()));
        }

        Ok(())
    }

    /// Validate backend name
    pub fn validate_backend_name(name: &str) -> Result<()> {
        if name.is_empty() {
            return Err(Error::Validation("Backend name cannot be empty".into()));
        }

        if !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Err(Error::Validation(format!(
                "Invalid backend name '{}': must be alphanumeric with underscores or hyphens",
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
    fn test_validate_package_name() {
        assert!(Validator::validate_package_name("valid-package").is_ok());
        assert!(Validator::validate_package_name("package_name").is_ok());
        assert!(Validator::validate_package_name("package.name").is_ok());
        assert!(Validator::validate_package_name("@scope/package").is_ok());

        assert!(Validator::validate_package_name("").is_err());
        assert!(Validator::validate_package_name("invalid package").is_err());
        assert!(Validator::validate_package_name("invalid;package").is_err());
    }

    #[test]
    fn test_dangerous_commands() {
        assert!(Validator::is_dangerous_command("rm -rf /"));
        assert!(Validator::is_dangerous_command("sudo rm -rf /"));
        assert!(!Validator::is_dangerous_command("ls -la"));
        assert!(!Validator::is_dangerous_command("apt install package"));
    }

    #[test]
    fn test_validate_backend_name() {
        assert!(Validator::validate_backend_name("apt").is_ok());
        assert!(Validator::validate_backend_name("dnf-manager").is_ok());
        assert!(Validator::validate_backend_name("").is_err());
        assert!(Validator::validate_backend_name("invalid backend").is_err());
    }
}
