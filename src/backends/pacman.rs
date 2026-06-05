use crate::core::{
    BackendCore, CommandExecutor, Installable, Package, PackageSpec,
    Queryable, Result, Upgradable
};
use crate::parsers::pacman;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;

/// Core backend implementation for Pacman (Arch Linux).
pub struct PacmanBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl PacmanBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "pacman".to_string(),
        }
    }
}

#[async_trait]
impl BackendCore for PacmanBackendCore {
    fn name(&self) -> &str { &self.name }

    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("pacman")
    }
}

pub struct PacmanInstallable {
    pub core: Arc<PacmanBackendCore>,
}

#[async_trait]
impl Installable for PacmanInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        if specs.is_empty() { return Ok(()); }
        
        let mut args = vec!["-S", "--noconfirm", "--needed"];
        let names: Vec<String> = specs.iter().map(|s| s.name.clone()).collect();
        for name in &names {
            args.push(name);
        }

        info!("Pacman: Installing {} package(s)...", specs.len());
        self.core.executor.run_exclusive("pacman", "pacman", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        if names.is_empty() { return Ok(()); }
        
        let mut args = vec!["-Rs", "--noconfirm"];
        for name in names {
            args.push(name);
        }

        info!("Pacman: Removing {} package(s)...", names.len());
        self.core.executor.run_exclusive("pacman", "pacman", &args, sudo).await?;
        Ok(())
    }
}

pub struct PacmanQueryable {
    pub core: Arc<PacmanBackendCore>,
}

#[async_trait]
impl Queryable for PacmanQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.core.executor.run_output("pacman", &["-Q"], false).await?;
        Ok(pacman::parse_list(&output))
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        let output = self.core.executor.run_output("pacman", &["-Qe"], false).await?;
        Ok(pacman::parse_list(&output))
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}

pub struct PacmanUpgradable {
    pub core: Arc<PacmanBackendCore>,
}

#[async_trait]
impl Upgradable for PacmanUpgradable {
    async fn update(&self, sudo: bool) -> Result<()> {
        info!("Pacman: Refreshing package databases...");
        self.core.executor.run("pacman", &["-Sy"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Pacman: Upgrading system packages...");
        self.core.executor.run_exclusive("pacman", "pacman", &["-Syu", "--noconfirm"], sudo).await?;
        Ok(())
    }

    async fn clean_orphans(&self, sudo: bool) -> Result<()> {
        let orphans = self.core.executor.run_output("pacman", &["-Qdtq"], false).await?;
        let orphan_list: Vec<&str> = orphans.lines().filter(|l| !l.is_empty()).collect();
        
        if orphan_list.is_empty() {
            return Ok(());
        }

        info!("Pacman: Removing {} orphan packages...", orphan_list.len());
        let mut args = vec!["-Rs", "--noconfirm"];
        args.extend(orphan_list);
        self.core.executor.run_exclusive("pacman", "pacman", &args, sudo).await?;
        Ok(())
    }
}