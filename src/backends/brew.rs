use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct BrewManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    #[allow(dead_code)] settings: Option<HashMap<String, String>>,
}

impl BrewManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), settings }
    }
}

#[async_trait]
impl PackageManager for BrewManager {
    fn name(&self) -> &str { "brew" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("brew").arg("--version").output().is_ok())
    }

    async fn install(&self, p: &[String], _sudo: bool) -> Result<()> {
        let mut args = vec!["install"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run("brew", &args, false).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], _sudo: bool) -> Result<()> {
        let mut args = vec!["uninstall"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run("brew", &args, false).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("brew", &["list", "--versions"], false).await?;
        Ok(out.lines().filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            if p.len() >= 2 { Some(Package { name: p[0].into(), version: Some(p[1].into()), backend: "brew".into(), ..Package::new("", "") }) }
            else { None }
        }).collect())
    }
}