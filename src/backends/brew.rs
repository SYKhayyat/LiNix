use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct BrewManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl BrewManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
}

#[async_trait]
impl PackageManager for BrewManager {
    fn name(&self) -> &str { "brew" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("brew")
                .arg("--version")
                .output()
                .is_ok()
        })
    }

    async fn install(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let mut args = vec!["install"];
        args.extend(p.iter().map(|s| s.as_str()));
        // Brew doesn't use sudo for its own commands
        self.executor.run("brew", &args, false).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let mut args = vec!["uninstall"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run("brew", &args, false).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("brew", &["list", "--versions"], false).await?;
        Ok(out.lines().filter_map(|l| {
            let (name, ver) = l.split_once(' ')?;
            Some(Package { 
                name: name.into(), 
                version: Some(ver.into()), 
                backend: "brew".into(), 
                ..Package::new("", "") 
            })
        }).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // 'brew leaves' lists packages that are not dependencies of other packages
        let out = self.executor.run_output("brew", &["leaves"], false).await?;
        Ok(out.lines().map(|l| Package::new(l.trim(), "brew")).collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let out = self.executor.run_output("brew", &["search", query], false).await?;
        Ok(out.lines()
            .filter(|l| !l.is_empty() && !l.starts_with("=="))
            .map(|l| Package::new(l.trim(), "brew"))
            .collect())
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // Query JSON to get description and homepage accurately
        let out = self.executor.run_output("brew", &["info", "--json", package], false).await?;
        let json: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
        
        if let Some(pkg_data) = json.as_array().and_then(|a| a.first()) {
            return Ok(Some(Package {
                name: package.to_string(),
                version: pkg_data["versions"]["stable"].as_str().map(|s| s.to_string()),
                description: pkg_data["desc"].as_str().map(|s| s.to_string()),
                repository: pkg_data["homepage"].as_str().map(|s| s.to_string()),
                backend: "brew".into(),
                ..Package::new("", "")
            }));
        }
        Ok(None)
    }

    /// FIX: Replaced stub with actual 'tap' logic
    async fn add_repo(&self, _: &str, url: &str, _: bool) -> Result<()> {
        self.executor.run("brew", &["tap", url], false).await?;
        Ok(())
    }

    async fn remove_repo(&self, name: &str, _: bool) -> Result<()> {
        self.executor.run("brew", &["untap", name], false).await?;
        Ok(())
    }

    async fn list_repos(&self) -> Result<Vec<(String, String)>> {
        let out = self.executor.run_output("brew", &["tap"], false).await?;
        Ok(out.lines().map(|l| (l.to_string(), "tap".to_string())).collect())
    }

    async fn update(&self, _: bool) -> Result<()> {
        self.executor.run("brew", &["update"], false).await?;
        Ok(())
    }

    async fn upgrade(&self, _: bool) -> Result<()> {
        self.executor.run("brew", &["upgrade"], false).await?;
        Ok(())
    }

    /// FIX: Enabled orphan cleanup
    fn supports_orphan_cleanup(&self) -> bool { true }
    async fn clean_orphans(&self, _: bool) -> Result<()> {
        // brew autoremove removes unused dependencies
        self.executor.run("brew", &["autoremove"], false).await?;
        Ok(())
    }
}