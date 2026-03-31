use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct BunManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    _settings: Option<HashMap<String, String>>,
}

impl BunManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), _settings: settings }
    }
}

#[async_trait]
impl PackageManager for BunManager {
    fn name(&self) -> &str { "bun" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("bun")
                .arg("--version")
                .output()
                .is_ok()
        })
    }
    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        let mut args = vec!["add", "-g"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run("bun", &args, s).await?;
        Ok(())
    }
    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        let mut args = vec!["remove", "-g"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run("bun", &args, s).await?;
        Ok(())
    }
    async fn list_installed(&self) -> Result<Vec<Package>> {
        // Bun global list is currently not standardized in the CLI
        Ok(vec![])
    }
}