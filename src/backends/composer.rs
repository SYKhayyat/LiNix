use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct ComposerManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    _settings: Option<HashMap<String, String>>,
}

impl ComposerManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), _settings: settings }
    }
}

#[async_trait]
impl PackageManager for ComposerManager {
    fn name(&self) -> &str { "composer" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("composer").arg("--version").output().is_ok())
    }
    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        let mut args = vec!["global", "require"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run("composer", &args, s).await?;
        Ok(())
    }
    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        let mut args = vec!["global", "remove"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run("composer", &args, s).await?;
        Ok(())
    }
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("composer", &["global", "show", "--format=json"], false).await?;
        let json: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
        let mut pkgs = vec![];
        if let Some(installed) = json.get("installed").and_then(|i| i.as_array()) {
            for pkg in installed {
                if let Some(name) = pkg.get("name").and_then(|n| n.as_str()) {
                    pkgs.push(Package::new(name, "composer"));
                }
            }
        }
        Ok(pkgs)
    }
}
