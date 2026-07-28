use crate::verbs::prelude::*;

/// SEC2: once an install finishes, a verified package and an unverified one are
/// indistinguishable on disk. `@unverified` is only a real decision if it stays visible after
/// the fact, so what it bought is listed for as long as the package is installed.
///
/// The heading avoids "downloaded": since Q5 the flag also covers a manager that verifies a
/// signature itself (`helm`), where LiNix downloaded nothing.
const UNVERIFIED_HEADING: &str = "! installed with `@unverified` — nothing checked the bytes";

/// Every managed package whose install skipped a verification. Reads the recorded option and
/// never the backend, so a backend that gains the flag is listed without editing this.
pub(crate) fn unverified_packages(state: &linix::core::StateRegistry) -> Vec<(String, String)> {
    state
        .packages
        .iter()
        .filter(|p| p.options.get("unverified").is_some_and(|v| v == "true"))
        .map(|p| (p.backend.clone(), p.name.clone()))
        .collect()
}

pub(crate) async fn handle_status(app: &App, json: bool) -> Result<()> {
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let state = resolver.resolve_model().await?;
    let desired = state.packages.clone();
    // A deleted `service:`/`link:`/`repo:` line is drift a sync will undo (S20), and `status`
    // that reports only packages says "nothing to do" on the run that disables a service.
    let extras_to_undo = app.extras().drift(&state).await.unwrap_or_default();
    // `status` reports what a full `sync` would do, so it scopes drift the same way.
    let enabled = app.priority_backends().await;
    let changes = {
        let state_guard = app.state.lock().await;
        let planner = linix::app::sync::planner::ChangePlanner::new(
            app.registry.clone(),
            &state_guard,
            &app.config,
        )
        .with_enabled(enabled);
        planner.plan(&desired, None).await?
    };
    let report = changes.generate_report();
    let unmanaged = app.installed_but_unmanaged().await.unwrap_or_default();

    let unverified: Vec<(String, String)> = {
        let state = app.state.lock().await;
        unverified_packages(&state)
    };

    if json {
        let out = serde_json::json!({
            "to_install": report.install,
            "to_remove": report.remove,
            "unmanaged": unmanaged.iter().map(|p| serde_json::json!({"backend": p.backend, "name": p.name})).collect::<Vec<_>>(),
            "unverified": unverified.iter().map(|(b, n)| serde_json::json!({"backend": b, "name": n})).collect::<Vec<_>>(),
            "extras_to_undo": extras_to_undo,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if report.install.is_empty()
        && report.remove.is_empty()
        && unmanaged.is_empty()
        && unverified.is_empty()
        && extras_to_undo.is_empty()
    {
        println!(
            "System matches your manifests; nothing to install, no drift, no unmanaged packages."
        );
        return Ok(());
    }
    if !report.install.is_empty() {
        println!("+ to install ({}):", report.install.len());
        for e in &report.install {
            println!(
                "    {}:{}{}",
                e.backend,
                e.name,
                e.version
                    .as_deref()
                    .map(|v| format!(" @ {}", v))
                    .unwrap_or_default()
            );
        }
    }
    if !report.remove.is_empty() {
        println!("- drift — `sync` would remove ({}):", report.remove.len());
        for e in &report.remove {
            println!("    {}:{}", e.backend, e.name);
        }
    }
    if !unmanaged.is_empty() {
        println!(
            "? unmanaged — installed but not in your manifests ({}):",
            unmanaged.len()
        );
        for p in &unmanaged {
            println!("    {}:{}", p.backend, p.name);
        }
    }
    if !unverified.is_empty() {
        println!("{} ({}):", UNVERIFIED_HEADING, unverified.len());
        for (backend, name) in &unverified {
            println!("    {}:{}", backend, name);
        }
    }
    if !extras_to_undo.is_empty() {
        println!(
            "- no longer declared — `sync` would undo ({}):",
            extras_to_undo.len()
        );
        for key in &extras_to_undo {
            println!("    {}", key);
        }
    }
    Ok(())
}

/// Write the currently-installed version of every managed package to locks/versions.json so a
/// later `sync --locked` reproduces those exact versions (where the backend supports it).
/// Compute the sync changes for the current desired state (shared by `plan` and `apply`).
/// Resolve, enforce and plan — returning both the changes and the variables the resolution used.
///
/// `frozen_vars` is `Some` only when applying a saved plan: the model resolves against the plan's
/// own variables instead of running the provider again, so a clock/shell/network variable does
/// not read differently at apply time than it did when the plan was captured (IX.6).
pub(crate) async fn compute_full_changes(
    app: &App,
    frozen_vars: Option<linix::model::vars::Vars>,
) -> Result<(linix::app::sync::SyncChanges, linix::model::vars::Vars)> {
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let resolver = match frozen_vars {
        Some(v) => resolver.with_vars(v),
        None => resolver,
    };
    let state = resolver.resolve_model().await?;
    enforce_policy(app, &state.packages).await?;
    let state_guard = app.state.lock().await;
    let planner = linix::app::sync::planner::ChangePlanner::new(
        app.registry.clone(),
        &state_guard,
        &app.config,
    );
    let changes = planner.plan(&state.packages, None).await?;
    Ok((changes, state.vars))
}

pub(crate) async fn handle_plan(app: &App, out: &str) -> Result<()> {
    let (changes, vars) = compute_full_changes(app, None).await?;
    // XIII.3's exit condition names `plan`: a script's hash, its run count and the decision
    // that follows are printed here, before anything happens. `plan` resolves the model a
    // second time rather than threading it out of `compute_full_changes`, which is a cheap
    // parse against the benefit of leaving that function's seam alone.
    {
        let resolver = linix::app::sync::resolver::StateResolver::new(
            &app.config,
            app.registry.clone(),
            false,
        )
        .await;
        if let Ok(state) = resolver.resolve_model().await {
            app.execs().print_plan(&state);
        }
    }
    let created_at = chrono::Utc::now().timestamp();
    let mut plan = linix::app::sync::SavedPlan::from_changes(&changes, Some(created_at));
    // Freeze the resolved variables so `apply` reproduces this exact resolution (IX.6).
    plan.vars = vars;
    tokio::fs::write(out, serde_json::to_string_pretty(&plan)?).await?;
    if plan.is_empty() {
        println!(
            "Wrote plan to {} — system already matches desired state (no changes).",
            out
        );
    } else {
        println!(
            "Wrote plan to {} — {} install(s), {} removal(s).\nReview it, then run `linix apply {}`.",
            out,
            plan.installs.len(),
            plan.removals.len(),
            out
        );
        // W13, on the path where it matters most: `plan` is read before anything is touched,
        // so a removal a `vars` edit caused has to be explained here too, not only at sync.
        if !plan.removals.is_empty() {
            print_vars_changed(app, &plan.vars).await;
        }
        // Writing a plan changes nothing, so this warns rather than refuses — but say it
        // here, where there is still time to fix the manifest, rather than letting the
        // refusal be a surprise at apply time.
        let removal_pairs: Vec<(String, String)> = plan
            .removals
            .iter()
            .map(|r| (r.backend.clone(), r.name.clone()))
            .collect();
        let report =
            linix::app::sync::guard::inspect(&app.config, &app.registry, &removal_pairs).await;
        if !report.is_empty() {
            println!(
                "\nWARNING: `linix apply` will refuse this plan.\n{}",
                report.message(
                    linix::app::sync::guard::GuardScope::Apply,
                    linix::app::sync::guard::RemovalKind::Package,
                )
            );
        }
    }
    Ok(())
}

/// Rebuild a `SyncChanges` graph from a saved plan's install/removal lists, so the shared
/// interactive review screen (which operates on a change graph) can also drive `apply`.
pub(crate) fn saved_plan_to_changes(
    installs: &[linix::core::PackageSpec],
    removals: &[linix::app::sync::saved_plan::PlanRemoval],
) -> linix::app::sync::planner::SyncChanges {
    use linix::core::GraphAction;
    let mut graph = petgraph::stable_graph::StableDiGraph::new();
    for spec in installs {
        graph.add_node(GraphAction::Install(spec.clone()));
    }
    for r in removals {
        graph.add_node(GraphAction::Remove {
            name: r.name.clone(),
            backend: r.backend.clone(),
        });
    }
    linix::app::sync::planner::SyncChanges {
        graph,
        ..Default::default()
    }
}

/// Collect the `backend:name` keys that survived an interactive review, split into
/// (install-keys, removal-keys) so the caller can filter the original plan lists.
pub(crate) fn surviving_keys(
    changes: &linix::app::sync::planner::SyncChanges,
) -> (
    std::collections::HashSet<String>,
    std::collections::HashSet<String>,
) {
    use linix::core::GraphAction;
    let mut installs = std::collections::HashSet::new();
    let mut removes = std::collections::HashSet::new();
    for w in changes.graph.node_weights() {
        match w {
            GraphAction::Install(s) => {
                installs.insert(format!("{}:{}", s.backend, s.name));
            }
            GraphAction::Remove { name, backend } => {
                removes.insert(format!("{}:{}", backend, name));
            }
        }
    }
    (installs, removes)
}

pub(crate) async fn handle_apply(app: &App, plan_path: &str, yes: bool) -> Result<()> {
    let raw = tokio::fs::read_to_string(plan_path)
        .await
        .with_context(|| format!("reading plan file {}", plan_path))?;
    let plan: linix::app::sync::SavedPlan =
        serde_json::from_str(&raw).context("parsing plan file")?;

    if plan.schema != linix::app::sync::PLAN_SCHEMA {
        anyhow::bail!(
            "plan schema {} is unsupported (this linix speaks schema {})",
            plan.schema,
            linix::app::sync::PLAN_SCHEMA
        );
    }
    // Integrity: refuse a hand-edited plan unless forced.
    if plan.recomputed_hash() != plan.desired_hash && !yes {
        anyhow::bail!(
            "plan file looks modified (content hash mismatch). Re-generate with `linix plan`, \
             or pass --yes to force."
        );
    }
    if plan.is_empty() {
        println!("Plan is empty — nothing to apply.");
        return Ok(());
    }

    // Drift detection, and the `[guard]` gate: `compute_full_changes` runs `enforce_policy`,
    // so an `Err` here is a refusal and must not be swallowed. Applying a captured plan to a
    // machine whose manifests no longer resolve is the case this stops.
    {
        // Resolve against the plan's frozen variables, so a clock/shell/network variable does
        // not read differently now and trip a drift warning for a change nobody made (IX.6).
        let (now_changes, _) = compute_full_changes(app, Some(plan.vars.clone())).await?;
        let current = linix::app::sync::SavedPlan::from_changes(&now_changes, None);
        if current.desired_hash != plan.desired_hash {
            if yes {
                warn!("apply: system has drifted from the captured plan; applying anyway (--yes).");
            } else {
                println!(
                    "WARNING: the system/manifests have drifted since this plan was captured."
                );
                use std::io::IsTerminal;
                // `unwrap_or(false)` alone aborts safely with nobody at the keyboard, and says
                // only "Aborted" — naming neither the reason nor `--yes`. Found by the test
                // that enumerates prompts from the source; no review had reported it.
                if !std::io::stdin().is_terminal() {
                    return Err(linix::core::Error::Refused(
                        "The captured plan no longer matches this machine, and there is no \
                         terminal to confirm on. Re-run with --yes to apply it anyway, or \
                         `linix plan` to capture a fresh one."
                            .to_string(),
                    )
                    .into());
                }
                let proceed = dialoguer::Confirm::new()
                    .with_prompt("Apply the captured plan anyway?")
                    .default(false)
                    .interact()
                    .unwrap_or(false);
                if !proceed {
                    println!("Aborted. Run `linix plan` to capture a fresh plan.");
                    return Ok(());
                }
            }
        }
    }

    if app.config.dry_run {
        println!(
            "[DRY-RUN] would install {} and remove {} package(s).",
            plan.installs.len(),
            plan.removals.len()
        );
        return Ok(());
    }

    // Optional interactive review: the same toggle screen as `sync`/`rollback`, so a captured
    // plan can still be trimmed at apply time. Skipped with --yes or without a terminal.
    let mut installs = plan.installs.clone();
    let mut removals = plan.removals.clone();
    if !yes && !app.config.yes {
        use std::io::IsTerminal;
        if std::io::stdin().is_terminal() {
            let changes = saved_plan_to_changes(&installs, &removals);
            let mut preview = TuiPreview::new(&changes, HashMap::new());
            if !preview.run()? {
                println!("Apply cancelled.");
                return Ok(());
            }
            let (keep_installs, keep_removes) = surviving_keys(&preview.get_filtered_changes());
            installs.retain(|s| keep_installs.contains(&format!("{}:{}", s.backend, s.name)));
            removals.retain(|r| keep_removes.contains(&format!("{}:{}", r.backend, r.name)));
            if installs.is_empty() && removals.is_empty() {
                println!("All changes deselected — nothing to apply.");
                return Ok(());
            }
        }
    }

    // `apply` executes its removals directly rather than through SyncEngine::sync, so it
    // needs its own call to the same guard. Placed after the interactive trim, so
    // deselecting the dangerous removals clears the guard honestly.
    let removal_pairs: Vec<(String, String)> = removals
        .iter()
        .map(|r| (r.backend.clone(), r.name.clone()))
        .collect();
    linix::app::sync::guard::enforce(
        &app.config,
        &app.registry,
        &removal_pairs,
        linix::app::sync::guard::GuardScope::Apply,
    )
    .await?;
    linix::app::sync::guard::enforce_installs(
        &app.config,
        installs.len(),
        linix::app::sync::guard::GuardScope::Apply,
    )
    .await?;

    let session_active = app.state.lock().await.active_session_id.is_some();
    let mut installed = 0usize;
    let mut removed = 0usize;

    for spec in &installs {
        let Some(b) = app.registry.get(&spec.backend) else {
            warn!(
                "apply: backend '{}' unavailable — skipping {}",
                spec.backend, spec.name
            );
            continue;
        };
        if let Some(inst) = b.as_installable() {
            info!("apply: installing {}:{}", spec.backend, spec.name);
            if let Err(e) = inst
                .install(std::slice::from_ref(spec), b.sudo_for_write())
                .await
            {
                warn!(
                    "apply: install {}:{} failed: {}",
                    spec.backend, spec.name, e
                );
                continue;
            }
            let source = spec
                .options
                .get("__source")
                .cloned()
                .or_else(|| Some("plan".into()));
            app.state.lock().await.add(
                &spec.backend,
                &spec.name,
                None,
                spec.options.clone(),
                source,
                session_active,
            );
            installed += 1;
        }
    }

    for r in &removals {
        let Some(b) = app.registry.get(&r.backend) else {
            continue;
        };
        if let Some(inst) = b.as_installable() {
            info!("apply: removing {}:{}", r.backend, r.name);
            if let Err(e) = inst
                .remove(std::slice::from_ref(&r.name), b.sudo_for_write())
                .await
            {
                warn!("apply: remove {}:{} failed: {}", r.backend, r.name, e);
                continue;
            }
            app.state.lock().await.remove(&r.backend, &r.name);
            removed += 1;
        }
    }

    app.state.lock().await.save()?;
    println!(
        "Applied plan: {} installed, {} removed.",
        installed, removed
    );
    perform_maintenance(app).await
}

/// Build and write `locks/versions.json` from the current managed state (live installed versions
/// preferred, falling back to recorded state). Returns the number of versions pinned. Shared
/// by `linix lock` and by `linix heal` (which reconciles the lockfile).
pub(crate) async fn build_and_write_locks(app: &App) -> Result<usize> {
    let mut locks = serde_json::Map::new();
    {
        let state = app.state.lock().await;
        for pkg in &state.packages {
            // Prefer the live installed version from the backend; fall back to recorded state.
            let version = match app
                .registry
                .get(&pkg.backend)
                .and_then(|b| b.as_queryable().cloned())
            {
                Some(q) => match q.info(&pkg.name).await {
                    Ok(Some(p)) => p.version.or_else(|| pkg.version.clone()),
                    _ => pkg.version.clone(),
                },
                None => pkg.version.clone(),
            };
            if let Some(v) = version {
                if !v.is_empty() && v != "unknown" {
                    locks.insert(
                        format!("{}:{}", pkg.backend, pkg.name),
                        serde_json::Value::String(v),
                    );
                }
            }
        }
    }
    let count = locks.len();
    let path = app.config.config_root().join("locks").join("versions.json");
    // The version pins live in the `locks/` directory (II.6) beside the hook and extras
    // ledgers — not a stray `locks.json` file beside that directory (the old layout).
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.ok();
    }
    let doc = serde_json::json!({ "locks": locks });
    tokio::fs::write(&path, serde_json::to_string_pretty(&doc)?)
        .await
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(count)
}

pub(crate) async fn handle_lock(app: &App) -> Result<()> {
    // Generators are approved FIRST, by scanning the files — before anything calls
    // `resolve_model`, which now runs generators and would refuse an unapproved one, so the very
    // command that approves it could never resolve far enough to reach it (U33).
    let generators = approve_generate_commands(app)?;
    if generators > 0 {
        println!(
            "Lock: approved {} generate command(s) at their current hash.",
            generators
        );
    }
    let count = build_and_write_locks(app).await?;
    println!(
        "Lock: pinned {} package version(s) to {}",
        count,
        app.config
            .config_root()
            .join("locks")
            .join("versions.json")
            .display()
    );
    // II.12: `lock` is also how you approve hooks. Record the current hash of every hook so a
    // later change to any of them stops the next sync until it is re-approved here. "Hash
    // everything, including your own scripts" — one rule, no exceptions.
    let hooks = app.hooks.approve_all_hooks()?;
    if hooks > 0 {
        println!(
            "Lock: approved {} hook(s) at their current script hash ({}).",
            hooks,
            linix::core::hook_lock::HookLedger::path_in(&app.config.config_root().join("locks"))
                .display()
        );
    }
    // A hook on one of LiNix's own events (XIII.13) is the same surface: a script the repo
    // carries, run without anyone watching. Both of U15's locations are approved here, and
    // separately — the shared policy's approval must not cover this machine's local file.
    let events = linix::app::events::EventHooks::load(&app.config);
    let approved_events = events.approve_all()?;
    if approved_events > 0 {
        println!(
            "Lock: approved {} event hook(s) — {}.",
            approved_events,
            events
                .all()
                .iter()
                .map(|h| format!("{} at {}", h.event, h.origin))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    // A `vars` provider that executes is a script on the same ledger (V.55). Approving it
    // here is the one deliberate act that lets it run — a changed provider stops resolution,
    // which is `status` and `plan`, not just `sync`.
    if let Some(file) = approve_vars_provider(app)? {
        println!(
            "Lock: approved the vars provider `{}` at its current hash.",
            file
        );
    }
    // And every `adapters/` file (7a/U10). They travel with the repo, and a definition is
    // argv LiNix will run, so each is approved here or it does not load.
    for name in approve_adapters(app)? {
        println!("Lock: approved `adapters/{}` at its current hash.", name);
    }
    // And every declared `exec:` script (XIII.3). II.12 admits no exceptions: a script the
    // configuration runs is approved by this command or it does not run.
    let execs = approve_exec_scripts(app).await?;
    if execs > 0 {
        println!(
            "Lock: approved {} exec script(s) at their current hash.",
            execs
        );
    }
    // And every user-declared health-check COMMAND (U31). A check is argv, run after a change,
    // so it is on the same trust model — approved here or the check counts as failed.
    let health = approve_health_checks(app).await?;
    if health > 0 {
        println!(
            "Lock: approved {} health-check command(s) at their current hash.",
            health
        );
    }
    Ok(())
}

/// Record every declared `exec:` script's current hash in the hook ledger, returning how many
/// were approved.
///
/// Reads the model rather than the filesystem so it approves exactly what a sync would run —
/// approving a script no active profile reaches would be approving something the user cannot
/// see in `plan`.
/// Approve every declared `generate:` command's current script hash (U33), scanning the files
/// directly rather than the resolved model — because resolving the model *runs* generators, and
/// a generator cannot be approved by a command that must resolve past it first. Reads
/// `modules/` and `profiles/`, ungated, so a generator behind a `when` is still approvable.
pub(crate) fn approve_generate_commands(app: &App) -> Result<usize> {
    use linix::config::grammar::{parse_document, Statement};
    use linix::core::hook_lock::{generate_id, hash_script, HookLedger};

    let layout = app.config.layout();
    let known = |name: &str| app.registry.get(name).is_some();
    let mut commands: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for dir in [layout.modules_dir(), layout.profiles_dir()] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Ok(body) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(doc) = parse_document(&path, &body, &known) else {
                continue;
            };
            for (stmt, _, _) in doc.every_statement() {
                if let Statement::Generate(cmd, _) = stmt {
                    commands.insert(cmd.clone());
                }
            }
        }
    }
    if commands.is_empty() {
        return Ok(0);
    }
    let locks = layout.locks_dir();
    let ledger_path = HookLedger::path_in(&locks);
    let mut ledger = HookLedger::load(&ledger_path)?;
    let mut approved = 0usize;
    for cmd in &commands {
        let declared = std::path::Path::new(cmd);
        let full = if declared.is_absolute() {
            declared.to_path_buf()
        } else {
            app.config.config_root().join(declared)
        };
        let body = std::fs::read_to_string(&full).map_err(|e| {
            anyhow::anyhow!(
                "cannot read `generate:{}` at {} ({})",
                cmd,
                full.display(),
                e
            )
        })?;
        ledger.approve(&generate_id(cmd), &hash_script(&body));
        approved += 1;
    }
    ledger.save(&ledger_path)?;
    Ok(approved)
}

pub(crate) async fn approve_exec_scripts(app: &App) -> Result<usize> {
    use linix::core::hook_lock::{exec_id, hash_script, HookLedger};

    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let state = resolver.resolve_model().await?;
    if !state.has_execs() {
        return Ok(0);
    }
    let locks = app.config.layout().locks_dir();
    let path = HookLedger::path_in(&locks);
    let mut ledger = HookLedger::load(&path)?;
    let mut approved = 0usize;
    for (script, _opts, origin) in state.execs() {
        let declared = std::path::Path::new(script);
        let full = if declared.is_absolute() {
            declared.to_path_buf()
        } else {
            app.config.config_root().join(declared)
        };
        let body = std::fs::read_to_string(&full).map_err(|e| {
            anyhow::anyhow!(
                "{}: cannot read `exec:{}` at {} ({})",
                origin,
                script,
                full.display(),
                e
            )
        })?;
        ledger.approve(&exec_id(script), &hash_script(&body));
        approved += 1;
    }
    ledger.save(&path)?;
    Ok(approved)
}

/// Record every declared health-check *command* in the hook ledger (U31), returning how many
/// were approved. Port probes run no code and are not approved; only `Probe::Command` is.
///
/// Reads the resolved model (every `@health=` line the active profiles reach) plus the
/// machine-wide `health` list, so it approves exactly the commands a sync would run.
pub(crate) async fn approve_health_checks(app: &App) -> Result<usize> {
    use linix::core::hook_lock::{hash_script, health_id, HookLedger};
    use linix::model::health::Probe;

    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let desired = resolver.resolve_desired_state().await?;

    let mut commands: Vec<String> = Vec::new();
    for specs in desired.values() {
        for spec in specs {
            if let Some(Probe::Command(cmd)) =
                spec.options.get("health").and_then(|s| Probe::parse(s))
            {
                commands.push(cmd);
            }
        }
    }
    for written in &app.config.health {
        if let Some(Probe::Command(cmd)) = Probe::parse(written) {
            commands.push(cmd);
        }
    }
    if commands.is_empty() {
        return Ok(0);
    }
    let path = HookLedger::path_in(&app.config.layout().locks_dir());
    let mut ledger = HookLedger::load(&path)?;
    let mut approved = 0usize;
    for cmd in commands {
        ledger.approve(&health_id(&cmd), &hash_script(&cmd));
        approved += 1;
    }
    ledger.save(&path)?;
    Ok(approved)
}

/// Record each `adapters/` file's hash in the hook ledger, returning the names approved.
///
/// One entry per file, not per definition: an edit that *adds* a `[[backend]]` must invalidate
/// the approval, and a per-definition identity would let exactly that slip through.
/// A file the repo does not carry is the ordinary case, never an error.
pub(crate) fn approve_adapters(app: &App) -> Result<Vec<String>> {
    use linix::core::hook_lock::{adapter_id, hash_script, HookLedger};

    let layout = app.config.layout();
    // Every `*.toml` in the adapters folder, not a hardcoded list. The list was the bug: it
    // named backends/settings/bootstrap and silently omitted `firewall.toml`, so a repo that
    // carried a firewall adapter could never approve it and its rows were refused on every
    // sync. Reading the folder means a new adapter kind (`init.toml`, `snapshot.toml`) is
    // approvable the day it is added, with no second place to remember to edit.
    let dir = layout.adapters_dir();
    let ledger_path = HookLedger::path_in(&layout.locks_dir());
    let mut ledger = HookLedger::load(&ledger_path)?;
    let mut approved = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(approved),
        Err(e) => return Err(e.into()),
    };
    let mut files: Vec<std::path::PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("toml"))
        .collect();
    files.sort();
    for file in files {
        let body = match std::fs::read_to_string(&file) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        ledger.approve(&adapter_id(&name), &hash_script(&body));
        approved.push(name);
    }
    if !approved.is_empty() {
        ledger.save(&ledger_path)?;
    }
    Ok(approved)
}

