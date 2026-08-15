//! What this machine has installed, asked of every manager the model uses.
//!
//! These were methods on `App`, where each of them could reach all twelve of its collaborators
//! while none used more than four. The fan-out (`query_backends_concurrently`) is the reason
//! they are one type rather than four: `list`, `info`, the unmanaged crawl and the warm-up must
//! not drift in how they bound concurrency or in what they do with a manager that fell over.

use crate::app::sync::resolver::StateResolver;
use crate::app::{Backends, UniversalSearch};
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::core::{Error, Package, Result, StateRegistry};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::warn;

/// What the unmanaged crawl found, and what it could not find out.
///
/// **Two fields, for the reason `OutdatedReport` has two.** A manager that could not be listed
/// contributes nothing, which is the safe direction for `purge-undeclared` — it deletes less,
/// never more. It is not a safe direction for the *sentence*: "nothing here is unmanaged" and
/// "three managers never answered" produced the identical empty vector, and `shall check drift`
/// printed *"System matches your manifests; nothing to install, no drift, nothing undeclared"*
/// over the second one (B4).
///
/// The asymmetry this closes is sharper than the omission: `check drift --json` already had a
/// `resources_unverifiable` key and no packages equivalent, so the distinction this codebase is
/// proudest of survived into the machine-readable contract for one half of the model and was
/// lost for the other.
#[derive(Default)]
pub struct UndeclaredReport {
    /// Installed, and nothing declares it.
    pub packages: Vec<Package>,
    /// `backend: reason` for every manager that could not be listed. Never empty *and* silent.
    pub unanswered: Vec<String>,
    /// The managers [`packages`](Self::packages) is a complete answer *about*: named by
    /// `priority`, installed here, and they answered.
    ///
    /// **Carried because a count of this list is half of a ratio, and the other half must be
    /// counted over the same managers.** `purge-undeclared` weighs what Shall manages against
    /// what it is about to delete; the deletion side is this narrow by design, so a management
    /// side counted over every backend in the state file compares two different machines. That
    /// mismatch is not academic — it silently inflates the ratio, which is the direction that
    /// *withdraws* the refusal.
    pub answered: Vec<String>,
}

/// Inventory holds only what it uses. It is built from an [`App`](crate::app::App) by
/// `App::inventory()` and can be built without one.
pub struct Inventory<'a> {
    pub(crate) config: &'a Arc<Config>,
    pub(crate) registry: &'a Arc<BackendRegistry>,
    pub(crate) state: &'a Arc<Mutex<StateRegistry>>,
    /// The registry paired with `priority` — what Shall may use, already resolved.
    pub(crate) backends: &'a Backends,
}

