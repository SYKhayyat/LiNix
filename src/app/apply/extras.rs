use crate::core::{Error, Result};
use tracing::warn;

/// Extras holds only what it uses. It is built from an [`App`](crate::app::App) by
/// `App::extras()` and can be built without one.
pub struct Extras<'a> {
    pub(crate) config: &'a std::sync::Arc<crate::config::Config>,
    pub(crate) executor: &'a crate::core::CommandExecutor,
    pub(crate) registry: &'a std::sync::Arc<crate::backends::BackendRegistry>,
    pub(crate) scheduler: &'a std::sync::Arc<crate::app::scheduler::SchedulerManager>,
}

/// What a sync would do to the resources a module declares — `link:`, `service:`, `setting:`,
/// `shim:`, `schedule:` and `repo:`, everything that is not a package.
///
/// **One computation, because five code paths answering separately is what N-2 was.** `sync`
/// placed three files under a summary reading `already up to date`; `check drift` reported that
/// the machine matched while a declared `link:` was missing from it, and again after one LiNix
/// had placed was deleted behind its back; `plan` froze an empty plan in both directions while
/// `--dry-run sync` on the same tree named three teardowns — and the guard's own refusal text
/// sent the user to `linix plan` to "see exactly what would be undone", where they saw nothing.
///
/// The package half has had this since II.7. This is its other half, and every reader of it —
/// `check`, `plan`, `apply`, `sync`'s summary — now reads this one value.
#[derive(Debug, Default, Clone)]
pub struct ResourceChanges {
    /// Declared and not in effect: never applied, or applied and since lost.
    pub place: Vec<String>,
    /// Applied last time and declared nowhere now. The teardown round 2 made safe.
    pub undo: Vec<String>,
    /// Recorded as applied, for a kind whose current state this machine cannot be asked
    /// about. Named rather than assumed converged — a resource nobody can check is a bound on
    /// what `check` means, and an unstated bound is the thing this whole assessment is about.
    pub unverifiable: Vec<String>,
}

impl ResourceChanges {
    /// Whether a sync would touch any resource. `unverifiable` is deliberately not work: it is
    /// what LiNix cannot see, and treating "I cannot tell" as "there is drift" would make
    /// `check` permanently red on any machine declaring a `setting:`.
    pub fn is_empty(&self) -> bool {
        self.place.is_empty() && self.undo.is_empty()
    }

    pub fn total(&self) -> usize {
        self.place.len() + self.undo.len()
    }

    /// One line for a summary that also counts packages.
    pub fn summary(&self) -> String {
        format!("{} to place, {} to undo", self.place.len(), self.undo.len())
    }
}

