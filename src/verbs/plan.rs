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
    // This report ends with a crawl of every manager on the machine, so every manager is asked
    // either way — asked here they answer at once instead of in the order the sections below
    // happen to need them (`App::warm_installed`).
    app.warm_installed().await;
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let state = resolver.resolve_model().await?;
    let desired = state.packages.clone();
    // A deleted `service:`/`link:`/`repo:` line is drift a sync will undo (S20), and `status`
    // that reports only packages says "nothing to do" on the run that disables a service.
    //
    // Both directions, because this view had only the teardown half — the same one-sided
    // reading as `check`'s summary (N-2). A declared resource that has never been applied is
    // work `sync` will do, and `status` calling that "nothing to do" is the identical defect
    // one command over.
    let resources = app.extras().changes(&state).await.unwrap_or_default();
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
            "resources_to_place": resources.place,
            "resources_to_undo": resources.undo,
            "resources_unverifiable": resources.unverifiable,
            "left_in_place": report.skipped,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if report.install.is_empty()
        && report.remove.is_empty()
        && report.skipped.is_empty()
        && unmanaged.is_empty()
        && unverified.is_empty()
        && resources.is_empty()
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
    // Distinct from `unmanaged` below, and the distinction is the point: an unmanaged package
    // is one LiNix never took responsibility for, and one of these is a package it manages,
    // that nothing declares, and that it has decided never to remove (AU1).
    if !report.skipped.is_empty() {
        println!(
            "~ drift — `sync` would leave in place ({}):",
            report.skipped.len()
        );
        for s in &report.skipped {
            println!("    {}  ({})", s.key, s.reason);
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
    if !resources.place.is_empty() {
        println!(
            "+ declared and not in effect — `sync` would place ({}):",
            resources.place.len()
        );
        for key in &resources.place {
            println!("    {}", key);
        }
    }
    if !resources.undo.is_empty() {
        println!(
            "- no longer declared — `sync` would undo ({}):",
            resources.undo.len()
        );
        for key in &resources.undo {
            println!("    {}", key);
        }
    }
    if !resources.unverifiable.is_empty() {
        // Said out loud on this view too: these are resources LiNix cannot read back, so
        // "nothing to do" about them is an assumption and not a measurement.
        println!(
            "? could not be read back — assumed in place ({}):",
            resources.unverifiable.len()
        );
        for key in &resources.unverifiable {
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
/// Everything one resolution produces, so `plan` and `apply` read the same model rather than
/// resolving it twice and comparing the halves they each happened to compute.
pub(crate) struct FullChanges {
    pub changes: linix::app::sync::SyncChanges,
    /// The resource half (N-2). `plan` froze only the package half, so a plan over three
    /// unapplied `link:` lines was an empty file and `apply` of it did nothing.
    pub resources: linix::app::apply::ResourceChanges,
    /// The resolved model itself — `apply` needs it to run the non-package phases, and its
    /// `vars` are what a plan freezes.
    pub state: linix::model::DesiredState,
}

pub(crate) async fn compute_full_changes(
    app: &App,
    frozen_vars: Option<linix::model::vars::Vars>,
) -> Result<FullChanges> {
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let resolver = match frozen_vars {
        Some(v) => resolver.with_vars(v),
        None => resolver,
    };
    let state = resolver.resolve_model().await?;
    enforce_policy(app, &state.packages).await?;
    let resources = app.extras().changes(&state).await?;
    let changes = {
        let state_guard = app.state.lock().await;
        let planner = linix::app::sync::planner::ChangePlanner::new(
            app.registry.clone(),
            &state_guard,
            &app.config,
        );
        planner.plan(&state.packages, None).await?
    };
    Ok(FullChanges {
        changes,
        resources,
        state,
    })
}

pub(crate) async fn handle_plan(app: &App, out: &str) -> Result<()> {
    let full = compute_full_changes(app, None).await?;
    // XIII.3's exit condition names `plan`: a script's hash, its run count and the decision
    // that follows are printed here, before anything happens. Read off the resolution
    // `compute_full_changes` already did — it used to resolve the model a second time for
    // this, which is one model resolved twice and free to disagree with itself.
    app.execs().print_plan(&full.state);
    let created_at = chrono::Utc::now().timestamp();
    let mut plan =
        linix::app::sync::SavedPlan::from_changes(&full.changes, &full.resources, Some(created_at));
    // Freeze the resolved variables so `apply` reproduces this exact resolution (IX.6).
    plan.vars = full.state.vars.clone();
    tokio::fs::write(out, serde_json::to_string_pretty(&plan)?).await?;
    // Beside the plan, never inside it: a skip is not an action `apply` can carry out, and a
    // saved plan is a list of actions. But `plan` is the command for "what would sync do", and
    // "nothing, to this package, forever" is part of that answer (AU1).
    crate::verbs::sync::print_skipped(&full.changes.skipped);
    if plan.is_empty() && !full.changes.skipped.is_empty() {
        // `already matches` is a claim about the machine, and the lines above have just named
        // packages it holds that nothing declares. The plan is genuinely empty; the machine is
        // genuinely not converged, and saying only the first is how AU1 read.
        println!(
            "Wrote plan to {} — no actions. {} package(s) are installed, declared nowhere, and \
             will not be removed (above).",
            out,
            full.changes.skipped.len()
        );
    } else if plan.is_empty() {
        println!(
            "Wrote plan to {} — system already matches desired state (no changes).",
            out
        );
        // Not silence: `check` says the same thing in the same breath, and a resource LiNix
        // cannot read back is a limit on what "already matches" means here.
        if !full.resources.unverifiable.is_empty() {
            println!(
                "  ({} resource(s) could not be read back and are assumed in place: {})",
                full.resources.unverifiable.len(),
                full.resources.unverifiable.join(", ")
            );
        }
    } else {
        println!(
            "Wrote plan to {} — {} install(s), {} removal(s), {} resource(s) to place, {} to \
             undo.\nReview it, then run `linix apply {}`.",
            out,
            plan.installs.len(),
            plan.removals.len(),
            plan.resources.place.len(),
            plan.resources.undo.len(),
            out
        );
        for key in &plan.resources.place {
            println!("  + {}", key);
        }
        for key in &plan.resources.undo {
            println!("  - {} (no longer declared)", key);
        }
        // W13, on the path where it matters most: `plan` is read before anything is touched,
        // so a removal a `vars` edit caused has to be explained here too, not only at sync.
        if !plan.removals.is_empty() {
            print_vars_changed(app, &plan.vars).await;
        }
        // Writing a plan changes nothing, so this warns rather than refuses — but say it
        // here, where there is still time to fix the manifest, rather than letting the
        // refusal be a surprise at apply time.
        //
        // **Asked the way `apply` asks it, kind by kind.** One `inspect` over both lists ran
        // the package rules over resource keys, and `protection_of` opens by asking whether a
        // package line could hold the name — which no `link:`/`service:` key can. So every
        // teardown came back `Undeclarable`: `plan` predicted a refusal for undeclaring three
        // dotfiles, `apply` performed it at rc=0, and the explanation a user read was a
        // sentence about package names. The guard carries a unit test asserting exactly that
        // cannot happen; it exercises `RemovalKind::Extra`, and this call site passed
        // `Package`.
        //
        // Each list still counts the other against the same ceiling, because the ceiling is a
        // property of the command: a sync dropping three packages and three links removes six
        // things. The `also_removing` split matches `sync`'s (`guard::enforce` then
        // `enforce_extras`), so the preview and the enforcer refuse on the same machine.
        let package_pairs: Vec<(String, String)> = plan
            .removals
            .iter()
            .map(|r| (r.backend.clone(), r.name.clone()))
            .collect();
        let extra_pairs = linix::app::sync::guard::extra_removal_pairs(&plan.resources.undo);
        let scope = linix::app::sync::guard::GuardScope::Apply;
        let mut refusals = Vec::new();
        for (pairs, kind, also) in [
            (
                &package_pairs,
                linix::app::sync::guard::RemovalKind::Package,
                0,
            ),
            (
                &extra_pairs,
                linix::app::sync::guard::RemovalKind::Extra,
                package_pairs.len(),
            ),
        ] {
            let report = linix::app::sync::guard::inspect_removals(
                &app.config,
                &app.registry,
                pairs,
                kind,
                also,
            )
            .await;
            if !report.is_empty() {
                refusals.push(report.message(scope, kind));
            }
        }
        if !refusals.is_empty() {
            println!(
                "\nWARNING: `linix apply` will refuse this plan.\n{}",
                refusals.join("\n")
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
    //
    // Resolve against the plan's frozen variables, so a clock/shell/network variable does not
    // read differently now and trip a drift warning for a change nobody made (IX.6). The
    // resolved model is kept: the resource phases below run against it, so `apply` executes
    // the same model it just checked rather than resolving a third one.
    let now = compute_full_changes(app, Some(plan.vars.clone())).await?;
    {
        let current = linix::app::sync::SavedPlan::from_changes(&now.changes, &now.resources, None);
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
            "[DRY-RUN] would install {} and remove {} package(s), place {} and undo {} \
             resource(s).",
            plan.installs.len(),
            plan.removals.len(),
            plan.resources.place.len(),
            plan.resources.undo.len()
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

    // The resource half, through the same phase list `sync` runs (N-2). Not a second
    // implementation: `apply_non_package_phases` is the one list, and the comment above it
    // records what four separate copies of it already cost. It carries its own guard for the
    // teardown, and the package removals just performed are passed in so `max_removals` is a
    // ceiling on the command rather than on each phase.
    let resources = if plan.resources.is_empty() {
        0
    } else {
        crate::verbs::sync::apply_non_package_phases(
            app,
            &now.state,
            linix::app::sync::guard::GuardScope::Apply,
            removed,
        )
        .await?
    };

    println!(
        "Applied plan: {} installed, {} removed, {} resource(s) reconciled.",
        installed, removed, resources
    );
    perform_maintenance(app).await
}

/// Where the version pins live (II.6): in the `locks/` directory beside the hook and extras
/// ledgers, never a stray `locks.json` beside that directory.
pub(crate) fn version_lock_path(app: &App) -> std::path::PathBuf {
    app.config.config_root().join("locks").join("versions.json")
}

/// The pins on disk. A missing or unreadable file is an empty set of pins — the ordinary state
/// of a machine that has never run `linix lock`, never an error.
pub(crate) fn load_version_locks(path: &std::path::Path) -> serde_json::Map<String, Value> {
    let Ok(body) = std::fs::read_to_string(path) else {
        return serde_json::Map::new();
    };
    serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|doc| doc.get("locks").and_then(Value::as_object).cloned())
        .unwrap_or_default()
}

/// Write the pins back. Returns whether the bytes reached the disk — a preview pins nothing.
pub(crate) async fn write_version_locks(
    path: &std::path::Path,
    locks: &serde_json::Map<String, Value>,
) -> Result<bool> {
    if !linix::core::dry_run::active() {
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await.ok();
        }
    }
    let doc = serde_json::json!({ "locks": locks });
    linix::utils::file::persist(path, &serde_json::to_string_pretty(&doc)?)
        .with_context(|| format!("Failed to write {}", path.display()))
}

/// The version every managed package is at *now*, keyed `backend:name`.
///
/// The live answer from the backend, falling back to recorded state. `list_installed` is memoized
/// once per run (`Queryable::list_installed`), so asking `info` per package costs one command per
/// manager, not one per package.
async fn scan_installed_versions(app: &App) -> serde_json::Map<String, Value> {
    let mut locks = serde_json::Map::new();
    let state = app.state.lock().await;
    for pkg in &state.packages {
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
                locks.insert(format!("{}:{}", pkg.backend, pkg.name), Value::String(v));
            }
        }
    }
    locks
}

/// Build and write `locks/versions.json` from the current managed state. Returns the number of
/// versions pinned. Shared by `linix lock versions` and by `linix heal` (which reconciles the
/// lockfile).
pub(crate) async fn build_and_write_locks(app: &App) -> Result<(usize, bool)> {
    let locks = scan_installed_versions(app).await;
    let count = locks.len();
    let written = write_version_locks(&version_lock_path(app), &locks).await?;
    Ok((count, written))
}

/// Re-record the pins that already exist, from what is installed now. Returns how many moved.
///
/// A pin nothing updates is a pin that fights the upgrade that just ran: `sync` reads the
/// recorded version back as `@version=`, the installed one no longer satisfies it, and the next
/// ordinary sync plans the package straight back down. So every path that deliberately moves a
/// version forward — `upgrade`, `sync --upgrade` — records where it landed (Z2).
///
/// **Only entries that are already pinned are refreshed.** A package nobody pinned gains no pin
/// here: it has no stale record to fight, and pinning it would turn every upgrade into a `lock`.
pub(crate) async fn refresh_version_locks(app: &App) -> Result<usize> {
    // A preview moved no version, so there is nothing to re-record — and reporting a count it
    // could not write is the "would" that reads as "did".
    if linix::core::dry_run::active() {
        return Ok(0);
    }
    let path = version_lock_path(app);
    let mut locks = load_version_locks(&path);
    if locks.is_empty() {
        return Ok(0);
    }
    let current = scan_installed_versions(app).await;
    let moved = move_pins_to(&mut locks, &current);
    if moved > 0 {
        write_version_locks(&path, &locks).await?;
    }
    Ok(moved)
}

/// Point every existing pin at its current reading. Returns how many moved.
///
/// **Adds nothing.** A package with no pin is not pinned here, and a pin whose package the scan
/// could not read keeps the version it had — an unreadable manager is not evidence that a
/// package moved.
fn move_pins_to(
    locks: &mut serde_json::Map<String, Value>,
    current: &serde_json::Map<String, Value>,
) -> usize {
    let mut moved = 0usize;
    for (key, was) in locks.iter_mut() {
        if let Some(now) = current.get(key) {
            if now != was {
                *was = now.clone();
                moved += 1;
            }
        }
    }
    moved
}

/// Whether a scoping name the user typed picks out this ledger key.
///
/// A key is `KIND:REST` — `apt:curl`, `after_install:nginx`, `adapters:backends.toml` — and both
/// halves are things a person would type: the whole key when two kinds carry the same tail, the
/// tail alone when they do not. No names at all means every key.
pub(crate) fn scoped_by(key: &str, names: &[String]) -> bool {
    if names.is_empty() {
        return true;
    }
    let tail = key.split_once(':').map_or(key, |(_, rest)| rest);
    names.iter().any(|n| n == key || n == tail)
}

/// The heading and verb a message uses. Every ledger these commands write goes through
/// `utils::file::persist`, so the answer about one of them is the answer about all: a preview
/// pins nothing, approves nothing and forgets nothing.
fn tense(label: &str, done: &'static str, would: &'static str) -> (String, &'static str) {
    if linix::core::dry_run::active() {
        (format!("[DRY-RUN] {}:", label), would)
    } else {
        (format!("{}:", label), done)
    }
}

/// The names a "nothing matched" warning quotes back.
fn quoted(names: &[String]) -> String {
    names
        .iter()
        .map(|n| format!("`{}`", n))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `linix lock [AXIS] [NAME…]` — freeze what a sync would otherwise decide again (Z2).
pub(crate) async fn handle_lock(
    app: &App,
    axis: LockAxis,
    names: &[String],
    list: bool,
) -> Result<()> {
    if list {
        return list_locks(app, axis);
    }
    // Scripts before either axis that resolves the model, and generators first within scripts:
    // resolving *runs* generators, so a command that resolved first could never reach the
    // generator it exists to approve (U33).
    if axis.covers(LockAxis::Scripts) {
        lock_scripts(app, names).await?;
    }
    if axis.covers(LockAxis::Versions) {
        lock_versions(app, names).await?;
    }
    if axis.covers(LockAxis::Backends) {
        lock_backends(app, names).await?;
    }
    Ok(())
}

/// Pin the installed version of every managed package, or of the ones `names` picks out.
async fn lock_versions(app: &App, names: &[String]) -> Result<()> {
    let (tag, pinned) = tense("Lock", "pinned", "would pin");
    let path = version_lock_path(app);
    if names.is_empty() {
        let (count, _) = build_and_write_locks(app).await?;
        println!(
            "{} {} {} package version(s) to {}",
            tag,
            pinned,
            count,
            path.display()
        );
        return Ok(());
    }
    // Scoped: merge over what is already pinned rather than rebuilding the file, or naming one
    // package would silently drop every other pin.
    let mut locks = load_version_locks(&path);
    let mut hit: Vec<String> = Vec::new();
    for (key, version) in scan_installed_versions(app).await {
        if scoped_by(&key, names) {
            locks.insert(key.clone(), version);
            hit.push(key);
        }
    }
    if hit.is_empty() {
        warn!(
            "no managed package matches {} — nothing pinned.",
            quoted(names)
        );
        return Ok(());
    }
    write_version_locks(&path, &locks).await?;
    println!("{} {} {}", tag, pinned, hit.join(", "));
    Ok(())
}

/// Record which manager each unpinned bare name resolved to (II.7 step 4).
///
/// Resolution is what records, so this runs one and lets the resolver write. A scope is applied
/// afterwards: the resolver settles the whole model or none of it, and "resolve these three
/// names only" is not a question it can be asked.
async fn lock_backends(app: &App, names: &[String]) -> Result<()> {
    use linix::core::BareLock;

    let path = BareLock::path_in(&app.config.layout().locks_dir());
    let before = BareLock::load(&path)?;
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await
            .recording_locks();
    resolver.resolve_model().await?;
    let after = BareLock::load(&path)?;

    let (tag, recorded) = tense("Lock", "recorded", "would record");
    if !names.is_empty() {
        let mut scoped = before.clone();
        let mut hit: Vec<String> = Vec::new();
        for (name, backend) in after.entries().map(|(n, b)| (n.to_string(), b.to_string())) {
            if scoped_by(&name, names) {
                scoped.record(&name, &backend);
                hit.push(format!("{} -> {}", name, backend));
            }
        }
        // A name the model no longer declares is dropped by resolution; inside the scope that
        // drop is part of the answer, outside it the entry stays.
        for name in before
            .entries()
            .map(|(n, _)| n.to_string())
            .collect::<Vec<_>>()
        {
            if scoped_by(&name, names) && after.get(&name).is_none() {
                scoped.forget(&name);
            }
        }
        if hit.is_empty() {
            warn!(
                "no unpinned name matches {} — nothing recorded.",
                quoted(names)
            );
            return Ok(());
        }
        scoped.save(&path)?;
        println!("{} {} {}", tag, recorded, hit.join(", "));
        return Ok(());
    }

    let fresh = after
        .entries()
        .filter(|(name, backend)| before.get(name) != Some(backend))
        .count();
    println!(
        "{} {} {} of {} unpinned name(s) to {}",
        tag,
        recorded,
        fresh,
        after.entries().count(),
        path.display()
    );
    Ok(())
}

/// Approve everything the configuration can execute, at its current hash (II.12).
///
/// A scope is applied by approving everything and then putting back every entry the names did
/// not pick out. The seven approvers each read the files they own; a filter threaded through all
/// seven would be seven places for a scope to be forgotten, and the ledger is one place.
async fn lock_scripts(app: &App, names: &[String]) -> Result<()> {
    use linix::core::hook_lock::HookLedger;

    let ledger_path = HookLedger::path_in(&app.config.layout().locks_dir());
    let before = HookLedger::load(&ledger_path)?;
    let (tag, approved) = tense("Lock", "approved", "would approve");
    // Scoped runs report from the ledger afterwards: each approver counts what it read, which is
    // everything, and printing those counts beside a scope would be a false sentence.
    let scoped = !names.is_empty();

    // Generators are approved FIRST, by scanning the files — before anything calls
    // `resolve_model`, which now runs generators and would refuse an unapproved one, so the very
    // command that approves it could never resolve far enough to reach it (U33).
    let generators = approve_generate_commands(app)?;
    if generators > 0 && !scoped {
        println!(
            "{} {} {} generate command(s) at their current hash.",
            tag, approved, generators
        );
    }
    // II.12: `lock` is also how you approve hooks. Record the current hash of every hook so a
    // later change to any of them stops the next sync until it is re-approved here. "Hash
    // everything, including your own scripts" — one rule, no exceptions.
    let hooks = app.hooks.approve_all_hooks()?;
    if hooks > 0 && !scoped {
        println!(
            "{} {} {} hook(s) at their current script hash ({}).",
            tag,
            approved,
            hooks,
            ledger_path.display()
        );
    }
    // A hook on one of LiNix's own events (XIII.13) is the same surface: a script the repo
    // carries, run without anyone watching. Both of U15's locations are approved here, and
    // separately — the shared policy's approval must not cover this machine's local file.
    let events = linix::app::events::EventHooks::load(&app.config);
    let approved_events = events.approve_all()?;
    if approved_events > 0 && !scoped {
        println!(
            "{} {} {} event hook(s) — {}.",
            tag,
            approved,
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
        if !scoped {
            println!(
                "{} {} the vars provider `{}` at its current hash.",
                tag, approved, file
            );
        }
    }
    // And every `adapters/` file (7a/U10). They travel with the repo, and a definition is
    // argv LiNix will run, so each is approved here or it does not load.
    for name in approve_adapters(app)? {
        if !scoped {
            println!(
                "{} {} `adapters/{}` at its current hash.",
                tag, approved, name
            );
        }
    }
    // And every declared `exec:` script (XIII.3). II.12 admits no exceptions: a script the
    // configuration runs is approved by this command or it does not run.
    let model = resolve_for_approval(app).await?;
    let execs = approve_exec_scripts(app, &model).await?;
    if execs > 0 && !scoped {
        println!(
            "{} {} {} exec script(s) at their current hash.",
            tag, approved, execs
        );
    }
    // And every user-declared health-check COMMAND (U31). A check is argv, run after a change,
    // so it is on the same trust model — approved here or the check counts as failed.
    let health = approve_health_checks(app, &model).await?;
    if health > 0 && !scoped {
        println!(
            "{} {} {} health-check command(s) at their current hash.",
            tag, approved, health
        );
    }
    if !scoped {
        return Ok(());
    }

    // Put back everything the names did not pick out. A preview wrote nothing, so there is
    // nothing on disk to put back and nothing to count — it says what it would do and stops.
    if linix::core::dry_run::active() {
        println!("{} {} the entries matching {}", tag, approved, quoted(names));
        return Ok(());
    }
    let mut ledger = HookLedger::load(&ledger_path)?;
    let entries: Vec<(String, String)> = ledger
        .entries()
        .map(|(id, hash)| (id.to_string(), hash.to_string()))
        .collect();
    let mut hit: Vec<String> = Vec::new();
    for (id, _) in entries {
        if scoped_by(&id, names) {
            hit.push(id);
        } else {
            match before.get(&id) {
                Some(was) => {
                    let was = was.to_string();
                    ledger.approve(&id, &was);
                }
                None => {
                    ledger.revoke(&id);
                }
            }
        }
    }
    if hit.is_empty() {
        warn!(
            "nothing the configuration can run matches {} — nothing approved. \
             `linix lock scripts --list` names what is approvable.",
            quoted(names)
        );
    }
    ledger.save(&ledger_path)?;
    if !hit.is_empty() {
        println!("{} {} {}", tag, approved, hit.join(", "));
    }
    Ok(())
}

/// `linix lock --list` / `linix unlock --list` — what is locked on this axis, changing nothing.
fn list_locks(app: &App, axis: LockAxis) -> Result<()> {
    use linix::core::hook_lock::HookLedger;
    use linix::core::BareLock;

    let locks_dir = app.config.layout().locks_dir();
    if axis.covers(LockAxis::Versions) {
        let locks = load_version_locks(&version_lock_path(app));
        if locks.is_empty() {
            println!("versions: nothing is pinned.");
        } else {
            for (key, version) in &locks {
                println!("versions: {} -> {}", key, version.as_str().unwrap_or("?"));
            }
        }
    }
    if axis.covers(LockAxis::Backends) {
        let lock = BareLock::load(&BareLock::path_in(&locks_dir))?;
        if lock.is_empty() {
            println!("backends: nothing is frozen on this host.");
        } else {
            for (name, backend) in lock.entries() {
                println!("backends: {} -> {}", name, backend);
            }
        }
    }
    if axis.covers(LockAxis::Scripts) {
        let ledger = HookLedger::load(&HookLedger::path_in(&locks_dir))?;
        if ledger.is_empty() {
            println!("scripts: nothing is approved.");
        } else {
            for (id, hash) in ledger.entries() {
                println!("scripts: {} -> sha256:{}", id, &hash[..hash.len().min(12)]);
            }
        }
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

/// The one resolution the approvers read. `exec:` scripts and `@health=` commands are two
/// questions about the same model, and asking it twice is asking every manager twice.
pub(crate) async fn resolve_for_approval(app: &App) -> Result<linix::model::DesiredState> {
    linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
        .await
        .resolve_model()
        .await
        .map_err(Into::into)
}

pub(crate) async fn approve_exec_scripts(
    app: &App,
    state: &linix::model::DesiredState,
) -> Result<usize> {
    use linix::core::hook_lock::{exec_id, hash_script, HookLedger};

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
pub(crate) async fn approve_health_checks(
    app: &App,
    state: &linix::model::DesiredState,
) -> Result<usize> {
    use linix::core::hook_lock::{hash_script, health_id, HookLedger};
    use linix::model::health::Probe;

    let mut commands: Vec<String> = Vec::new();
    for specs in state.packages.values() {
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

/// `linix unlock [AXIS] [NAME…]` — release a lock, so the next sync decides it again (Z2).
pub(crate) async fn handle_unlock(
    app: &App,
    axis: LockAxis,
    names: &[String],
    list: bool,
) -> Result<()> {
    if list {
        return list_locks(app, axis);
    }
    // Q9: an unknown prefix reported "was not frozen on this host — nothing to unlock", which is
    // what a real name that is not frozen also reports.
    //
    // **This axis only.** A backend prefix here is a question about the managers *this* host
    // uses, which is what the check answers. It is not that question on the other two: a version
    // pin names whichever manager wrote it and `locks/` travels between machines, so
    // `apt:curl` is an ordinary entry on a host with no apt; a script id's prefix
    // (`after_install:`, `adapters:`) is not a backend at all; and on `all` the names span all
    // three namespaces at once. Those rely on each axis warning when a name picks nothing out —
    // which is a louder answer than this one, because it names the ledger as well as the name.
    if axis == LockAxis::Backends {
        app.require_known_spec_backends(names).await?;
    }
    if axis.covers(LockAxis::Backends) {
        unlock_backends(app, names).await?;
    }
    if axis.covers(LockAxis::Versions) {
        unlock_versions(app, names).await?;
    }
    if axis.covers(LockAxis::Scripts) {
        unlock_scripts(app, names)?;
    }
    Ok(())
}

/// Forget which manager an unpinned name resolved to, so the next sync asks again (II.6).
async fn unlock_backends(app: &App, names: &[String]) -> Result<()> {
    let path = linix::core::BareLock::path_in(&app.config.layout().locks_dir());
    let mut lock = linix::core::BareLock::load(&path)?;
    if lock.is_empty() {
        println!("backends: nothing is frozen on this host.");
        return Ok(());
    }

    let (tag, forgot) = tense("Unlock", "forgot", "would forget");
    let changed = if names.is_empty() {
        let n = lock.entries().count();
        lock.clear();
        println!(
            "{} backends: {} {} name(s). The next sync asks again.",
            tag, forgot, n
        );
        true
    } else {
        let mut any = false;
        for name in names {
            if lock.forget(name) {
                any = true;
                println!(
                    "{} backends: {} `{}`. The next sync asks again.",
                    tag, forgot, name
                );
            } else {
                // Not an error: a name with a manager written on its line was never frozen,
                // and saying so is more use than a failure the caller has to interpret.
                warn!("`{}` was not frozen on this host — nothing to unlock.", name);
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

/// Drop the version pins, so the next sync takes what the managers offer.
async fn unlock_versions(app: &App, names: &[String]) -> Result<()> {
    let path = version_lock_path(app);
    let mut locks = load_version_locks(&path);
    if locks.is_empty() {
        println!("versions: nothing is pinned.");
        return Ok(());
    }
    let dropped: Vec<String> = locks
        .keys()
        .filter(|key| scoped_by(key, names))
        .cloned()
        .collect();
    if dropped.is_empty() {
        warn!("no pin matches {} — nothing unpinned.", quoted(names));
        return Ok(());
    }
    for key in &dropped {
        locks.remove(key);
    }
    write_version_locks(&path, &locks).await?;
    let (tag, unpinned) = tense("Unlock", "unpinned", "would unpin");
    println!(
        "{} versions: {} {}. The next sync takes what the managers offer.",
        tag,
        unpinned,
        dropped.join(", ")
    );
    Ok(())
}

/// Withdraw script approvals, so a sync that reaches one refuses to run it until `lock scripts`
/// approves it again (II.12).
fn unlock_scripts(app: &App, names: &[String]) -> Result<()> {
    use linix::core::hook_lock::HookLedger;

    let path = HookLedger::path_in(&app.config.layout().locks_dir());
    let mut ledger = HookLedger::load(&path)?;
    if ledger.is_empty() {
        println!("scripts: nothing is approved.");
        return Ok(());
    }
    let revoked: Vec<String> = ledger
        .entries()
        .filter(|(id, _)| scoped_by(id, names))
        .map(|(id, _)| id.to_string())
        .collect();
    if revoked.is_empty() {
        warn!("no approval matches {} — nothing withdrawn.", quoted(names));
        return Ok(());
    }
    for id in &revoked {
        ledger.revoke(id);
    }
    ledger.save(&path)?;
    let (tag, withdrew) = tense("Unlock", "withdrew", "would withdraw");
    println!(
        "{} scripts: {} {}. A sync that reaches one now refuses to run it until \
         `linix lock scripts` approves it again.",
        tag,
        withdrew,
        revoked.join(", ")
    );
    Ok(())
}

#[cfg(test)]
mod lock_axis_tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn pins(entries: &[(&str, &str)]) -> serde_json::Map<String, Value> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
            .collect()
    }

    /// No names is every key — a bare `lock versions` pins everything, as it always did.
    #[test]
    fn an_empty_scope_takes_everything() {
        assert!(scoped_by("apt:curl", &[]));
        assert!(scoped_by("after_install:nginx", &[]));
    }

    /// Both halves of a key are things a person types, on every axis. One rule, and the same
    /// rule for version pins, bare names, hooks, adapters, `exec:`, events and generators —
    /// so a scope cannot work on one ledger and quietly miss its twin.
    #[test]
    fn a_scope_matches_the_whole_key_or_its_tail() {
        for (key, tail) in [
            ("apt:curl", "curl"),
            ("after_install:nginx", "nginx"),
            ("adapters:backends.toml", "backends.toml"),
            ("exec:./setup.sh", "./setup.sh"),
            ("event:before_sync@repo", "before_sync@repo"),
            ("generate:./pick.sh", "./pick.sh"),
            ("health:systemctl is-active nginx", "systemctl is-active nginx"),
        ] {
            assert!(scoped_by(key, &names(&[key])), "the whole key: {key}");
            assert!(scoped_by(key, &names(&[tail])), "the tail: {key}");
            assert!(!scoped_by(key, &names(&["something-else"])), "{key}");
        }
    }

    /// A bare name with no `:` at all — every entry in `locks/bare.HOST.toml` is one.
    #[test]
    fn a_key_with_no_prefix_matches_itself_and_nothing_else() {
        assert!(scoped_by("ripgrep", &names(&["ripgrep"])));
        assert!(!scoped_by("ripgrep", &names(&["rip"])));
    }

    /// One name out of several still selects.
    #[test]
    fn any_of_the_names_selects() {
        assert!(scoped_by("apt:curl", &names(&["jq", "curl", "fd"])));
        assert!(!scoped_by("apt:curl", &names(&["jq", "fd"])));
    }

    /// Z2's second half: after an upgrade the pin names the version that was replaced, and the
    /// next ordinary sync converges back down to it. Moving the pin is what stops that.
    #[test]
    fn a_pin_follows_the_package_that_moved() {
        let mut locks = pins(&[("apt:curl", "7.81.0")]);
        let moved = move_pins_to(&mut locks, &pins(&[("apt:curl", "8.0.1")]));
        assert_eq!(moved, 1);
        assert_eq!(locks["apt:curl"], Value::String("8.0.1".into()));
    }

    /// An upgrade is not a `lock`. A package nobody pinned has no stale record to fight, so it
    /// gains no pin here — otherwise every `upgrade` would silently pin the whole machine.
    #[test]
    fn an_unpinned_package_gains_no_pin() {
        let mut locks = pins(&[("apt:curl", "7.81.0")]);
        let moved = move_pins_to(
            &mut locks,
            &pins(&[("apt:curl", "7.81.0"), ("cargo:ripgrep", "14.1.0")]),
        );
        assert_eq!(moved, 0, "nothing moved");
        assert_eq!(locks.len(), 1, "an unpinned package was pinned: {:?}", locks);
    }

    /// A manager that could not be read is not evidence that its package moved (V.7c's rule,
    /// applied to the pins): the recorded version stays rather than being dropped or blanked.
    #[test]
    fn a_pin_the_scan_could_not_read_keeps_its_version() {
        let mut locks = pins(&[("apt:curl", "7.81.0"), ("brew:jq", "1.7")]);
        let moved = move_pins_to(&mut locks, &pins(&[("apt:curl", "8.0.1")]));
        assert_eq!(moved, 1);
        assert_eq!(locks["brew:jq"], Value::String("1.7".into()));
    }

    /// Re-recording twice in a row moves nothing the second time, so a `sync --upgrade` that
    /// changed nothing does not rewrite the lockfile and make every run a commit.
    #[test]
    fn re_recording_an_already_current_pin_is_not_a_change() {
        let mut locks = pins(&[("apt:curl", "8.0.1")]);
        assert_eq!(move_pins_to(&mut locks, &pins(&[("apt:curl", "8.0.1")])), 0);
    }
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
