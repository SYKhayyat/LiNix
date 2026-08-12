use crate::verbs::perform_maintenance;
use crate::verbs::prelude::*;

// ============================================================================
// COMMAND HANDLERS
// ============================================================================

/// How one reconcile pass should behave. The pass itself is identical for `sync` and
/// `watch` — II.7's ordering phases, the guard, the same planner — and these are the only
/// things that legitimately differ between an attended run and an unattended one.
pub struct Reconcile {
    /// Strict version matching against the lockfile: a package that is not in it is an error.
    locked: bool,
    /// Take what the managers offer now instead of what the lock recorded. Off by default —
    /// a sync converges to what was decided (owner ruling, 2026-07-24).
    upgrade: bool,
    /// Emit the change report as JSON instead of a planned-changes list.
    out: Output,
    /// Which scope the guard reports refusals under.
    scope: crate::app::sync::guard::GuardScope,
    /// Whether to ask before applying. `watch` is unattended by definition and never asks;
    /// `sync` asks unless `--yes`.
    confirm: bool,
}

/// What one reconcile pass did — and what it decided not to do.
///
/// **Two numbers, because one could not tell the two silences apart.** A pass returning `0` used
/// to mean "the machine already matches", and its caller printed exactly that; it also meant
/// "there was a removal and Shall declined it", which is the opposite claim about the same
/// machine (AU1).
pub struct Reconciled {
    /// Package and resource changes actually carried out.
    pub applied: usize,
    /// Removals the planner declined, each already named on the way past.
    pub left_in_place: usize,
}

