use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct ApkManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    _settings: Option<HashMap<String, String>>,
}

impl ApkManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), _settings: settings }
    }
}

#[async_trait]
impl PackageManager for ApkManager {
    fn name(&self) -> &str { "apk" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("apk").arg("--version").output().is_ok())
    }
    async fn install(&self, p: &[String], sudo: bool) -> Result<()> {
        let mut args = vec!["add"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run("apk", &args, sudo).await?;
        Ok(())
    }
    async fn remove(&self, p: &[String], sudo: bool) -> Result<()> {
        let mut args = vec!["del"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run("apk", &args, sudo).await?;
        Ok(())
    }
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("apk", &["info", "-v"], false).await?;
        Ok(out.lines().filter_map(|line| {
            let (name, _) = line.split_once("-")?;
            Some(Package::new(name, "apk"))
        }).collect())
    }
}
