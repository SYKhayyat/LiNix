use crate::core::{Package, Result};
use async_trait::async_trait;

/// Trait that all package managers must implement
#[async_trait]
pub trait PackageManager: Send + Sync {
    /// Get the name of this package manager
    fn name(&self) -> &str;

    /// Check if this package manager is available on the system
    fn is_available(&self) -> bool;

    /// Install packages
    async fn install(&self, packages: &[String], sudo: bool) -> Result<()>;

    /// Remove/uninstall packages
    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()>;

    /// List all installed packages
    async fn list_installed(&self) -> Result<Vec<Package>>;

    /// Update package database/cache
    async fn update(&self, sudo: bool) -> Result<()> {
        // Default implementation does nothing
        let _ = sudo;
        Ok(())
    }

    /// Upgrade all packages
    async fn upgrade(&self, sudo: bool) -> Result<()> {
        // Default implementation does nothing
        let _ = sudo;
        Ok(())
    }

    /// Search for packages
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // Default implementation returns empty list
        let _ = query;
        Ok(Vec::new())
    }

    /// Clean orphaned packages
    async fn clean_orphans(&self, sudo: bool) -> Result<()> {
        // Default implementation does nothing
        let _ = sudo;
        Ok(())
    }

    /// Check if this manager supports orphan cleanup
    fn supports_orphan_cleanup(&self) -> bool {
        false
    }

    /// Get information about a specific package
    async fn info(&self, package: &str) -> Result<Option<Package>> {
        let _ = package;
        Ok(None)
    }

    /// Check if a package is installed
    async fn is_installed(&self, package: &str) -> Result<bool> {
        let installed = self.list_installed().await?;
        Ok(installed.iter().any(|p| p.name == package))
    }
}
