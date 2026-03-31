use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct ChocoManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    _settings: Option<HashMap<String, String>>,
}

impl ChocoManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), _settings: settings }
    }
}

#[async_trait]
impl PackageManager for ChocoManager {
    fn name(&self) -> &str { "choco" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("where").arg("choco").output().is_ok())
    }

    async fn install(&self, p: &[String], _s: bool) -> Result<()> {
        let mut args = vec!["install", "-y"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run("choco", &args, false).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], _s: bool) -> Result<()> {
        let mut args = vec!["uninstall", "-y"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run("choco", &args, false).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("choco", &["list", "-lo", "-r"], false).await?;
        Ok(out.lines().filter_map(|line| {
            let (name, ver) = line.split_once("|")?;
            let mut p = Package::new(name, "choco");
            p.version = Some(ver.to_string());
            Some(p)
        }).collect())
    }
}
