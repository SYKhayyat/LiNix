use crate::core::{CommandExecutor, Package, PackageManager, Result};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct GoManager {
    executor: CommandExecutor,
    available: OnceCell<bool>,
}

impl GoManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor, available: OnceCell::new() }
    }

    /// REAL LOGIC: Determines the correct Go binary installation directory
    fn get_gobin() -> PathBuf {
        if let Ok(gobin) = std::env::var("GOBIN") {
            PathBuf::from(gobin)
        } else if let Ok(gopath) = std::env::var("GOPATH") {
            PathBuf::from(gopath).join("bin")
        } else {
            // Default Go path is ~/go/bin
            dirs::home_dir().unwrap_or_default().join("go").join("bin")
        }
    }
}

#[async_trait]
impl PackageManager for GoManager {
    fn name(&self) -> &str { "go" }

    fn is_available(&self) -> bool {
        *self.available.get_or_init(|| {
            std::process::Command::new("go").arg("version").output().is_ok()
        })
    }

    async fn install(&self, p: &[String], sudo: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for pkg in p {
            // REAL LOGIC: Go requires @version (usually @latest) for installs
            let spec = if pkg.contains('@') { pkg.clone() } else { format!("{}@latest", pkg) };
            self.executor.run("go", &["install", &spec], sudo).await?;
        }
        Ok(())
    }

    async fn remove(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        let bin_dir = Self::get_gobin();
        for pkg in p {
            // Converts "github.com/user/repo" to "repo" to find the file
            let binary_name = pkg.split('/').last().unwrap_or(pkg);
            let path = bin_dir.join(binary_name);
            if path.exists() {
                let _ = tokio::fs::remove_file(path).await;
            }
        }
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let bin_dir = Self::get_gobin();
        if !bin_dir.exists() { return Ok(vec![]); }
        
        let mut entries = tokio::fs::read_dir(bin_dir).await?;
        let mut pkgs = Vec::new();

        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_file() {
                pkgs.push(Package::new(entry.file_name().to_string_lossy(), "go"));
            }
        }
        Ok(pkgs)
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: Query the search API (proxied via godoc) to find packages
        let url = format!("https://api.godoc.org/search?q={}", query);
        let client = reqwest::Client::new();
        let res = client.get(url).header("User-Agent", "linix-manager").send().await?;
        
        if res.status().is_success() {
            let json: serde_json::Value = res.json().await?;
            if let Some(results) = json.get("results").and_then(|r| r.as_array()) {
                return Ok(results.iter().filter_map(|r| {
                    let path = r.get("path")?.as_str()?;
                    Some(Package::new(path, "go"))
                }).collect());
            }
        }
        Ok(vec![])
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Query Google's Go Proxy to get the latest stable version info
        let url = format!("https://proxy.golang.org/{}/@latest", package);
        let client = reqwest::Client::new();
        let res = client.get(url).header("User-Agent", "linix-manager").send().await?;
        
        if res.status().is_success() {
            let json: serde_json::Value = res.json().await?;
            return Ok(Some(Package {
                name: package.to_string(),
                version: json.get("Version").and_then(|v| v.as_str()).map(|s| s.to_string()),
                description: None, // Go Proxy doesn't provide descriptions
                repository: Some(format!("https://pkg.go.dev/{}", package)),
                backend: "go".into(),
                ..Package::new("", "")
            }));
        }
        Ok(None)
    }

    async fn update(&self, _: bool) -> Result<()> {
        // Go doesn't have a local index to refresh
        Ok(())
    }

    async fn upgrade(&self, s: bool) -> Result<()> {
        // REAL LOGIC: Go upgrades are performed by re-running 'go install' at @latest.
        // We iterate through all binaries found in GOBIN.
        let installed = self.list_installed().await?;
        let names: Vec<String> = installed.into_iter().map(|p| p.name).collect();
        self.install(&names, s).await
    }
}