/// Record the active executing `vars` provider's current hash in the hook ledger. Returns the
/// filename if one was approved, `None` if the repo has no provider or a non-executing line
/// file. The single source of which provider is active is `vars_provider::select`, shared
/// with resolution so `lock` and the gate can never disagree about what runs.
pub(crate) fn approve_vars_provider(app: &App) -> Result<Option<String>> {
    use linix::core::hook_lock::{hash_script, vars_id, HookLedger};
    use linix::model::vars_provider::{self, Kind};

    let root = app.config.config_root();
    let Some(selected) = vars_provider::select(&root, &app.config.vars.source)? else {
        return Ok(None);
    };
    if matches!(selected.kind, Kind::LineFile) {
        return Ok(None);
    }
    let filename = selected
        .path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    let body = std::fs::read_to_string(&selected.path)?;
    let locks = root.join("locks");
    let path = HookLedger::path_in(&locks);
    let mut ledger = HookLedger::load(&path)?;
    ledger.approve(&vars_id(&filename), &hash_script(&body));
    ledger.save(&path)?;
    Ok(Some(filename))
}

pub(crate) async fn handle_unlock(app: &App, names: &[String], list: bool) -> Result<()> {
    let path = linix::core::BareLock::path_in(&app.config.config_root().join("locks"));
    let mut lock = linix::core::BareLock::load(&path)?;

    if list || (names.is_empty() && lock.is_empty()) {
        if lock.is_empty() {
            println!("Nothing is frozen on this host.");
            return Ok(());
        }
        for (name, backend) in lock.entries() {
            println!("{} -> {}", name, backend);
        }
        return Ok(());
    }

    let changed = if names.is_empty() {
        let n = lock.entries().count();
        lock.clear();
        println!("Unlocked {} name(s). The next sync asks again.", n);
        true
    } else {
        let mut any = false;
        for name in names {
            if lock.forget(name) {
                any = true;
                println!("Unlocked `{}`. The next sync asks again.", name);
            } else {
                // Not an error: a name with a manager written on its line was never frozen,
                // and saying so is more use than a failure the caller has to interpret.
                warn!(
                    "`{}` was not frozen on this host — nothing to unlock.",
                    name
                );
            }
        }
        any
    };

    if changed {
        lock.save(&path)?;
        println!(
            "Run `linix sync` to re-resolve. A name that moves manager is reinstalled from \
             the new one and removed from the old."
        );
    }
    Ok(())
}