impl Inventory<'_> {
    /// A resolver over the same config and registry, for the questions about what a *name*
    /// means. One per call rather than one per question: `info` asked four and built four.
    async fn resolver(&self) -> StateResolver<'_> {
        StateResolver::new(self.config, self.registry.clone(), false).await
    }

    pub async fn list(&self, backend_filter: Option<&str>) -> Result<Vec<Package>> {
        // Narrowed by the registry, not after it: filtering the result of `available()` means
        // fifty-two PATH walks to answer a question about one manager (see `available_named`).
        // What Shall uses, narrowed to one when `--backend` named one. `list` reports on the
        // model's managers; a manager `priority` excludes has nothing to do with this machine's
        // declarations, and probing it to find that out is the cost W4 is about.
        let backends = self.backends.usable_named(backend_filter)?;
        // The fan-out drops backends that cannot be queried at all, so the names have to be
        // taken through the same filter to stay aligned with the answers.
        let names: Vec<String> = backends
            .iter()
            .filter(|b| b.as_queryable().is_some())
            .map(|b| b.name().to_string())
            .collect();
        // Every backend's lister is a separate process (`apt list`, `cargo install --list`,
        // …) with nothing to share, so querying them one after another is latency the machine
        // is not spending — it is waiting. Fan out, bounded by `max_parallel`.
        let results = self
            .query_backends_concurrently(backends, |q| async move { q.list_installed().await })
            .await;

        // `unwrap_or_default()` stood here, and it is how `shall list --backend winget` printed
        // nothing and exited 0 on a machine with 280 winget packages on it: the manager fell
        // over, its rows became an empty vector, and the empty vector became the answer.
        // Measured 1 run in 16 under concurrent cold start. A listing missing a manager is a
        // different thing from a machine missing its packages, and the user has to be able to
        // tell which one they are holding.
        let mut rows = Vec::new();
        let mut unlisted = Vec::new();
        for (name, result) in names.into_iter().zip(results) {
            match result {
                Ok(pkgs) => rows.extend(pkgs),
                Err(e) => unlisted.push((name, e)),
            }
        }
        if !unlisted.is_empty() {
            // One manager was asked about, so its failure *is* the answer — there is no
            // partial listing to hand back, only an empty one that would read as "you have
            // none of these".
            if backend_filter.is_some() {
                let (name, e) = unlisted.remove(0);
                return Err(Error::command_failed(format!(
                    "`{name}` could not be listed, so Shall cannot tell you what it has: {e}"
                )));
            }
            // Listing everything: one unwell manager must not take the other twenty-three
            // with it, but it must not pass quietly either.
            for (name, e) in &unlisted {
                warn!(
                    "`{name}` could not be listed — anything it has is missing from this \
                     listing: {e}"
                );
            }
        }
        // One installed package is one row, however many clients of its database answered.
        // Three pacman clients on an Arch box turn 203 packages into 609 lines, each triple
        // identical but for the backend column.
        Ok(crate::backends::capability::one_row_per_shared_database(
            rows,
        ))
    }

    pub async fn get_info(&self, package_name: &str) -> Result<Option<Package>> {
        // An explicit `backend:name` narrows the question to one manager. This used to hand
        // the raw string to every backend, so `shall info cargo:ripgrep` asked each of them
        // for a package literally named "cargo:ripgrep" — a name none of them has. That is
        // both the wrong question and the slow one: every manager was probed, and the answer
        // was always "not found", while `shall search ripgrep` in the same tree found it.
        //
        // Split by the one parser (`resolve_spec`, which goes through the grammar), never by
        // `split_once(':')` here — a second place that decides what a prefix means is the bug
        // CLAUDE.md names, and C13 records six parsers that had it.
        let resolver = self.resolver().await;
        // Does the string name a manager, and is it one? Asked first, and refused with the
        // sentence `install` and `list --backend` already use — from the same function, so
        // there is one answer to one question. `info nosuchbackend:foo` used to reach the
        // fan-out below and ask every manager on the machine for a package literally named
        // `nosuchbackend:foo`: the wrong answer ("not installed" — the *manager* does not
        // exist), arrived at slowly, at exit 0 (N-3).
        // Note the `?`. The grammar is what rejects an unknown prefix — `parse_prefix` writes
        // the sentence — and every version of this bug has been someone dropping that error on
        // the floor: `get_info` had `if let Ok(specs) = resolve_spec(…)`, and the refusal it
        // discarded was the answer.
        let named_backend = resolver.declared_backend(package_name).await?;
        // The registry's half of the same question: a name `priority` lists and this build has
        // no backend for.
        resolver.require_known_backend(named_backend.as_deref())?;

        // `service:`, `link:` and `setting:` are each a grammar prefix AND a registered
        // backend, and `list` prints them as those two columns — so a string copied out of a
        // listing parses as a typed resource statement rather than as `backend:name`, and
        // everything below understands only packages. `list` reported
        // `service:com.apple.SafariHistoryServiceAgent` and `info` about that exact name said
        // "not installed" (R-4). A list that disagrees with the machine breaks the one thing it
        // promises, so this is answered before the package path rather than after it.
        if let Ok(Some((backend_name, resource))) = resolver.queried_resource(package_name).await {
            // A resource name off a `list` row becomes the queried manager's argv with no
            // package-name rule in front of it — the same hole as the bare-name fallback
            // below, through the door `list` invites the user to copy from.
            crate::core::Validator::refuse_command_metacharacters(&resource, "a resource name")?;
            if let Some(backend) = self.registry.get(&backend_name) {
                if let Some(q) = backend.as_queryable() {
                    // A resource the backend does not have answers `None`, the same as any
                    // other name it does not carry — the point is that it was *asked*.
                    return q.info(&resource).await;
                }
            }
        }

        // A resolution failure is not fatal here — `info` answers about the machine, and a name
        // no manager *carries* can still be installed on it — but it is not nothing either, and
        // `unwrap_or_default()` made it nothing.
        //
        // The two halves of this function need opposite things from a failed resolve. A **bare**
        // name survives it: the fan-out at the bottom asks every manager anyway and already
        // refuses to say "absent" while any of them is unanswered. A name that **named its
        // manager** does not: the branch below returns `Ok(None)`, which `info` prints as *"is
        // not installed on this machine"* — a claim about the user's machine, made without
        // asking anybody, out of an error nobody read. Thirteen lines further down sits the
        // comment that fixed this exact class for the query; the resolve above it was still
        // swallowing (Q36's sibling, one call earlier).
        let (specs, resolve_failed) = match resolver.resolve_spec(package_name).await {
            Ok(specs) => (specs, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        for spec in &specs {
            let Some(backend) = self.registry.get(&spec.backend) else {
                continue;
            };
            let Some(q) = backend.as_queryable() else {
                continue;
            };
            // A manager that could not be asked has not said no. Dropping the error here made
            // `info` print "is not installed on this machine" — a claim about the user's
            // machine — whenever a manager fell over, and a `winget list` that fails under
            // concurrent load does exactly that, silently (Q36's sibling). Absence and
            // unavailability are different answers and only one of them is knowable.
            match q.info(&spec.name).await {
                Ok(Some(found)) => return Ok(Some(found)),
                Ok(None) => {}
                Err(e) => return Err(e),
            }
        }

        // The user named the manager, and it does not have it. Asking a different one would
        // answer a question nobody asked — `info cargo:ripgrep` must never report the choco
        // copy.
        if let Some(backend) = &named_backend {
            // **Unless nobody was asked.** With a named manager, `specs` is empty only when the
            // resolve failed — a `priority` that refuses the backend, a name the validator
            // rejects, a `@requires` chain that could not be probed — so the loop above ran
            // zero times and this `Ok(None)` would be `info` reporting on a manager it never
            // spoke to. That is the same category error the loop's own comment names, one
            // branch earlier: absence and unavailability are different answers, and only one
            // of them is knowable.
            if let Some(e) = resolve_failed {
                return Err(Error::command_failed(format!(
                    "Shall could not work out what `{package_name}` refers to, so it cannot \
                     tell you whether `{backend}` has it: {e}"
                )));
            }
            return Ok(None);
        }

        // A bare name: *which* manager has it installed is a fact about this machine, and
        // `priority` order is not that fact. The resolver picks by priority, so `info hexyl`
        // asked `choco` (first in `priority`, and it carries the name), choco had nothing
        // installed, and Shall reported a package the user has under `cargo` as absent — while
        // `list` reported it present. Two read commands must never contradict each other about
        // the machine.
        //
        // Asked of every backend at once, and the first answer wins. Serial, this waited on
        // every manager that did not have it before reaching the one that did.
        // The fallback is the one branch of this function that asks every manager about a
        // string nothing validated: `specs` is empty exactly when the resolve failed, and the
        // resolve is where the character check lives. So a name refused as a name was handed
        // to 22 managers anyway, and on Windows some of them are `.cmd` shims (B-1).
        let name = match specs.first() {
            Some(s) => s.name.clone(),
            None => {
                crate::core::Validator::refuse_command_metacharacters(
                    package_name,
                    "a package name",
                )?;
                package_name.to_string()
            }
        };
        // What Shall uses. `info` answers about a package this machine is managing, and a
        // manager outside `priority` cannot be managing one.
        let backends = self.backends.usable()?;
        let answers = self
            .query_backends_concurrently(backends, move |q| {
                let name = name.clone();
                async move { q.info(&name).await }
            })
            .await;
        // In registry order, so the first manager that *has* it still wins and the fan-out
        // stays the only thing deciding that.
        let mut unanswered = Vec::new();
        // The bare-name path survives a failed resolve — every manager is asked below either
        // way — but it does not get to *forget* it. If the fan-out also comes back empty, the
        // resolve failure is part of why nobody answered, and a run that reports "absent" while
        // holding an unread error is the shape this whole function keeps producing.
        if let Some(e) = resolve_failed {
            unanswered.push(format!("resolving `{package_name}`: {e}"));
        }
        for answer in answers {
            match answer {
                Ok(Some(found)) => return Ok(Some(found)),
                Ok(None) => {}
                Err(e) => unanswered.push(e.to_string()),
            }
        }
        // Nobody has it — but "nobody has it" is only true if everybody was asked. A manager
        // that fell over contributes silence, and silence read as a "no" is how `info` reports
        // an installed package as absent. `.ok().flatten()` here did precisely that.
        if !unanswered.is_empty() {
            return Err(Error::command_failed(format!(
                "no manager reported `{package_name}`, but {} could not be asked, so Shall \
                 cannot tell you it is absent:\n  {}",
                if unanswered.len() == 1 {
                    "one of them".to_string()
                } else {
                    format!("{} of them", unanswered.len())
                },
                unanswered.join("\n  ")
            )));
        }
        Ok(None)
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Package>> {
        UniversalSearch::new(self.backends, self.config)
            .search(query)
            .await
    }

    /// Ask every ready manager what it has installed, all at once, before anything needs it.
    ///
    /// For a command that reports on the whole machine this changes no question and no answer —
    /// every one of these listings is asked during the run regardless, and the memo means each
    /// is fetched once either way. What changes is *when*: `check` plans drift, then crawls for
    /// unmanaged packages, then probes health, and each stage asks the managers it needs at the
    /// moment it needs them. Measured on a 298-package config, that put nine managers — gem,
    /// pip, emacs, luarocks, dotnet, dart, nimble, bun, service — at the starting line 5.4 s
    /// into a 9.1 s run, idle until then, because the crawl that wanted them was queued behind
    /// a plan that did not.
    ///
    /// **Only for commands that already ask everyone.** A command that consults three managers
    /// must not be made to wake twenty-four; that is why this is called by name at those call
    /// sites rather than folded into `App::new`.
    pub async fn warm_installed(&self) {
        // What Shall uses — and a warm-up must not be the one thing that widens the set: it
        // exists so the *later* fan-outs find their answers cached, so it has to ask exactly
        // who they will ask. A priority that will not resolve warms nothing rather than
        // everything; the command that needs the answer reports the failure itself.
        let Ok(backends) = self.backends.usable() else {
            return;
        };
        let _ =
            self.query_backends_concurrently(backends, |q| async move {
                q.list_installed().await.is_ok()
            })
            .await;
    }

    /// Everything installed that Shall does not manage — the dependency closure included.
    ///
    /// **This is not `unmanaged` (II.8), which is "what `adopt` would adopt".** They are two
    /// questions with very different answers: on a stock Ubuntu this is ~476 packages and
    /// `adopt` is ~103. Only `purge-undeclared` wants this one, and its whole job is deleting
    /// all of it (II.11) — which is why the ratio check exists.
    pub async fn installed_but_undeclared(&self) -> Result<UndeclaredReport> {
        // What Shall uses. `purge-undeclared` deletes from this list, so widening it past
        // `priority` would delete through managers the user told Shall not to touch.
        let backends = self.backends.usable()?;
        let names: Vec<String> = backends
            .iter()
            .filter(|b| b.as_queryable().is_some())
            .map(|b| b.name().to_string())
            .collect();
        let answers = self
            .query_backends_concurrently(backends, |q| async move { q.list_installed().await })
            .await;
        // A manager that could not be listed contributes nothing, which is the safe direction
        // for the caller that deletes — `purge-undeclared` removes less, never more. It is not
        // a safe direction for the *sentence*: "nothing here is unmanaged" and "one manager
        // never answered" both come out as an empty list, and only one of them is a clean bill.
        let mut listed = Vec::new();
        let mut unanswered = Vec::new();
        let mut answered = Vec::new();
        for (name, answer) in names.into_iter().zip(answers) {
            match answer {
                Ok(pkgs) => {
                    answered.push(name);
                    listed.push(pkgs);
                }
                Err(e) => {
                    warn!(
                        "`{name}` could not be listed, so nothing it has counts as unmanaged \
                         here — this is not a clean bill for `{name}`: {e}"
                    );
                    // Returned as well as warned about. The comment above has said this since
                    // the function was written, and the caller still printed *"System matches
                    // your manifests; nothing to install, no drift, nothing undeclared"* over
                    // three managers that never answered (B4). A warning on stderr is not a
                    // field, and `check drift --json` had no key that could carry it.
                    unanswered.push(format!("{name}: {e}"));
                }
            }
        }
        // D5: a `.deb`/`.rpm` a download backend handed to a system manager is listed by that
        // manager as installed, but a download declaration owns it — so it is not unmanaged, and
        // `purge-undeclared` must defer to the recorded installer rather than delete it. Match by
        // name: the installer is `dpkg`/`rpm`, the lister is `apt`/`dnf`, and the name is the one
        // identity they share.
        let owned = self.owned_system_package_names().await;
        // The managed check touches the state lock once, after the process work is done,
        // rather than holding it across every backend's query.
        let state = self.state.lock().await;
        let managed = state.managed_index();
        Ok(UndeclaredReport {
            // Collapsed BEFORE the managed check, not after: `pacman:jq` being declared is
            // what makes jq declared, and a `yay:jq` row surviving that filter would report a
            // declared package as undeclared — and offer `purge-undeclared` a second removal
            // of something the first one already took.
            packages: crate::backends::capability::one_row_per_shared_database(
                listed.into_iter().flatten().collect(),
            )
            .into_iter()
            .filter(|pkg| !managed.contains(&(pkg.backend.as_str(), pkg.name.as_str())))
            .filter(|pkg| !owned.contains(&pkg.name))
            .collect(),
            unanswered,
            answered,
        })
    }

    /// Every system package a download backend (`github:`/`web:`) installed through a second
    /// manager (D5), by name. Used to keep those packages out of the unmanaged crawl so they are
    /// neither double-counted nor purged out from under the declaration that owns them.
    pub async fn owned_system_package_names(&self) -> std::collections::HashSet<String> {
        // What Shall uses: this answers "which system packages did a download backend of ours
        // install", and only a backend of ours can have.
        let Ok(backends) = self.backends.usable() else {
            return std::collections::HashSet::new();
        };
        let owned =
            self.query_backends_concurrently(backends, |q| async move {
                q.owned_system_packages().await
            })
            .await;
        owned
            .into_iter()
            .flatten()
            .map(|(_installer, pkg)| pkg)
            .collect()
    }

    /// Run one read-only query against every queryable backend concurrently, capped at
    /// `max_parallel`, returning each backend's result in registry order (a failed or absent
    /// query contributes nothing). One place for the fan-out so `list`, `info` and the
    /// unmanaged crawl cannot drift in how they bound concurrency or swallow errors.
    async fn query_backends_concurrently<T, F, Fut>(
        &self,
        backends: Vec<Arc<crate::core::BackendCapabilities>>,
        query: F,
    ) -> Vec<T>
    where
        T: Send + 'static,
        F: Fn(Arc<dyn crate::core::Queryable>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = T> + Send,
    {
        use futures::stream::{FuturesOrdered, StreamExt};
        let cap = self.config.max_parallel.max(1);
        let query = Arc::new(query);
        let mut ordered = FuturesOrdered::new();
        let mut queued = backends.into_iter().filter_map(|b| {
            b.as_queryable().cloned().map(|q| {
                let query = query.clone();
                async move { query(q).await }
            })
        });

        let mut out = Vec::new();
        for _ in 0..cap {
            if let Some(fut) = queued.next() {
                ordered.push_back(fut);
            }
        }
        while let Some(res) = ordered.next().await {
            out.push(res);
            if let Some(fut) = queued.next() {
                ordered.push_back(fut);
            }
        }
        out
    }
}
