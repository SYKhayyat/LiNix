use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct VscodeManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    #[allow(dead_code)] settings: Option<HashMap<String, String>>,
}

impl VscodeManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), settings }
    }
}

#[async_trait]
impl PackageManager for VscodeManager {
    fn name(&self) -> &str { "vscode" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("code").arg("--version").output().is_ok())
    }

    async fn install(&self, p: &[String], _s: bool) -> Result<()> {
        for pkg in p { self.executor.run("code", &["--install-extension", pkg], false).await?; }
        Ok(())
    }

    async fn remove(&self, p: &[String], _s: bool) -> Result<()> {
        for pkg in p { self.executor.run("code", &["--uninstall-extension", pkg], false).await?; }
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("code", &["--list-extensions", "--show-versions"], false).await?;
        Ok(out.lines().filter_map(|l| {
            let p: Vec<&str> = l.split('@').collect();
            if !p[0].is_empty() {
                Some(Package { name: p[0].into(), version: p.get(1).map(|v| v.to_string()), backend: "vscode".into(), ..Package::new("", "") })
            } else { None }
        }).collect())
    }
}