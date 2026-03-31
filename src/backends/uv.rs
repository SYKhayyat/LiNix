use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct UvManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    _settings: Option<HashMap<String, String>>,
}

impl UvManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), _settings: settings }
    }
}

#[async_trait]
impl PackageManager for UvManager {
    fn name(&self) -> &str { "uv" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("uv").arg("--version").output().is_ok())
    }
    async fn install(&self, p: &[String], _s: bool) -> Result<()> {
        for pkg in p { self.executor.run("uv", &["tool", "install", pkg], false).await?; }
        Ok(())
    }
    async fn remove(&self, p: &[String], _s: bool) -> Result<()> {
        for pkg in p { self.executor.run("uv", &["tool", "uninstall", pkg], false).await?; }
        Ok(())
    }
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("uv", &["tool", "list", "--json"], false).await?;
        let json: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
        Ok(json.as_array().unwrap_or(&vec![]).iter().filter_map(|v| {
            let name = v.get("name")?.as_str()?;
            let ver = v.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
            Some(Package { name: name.into(), version: ver, backend: "uv".into(), ..Package::new("", "") })
        }).collect())
    }
}
