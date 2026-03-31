use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct MiseManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    _settings: Option<HashMap<String, String>>,
}

impl MiseManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), _settings: settings }
    }
}

#[async_trait]
impl PackageManager for MiseManager {
    fn name(&self) -> &str { "mise" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("mise").arg("--version").output().is_ok())
    }
    async fn install(&self, p: &[String], _s: bool) -> Result<()> {
        for pkg in p { self.executor.run("mise", &["tool", "install", pkg], false).await?; }
        Ok(())
    }
    async fn remove(&self, p: &[String], _s: bool) -> Result<()> {
        for pkg in p { self.executor.run("mise", &["tool", "uninstall", pkg], false).await?; }
        Ok(())
    }
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("mise", &["tool", "list", "--json"], false).await?;
        let json: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
        let mut pkgs = vec![];
        if let Some(arr) = json.as_array() {
            for v in arr {
                if let Some(n) = v.get("name").and_then(|n| n.as_str()) {
                    pkgs.push(Package::new(n, "mise"));
                }
            }
        }
        Ok(pkgs)
    }
}
