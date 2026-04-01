use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;

pub struct NixManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl NixManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }
}

#[async_trait]
impl PackageManager for NixManager {
    fn name(&self) -> &str { "nix" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("nix-env").arg("--version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        // -iA uses attribute paths which are faster and more precise
        let mut args = vec!["-iA"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run("nix-env", &args, false).await?;
        Ok(())
    }

    async fn remove(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let mut args = vec!["-e"];
        args.extend(p.iter().map(|s| s.as_str()));
        self.executor.run("nix-env", &args, false).await?;
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self.executor.run_output("nix-env", &["-q"], false).await?;
        Ok(out.lines().filter_map(|l| {
            let (name, ver) = l.rsplit_once('-')?;
            Some(Package { name: name.to_string(), version: Some(ver.to_string()), backend: "nix".into(), ..Package::new("", "") })
        }).collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: Uses JSON output to get clean attribute paths
        let out = self.executor.run_output("nix-env", &["-qa", "--json", query], false).await?;
        let json: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
        let mut res = vec![];
        if let Some(obj) = json.as_object() {
            for (attr, data) in obj {
                let mut p = Package::new(attr, "nix");
                p.description = data.get("description").and_then(|v| v.as_str()).map(|s| s.to_string());
                res.push(p);
            }
        }
        Ok(res)
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        let out = self.executor.run_output("nix-env", &["-qa", "--json", "-A", package], false).await?;
        let json: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
        if let Some(data) = json.get(package) {
            return Ok(Some(Package {
                name: package.to_string(),
                version: data.get("version").and_then(|v| v.as_str()).map(|s| s.to_string()),
                description: data.get("description").and_then(|v| v.as_str()).map(|s| s.to_string()),
                repository: None,
                backend: "nix".into(),
                ..Package::new("", "")
            }));
        }
        Ok(None)
    }

    async fn upgrade(&self, _: bool) -> Result<()> {
        self.executor.run("nix-env", &["-u"], false).await?;
        Ok(())
    }
}