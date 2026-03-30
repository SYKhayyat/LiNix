use crate::core::{Package, PackageSpec, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HealthStatus { Ok, Warning, Error }

pub struct HealthReport {
    pub status: HealthStatus,
    pub message: Option<String>,
}

#[async_trait]
pub trait PackageManager: Send + Sync {
    fn name(&self) -> &str;
    fn is_available(&self) -> bool;
    async fn install(&self, packages: &[String], sudo: bool) -> Result<()>;
    async fn remove(&self, packages: &[String], sudo: bool) -> Result<()>;
    async fn list_installed(&self) -> Result<Vec<Package>>;

    async fn update(&self, _sudo: bool) -> Result<()> { Ok(()) }
    async fn upgrade(&self, _sudo: bool) -> Result<()> { Ok(()) }
    async fn search(&self, _query: &str) -> Result<Vec<Package>> { Ok(vec![]) }
    async fn clean_orphans(&self, _sudo: bool) -> Result<()> { Ok(()) }
    fn supports_orphan_cleanup(&self) -> bool { false }
    async fn info(&self, _package: &str) -> Result<Option<Package>> { Ok(None) }

    async fn install_with_options(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        let packages: Vec<String> = specs.iter().map(|s| s.name.clone()).collect();
        self.install(&packages, sudo).await
    }

    async fn add_repo(&self, name: &str, url: &str, _sudo: bool) -> Result<()> {
        Err(crate::core::Error::UnsupportedPlatform(format!("{} cannot add repos ({} -> {})", self.name(), name, url)))
    }
    async fn remove_repo(&self, name: &str, _sudo: bool) -> Result<()> {
        Err(crate::core::Error::UnsupportedPlatform(format!("{} cannot remove repos ({})", self.name(), name)))
    }
    async fn list_repos(&self) -> Result<Vec<(String, String)>> {
        Err(crate::core::Error::UnsupportedPlatform(format!("{} cannot list repos", self.name())))
    }
    
    async fn check_health(&self) -> Result<HealthReport> {
        if self.is_available() {
            Ok(HealthReport { status: HealthStatus::Ok, message: None })
        } else {
            Ok(HealthReport { 
                status: HealthStatus::Error, 
                message: Some(format!("The underlying CLI tool for '{}' is missing from PATH.", self.name())) 
            })
        }
    }
}