/// One reconcile pass: resolve the model, apply repos, plan, apply, then dependents,
/// schedules and extras — II.7's ordering, in order.
///
/// Returns what the pass did, and what it declined to do. `sync` and `watch` both call this;
/// the copy `watch` used to carry drifted from this body every time sync's ordering changed,
/// which is why it is one function now.
pub async fn reconcile(app: &App, opts: Reconcile) -> Result<Reconciled> {
    // A reconcile pass is one invocation for IX.6's purposes, and `watch` runs many of them in
    // one process. Without this a `when $hour` would freeze at whatever hour the daemon started.
    crate::app::sync::resolver::new_resolution();
    let engine = app.sync_engine();
    if app.journal.lock().await.needs_recovery() {
        warn!("the transaction journal records an interrupted run; healing first.");
    }

    let mut resolver = crate::app::sync::resolver::StateResolver::new(
        &app.config,
        app.registry.clone(),
        opts.locked,
    )
    .await
    .recording_locks();
    if opts.upgrade {
        resolver = resolver.upgrading();
    }
    // The whole desired state, extras included — repos must be applied before packages
    // (II.7), so this needs more than the package map.
    let state = resolver.resolve_model().await?;
    let desired = state.packages.clone();

    // After the resolution and before the plan, and both halves of that are load-bearing.
    // After, because ownership is read from what this machine declares and the resolver is
    // what knows it; before, because the plan reads ownership to decide what is drift, and a
    // repair that landed afterwards would be a sync too late every time.
    //
    // Called whether or not anything is interrupted. `needs_recovery` asks about entries that
    // are still open, and an unrecorded package has nothing open about it — so gating the call
    // on that predicate is what left an orphaned package orphaned through every subsequent
    // sync. `heal` returns at once when there is nothing of either kind.
    //
    // `heal` fails when it could not close an entry, and `shall heal` exits non-zero for it.
    // Here it must not: one package whose recovery cannot complete would block every other
    // package on the machine from converging, and the entry stays recorded as interrupted
    // either way, so the next run tries it again.
    let declared: Vec<crate::core::PackageSpec> = desired.values().flatten().cloned().collect();
    if let Err(e) = engine.heal(&declared).await {
        warn!("{e} Continuing with the sync.");
    }

    enforce_policy(app, &desired).await?;

    // SEC3, before the first repo is added and before any package is touched: a `link:` line
    // whose `@target` lands outside the home directory is asked about once. A confirmation
    // offered after the file is placed is a notification.
    app.dotfiles().confirm_outside_home(&state)?;

    // Ordering phase 0 (7c): a manager the configuration declares and this machine lacks is
    // offered before anything is planned — a package cannot install through a manager that is
    // not there, and finding that out per-package is a pile of identical failures.
    app.bootstrap().offer(&state).await?;

    // And the half after it: a manager that IS here and cannot install anything until
    // something is set up — Hex for `mix`, a plugin for `asdf`, a switch for `opam` (Q10, Q11,
    // Q13). After the bootstrap, because probing a manager this machine does not have would
    // ask a question with no answer.
    app.prereqs().offer(&state).await?;

    // Ordering phase 1: repos → refresh indexes. A package from a PPA cannot install until
    // the PPA is added, so this runs before the package plan (not inside it).
    app.repositories().apply(&state).await?;

    // Drift is scoped to the backends this host lists in `priority`: a full sync must not
    // reap a backend you have simply stopped listing.
    let hosts = app.resolver().await.host_backends().await;
    let mut changes = {
        let state_guard = app.state.lock().await;
        let planner = crate::app::sync::planner::ChangePlanner::new(
            app.registry.clone(),
            &state_guard,
            &app.config,
        );
        planner.plan(&desired, PlanScope::Whole(hosts)).await?
    };

    // Before the "nothing to do" exit, never inside it: a plan can be empty of ACTIONS and
    // still have something to say. `sync` printed `already up to date` over a managed,
    // undeclared, protected package it had just declined to remove — and the exit below is the
    // line it returned through (AU1).
    if opts.out.is_human() {
        print_flight_plan(&app.config, &app.registry, &changes);
        // W13: a `vars` edit can be the cause of a removal, so when the plan removes anything,
        // name the variables that changed since the last sync — a hundred removals should never
        // be unexplained.
        if changes.total_remove() > 0 {
            print_vars_changed(&app.config, &app.registry, &app.vcs(), &state.vars).await;
        }
    }

    // The machine-readable plan is emitted here, above the "nothing to do" exit and not below
    // it. It used to sit inside the dry-run block further down, which a converged machine never
    // reached — so `sync --dry-run --json` answered every question except the one asked of it
    // most, "is this machine already in sync?", and answered that one with the words
    // `already up to date` where a document was expected.
    if app.config.dry_run && opts.out.is_json() {
        println!(
            "{}",
            serde_json::to_string_pretty(&changes.generate_report())?
        );
    }

    // A config can be all dependents/schedules and no package changes (just a `service:` or a
    // `schedule:` line). That is still work, so the "nothing to do" exit has to account for
    // every phase after the package plan — which it now asks `Phase` for rather than listing.
    if changes.is_empty() && !state.has_non_package_work() {
        // Even with no packages/dependents/schedules to apply, an extra may have been
        // *removed* — deleting the last `service:` line is a real change (S20). Reconcile the
        // applied-extras ledger so that undo still happens; it is a cheap no-op otherwise.
        //
        // The count is returned rather than dropped: a teardown is work, and reporting `already
        // up to date` over five deleted files is the summary disagreeing with the machine. The
        // placement half is counted here too — `repo:` is phase 1 and was applied above, so a
        // run that reaches this line can still have put a resource in place.
        let resources = app.extras().changes(&state).await?;
        let undone = app.extras().reconcile(&state, opts.scope).await?;
        return Ok(Reconciled {
            applied: resources.place.len() + undone,
            left_in_place: changes.skipped.len(),
        });
    }

    let applied = changes.total_install() + changes.total_remove();
    // Read before the plan is consumed by the engine below.
    let left_in_place = changes.skipped.len();

    // XIII.3: a script's decision is printed before anything happens — the hash, how many
    // times that content has run, and what this run will therefore do. Outside the
    // `!changes.is_empty()` block on purpose: a config whose only work is an `exec:` still has
    // to show it.
    if opts.out.is_human() {
        app.execs().print_plan(&state);
    }

    // Dry-run is preview-only: never prompt, never mutate. (The report went out above, on the
    // path a converged machine also takes.)
    if app.config.dry_run {
        // The same phases a real run would perform, in the same order, from the same list —
        // each honours `dry_run` itself and previews instead of acting.
        let extras_undone = apply_non_package_phases(app, &state, opts.scope).await?;
        return Ok(Reconciled {
            applied: applied + extras_undone,
            left_in_place: changes.skipped.len(),
        });
    }

    // The package plan runs only when it has something in it — a dependents-only sync skips
    // straight to phase 3, with no planned-changes list and no confirmation to answer.
    if !changes.is_empty() {
        // Interactive confirmation — but only with a real terminal. A non-interactive caller
        // (pipe/CI/script) must pass --yes (or --json); otherwise we neither hang on a TUI
        // that can't receive input nor silently apply unconfirmed changes.
        if opts.confirm && !app.config.yes && opts.out.is_human() {
            use std::io::IsTerminal;
            if !std::io::stdin().is_terminal() {
                return Err(crate::core::Error::Refused(
                    "Refusing to apply changes without confirmation in a non-interactive shell. Re-run with --yes to proceed, or --dry-run to preview."
                .to_string()).into());
            }
            let mut preview = TuiPreview::new(&changes, HashMap::new());
            if !crate::core::on_the_terminal(|| preview.run())? {
                return Ok(Reconciled {
                    applied: 0,
                    left_in_place: changes.skipped.len(),
                });
            }
            changes = preview.get_filtered_changes();
        }

        // Read before the plan is consumed, and used after the sync succeeds: a warning about
        // a package that failed to install would be answering a question nobody reached.
        //
        // The removal count is no longer read here. It used to be, and passed down to the
        // teardown so `max_removals` was a ceiling on the command — a number two callers
        // assembled correctly and one passed as `0` (`S55`). `app.reaping` is that number now,
        // written where the guard clears a set, so it counts what was actually removed rather
        // than what a caller believed had been.
        let installed_by = backends_that_installed(&changes);
        engine.sync(changes, opts.scope).await?;
        warn_about_unreachable_binaries(&app.config, &app.executor, &installed_by).await;
    }

    let extras_undone = apply_non_package_phases(app, &state, opts.scope).await?;
    perform_maintenance(app).await?;
    Ok(Reconciled {
        applied: applied + extras_undone,
        left_in_place,
    })
}

