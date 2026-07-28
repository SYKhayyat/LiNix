use crate::verbs::prelude::*;

// ============================================================================
// COMMAND HANDLERS
// ============================================================================

/// How one reconcile pass should behave. The pass itself is identical for `sync` and
/// `watch` — II.7's ordering phases, the guard, the same planner — and these are the only
/// things that legitimately differ between an attended run and an unattended one.
pub(crate) struct Reconcile {
    /// Strict version matching against the lockfile: a package that is not in it is an error.
    locked: bool,
    /// Take what the managers offer now instead of what the lock recorded. Off by default —
    /// a sync converges to what was decided (owner ruling, 2026-07-24).
    upgrade: bool,
    /// Emit the change report as JSON instead of a planned-changes list.
    json: bool,
    /// Which scope the guard reports refusals under.
    scope: linix::app::sync::guard::GuardScope,
    /// Whether to ask before applying. `watch` is unattended by definition and never asks;
    /// `sync` asks unless `--yes`.
    confirm: bool,
}

/// What to call this run in a refusal a user reads — the difference that matters is whether
/// anybody was there, because an unattended tick is the dangerous one (N7).
pub(crate) fn scope_label(scope: linix::app::sync::guard::GuardScope) -> &'static str {
    match scope {
        linix::app::sync::guard::GuardScope::Watch => "an unattended watch tick",
        _ => "sync",
    }
}

/// One reconcile pass: resolve the model, apply repos, plan, apply, then dependents,
/// schedules and extras — II.7's ordering, in order.
///
/// Returns the number of package changes applied. `sync` and `watch` both call this; the
/// copy `watch` used to carry drifted from this body every time sync's ordering changed,
/// which is why it is one function now.
pub(crate) async fn reconcile(app: &App, opts: Reconcile) -> Result<usize> {
    let engine = app.sync_engine().await;
    if app.journal.lock().await.needs_recovery() {
        warn!("the transaction journal records an interrupted run; healing first.");
        engine.heal().await?;
    }

    let mut resolver = linix::app::sync::resolver::StateResolver::new(
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
    enforce_policy(app, &desired).await?;

    // SEC3, before the first repo is added and before any package is touched: a `link:` line
    // whose `@target` lands outside the home directory is asked about once. A confirmation
    // offered after the file is placed is a notification.
    app.dotfiles().confirm_outside_home(&state)?;

    // Ordering phase 0 (7c): a manager the configuration declares and this machine lacks is
    // offered before anything is planned — a package cannot install through a manager that is
    // not there, and finding that out per-package is a pile of identical failures.
    app.bootstrap().offer(&state).await?;

    // Ordering phase 1: repos → refresh indexes. A package from a PPA cannot install until
    // the PPA is added, so this runs before the package plan (not inside it).
    app.repositories().apply(&state).await?;

    // Drift is scoped to the backends this host lists in `priority`: a full sync must not
    // reap a backend you have simply stopped listing.
    let enabled = app.priority_backends().await;
    let mut changes = {
        let state_guard = app.state.lock().await;
        let planner = linix::app::sync::planner::ChangePlanner::new(
            app.registry.clone(),
            &state_guard,
            &app.config,
        )
        .with_enabled(enabled);
        planner.plan(&desired, None).await?
    };

    // A config can be all dependents/schedules and no package changes (just a `service:` or a
    // `schedule:` line). That is still work, so the "nothing to do" exit has to account for
    // the dependent phase and the schedule phase too.
    if changes.is_empty() && !state.has_non_package_work() {
        // Even with no packages/dependents/schedules to apply, an extra may have been
        // *removed* — deleting the last `service:` line is a real change (S20). Reconcile the
        // applied-extras ledger so that undo still happens; it is a cheap no-op otherwise.
        app.extras().reconcile(&state).await?;
        return Ok(0);
    }

    let applied = changes.total_install() + changes.total_remove();

    if !opts.json && !changes.is_empty() {
        print_flight_plan(app, &changes);
        // W13: a `vars` edit can be the cause of a removal, so when the plan removes anything,
        // name the variables that changed since the last sync — a hundred removals should never
        // be unexplained.
        if changes.total_remove() > 0 {
            print_vars_changed(app, &state.vars).await;
        }
    }

    // XIII.3: a script's decision is printed before anything happens — the hash, how many
    // times that content has run, and what this run will therefore do. Outside the
    // `!changes.is_empty()` block on purpose: a config whose only work is an `exec:` still has
    // to show it.
    if !opts.json {
        app.execs().print_plan(&state);
    }

    // Dry-run is preview-only: never prompt, never mutate. (JSON dry-run emits the report.)
    if app.config.dry_run {
        if opts.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&changes.generate_report())?
            );
        }
        // The same phases a real run would perform, in the same order, from the same list —
        // each honours `dry_run` itself and previews instead of acting.
        apply_non_package_phases(app, &state, opts.scope).await?;
        return Ok(applied);
    }

    // The package plan runs only when it has something in it — a dependents-only sync skips
    // straight to phase 3, with no planned-changes list and no confirmation to answer.
    if !changes.is_empty() {
        // Interactive confirmation — but only with a real terminal. A non-interactive caller
        // (pipe/CI/script) must pass --yes (or --json); otherwise we neither hang on a TUI
        // that can't receive input nor silently apply unconfirmed changes.
        if opts.confirm && !app.config.yes && !opts.json {
            use std::io::IsTerminal;
            if !std::io::stdin().is_terminal() {
                return Err(linix::core::Error::Refused(
                    "Refusing to apply changes without confirmation in a non-interactive shell. Re-run with --yes to proceed, or --dry-run to preview."
                .to_string()).into());
            }
            let mut preview = TuiPreview::new(&changes, HashMap::new());
            if !preview.run()? {
                return Ok(0);
            }
            changes = preview.get_filtered_changes();
        }

        // Read before the plan is consumed, and used after the sync succeeds: a warning about
        // a package that failed to install would be answering a question nobody reached.
        let installed_by = backends_that_installed(&changes);
        engine.sync(changes, opts.scope).await?;
        warn_about_unreachable_binaries(&installed_by);
    }

    apply_non_package_phases(app, &state, opts.scope).await?;
    perform_maintenance(app).await?;
    Ok(applied)
}

