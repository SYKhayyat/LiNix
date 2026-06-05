use crate::core::{
    CommandExecutor, Package, Result, PackageSpec, 
    BackendCore, Installable, Queryable, Upgradable
};
use crate::parsers::utils::sanitize;
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info};

/// Core backend implementation for Flatpak applications.
pub struct FlatpakBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
    pub available: OnceCell<bool>,
    /// Backend-specific settings like default scope (user vs system).
    pub settings: HashMap<String, String>,
}

impl FlatpakBackendCore {
    pub fn new(executor: CommandExecutor, settings: HashMap<String, String>) -> Self {
        Self { 
            executor, 
            name: "flatpak".to_string(),
            available: OnceCell::new(),
            settings 
        }
    }

    /// Helper to determine if the manager should operate in --user or --system scope.
    pub fn scope_args(&self) -> Vec<&str> {
        if self.settings.get("user").map(|v| v == "true").unwrap_or(false) {
            vec!["--user"]
        } else {
            vec!["--system"]
        }
    }
}

#[async_trait]
impl BackendCore for FlatpakBackendCore {
    fn name(&self) -> &str { &self.name }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.executor.command_exists_sync("flatpak"))
    }
}

pub struct FlatpakInstallable {
    pub core: Arc<FlatpakBackendCore>,
}

#[async_trait]
impl Installable for FlatpakInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        if specs.is_empty() { return Ok(()); }

        let mut args = self.core.scope_args();
        args.extend(["install", "-y", "--noninteractive"]);
        
        let names: Vec<String> = specs.iter().map(|s| s.name.clone()).collect();
        let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        args.extend(name_refs);

        info!("Flatpak: Installing {} package(s)...", specs.len());
        self.core.executor.run_exclusive("flatpak", "flatpak", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        if names.is_empty() { return Ok(()); }

        let mut args = self.core.scope_args();
        args.extend(["uninstall", "-y", "--noninteractive"]);
        let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        args.extend(name_refs);

        info!("Flatpak: Removing {} package(s)...", names.len());
        self.core.executor.run_exclusive("flatpak", "flatpak", &args, sudo).await?;
        Ok(())
    }
}

pub struct FlatpakQueryable {
    pub core: Arc<FlatpakBackendCore>,
}

#[async_trait]
impl Queryable for FlatpakQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.core.executor.run_output("flatpak", &["list", "--app", "--columns=application,version"], false).await?;
        let mut packages = Vec::new();

        for line in sanitize(&out).lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                packages.push(Package::with_version(parts[0], parts[1], "flatpak"));
            } else if !line.is_empty() {
                packages.push(Package::new(line.trim(), "flatpak"));
            }
        }
        Ok(packages)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}

pub struct FlatpakUpgradable {
    pub core: Arc<FlatpakBackendCore>,
}

#[async_trait]
impl Upgradable for FlatpakUpgradable {
    async fn update(&self, sudo: bool) -> Result<()> {
        let mut args = self.core.scope_args();
        args.push("update");
        debug!("Flatpak: Refreshing remotes...");
        self.core.executor.run_exclusive("flatpak", "flatpak", &args, sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        let mut args = self.core.scope_args();
        args.extend(["update", "-y", "--noninteractive"]);
        info!("Flatpak: Upgrading all applications...");
        self.core.executor.run_exclusive("flatpak", "flatpak", &args, sudo).await?;
        Ok(())
    }

    async fn clean_orphans(&self, sudo: bool) -> Result<()> {
        let mut args = self.core.scope_args();
        args.extend(["uninstall", "--unused", "-y", "--noninteractive"]);
        info!("Flatpak: Removing unused runtimes and extensions...");
        self.core.executor.run_exclusive("flatpak", "flatpak", &args, sudo).await?;
        Ok(())
    }
}