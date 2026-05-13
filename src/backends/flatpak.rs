use crate::core::{CommandExecutor, Package, Result, PackageSpec, Backend, Installable, Queryable, Upgradable};
use crate::parsers::utils::sanitize;
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use tracing::{debug, info};

/// Specialized manager for Flatpak applications.
/// Supports both --system (default) and --user scopes.
/// Uses the LockMap key "flatpak" to serialize installation and updates.
pub struct FlatpakManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    /// Backend-specific settings like default scope.
    settings: HashMap<String, String>,
}

impl FlatpakManager {
    pub fn new(executor: CommandExecutor, settings: HashMap<String, String>) -> Self {
        Self { 
            executor, 
            available: OnceCell::new(),
            settings 
        }
    }

    /// Helper to determine if the manager should operate in --user or --system scope.
    fn scope_args(&self) -> Vec<&str> {
        if self.settings.get("user").map(|v| v == "true").unwrap_or(false) {
            vec!["--user"]
        } else {
            vec!["--system"]
        }
    }
}

impl Backend for FlatpakManager {
    fn name(&self) -> &str { "flatpak" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.executor.command_exists_sync("flatpak"))
    }

    fn as_installable(&self) -> Option<&dyn Installable> { Some(self) }
    fn as_queryable(&self) -> Option<&dyn Queryable> { Some(self) }
    fn as_upgradable(&self) -> Option<&dyn Upgradable> { Some(self) }
}

#[async_trait]
impl Installable for FlatpakManager {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        if specs.is_empty() { return Ok(()); }

        let mut args = self.scope_args();
        // -y: assume yes, --noninteractive: don't prompt for auth if possible
        args.extend(["install", "-y", "--noninteractive"]);
        
        let names: Vec<String> = specs.iter().map(|s| s.name.clone()).collect();
        args.extend(names.iter().map(|s| s.as_str()));

        info!("Flatpak: Installing {} package(s)...", specs.len());
        // Flatpak mutations are serialized via the LockMap
        self.executor.run_exclusive("flatpak", "flatpak", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        if names.is_empty() { return Ok(()); }

        let mut args = self.scope_args();
        args.extend(["uninstall", "-y", "--noninteractive"]);
        args.extend(names.iter().map(|s| s.as_str()));

        info!("Flatpak: Removing {} package(s)...", names.len());
        self.executor.run_exclusive("flatpak", "flatpak", &args, sudo).await?;
        Ok(())
    }
}

#[async_trait]
impl Queryable for FlatpakManager {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        // We query specific columns to make parsing deterministic.
        let out = self.executor.run_output("flatpak", &["list", "--app", "--columns=application,version"], false).await?;
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
        // Flatpak list --app essentially returns the user-installed applications.
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}

#[async_trait]
impl Upgradable for FlatpakManager {
    async fn update(&self, sudo: bool) -> Result<()> {
        let mut args = self.scope_args();
        args.push("update");
        // Update check
        debug!("Flatpak: Refreshing remotes...");
        self.executor.run_exclusive("flatpak", "flatpak", &args, sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        let mut args = self.scope_args();
        args.extend(["update", "-y", "--noninteractive"]);
        info!("Flatpak: Upgrading all applications...");
        self.executor.run_exclusive("flatpak", "flatpak", &args, sudo).await?;
        Ok(())
    }

    async fn clean_orphans(&self, sudo: bool) -> Result<()> {
        let mut args = self.scope_args();
        args.extend(["uninstall", "--unused", "-y", "--noninteractive"]);
        info!("Flatpak: Removing unused runtimes and extensions...");
        self.executor.run_exclusive("flatpak", "flatpak", &args, sudo).await?;
        Ok(())
    }
}