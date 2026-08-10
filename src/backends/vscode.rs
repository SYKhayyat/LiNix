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
                // Inside the rate limiter's retry closure, so a retried request used to build a
                // whole new client — and with it a whole new TLS handshake to the marketplace.
                let client = crate::core::http::api(
                    "linix-manager",
                    crate::backends::node_registry::http_timeout().as_secs(),
                )?;

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
    fn probes(&self) -> Vec<String> {
        vec!["code".into()]
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
    /// One `code` process for every extension (`Q45`).
    ///
    /// **The flag repeats; the operand does not.** `--install-extension a --install-extension b`
    /// is how VS Code takes several, so the names cannot simply be appended the way an operand
    /// list is — which is why this does not go through the generic batching.
    ///
    /// Each launch of `code` starts an Electron process and re-scans the extension host, so N
    /// extensions one at a time is N of those.
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        if specs.is_empty() {
            return Ok(());
        }
        let mut args = vec!["--force".to_string()];
        for spec in specs {
            // VS Code supports pinning an extension version: `publisher.ext@1.2.3`.
            let target = match spec.options.one("version") {
                Some(v) if crate::backends::concrete_version(v) => format!("{}@{}", spec.name, v),
                _ => spec.name.clone(),
            };
            args.push("--install-extension".to_string());
            crate::core::argv::push_names(&mut args, "code", [&target]);
        }
        info!("VSCode: Installing {} extension(s)...", specs.len());
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.core.executor.run("code", &arg_refs, false).await?;
        Ok(())
    }

    async fn remove(
        &self,
        names: &[String],
        _: bool,
        _reaped: crate::app::sync::guard::Reaped,
    ) -> Result<()> {
        if names.is_empty() {
            return Ok(());
        }
        let mut args = Vec::new();
        for name in names {
            args.push("--uninstall-extension".to_string());
            crate::core::argv::push_names(&mut args, "code", [name]);
        }
        info!("VSCode: Uninstalling {} extension(s)...", names.len());
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.core.executor.run("code", &arg_refs, false).await?;
        Ok(())
    }
}

pub struct VscodeQueryable {
    pub core: Arc<VscodeBackendCore>,
}