/// Which managers this plan installs through, each named once.
fn backends_that_installed(changes: &crate::app::sync::planner::SyncChanges) -> Vec<String> {
    let mut out: Vec<String> = changes
        .generate_report()
        .install
        .into_iter()
        .map(|e| e.backend)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// A package that installed and cannot be run is reported as a success (E6c).
///
/// Here rather than in each backend, because the fact is about the ecosystem's convention and
/// eleven copies of it is eleven chances to disagree. Once per manager, not once per package:
/// installing forty rocks must not print the same paragraph forty times.
async fn warn_about_unreachable_binaries(
    config: &Config,
    executor: &crate::core::CommandExecutor,
    backends: &[String],
) {
    // Each of these runs the manager to ask where it puts binaries (`npm prefix -g`, `go env
    // GOPATH`, …) — a subprocess per backend, on the sync path, for something purely
    // informational. Ordered, so the warnings print in the order the backends were named.
    use futures::stream::StreamExt;
    let messages: Vec<Option<String>> = futures::stream::iter(backends.iter())
        .map(|be| crate::app::reachable::unreachable_warning(be, config, executor))
        .buffered(config.max_parallel.max(1))
        .collect()
        .await;
    for message in messages.into_iter().flatten() {
        warn!("{}", message);
    }
}

/// Everything a sync does after the package plan, in II.7's order.
///
/// **The list is `Phase`, and the order is `Phase`'s order.** It was a hand-written sequence
/// of calls, and before that two of them — the dry-run branch kept its own copy — and every
/// statement kind added since was missed by one of them: extras (S20), then `exec:`, then
/// `dotfiles:`, then `firewall:`. Four times is not four mistakes, it is one list nothing
/// checked. The match below is exhaustive over the enum, so a phase added to the grammar
/// cannot compile until this function says what to do with it, and it runs in the enum's
/// declaration order rather than in the order somebody typed the calls.
///
/// Each phase honours `dry_run` internally and previews rather than acting, which is what
/// makes one list correct for the preview and the real run alike.
pub async fn apply_non_package_phases(
    app: &App,
    state: &crate::model::DesiredState,
    scope: crate::app::sync::guard::GuardScope,
) -> Result<usize> {
    use crate::config::grammar::Phase;

    // Asked before the first phase runs, because afterwards the answer is zero: the resources
    // are in effect, `changes()` correctly reports nothing to do, and a summary reading that
    // back is how `sync` placed three files and printed `already up to date` (N-2). The
    // teardown half is counted from what `reconcile` actually attempted, below.
    let resources = app.extras().changes(state).await?;

    // Placing a resource takes nothing away, so it answers to no ceiling of its own — but it is
    // a change, and `max_total_changes` counts changes (`N8`). Here because this is the one
    // place that knows how many before any of them happen; the five appliers below each know
    // only their own share, and five gates is five chances to add a sixth applier without one.
    //
    // **Not on a preview**, and this is the opposite of the teardown gate one function over,
    // which deliberately runs on both. A dry run reaches here without the engine having run, so
    // the ledger holds no packages and no installs — the total this gate could compute is a
    // fraction of the one the real run computes, and a preview that measures a smaller number
    // says *yes* where the run says *no*. `shall plan` previews the ceilings properly, over one
    // ledger in the engine's own order (`guard::preview_refusals`); a half-answer here would be
    // the second implementation of that, and the wrong one.
    if !app.config.dry_run {
        crate::app::sync::guard::enforce_additions(
            &app.config,
            resources.place.len(),
            &app.reaping,
            scope,
        )
        .await?;
    }

    for phase in Phase::all() {
        match phase {
            // Not this list's, and each for a reason rather than by omission: `Resolution` is
            // consumed before a desired state exists, `Repositories` ran before the package
            // plan (a package from a PPA cannot install until the PPA is there), and
            // `Packages` is the transaction the engine closed above.
            Phase::Resolution | Phase::Repositories | Phase::Packages => {}
            // Phase 3: the dependent extras, now that every package they lean on is in.
            Phase::Dependents => app.dependents().apply(state).await?,
            // Phase 3b (7n): the dotfiles trees — a tree is a pile of `link:` lines and
            // belongs where they do.
            Phase::Dotfiles => app.dotfiles().apply(state).await?,
            // Phase 3c (Part XI): the perimeter. After the packages, because a rule usually
            // exists to let something in that was just installed — and its lockout check runs
            // before any command it would issue, on this path and on the unattended one alike.
            Phase::Firewall => app.firewall().apply(state, scope).await?,
            // Phase 4 (S21): provision the declared schedules onto the OS scheduler.
            Phase::Schedules => app.schedules().apply(state).await?,
            // Phase 4b (XIII.3): the declared `exec:` scripts, after the packages and
            // dependents a script is likely to lean on. A verb, so it has no teardown phase.
            Phase::Execs => app.execs().apply(state).await?,
        }
    }

    // The teardown (S20): undo extras that were applied before but are no longer declared.
    // Not a `Phase` — a phase is where a *declaration's* work happens, and this is the half
    // that runs on the declarations that are gone.
    //
    // It runs after `Phase::Firewall`. Since `N8` the two spend different budgets — ports answer
    // to `max_port_closures`, resources to `max_extra_removals` — but `app.reaping` carries both
    // into this call, because they still spend one `max_total_changes` between them.
    let undone = app.extras().reconcile(state, scope).await?;
    Ok(resources.place.len() + undone)
}

/// `shall rebuild` — remove and reinstall what is declared, one backend at a time (X.1, K1).
pub async fn handle_rebuild(
    app: &App,
    packages: &[String],
    backend: Option<&str>,
    all: bool,
) -> Result<()> {
    use crate::app::rebuild::{self, Scope};
    use crate::app::sync::guard::{self, GuardScope};

    // Before the warning about rebuilding everything: `rebuild --backend aptt` scoped to a
    // manager that does not exist, found nothing to rebuild, and said it had succeeded (Q9).
    app.resolver().await.require_known_backend(backend)?;
    // The positional form of the same ruling: `rebuild nosuchbackend:foo` answered
    // "skipping — not declared in any active module" at exit 0.
    app.resolver()
        .await
        .require_known_spec_backends(packages)
        .await?;

    // K2 (ruled 2026-07-24): a bare `rebuild` WARNS and rebuilds everything, rather than
    // refusing. The default is `--all`, but because the failure mode is software missing from a
    // machine, arriving there by pressing enter is announced loudly first — the warning is the
    // safeguard, not a refusal.
    let scope = match (packages.is_empty(), backend, all) {
        (_, Some(b), _) => Scope::Backend(b.to_string()),
        (_, None, true) => Scope::All,
        (false, None, false) => {
            let registry = app.registry.clone();
            Scope::Packages(
                packages
                    .iter()
                    .map(|p| rebuild::Target::parse(p, |b| registry.get(b).is_some()))
                    .collect(),
            )
        }
        (true, None, false) => {
            warn!(
                "rebuild with no scope rebuilds EVERY declared package on this machine — it \
                 removes software in order to put it back. Proceeding with `--all`.\n  \
                 Narrow it with `shall rebuild <pkg>` or `shall rebuild --backend <name>` if \
                 that is not what you meant."
            );
            Scope::All
        }
    };

    let resolver =
        crate::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let desired = resolver.resolve_desired_state().await?;
    // A rebuild reinstalls, so it is a change path and the `[guard]` gate applies. Checked
    // against the declared set before anything is removed — a `deny_packages` hit must stop
    // the removal, not be discovered between the removal and the reinstall.
    enforce_policy(app, &desired).await?;
    let declared: Vec<crate::core::PackageSpec> = desired.into_values().flatten().collect();

    let priority = app.backends().await.names()?;
    let registry = app.registry.clone();
    let is_foundation = |b: &str| registry.get(b).map(|m| m.needs_root()).unwrap_or(false);

    let mut plan = {
        let state = app.state.lock().await;
        rebuild::plan(
            &scope,
            &declared,
            &|backend, name| state.is_managed(backend, name),
            &priority,
            &is_foundation,
        )
    };

    // The guard refuses to remove a protected package, and it is right to: a rebuild's removal
    // is only safe because a reinstall follows, and if that reinstall fails the machine is
    // genuinely without it. Narrow the scope here rather than ask the guard for an exception —
    // `rebuild --all` stays usable on a machine whose `bash` is protected, and the refusal
    // keeps meaning what it says.
    {
        let all_pairs: Vec<(String, String)> = plan
            .batches
            .iter()
            .flat_map(|b| b.specs.iter().map(|s| (b.backend.clone(), s.name.clone())))
            .collect();
        let backends: std::collections::HashSet<String> =
            all_pairs.iter().map(|(b, _)| b.clone()).collect();
        let essential =
            guard::essential_names(&app.registry, &backends, app.config.max_parallel).await;
        rebuild::without_protected(&mut plan, &|backend, name| {
            guard::protection_of(&app.config, Some(backend), name, &essential).map(|p| p.reason())
        });
    }

    for skip in &plan.skipped {
        println!("skipping {} — {}", skip.key, skip.reason);
    }
    if plan.is_empty() {
        println!("nothing to rebuild.");
        return Ok(());
    }

    println!(
        "\nRebuilding {} package(s) across {} backend(s), one backend at a time:",
        plan.total(),
        plan.batches.len()
    );
    for batch in &plan.batches {
        println!("  {:<10} {}", batch.backend, batch.names().join(" "));
    }
    println!(
        "\nEach backend's packages are removed together, then reinstalled together. If a \
         reinstall fails,\nthe whole rebuild rolls back to a snapshot taken before the first \
         removal — or, where no\nsnapshot provider exists, stops and names what is missing."
    );

    if app.config.dry_run {
        return Ok(());
    }

    let proceed = crate::core::prompt::confirm(
        app.config.yes,
        "Remove and reinstall these packages?",
        crate::core::prompt::Unattended::Refuse(
            "Refusing to rebuild without confirmation in a non-interactive shell. Re-run with --yes, or --dry-run to preview.",
        ),
    )?;
    if !proceed {
        return Ok(());
    }

    // K3: a rebuild removes before it installs, so a failed reinstall leaves the machine
    // missing declared software. The snapshot is taken before the first removal, because a
    // snapshot taken per batch could only restore the batch that failed — and by then an
    // earlier batch may already have been rebuilt on top of it.
    let snapshot = match app
        .snapshot_manager
        .auto_snapshot(crate::core::snapshot::SnapshotLabel::PreRebuild)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            warn!("could not take a pre-rebuild snapshot ({}).", e);
            None
        }
    };
    match &snapshot {
        Some(s) => info!(
            "snapshot {} taken; a failed reinstall rolls back to it.",
            s.id
        ),
        None => warn!(
            "no snapshot provider here, so a failed reinstall cannot be rolled back \
             automatically."
        ),
    }

    let engine = app.sync_engine();
    for batch in &plan.batches {
        info!(
            "rebuilding {} ({} package(s))",
            batch.backend,
            batch.specs.len()
        );

        // Removal and reinstall are two transactions, not one graph. The transaction engine
        // runs independent nodes concurrently, and a Remove and an Install of the same package
        // have no edge between them — in one graph they would race.
        let mut down = crate::app::sync::planner::SyncChanges::default();
        for spec in &batch.specs {
            down.add_removal(&batch.backend, &spec.name);
        }
        engine.sync(down, GuardScope::Rebuild).await?;

        // `add_installs`, not a loop of `add_node`: a `@requires` between two packages of the
        // same backend is exactly what a rebuild has to honour, and this loop keyed its map by
        // the bare name — so the lookup `requires` does, which is `backend:name`, could never
        // hit it and the graph came out edgeless.
        let mut up = crate::app::sync::planner::SyncChanges::default();
        up.add_installs(&batch.specs);
        // The removal has already happened, so a failure here means the batch's software is
        // gone. Roll the whole rebuild back rather than leaving a half-rebuilt machine.
        if let Err(e) = engine.sync(up, GuardScope::Rebuild).await {
            let Some(snap) = &snapshot else {
                anyhow::bail!(
                    "rebuild of `{}` failed while reinstalling: {}\n\n\
                     These packages were removed and are NOT back:\n    {}\n\n\
                     There was no snapshot to roll back to. Re-run \
                     `shall rebuild --backend {}` once the cause is fixed.\n\
                     Remaining backends were not started.",
                    batch.backend,
                    e,
                    batch.names().join(" "),
                    batch.backend
                );
            };
            warn!(
                "rebuild of `{}` failed while reinstalling ({}); rolling back to snapshot {}...",
                batch.backend, e, snap.id
            );
            // A failed restore is the worse outcome and must not be reported as a rollback:
            // the machine is then both half-rebuilt and un-restored, and the user needs to
            // know that rather than be told it was handled.
            if let Err(restore_err) = app.snapshot_manager.restore_snapshot(&snap.id).await {
                anyhow::bail!(
                    "rebuild of `{}` failed while reinstalling: {}\n\
                     AND the rollback to snapshot {} failed: {}\n\n\
                     These packages were removed and are NOT back:\n    {}\n\n\
                     Restore snapshot {} by hand before doing anything else.",
                    batch.backend,
                    e,
                    snap.id,
                    restore_err,
                    batch.names().join(" "),
                    snap.id
                );
            }
            anyhow::bail!(
                "rebuild of `{}` failed while reinstalling: {}\n\n\
                 Rolled back to snapshot {} — the machine is as it was before the rebuild \
                 started.\nRe-run `shall rebuild --backend {}` once the cause is fixed.",
                batch.backend,
                e,
                snap.id,
                batch.backend
            );
        }
    }

    println!("rebuild complete.");
    Ok(())
}

