use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct ApkManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl ApkManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
}

#[async_trait]
impl PackageManager for ApkManager {
    fn name(&self) -> &str { "apk" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("apk")
                .arg("--version")
                .output()
                .is_ok()
        })
    }

    async fn install(&self, p: &[String], sudo: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        // --no-cache avoids using stale local indexes during automation
        let mut args = vec!["add", "--no-cache"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run_exclusive("apk", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], sudo: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let mut args = vec!["del"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run_exclusive("apk", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("apk", &["info", "-v"], false).await?;
        Ok(out.lines().filter_map(|line| {
            let parts: Vec<&str> = line.rsplitn(3, '-').collect();
            if parts.len() >= 3 {
                let name = parts[2].to_string();
                let version = format!("{}-{}", parts[1], parts[0]);
                Some(Package { name, version: Some(version), backend: "apk".into(), ..Package::new("", "") })
            } else { Some(Package::new(line, "apk")) }
        }).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Read the 'world' file which contains ONLY user-requested packages
        let world_content = tokio::fs::read_to_string("/etc/apk/world").await.unwrap_or_default();
        let world_names: std::collections::HashSet<&str> = world_content.lines().map(|l| l.trim()).collect();
        let installed = self.list_installed().await?;
        Ok(installed.into_iter().filter(|p| world_names.contains(p.name.as_str())).collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let out = self.executor.run_output("apk", &["search", "-q", query], false).await?;
        Ok(out.lines().filter(|l| !l.is_empty()).map(|line| Package::new(line.trim(), "apk")).collect())
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        let desc_out = self.executor.run_output("apk", &["info", "-d", package], false).await?;
        let ver_out = self.executor.run_output("apk", &["info", "-v", package], false).await?;
        
        if desc_out.is_empty() { return Ok(None); }
        
        let mut pkg = Package::new(package, "apk");
        pkg.version = ver_out.lines().next().map(|s| s.to_string());
        pkg.description = Some(desc_out.replace(&format!("{} description:", package), "").trim().to_string());
        Ok(Some(pkg))
    }

    async fn update(&self, sudo: bool) -> Result<()> {
        self.executor.run_exclusive("apk", &["update"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        self.executor.run_exclusive("apk", &["upgrade"], sudo).await?;
        Ok(())
    }
}