impl Extras<'_> {
    /// The shim directory's manager. Built from the same field `App` builds it from; a shim
    /// is a file on disk, so nothing else is needed to reach one.
    async fn shim_manager(&self) -> Result<crate::app::ShimManager> {
        crate::app::ShimManager::with_bin_dir(self.config.bin_dir.clone()).await
    }

    /// The resource half of the plan: what would be placed, what would be undone.
    ///
    /// Two questions, and they are answered by two different sources on purpose. *Has this ever
    /// been applied?* is the ledger's to answer — it is the only record of what a previous sync
    /// put in place. *Is it still in effect?* is the machine's, because a file LiNix placed and
    /// a user then deleted is drift the record cannot see, and that was half of N-2's
    /// reproduction.
    pub async fn changes(&self, state: &crate::model::DesiredState) -> Result<ResourceChanges> {
        use crate::core::extras_lock::ExtrasLedger;

        let declared = declared_extras(state);
        let path = ExtrasLedger::path_in(&self.config.config_root().join("locks"));
        let ledger = ExtrasLedger::load(&path)?;

        let mut changes = ResourceChanges {
            undo: ledger.drift(&declared),
            ..Default::default()
        };
        // By key, so a line declared twice is one entry and the order is the file's — the same
        // set `declared_extras` builds, carrying the statement each key came from because the
        // probe needs the source a `link:` was written from.
        let by_key: std::collections::BTreeMap<String, &crate::config::grammar::Statement> = state
            .extras
            .iter()
            .filter_map(|(s, _)| crate::core::extras_lock::extra_key(s).map(|k| (k, s)))
            .collect();
        for (key, stmt) in by_key {
            if !ledger.applied().contains(&key) {
                // Never applied. `sync` will place it, whatever the machine looks like — so
                // this needs no probe and is the same answer for all six kinds.
                changes.place.push(key);
                continue;
            }
            match in_effect(self.config, stmt, &key).await {
                Some(true) => {}
                Some(false) => changes.place.push(key),
                None => changes.unverifiable.push(key),
            }
        }
        Ok(changes)
    }

    /// Undo the extras that were applied but are no longer declared (S20). Extras had no
    /// record of what was put in place, so deleting a `service:`/`repo:`/`shim:`/`link:`/
    /// `schedule:` line left it in effect forever — `sync` could not even *detect* the
    /// removal. The applied-extras ledger (`locks/extras.toml`) closes that: this diffs the
    /// currently-declared extras against what the last sync recorded, undoes the difference,
    /// and records the new set. It is the extras' half of "removing a line removes the thing".
    ///
    /// Best-effort per item: a backend that cannot undo one extra must not block the rest, so
    /// each failure warns and the run continues. The ledger is still updated to the declared
    /// set — a drifted extra we could not undo is reported, not retried forever.
    pub async fn reconcile(
        &self,
        state: &crate::model::DesiredState,
        scope: crate::app::sync::guard::GuardScope,
        packages_being_removed: usize,
    ) -> Result<usize> {
        use crate::app::sync::guard;
        use crate::core::extras_lock::{split_key, ExtrasLedger};

        let declared = declared_extras(state);

        let path = ExtrasLedger::path_in(&self.config.config_root().join("locks"));
        let ledger = ExtrasLedger::load(&path)?;
        let drift = ledger.drift(&declared);

        // Nothing drifted and the record already matches — no work and, crucially, no write, so
        // an ordinary no-op sync does not churn `locks/extras.toml` on every run.
        if drift.is_empty() && ledger.applied() == &declared {
            return Ok(0);
        }

        // Before the first resource is torn down, and before the dry-run branch: a preview that
        // skipped the guard would report a teardown the real run then refuses, and the two must
        // never disagree about the same machine.
        if !drift.is_empty() {
            guard::enforce_extras(
                self.config,
                self.registry,
                &guard::extra_removal_pairs(&drift),
                packages_being_removed,
                scope,
            )
            .await?;
        }

        // Said with `warn!` rather than `info!`: a deletion the user cannot see coming is the
        // wrong shape, and `info!` is below the default filter, which is why this teardown
        // could delete five files under a summary reading `already up to date`.
        for key in &drift {
            if self.config.dry_run {
                warn!(
                    "[DRY-RUN] `{}` is no longer declared — sync would undo it.",
                    key
                );
            } else {
                warn!("`{}` is no longer declared — undoing it.", key);
            }
        }

        // An undo that failed leaves the extra in place, so its key stays in the ledger and
        // the next sync tries again. Dropping it would turn one warning into a service or
        // timer LiNix has permanently forgotten it owns.
        let mut still_applied = std::collections::BTreeSet::new();
        for key in &drift {
            let Some((kind, id)) = split_key(key) else {
                continue;
            };
            if self.config.dry_run {
                continue;
            }
            if let Err(e) = self.undo_extra(kind, id).await {
                warn!(
                    "could not undo `{}` ({}); it is still in place and the next sync will \
                     try again.",
                    key, e
                );
                still_applied.insert(key.clone());
            }
        }

        // Record what is declared now (even in dry-run? no — a dry run changes nothing, so
        // the ledger must not move, or the next real run would miss the drift).
        if !self.config.dry_run {
            let mut ledger = ledger;
            let mut recorded = declared;
            recorded.append(&mut still_applied);
            ledger.record(recorded);
            ledger.save(&path)?;
        }
        Ok(drift.len())
    }
    /// Execute the undo for one drifted extra, dispatched on its kind (S20). Each arm uses the
    /// same removal path the imperative command would.
    async fn undo_extra(&self, kind: &str, id: &str) -> Result<()> {
        match kind {
            "shim" => self.shim_manager().await?.remove_shim(id).await,
            "schedule" => self.scheduler.deprovision(self.executor, id).await,
            "service" | "link" | "setting" => {
                let Some(b) = self.registry.get(kind) else {
                    return Err(Error::BackendNotFound(format!(
                        "the `{}` backend is not available to undo `{}:{}`",
                        kind, kind, id
                    )));
                };
                let Some(inst) = b.as_installable() else {
                    return Ok(());
                };
                inst.remove(std::slice::from_ref(&id.to_string()), b.sudo_for_write())
                    .await
                    .map(|_| ())
            }
            "repo" => {
                // A repo key is `repo:<backend>:<spec>`; `id` here is `<backend>:<spec>`.
                let Some((backend, spec)) = id.split_once(':') else {
                    return Err(Error::Config(format!("malformed repo key `repo:{}`", id)));
                };
                let Some(b) = self.registry.get(backend) else {
                    return Err(Error::BackendNotFound(format!(
                        "the `{}` backend is not available to undo `repo:{}:{}`",
                        backend, backend, spec
                    )));
                };
                let Some(mgr) = b.as_repo_manager() else {
                    return Err(Error::Unsupported(format!(
                        "`{}` does not manage repositories",
                        backend
                    )));
                };
                mgr.remove_repo(spec, b.sudo_for_write()).await.map(|_| ())
            }
            other => {
                warn!("no undo known for extra kind `{}`.", other);
                Ok(())
            }
        }
    }
}

