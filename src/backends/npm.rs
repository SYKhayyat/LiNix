use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct NpmManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    settings: Option<HashMap<String, String>>,
}

impl NpmManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), settings }
    }
}

#[async_trait]
impl PackageManager for NpmManager {
    fn name(&self) -> &str { "npm" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("npm").arg("--version").output().is_ok())
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        let mut args = vec!["install", "-g"];
        if let Some(reg) = self.settings.as_ref().and_then(|s| s.get("registry")) { args.extend(["--registry", reg]); }
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run("npm", &args, s).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        let mut args = vec!["uninstall", "-g"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run("npm", &args, s).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("npm", &["list", "-g", "--depth=0", "--json"], false).await?;
        let json: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
        let mut res = vec![];
        if let Some(deps) = json.get("dependencies").and_then(|d| d.as_object()) {
            for (name, val) in deps {
                let version = val.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
                res.push(Package { name: name.clone(), version, backend: "npm".into(), ..Package::new("", "") });
            }
        }
        Ok(res)
    }
}