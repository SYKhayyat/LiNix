use crate::core::{
    Backend, CommandExecutor, Installable, Package, PackageSpec, 
    Queryable, Result, Upgradable, Error
};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use serde_json::Value;
use tracing::{debug, info};

/// Specialized manager for Mise (formerly rtx).
/// Mise handles multiple runtime versions (Node, Python, Ruby, etc.) efficiently.
/// LiNix integrates Mise to manage global toolchains.
pub struct MiseManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl MiseManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            available: OnceCell::new(),
        }
    }
}

impl Backend for MiseManager {
    fn name(&self) -> &str { "mise" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| self.executor.command_exists_sync("mise"))
    }

    fn as_installable(&self) -> Option<&dyn Installable> { Some(self) }
    fn as_queryable(&self) -> Option<&dyn Queryable> { Some(self) }
    fn as_upgradable(&self) -> Option<&dyn Upgradable> { Some(self) }
}

#[async_trait]
impl Installable for MiseManager {
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        for spec in specs {
            // Logic: 'mise use -g' ensures the tool is installed and set in global config.
            // Format: mise use -g <plugin>@<version>
            let version = spec.options.get("version").map(|v| v.as_str()).unwrap_or("latest");
            let tool_spec = format!("{}@{}", spec.name, version);

            info!("Mise: Installing global tool {}...", tool_spec);
            // Mise handles its own locking, but we use the "mise" key for logical isolation
            self.executor.run_exclusive("mise", "mise", &["use", "-g", &tool_spec], false).await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _: bool) -> Result<()> {
        for name in names {
            info!("Mise: Uninstalling tool {}...", name);
            // Removes the tool from the Mise store and cleans up shims
            self.executor.run_exclusive("mise", "mise", &["uninstall", name], false).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl Queryable for MiseManager {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        // Use JSON for reliable parsing of multi-version toolchains
        let output = self.executor.run_output("mise", &["ls", "--json"], false).await?;
        if output.is_empty() || output == "{}" { return Ok(vec![]); }

        let json: Value = serde_json::from_str(&output).map_err(|e| Error::Other(format!("Mise JSON error: {}", e)))?;
        let mut packages = Vec::new();

        // Mise ls --json returns an object where keys are tool names
        if let Some(tools) = json.as_object() {
            for (name, versions) in tools {
                if let Some(v_list) = versions.as_array() {
                    for v_obj in v_list {
                        let version = v_obj.get("version")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        
                        let mut p = Package::with_version(name, version, "mise");
                        
                        // Extract installation source (global vs local)
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
        // Filter the installed list for tools that originate from the global config
        let all = self.list_installed().await?;
        Ok(all.into_iter()
            .filter(|p| p.properties.get("source_type").map(|s| s == "global").unwrap_or(false))
            .collect())
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        // Query Mise for the plugin's source repository
        let output = self.executor.run_output("mise", &["plugins", "ls", "--all", "--urls"], false).await?;
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

#[async_trait]
impl Upgradable for MiseManager {
    async fn update(&self, _: bool) -> Result<()> {
        info!("Mise: Updating plugin repository metadata...");
        self.executor.run("mise", &["plugins", "update"], false).await?;
        Ok(())
    }

    async fn upgrade(&self, _: bool) -> Result<()> {
        info!("Mise: Upgrading all globally installed tools...");
        // Upgrade all tools managed in the global configuration
        self.executor.run_exclusive("mise", "mise", &["upgrade"], false).await?;
        Ok(())
    }

    async fn clean_orphans(&self, _: bool) -> Result<()> {
        info!("Mise: Pruning unused tool versions from cache...");
        // Prunes old versions that are no longer referenced in any config
        self.executor.run("mise", &["prune", "--force"], false).await?;
        Ok(())
    }
}