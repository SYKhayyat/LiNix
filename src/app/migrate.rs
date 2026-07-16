use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::core::{Error, Package, Result, StateRegistry};
use chrono::Local;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tracing::{debug, info, instrument, trace, warn};

/// The System Ingestion Engine.
///
/// The Migrator identifies components currently installed on the operating
/// system that are not yet managed by LiNix. It generates declarative
/// manifests for these components and acquires ownership in the StateRegistry.
pub struct Migrator {
    /// Registry for capability-based discovery across all backends.
    registry: Arc<BackendRegistry>,
    /// Shared mutable access to the mission-critical system state.
    state: Arc<Mutex<StateRegistry>>,
    /// Global application configuration.
    config: Arc<Config>,
}

impl Migrator {
    /// Initializes a new Migrator with explicit kernel dependencies.
    pub fn new(
        registry: Arc<BackendRegistry>,
        state: Arc<Mutex<StateRegistry>>,
        config: &Config,
    ) -> Self {
        Self {
            registry,
            state,
            config: Arc::new(config.clone()),
        }
    }

    /// Primary entry point: Discovery -> Manifesting -> Acquisition.
    ///
    /// This method performs a non-destructive system crawl to identify
    /// manual installations and bring them under LiNix control.
    #[instrument(skip(self))]
    pub async fn migrate(&self) -> Result<()> {
        info!("Migrator: Initiating automated system discovery closure.");

        let mut discovered_packages = Vec::new();
        let mut seen_keys = HashSet::new();

        // --- PHASE 1: DISCOVERY ---
        // Query every backend that supports the Queryable trait
        for backend in self.registry.available() {
            if let Some(queryable) = backend.as_queryable() {
                // Adoption is only safe for backends that can name the packages a person
                // actually chose. Where a manager installs dependencies but exposes no way
                // to tell them apart, the honest answer is to adopt nothing: everything
                // adopted here lands in the global state registry, and anything in that
                // registry is a removal candidate on the next sync. Adopting nothing costs
                // the user a manual manifest entry; adopting a dependency graph costs them
                // their system.
                if !queryable.tracks_manual() {
                    info!(
                        "Migrator: Backend '{}' cannot distinguish user-chosen packages from \
                         dependencies — skipping adoption. Add its packages to a manifest by \
                         hand if you want them managed.",
                        backend.name()
                    );
                    continue;
                }

                debug!(
                    "Migrator: Probing backend '{}' for unmanaged components...",
                    backend.name()
                );

                // Identify packages explicitly installed by the user
                match queryable.list_manual().await {
                    Ok(pkgs) => {
                        let state_guard = self.state.lock().await;
                        for pkg in pkgs {
                            let key = format!("{}:{}", pkg.backend, pkg.name);

                            // Candidate Criteria:
                            // 1. Not currently tracked in LiNix state.
                            // 2. Not already identified in this discovery cycle.
                            // 3. Not a core protected system package (sudo, kernel, etc).
                            if !state_guard.is_managed(&pkg.backend, &pkg.name)
                                && seen_keys.insert(key.clone())
                                && !self.config.is_protected(&pkg.name)
                            {
                                trace!("Migrator: Candidate identified for ingestion: {}", key);
                                discovered_packages.push(pkg);
                            }
                        }
                    }
                    Err(e) => {
                        warn!(
                            "Migrator: Backend '{}' discovery failed: {}. Continuing crawl.",
                            backend.name(),
                            e
                        );
                    }
                }
            }
        }

        if discovered_packages.is_empty() {
            info!("Migrator: Discovery cycle complete. System state is already synchronized.");
            return Ok(());
        }

        info!(
            "Migrator: Discovered {} candidates for declarative ingestion.",
            discovered_packages.len()
        );

        // --- PHASE 2: MANIFEST GENERATION ---
        // Create a new .txt manifest file for the ingested components
        let timestamp = Local::now().format("%Y%m%d_%H%M%S");
        let filename = format!("migrated_{}.txt", timestamp);
        let manifest_path = self.config.groups_dir.join(&filename);

        info!(
            "Migrator: Constructing declarative ingestion manifest: {:?}",
            manifest_path
        );

        let manifest_lines: Vec<String> = discovered_packages
            .iter()
            .map(|p| format!("{}:{}", p.backend, p.name))
            .collect();

        // Ensure manifest destination directory exists asynchronously
        if let Some(parent) = manifest_path.parent() {
            if !tokio::fs::try_exists(parent).await.unwrap_or(false) {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(Error::from)?;
            }
        }

        // Atomically create and write the manifest file
        let mut file = tokio::fs::File::create(&manifest_path)
            .await
            .map_err(Error::from)?;
        let header = format!(
            "# LiNix Ingestion Manifest\n# Timestamp: {}\n# Origin: Automated Migration\n\n",
            Local::now()
        );

        file.write_all(header.as_bytes()).await?;
        file.write_all(manifest_lines.join("\n").as_bytes()).await?;
        file.write_all(b"\n").await?;
        file.flush().await?;

        // --- PHASE 3: STATE ACQUISITION ---
        // Finalize ownership by updating the StateRegistry with source metadata
        {
            let mut state_mut = self.state.lock().await;
            // Feature 3: The filename serves as the source origin for these packages
            let source_meta = Some(format!("manifest:{}", filename));

            for pkg in &discovered_packages {
                // A+ Hardening: Provide all 6 arguments to modernized state.add
                state_mut.add(
                    &pkg.backend,
                    &pkg.name,
                    pkg.version.clone(),
                    HashMap::new(), // Default options for ingested packages
                    source_meta.clone(),
                    false, // Ingested packages are permanent (non-transient)
                );
            }

            // Persist ownership records to disk (Offloaded to dedicated task)
            let state_to_persist = state_mut.clone();
            tokio::task::spawn_blocking(move || state_to_persist.save())
                .await
                .map_err(|e| Error::Other(format!("State-save thread failure: {}", e)))??;
        }

        info!("Migrator: State registry aligned. Migration successful.");

        println!("\nIngestion Complete!");
        println!("{:-<60}", "");
        println!("Manifest Created:  {}", manifest_path.display());
        println!("Packages Ingested: {}", discovered_packages.len());
        println!("{:-<60}", "");
        println!("Success: Discovered components are now managed declaratively by LiNix.");

        Ok(())
    }

