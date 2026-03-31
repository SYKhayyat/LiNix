use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct YarnManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    _settings: Option<HashMap<String, String>>,
}

impl YarnManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), _settings: settings }
    }
}

#[async_trait]
impl PackageManager for YarnManager {
    fn name(&self) -> &str { "yarn" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("yarn").arg("--version").output().is_ok())
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        let mut args = vec!["global", "add"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run("yarn", &args, s).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        let mut args = vec!["global", "remove"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run("yarn", &args, s).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("yarn", &["global", "list", "--depth=0"], false).await?;
        Ok(out.lines().filter_map(|line| {
            if !line.contains("@") { return None; }
            let cleaned = line.trim_start_matches("├── ").trim_start_matches("└── ");
            let (name, ver) = cleaned.rsplit_once("@")?;
            Some(Package { name: name.to_string(), version: Some(ver.to_string()), backend: "yarn".into(), ..Package::new("", "") })
        }).collect())
    }
}
