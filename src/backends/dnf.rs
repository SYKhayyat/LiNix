use crate::core::{CommandExecutor, Package, PackageManager, PackageSpec, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct DnfManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    settings: Option<HashMap<String, String>>,
}

impl DnfManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), settings }
    }
}

#[async_trait]
impl PackageManager for DnfManager {
    fn name(&self) -> &str { "dnf" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("dnf").arg("--version").output().is_ok())
    }
    async fn install_with_options(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        let mut args = vec!["install".to_string(), "-y".to_string()];
        if let Some(s) = &self.settings {
            if s.get("nogpgcheck") == Some(&"true".to_string()) { args.push("--nogpgcheck".to_string()); }
        }
        args.extend(specs.iter().map(|s| s.name.clone()));
        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.executor.run("dnf", &refs, sudo).await?;
        Ok(())
    }
    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        let specs: Vec<_> = p.iter().map(|n| PackageSpec { name: n.clone(), backend: "dnf".into(), options: HashMap::new() }).collect();
        self.install_with_options(&specs, s).await
    }
    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        let mut args = vec!["remove", "-y"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run("dnf", &args, s).await?;
        Ok(())
    }
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("dnf", &["list", "installed"], false).await?;
        Ok(out.lines().skip(1).filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let (name, _) = parts[0].split_once(".")?;
                Some(Package { name: name.to_string(), version: Some(parts[1].to_string()), backend: "dnf".into(), ..Package::new("", "") })
            } else { None }
        }).collect())
    }
}