#[async_trait]
impl Queryable for VscodeQueryable {
    fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
        (self.core.executor.installed_listings(), &self.core.name)
    }

    async fn fetch_installed(&self) -> Result<Vec<Package>> {
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

    /// Is this extension installed **here**?
    ///
    /// It used to POST to `marketplace.visualstudio.com` and answer `Some` for anything that
    /// *exists* there, carrying the marketplace's *latest* version. Three consequences, all
    /// silent: `linix install vscode:x` reported success and installed nothing, a `@version=`
    /// pin re-installed for ever the moment upstream published a newer build, and every plan
    /// made one rate-limited HTTPS POST per declared extension. Same bug as `mise:`, whose
    /// obituary is at `mise.rs:183`.
    ///
    /// The marketplace answers *could this be installed?* — that is `search`, and
    /// `get_dependencies`, both of which still ask it.
    ///
    /// Extension ids are case-insensitive on the marketplace and `code --list-extensions`
    /// prints the publisher's canonical casing, so a manifest that spells `ms-python.Python`
    /// must still resolve to the installed row.
    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.installed_listing().await?;
        Ok(all
            .iter()
            .find(|p| p.name == name || p.name.eq_ignore_ascii_case(name))
            .cloned()
            .map(|mut p| {
                if let Some((publisher, _)) = p.name.split_once('.') {
                    p.properties
                        .insert("publisher".to_string(), publisher.to_string());
                }
                p
            }))
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
            let mut args = vec!["--force".to_string(), "--install-extension".to_string()];
            crate::core::argv::push_names(&mut args, "code", [id]);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core.executor.run("code", &arg_refs, false).await?;
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
                    p.properties
                        .insert("description".to_string(), desc.to_string());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn vscode_with(
        list_output: &str,
    ) -> (
        Arc<VscodeBackendCore>,
        Arc<crate::core::executor::MockExecutor>,
    ) {
        let vfs = Arc::new(dashmap::DashMap::new());
        let mock = Arc::new(crate::core::executor::MockExecutor::new(vfs.clone()));
        mock.set_response(
            "code --list-extensions --show-versions",
            Ok(crate::core::executor::DryRunOutput {
                stdout: list_output.as_bytes().to_vec(),
                stderr: vec![],
            }
            .into()),
        );
        let exec = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(dashmap::DashMap::new()),
        );
        (Arc::new(VscodeBackendCore::new(exec)), mock)
    }

    /// `info` asked `marketplace.visualstudio.com` and returned `Some` for anything that
    /// *exists* there, at the marketplace's *latest* version. Three silent consequences:
    /// `linix install vscode:x` reported success and installed nothing, a `@version=` pin
    /// re-installed for ever the moment upstream published, and every plan made one
    /// rate-limited HTTPS POST per declared extension.
    ///
    /// The same bug's obituary is in `mise.rs:183`, written 2026-07-24. The assertion that the
    /// catalogue is not consulted is the one that file already makes; this is its sibling.
    #[tokio::test]
    async fn info_answers_installed_here_not_published_to_the_marketplace() {
        let (core, mock) =
            vscode_with("ms-python.python@2024.2.1\nrust-lang.rust-analyzer@0.3.1\n");
        let q = VscodeQueryable { core };

        // An extension the marketplace certainly carries, and this machine does not.
        assert!(
            q.info("ms-vscode.cpptools").await.unwrap().is_none(),
            "info claimed an uninstalled extension was present — the planner skips its install"
        );
        // One that really is installed, at the version the machine has and not the latest one.
        let found = q
            .info("ms-python.python")
            .await
            .unwrap()
            .expect("installed");
        assert_eq!(found.version.as_deref(), Some("2024.2.1"));
        assert_eq!(
            found.properties.get("publisher").map(String::as_str),
            Some("ms-python")
        );

        // Extension ids are case-insensitive on the marketplace, so a manifest that spells the
        // publisher's name differently still has to resolve to the installed row.
        assert!(q.info("MS-Python.Python").await.unwrap().is_some());

        // The network is not the machine, and nothing here asked it.
        let calls = mock.get_calls().await;
        assert!(
            calls.iter().all(|c| c.starts_with("code ")),
            "info reached past the `code` CLI: {:?}",
            calls
        );
    }

    #[tokio::test]
    async fn an_extension_id_is_an_option_value_so_no_terminator_is_emitted() {
        let vfs = Arc::new(dashmap::DashMap::new());
        let mock = Arc::new(crate::core::executor::MockExecutor::new(vfs.clone()));
        let exec = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(dashmap::DashMap::new()),
        );
        let core = Arc::new(VscodeBackendCore::new(exec));

        VscodeInstallable { core: core.clone() }
            .install(
                &[PackageSpec {
                    name: "ms-python.python".into(),
                    backend: "vscode".into(),
                    ..Default::default()
                }],
                false,
            )
            .await
            .unwrap();
        VscodeInstallable { core: core.clone() }
            .remove(
                &["ms-python.python".to_string()],
                false,
                crate::app::sync::guard::Reaped::for_reason(
                    crate::app::sync::guard::GuardScope::Remove,
                    "a unit test of the effector itself",
                ),
            )
            .await
            .unwrap();

        assert_eq!(
            mock.get_calls().await,
            vec![
                "code --force --install-extension ms-python.python",
                "code --uninstall-extension ms-python.python",
            ]
        );
        assert!(!crate::core::argv::terminates_options("code"));
    }

    /// Q45, and the one whose argv is not a name list.
    ///
    /// **The flag repeats; the operand does not.** VS Code takes several extensions as
    /// `--install-extension a --install-extension b`, so this cannot go through the generic
    /// name-appending batcher — and each `code` launch starts an Electron process and rescans
    /// the extension host, which is what N launches costs.
    #[tokio::test]
    async fn a_batch_of_extensions_is_one_code_process() {
        let vfs = Arc::new(dashmap::DashMap::new());
        let mock = Arc::new(crate::core::executor::MockExecutor::new(vfs.clone()));
        let exec = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(dashmap::DashMap::new()),
        );
        let core = Arc::new(VscodeBackendCore::new(exec));
        let specs = vec![
            crate::core::PackageSpec {
                name: "rust-lang.rust-analyzer".into(),
                backend: "vscode".into(),
                ..Default::default()
            },
            crate::core::PackageSpec {
                name: "tamasfe.even-better-toml".into(),
                backend: "vscode".into(),
                ..Default::default()
            },
        ];
        VscodeInstallable { core: core.clone() }
            .install(&specs, false)
            .await
            .unwrap();
        VscodeInstallable { core: core.clone() }
            .remove(
                &[
                    "rust-lang.rust-analyzer".to_string(),
                    "tamasfe.even-better-toml".to_string(),
                ],
                false,
                crate::app::sync::guard::Reaped::for_reason(
                    crate::app::sync::guard::GuardScope::Remove,
                    "a unit test of the effector itself",
                ),
            )
            .await
            .unwrap();

        let calls = mock.get_calls().await;
        assert_eq!(calls.len(), 2, "one process each way, got {:?}", calls);
        assert_eq!(
            calls[0].matches("--install-extension").count(),
            2,
            "the flag has to repeat per extension: {:?}",
            calls
        );
        assert!(calls[0].contains("rust-lang.rust-analyzer"), "{:?}", calls);
        assert!(calls[0].contains("tamasfe.even-better-toml"), "{:?}", calls);
        assert_eq!(
            calls[1].matches("--uninstall-extension").count(),
            2,
            "{:?}",
            calls
        );
    }
}
