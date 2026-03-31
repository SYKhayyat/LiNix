use crate::core::{CommandExecutor, Package, PackageManager, PackageSpec, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct AptManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    settings: Option<HashMap<String, String>>,
}

impl AptManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), settings }
    }
}

#[async_trait]
impl PackageManager for AptManager {
    fn name(&self) -> &str { "apt" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("apt").arg("--version").output().is_ok())
    }

    async fn install_with_options(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        let mut args = vec!["install".to_string(), "-y".to_string()];
        
        if let Some(s) = &self.settings {
            if s.get("no_install_recommends") == Some(&"true".to_string()) {
                args.push("--no-install-recommends".to_string());
            }
        }

        for s in specs {
            if let Some(v) = s.options.get("version") { args.push(format!("{}={}", s.name, v)); }
            else { args.push(s.name.clone()); }
        }
        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.executor.run("apt", &refs, sudo).await?;
        Ok(())
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        let specs: Vec<_> = p.iter().map(|n| PackageSpec { name: n.clone(), backend: "apt".into(), options: HashMap::new() }).collect();
        self.install_with_options(&specs, s).await
    }

    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        let mut args = vec!["remove", "-y"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run("apt", &args, s).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("apt", &["list", "--installed"], false).await?;
        Ok(out.lines().skip(1).filter_map(|l| {
            let (name_part, rest) = l.split_once('/')?;
            let version = rest.split_whitespace().next().map(|s| s.to_string());
            Some(Package { name: name_part.to_string(), version, backend: "apt".into(), description: None, repository: None, size: None })
        }).collect())
    }
}