/// The two things a caller can ask a sync to do differently. A struct rather than two
/// parameters because `handle_sync(app, false, false, ..)` was three positional booleans in a
/// row at four call sites, where transposing `locked` and `upgrade` compiles and converges the
/// machine to the other answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SyncMode {
    /// Strict version matching against the lockfile: a package that is not in it is an error.
    pub locked: bool,
    /// Take what the managers offer now instead of what the lock recorded.
    pub upgrade: bool,
}

pub async fn handle_sync(app: &App, mode: SyncMode, out: Output) -> Result<()> {
    let SyncMode { locked, upgrade } = mode;
    let done = reconcile(
        app,
        Reconcile {
            locked,
            upgrade,
            out,
            scope: crate::app::sync::guard::GuardScope::Sync,
            confirm: true,
        },
    )
    .await?;
    // `--upgrade` is the run that means to move forward, so it records where it landed. Without
    // this the pins still name the versions it just replaced, and the next ordinary sync — which
    // converges to the lock — plans every one of them back down (Z2).
    if upgrade {
        let moved =
            crate::verbs::plan::refresh_version_locks(&app.config, &app.registry, &app.state)
                .await?;
        if moved > 0 && out.is_human() {
            println!("Lock: re-recorded {} version pin(s).", moved);
        }
    }
    // Never over a skip. `already up to date` is a claim about the machine, and the machine
    // holds a package this run decided not to remove — which the lines above have just named
    // (AU1). The claim was made, in that exact state, three times: here, in `uninstall`, and
    // in `check`.
    // Not under `--json`: the answer there is the document, and a sentence after it is a second
    // answer in a second language that no consumer can read.
    if done.applied == 0 && done.left_in_place == 0 && out.is_human() {
        println!("already up to date");
    }
    Ok(())
}

