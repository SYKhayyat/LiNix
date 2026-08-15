//! The two commands that act on every manager at once, rather than on a package.
//!
//! `update` refreshes metadata and `upgrade` moves versions; both fan out across the whole
//! `priority` list, and both had the same bug before they had this type — `?` on the first
//! failure, so one manager that could not answer silently cancelled every manager after it.
//! They live together because that rule is the thing they share.

use crate::app::{Backends, MetricsCollector};
use crate::config::Config;
use crate::core::{Error, Result, SnapshotManager};
use std::sync::Arc;
use tracing::{info, warn};

/// Managers holds only what it uses. It is built from an [`App`](crate::app::App) by
/// `App::managers()` and can be built without one.
pub struct Managers<'a> {
    pub(crate) config: &'a Arc<Config>,
    pub(crate) metrics: &'a MetricsCollector,
    pub(crate) snapshot_manager: &'a Arc<SnapshotManager>,
    /// The registry paired with `priority`: a manager the model does not use is not this
    /// command's business, however installed it is.
    pub(crate) backends: &'a Backends,
}

impl Managers<'_> {
    /// Refresh every backend's metadata, and do not let one stop the rest.
    ///
    /// `?` on the first failure meant a single manager that could not refresh — a plugin
    /// missing, a repo down — silently skipped every backend after it, and the ones that
    /// did refresh went unmentioned. Each failure is named and the command still reports
    /// one, because a refresh that half-happened is not a refresh that worked.
    pub async fn update(&self) -> Result<()> {
        use futures::stream::{self, StreamExt};
        info!("refreshing package metadata");
        // Each backend's refresh is an independent network fetch (`apt update`, `brew update`,
        // …) — seconds of waiting with nothing shared between them. Overlap the waits, capped
        // at `max_parallel`. Unlike `upgrade`, this changes no package, so concurrent runs
        // cannot contend on a package database.
        let cap = self.config.max_parallel.max(1);
        // What Shall uses: refreshing the package index of a manager `priority` does not
        // name is work nobody asked for, on a tool this run will never call.
        //
        // **Once per database, not once per client.** The comment above says concurrent
        // refreshes cannot contend on a package database, and on Arch that is false twice
        // over: `pacman -Sy`, `yay -Sy` and `paru -Sy` all take `/var/lib/pacman/db.lck`, and
        // all three fetch the same repositories. Measured in the arch image — *"yay: could not
        // refresh — failed to synchronize all databases (unable to lock database)"* — which is
        // a command reporting failure for doing the same work three times at once.
        //
        // Dropping the clients is not a narrowing: a client's `-Sy` refreshes the owner's
        // repositories and nothing of its own, so one refresh is the whole answer.
        let mut refreshing: Vec<String> = self
            .backends
            .usable()?
            .iter()
            .map(|b| b.name().to_string())
            .collect();
        crate::backends::capability::one_backend_per_shared_database(&mut refreshing);
        let refreshing: std::collections::HashSet<String> = refreshing.into_iter().collect();
        for b in self.backends.usable()? {
            if !refreshing.contains(b.name()) {
                info!(
                    "{}: not refreshed separately — it reads {}'s repositories, which this run \
                     already refreshed",
                    b.name(),
                    crate::backends::capability::package_database(b.name())
                );
            }
        }
        let failed: Vec<String> = stream::iter(
            self.backends
                .usable()?
                .into_iter()
                .filter(|b| refreshing.contains(b.name())),
        )
        .map(|backend| async move {
            let upgradable = backend.as_upgradable()?;
            match upgradable.update(backend.sudo_for_write()).await {
                Ok(()) => None,
                Err(e) => {
                    warn!("{}: could not refresh — {}", backend.name(), e);
                    Some(format!("{} ({})", backend.name(), e))
                }
            }
        })
        .buffer_unordered(cap)
        .filter_map(|x| async move { x })
        .collect()
        .await;
        if failed.is_empty() {
            return Ok(());
        }
        Err(Error::Other(format!(
            "{} backend(s) could not refresh their metadata; the rest were refreshed: {}",
            failed.len(),
            failed.join("; ")
        )))
    }

    pub async fn upgrade(&self) -> Result<()> {
        let _ = self
            .snapshot_manager
            .auto_snapshot(crate::core::snapshot::SnapshotLabel::PreUpgrade)
            .await?;
        use futures::stream::{self, StreamExt};
        info!("upgrading all packages");
        // The same rule as `update`, and for the same reason: one manager that cannot
        // upgrade must not silently cancel every manager after it in the list.
        //
        // **Grouped by what they contend for, not run one at a time.** This was deliberately
        // serial, and the reason given — "it changes packages, so concurrent sudo operations
        // would interleave" — is true of the managers that share a system package database and
        // false of `cargo`, `npm`, `pipx`, `uv`, `yarn`, `pnpm`, `vscode`, `emacs`, `krew` and
        // `go`, which contend with nothing and are typically the slow ones because each
        // rebuilds or refetches from a registry. So the contending set stays strictly
        // sequential and the rest overlap. `run_exclusive`'s per-manager mutex is still
        // underneath both, which is the safety this loop was being blunt about.
        //
        // **`needs_root` was standing in for "does this contend", and the AUR helpers are
        // exactly where the proxy breaks.** `yay` and `paru` have `needs_root = false` —
        // correctly, they must run unprivileged and escalate internally — so they landed in
        // the concurrent half while `pacman` ran in the serial half, over the one libalpm
        // database all three of them lock. `update` above measured that collision; this is
        // the same three managers one function down. The question the partition wants to ask
        // is asked directly now.
        // What Shall uses: `upgrade` acts on the model, so a manager outside `priority` is
        // not this command's business however installed it is.
        let (contending, independent): (Vec<_>, Vec<_>) = self
            .backends
            .usable()?
            .into_iter()
            .filter(|b| b.is_upgradable())
            .partition(|b| {
                b.needs_root() || !crate::backends::capability::owns_its_database(b.name())
            });

        let mut failed: Vec<String> = Vec::new();
        for backend in contending {
            if let Some(upgradable) = backend.as_upgradable() {
                if let Err(e) = upgradable.upgrade(backend.sudo_for_write()).await {
                    warn!("{}: could not upgrade — {}", backend.name(), e);
                    failed.push(format!("{} ({})", backend.name(), e));
                }
            }
        }
        let cap = self.config.max_parallel.max(1);
        let user_failures: Vec<String> = stream::iter(independent)
            .map(|backend| async move {
                let upgradable = backend.as_upgradable()?;
                match upgradable.upgrade(backend.sudo_for_write()).await {
                    Ok(()) => None,
                    Err(e) => {
                        warn!("{}: could not upgrade — {}", backend.name(), e);
                        Some(format!("{} ({})", backend.name(), e))
                    }
                }
            })
            .buffer_unordered(cap)
            .filter_map(|x| async move { x })
            .collect()
            .await;
        failed.extend(user_failures);
        self.metrics
            .print_summary(crate::app::metrics::Narration::Change);
        if failed.is_empty() {
            return Ok(());
        }
        Err(Error::Other(format!(
            "{} backend(s) could not upgrade; the rest were upgraded: {}",
            failed.len(),
            failed.join("; ")
        )))
    }
}
