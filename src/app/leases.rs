use crate::core::{Error, PackageSpec, Result};
use tracing::{info, warn};

/// Leases holds only what it uses. It is built from an [`App`](crate::app::App) by
/// `App::leases()` and can be built without one.
pub struct Leases<'a> {
    pub(crate) config: &'a std::sync::Arc<crate::config::Config>,
    pub(crate) registry: &'a std::sync::Arc<crate::backends::BackendRegistry>,
    pub(crate) state: &'a std::sync::Arc<tokio::sync::Mutex<crate::core::state::StateRegistry>>,
}

impl Leases<'_> {
    /// Remove any managed packages whose `@expires` datetime has passed, across their
    /// backends, and persist the updated state. Runs as post-command maintenance so a dated
    /// line takes effect on time rather than waiting for the next explicit `sync`. No-op in
    /// dry-run mode.
    pub async fn sweep_expired(&self) -> Result<()> {
        if self.config.dry_run {
            return Ok(());
        }
        let expired = { self.state.lock().await.get_expired_packages() };
        if expired.is_empty() {
            return Ok(());
        }

        // A lease is a promise to remove something later; it is not a promise to remove
        // something the system needs. Drop protected packages from the sweep rather than
        // failing: this runs as maintenance after every state-changing command, so a hard
        // error here would break unrelated commands. The package simply stays, and its
        // lease stays expired, which is the safe direction.
        let backends: std::collections::HashSet<String> =
            expired.iter().map(|(b, _)| b.clone()).collect();
        let os_essential = crate::app::sync::guard::essential_names(
            self.registry,
            &backends,
            self.config.max_parallel,
        )
        .await;
        let (protected, expired): (Vec<_>, Vec<_>) = expired.into_iter().partition(|(b, n)| {
            crate::app::sync::guard::protection_of(self.config, Some(b), n, &os_essential).is_some()
        });
        for (backend, name) in &protected {
            warn!(
                "lease on {}:{} expired, but it is protected — leaving it installed. \
                 Run `linix protected {}:{}` to see why.",
                backend, name, backend, name
            );
        }
        if expired.is_empty() {
            return Ok(());
        }

        // The count check still applies: a state file that expires hundreds of packages at
        // once is a bug, not an intention.
        let pairs: Vec<(String, String)> = expired.clone();
        if let Err(e) = crate::app::sync::guard::enforce(
            self.config,
            self.registry,
            &pairs,
            crate::app::sync::guard::GuardScope::ExpirySweep,
        )
        .await
        {
            warn!(
                "expired-lease sweep refused, leaving them installed.\n{}",
                e
            );
            return Ok(());
        }

        info!(
            "{} package(s) have expired leases — reclaiming.",
            expired.len()
        );
        let mut failed = Vec::new();
        for (backend, name) in expired {
            if let Some(b) = self.registry.get(&backend) {
                if let Some(inst) = b.as_installable() {
                    info!("Lease expired: removing {}:{}", backend, name);
                    if let Err(e) = inst
                        .remove(std::slice::from_ref(&name), b.sudo_for_write())
                        .await
                    {
                        warn!("failed to remove expired {}:{}: {}", backend, name, e);
                        failed.push(format!("{}:{}", backend, name));
                        continue;
                    }
                    self.state.lock().await.remove(&backend, &name);
                }
            }
        }
        self.state.lock().await.save()?;
        // Reported to the caller rather than swallowed. `perform_maintenance` still decides
        // that a failed sweep must not fail the user's command — housekeeping is not the
        // command — but the decision is now made at the call site, in a line a reader can see,
        // instead of by a function that returns success after failing. `heal` had the same
        // shape and it reached the exit code (R-6).
        if failed.is_empty() {
            Ok(())
        } else {
            Err(Error::Other(format!(
                "{} expired lease(s) could not be reclaimed and are still installed: {}",
                failed.len(),
                failed.join(", ")
            )))
        }
    }
    /// Reinstall a single package by backend + name (best-effort restore). Version is
    /// intentionally not pinned — restore is reinstall-by-name, and a backend that no
    /// longer offers the package surfaces as an `Err` the caller can warn-and-move-on.
    async fn restore_package(&self, backend: &str, name: &str) -> Result<()> {
        let b = self
            .registry
            .get(backend)
            .ok_or_else(|| Error::BackendNotFound(backend.to_string()))?;
        let inst = b
            .as_installable()
            .ok_or_else(|| Error::Other(format!("Backend '{}' cannot install", backend)))?;
        let spec = PackageSpec {
            name: name.to_string(),
            backend: backend.to_string(),
            options: std::collections::HashMap::new(),
            requires: Vec::new(),
            present: true,
        };
        inst.install(std::slice::from_ref(&spec), b.sudo_for_write())
            .await
    }
    /// Restore any packages whose temporary-uninstall timer has elapsed (the mirror of
    /// `sweep_expired`). If a package can no longer be installed, we warn and move
    /// on — the suspension is cleared either way so a permanently-gone package doesn't
    /// nag on every run. No-op in dry-run mode.
    pub async fn sweep_due_suspensions(&self) -> Result<()> {
        if self.config.dry_run {
            return Ok(());
        }
        let due = { self.state.lock().await.get_due_suspensions() };
        if due.is_empty() {
            return Ok(());
        }
        info!(
            "{} temporary uninstall(s) are due for restoration.",
            due.len()
        );
        self.restore_suspensions(due, "temporarily-removed").await
    }
    /// Reinstall a set of suspended packages and clear each suspension — whether the reinstall
    /// succeeds or fails (a suspension LiNix cannot honour is dropped, not retried forever).
    /// One implementation shared by the timed sweep and the shell-exit restore, which used to
    /// carry byte-identical copies of this loop (E11); `occasion` is the only thing that
    /// differed, and it only ever changed the log wording.
    async fn restore_suspensions(
        &self,
        items: Vec<crate::core::state::Suspension>,
        occasion: &str,
    ) -> Result<()> {
        for s in items {
            match self.restore_package(&s.backend, &s.name).await {
                Ok(()) => {
                    info!("Restored {} {}:{}", occasion, s.backend, s.name);
                    let mut state = self.state.lock().await;
                    state.add(
                        &s.backend,
                        &s.name,
                        s.version.clone(),
                        std::collections::HashMap::new(),
                        "imperative",
                        false,
                    );
                    state.clear_suspension(&s.backend, &s.name);
                }
                Err(e) => {
                    warn!(
                        "could not restore {} {}:{} ({}); dropping the suspension.",
                        occasion, s.backend, s.name, e
                    );
                    self.state
                        .lock()
                        .await
                        .clear_suspension(&s.backend, &s.name);
                }
            }
        }
        self.state.lock().await.save()?;
        Ok(())
    }
}