/// A cheap fingerprint of the manifest directory: (path, size, mtime) for every `*.txt`. If it
/// changes between ticks, a manifest was edited. Best-effort — errors just yield an empty sig.
/// A fingerprint of every wish-list manifest, so `watch` notices an edit.
///
pub async fn manifest_signature(dir: &std::path::Path) -> Vec<(String, u64, i64)> {
    let mut sig = Vec::new();
    {
        let Ok(mut rd) = tokio::fs::read_dir(dir).await else {
            return sig;
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let path = entry.path();
            if path.extension().map(|e| e == "txt").unwrap_or(false) {
                if let Ok(meta) = entry.metadata().await {
                    let mtime = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    sig.push((path.to_string_lossy().into_owned(), meta.len(), mtime));
                }
            }
        }
    }
    sig.sort();
    sig
}

/// One unattended reconcile pass. `watch` is unattended by definition, so it never asks —
/// that flag is the only thing separating it from `sync`, which is why both go through the
/// same [`reconcile`].
///
/// **The data lock is taken here, per tick, and released before the sleep** (`LockScope::
/// Deferred`). `watch` is the GitOps deployment: the documented use is to leave it running. It
/// was a whole-run writer, so for as long as the daemon was up, every writing Shall command on
/// that machine — `install`, `sync`, and the `hook-reconcile` a hand-typed `apt install` fires —
/// waited 120 seconds and then failed. A user following the documentation disabled their own
/// CLI. The tick is the mutating action; the sleep between ticks is not.
pub async fn watch_reconcile(app: &App) -> Result<Reconciled> {
    let _data_lock = crate::core::datalock::DataLock::for_one_step("watch").await?;
    reconcile(
        app,
        Reconcile {
            locked: false,
            // `watch` is `sync` with nobody watching, so it converges the same way: to the
            // versions the lock recorded. Moving forward is a decision, and 3am is the worst
            // time to make one nobody asked for (owner ruling, 2026-07-24).
            upgrade: false,
            out: Output::Human,
            scope: crate::app::sync::guard::GuardScope::Watch,
            confirm: false,
        },
    )
    .await
}

