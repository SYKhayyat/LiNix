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
/// The Adopter asks every backend which packages a person chose to install, and writes
/// the answer out as a manifest. Two properties matter more than anything else here:
///
/// 1. *The answer is an estimate.* Package managers differ in how well they separate a
///    user's choices from packages dragged in as dependencies, and some cannot do it at
///    all. The manifest says so, in those words, and names the command behind each answer
///    so a reader can check it.
/// 2. *Adoption is the dangerous half.* Everything adopted lands in the global state
///    registry, and anything in that registry is a removal candidate on the next sync. An
///    over-broad adoption is not a cosmetic mistake; it is a queued mass removal.
pub struct Adopter {
    registry: Arc<BackendRegistry>,
    state: Arc<Mutex<StateRegistry>>,
    config: Arc<Config>,
}

/// A package that was discovered but deliberately not adopted.
#[derive(Debug, Clone)]
pub struct Skipped {
    pub package: Package,
    /// Why, in words fit to print in the manifest.
    pub reason: String,
}

/// What a discovery crawl found. `adopt` and `audit` share this so the preview cannot
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

impl Adopter {
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
    /// `adopt` and `audit` both go through here. They used to be near-duplicate loops
    /// and had already drifted on two points — one keyed managed-state lookups off the
    /// package's backend and the other off the backend's own name, and one warned on a
    /// backend error while the other swallowed it, so the preview could hide a failure the
    /// real run reported. A preview that does not run the same code is not a preview.
    pub async fn discover(&self) -> Result<Discovery> {
        let mut found = Discovery::default();
        let mut seen_keys = HashSet::new();
        let mut candidates: Vec<Package> = Vec::new();

        // D5: a `.deb`/`.rpm` a download backend handed to a system manager is listed by that
        // manager as manually installed, but a `github:`/`web:` declaration already owns it —
        // so adopt must not offer to manage it a second time. Gathered once, matched by name.
        let mut owned_system: HashSet<String> = HashSet::new();
        for backend in self.registry.available() {
            if let Some(q) = backend.as_queryable() {
                for (_installer, pkg) in q.owned_system_packages().await {
                    owned_system.insert(pkg);
                }
            }
        }

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
                    "backend '{}' cannot distinguish user-chosen packages from \
                     dependencies — skipping adoption. Add its packages to a manifest by \
                     hand if you want them managed.",
                    backend.name()
                );
                continue;
            }

            debug!("probing backend '{}'...", backend.name());

            match queryable.list_manual().await {
                Ok(pkgs) => {
                    found
                        .sources
                        .insert(backend.name().to_string(), queryable.manual_source());
                    let state_guard = self.state.lock().await;
                    for pkg in pkgs {
                        let key = format!("{}:{}", pkg.backend, pkg.name);
                        if !state_guard.is_managed(&pkg.backend, &pkg.name)
                            && !owned_system.contains(&pkg.name)
                            && seen_keys.insert(key.clone())
                        {
                            trace!("candidate: {}", key);
                            candidates.push(pkg);
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        "backend '{}' discovery failed: {}. Continuing crawl.",
                        backend.name(),
                        e
                    );
                }
            }
        }

        // E7/II.9: **protection means one thing — never remove.** Adopt takes every manual
        // package, protected ones included: the line belongs in your file, and deleting it
        // is what the guard then refuses (V.26). Routing adoption through
        // `guard::protection_of` unified the code while keeping the word ambiguous — a
        // package you could not adopt and could not remove, for the same reason, which is
        // two opposite meanings wearing one name.
        //
        // OS-essential is different, and II.9 says what to do with it: a second section,
        // commented out. Base-image packages like `grub-pc` ARE adopted — they keep the
        // machine bootable and `purge-unmanaged` deletes what is not declared — but what
        // the OS itself calls essential is not something to hand someone as a line whose
        // deletion means "uninstall".
        let backends: HashSet<String> = candidates.iter().map(|p| p.backend.clone()).collect();
        let os_essential = guard::essential_names(&self.registry, &backends).await;

        for pkg in candidates {
            let key = format!("{}:{}", pkg.backend, pkg.name);
            if os_essential.contains(&key) {
                found.skipped.push(Skipped {
                    reason: format!("{} reports it as essential to the system", pkg.backend),
                    package: pkg,
                });
            } else {
                found.adopt.push(pkg);
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
    pub async fn adopt(&self) -> Result<()> {
        debug!("scanning for packages to adopt");
        let mut found = self.discover().await?;

        // Before the count is reported, or "3 candidates" is followed by a manifest holding
        // one. II.9: one `modules/adopted.txt`, overwritten. Adopting twice must answer "the
        // machine as it is now" and not "the machine plus history" — a per-run file would
        // declare every package twice, which the resolver refuses as a contradiction
        // (II.7 rule 5).
        let priority = crate::app::sync::resolver::StateResolver::new(
            &self.config,
            self.registry.clone(),
            false,
        )
        .await
        .priority_for_host()
        .await?;
        let vocab = crate::app::vocab::Vocab::new(&self.registry, &self.config, &priority);
        Self::hold_back_what_cannot_be_written(&mut found);

        if found.adopt.is_empty() {
            info!("nothing new to adopt");
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

        info!("{} candidate(s) for adoption.", found.adopt.len());

        let layout = self.config.layout();
        let manifest = self.render_manifest(&found);
        let facts =
            crate::app::sync::StateResolver::new(&self.config, self.registry.clone(), false)
                .await
                .facts_for_host()
                .await?;
        let edit = crate::model::Editor::new(&layout, &vocab, facts)
            .write_module(&crate::model::Landing::Adopted.target(), &manifest)?;
        let manifest_path = edit.file.clone();
        info!("{}", edit.describe("Wrote"));

        {
            let mut state_mut = self.state.lock().await;
            let source_meta = Some("adopt".to_string());

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

        debug!("state registry aligned");

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
    /// Move every candidate whose `backend:name` the grammar cannot read into `skipped`.
    ///
    /// A manager may report something that is not a declarable name: `winget list` answers
    /// for Add/Remove-Programs entries with pseudo-IDs like `ARP\Machine\X64\Android Studio`,
    /// and a package name is one word (II.2). Written out, that line is a parse error in the
    /// file LiNix just generated — and since every later command parses the model, adopting
    /// wedged the whole config until someone hand-edited it.
    ///
    /// Asked through `statement::parse`, not a second copy of the naming rule: whatever the
    /// grammar accepts is exactly what may be written.
    fn hold_back_what_cannot_be_written(found: &mut Discovery) {
        let mut kept = Vec::with_capacity(found.adopt.len());
        for pkg in std::mem::take(&mut found.adopt) {
            if crate::config::grammar::is_declarable(&pkg.backend, &pkg.name) {
                kept.push(pkg);
            } else {
                warn!(
                    "`{}:{}` cannot be written as a package line.",
                    pkg.backend, pkg.name
                );
                found.skipped.push(Skipped {
                    package: pkg,
                    reason: "its manager reports a name no package line can hold".to_string(),
                });
            }
        }
        found.adopt = kept;
        found.skipped.sort_by(|a, b| {
            (&a.package.backend, &a.package.name).cmp(&(&b.package.backend, &b.package.name))
        });
    }

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
# === Found, but left alone ===\n\
#   Commented out on purpose: they are listed so you know they exist, not handed to\n\
#   you as lines whose deletion means \"uninstall\". They stay installed either way.\n\
#   Most are packages the OS calls essential — uncomment one to manage it, and the\n\
#   guard still refuses to remove it unless you put it in `unprotected_packages`.\n\
#   The rest are names their manager reports in a form no line can hold, so there is\n\
#   nothing to uncomment: the reason says which.\n\
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

    /// A read-only preview of what `adopt` would adopt. Runs the same crawl.
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
            binary: None,
            remove_binary: None,
            install_args: vec![],
            remove_args: vec![],
            list_args: vec!["-W".into()],
            manual: ManualListing::AllInstalled,
            essential_args: Some(vec![
                "-W".into(),
                "-f=${Essential} ${Priority} ${Package}\\n".into(),
            ]),
            search_args: vec![],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: Some("dpkg-query".into()),
            upgrade_args: vec![],
            update_args: None,
            purge_args: None,
            orphan_dry_run: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            repo_binary: None,
            repo_list_binary: None,
            depends_args: None,
            version_pin: None,
            needs_root: false,
            is_exclusive: false,
            install_source_option: None,
            flag_map: HashMap::new(),
        };
        config.manual = manual;

        let core = Arc::new(GenericBackendCore {
            name: "apt".into(),
            executor: exec,
            config,
            // apt's real parser: `LambdaParser` has no `parse_essential`, so it inherits
            // the trait default and answers "nothing is essential" — which would make a
            // test about essential packages pass for the wrong reason.
            parser: Arc::new(crate::parsers::apt::AptParser),
        });
        let mut reg = BackendRegistry::new();
        reg.register(Arc::new(
            BackendCapabilities::builder(core.clone())
                .with_queryable(Arc::new(GenericQueryable { core }))
                .build(),
        ));
        Arc::new(reg)
    }

    fn adopter(reg: Arc<BackendRegistry>) -> Adopter {
        let config = Config {
            // Keep the default protected list out of these assertions.
            guard: crate::config::GuardSettings {
                protected_packages: vec![],
                ..Default::default()
            },
            ..Config::default()
        };
        let state = Arc::new(Mutex::new(StateRegistry::default()));
        Adopter::new(reg, state, &config)
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

        let found = adopter(registry_with(ManualListing::Unsupported, mock.clone()))
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
        let names: Vec<String> = adopter(reg)
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
    async fn a_protected_package_is_adopted_like_any_other() {
        // E7/II.9. This test used to assert the opposite — that `protected_packages` kept a
        // package out of the manifest — which made "protected" mean two contradictory
        // things: *never remove* in the guard, *never adopt* here. A package you could not
        // adopt and could not remove, for the same reason.
        //
        // Protection means one thing: never remove. `python3` belongs in your file like
        // everything else you chose, and deleting that line is what the guard refuses
        // (V.26).
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
            guard: crate::config::GuardSettings {
                protected_packages: vec!["python3".into()],
                ..Default::default()
            },
            ..Config::default()
        };
        let m = Adopter::new(reg, Arc::new(Mutex::new(StateRegistry::default())), &config);
        let names: Vec<String> = m
            .audit()
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(
            names,
            vec!["jq", "python3"],
            "adopt must take a protected package: protection stops its REMOVAL, not its \
             adoption"
        );
    }

    #[tokio::test]
    async fn an_os_essential_package_is_reported_rather_than_silently_dropped() {
        // Leaving something out of the manifest is the right call for what the OS itself
        // calls essential (II.9), but a silent skip leaves the user with a list that is
        // quietly incomplete and no way to know why.
        //
        // This test used to manufacture its skip with `protected_packages`, which is E7's
        // confusion: protection stops a REMOVAL, and has nothing to say about adoption.
        let vfs: Arc<DashMap<PathBuf, String>> = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs));
        mock.set_response(
            "apt-mark showmanual",
            Ok(DryRunOutput {
                stdout: b"jq\nbash\n".to_vec(),
                stderr: vec![],
            }
            .into()),
        );
        // The OS's own answer, in dpkg's real format: bash is Essential, jq is not.
        mock.set_response(
            "dpkg-query -W -f=${Essential} ${Priority} ${Package}\\n",
            Ok(DryRunOutput {
                stdout: b"yes required bash\nno optional jq\n".to_vec(),
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
        let m = Adopter::new(
            reg,
            Arc::new(Mutex::new(StateRegistry::default())),
            &Config::default(),
        );
        let found = m.discover().await.unwrap();

        assert_eq!(found.adopt.len(), 1, "jq is adopted");
        assert_eq!(found.adopt[0].name, "jq");
        assert_eq!(found.skipped.len(), 1, "bash is reported, not dropped");
        assert_eq!(found.skipped[0].package.name, "bash");
        assert!(
            found.skipped[0].reason.contains("essential"),
            "the reason must say why, got: {}",
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
        // No `protected_packages` here: protection has nothing to say about the manifest
        // (E7/II.9). What is commented out is what the OS calls essential.
        let m = Adopter::new(
            reg,
            Arc::new(Mutex::new(StateRegistry::default())),
            &Config::default(),
        );
        let text = m.render_manifest(&m.discover().await.unwrap());

        assert!(text.contains("THIS IS AN ESTIMATE"), "{}", text);
        assert!(text.contains("UNINSTALLS"), "{}", text);
        assert!(text.contains("linix unmanage"), "{}", text);
        // The source of the estimate, so a reader can reproduce it.
        assert!(text.contains("apt-mark showmanual"), "{}", text);
        // Every manual package is a live line — a PROTECTED one too, if it were listed:
        // protection stops the removal, not the adoption (V.26). Here both are just manual.
        assert!(text.contains("\napt:jq\n"), "{}", text);
        assert!(text.contains("\napt:python3\n"), "{}", text);
    }

    #[test]
    fn a_name_no_line_can_hold_is_reported_rather_than_written() {
        // `winget list` answers for Add/Remove-Programs entries with pseudo-IDs like
        // `ARP\Machine\X64\Android Studio`. Written into `modules/adopted.txt`, that is a
        // parse error in a file LiNix generated — and since every later command parses the
        // model, one such name wedged the whole config until it was hand-edited. Found by
        // the live Windows sweep, where `rollback` died on adopted.txt:69.

        let mut found = Discovery {
            adopt: vec![
                Package::new("7zip.7zip", "winget"),
                Package::new(r"ARP\Machine\X64\Android Studio", "winget"),
            ],
            ..Default::default()
        };
        Adopter::hold_back_what_cannot_be_written(&mut found);

        assert_eq!(found.adopt.len(), 1, "only the writable name is adopted");
        assert_eq!(found.adopt[0].name, "7zip.7zip");
        assert_eq!(
            found.skipped.len(),
            1,
            "and the other is reported, not dropped"
        );
        assert!(
            found.skipped[0].package.name.contains("Android Studio"),
            "the manifest has to name what it could not take"
        );
    }

    #[tokio::test]
    async fn the_os_essential_section_is_commented_out() {
        // II.9: a second section lists OS-essential packages, commented out — listed so you
        // know they exist, not handed to you as lines whose deletion means "uninstall".
        let vfs: Arc<DashMap<PathBuf, String>> = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs));
        mock.set_response(
            "apt-mark showmanual",
            Ok(DryRunOutput {
                stdout: b"jq\nbash\n".to_vec(),
                stderr: vec![],
            }
            .into()),
        );
        mock.set_response(
            "dpkg-query -W -f=${Essential} ${Priority} ${Package}\\n",
            Ok(DryRunOutput {
                stdout: b"yes required bash\nno optional jq\n".to_vec(),
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
        let m = Adopter::new(
            reg,
            Arc::new(Mutex::new(StateRegistry::default())),
            &Config::default(),
        );
        let text = m.render_manifest(&m.discover().await.unwrap());

        assert!(text.contains("\napt:jq\n"), "jq is a live line:\n{}", text);
        assert!(
            !text.contains("\napt:bash\n"),
            "an OS-essential package must not be a live line:\n{}",
            text
        );
        assert!(
            text.contains("#   apt:bash — "),
            "bash is commented, with a reason:\n{}",
            text
        );
        assert!(text.contains("essential"), "{}", text);
    }

    #[tokio::test]
    async fn audit_and_adopt_cannot_disagree() {
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
        let m = adopter(reg);
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
