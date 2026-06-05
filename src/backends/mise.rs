use crate::core::{
    BackendCore, CommandExecutor, Installable, Package, PackageSpec, 
    Queryable, Result, Upgradable, Error
};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tracing::info;

/// Core backend implementation for Mise (runtime version manager).
pub struct MiseBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl MiseBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "mise".to_string(),
        }
    }
}

#[async_trait]
impl BackendCore for MiseBackendCore {
    fn name(&self) -> &str { &self.name }

    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("mise")
    }
}

pub struct MiseInstallable {
    pub core: Arc<MiseBackendCore>,
}

#[async_trait]
impl Installable for MiseInstallable {
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        for spec in specs {
            let version = spec.options.get("version").map(|v| v.as_str()).unwrap_or("latest");
            let tool_spec = format!("{}@{}", spec.name, version);

            info!("Mise: Installing global tool {}...", tool_spec);
            self.core.executor.run_exclusive("mise", "mise", &["use", "-g", &tool_spec], false).await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _: bool) -> Result<()> {
        for name in names {
            info!("Mise: Uninstalling tool {}...", name);
            self.core.executor.run_exclusive("mise", "mise", &["uninstall", name], false).await?;
        }
        Ok(())
    }
}

pub struct MiseQueryable {
    pub core: Arc<MiseBackendCore>,
}

#[async_trait]
impl Queryable for MiseQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self.core.executor.run_output("mise", &["ls", "--json"], false).await?;
        if output.is_empty() || output == "{}" { return Ok(vec![]); }

        let json: Value = serde_json::from_str(&output).map_err(|e| Error::Other(format!("Mise JSON error: {}", e)))?;
        let mut packages = Vec::new();

        if let Some(tools) = json.as_object() {
            for (name, versions) in tools {
                if let Some(v_list) = versions.as_array() {
                    for v_obj in v_list {
                        let version = v_obj.get("version")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        
                        let mut p = Package::with_version(name, version, "mise");
                        if let Some(source) = v_obj.get("source")
                            .and_then(|s| s.get("type"))
                            .and_then(|t| t.as_str()) 
                        {
                            p.properties.insert("source_type".into(), source.to_string());
                        }
                        packages.push(p);
                    }
                }
            }
        }
        Ok(packages)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter()
            .filter(|p| p.properties.get("source_type").map(|s| s == "global").unwrap_or(false))
            .collect())
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let output = self.core.executor.run_output("mise", &["plugins", "ls", "--all", "--urls"], false).await?;
        for line in output.lines() {
            if let Some((plugin_name, url)) = line.split_once(' ') {
                if plugin_name.trim() == name {
                    let mut p = Package::new(name, "mise");
                    p.properties.insert("repository_url".into(), url.trim().to_string());
                    return Ok(Some(p));
                }
            }
        }
        Ok(None)
    }
}

pub struct MiseUpgradable {
    pub core: Arc<MiseBackendCore>,
}

#[async_trait]
impl Upgradable for MiseUpgradable {
    async fn update(&self, _: bool) -> Result<()> {
        info!("Mise: Updating plugin repository metadata...");
        self.core.executor.run("mise", &["plugins", "update"], false).await?;
        Ok(())
    }

    async fn upgrade(&self, _: bool) -> Result<()> {
        info!("Mise: Upgrading all globally installed tools...");
        self.core.executor.run_exclusive("mise", "mise", &["upgrade"], false).await?;
        Ok(())
    }

    async fn clean_orphans(&self, _: bool) -> Result<()> {
        info!("Mise: Pruning unused tool versions from cache...");
        self.core.executor.run("mise", &["prune", "--force"], false).await?;
        Ok(())
    }
}