pub async fn handle_watch(
    app: &App,
    interval: u64,
    on_change: bool,
    pull: bool,
    once: bool,
) -> Result<()> {
    let interval = interval.max(1);
    println!(
        "shall watch: reconciling {} every {}s{}{}. Ctrl-C to stop.",
        app.config.config_root().display(),
        interval,
        if pull { " (git pull each tick)" } else { "" },
        if on_change { " (on change only)" } else { "" },
    );
    let mut last_sig = manifest_signature(&app.config.config_root().join("modules")).await;
    let mut first = true;
    let mut failed: Option<anyhow::Error> = None;
    loop {
        if pull {
            let git = app.vcs().manager();
            if git.is_repo() {
                match git.pull() {
                    Ok(msg) => info!("watch: git pull — {}", msg.lines().last().unwrap_or("")),
                    Err(e) => warn!("watch: git pull failed: {}", e),
                }
            }
        }
        let sig = manifest_signature(&app.config.config_root().join("modules")).await;
        let changed = sig != last_sig;
        // Reconcile on the first pass and whenever something changed; with --on-change we skip
        // ticks where nothing moved (the manifests and, after a pull, the repo are unchanged).
        if first || changed || !on_change {
            if changed && !first {
                println!("watch: manifests changed — reconciling.");
            }
            match watch_reconcile(app).await {
                Ok(done) if done.applied == 0 && done.left_in_place == 0 => {
                    if changed || first {
                        println!("watch: already in sync.");
                    }
                }
                Ok(done) if done.applied == 0 => println!(
                    "watch: nothing applied; {} package(s) left in place (listed above).",
                    done.left_in_place
                ),
                Ok(done) => println!("watch: applied {} change(s).", done.applied),
                Err(e) => {
                    warn!("watch: reconcile failed: {}", e);
                    failed = Some(e);
                }
            }
            last_sig = sig;
        }
        first = false;
        if once {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
    }
    // A `watch --once` is read by its exit code and by nothing else — that is what a cron entry
    // or a timer unit is. Warning that the reconcile failed and then exiting 0 tells the
    // scheduler the opposite of what it told the log, on the surface with the fewest readers
    // (U21's exit vocabulary; Q28's class).
    //
    // The looping form never reaches here: one failed tick is not a reason to stop reconciling,
    // and the warning is what a long-running watch has always had. The sibling one line up —
    // `git pull failed` — stays a warning on purpose: the reconcile after it still converged the
    // machine to the manifests this host holds, which is what `watch` promises.
    match failed {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Enforce the `[guard]` install/change rules against the desired state before any change
/// (II.10). The spec-level rules (`deny_packages`, `pinned_only`) are checked purely by the
/// guard; the two that need runtime state (`require_snapshot`, `deny_vulnerable`) are checked
/// here, where the snapshot provider and the audit report are in hand. All ten refusals now
/// share one decision surface — this replaces the old parallel `policy.toml` gate (II.17).
/// Every `[guard]` install/change rule this desired state violates.
///
/// **Split out so `shall policy` can preview exactly what `sync` will enforce.**
/// `verbs/setup.rs:637` used to re-implement this — the same `inspect_desired` call, the same
/// `require_snapshot` check, and **no `deny_vulnerable`** — and then printed a footnote at `:646`
/// admitting the gap: *"(deny_vulnerable is also enforced at sync time via `shall check
/// security`.)"*
///
/// So `shall policy` could report **compliant** for a config `sync` would refuse. That is not an
/// argument for deleting a preview; it is an argument that the preview was not calling the thing
/// it previews. One implementation, two callers, and the footnote deletes itself.
pub async fn policy_violations(
    app: &App,
    desired: &HashMap<String, Vec<crate::core::PackageSpec>>,
) -> Vec<String> {
    let guard = &app.config.guard;
    if guard.is_empty() {
        return Vec::new();
    }
    let mut violations: Vec<String> = crate::app::sync::guard::inspect_desired(guard, desired)
        .iter()
        .map(crate::app::sync::guard::describe_objection)
        .collect();
    if guard.require_snapshot && !app.snapshot_manager.has_provider() {
        violations
            .push("requires a snapshot provider but none is available (require_snapshot)".into());
    }
    if guard.deny_vulnerable {
        match crate::app::insight::audit(&app.config, &app.registry, &app.state).await {
            Ok(report) => {
                for f in report.findings {
                    violations.push(format!(
                        "{}:{} — known vulnerability {} (deny_vulnerable)",
                        f.backend, f.name, f.id
                    ));
                }
            }
            Err(e) => warn!("vulnerability check skipped ({}).", e),
        }
    }
    violations
}

pub async fn enforce_policy(
    app: &App,
    desired: &HashMap<String, Vec<crate::core::PackageSpec>>,
) -> Result<()> {
    let violations = policy_violations(app, desired).await;
    if violations.is_empty() {
        return Ok(());
    }
    eprintln!("Blocked by [guard] ({} violation(s)):", violations.len());
    for v in &violations {
        eprintln!("  - {}", v);
    }
    Err(anyhow::anyhow!(
        "guard rules prevent this operation; nothing was changed"
    ))
}

/// A concise pre-flight summary of what a sync/upgrade is about to do. Real download-size
/// and time estimates are backend-specific and deliberately not faked.
pub fn print_flight_plan(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    changes: &crate::app::sync::planner::SyncChanges,
) {
    if config.quiet {
        return;
    }
    let report = changes.generate_report();
    // The skips print even when there is nothing else to print, and that is the whole point:
    // an empty plan over a machine holding a package Shall declined to remove is the run that
    // said `already up to date` about a wedge (AU1).
    if report.install.is_empty() && report.remove.is_empty() {
        print_skipped(&report.skipped);
        return;
    }
    let mut backends: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut needs_root = false;
    let mut service_ops = 0;
    for e in report.install.iter().chain(report.remove.iter()) {
        backends.insert(e.backend.clone());
        if let Some(b) = registry.get(&e.backend) {
            if b.needs_root() {
                needs_root = true;
            }
        }
        if e.backend == "service" {
            service_ops += 1;
        }
    }
    println!("Planned changes:");
    println!(
        "  install {}   remove {}   (total {} change(s))",
        report.install.len(),
        report.remove.len(),
        report.install.len() + report.remove.len()
    );
    println!(
        "  backends: {}",
        backends.into_iter().collect::<Vec<_>>().join(", ")
    );
    if needs_root {
        println!("  privileges: some operations require root/sudo");
    }
    if service_ops > 0 {
        println!(
            "  services: {} change(s) may restart running services",
            service_ops
        );
    }
    print_skipped(&report.skipped);
}

/// How many skips are named individually before the rest become a count. The guard uses the
/// same ceiling for the same reason: a machine that stops listing a backend with two hundred
/// packages in it would otherwise bury the plan under the list of what is NOT happening.
const MAX_LISTED_SKIPS: usize = 10;

/// The removals the plan declined, each with the reason that item carries.
///
/// Free-standing so that every surface showing a plan shows the same lines — `sync`, its
/// preview and `prune` all reach it, and a fourth caller added later gets it by calling this
/// rather than by remembering the rule.
pub fn print_skipped(skipped: &[crate::app::sync::planner::Skipped]) {
    if skipped.is_empty() {
        return;
    }
    println!(
        "Left in place ({}) — installed, declared nowhere, and not removed:",
        skipped.len()
    );
    for item in skipped.iter().take(MAX_LISTED_SKIPS) {
        println!("  ~ {}  ({})", item.key, item.reason);
    }
    if skipped.len() > MAX_LISTED_SKIPS {
        println!("  … and {} more", skipped.len() - MAX_LISTED_SKIPS);
    }
    println!(
        "  Declare them to keep them, or run `shall protected <name>` to see what decides this."
    );
}

/// W13: name the variables whose value changed since the last successful sync (HEAD), so a
/// removal driven by a `vars` edit is explained rather than presented as a bare count. Compares
/// this run's resolved variables to the committed baseline; silent when nothing changed or there
/// is no baseline (a fresh repo, or a script/program provider whose values do not commit).
pub async fn print_vars_changed(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    vcs: &crate::app::Vcs<'_>,
    current: &crate::model::vars::Vars,
) {
    let resolver =
        crate::app::sync::resolver::StateResolver::new(config, registry.clone(), false).await;
    let git = vcs.manager();
    let prev = match resolver.vars_at_last_sync(&git).await {
        Ok(Some(p)) => p,
        _ => return,
    };
    let changes = crate::model::vars::diff(&prev, current);
    if changes.is_empty() {
        return;
    }
    println!("  variables changed since the last sync:");
    for (name, before, after) in changes {
        match (before, after) {
            (Some(a), Some(b)) => println!("    ${}  {} → {}", name, a, b),
            (None, Some(b)) => println!("    ${}  (new) {}", name, b),
            (Some(a), None) => println!("    ${}  {} → (gone)", name, a),
            (None, None) => {}
        }
    }
}
