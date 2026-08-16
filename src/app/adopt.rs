use crate::config::Config;
use crate::core::{Error, Package, Result, StateRegistry};
use chrono::Local;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, instrument, trace, warn};

/// Adoption: bringing packages that are already installed under Shall's management.
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
    /// The registry **and** `priority`, not the registry alone. `adopt` writes declarations,
    /// and a declaration naming a manager the file excludes is refused on the next read — so
    /// crawling outside `priority` produces a manifest the user cannot sync.
    backends: crate::app::Backends,
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
fn print_skipped_backends(skipped: &[(String, String)]) {
    if skipped.is_empty() {
        return;
    }
    // "Not asked" was accurate while every row here was a backend adoption skipped. A client
    // of another backend's database IS asked and its answer is folded into the owner's, so the
    // header has to describe both — what these rows share is that no declaration came from
    // them, and each says why.
    println!("Nothing taken from: {}", skipped.len());
    for (backend, reason) in skipped {
        println!("  {}: {}", backend, reason);
    }
}

pub fn print_left_alone(skipped: &[Skipped]) {
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
    /// Discovered, but Shall leaves them alone. Reported, never adopted.
    pub skipped: Vec<Skipped>,
    /// Backend name -> how its manual set was determined, for the manifest header.
    pub sources: BTreeMap<String, String>,
    /// Whole backends this run did not ask, and why — one line each rather than one per
    /// package, because the answer is about the backend and not about any of its names.
    pub skipped_backends: Vec<(String, String)>,
}

/// What this `adopt` was asked for.
///
/// A bare `shall adopt` takes the backends that answer [`Queryable::adopted_unasked`], which is
/// all of them except the ones where being on the machine is not evidence anybody chose it.
/// Naming a backend takes that one and only that one, opt-out included.
#[derive(Debug, Clone, Default)]
pub struct AdoptScope {
    /// Backends named on the command line. Empty means "whatever adopt takes unasked".
    pub backends: Vec<String>,
    /// Take only what this machine starts on its own — services set to run at boot rather
    /// than services that happen to be running this minute.
    pub enabled_only: bool,
}

impl AdoptScope {
    fn asked_for(&self, backend: &str) -> bool {
        self.backends.iter().any(|b| b == backend)
    }
}

