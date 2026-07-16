use crate::app::sync::guard;
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::core::{Error, Package, Result, StateRegistry};
use chrono::Local;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, instrument, trace, warn};

/// Adoption: bringing packages that are already installed under LiNix's management.
///
/// The Migrator asks every backend which packages a person chose to install, and writes
/// the answer out as a manifest. Two properties matter more than anything else here:
///
/// 1. *The answer is an estimate.* Package managers differ in how well they separate a
///    user's choices from packages dragged in as dependencies, and some cannot do it at
///    all. The manifest says so, in those words, and names the command behind each answer
///    so a reader can check it.
/// 2. *Adoption is the dangerous half.* Everything adopted lands in the global state
///    registry, and anything in that registry is a removal candidate on the next sync. An
///    over-broad adoption is not a cosmetic mistake; it is a queued mass removal.
pub struct Migrator {
    /// Registry for capability-based discovery across all backends.
    registry: Arc<BackendRegistry>,
    /// Shared mutable access to the system state.
    state: Arc<Mutex<StateRegistry>>,
    /// Global application configuration.
    config: Arc<Config>,
}

/// A package that was discovered but deliberately not adopted.
#[derive(Debug, Clone)]
pub struct Skipped {
    pub package: Package,
    /// Why, in words fit to print in the manifest.
    pub reason: String,
}

/// What a discovery crawl found. `migrate` and `audit` share this so the preview cannot
/// disagree with the real run — they were near-duplicate loops that had already drifted
/// apart on two separate points.
#[derive(Debug, Default)]
pub struct Discovery {
    /// Unmanaged, user-chosen, and not protected: these get adopted.
    pub adopt: Vec<Package>,
    /// Discovered, but LiNix leaves them alone. Reported, never adopted.
    pub skipped: Vec<Skipped>,
    /// Backend name -> how its manual set was determined, for the manifest header.
    pub sources: BTreeMap<String, String>,
}

impl Migrator {
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

