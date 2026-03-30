use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct PacmanManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    #[allow(dead_code)] settings: Option<HashMap<String, String>>,
}

impl PacmanManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), settings }
    }
}

#[async_trait]
impl PackageManager for PacmanManager {
    fn name(&self) -> &str { "pacman" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("pacman").arg("--version").output().is_ok())
    }

    async fn install(&self, p: &[String], sudo: bool) -> Result<()> {
        let mut args = vec!["-S", "--noconfirm", "--needed"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run("pacman", &args, sudo).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], sudo: bool) -> Result<()> {
        let mut args = vec!["-Rs", "--noconfirm"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run("pacman", &args, sudo).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("pacman", &["-Q"], false).await?;
        Ok(out.lines().filter_map(|line| {
            let p: Vec<&str> = line.split_whitespace().collect();
            if p.len() >= 2 {
                Some(Package { name: p[0].to_string(), version: Some(p[1].to_string()), backend: "pacman".into(), ..Package::new("", "") })
            } else { None }
        }).collect())
    }

    async fn update(&self, sudo: bool) -> Result<()> {
        self.executor.run("pacman", &["-Sy"], sudo).await?;
        Ok(())
    }

    fn supports_orphan_cleanup(&self) -> bool { true }

    async fn clean_orphans(&self, sudo: bool) -> Result<()> {
        let orphans = self.executor.run_output("pacman", &["-Qdtq"], false).await?;
        let list: Vec<&str> = orphans.lines().filter(|l| !l.is_empty()).collect();
        if !list.is_empty() {
            let mut args = vec!["-Rs", "--noconfirm"];
            args.extend(list);
            self.executor.run("pacman", &args, sudo).await?;
        }
        Ok(())
    }
}