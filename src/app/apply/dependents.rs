use crate::core::{Error, PackageSpec, Result};
use tracing::{info, warn};

/// Dependents holds only what it uses. It is built from an [`App`](crate::app::App) by
/// `App::dependents()` and can be built without one.
pub struct Dependents<'a> {
    pub(crate) config: &'a std::sync::Arc<crate::config::Config>,
    pub(crate) registry: &'a std::sync::Arc<crate::backends::BackendRegistry>,
    pub(crate) executor: &'a crate::core::CommandExecutor,
}

impl Dependents<'_> {
    /// The shim directory's manager. Built from the same field `App` builds it from; a shim
    /// is a file on disk, so nothing else is needed to reach one.
    async fn shim_manager(&self) -> Result<crate::app::ShimManager> {
        crate::app::ShimManager::with_bin_dir(self.config.bin_dir.clone()).await
    }

    /// Apply the dependent extras — shims, services and links — AFTER the package plan has
    /// run (II.7's dependent phase, the mirror of `apply`'s phase 1).
    ///
    /// **Why after packages, not interleaved:** each of these presupposes a package. A
    /// `shim:` wraps a binary that must already be on disk; a `service:` enables a unit a
    /// package just installed; a `link:` writes the config a package expects to read. So
    /// they cannot be planned alongside packages — they must wait for the whole package
    /// plan to finish. Applied in declaration order, so a user who writes the config `link:`
    /// above the `service:` that reads it gets that order honoured.
    ///
    /// Idempotent, and **asked rather than assumed**: this doc line used to claim that
    /// re-writing an unchanged link was a no-op, and on Windows — where the deploy falls back
    /// to a copy — it was not. Every sync re-copied all three links in the fixture and the
    /// second run left `.linix-backup` files beside them, backups of the copies LiNix itself
    /// had made, under a summary reading `already up to date`. `check` and `plan` reported
    /// the same machine as converged, because they asked the probe this loop did not.
    ///
    /// So the probe decides here too: a resource it reports in effect is skipped, one it
    /// cannot verify is placed. This is the forward (declared → applied) direction only;
    /// undoing a *removed* line is `Extras::reconcile`'s half.
    pub async fn apply(&self, state: &crate::model::DesiredState) -> Result<()> {
        use crate::config::grammar::Statement;

        for (stmt, origin) in state.dependents() {
            if let Some(key) = crate::core::extras_lock::extra_key(stmt) {
                if crate::app::apply::extras::in_effect(self.config, stmt, &key).await == Some(true)
                {
                    tracing::debug!("`{}` is already in effect — nothing to do", key);
                    continue;
                }
            }
            match stmt {
                Statement::Shim(name, opts) => {
                    // U19: a shim lands in `~/.local/bin`, which is this account's PATH and
                    // nobody else's. LiNix has no machine-wide shim directory yet, so a line
                    // asking for one is refused by name rather than quietly deploying a
                    // per-user shim under a declaration that says every account (P7).
                    if crate::model::scope::Scope::resolve(
                        opts.one("scope"),
                        crate::model::scope::Scope::User,
                    ) == crate::model::scope::Scope::System
                    {
                        return Err(Error::Validation(format!(
                            "{}: `shim:{}` asks for scope=system, and LiNix deploys shims only                              into this account's `~/.local/bin`. A per-user shim under a line                              that says every account would be the wrong answer quietly, so                              this is refused. Drop `@scope=system`.",
                            origin, name
                        )));
                    }
                    if self.config.dry_run {
                        info!("[DRY-RUN] would deploy shim `{}`", name);
                        continue;
                    }
                    info!("deploying `{}` ({})", name, origin);
                    self.shim_manager().await?.create_shim(name).await?;
                    // A shim in a directory nobody's PATH names is a file, not a command —
                    // the same event as E6c's install, and it needs saying here too because
                    // `shim:` never reaches the package plan that says it there.
                    if let Some(msg) = crate::app::reachable::unreachable_warning(
                        "shim",
                        self.config,
                        self.executor,
                    )
                    .await
                    {
                        warn!("{}", msg);
                    }
                }
                Statement::Service(name, opts) => {
                    let Some(b) = self.registry.get("service") else {
                        warn!(
                            "{}: the service backend is not available here — skipping `service:{}`.",
                            origin, name
                        );
                        continue;
                    };
                    if self.config.dry_run {
                        info!("[DRY-RUN] would apply service `{}`", name);
                        continue;
                    }
                    let Some(inst) = b.as_installable() else {
                        continue;
                    };
                    info!("applying `{}` ({})", name, origin);
                    let spec = spec_from_extra("service", name, opts);
                    inst.install(std::slice::from_ref(&spec), b.sudo_for_write())
                        .await?;
                }
                Statement::Link(name, opts) => {
                    let Some(b) = self.registry.get("link") else {
                        warn!(
                            "{}: the link backend is not available here — skipping `link:{}`.",
                            origin, name
                        );
                        continue;
                    };
                    if self.config.dry_run {
                        info!("[DRY-RUN] would apply link `{}`", name);
                        continue;
                    }
                    let Some(inst) = b.as_installable() else {
                        continue;
                    };
                    info!("Link: applying `{}` ({})", name, origin);
                    let spec = spec_from_extra("link", name, opts);
                    inst.install(std::slice::from_ref(&spec), b.sudo_for_write())
                        .await?;
                }
                Statement::Setting(name, opts) => {
                    let Some(b) = self.registry.get("setting") else {
                        warn!(
                            "{}: no settings adapter here — skipping `setting:{}`.",
                            origin, name
                        );
                        continue;
                    };
                    if self.config.dry_run {
                        info!("[DRY-RUN] would apply setting `{}`", name);
                        continue;
                    }
                    let Some(inst) = b.as_installable() else {
                        continue;
                    };
                    info!("Setting: applying `{}` ({})", name, origin);
                    let spec = spec_from_extra("setting", name, opts);
                    inst.install(std::slice::from_ref(&spec), b.sudo_for_write())
                        .await?;
                }
                // dependents() yields only these four variants.
                _ => {}
            }
        }
        Ok(())
    }
}

/// Turn a non-package statement's options into a `PackageSpec` the `service`/`link`
/// backends consume. Their `Installable::install` reads the options it knows (`enabled`,
/// `status`, `target`, `content`, `template`, `decrypt`, …); a key it doesn't know is
/// simply ignored, which is why the grammar — not this conversion — is where an unknown
/// key is refused. Options are single-valued here (a service is enabled or not), so the
/// first value of each key is taken.
fn spec_from_extra(
    backend: &str,
    name: &str,
    opts: &crate::config::grammar::Options,
) -> PackageSpec {
    let mut options = std::collections::HashMap::new();
    for (key, values) in opts.iter() {
        if let Some(first) = values.first() {
            options.insert(key.to_string(), first.clone());
        }
    }
    PackageSpec {
        name: name.to_string(),
        backend: backend.to_string(),
        options,
        requires: Vec::new(),
        present: true,
    }
}