    /// The one discovery crawl. Read-only: it acquires nothing and writes nothing.
    ///
    /// `migrate` and `audit` both go through here. They used to be near-duplicate loops
    /// and had already drifted on two points — one keyed managed-state lookups off the
    /// package's backend and the other off the backend's own name, and one warned on a
    /// backend error while the other swallowed it, so the preview could hide a failure the
    /// real run reported. A preview that does not run the same code is not a preview.
    pub async fn discover(&self) -> Result<Discovery> {
        let mut found = Discovery::default();
        let mut seen_keys = HashSet::new();
        let mut candidates: Vec<Package> = Vec::new();

        for backend in self.registry.available() {
            let Some(queryable) = backend.as_queryable() else {
                continue;
            };

            // Adoption is only safe for backends that can name the packages a person
            // actually chose. Where a manager installs dependencies but exposes no way to
            // tell them apart, the honest answer is to adopt nothing. Adopting nothing
            // costs the user a manual manifest entry; adopting a dependency graph costs
            // them their system.
            if !queryable.tracks_manual() {
                info!(
                    "Migrator: backend '{}' cannot distinguish user-chosen packages from \
                     dependencies — skipping adoption. Add its packages to a manifest by \
                     hand if you want them managed.",
                    backend.name()
                );
                continue;
            }

            debug!("Migrator: probing backend '{}'...", backend.name());

            match queryable.list_manual().await {
                Ok(pkgs) => {
                    found
                        .sources
                        .insert(backend.name().to_string(), queryable.manual_source());
                    let state_guard = self.state.lock().await;
                    for pkg in pkgs {
                        let key = format!("{}:{}", pkg.backend, pkg.name);
                        if !state_guard.is_managed(&pkg.backend, &pkg.name)
                            && seen_keys.insert(key.clone())
                        {
                            trace!("Migrator: candidate: {}", key);
                            candidates.push(pkg);
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        "Migrator: backend '{}' discovery failed: {}. Continuing crawl.",
                        backend.name(),
                        e
                    );
                }
            }
        }

        // Ask the same question the removal guard asks, through the same function, so
        // "protected" means one thing across the whole tool: a package LiNix does not
        // touch. It will not adopt it, and it will not remove it. Two separate notions of
        // protection is how a package ends up adoptable but unremovable, or the reverse.
        let backends: HashSet<String> = candidates.iter().map(|p| p.backend.clone()).collect();
        let os_essential = guard::essential_names(&self.registry, &backends).await;

        for pkg in candidates {
            match guard::protection_of(&self.config, &pkg.backend, &pkg.name, &os_essential) {
                Some(p) => found.skipped.push(Skipped {
                    reason: p.reason(),
                    package: pkg,
                }),
                None => found.adopt.push(pkg),
            }
        }

        // Deterministic output: the manifest is a file people read and diff, and the
        // discovery order is whatever order the backends happened to answer in.
        found
            .adopt
            .sort_by(|a, b| (&a.backend, &a.name).cmp(&(&b.backend, &b.name)));
        found.skipped.sort_by(|a, b| {
            (&a.package.backend, &a.package.name).cmp(&(&b.package.backend, &b.package.name))
        });

        Ok(found)
    }

    /// Discovery -> manifest -> acquisition.
    #[instrument(skip(self))]
    pub async fn migrate(&self) -> Result<()> {
        info!("Migrator: initiating system discovery.");
        let found = self.discover().await?;

        if found.adopt.is_empty() {
            info!("Migrator: discovery complete. Nothing new to adopt.");
            println!("Nothing to adopt: every package your managers report as user-chosen is");
            println!("already managed, or is protected and deliberately left alone.");
            if !found.skipped.is_empty() {
                println!(
                    "\n{} discovered package(s) were left alone. See `linix protected <pkg>`.",
                    found.skipped.len()
                );
            }
            return Ok(());
        }

        info!("Migrator: {} candidate(s) for adoption.", found.adopt.len());

        let timestamp = Local::now().format("%Y%m%d_%H%M%S");
        let filename = format!("migrated_{}.txt", timestamp);
        let manifest_path = self.config.groups_dir.join(&filename);

        info!("Migrator: writing manifest {:?}", manifest_path);

        // `atomic_write` writes to a temp file and renames, so a crash mid-write leaves
        // either the old file or the new one, never a truncated list. The previous code
        // claimed to be atomic in a comment while doing `File::create` plus three
        // sequential writes — a truncated manifest is a silent mass removal next sync.
        let manifest = self.render_manifest(&found);
        let path = manifest_path.clone();
        tokio::task::spawn_blocking(move || crate::utils::file::atomic_write(&path, &manifest))
            .await
            .map_err(|e| Error::Other(format!("manifest-write thread failure: {}", e)))??;

        {
            let mut state_mut = self.state.lock().await;
            let source_meta = Some(format!("manifest:{}", filename));

            for pkg in &found.adopt {
                state_mut.add(
                    &pkg.backend,
                    &pkg.name,
                    pkg.version.clone(),
                    HashMap::new(),
                    source_meta.clone(),
                    false, // Adopted packages are permanent, not transient.
                );
            }

            let state_to_persist = state_mut.clone();
            tokio::task::spawn_blocking(move || state_to_persist.save())
                .await
                .map_err(|e| Error::Other(format!("State-save thread failure: {}", e)))??;
        }

        info!("Migrator: state registry aligned.");

        println!("\nAdopted {} package(s).", found.adopt.len());
        println!("{:-<64}", "");
        println!("Manifest:  {}", manifest_path.display());
        if !found.skipped.is_empty() {
            println!(
                "Left alone: {} (listed in the manifest)",
                found.skipped.len()
            );
        }
        println!("{:-<64}", "");
        println!("This list is an ESTIMATE of what you chose to install — read it.");
        println!("Deleting a line UNINSTALLS that package on the next sync.");
        println!("To stop managing one without uninstalling: linix unmanage <backend>:<name>");

        Ok(())
    }

    /// The manifest, as a string. Split out from the write so it can be tested without a
    /// filesystem, and so the exact words a user is asked to trust are pinned by a test.
    fn render_manifest(&self, found: &Discovery) -> String {
        let mut out = String::new();

        out.push_str(&format!(
            "\
# LiNix adoption manifest — generated {}
#
# WHAT THIS IS
#   LiNix asked every package manager on this machine which packages you chose to
#   install, as opposed to packages that were pulled in automatically to satisfy
#   something else. Their answers are below.
#
# THIS IS AN ESTIMATE
#   Managers vary in how well they track that difference, and some cannot track it
#   at all. This list may name things you never asked for, and may miss things you
#   did. Read it before you rely on it. Each answer's source is named below so you
#   can run the command yourself and disagree.
#
# WHAT HAPPENS NEXT
#   LiNix now manages every package on an uncommented line below.
#   Deleting a line UNINSTALLS that package on the next sync.
#   To stop managing one WITHOUT uninstalling it:
#       linix unmanage <backend>:<name>
#
",
            Local::now().format("%Y-%m-%d %H:%M:%S")
        ));

        out.push_str("# === You installed these ===\n");
        if found.sources.is_empty() {
            out.push_str("#   (no backend could report user intent)\n");
        } else {
            out.push_str("#   Where each answer came from:\n");
            for (backend, source) in &found.sources {
                out.push_str(&format!("#     {:<10} {}\n", backend, source));
            }
        }
        out.push('\n');
        for pkg in &found.adopt {
            out.push_str(&format!("{}:{}\n", pkg.backend, pkg.name));
        }

        if !found.skipped.is_empty() {
            out.push_str(
                "\n\
# === Not adopted: LiNix leaves these alone ===\n\
#   These came back in the answers above, but LiNix will neither manage nor remove\n\
#   them. They stay installed. Listed so you know they exist and why they were\n\
#   skipped. To take one over anyway, add it to `unprotected_packages` in\n\
#   config.toml and run `linix migrate` again.\n\
#\n",
            );
            for s in &found.skipped {
                out.push_str(&format!(
                    "#   {}:{} — {}\n",
                    s.package.backend, s.package.name, s.reason
                ));
            }
        }

        out
    }

    /// A read-only preview of what `migrate` would adopt. Runs the same crawl.
    pub async fn audit(&self) -> Result<Vec<Package>> {
        Ok(self.discover().await?.adopt)
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
        let exec =
            CommandExecutor::with_layer(true, false, mock.clone(), vfs, Arc::new(DashMap::new()));
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
        let names: Vec<String> = m
            .audit()
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, vec!["jq"]);
    }

