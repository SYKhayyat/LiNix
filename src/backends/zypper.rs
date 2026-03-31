use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct ZypperManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    _settings: Option<HashMap<String, String>>,
}

impl ZypperManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), _settings: settings }
    }
}

#[async_trait]
impl PackageManager for ZypperManager {
    fn name(&self) -> &str { "zypper" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("zypper").arg("--version").output().is_ok())
    }
    async fn install(&self, p: &[String], sudo: bool) -> Result<()> {
        let mut args = vec!["install", "-y", "--no-confirm"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run("zypper", &args, sudo).await?;
        Ok(())
    }
    async fn remove(&self, p: &[String], sudo: bool) -> Result<()> {
        let mut args = vec!["remove", "-y"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run("zypper", &args, sudo).await?;
        Ok(())
    }
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("zypper", &["search", "--installed-only"], false).await?;
        Ok(out.lines().filter_map(|l| {
            let parts: Vec<&str> = l.split("|").collect();
            if parts.len() > 1 && parts[0].contains("i") {
                Some(Package::new(parts[1].trim(), "zypper"))
            } else { None }
        }).collect())
    }
}
