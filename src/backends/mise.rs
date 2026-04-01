use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct MiseManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl MiseManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
}

#[async_trait]
impl PackageManager for MiseManager {
    fn name(&self) -> &str { "mise" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("mise").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for pkg in p {
            // REAL LOGIC: 'use -g' ensures the tool is installed and set in the global config
            let spec = if pkg.contains('@') { pkg.clone() } else { format!("{}@latest", pkg) };
            self.executor.run("mise", &["use", "-g", &spec], false).await?;
        }
        Ok(())
    }

    async fn remove(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for pkg in p {
            // Uninstalls the tool and cleans up the shim
            self.executor.run("mise", &["uninstall", pkg], false).await?;
        }
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse the JSON output of mise list
        let out = self.executor.run_output("mise", &["ls", "--json"], false).await?;
        let json: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
        let mut pkgs = vec![];

        if let Some(map) = json.as_object() {
            for (name, versions) in map {
                if let Some(v_arr) = versions.as_array() {
                    for v in v_arr {
                        let ver_str = v.get("version").and_then(|s| s.as_str()).unwrap_or("unknown");
                        pkgs.push(Package {
                            name: name.clone(),
                            version: Some(ver_str.to_string()),
                            backend: "mise".into(),
                            ..Package::new("", "")
                        });
                    }
                }
            }
        }
        Ok(pkgs)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // NUCLEAR DELETE FIX: Only returns tools defined in the GLOBAL config.
        // This prevents LiNix from deleting local project shims or auto-installed plugins.
        let out = self.executor.run_output("mise", &["ls", "--json"], false).await?;
        let json: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
        let mut manual = vec![];

        if let Some(map) = json.as_object() {
            for (name, versions) in map {
                if let Some(v_arr) = versions.as_array() {
                    // Filter for versions where the 'source' indicates the global mise.toml
                    let is_global = v_arr.iter().any(|v| {
                        v.get("source").and_then(|s| s.get("type")).map(|t| t == "global").unwrap_or(false)
                    });
                    if is_global {
                        manual.push(Package::new(name, "mise"));
                    }
                }
            }
        }
        Ok(manual)
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: Searches the registry of all available mise/asdf plugins
        let out = self.executor.run_output("mise", &["plugins", "ls", "--all"], false).await?;
        Ok(out.lines()
            .filter(|l| l.contains(query))
            .map(|l| Package::new(l.trim(), "mise"))
            .collect())
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Fetches the plugin's source repository URL
        let out = self.executor.run_output("mise", &["plugins", "ls", "--all", "--urls"], false).await?;
        for line in out.lines() {
            if let Some((name, url)) = line.split_once(' ') {
                if name.trim() == package {
                    let mut p = Package::new(package, "mise");
                    p.repository = Some(url.trim().to_string());
                    return Ok(Some(p));
                }
            }
        }
        Ok(None)
    }

    async fn update(&self, _: bool) -> Result<()> {
        // Refreshes the mise core and plugin metadata
        self.executor.run("mise", &["plugins", "update"], false).await?;
        Ok(())
    }

    async fn upgrade(&self, _: bool) -> Result<()> {
        // Upgrades all globally installed tools to their latest versions
        self.executor.run("mise", &["upgrade"], false).await?;
        Ok(())
    }
}