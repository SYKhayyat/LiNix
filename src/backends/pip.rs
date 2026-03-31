use crate::core::{CommandExecutor, Package, PackageManager, Result, Error};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct PipManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
    settings: Option<HashMap<String, String>>,
}

impl PipManager {
    pub fn new(executor: CommandExecutor, settings: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new(), settings }
    }
    async fn get_cmd(&self) -> &str {
        if self.executor.command_exists("pip3").await { "pip3" } else { "pip" }
    }
}

#[async_trait]
impl PackageManager for PipManager {
    fn name(&self) -> &str { "pip" }
    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| std::process::Command::new("pip").arg("--version").output().is_ok())
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let cmd = self.get_cmd().await;
        let mut args = vec!["install".to_string()];
        if let Some(set) = &self.settings {
            if let Some(url) = set.get("index_url") { args.extend(["--index-url".into(), url.clone()]); }
        }
        args.extend(p.iter().cloned());
        let refs: Vec<&str> = args.iter().map(|x| x.as_str()).collect();
        self.executor.run(cmd, &refs, s).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        let cmd = self.get_cmd().await;
        let mut args = vec!["uninstall", "-y"];
        args.extend(p.iter().map(|x| x.as_str()));
        self.executor.run(cmd, &args, s).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let cmd = self.get_cmd().await;
        let out = self.executor.run_output(cmd, &["list", "--format=json"], false).await?;
        let json: serde_json::Value = serde_json::from_str(&out).map_err(|e| Error::Other(e.to_string()))?;
        Ok(json.as_array().unwrap_or(&vec![]).iter().filter_map(|p| {
            let name = p.get("name")?.as_str()?.to_string();
            let version = p.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
            Some(Package { name, version, backend: "pip".to_string(), ..Package::new("", "") })
        }).collect())
    }
}