    #[tokio::test]
    async fn a_skipped_package_is_reported_rather_than_silently_dropped() {
        // Adoption skipping something is the *right* call, but a silent skip leaves the
        // user with a manifest that is quietly incomplete and no way to know why.
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
        let found = m.discover().await.unwrap();

        assert_eq!(found.adopt.len(), 1);
        assert_eq!(found.skipped.len(), 1);
        assert_eq!(found.skipped[0].package.name, "python3");
        assert!(
            found.skipped[0].reason.contains("python3"),
            "the reason must cite the rule that fired, got: {}",
            found.skipped[0].reason
        );
    }

    #[tokio::test]
    async fn the_manifest_warns_that_it_is_an_estimate_and_names_its_source() {
        // The manifest asks the user to trust a guess. These three facts are the whole
        // reason it is safe to do that, so they are pinned rather than left to drift.
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
        let text = m.render_manifest(&m.discover().await.unwrap());

        assert!(text.contains("THIS IS AN ESTIMATE"), "{}", text);
        assert!(text.contains("UNINSTALLS"), "{}", text);
        assert!(text.contains("linix unmanage"), "{}", text);
        // The source of the estimate, so a reader can reproduce it.
        assert!(text.contains("apt-mark showmanual"), "{}", text);
        // Adopted packages are live lines; skipped ones are commented out and explained.
        assert!(text.contains("\napt:jq\n"), "{}", text);
        assert!(
            !text.contains("\napt:python3\n"),
            "a skipped package must not be a live line:\n{}",
            text
        );
        assert!(text.contains("#   apt:python3 — protected"), "{}", text);
    }

    #[tokio::test]
    async fn audit_and_migrate_cannot_disagree() {
        // They were two loops that had already drifted apart on two points. `audit` is now
        // literally `discover().adopt`, so this test would have to be deleted to break it.
        let vfs: Arc<DashMap<PathBuf, String>> = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs));
        mock.set_response(
            "apt-mark showmanual",
            Ok(DryRunOutput {
                stdout: b"jq\nhtop\n".to_vec(),
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
        let m = migrator(reg);
        let audited: Vec<String> = m
            .audit()
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        let discovered: Vec<String> = m
            .discover()
            .await
            .unwrap()
            .adopt
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(audited, discovered);
        // Sorted, because the manifest is a file people diff.
        assert_eq!(audited, vec!["htop", "jq"]);
    }
}
