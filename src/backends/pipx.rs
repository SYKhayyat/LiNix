use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct PipxManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    _settings: Option<HashMap<String, String>>,
}

impl PipxManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), _settings: settings }
    }
}

#[async_trait]
impl PackageManager for PipxManager {
    fn name(&self) -> &str { "pipx" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("pipx").arg("--version").output().is_ok())
    }
    async fn install(&self, p: &[String], _s: bool) -> Result<()> {
        for pkg in p { self.executor.run("pipx", &["install", pkg], false).await?; }
        Ok(())
    }
    async fn remove(&self, p: &[String], _s: bool) -> Result<()> {
        for pkg in p { self.executor.run("pipx", &["uninstall", pkg], false).await?; }
        Ok(())
    }
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("pipx", &["list", "--json"], false).await?;
        let json: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
        let mut pkgs = vec![];
        if let Some(venvs) = json.get("venvs").and_then(|v| v.as_object()) {
            for name in venvs.keys() { pkgs.push(Package::new(name, "pipx")); }
        }
        Ok(pkgs)
    }
}
