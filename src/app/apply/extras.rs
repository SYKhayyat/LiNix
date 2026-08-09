use crate::config::grammar::{Origin, Statement};
use crate::core::LockFile;
use crate::app::sync::guard;
use crate::core::{Error, Result};
use tracing::warn;

/// A declared resource and the line it came from. Named because a `dotfiles:` tree is expanded
/// into these, so the pair travels between three functions here and reads worse spelled out.
type Declared = (Statement, Origin);

/// Extras holds only what it uses. It is built from an [`App`](crate::app::App) by
/// `App::extras()` and can be built without one.
pub struct Extras<'a> {
    pub(crate) config: &'a std::sync::Arc<crate::config::Config>,
    pub(crate) executor: &'a crate::core::CommandExecutor,
    pub(crate) registry: &'a std::sync::Arc<crate::backends::BackendRegistry>,
    pub(crate) scheduler: &'a std::sync::Arc<crate::app::scheduler::SchedulerManager>,
    /// The command's teardown budget, shared with every other phase that removes.
    pub(crate) reaping: &'a guard::Reaping,
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

    /// The `link:` lines the declared `dotfiles:` trees stand for (U22), which are extras like
    /// any other once expanded.
    ///
    /// The expansion has to happen here rather than in [`extra_key`], because a tree's files
    /// are a fact about the disk and that function only has the declaration. That is why the
    /// row it documents never existed: nothing was in a position to write it.
    ///
    /// A tree that cannot be walked propagates its error rather than contributing nothing. An
    /// empty answer would read as "every file this tree ever placed has departed" and tear the
    /// lot down over a deleted directory — the same trap `declared_exec_paths` names.
    fn tree_links(&self, state: &crate::model::DesiredState) -> Result<Vec<Declared>> {
        crate::app::Dotfiles {
            config: self.config,
            registry: self.registry,
        }
        .links(state)
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

        let trees = self.tree_links(state)?;
        let declared = declared_extras(state.extras.iter().chain(trees.iter()));
        let path = ExtrasLedger::path_in(&self.config.config_root().join("locks"));
        let ledger = ExtrasLedger::load(&path)?;

        let mut changes = ResourceChanges {
            undo: ledger.drift(&declared),
            ..Default::default()
        };
        // By key, so a line declared twice is one entry and the order is the file's — the same
        // set `declared_extras` builds, carrying the statement each key came from because the
        // probe needs the source a `link:` was written from.
        let by_key: std::collections::BTreeMap<String, &Statement> = state
            .extras
            .iter()
            .chain(trees.iter())
            .filter_map(|(s, _)| crate::core::extras_lock::extra_key(s).map(|k| (k, s)))
            .collect();
        for (key, stmt) in by_key {
            // The probe first and the ledger only after it. `Dependents::apply` has never
            // consulted the ledger — it skips whatever the probe reports in effect — so a
            // short-circuit on "never applied" made `plan` promise work `sync` would not do.
            // Invisible until `adopt` wrote 150 running services LiNix had never placed:
            // `plan` said 150 resources to place and `sync` correctly placed none.
            //
            // The record still answers the case nothing can be asked about: a resource this
            // machine cannot be queried for has been applied, or it has not, and only one of
            // those is work.
            match in_effect(self.config, self.registry, stmt, &key).await {
                Some(true) => {}
                Some(false) => changes.place.push(key),
                None if !ledger.applied().contains(&key) => changes.place.push(key),
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
        scope: guard::GuardScope,
    ) -> Result<usize> {
        use crate::core::extras_lock::{split_key, ExtrasLedger};

        let trees = self.tree_links(state)?;
        let declared = declared_extras(state.extras.iter().chain(trees.iter()));

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
        // The token this returns is what the four effectors below will not run without, so
        // the `if` cannot be widened into a path that skips the call: there would be nothing to
        // pass. Before it existed, `enforce_extras` was a statement whose absence a reader had
        // to notice.
        let reaped = if drift.is_empty() {
            None
        } else {
            Some(
                guard::enforce_extras(
                    self.config,
                    self.registry,
                    &guard::extra_removal_pairs(&drift),
                    self.reaping,
                    scope,
                )
                .await?,
            )
        };

        // Said with `warn!` rather than `info!`: a deletion the user cannot see coming is the
        // wrong shape, and `info!` is below the default filter, which is why this teardown
        // could delete five files under a summary reading `already up to date`.
        for key in &drift {
            if self.config.dry_run {
                crate::would_warn!(
                    "`{}` is no longer declared — sync would undo it.",
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
            let Some(reaped) = reaped else {
                // Unreachable: `drift` is non-empty inside this loop, so the guard ran above.
                // Written as a skip rather than an `unwrap` because an effector that removes is
                // the wrong place to learn that an invariant was wrong.
                continue;
            };
            if let Err(e) = self.undo_extra(kind, id, reaped).await {
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
    async fn undo_extra(&self, kind: &str, id: &str, reaped: guard::Reaped) -> Result<()> {
        match kind {
            "shim" => self.shim_manager().await?.remove_shim(id, reaped).await,
            "schedule" => self.scheduler.deprovision(self.executor, id, reaped).await,
            "service" | "link" | "setting" => {
                let Some(b) = self.registry.get(kind) else {
                    return Err(Error::BackendNotFound(format!(
                        "the `{}` backend is not available to undo `{}:{}`",
                        kind, kind, id
                    )));
                };
                // The twin of `Dependents::apply_through_backend`'s check, and it had the twin
                // bug: `Ok(())` here reports the undo as done to a caller that then clears the
                // extras lock, so the drifted resource is forgotten while still in effect and
                // no later sync will look at it again.
                let Some(inst) = b.as_installable() else {
                    return Err(Error::Validation(format!(
                        "the `{}` backend is registered but cannot remove, so `{}:{}` could not \
                         be undone. This is a wiring fault in LiNix.",
                        kind, kind, id
                    )));
                };
                inst.remove(
                    std::slice::from_ref(&id.to_string()),
                    b.sudo_for_write(),
                    reaped,
                )
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
                mgr.remove_repo(spec, b.sudo_for_write(), reaped)
                    .await
                    .map(|_| ())
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
    registry: &crate::backends::BackendRegistry,
    stmt: &crate::config::grammar::Statement,
    key: &str,
) -> Option<bool> {
    use crate::config::grammar::Statement;
    use crate::core::extras_lock::split_key;

    let (kind, id) = split_key(key)?;
    match kind {
        // A running service is a state the init can be asked about, and asking costs one
        // cached listing for the whole run. Left unasked, every `service:` line was
        // `unverifiable` — which places — so adopting a machine's 150 running services made
        // every later sync run 150 `sc start` calls on services that were already running.
        "service" => {
            let Statement::Service(_, opts) = stmt else {
                return None;
            };
            // Enablement is a second axis and no init here reports it in the listing. A line
            // that declares it is answered by the machine, not by this shortcut.
            if opts.one("enabled").is_some() {
                return None;
            }
            let running = registry
                .get("service")?
                .as_queryable()?
                .list_installed()
                .await
                .ok()?
                .iter()
                .any(|p| p.name == id);
            match opts.one("status")? {
                "running" | "started" | "start" => Some(running),
                "stopped" | "stop" => Some(!running),
                // A restart is a transition, not a state: no listing can say it happened.
                _ => None,
            }
        }
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

fn declared_extras<'a>(
    statements: impl Iterator<Item = &'a Declared>,
) -> std::collections::BTreeSet<String> {
    statements
        .filter_map(|(s, _)| crate::core::extras_lock::extra_key(s))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::in_effect;
    use crate::backends::service::{InitProviderFile, ServiceBackendCore, ServiceQueryable};
    use crate::backends::BackendRegistry;
    use crate::config::grammar::{Options, Statement};
    use crate::core::{BackendCapabilities, CommandExecutor};
    use std::sync::Arc;

    /// A registry whose `service` backend reports exactly one running service, from a real
    /// child process — so the listing is parsed rather than handed over.
    fn registry_listing_nginx() -> BackendRegistry {
        #[cfg(windows)]
        let toml = r#"
[[init]]
name = "probe"
detect = "cmd"
start = [["cmd", "/C", "exit 0"]]
stop  = [["cmd", "/C", "exit 0"]]
list  = ["cmd", "/C", "echo SERVICE_NAME: nginx"]
list_pattern = 'SERVICE_NAME:\s+(\S+)'
"#;
        #[cfg(not(windows))]
        let toml = r#"
[[init]]
name = "probe"
detect = "sh"
start = [["sh", "-c", "exit 0"]]
stop  = [["sh", "-c", "exit 0"]]
list  = ["sh", "-c", "echo 'SERVICE_NAME: nginx'"]
list_pattern = 'SERVICE_NAME:\s+(\S+)'
"#;
        let file: InitProviderFile = toml::from_str(toml).expect("the probe row parses");
        let core = Arc::new(ServiceBackendCore::with_providers(
            CommandExecutor::new(false, false),
            file.init,
        ));
        let mut reg = BackendRegistry::new();
        reg.register(Arc::new(
            BackendCapabilities::builder(core.clone())
                .with_queryable(Arc::new(ServiceQueryable { core }))
                .build(),
        ));
        reg
    }

    fn service(name: &str, key: &str, value: &str) -> (Statement, String) {
        let mut opts = Options::default();
        opts.insert(key, value);
        (
            Statement::Service(name.to_string(), opts),
            format!("service:{}", name),
        )
    }

    /// The reason `adopt` made every later sync fail: a `service:` line was `unverifiable`, and
    /// unverifiable places — so 150 adopted running services meant 150 `sc start` calls per
    /// sync, each on a service that was already running.
    #[tokio::test]
    async fn a_service_already_in_its_declared_state_is_not_placed_again() {
        let config = Arc::new(crate::config::Config::default());
        let reg = registry_listing_nginx();

        let (running, key) = service("nginx", "status", "running");
        assert_eq!(
            in_effect(&config, &reg, &running, &key).await,
            Some(true),
            "nginx is in the listing and the line asks for running"
        );

        let (stopped, key) = service("nginx", "status", "stopped");
        assert_eq!(
            in_effect(&config, &reg, &stopped, &key).await,
            Some(false),
            "a running service declared stopped is drift, and drift places"
        );

        // The same two questions about a service the init does not report.
        let (running, key) = service("absent-svc", "status", "running");
        assert_eq!(in_effect(&config, &reg, &running, &key).await, Some(false));
        let (stopped, key) = service("absent-svc", "status", "stopped");
        assert_eq!(in_effect(&config, &reg, &stopped, &key).await, Some(true));
    }

    /// What the listing cannot answer stays unanswered. A restart is a transition no listing
    /// records; enablement is a second axis none of the shipped inits report. Answering either
    /// from "is it running" would report converged on a machine that is not.
    #[tokio::test]
    async fn what_the_listing_cannot_answer_is_left_unverifiable() {
        let config = Arc::new(crate::config::Config::default());
        let reg = registry_listing_nginx();

        let (restarted, key) = service("nginx", "status", "restarted");
        assert_eq!(in_effect(&config, &reg, &restarted, &key).await, None);

        let (enabled, key) = service("nginx", "enabled", "true");
        assert_eq!(in_effect(&config, &reg, &enabled, &key).await, None);

        // Enablement declared *alongside* a status is still unanswered: the status half being
        // satisfied says nothing about the half that is not.
        let mut opts = Options::default();
        opts.insert("status".to_string(), "running");
        opts.insert("enabled".to_string(), "true");
        let both = Statement::Service("nginx".to_string(), opts);
        assert_eq!(
            in_effect(&config, &reg, &both, "service:nginx").await,
            None,
            "running is satisfied and enabled is unknown — the line as a whole is unknown"
        );
    }
}
