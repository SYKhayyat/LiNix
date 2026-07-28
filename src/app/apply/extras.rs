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

impl Extras<'_> {
    /// The shim directory's manager. Built from the same field `App` builds it from; a shim
    /// is a file on disk, so nothing else is needed to reach one.
    async fn shim_manager(&self) -> Result<crate::app::ShimManager> {
        crate::app::ShimManager::with_bin_dir(self.config.bin_dir.clone()).await
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
    /// The extras a sync would undo: applied last time, declared nowhere now. `status` and
    /// `reconcile` ask the same question, so they ask it in the same place — a preview
    /// computed a second way is a preview free to disagree with the run.
    pub async fn drift(&self, state: &crate::model::DesiredState) -> Result<Vec<String>> {
        use crate::core::extras_lock::ExtrasLedger;

        let path = ExtrasLedger::path_in(&self.config.config_root().join("locks"));
        let ledger = ExtrasLedger::load(&path)?;
        Ok(ledger.drift(&declared_extras(state)))
    }
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
fn declared_extras(state: &crate::model::DesiredState) -> std::collections::BTreeSet<String> {
    state
        .extras
        .iter()
        .filter_map(|(s, _)| crate::core::extras_lock::extra_key(s))
        .collect()
}
