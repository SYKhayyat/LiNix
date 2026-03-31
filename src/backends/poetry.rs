use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct PoetryManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    _settings: Option<HashMap<String, String>>,
}

impl PoetryManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), _settings: settings }
    }
}

#[async_trait]
impl PackageManager for PoetryManager {
    fn name(&self) -> &str { "poetry" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("poetry").arg("--version").output().is_ok())
    }
    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        for pkg in p { self.executor.run("poetry", &["add", pkg], s).await?; }
        Ok(())
    }
    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        for pkg in p { self.executor.run("poetry", &["remove", pkg], s).await?; }
        Ok(())
    }
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("poetry", &["show"], false).await?;
        Ok(out.lines().filter_map(|l| {
            let (name, _) = l.split_once(" ")?;
            Some(Package::new(name.trim(), "poetry"))
        }).collect())
    }
}