#[cfg(test)]
mod unverified_tests {
    use super::*;
    use linix::core::state::ManagedPackage;

    fn pkg(backend: &str, name: &str, unverified: bool) -> ManagedPackage {
        ManagedPackage {
            name: name.into(),
            backend: backend.into(),
            version: None,
            installed_at: 0,
            expires_at: None,
            options: if unverified {
                [("unverified".to_string(), "true".to_string())]
                    .into_iter()
                    .collect()
            } else {
                Default::default()
            },
            source: None,
            is_transient: false,
            session_id: None,
        }
    }

    /// Every backend the flag is legal on stays visible after the install — the download ones
    /// and, since Q5, the manager that verifies a signature itself.
    #[test]
    fn what_skipped_a_check_is_listed_whichever_backend_skipped_it() {
        let state = linix::core::StateRegistry {
            packages: vec![
                pkg("helm", "diff", true),
                pkg("github", "sharkdp/fd", true),
                pkg("web", "https://example.com/tool", true),
                pkg("appimage", "https://example.com/x.AppImage", true),
                pkg("apt", "curl", false),
                pkg("github", "BurntSushi/ripgrep", false),
            ],
            ..Default::default()
        };

        let listed = unverified_packages(&state);
        assert_eq!(
            listed,
            vec![
                ("helm".to_string(), "diff".to_string()),
                ("github".to_string(), "sharkdp/fd".to_string()),
                ("web".to_string(), "https://example.com/tool".to_string()),
                (
                    "appimage".to_string(),
                    "https://example.com/x.AppImage".to_string()
                ),
            ],
            "the listing must name exactly what skipped a check"
        );
    }

    /// helm downloads nothing LiNix can see, so the heading cannot claim it did.
    #[test]
    fn the_heading_does_not_claim_linix_downloaded_it() {
        assert!(!UNVERIFIED_HEADING.contains("downloaded"));
        assert!(UNVERIFIED_HEADING.contains("@unverified"));
    }
}
