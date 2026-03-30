use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct SnapManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    #[allow(dead_code)] settings: Option<HashMap<String, String>>,
}

impl SnapManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), settings }
    }
}

#[async_trait]
impl PackageManager for SnapManager {
    fn name(&self) -> &str { "snap" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("snap").arg("version").output().is_ok())
    }

    async fn install(&self, p: &[String], sudo: bool) -> Result<()> {
        for pkg in p { self.executor.run("snap", &["install", pkg], sudo).await?; }
        Ok(())
    }

    async fn remove(&self, p: &[String], sudo: bool) -> Result<()> {
        for pkg in p { self.executor.run("snap", &["remove", pkg], sudo).await?; }
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("snap", &["list"], false).await?;
        Ok(out.lines().skip(1).filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            if p.len() >= 2 { Some(Package { name: p[0].into(), version: Some(p[1].into()), backend: "snap".into(), ..Package::new("", "") }) }
            else { None }
        }).collect())
    }
}