/// Every declared extra key: the dependents (repo/shim/service/link/setting) and the schedules.
/// Whether a declared resource is already in effect on this machine.
///
/// **The one probe.** `check` and `plan` ask it to report, and the loop that places resources
/// asks it before doing the work — because when only the reporting half asked, `check` said
/// *the machine matches your files* and `plan` said *0 resource(s) to place* while `sync`
/// re-copied all three under a summary reading `already up to date`, and the second run backed
/// up the copies LiNix had made itself.
///
/// `None` means LiNix cannot ask — **not** that the answer is yes. A `setting:` reads back
/// through an adapter that does not report a current value, a `service:` state costs a process
/// launch per line, and a `@decrypt`ed secret's plaintext cannot be compared with its ciphertext
/// without running the tool. Those are named `unverifiable` and placed rather than guessed,
/// which keeps today's behaviour for the kinds nothing can verify.
///
/// A `link:` is compared by content, not by existence: the destination existing is what the
/// ledger already knew, and a user who edits the deployed file has drift the file test cannot
/// see. On Windows the deploy falls back to a copy, so "is it a symlink to the source" is not
/// the whole question either.
pub(crate) async fn in_effect(
    config: &std::sync::Arc<crate::config::Config>,
    stmt: &crate::config::grammar::Statement,
    key: &str,
) -> Option<bool> {
    use crate::config::grammar::Statement;
    use crate::core::extras_lock::split_key;

    let (kind, id) = split_key(key)?;
    match kind {
        // The ledger keys a link by its resolved destination — exactly so the teardown can
        // find what was written — so the destination is the key and the source is the
        // declaration's own name.
        "link" => {
            let dest = std::path::Path::new(id);
            if !dest.exists() && !dest.is_symlink() {
                return Some(false);
            }
            let Statement::Link(source, opts) = stmt else {
                return None;
            };
            let want: Vec<u8> = match (
                opts.one("content"),
                opts.one("decrypt"),
                opts.one("template"),
            ) {
                // Declared inline: the bytes are in the line, and that is what gets written.
                (Some(content), None, None) => content.as_bytes().to_vec(),
                // A rendered template or a decrypted secret is not its source, and comparing
                // them would need the transform run; both are `unverifiable`, which places.
                (_, None, None) => std::fs::read(source).ok()?,
                _ => return None,
            };
            if let Ok(link) = std::fs::read_link(dest) {
                if link == std::path::Path::new(source) {
                    return Some(true);
                }
            }
            Some(std::fs::read(dest).ok()? == want)
        }
        "shim" => Some(
            crate::app::ShimManager::with_bin_dir(config.bin_dir.clone())
                .await
                .ok()?
                .is_in_effect(id)
                .await,
        ),
        _ => None,
    }
}

fn declared_extras(state: &crate::model::DesiredState) -> std::collections::BTreeSet<String> {
    state
        .extras
        .iter()
        .filter_map(|(s, _)| crate::core::extras_lock::extra_key(s))
        .collect()
}
