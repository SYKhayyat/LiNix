use crate::core::{
    BackendCore, CommandExecutor, Installable, Package, PackageSpec, 
    Queryable, Result, Upgradable, MetadataProvider
};
use crate::parsers::dnf;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;

/// Core backend implementation for DNF (Fedora/RHEL).
pub struct DnfBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl DnfBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "dnf".to_string(),
        }
    }
}

#[async_trait]
impl BackendCore for DnfBackendCore {
    fn name(&self) -> &str { &self.name }

    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("dnf")
    }

    fn needs_root(&self) -> bool {
        // DNF system operations require root privileges.
        true
    }
}

#[async_trait]
impl MetadataProvider for DnfBackendCore {
    async fn get_dependencies(&self, name: &str) -> Result<Vec<String>> {
        // 'dnf repoquery --requires --resolve' identifies the actual packages needed.
        let output = self.executor.run_output("dnf", &["repoquery", "--requires", "--resolve", "--queryformat", "%{name}", name], false).await?;
        Ok(output.lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect())
    }
}

pub struct DnfInstallable {
    pub core: Arc<DnfBackendCore>,
}

#[async_trait]
impl Installable for DnfInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        if specs.is_empty() { return Ok(()); }
        let mut args = vec!["install", "-y"];
        let names: Vec<String> = specs.iter().map(|s| s.name.clone()).collect();
        for name in &names {
            args.push(name);
        }

        info!("DNF: Installing {} package(s)...", specs.len());
        self.core.executor.run_exclusive("dnf", "dnf", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        if names.is_empty() { return Ok(()); }
        let mut args = vec!["remove", "-y"];
        for name in names {
            args.push(name);
        }

        info!("DNF: Removing {} package(s)...", names.len());
        self.core.executor.run_exclusive("dnf", "dnf", &args, sudo).await?;
        Ok(())
    }
}

pub struct DnfQueryable {
    pub core: Arc<DnfBackendCore>,
}

#[async_trait]
impl Queryable for DnfQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.core.executor.run_output("rpm", &["-qa", "--queryformat", "%{NAME}|%{VERSION}\n"], false).await?;
        Ok(dnf::parse_rpm_qa(&output, "dnf"))
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // DNF 'userinstalled' list
        let output = self.core.executor.run_output("dnf", &["repoquery", "--userinstalled", "--qf", "%{name}|%{version}"], false).await?;
        Ok(dnf::parse_rpm_qa(&output, "dnf"))
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}

pub struct DnfUpgradable {
    pub core: Arc<DnfBackendCore>,
}

#[async_trait]
impl Upgradable for DnfUpgradable {
    async fn update(&self, sudo: bool) -> Result<()> {
        self.core.executor.run("dnf", &["makecache"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("DNF: Upgrading system packages...");
        self.core.executor.run_exclusive("dnf", "dnf", &["upgrade", "-y"], sudo).await?;
        Ok(())
    }

    async fn clean_orphans(&self, sudo: bool) -> Result<()> {
        info!("DNF: Removing unused dependencies...");
        self.core.executor.run_exclusive("dnf", "dnf", &["autoremove", "-y"], sudo).await?;
        Ok(())
    }
}