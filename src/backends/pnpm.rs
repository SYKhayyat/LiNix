use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct PnpmManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    _settings: Option<HashMap<String, String>>,
}

impl PnpmManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), _settings: settings }
    }
}

#[async_trait]
impl PackageManager for PnpmManager {
    fn name(&self) -> &str { "pnpm" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("pnpm").arg("--version").output().is_ok())
    }
    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        let mut args = vec!["add", "-g"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run("pnpm", &args, s).await?;
        Ok(())
    }
    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        let mut args = vec!["remove", "-g"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run("pnpm", &args, s).await?;
        Ok(())
    }
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("pnpm", &["list", "-g", "--json"], false).await?;
        let json: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
        let mut pkgs = vec![];
        if let Some(deps) = json.get("dependencies").and_then(|d| d.as_object()) {
            for (name, data) in deps {
                let ver = data.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
                pkgs.push(Package { name: name.clone(), version: ver, backend: "pnpm".into(), ..Package::new("", "") });
            }
        }
        Ok(pkgs)
    }
}
