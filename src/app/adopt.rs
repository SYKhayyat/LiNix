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

/// The skips, grouped by the reason each one carries.
///
/// The rollup used to be `skipped.len()` under one sentence — *"(listed in the manifest)"* —
/// which was a filter that does not exist, so the reason was wrong for every item it counted,
/// always. A count that explains itself with a reason belonging to none of its inputs is worse
/// than a count with no explanation: it answers the question a reader would otherwise go and
/// look up.
fn by_reason(skipped: &[Skipped]) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for s in skipped {
        *counts.entry(s.reason.as_str()).or_default() += 1;
    }
    let mut out: Vec<(String, usize)> = counts
        .into_iter()
        .map(|(r, n)| (r.to_string(), n))
        .collect();
    // Biggest group first; ties keep the alphabetical order the map gave them, so the same
    // machine prints the same lines twice running.
    out.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    out
}

/// The skip lines a user sees, printed under a total that agrees with them.
fn print_left_alone(skipped: &[Skipped]) {
    if skipped.is_empty() {
        return;
    }
    println!("Left alone: {}", skipped.len());
    for (reason, n) in by_reason(skipped) {
        println!("  {:>5}  {}", n, reason);
    }
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

    /// D5: a `.deb`/`.rpm` a download backend handed to a system manager is listed by that
    /// manager as manually installed, but a `github:`/`web:` declaration already owns it — so
    /// adopt must not offer to manage it a second time. Gathered once, matched by name.
    async fn owned_system_names(
        &self,
        backends: &[Arc<crate::core::BackendCapabilities>],
    ) -> HashSet<String> {
        use futures::stream::StreamExt;
        futures::stream::iter(backends.iter().filter_map(|b| b.as_queryable().cloned()))
            .map(|q| async move { q.owned_system_packages().await })
            .buffer_unordered(self.config.max_parallel.max(1))
            .flat_map(futures::stream::iter)
            .map(|(_installer, pkg)| pkg)
            .collect()
            .await
    }

    /// Each backend's user-chosen packages, as `(backend, how it decided, packages)`.
    ///
    /// Ordered, so what `adopt` offers does not depend on which manager answered first.
    async fn manual_listings(
        &self,
        backends: &[Arc<crate::core::BackendCapabilities>],
    ) -> Vec<(String, String, Vec<Package>)> {
        use futures::stream::StreamExt;
        futures::stream::iter(backends.iter().cloned())
            .map(|backend| async move {
                let queryable = backend.as_queryable()?;
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
                    return None;
                }
                debug!("probing backend '{}'...", backend.name());
                match queryable.list_manual().await {
                    Ok(pkgs) => Some((backend.name().to_string(), queryable.manual_source(), pkgs)),
                    Err(e) => {
                        warn!(
                            "backend '{}' discovery failed: {}. Continuing crawl.",
                            backend.name(),
                            e
                        );
                        None
                    }
                }
            })
            .buffered(self.config.max_parallel.max(1))
            .filter_map(|r| async move { r })
            .collect()
            .await
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
        // Both crawls at once, and both concurrent within themselves. This walked every
        // backend serially, twice, while the identical first question already had a concurrent
        // implementation twenty lines away in `AppContext::owned_system_package_names`. There
        // is nothing shared between one backend's answer and another's.
        let backends = self.registry.available();
        let (owned_system, manual) = tokio::join!(
            self.owned_system_names(&backends),
            self.manual_listings(&backends)
        );

        for (name, source, pkgs) in manual {
            found.sources.insert(name, source);
            let state_guard = self.state.lock().await;
            let managed = state_guard.managed_index();
            for pkg in pkgs {
                let key = format!("{}:{}", pkg.backend, pkg.name);
                if !managed.contains(&(pkg.backend.as_str(), pkg.name.as_str()))
                    && !owned_system.contains(&pkg.name)
                    && seen_keys.insert(key.clone())
                {
                    trace!("candidate: {}", key);
                    candidates.push(pkg);
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
        let os_essential =
            guard::essential_names(&self.registry, &backends, self.config.max_parallel).await;
        let declared = self.declared_outside_the_adoption_manifest().await;

        for pkg in candidates {
            let key = format!("{}:{}", pkg.backend, pkg.name);
            if let Some(file) = declared.get(&key) {
                found.skipped.push(Skipped {
                    reason: format!("you already declare it, in {}", file),
                    package: pkg,
                });
            } else if os_essential.contains(&key) {
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

    /// `backend:name` -> the file that declares it, for every declaration the model holds
    /// outside `adopt`'s own manifest.
    ///
    /// Discovery used to ask the managed-state **registry** and nothing else, so a package
    /// written by hand and not yet synced was offered again — and `adopt` wrote a second
    /// declaration for it, after which deleting the user's own line uninstalled nothing. That
    /// is the direct opposite of the sentence `adopt` prints three lines under its count.
    ///
    /// `modules/adopted.txt` is excluded because `adopt` overwrites it every run (II.9): a line
    /// this command wrote last time must not stop it writing the same line now, or the second
    /// run would answer "the machine plus history" instead of "the machine as it is".
    async fn declared_outside_the_adoption_manifest(&self) -> HashMap<String, String> {
        let resolver = crate::app::sync::resolver::StateResolver::new(
            &self.config,
            self.registry.clone(),
            false,
        )
        .await;
        let model = match resolver.resolve_model().await {
            Ok(m) => m,
            Err(e) => {
                // A config that does not resolve is not a reason to fail `adopt` — adopting is
                // one of the ways out of a broken config. It IS a reason to say what could not
                // be checked, rather than quietly re-enabling the duplication.
                warn!("could not read your declarations, so this run cannot tell whether a discovered package is already declared: {e}");
                println!(
                    "Note: your modules did not resolve ({e}), so packages you have already\n\
                     declared may be listed below. Run `linix check config`."
                );
                return HashMap::new();
            }
        };

        let ours = crate::model::Landing::Adopted.module();
        model
            .packages
            .values()
            .flatten()
            .filter_map(|spec| {
                let origin = spec.options.get("__source")?;
                // `__source` is `path:line`; the line number is not part of the file name.
                let file = origin.rsplit_once(':').map_or(origin.as_str(), |(f, _)| f);
                let stem = std::path::Path::new(file).file_stem()?.to_str()?;
                (stem != ours)
                    .then(|| (format!("{}:{}", spec.backend, spec.name), file.to_string()))
            })
            .collect()
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
        let options = self.adoption_options();
        Self::hold_back_what_cannot_be_written(&mut found, &options);

        if found.adopt.is_empty() {
            info!("nothing new to adopt");
            println!("Nothing to adopt: every package your managers report as user-chosen is");
            println!("already managed, or is protected and deliberately left alone.");
            if !found.skipped.is_empty() {
                println!();
                print_left_alone(&found.skipped);
            }
            return Ok(());
        }

        info!("{} candidate(s) for adoption.", found.adopt.len());

        let layout = self.config.layout();
        let manifest = self.render_manifest(&found, &options);
        let facts =
            crate::app::sync::StateResolver::new(&self.config, self.registry.clone(), false)
                .await
                .facts_for_host()
                .await?;
        let edit = crate::model::Editor::new(
            &layout,
            &vocab,
            facts,
            crate::model::Writes::for_run(self.config.dry_run),
        )
        .write_module(&crate::model::Landing::Adopted.target(), &manifest)?;
        let manifest_path = edit.file.clone();
        info!(
            "{}",
            edit.describe(if self.config.dry_run {
                "Would write"
            } else {
                "Wrote"
            })
        );

        let recorded;
        {
            let mut state_mut = self.state.lock().await;
            let source_meta = Some("adopt".to_string());

            for pkg in &found.adopt {
                // Packages only. A `service:` line resolves to a resource statement, so it is
                // never in the model's package set — recording one here makes it managed with
                // nothing declaring it, and the very next `plan` schedules all 154 for removal
                // and then refuses itself. Resources enter the ledger when `sync` places them.
                if matches!(
                    Self::declared_for(&options, pkg),
                    crate::config::grammar::Declared::Resource(_)
                ) {
                    continue;
                }
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
            recorded = tokio::task::spawn_blocking(move || state_to_persist.save())
                .await
                .map_err(|e| Error::Other(format!("State-save thread failure: {}", e)))??;
        }

        debug!("state registry aligned");

        if !recorded {
            // Both halves of a preview, said once each, at a level the default filter shows.
            // The manifest line used to go out at `info!` and the count as a `println!`, so the
            // sentence a user saw was `Adopted 112 package(s).` about a file that was never
            // written.
            println!("\n[DRY-RUN] would adopt {} package(s).", found.adopt.len());
            println!("{:-<64}", "");
            println!(
                "Manifest that would be written:  {}",
                manifest_path.display()
            );
            println!("Nothing was written and nothing is managed. Run without `--dry-run` to");
            println!("record them, and read the manifest before the next `linix sync`.");
            return Ok(());
        }

        println!("\nAdopted {} declaration(s).", found.adopt.len());
        println!("{:-<64}", "");
        println!("Manifest:  {}", manifest_path.display());
        print_left_alone(&found.skipped);
        println!("{:-<64}", "");
        println!("This list is an ESTIMATE of what you chose to install — read it.");
        println!("Deleting a line UNDOES it on the next sync: a package is uninstalled,");
        println!("a service is stopped and disabled.");
        println!(
            "To stop managing a package without uninstalling: linix unmanage <backend>:<name>"
        );

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
    fn hold_back_what_cannot_be_written(
        found: &mut Discovery,
        options: &HashMap<String, Vec<(String, String)>>,
    ) {
        use crate::config::grammar::Declared;
        let mut kept = Vec::with_capacity(found.adopt.len());
        for pkg in std::mem::take(&mut found.adopt) {
            // A resource is adopted like a package, because it is the same offer: a line you
            // can read, keep or delete. It was held back for a whole release with the reason
            // "a name no package line can hold" — a sentence that was false of all 155 of
            // them, and that only sounded like a policy because it was a parser answer.
            let reason = match Self::declared_for(options, &pkg) {
                Declared::Package(_) | Declared::Resource(_) => {
                    kept.push(pkg);
                    continue;
                }
                Declared::Nothing => {
                    warn!(
                        "`{}:{}` cannot be written as a line.",
                        pkg.backend, pkg.name
                    );
                    "its manager reports a name no line can hold".to_string()
                }
            };
            found.skipped.push(Skipped {
                package: pkg,
                reason,
            });
        }
        found.adopt = kept;
        found.skipped.sort_by(|a, b| {
            (&a.package.backend, &a.package.name).cmp(&(&b.package.backend, &b.package.name))
        });
    }

    /// Each backend's `adoption_options`, by backend name. Asked once: it is a constant per
    /// backend, and the crawl calls it for every candidate.
    fn adoption_options(&self) -> HashMap<String, Vec<(String, String)>> {
        self.registry
            .available()
            .iter()
            .filter_map(|b| {
                let opts = b.as_queryable()?.adoption_options();
                (!opts.is_empty()).then(|| (b.name().to_string(), opts))
            })
            .collect()
    }

    /// What the grammar makes of this candidate, asked with the options its backend needs
    /// written beside it. One call decides both whether it can be declared and how.
    fn declared_for(
        options: &HashMap<String, Vec<(String, String)>>,
        pkg: &Package,
    ) -> crate::config::grammar::Declared {
        let owned = options.get(&pkg.backend).map(Vec::as_slice).unwrap_or(&[]);
        let refs: Vec<(&str, &str)> = owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        crate::config::grammar::declared_as(Some(&pkg.backend), &pkg.name, &refs)
    }

    fn render_manifest(
        &self,
        found: &Discovery,
        options: &HashMap<String, Vec<(String, String)>>,
    ) -> String {
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
#   Not every line is a package. A `service:` line is a running service, and it
#   carries the state it was found in: `@status=running` means keep it running,
#   and says nothing about whether it starts at boot. That is deliberate — the
#   init was asked which services are running, not how they are configured.
#
# THIS IS AN ESTIMATE
#   Managers vary in how well they track that difference, and some cannot track it
#   at all. This list may name things you never asked for, and may miss things you
#   did. Read it before you rely on it. Each answer's source is named below so you
#   can run the command yourself and disagree.
#
# WHAT HAPPENS NEXT
#   LiNix now manages everything on an uncommented line below.
#   Deleting a line UNDOES it on the next sync: a package is UNINSTALLED, and a
#   service is STOPPED AND DISABLED.
#   To stop managing a package WITHOUT uninstalling it:
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
            // The grammar spells the line, so a name that needed quoting is written quoted and
            // a service carries the state it was found in.
            // `hold_back_what_cannot_be_written` has already dropped anything this cannot
            // render, so the fallback is unreachable — and it is the unquoted form rather than
            // a panic, because a manifest missing one line is recoverable and a crash in the
            // middle of writing one is not.
            match Self::declared_for(options, pkg).line() {
                Some(line) => out.push_str(line),
                None => out.push_str(&format!("{}:{}", pkg.backend, pkg.name)),
            }
            out.push('\n');
        }

        if !found.skipped.is_empty() {
            out.push_str(
                "\n\
# === Found, but left alone ===\n\
#   Commented out on purpose: they are listed so you know they exist, not handed to\n\
#   you as lines whose deletion means \"uninstall\". They stay installed either way.\n\
#   Uncommenting one manages it — except where the reason says the name cannot be\n\
#   written as a line, or that you already declare it somewhere else, in which case\n\
#   uncommenting would declare it twice.\n\
#\n",
            );
            // Counted from the reasons the items carry, not from a sentence written here
            // that names the causes it happened to know about when it was written.
            for (reason, n) in by_reason(&found.skipped) {
                out.push_str(&format!("#   {:>5}  {}\n", n, reason));
            }
            out.push_str("#\n");
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
        let text = m.render_manifest(&m.discover().await.unwrap(), &m.adoption_options());

        assert!(text.contains("THIS IS AN ESTIMATE"), "{}", text);
        assert!(text.contains("UNINSTALLED"), "{}", text);
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

        // **Y7 moved the boundary.** A name with a space is written quoted now, so the ARP row
        // that once wedged the config is adopted rather than held back. What is still beyond a
        // line is a name carrying a quote — one line cannot express it, and this is the case
        // that keeps the hold-back alive.
        // A resource is adopted like a package now (ruled 2026-08-03): the whole family, so a
        // `link:` or `setting:` cannot quietly keep the old behaviour after `service:` lost it.
        let options = HashMap::from([(
            "service".to_string(),
            vec![("status".to_string(), "running".to_string())],
        )]);
        let mut found = Discovery {
            adopt: vec![
                Package::new("7zip.7zip", "winget"),
                Package::new(r"ARP\Machine\X64\Android Studio", "winget"),
                Package::new("Some \"Quoted\" Program", "winget"),
                Package::new("AppMgmt", "service"),
                Package::new("/home/u/.vimrc", "link"),
            ],
            ..Default::default()
        };
        Adopter::hold_back_what_cannot_be_written(&mut found, &options);

        assert_eq!(found.adopt.len(), 4, "every writable name is adopted");
        for adopted in [
            "7zip.7zip",
            r"ARP\Machine\X64\Android Studio",
            "AppMgmt",
            "/home/u/.vimrc",
        ] {
            assert!(
                found.adopt.iter().any(|p| p.name == adopted),
                "`{adopted}` can be written, so it is a line and not a comment"
            );
        }
        assert_eq!(
            found.skipped.len(),
            1,
            "and what could not be taken is reported, not dropped"
        );
        assert!(
            found.skipped[0].reason.contains("no line can hold"),
            "the manifest has to name what it could not take: {}",
            found.skipped[0].reason
        );
    }

    #[test]
    fn a_service_is_declared_as_the_state_it_was_found_in() {
        // A bare `service:AppMgmt` means enable AND start, and enabling on Windows rewrites the
        // start type to automatic. The init only reports RUNNING services, so that is the only
        // half that was observed — adopting the other half would reconfigure boot on the first
        // sync after a command whose promise is to describe the machine as it already is.
        let options = HashMap::from([(
            "service".to_string(),
            vec![("status".to_string(), "running".to_string())],
        )]);
        let found = Discovery {
            adopt: vec![
                Package::new("AppMgmt", "service"),
                Package::new("jq", "apt"),
            ],
            ..Default::default()
        };
        let m = Adopter::new(
            Arc::new(BackendRegistry::default()),
            Arc::new(Mutex::new(StateRegistry::default())),
            &Config::default(),
        );
        let text = m.render_manifest(&found, &options);

        assert!(
            text.contains("\nservice:AppMgmt@status=running\n"),
            "the observed state, and only the observed state:\n{text}"
        );
        assert!(
            !text.contains("enabled="),
            "the start type was never looked at, so no line may claim it:\n{text}"
        );
        // A package carries no options, because its name already said everything the listing did.
        assert!(text.contains("\napt:jq\n"), "{text}");
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
        let text = m.render_manifest(&m.discover().await.unwrap(), &m.adoption_options());

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
