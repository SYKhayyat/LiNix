use crate::core::{Package, PackageSpec, Result, Error};
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
    
    async fn list_manual(&self) -> Result<Vec<Package>> { self.list_installed().await }
    async fn update(&self, _sudo: bool) -> Result<()> { Ok(()) }
    async fn upgrade(&self, _sudo: bool) -> Result<()> { Ok(()) }
    async fn search(&self, _query: &str) -> Result<Vec<Package>> { Ok(vec![]) }
    async fn clean_orphans(&self, _sudo: bool) -> Result<()> { Ok(()) }
    fn supports_orphan_cleanup(&self) -> bool { false }

    async fn info(&self, package: &str) -> Result<Option<Package>> { 
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == package))
    }

    async fn install_with_options(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        let names: Vec<String> = specs.iter().map(|s| s.name.clone()).collect();
        self.install(&names, sudo).await
    }

    // Repository Management (Now used in Main)
    async fn add_repo(&self, _name: &str, _url: &str, _sudo: bool) -> Result<()> {
        Err(Error::UnsupportedPlatform(format!("{} cannot add repos via CLI", self.name())))
    }
    async fn remove_repo(&self, _name: &str, _sudo: bool) -> Result<()> {
        Err(Error::UnsupportedPlatform(format!("{} cannot remove repos via CLI", self.name())))
    }
    async fn list_repos(&self) -> Result<Vec<(String, String)>> { Ok(vec![]) }

    async fn check_health(&self) -> Result<HealthReport> {
        if self.is_available() {
            Ok(HealthReport { status: HealthStatus::Ok, message: None })
        } else {
            Ok(HealthReport { status: HealthStatus::Error, message: Some("Binary not in PATH".into()) })
        }
    }
}