    /// Performs a destructive Discovery cycle without generating files or
    /// acquiring state.
    ///
    /// Used by the CLI to show users what LiNix *would* ingest.
    pub async fn audit(&self) -> Result<Vec<Package>> {
        let mut unmanaged = Vec::new();
        let mut seen = HashSet::new();
        let state_guard = self.state.lock().await;

        for backend in self.registry.available() {
            if let Some(queryable) = backend.as_queryable() {
                // Same gate as `migrate`: a backend that cannot report user intent has
                // nothing trustworthy to preview.
                if !queryable.tracks_manual() {
                    continue;
                }
                if let Ok(pkgs) = queryable.list_manual().await {
                    for pkg in pkgs {
                        let key = format!("{}:{}", pkg.backend, pkg.name);
                        if !state_guard.is_managed(backend.name(), &pkg.name)
                            && seen.insert(key)
                            && !self.config.is_protected(&pkg.name)
                        {
                            unmanaged.push(pkg);
                        }
                    }
                }
            }
        }
        Ok(unmanaged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::generic::{
        GenericBackendCore, GenericQueryable, ManagerConfig, ManualFormat, ManualListing,
    };
    use crate::core::executor::{DryRunOutput, MockExecutor};
    use crate::core::{BackendCapabilities, CommandExecutor};
    use crate::parsers::LambdaParser;
    use dashmap::DashMap;
    use std::path::PathBuf;

    /// A backend named `apt` whose manual-listing behaviour is whatever the test needs.
    fn registry_with(manual: ManualListing, mock: Arc<MockExecutor>) -> Arc<BackendRegistry> {
        let vfs: Arc<DashMap<PathBuf, String>> = Arc::new(DashMap::new());
        mock.set_command_exists("apt", true);
        let exec = CommandExecutor::with_layer(
            true,
            false,
            mock.clone(),
            vfs,
            Arc::new(DashMap::new()),
        );
        let mut config = ManagerConfig {
            name: "apt".into(),
            install_args: vec![],
            remove_args: vec![],
            list_args: vec!["-W".into()],
            manual: ManualListing::AllInstalled,
            essential_args: None,
            search_args: vec![],
            search_binary: None,
            list_binary: Some("dpkg-query".into()),
            upgrade_args: vec![],
            update_args: None,
            orphan_args: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            depends_args: None,
            version_pin: None,
            needs_root: false,
            is_exclusive: false,
            flag_map: HashMap::new(),
        };
        config.manual = manual;

        let core = Arc::new(GenericBackendCore {
            name: "apt".into(),
            executor: exec,
            config,
            parser: Arc::new(LambdaParser {
                installed_fn: crate::parsers::apt::parse_list,
                search_fn: crate::parsers::apt::parse_search,
            }),
        });
        let mut reg = BackendRegistry::new();
        reg.register(Arc::new(
            BackendCapabilities::builder(core.clone())
                .with_queryable(Arc::new(GenericQueryable { core }))
                .build(),
        ));
        Arc::new(reg)
    }

    fn migrator(reg: Arc<BackendRegistry>) -> Migrator {
        let config = Config {
            // Keep the default protected list out of these assertions.
            protected_packages: vec![],
            ..Config::default()
        };
        let state = Arc::new(Mutex::new(StateRegistry::default()));
        Migrator::new(reg, state, &config)
    }

    #[tokio::test]
    async fn adopts_nothing_from_a_backend_that_cannot_report_intent() {
        // The safety backstop. `dpkg-query -W` is wired and would happily return an entire
        // dependency graph; because the backend admits it cannot tell user-chosen packages
        // from dependencies, adoption must skip it entirely rather than adopt all of it.
        let vfs: Arc<DashMap<PathBuf, String>> = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs));
        mock.set_response(
            "dpkg-query -W",
            Ok(DryRunOutput {
                stdout: b"apt 2.7.14\nlibperl5.38t64 5.38.2\npython3 3.12.3\n".to_vec(),
                stderr: vec![],
            }
            .into()),
        );

        let found = migrator(registry_with(ManualListing::Unsupported, mock.clone()))
            .audit()
            .await
            .unwrap();
        assert!(
            found.is_empty(),
            "expected no adoption candidates, got {:?}",
            found.iter().map(|p| &p.name).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn adopts_only_the_packages_the_backend_reports_as_user_chosen() {
        // The real fix: ask `apt-mark showmanual`, not the installed listing. The
        // dependency present in dpkg-query's output must not be adopted.
        let vfs: Arc<DashMap<PathBuf, String>> = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs));
        mock.set_response(
            "apt-mark showmanual",
            Ok(DryRunOutput {
                stdout: b"apt\njq\n".to_vec(),
                stderr: vec![],
            }
            .into()),
        );
        mock.set_response(
            "dpkg-query -W",
            Ok(DryRunOutput {
                stdout: b"apt 2.7.14\njq 1.7.1\nlibperl5.38t64 5.38.2\n".to_vec(),
                stderr: vec![],
            }
            .into()),
        );

        let reg = registry_with(
            ManualListing::Command {
                binary: Some("apt-mark".into()),
                args: vec!["showmanual".into()],
                format: ManualFormat::BareNames,
            },
            mock.clone(),
        );
        let names: Vec<String> = migrator(reg)
            .audit()
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();

        assert_eq!(names, vec!["apt", "jq"]);
        assert!(
            !names.contains(&"libperl5.38t64".to_string()),
            "a pure dependency was adopted — this is the bug that purged the container"
        );
    }

    #[tokio::test]
    async fn protected_packages_are_never_adopted() {
        let vfs: Arc<DashMap<PathBuf, String>> = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs));
        mock.set_response(
            "apt-mark showmanual",
            Ok(DryRunOutput {
                stdout: b"jq\npython3\n".to_vec(),
                stderr: vec![],
            }
            .into()),
        );
        let reg = registry_with(
            ManualListing::Command {
                binary: Some("apt-mark".into()),
                args: vec!["showmanual".into()],
                format: ManualFormat::BareNames,
            },
            mock,
        );
        let config = Config {
            protected_packages: vec!["python3".into()],
            ..Config::default()
        };
        let m = Migrator::new(reg, Arc::new(Mutex::new(StateRegistry::default())), &config);
        let names: Vec<String> = m.audit().await.unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["jq"]);
    }
}
