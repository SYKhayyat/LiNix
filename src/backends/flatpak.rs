use crate::core::{CommandExecutor, Package, PackageManager, PackageSpec, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct FlatpakManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    settings: Option<HashMap<String, String>>,
}

impl FlatpakManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), settings }
    }
    fn is_user(&self) -> bool {
        self.settings.as_ref().and_then(|s| s.get("user_installation")).map(|v| v == "true").unwrap_or(false)
    }
}

#[async_trait]
impl PackageManager for FlatpakManager {
    fn name(&self) -> &str { "flatpak" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("flatpak").arg("--version").output().is_ok())
    }

    async fn install_with_options(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        let mut args = if self.is_user() { vec!["--user"] } else { vec!["--system"] };
        args.extend(["install", "-y"]);
        let names: Vec<_> = specs.iter().map(|s| s.name.as_str()).collect();
        args.extend(names);
        self.executor.run("flatpak", &args, !self.is_user() && sudo).await?;
        Ok(())
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        let specs: Vec<_> = p.iter().map(|n| PackageSpec { name: n.clone(), backend: "flatpak".into(), options: HashMap::new() }).collect();
        self.install_with_options(&specs, s).await
    }

    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        let mut args = vec!["uninstall", "-y"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run("flatpak", &args, !self.is_user() && s).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("flatpak", &["list", "--app", "--columns=application,version"], false).await?;
        Ok(out.lines().filter_map(|l| {
            let p: Vec<_> = l.split('\t').collect();
            if p.len() >= 2 { Some(Package { name: p[0].into(), version: Some(p[1].into()), backend: "flatpak".into(), ..Package::new("", "") }) }
            else { None }
        }).collect())
    }
}