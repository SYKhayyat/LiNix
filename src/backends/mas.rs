use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct MasManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl MasManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
}

#[async_trait]
impl PackageManager for MasManager {
    fn name(&self) -> &str { "mas" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            // mas is only relevant on macOS
            if cfg!(target_os = "macos") {
                std::process::Command::new("mas").arg("version").output().is_ok()
            } else {
                false
            }
        })
    }

    async fn install(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for id in p {
            // mas uses numeric IDs for installation
            self.executor.run("mas", &["install", id], false).await?;
        }
        Ok(())
    }

    async fn remove(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for id in p {
            self.executor.run("mas", &["uninstall", id], false).await?;
        }
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse 'mas list'
        // Format: "123456789 App Name (1.2.3)"
        let out = self.executor.run_output("mas", &["list"], false).await?;
        Ok(out.lines().filter_map(|l| {
            let (id_name, ver_part) = l.rsplit_once(' ')?;
            let (id, name) = id_name.split_once(' ')?;
            Some(Package {
                name: id.trim().to_string(), // We use the ID as the unique name
                version: Some(ver_part.trim_matches(|c| c == '(' || c == ')').to_string()),
                description: Some(name.trim().to_string()), // Put human name in description
                backend: "mas".into(),
                ..Package::new("", "")
            })
        }).collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: Parse 'mas search'
        // Format: "123456789  App Name  (1.2.3)"
        let out = self.executor.run_output("mas", &["search", query], false).await?;
        Ok(out.lines().filter_map(|l| {
            let parts: Vec<&str> = l.split_whitespace().collect();
            if parts.len() < 2 { return None; }
            let id = parts[0];
            let mut p = Package::new(id, "mas");
            // The rest is the name; search doesn't provide easy versioning
            p.description = Some(parts[1..].join(" "));
            Some(p)
        }).collect())
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Parse 'mas info' for price, seller, and version
        let out = self.executor.run_output("mas", &["info", package], false).await?;
        if out.is_empty() { return Ok(None); }

        let mut p = Package::new(package, "mas");
        let mut desc = String::new();
        for line in out.lines() {
            if line.contains("Price:") || line.contains("Seller:") {
                desc.push_str(line.trim());
                desc.push_str(". ");
            }
            if let Some(v) = line.strip_prefix("Version:") { p.version = Some(v.trim().to_string()); }
        }
        p.description = Some(desc);
        Ok(Some(p))
    }

    async fn upgrade(&self, _: bool) -> Result<()> {
        // Upgrades all App Store apps
        self.executor.run("mas", &["upgrade"], false).await?;
        Ok(())
    }
}