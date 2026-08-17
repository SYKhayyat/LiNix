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
    /// second run left `.shall-backup` files beside them, backups of the copies Shall itself
    /// had made, under a summary reading `already up to date`. `check` and `plan` reported
    /// the same machine as converged, because they asked the probe this loop did not.
    ///
    /// So the probe decides here too: a resource it reports in effect is skipped, one it
    /// cannot verify is placed. This is the forward (declared → applied) direction only;
    /// undoing a *removed* line is `Extras::reconcile`'s half.
    pub async fn apply(&self, state: &crate::model::DesiredState) -> Result<()> {
        use crate::config::grammar::Statement;

        // **On NixOS a service's enablement is not this loop's to apply** (`J5`, ruling 4).
        // `systemctl enable` writes into a tree the next `nixos-rebuild switch` regenerates —
        // including the rebuild Shall itself runs when a `nixos:` package changes — so the
        // enablement is declared in the generated module instead and this loop is left holding
        // only the transitions. See `apply::nixos`.
        let declared_by_nixos = crate::app::apply::nixos::owns_extras(self.registry);

        for (stmt, origin) in state.dependents() {
            if let Some(key) = crate::core::extras_lock::extra_key(stmt) {
                if crate::app::apply::extras::in_effect(
                    self.config,
                    self.registry,
                    self.executor,
                    stmt,
                    &key,
                )
                .await
                    == Some(true)
                {
                    tracing::debug!("`{}` is already in effect — nothing to do", key);
                    continue;
                }
            }
            match stmt {
                Statement::Shim(name, opts) => {
                    // U19: a shim lands in `~/.local/bin`, which is this account's PATH and
                    // nobody else's. Shall has no machine-wide shim directory yet, so a line
                    // asking for one is refused by name rather than quietly deploying a
                    // per-user shim under a declaration that says every account (P7).
                    if crate::model::scope::Scope::resolve(
                        opts.one("scope"),
                        crate::model::scope::Scope::User,
                    ) == crate::model::scope::Scope::System
                    {
                        return Err(Error::Validation(format!(
                            "{}: `shim:{}` asks for scope=system, and Shall deploys shims only                              into this account's `~/.local/bin`. A per-user shim under a line                              that says every account would be the wrong answer quietly, so                              this is refused. Drop `@scope=system`.",
                            origin, name
                        )));
                    }
                    if self.config.dry_run {
                        crate::would!("would deploy shim `{}`", name);
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
                    let opts = if declared_by_nixos {
                        match crate::app::apply::nixos::imperative_remainder(opts) {
                            // A restart is a transition no attribute can express, so the init
                            // still gets it — with the enablement trimmed off, because that half
                            // is the configuration's and asking twice is two owners.
                            Some(remainder) => std::borrow::Cow::Owned(remainder),
                            None => continue,
                        }
                    } else {
                        std::borrow::Cow::Borrowed(opts)
                    };
                    self.apply_through_backend("service", name, &opts, origin)
                        .await?
                }
                Statement::Link(name, opts) => {
                    self.apply_through_backend("link", name, opts, origin)
                        .await?
                }
                Statement::Setting(name, opts) => {
                    self.apply_through_backend("setting", name, opts, origin)
                        .await?
                }
                // dependents() yields only these four variants.
                _ => {}
            }
        }
        Ok(())
    }

    /// Apply one `service:` / `link:` / `setting:` line through the backend that owns its
    /// keyword.
    ///
    /// **One body, because there was never more than one behaviour.** The three arms this
    /// replaces were byte-identical apart from the keyword and three differently-worded log
    /// lines — `applying`, `Link: applying`, `Setting: applying` — so a reader could not tell
    /// whether the difference in wording meant a difference in what happened. It did not.
    ///
    /// **A backend that cannot install is reported, not skipped.** Each arm ended
    /// `let Some(inst) = b.as_installable() else { continue };` — a declared line, a registered
    /// backend, and nothing done, with no output at all. That is `Q40`'s failure again: silence
    /// standing in for success. It is a `Validation` error now, because a registry that hands
    /// back a `setting` backend which cannot write a setting is a wiring bug in Shall, not
    /// something the user can fix by editing their file.
    async fn apply_through_backend(
        &self,
        keyword: &str,
        name: &str,
        opts: &crate::config::grammar::Options,
        origin: &crate::config::grammar::Origin,
    ) -> Result<()> {
        let Some(b) = self.registry.get(keyword) else {
            warn!(
                "{}: the `{}` backend is not available here — skipping `{}:{}`.",
                origin, keyword, keyword, name
            );
            return Ok(());
        };
        if self.config.dry_run {
            crate::would!("would apply {} `{}`", keyword, name);
            return Ok(());
        }
        let Some(inst) = b.as_installable() else {
            return Err(Error::Validation(format!(
                "{}: the `{}` backend is registered but cannot install, so `{}:{}` could not be \
                 applied. This is a wiring fault in Shall rather than a problem with your file.",
                origin, keyword, keyword, name
            )));
        };
        info!("applying `{}:{}` ({})", keyword, name, origin);
        let spec = spec_from_extra(keyword, name, opts);
        inst.install(std::slice::from_ref(&spec), b.sudo_for_write())
            .await?;
        Ok(())
    }
}

/// Turn a non-package statement's options into a `PackageSpec` the `service`/`link`
/// backends consume. Their `Installable::install` reads the options it knows (`enabled`,
/// `status`, `target`, `content`, `template`, `decrypt`, …); a key it doesn't know is
/// simply ignored, which is why the grammar — not this conversion — is where an unknown
/// key is refused. Options are single-valued here (a service is enabled or not), so the
/// first value of each key is taken.
///
/// Shared with the dotfiles tree, whose files reach the `link:` backend as the very same value
/// a hand-written line does. A second converter beside it is how the two paths would drift
/// apart again.
pub(crate) fn spec_from_extra(
    backend: &str,
    name: &str,
    opts: &crate::config::grammar::Options,
) -> PackageSpec {
    // **Every value, not the first of each.** This took `values.first()` because the spec's
    // options were `HashMap<String, String>` and a list had nowhere to go — so a `link:` line
    // with a repeated key reached the backend one value short, silently. The spec carries the
    // grammar's own type now, so the conversion is a copy.
    let mut options = crate::config::grammar::Options::default();
    for (key, values) in opts.iter() {
        options.set_all(key, values.to_vec());
    }
    PackageSpec {
        name: name.to_string(),
        backend: backend.to_string(),
        options,
        requires: Vec::new(),
        present: true,
    }
}
