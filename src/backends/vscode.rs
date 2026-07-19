use crate::core::{
    BackendCore, CommandExecutor, Installable, MetadataProvider, Package, PackageSpec, Queryable,
    RateLimiter, Result, Searchable, Upgradable,
};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tracing::info;

pub struct VscodeBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
    pub rate_limiter: RateLimiter,
}

impl VscodeBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "vscode".to_string(),
            rate_limiter: RateLimiter::vscode_marketplace(),
        }
    }

    pub async fn query_marketplace(&self, query: &str) -> Result<serde_json::Value> {
        self.rate_limiter
            .execute(|| async {
                let client = reqwest::Client::builder()
                    .user_agent("linix-manager")
                    .build()
                    .map_err(crate::core::Error::from)?;

                let body = json!({
                    "filters": [{
                        "criteria": [
                            { "filterType": 10, "value": query },
                            { "filterType": 8, "value": "Microsoft.VisualStudio.Code" }
                        ],
                        "pageSize": 20,
                        "pageNumber": 1
                    }],
                    "flags": 0x21c
                });

                let res = client
                    .post(
                        "https://marketplace.visualstudio.com/_apis/public/gallery/extensionquery",
                    )
                    .header("Accept", "application/json;api-version=3.0-preview.1")
                    .header("Content-Type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(crate::core::Error::from)?;

                if !res.status().is_success() {
                    return Err(crate::core::Error::Other(format!(
                        "Marketplace API error: {}",
                        res.status()
                    )));
                }

                res.json().await.map_err(crate::core::Error::from)
            })
            .await
    }
}

#[async_trait]
impl BackendCore for VscodeBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("code")
    }
    fn needs_root(&self) -> bool {
        false
    }
    // Deliberately no `check_health` override: the default reports Critical when the
    // backend is unavailable, and an always-Ok override here masks a missing `code` binary.
}

#[async_trait]
impl MetadataProvider for VscodeBackendCore {
    async fn get_dependencies(&self, name: &str) -> Result<Vec<String>> {
        let json = self.query_marketplace(name).await?;
        let mut deps = Vec::new();

        if let Some(extensions) = json["results"][0]["extensions"].as_array() {
            if let Some(ext) = extensions.first() {
                if let Some(extension_deps) = ext["extensionDependencies"].as_array() {
                    for dep in extension_deps {
                        if let Some(dep_name) = dep.as_str() {
                            deps.push(dep_name.to_string());
                        }
                    }
                }
            }
        }
        Ok(deps)
    }
}

pub struct VscodeInstallable {
    pub core: Arc<VscodeBackendCore>,
}

#[async_trait]
impl Installable for VscodeInstallable {
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        for spec in specs {
            // VS Code supports pinning an extension version: `publisher.ext@1.2.3`.
            let target = match spec.options.get("version") {
                Some(v) if crate::backends::concrete_version(v) => format!("{}@{}", spec.name, v),
                _ => spec.name.clone(),
            };
            info!("VSCode: Installing extension '{}'...", target);
            self.core
                .executor
                .run("code", &["--install-extension", &target, "--force"], false)
                .await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _: bool) -> Result<()> {
        for name in names {
            info!("VSCode: Uninstalling extension '{}'...", name);
            self.core
                .executor
                .run("code", &["--uninstall-extension", name], false)
                .await?;
        }
        Ok(())
    }
}

pub struct VscodeQueryable {
    pub core: Arc<VscodeBackendCore>,
}

#[async_trait]
impl Queryable for VscodeQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let out = self
            .core
            .executor
            .run_output("code", &["--list-extensions", "--show-versions"], false)
            .await?;
        let mut extensions = Vec::new();

        for line in out.lines() {
            if let Some((name, version)) = line.split_once('@') {
                extensions.push(Package::with_version(name.trim(), version.trim(), "vscode"));
            } else if !line.trim().is_empty() {
                extensions.push(Package::new(line.trim(), "vscode"));
            }
        }
        Ok(extensions)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let json = self.core.query_marketplace(name).await?;

        if let Some(ext) = json["results"][0]["extensions"]
            .as_array()
            .and_then(|a| a.first())
        {
            let publisher = ext["publisher"]["publisherName"]
                .as_str()
                .unwrap_or("unknown");
            let ext_name = ext["extensionName"].as_str().unwrap_or("unknown");

            let mut p = Package::new(format!("{}.{}", publisher, ext_name), "vscode");
            p.version = ext["versions"][0]["version"]
                .as_str()
                .map(|s| s.to_string());

            if let Some(desc) = ext["shortDescription"].as_str() {
                p.properties.insert("description".into(), desc.to_string());
            }

            p.properties
                .insert("publisher".into(), publisher.to_string());
            return Ok(Some(p));
        }
        Ok(None)
    }
}

pub struct VscodeUpgradable {
    pub core: Arc<VscodeBackendCore>,
}

#[async_trait]
impl Upgradable for VscodeUpgradable {
    // The `code` CLI has no batch-update command; refreshing metadata is a no-op.
    async fn update(&self, _: bool) -> Result<()> {
        Ok(())
    }

    /// Re-install every installed extension with `--force`, which pulls the latest
    /// published version for each. This is the documented way to upgrade extensions
    /// from the CLI.
    async fn upgrade(&self, _: bool) -> Result<()> {
        let out = self
            .core
            .executor
            .run_output("code", &["--list-extensions"], false)
            .await?;
        for line in out.lines() {
            let id = line.trim();
            if id.is_empty() {
                continue;
            }
            info!("VSCode: Upgrading extension '{}'...", id);
            self.core
                .executor
                .run("code", &["--install-extension", id, "--force"], false)
                .await?;
        }
        Ok(())
    }

    // Extensions have no orphan concept managed by the CLI.
}

pub struct VscodeSearchable {
    pub core: Arc<VscodeBackendCore>,
}

#[async_trait]
impl Searchable for VscodeSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let json = self.core.query_marketplace(query).await?;
        let mut results = Vec::new();

        if let Some(extensions) = json["results"][0]["extensions"].as_array() {
            for ext in extensions {
                let publisher = ext["publisher"]["publisherName"].as_str().unwrap_or("");
                let name = ext["extensionName"].as_str().unwrap_or("");

                let mut p = Package::new(format!("{}.{}", publisher, name), "vscode");
                if let Some(desc) = ext["shortDescription"].as_str() {
                    p.properties.insert("description".into(), desc.to_string());
                }
                results.push(p);
            }
        }
        Ok(results)
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let core = Arc::new(VscodeBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(VscodeInstallable { core: core.clone() }))
            .with_queryable(Arc::new(VscodeQueryable { core: core.clone() }))
            .with_searchable(Arc::new(VscodeSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(VscodeUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}
