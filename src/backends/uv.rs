use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct UvManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    #[allow(dead_code)] settings: Option<HashMap<String, String>>,
}

impl UvManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), settings }
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
        let mut res = vec![];
        if let Some(arr) = json.as_array() {
            for v in arr {
                if let Some(n) = v.get("name").and_then(|n| n.as_str()) {
                    let ver = v.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
                    res.push(Package { name: n.into(), version: ver, backend: "uv".into(), ..Package::new("", "") });
                }
            }
        }
        Ok(res)
    }
}