/// Which managers this plan installs through, each named once.
fn backends_that_installed(changes: &linix::app::sync::planner::SyncChanges) -> Vec<String> {
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
fn warn_about_unreachable_binaries(backends: &[String]) {
    for be in backends {
        if let Some(message) = linix::app::reachable::unreachable_warning(be) {
            warn!("{}", message);
        }
    }
}

/// Everything a sync does after the package plan, in II.7's order.
///
/// **One list, called by the preview and by the real run.** It used to be two — the dry-run
/// branch kept its own copy — and every statement kind added since was missed by one of them:
/// extras (S20), then `exec:`, then `dotfiles:`, then `firewall:`. Four times is not four
/// mistakes, it is one duplicated list. Each phase honours `dry_run` internally and previews
/// rather than acting, which is what makes a single list correct for both callers.
pub(crate) async fn apply_non_package_phases(
    app: &App,
    state: &linix::model::DesiredState,
    scope: linix::app::sync::guard::GuardScope,
) -> Result<()> {
    // Phase 3: the dependent extras, now that every package they lean on is in.
    app.dependents().apply(state).await?;
    // Phase 3b (7n): the dotfiles trees — a tree is a pile of `link:` lines and belongs where
    // they do.
    app.dotfiles().apply(state).await?;
    // Phase 3c (Part XI): the perimeter. After the packages, because a rule usually exists to
    // let something in that was just installed — and its lockout check runs before any command
    // it would issue, on this path and on the unattended one alike.
    app.firewall().apply(state, scope_label(scope)).await?;
    // Phase 4 (S21): provision the declared schedules onto the OS scheduler.
    app.schedules().apply(state).await?;
    // Phase 4b (XIII.3): the declared `exec:` scripts, after the packages and dependents a
    // script is likely to lean on. A verb, so it has no teardown phase of its own.
    app.execs().apply(state).await?;
    // Phase 5 (S20): undo extras that were applied before but are no longer declared.
    app.extras().reconcile(state).await?;
    Ok(())
}

/// `linix rebuild` — remove and reinstall what is declared, one backend at a time (X.1, K1).
pub(crate) async fn handle_rebuild(
    app: &App,
    packages: &[String],
    backend: Option<&str>,
    all: bool,
) -> Result<()> {
    use linix::app::rebuild::{self, Scope};
    use linix::app::sync::guard::{self, GuardScope};
    use linix::core::transaction::GraphAction;

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
                 Narrow it with `linix rebuild <pkg>` or `linix rebuild --backend <name>` if \
                 that is not what you meant."
            );
            Scope::All
        }
    };

    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let desired = resolver.resolve_desired_state().await?;
    // A rebuild reinstalls, so it is a change path and the `[guard]` gate applies. Checked
    // against the declared set before anything is removed — a `deny_packages` hit must stop
    // the removal, not be discovered between the removal and the reinstall.
    enforce_policy(app, &desired).await?;
    let declared: Vec<linix::core::PackageSpec> = desired.into_values().flatten().collect();

    let priority = app.priority_backends().await;
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
        let essential = guard::essential_names(&app.registry, &backends).await;
        rebuild::without_protected(&mut plan, &|backend, name| {
            guard::protection_of(&app.config, backend, name, &essential).map(|p| p.reason())
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

    if !app.config.yes {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            return Err(linix::core::Error::Refused(
                "Refusing to rebuild without confirmation in a non-interactive shell. Re-run with --yes, or --dry-run to preview."
            .to_string()).into());
        }
        let proceed = dialoguer::Confirm::new()
            .with_prompt("Remove and reinstall these packages?")
            .default(false)
            .interact()?;
        if !proceed {
            return Ok(());
        }
    }

    // K3: a rebuild removes before it installs, so a failed reinstall leaves the machine
    // missing declared software. The snapshot is taken before the first removal, because a
    // snapshot taken per batch could only restore the batch that failed — and by then an
    // earlier batch may already have been rebuilt on top of it.
    let snapshot = match app
        .snapshot_manager
        .auto_snapshot(linix::core::snapshot::SnapshotLabel::PreRebuild)
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

    let engine = app.sync_engine().await;
    for batch in &plan.batches {
        info!(
            "rebuilding {} ({} package(s))",
            batch.backend,
            batch.specs.len()
        );

        // Removal and reinstall are two transactions, not one graph. The transaction engine
        // runs independent nodes concurrently, and a Remove and an Install of the same package
        // have no edge between them — in one graph they would race.
        let mut down = linix::app::sync::planner::SyncChanges::default();
        for spec in &batch.specs {
            down.removal_tracker
                .insert(format!("{}:{}", batch.backend, spec.name));
            down.graph.add_node(GraphAction::Remove {
                name: spec.name.clone(),
                backend: batch.backend.clone(),
            });
        }
        engine.sync(down, GuardScope::Rebuild).await?;

        let mut up = linix::app::sync::planner::SyncChanges::default();
        for spec in &batch.specs {
            let idx = up.graph.add_node(GraphAction::Install(spec.clone()));
            up.install_map.insert(spec.name.clone(), idx);
        }
        // The removal has already happened, so a failure here means the batch's software is
        // gone. Roll the whole rebuild back rather than leaving a half-rebuilt machine.
        if let Err(e) = engine.sync(up, GuardScope::Rebuild).await {
            let Some(snap) = &snapshot else {
                anyhow::bail!(
                    "rebuild of `{}` failed while reinstalling: {}\n\n\
                     These packages were removed and are NOT back:\n    {}\n\n\
                     There was no snapshot to roll back to. Re-run \
                     `linix rebuild --backend {}` once the cause is fixed.\n\
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
                 started.\nRe-run `linix rebuild --backend {}` once the cause is fixed.",
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

pub(crate) async fn handle_sync(app: &App, locked: bool, upgrade: bool, json: bool) -> Result<()> {
    let applied = reconcile(
        app,
        Reconcile {
            locked,
            upgrade,
            json,
            scope: linix::app::sync::guard::GuardScope::Sync,
            confirm: true,
        },
    )
    .await?;
    if applied == 0 {
        println!("already up to date");
    }
    Ok(())
}

/// A cheap fingerprint of the manifest directory: (path, size, mtime) for every `*.txt`. If it
/// changes between ticks, a manifest was edited. Best-effort — errors just yield an empty sig.
/// A fingerprint of every wish-list manifest, so `watch` notices an edit.
///
pub(crate) async fn manifest_signature(dir: &std::path::Path) -> Vec<(String, u64, i64)> {
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
pub(crate) async fn watch_reconcile(app: &App) -> Result<usize> {
    reconcile(
        app,
        Reconcile {
            locked: false,
            // `watch` is `sync` with nobody watching, so it converges the same way: to the
            // versions the lock recorded. Moving forward is a decision, and 3am is the worst
            // time to make one nobody asked for (owner ruling, 2026-07-24).
            upgrade: false,
            json: false,
            scope: linix::app::sync::guard::GuardScope::Watch,
            confirm: false,
        },
    )
    .await
}

pub(crate) async fn handle_watch(
    app: &App,
    interval: u64,
    on_change: bool,
    pull: bool,
    once: bool,
) -> Result<()> {
    let interval = interval.max(1);
    println!(
        "linix watch: reconciling {} every {}s{}{}. Ctrl-C to stop.",
        app.config.config_root().display(),
        interval,
        if pull { " (git pull each tick)" } else { "" },
        if on_change { " (on change only)" } else { "" },
    );
    let mut last_sig = manifest_signature(&app.config.config_root().join("modules")).await;
    let mut first = true;
    loop {
        if pull {
            let git = app.git_manager();
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
                Ok(0) => {
                    if changed || first {
                        println!("watch: already in sync.");
                    }
                }
                Ok(n) => println!("watch: applied {} change(s).", n),
                Err(e) => warn!("watch: reconcile failed: {}", e),
            }
            last_sig = sig;
        }
        first = false;
        if once {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
    }
    Ok(())
}

/// Enforce the `[guard]` install/change rules against the desired state before any change
/// (II.10). The spec-level rules (`deny_packages`, `pinned_only`) are checked purely by the
/// guard; the two that need runtime state (`require_snapshot`, `deny_vulnerable`) are checked
/// here, where the snapshot provider and the audit report are in hand. All ten refusals now
/// share one decision surface — this replaces the old parallel `policy.toml` gate (II.17).
pub(crate) async fn enforce_policy(
    app: &App,
    desired: &HashMap<String, Vec<linix::core::PackageSpec>>,
) -> Result<()> {
    let guard = &app.config.guard;
    if guard.is_empty() {
        return Ok(());
    }
    let mut violations: Vec<String> = linix::app::sync::guard::inspect_desired(guard, desired)
        .iter()
        .map(linix::app::sync::guard::describe_objection)
        .collect();
    if guard.require_snapshot && !app.snapshot_manager.has_provider() {
        violations
            .push("requires a snapshot provider but none is available (require_snapshot)".into());
    }
    if guard.deny_vulnerable {
        match linix::app::insight::audit(app).await {
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
pub(crate) fn print_flight_plan(app: &App, changes: &linix::app::sync::planner::SyncChanges) {
    if app.config.quiet {
        return;
    }
    let report = changes.generate_report();
    if report.install.is_empty() && report.remove.is_empty() {
        return;
    }
    let mut backends: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut needs_root = false;
    let mut service_ops = 0;
    for e in report.install.iter().chain(report.remove.iter()) {
        backends.insert(e.backend.clone());
        if let Some(b) = app.registry.get(&e.backend) {
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
}

/// W13: name the variables whose value changed since the last successful sync (HEAD), so a
/// removal driven by a `vars` edit is explained rather than presented as a bare count. Compares
/// this run's resolved variables to the committed baseline; silent when nothing changed or there
/// is no baseline (a fresh repo, or a script/program provider whose values do not commit).
pub(crate) async fn print_vars_changed(app: &App, current: &linix::model::vars::Vars) {
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let git = app.git_manager();
    let prev = match resolver.vars_at_last_sync(&git).await {
        Ok(Some(p)) => p,
        _ => return,
    };
    let changes = linix::model::vars::diff(&prev, current);
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
