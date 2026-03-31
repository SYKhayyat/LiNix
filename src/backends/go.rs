use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct GoManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    _settings: Option<HashMap<String, String>>,
}

impl GoManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), _settings: settings }
    }
    fn get_gobin() -> PathBuf {
        std::env::var("GOBIN").map(PathBuf::from).unwrap_or_else(|_| {
            dirs::home_dir().unwrap_or_default().join("go").join("bin")
        })
    }
}

#[async_trait]
impl PackageManager for GoManager {
    fn name(&self) -> &str { "go" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("go").arg("version").output().is_ok())
    }
    async fn install(&self, p: &[String], sudo: bool) -> Result<()> {
        for pkg in p {
            let path = if pkg.contains("@") { pkg.clone() } else { format!("{}@latest", pkg) };
            self.executor.run("go", &["install", &path], sudo).await?;
        }
        Ok(())
    }
    async fn remove(&self, p: &[String], _s: bool) -> Result<()> {
        let bin = Self::get_gobin();
        for pkg in p {
            let name = pkg.split("/").last().unwrap_or(pkg);
            let _ = tokio::fs::remove_file(bin.join(name)).await;
        }
        Ok(())
    }
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let bin = Self::get_gobin();
        if !bin.exists() { return Ok(vec![]); }
        let mut pkgs = vec![];
        let mut entries = tokio::fs::read_dir(bin).await?;
        while let Some(entry) = entries.next_entry().await? {
            pkgs.push(Package::new(entry.file_name().to_string_lossy(), "go"));
        }
        Ok(pkgs)
    }
}