impl Adopter {
    pub fn new(
        backends: crate::app::Backends,
        state: Arc<Mutex<StateRegistry>>,
        config: &Config,
    ) -> Self {
        Self {
            backends,
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
        scope: &AdoptScope,
    ) -> Vec<(String, String, Vec<Package>)> {
        use futures::stream::StreamExt;
        futures::stream::iter(backends.iter().cloned())
            .map(|backend| async move {
                let queryable = backend.as_queryable()?;
                // Named on the command line beats the default, in both directions: `shall adopt
                // service` takes a backend a bare run leaves alone, and naming any backend at
                // all means the others are not this run's business.
                if !scope.backends.is_empty() {
                    if !scope.asked_for(backend.name()) {
                        return None;
                    }
                } else if !queryable.adopted_unasked() {
                    info!(
                        "not adopting `{}` unless asked: {} — run `shall adopt {}` to take them.",
                        backend.name(),
                        queryable.manual_source(),
                        backend.name(),
                    );
                    return None;
                }
                if scope.enabled_only {
                    return match queryable.list_manual_enabled().await {
                        Ok(Some(pkgs)) => Some((
                            backend.name().to_string(),
                            format!("{} (--enabled-only)", queryable.manual_source()),
                            pkgs,
                        )),
                        // Refused by name rather than quietly widened back to everything: a
                        // filter that silently does nothing is how you adopt 150 services while
                        // believing you asked for the 40 that start at boot.
                        Ok(None) => {
                            warn!(
                                "`{}` cannot say which of its entries this machine starts on                                  its own, so `--enabled-only` has nothing to filter on here —                                  skipping it rather than taking all of them.",
                                backend.name()
                            );
                            None
                        }
                        Err(e) => {
                            warn!("`{}` could not be asked what starts at boot: {e}", backend.name());
                            None
                        }
                    };
                }
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
        self.discover_scoped(&AdoptScope::default()).await
    }

    pub async fn discover_scoped(&self, scope: &AdoptScope) -> Result<Discovery> {
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
        // What Shall uses. An adopted line naming a manager outside `priority` is a line the
        // next command refuses, so offering it would be offering a wedge.
        let backends = self.backends.usable()?;
        let (owned_system, mut manual) = tokio::join!(
            self.owned_system_names(&backends),
            self.manual_listings(&backends, scope)
        );
        // Whether this run sweeps `name` at all: everything, unless the command line named
        // backends, in which case only those.
        let sweeping = |name: &str| scope.backends.is_empty() || scope.asked_for(name);
        for backend in &backends {
            let Some(q) = backend.as_queryable() else {
                continue;
            };
            if scope.backends.is_empty() && !q.adopted_unasked() {
                found.skipped_backends.push((
                    backend.name().to_string(),
                    format!(
                        "{} — `shall adopt {}` takes them",
                        q.manual_source(),
                        backend.name()
                    ),
                ));
                continue;
            }
            // Said once per client, not once per package: forty rows of "pacman already has
            // this one" is noise, and saying nothing about forty declarations that did not
            // appear is worse.
            let owner = crate::backends::shared_database::package_database(backend.name());
            if owner != backend.name()
                && sweeping(backend.name())
                && sweeping(owner)
                && backends.iter().any(|b| b.name() == owner)
            {
                found.skipped_backends.push((
                    backend.name().to_string(),
                    format!(
                        "reads {0}'s package database — the same packages, taken once under {0}",
                        owner
                    ),
                ));
            }
        }

        // **One database, three clients.** `yay` and `paru` read pacman's libalpm database, so
        // all three answer `-Qe` with the same set. The backend that keeps the database is asked
        // first and claims each name; the clients that follow find it taken. Sorted rather than
        // left to whichever backend replied first, because which backend owns a declaration must
        // not depend on the order the answers happened to arrive in.
        manual.sort_by(|a, b| {
            let key = |n: &str| {
                (
                    !crate::backends::shared_database::owns_its_database(n),
                    n.to_string(),
                )
            };
            key(&a.0).cmp(&key(&b.0))
        });

        // **Which client speaks for a package depends on the package** (`J3`). pacman removes
        // an AUR package and cannot put it back, so a `pacman:` line for one is a declaration
        // that cannot be deleted and re-added — which is the whole thing a manifest is for. The
        // helper does both, so it claims the foreign set and pacman claims the rest.
        let foreign =
            crate::backends::shared_database::ForeignSets::probe(self.backends.registry()).await;
        let clients_here: std::collections::HashSet<&str> = manual
            .iter()
            .map(|(n, _, _)| n.as_str())
            .filter(|n| !crate::backends::shared_database::owns_its_database(n))
            .collect();

        for (name, source, pkgs) in &manual {
            found.sources.insert(name.clone(), source.clone());
            let state_guard = self.state.lock().await;
            let managed = state_guard.managed_index();
            for pkg in pkgs {
                // Keyed on the database, not the client. A name claimed here is claimed for
                // every backend that shares the database — including when the claim is a skip,
                // which is why the insert comes before the two filters rather than after them:
                // `install jq` declares `pacman:jq`, so pacman skipped it as already managed,
                // and the two clients then adopted the same jq under their own names.
                let db = crate::backends::shared_database::package_database(&pkg.backend);
                let key = format!("{}:{}", db, pkg.name);
                // The owner stands aside on a package it cannot reinstall, but only when a
                // client that can is answering in this same run — on a box with no helper
                // installed, `pacman:<aur package>` is still the best row there is.
                if pkg.backend == db
                    && foreign.is_foreign(db, &pkg.name)
                    && clients_here
                        .iter()
                        .any(|c| crate::backends::shared_database::package_database(c) == db)
                {
                    continue;
                }
                if !seen_keys.insert(key.clone()) {
                    continue;
                }
                // Managed under **any** client of this database, not under this one. A
                // `pacman:jq` an earlier run declared is jq declared, and asking about
                // `yay:jq` would adopt a second line for the same package — which is the
                // duplicate this whole relation exists to stop.
                let already = managed.iter().any(|(b, n)| {
                    *n == pkg.name.as_str()
                        && crate::backends::shared_database::package_database(b) == db
                });
                if !already && !owned_system.contains(&pkg.name) {
                    trace!("candidate: {}", key);
                    candidates.push(pkg.clone());
                }
            }
        }

        // E7/II.9: **protection means one thing — never remove.** Adopt takes every manual
        // package, protected and OS-essential ones included: the line belongs in your file,
        // and deleting it is what the guard then refuses (V.26). Routing adoption through
        // `guard::protection_of` unified the code while keeping the word ambiguous — a
        // package you could not adopt and could not remove, for the same reason, which is
        // two opposite meanings wearing one name.
        let declared = self.declared_outside_the_adoption_manifest().await;

        for pkg in candidates {
            let key = format!("{}:{}", pkg.backend, pkg.name);
            if let Some(file) = declared.get(&key) {
                found.skipped.push(Skipped {
                    reason: format!("you already declare it, in {}", file),
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
            self.backends.registry().clone(),
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
                // stderr, not stdout: `discover` is also what `check --json` calls, and this
                // note landing in front of the document made the answer unparseable on exactly
                // the machines least able to spare it. The note is for a person either way.
                eprintln!(
                    "Note: your modules did not resolve ({e}), so packages you have already\n\
                     declared may be listed below. Run `shall check config`."
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
                let origin = spec.options.one("__source")?;
                // `__source` is `path:line`; the line number is not part of the file name.
                let file = origin.rsplit_once(':').map_or(origin, |(f, _)| f);
                let stem = std::path::Path::new(file).file_stem()?.to_str()?;
                (stem != ours)
                    .then(|| (format!("{}:{}", spec.backend, spec.name), file.to_string()))
            })
            .collect()
    }

    /// Discovery -> manifest -> acquisition.
    #[instrument(skip(self))]
    pub async fn adopt(&self) -> Result<()> {
        self.adopt_scoped(&AdoptScope::default()).await
    }

    pub async fn adopt_scoped(&self, scope: &AdoptScope) -> Result<()> {
        debug!("scanning for packages to adopt");
        let mut found = self.discover_scoped(scope).await?;

        // Before the count is reported, or "3 candidates" is followed by a manifest holding
        // one. II.9: one `modules/adopted.txt`, overwritten. Adopting twice must answer "the
        // machine as it is now" and not "the machine plus history" — a per-run file would
        // declare every package twice, which the resolver refuses as a contradiction
        // (II.7 rule 5).
        let priority = crate::app::sync::resolver::StateResolver::new(
            &self.config,
            self.backends.registry().clone(),
            false,
        )
        .await
        .priority_for_host()
        .await?;
        let vocab =
            crate::app::vocab::Vocab::new(self.backends.registry(), &self.config, &priority);
        let options = self.adoption_options();
        Self::hold_back_what_cannot_be_written(&mut found, &options);

        if found.adopt.is_empty() {
            info!("nothing new to adopt");
            println!("Nothing to adopt: every package your managers report as user-chosen is");
            println!("already managed, or is protected and deliberately left alone.");
            if !found.skipped.is_empty() {
                println!();
                print_left_alone(&found.skipped);
                print_skipped_backends(&found.skipped_backends);
            }
            return Ok(());
        }

        info!("{} candidate(s) for adoption.", found.adopt.len());

        let layout = self.config.layout();
        let manifest = self.render_manifest(&found, &options);
        let facts = crate::app::sync::StateResolver::new(
            &self.config,
            self.backends.registry().clone(),
            false,
        )
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
            let source_meta = "adopt";

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
                    Default::default(),
                    source_meta,
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
            println!();
            crate::would_print!("would adopt {} package(s).", found.adopt.len());
            println!("{:-<64}", "");
            println!(
                "Manifest that would be written:  {}",
                manifest_path.display()
            );
            println!("Nothing was written and nothing is managed. Run without `--dry-run` to");
            println!("record them, and read the manifest before the next `shall sync`.");
            return Ok(());
        }

        println!("\nAdopted {} declaration(s).", found.adopt.len());
        println!("{:-<64}", "");
        println!("Manifest:  {}", manifest_path.display());
        print_left_alone(&found.skipped);
        print_skipped_backends(&found.skipped_backends);
        println!("{:-<64}", "");
        println!("This list is an ESTIMATE of what you chose to install — read it.");
        println!("Deleting a line UNDOES it on the next sync: a package is uninstalled,");
        println!("a service is stopped and disabled.");
        println!(
            "To stop managing a package without uninstalling: shall unmanage <backend>:<name>"
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
    /// file Shall just generated — and since every later command parses the model, adopting
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
                // **V.113 has two halves and this function only ever asked one of them.** A
                // name is admitted by a grammar **and** a validator, and the two must agree —
                // so a name the grammar can write and the validator refuses gets adopted into
                // `adopted.txt` and wedges the model on the next read. That is what happened to
                // 340 winget rows when the grammar was widened and the validator was not.
                //
                // Asked *after* the grammar, not before it: a name no line can hold has a
                // better sentence already, and the gap this closes is specifically the one
                // between the two gates.
                Declared::Package(_) | Declared::Resource(_) => {
                    match crate::core::Validator::validate_package_name_for(&pkg.name, &pkg.backend)
                    {
                        Ok(()) => {
                            kept.push(pkg);
                            continue;
                        }
                        Err(e) => {
                            warn!("`{}:{}` cannot be declared: {}", pkg.backend, pkg.name, e);
                            format!("its manager reports a name Shall refuses to declare: {e}")
                        }
                    }
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
        // Unwrapped to empty on purpose: this is a *decoration* pass over candidates the crawl
        // already produced, and the crawl above refuses first if `priority` will not resolve.
        // A second refusal here would report the same failure twice.
        self.backends
            .usable()
            .unwrap_or_default()
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
# Shall adoption manifest — generated {}
#
# WHAT THIS IS
#   Shall asked every package manager on this machine which packages you chose to
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
#   Shall now manages everything on an uncommented line below.
#   Deleting a line UNDOES it on the next sync: a package is UNINSTALLED, and a
#   service is STOPPED AND DISABLED.
#   Except where the guard refuses. A package you protected, or one the OS itself
#   calls essential, is declared here so Shall keeps it — deleting its line stops
#   Shall keeping it, and the guard still refuses to remove it.
#   To stop managing a package WITHOUT uninstalling it:
#       shall unmanage <backend>:<name>
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
    use crate::backends::generic::SearchSource;
    use crate::backends::BackendRegistry;

    /// A [`Backends`](crate::app::Backends) over every backend the fixture registered, so these
    /// tests are about *adoption* and not about `priority`.
    ///
    /// Spelled out here rather than shipped as a `Backends::everything()` constructor: a type
    /// whose whole job is the priority gate must not offer a way to open it, or the day someone
    /// needs one in production there will be one to reach for.
    fn over(reg: Arc<BackendRegistry>) -> crate::app::Backends {
        let names = reg.all().iter().map(|b| b.name().to_string()).collect();
        crate::app::Backends::new(
            reg,
            Ok(crate::model::priority::Priority::from_backends(names)),
        )
    }

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
            foreign_args: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            repo_binary: None,
            repo_list_binary: None,
            repo_remove_binary: None,
            repo_list_shape: crate::backends::generic::RepoListing::Columns,
            depends: None,
            clean_cache: None,
            version_pin: None,
            needs_root: false,
            is_exclusive: false,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: None,
            search_source: SearchSource::Command,
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
        Adopter::new(over(reg), state, &config)
    }

    #[tokio::test]
    async fn adopts_nothing_from_a_backend_that_cannot_report_intent() {
        // The safety backstop. `dpkg-query -W` is wired and would happily return an entire
        // dependency graph; because the backend admits it cannot tell user-chosen packages
        // from dependencies, adoption must skip it entirely rather than adopt all of it.
        let vfs: Arc<DashMap<PathBuf, String>> = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs));
        mock.set_response_that_must_not_be_used(
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
        mock.set_response_that_must_not_be_used(
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
        let m = Adopter::new(
            over(reg),
            Arc::new(Mutex::new(StateRegistry::default())),
            &config,
        );
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
    async fn an_os_essential_package_is_adopted_like_any_other() {
        // II.9, ruled 2026-08-05: adoption is a claim that Shall keeps the thing, and the
        // guard is what refuses to remove it. Commenting these lines out left Shall with no
        // opinion at all about the packages that matter most — nothing declared `bash`, so
        // nothing would put it back.
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
        // `Q47` ruled that adoption does not consult the guard, so this stub going unused *is*
        // the proof of that ruling — and nothing checked it until it was registered as one that
        // must not be used. Its content is dpkg's real format: bash is Essential, jq is not.
        mock.set_response_that_must_not_be_used(
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
            over(reg),
            Arc::new(Mutex::new(StateRegistry::default())),
            &Config::default(),
        );
        let found = m.discover().await.unwrap();

        let names: Vec<&str> = found.adopt.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["bash", "jq"],
            "the OS calling `bash` essential is a reason to declare it, not to skip it"
        );
        assert!(
            found.skipped.is_empty(),
            "nothing is left alone for being essential: {:?}",
            found.skipped
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
            over(reg),
            Arc::new(Mutex::new(StateRegistry::default())),
            &Config::default(),
        );
        let text = m.render_manifest(&m.discover().await.unwrap(), &m.adoption_options());

        assert!(text.contains("THIS IS AN ESTIMATE"), "{}", text);
        assert!(text.contains("UNINSTALLED"), "{}", text);
        assert!(text.contains("shall unmanage"), "{}", text);
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
        // parse error in a file Shall generated — and since every later command parses the
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
            over(Arc::new(BackendRegistry::default())),
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
    async fn an_os_essential_package_is_a_live_line() {
        // II.9, ruled 2026-08-05. The commented-out section guarded against a deletion the
        // guard already refuses (V.26) — and bought that redundancy by leaving the machine's
        // load-bearing packages undeclared, so drift in them was invisible.
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
        // `Q47` ruled that adoption does not consult the guard, so this stub going unused *is*
        // the proof of that ruling — and nothing checked it until it was registered as one that
        // must not be used. Its content is dpkg's real format: bash is Essential, jq is not.
        mock.set_response_that_must_not_be_used(
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
            over(reg),
            Arc::new(Mutex::new(StateRegistry::default())),
            &Config::default(),
        );
        let text = m.render_manifest(&m.discover().await.unwrap(), &m.adoption_options());

        assert!(text.contains("\napt:jq\n"), "jq is a live line:\n{}", text);
        assert!(
            text.contains("\napt:bash\n"),
            "an OS-essential package is a live line too:\n{}",
            text
        );
        assert!(
            !text.contains("#   apt:bash — "),
            "and it is not also commented out:\n{}",
            text
        );
        // The header must not promise a deletion the guard will refuse.
        assert!(
            text.contains("Except where the guard refuses"),
            "the header owes the reader the exception:\n{}",
            text
        );
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

    /// One backend of an Arch machine's three, all reading the same libalpm database and so
    /// all answering `-Qe` with the same lines.
    fn alpm_client(
        reg: &mut BackendRegistry,
        exec: &CommandExecutor,
        name: &'static str,
        installed_fn: fn(&str) -> crate::parsers::ParseResult,
        search_fn: fn(&str) -> Vec<crate::core::Package>,
    ) {
        let core = Arc::new(GenericBackendCore {
            name: name.into(),
            executor: exec.clone(),
            config: ManagerConfig {
                list_args: vec!["-Q".into()],
                manual: ManualListing::Command {
                    binary: None,
                    args: vec!["-Qe".into()],
                    format: ManualFormat::SameAsInstalled,
                },
                ..crate::backends::registry::base_config(name)
            },
            parser: Arc::new(crate::parsers::LambdaParser {
                installed_fn,
                search_fn,
            }),
        });
        reg.register(Arc::new(
            BackendCapabilities::builder(core.clone())
                .with_queryable(Arc::new(GenericQueryable { core }))
                .build(),
        ));
    }

    /// `order` is the order the backends are registered in, which is the order they answer in.
    fn alpm_registry(mock: Arc<MockExecutor>, order: [&'static str; 3]) -> Arc<BackendRegistry> {
        let vfs: Arc<DashMap<PathBuf, String>> = Arc::new(DashMap::new());
        let exec =
            CommandExecutor::with_layer(true, false, mock.clone(), vfs, Arc::new(DashMap::new()));
        let mut reg = BackendRegistry::new();
        for name in order {
            mock.set_command_exists(name, true);
            // The same three lines from all three, because there is one database under them.
            mock.set_response(
                &format!("{} -Qe", name),
                Ok(DryRunOutput {
                    stdout: b"bash 5.2.037-1\njq 1.8.2-1\nyay 13.0.1-1\n".to_vec(),
                    stderr: vec![],
                }
                .into()),
            );
            match name {
                "pacman" => alpm_client(
                    &mut reg,
                    &exec,
                    "pacman",
                    |o| crate::parsers::pacman::parse_list_for(o, "pacman"),
                    |o| crate::parsers::pacman::parse_search_for(o, "pacman"),
                ),
                "yay" => alpm_client(
                    &mut reg,
                    &exec,
                    "yay",
                    |o| crate::parsers::pacman::parse_list_for(o, "yay"),
                    |o| crate::parsers::pacman::parse_search_for(o, "yay"),
                ),
                _ => alpm_client(
                    &mut reg,
                    &exec,
                    "paru",
                    |o| crate::parsers::pacman::parse_list_for(o, "paru"),
                    |o| crate::parsers::pacman::parse_search_for(o, "paru"),
                ),
            }
        }
        Arc::new(reg)
    }

    /// Three clients of one database must adopt each package once, not once each.
    ///
    /// Measured on the arch image before this held: 20 installed packages became 60
    /// declarations, and the `uninstall jq` that followed planned three removals — pacman
    /// removed jq and the two clients were then asked to remove a package that was gone,
    /// which is `error: target not found: jq` and a sync that cannot converge again.
    #[tokio::test]
    async fn three_clients_of_one_package_database_adopt_each_package_once() {
        let vfs: Arc<DashMap<PathBuf, String>> = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs));
        let found = adopter(alpm_registry(mock, ["pacman", "yay", "paru"]))
            .discover()
            .await
            .unwrap();
        let keys: Vec<String> = found
            .adopt
            .iter()
            .map(|p| format!("{}:{}", p.backend, p.name))
            .collect();
        assert_eq!(keys, vec!["pacman:bash", "pacman:jq", "pacman:yay"]);
    }

    /// And the backend that keeps the database claims them however the answers are ordered.
    /// Registration order is the order the crawl replies in, and which backend owns a
    /// declaration must not depend on it.
    #[tokio::test]
    async fn the_database_owner_claims_the_name_whichever_client_answers_first() {
        let vfs: Arc<DashMap<PathBuf, String>> = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs));
        let found = adopter(alpm_registry(mock, ["yay", "paru", "pacman"]))
            .discover()
            .await
            .unwrap();
        let backends: Vec<&str> = found.adopt.iter().map(|p| p.backend.as_str()).collect();
        assert_eq!(backends, vec!["pacman", "pacman", "pacman"]);
    }

    /// A package already declared under the database's owner is not re-adopted under a client.
    ///
    /// This is the exact shape the container hit: `install jq` writes `pacman:jq`, so pacman
    /// skipped it as already managed — and the two clients, filtered separately, adopted the
    /// same jq under their own names. The name has to be claimed by the skip as well as by
    /// the take.
    #[tokio::test]
    async fn a_name_the_owner_already_manages_is_not_adopted_under_a_client() {
        let vfs: Arc<DashMap<PathBuf, String>> = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs));
        let reg = alpm_registry(mock, ["pacman", "yay", "paru"]);
        let config = Config {
            guard: crate::config::GuardSettings {
                protected_packages: vec![],
                ..Default::default()
            },
            ..Config::default()
        };
        let mut state = StateRegistry::default();
        state.add("pacman", "jq", None, Default::default(), "test", false);
        let m = Adopter::new(over(reg), Arc::new(Mutex::new(state)), &config);
        let keys: Vec<String> = m
            .discover()
            .await
            .unwrap()
            .adopt
            .iter()
            .map(|p| format!("{}:{}", p.backend, p.name))
            .collect();
        assert_eq!(keys, vec!["pacman:bash", "pacman:yay"]);
    }
}
