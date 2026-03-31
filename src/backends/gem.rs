use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct GemManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    _settings: Option<HashMap<String, String>>,
}

impl GemManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), _settings: settings }
    }
}

#[async_trait]
impl PackageManager for GemManager {
    fn name(&self) -> &str { "gem" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("gem").arg("--version").output().is_ok())
    }
    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        let mut args = vec!["install", "--no-document"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run("gem", &args, s).await?;
        Ok(())
    }
    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        let mut args = vec!["uninstall", "-x"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run("gem", &args, s).await?;
        Ok(())
    }
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("gem", &["list", "--local"], false).await?;
        Ok(out.lines().filter_map(|l| {
            if l.starts_with("***") || l.is_empty() { return None; }
            let (name, _) = l.split_once(" ")?;
            Some(Package::new(name.trim(), "gem"))
        }).collect())
    }
}
