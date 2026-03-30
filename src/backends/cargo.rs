use crate::core::{CommandExecutor, Package, PackageManager, PackageSpec, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct CargoManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    settings: Option<HashMap<String, String>>,
}

impl CargoManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), settings }
    }
}

#[async_trait]
impl PackageManager for CargoManager {
    fn name(&self) -> &str { "cargo" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("cargo").arg("--version").output().is_ok())
    }

    async fn install_with_options(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        let use_b = self.settings.as_ref().and_then(|s| s.get("binstall")).map(|v| v == "true").unwrap_or(false);
        let cmd = if use_b && self.executor.command_exists("cargo-binstall").await { "cargo-binstall" } else { "cargo" };

        for s in specs {
            let mut args = if cmd == "cargo-binstall" { vec!["-y".to_string()] } else { vec!["install".to_string()] };
            if let Some(f) = s.options.get("features") { args.extend(["--features".into(), f.clone()]); }
            args.push(s.name.clone());
            let refs: Vec<&str> = args.iter().map(|a| a.as_str()).collect();
            self.executor.run(cmd, &refs, sudo).await?;
        }
        Ok(())
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        let specs: Vec<_> = p.iter().map(|n| PackageSpec { name: n.clone(), backend: "cargo".into(), options: HashMap::new() }).collect();
        self.install_with_options(&specs, s).await
    }

    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        let mut args = vec!["uninstall"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run("cargo", &args, s).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("cargo", &["install", "--list"], false).await?;
        let mut res = vec![];
        for l in out.lines() {
            if !l.starts_with(' ') && l.contains('v') && l.contains(':') {
                let p: Vec<_> = l.split_whitespace().collect();
                if p.len() >= 2 { res.push(Package { name: p[0].into(), version: Some(p[1].trim_end_matches(':').into()), backend: "cargo".into(), ..Package::new("", "") }); }
            }
        }
        Ok(res)
    }
}