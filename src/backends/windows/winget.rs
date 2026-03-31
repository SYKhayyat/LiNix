use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct WingetManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    _settings: Option<HashMap<String, String>>,
}

impl WingetManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), _settings: settings }
    }
}

#[async_trait]
impl PackageManager for WingetManager {
    fn name(&self) -> &str { "winget" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("winget").arg("--version").output().is_ok())
    }
    async fn install(&self, p: &[String], _s: bool) -> Result<()> {
        for pkg in p { 
            self.executor.run("winget", &["install", "--silent", "--accept-source-agreements", pkg], false).await?; 
        }
        Ok(())
    }
    async fn remove(&self, p: &[String], _s: bool) -> Result<()> {
        for pkg in p { self.executor.run("winget", &["uninstall", "--silent", pkg], false).await?; }
        Ok(())
    }
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("winget", &["list"], false).await?;
        Ok(out.lines().skip(2).filter_map(|l| {
            let parts: Vec<&str> = l.split_whitespace().collect();
            if parts.len() >= 2 { Some(Package::new(parts[0], "winget")) } else { None }
        }).collect())
    }
}
