use crate::app::sync::resolver::StateResolver;
use crate::verbs::perform_maintenance;
use crate::verbs::prelude::*;
use crate::verbs::sync::{enforce_policy, print_vars_changed};

/// SEC2: once an install finishes, a verified package and an unverified one are
/// indistinguishable on disk. `@unverified` is only a real decision if it stays visible after
/// the fact, so what it bought is listed for as long as the package is installed.
///
/// The heading avoids "downloaded": since Q5 the flag also covers a manager that verifies a
/// signature itself (`helm`), where Shall downloaded nothing.
const UNVERIFIED_HEADING: &str = "! installed with `@unverified` — nothing checked the bytes";

/// Every managed package whose install skipped a verification. Reads the recorded option and
/// never the backend, so a backend that gains the flag is listed without editing this.
pub fn unverified_packages(state: &crate::core::StateRegistry) -> Vec<(String, String)> {
    state
        .packages
        .iter()
        .filter(|p| p.options.one("unverified").is_some_and(|v| v == "true"))
        .map(|p| (p.backend.clone(), p.name.clone()))
        .collect()
}

pub async fn handle_status(app: &App, out: Output) -> Result<()> {
    // This report ends with a crawl of every manager on the machine, so every manager is asked
    // either way — asked here they answer at once instead of in the order the sections below
    // happen to need them (`App::warm_installed`).
    app.inventory().await.warm_installed().await;
    let resolver =
        crate::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
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
    // `?`, not `unwrap_or_default()`. A resource plan that could not be built came back as an
    // empty one, and an empty one is indistinguishable from "no resource work" — so a `link:`
    // whose source is not on disk, which this now refuses to plan, would have been reported as
    // a converged machine (B4).
    let resources = app.extras().changes(&state).await?;
    // `status` reports what a full `sync` would do, so it scopes drift the same way.
    let hosts = app.resolver().await.host_backends().await;
    let changes = {
        let state_guard = app.state.lock().await;
        let planner = crate::app::sync::planner::ChangePlanner::new(
            app.registry.clone(),
            &state_guard,
            &app.config,
        );
        planner.plan(&desired, PlanScope::Whole(hosts)).await?
    };
    let report = changes.generate_report();
    // Likewise `?`: the crawl failing entirely is not the same as it finding nothing.
    let crawl = app.inventory().await.installed_but_undeclared().await?;
    let undeclared = crawl.packages;
    let unanswered = crawl.unanswered;

    let unverified: Vec<(String, String)> = {
        let state = app.state.lock().await;
        unverified_packages(&state)
    };

    if out.is_json() {
        let out = serde_json::json!({
            "to_install": report.install,
            "to_remove": report.remove,
            "undeclared": undeclared.iter().map(|p| serde_json::json!({"backend": p.backend, "name": p.name})).collect::<Vec<_>>(),
            "unverified": unverified.iter().map(|(b, n)| serde_json::json!({"backend": b, "name": n})).collect::<Vec<_>>(),
            "resources_to_place": resources.place,
            "resources_to_undo": resources.undo,
            "resources_unverifiable": resources.unverifiable,
            // The packages half of `resources_unverifiable`, which had no counterpart. Without
            // it a script consuming this document could not tell "no drift" from "three
            // managers never answered" — the distinction this codebase exists to make, lost at
            // the last step and only for packages (B4).
            "packages_unverifiable": unanswered,
            "left_in_place": report.skipped,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if report.install.is_empty()
        && report.remove.is_empty()
        && report.skipped.is_empty()
        && undeclared.is_empty()
        && unverified.is_empty()
        && resources.is_empty()
        // **The clean bill requires that every question was answered.** Without this, a run in
        // which three managers fell over printed the same sentence as a converged machine —
        // both produce an empty list, and only one of them is a clean bill (B4).
        && unanswered.is_empty()
    {
        println!(
            "System matches your manifests; nothing to install, no drift, nothing undeclared."
        );
        return Ok(());
    }
    if !unanswered.is_empty() {
        println!(
            "! could not be asked — nothing these managers have is counted below ({}):",
            unanswered.len()
        );
        for who in &unanswered {
            println!("    {}", who);
        }
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
    // Distinct from `undeclared` below, and the distinction is the point: an undeclared
    // package is one Shall never took responsibility for, and one of these is a package it
    // manages, that nothing declares, and that it has decided never to remove (AU1).
    // Grouped by kind: this list holds declined removals *and* declarations this machine cannot
    // act on, and "would leave in place" is only true of the first.
    for (kind, rows) in crate::app::sync::planner::Skipped::by_kind(&report.skipped) {
        println!("~ drift — {}:", kind.heading(rows.len()));
        for s in rows {
            println!("    {}  ({})", s.key, s.reason);
        }
    }
    if !undeclared.is_empty() {
        println!(
            "? undeclared — installed, nothing declares it, `purge-undeclared` removes it ({}):",
            undeclared.len()
        );
        for p in &undeclared {
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
        // Said out loud on this view too: these are resources Shall cannot read back, so
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
pub struct FullChanges {
    pub changes: crate::app::sync::SyncChanges,
    /// The resource half (N-2). `plan` froze only the package half, so a plan over three
    /// unapplied `link:` lines was an empty file and `apply` of it did nothing.
    pub resources: crate::app::apply::ResourceChanges,
    /// The resolved model itself — `apply` needs it to run the non-package phases, and its
    /// `vars` are what a plan freezes.
    pub state: crate::model::DesiredState,
}

pub async fn compute_full_changes(
    app: &App,
    frozen_vars: Option<crate::model::vars::Vars>,
) -> Result<FullChanges> {
    let resolver =
        crate::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let resolver = match frozen_vars {
        Some(v) => resolver.with_vars(v),
        None => resolver,
    };
    let state = resolver.resolve_model().await?;
    enforce_policy(app, &state.packages).await?;
    let resources = app.extras().changes(&state).await?;
    // A saved plan is what `sync` would do, frozen — so it is scoped the way `sync` scopes,
    // and for the stronger reason: `sync` re-plans every run and would correct itself, while
    // this plan is written to a file and applied later. Unscoped, it froze a removal for every
    // managed package whose backend `priority` does not name, and `apply` then carried them out
    // against a machine that had never agreed to Shall touching that manager.
    let hosts = app.resolver().await.host_backends().await;
    let changes = {
        let state_guard = app.state.lock().await;
        let planner = crate::app::sync::planner::ChangePlanner::new(
            app.registry.clone(),
            &state_guard,
            &app.config,
        );
        planner
            .plan(&state.packages, PlanScope::Whole(hosts))
            .await?
    };
    Ok(FullChanges {
        changes,
        resources,
        state,
    })
}

pub async fn handle_plan(app: &App, out: &str) -> Result<()> {
    let full = compute_full_changes(app, None).await?;
    // XIII.3's exit condition names `plan`: a script's hash, its run count and the decision
    // that follows are printed here, before anything happens. Read off the resolution
    // `compute_full_changes` already did — it used to resolve the model a second time for
    // this, which is one model resolved twice and free to disagree with itself.
    app.execs()
        .print_plan(&full.state, crate::model::exec::Verb::Sync);
    let created_at = chrono::Utc::now().timestamp();
    let mut plan =
        crate::app::sync::SavedPlan::from_changes(&full.changes, &full.resources, Some(created_at));
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
        // By kind. This sentence used to call every skipped row "installed, declared nowhere,
        // and will not be removed" — three clauses, all three false of a declaration this
        // machine cannot act on, which is the other half of what the list holds.
        let by_kind = crate::app::sync::planner::Skipped::by_kind(&full.changes.skipped)
            .into_iter()
            .map(|(kind, rows)| kind.heading(rows.len()))
            .collect::<Vec<_>>()
            .join("; ");
        println!("Wrote plan to {} — no actions. {} (above).", out, by_kind);
    } else if plan.is_empty() {
        println!(
            "Wrote plan to {} — system already matches desired state (no changes).",
            out
        );
        // Not silence: `check` says the same thing in the same breath, and a resource Shall
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
             undo.\nReview it, then run `shall apply {}`.",
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
            print_vars_changed(&app.config, &app.registry, &app.vcs(), &plan.vars).await;
        }
        // Writing a plan changes nothing, so this warns rather than refuses — but say it
        // here, where there is still time to fix the manifest, rather than letting the
        // refusal be a surprise at apply time.
        //
        // The question itself belongs to the guard (`preview_refusals`): a preview whose only
        // value is agreeing with the enforcer must not be a second implementation of it.
        let package_pairs: Vec<(String, String)> = plan
            .removals
            .iter()
            .map(|r| (r.backend.clone(), r.name.clone()))
            .collect();
        let extra_pairs = crate::app::sync::guard::extra_removal_pairs(&plan.resources.undo);
        let refusals = crate::app::sync::guard::preview_refusals(
            &app.config,
            &app.registry,
            plan.installs.len(),
            &package_pairs,
            &extra_pairs,
            crate::app::sync::guard::GuardScope::Apply,
        )
        .await;
        if !refusals.is_empty() {
            println!(
                "\nWARNING: `shall apply` will refuse this plan.\n{}",
                refusals.join("\n")
            );
        }
    }

    // **`Q-A`, ruled 2026-08-13: a read-only command that finds work exits 2.**
    //
    // `target-state.md` says exit 2 *"means a read-only command found work to do"*, and until
    // now only `check` ever built it: `shall plan` printed *"1 install(s), 0 removal(s)"* and
    // exited **0**. `plan` answers the question `check` answers *and* writes the artifact a
    // script consumes, so a pipeline that branches on drift reaches for it and is told the
    // machine has converged, every time.
    //
    // The condition is `check`'s condition, deliberately — the same quantities in the same
    // combination — because the rule this repo keeps paying for is that two readings of one
    // machine disagree. `list --outdated` is **not** given this treatment: a listing's subject
    // is inventory rather than a verdict, and one that exited non-zero for having contents
    // would be surprising in a way this is not.
    let found_work = !plan.is_empty()
        || !full.changes.skipped.is_empty()
        || !full.resources.unverifiable.is_empty();
    if found_work {
        return Err(crate::core::Error::Differences(format!(
            "the plan written to {out} is not empty"
        ))
        .into());
    }
    Ok(())
}

/// Rebuild a `SyncChanges` graph from a saved plan's install/removal lists — the interactive
/// review screen operates on a change graph, and so does the engine that executes one.
///
/// **This is the plan, not a rendering of it.** It used to call `add_node` in a loop and stop
/// there, which made it a list rather than a graph: a `@requires` a user wrote was honoured on
/// the run that planned it and dropped by the command whose promise is that the plan you
/// inspect is the plan you apply. `add_installs` wires the edges from the specs' own
/// `requires`, which the plan file carries — they were always in the JSON, and nothing read
/// them back.
pub fn saved_plan_to_changes(
    installs: &[crate::core::PackageSpec],
    removals: &[crate::app::sync::saved_plan::PlanRemoval],
) -> crate::app::sync::planner::SyncChanges {
    let mut changes = crate::app::sync::planner::SyncChanges::default();
    changes.add_installs(installs);
    for r in removals {
        changes.add_removal(&r.backend, &r.name);
    }
    changes
}

pub async fn handle_apply(app: &App, plan_path: &str, yes: bool) -> Result<()> {
    let raw = tokio::fs::read_to_string(plan_path)
        .await
        .with_context(|| format!("reading plan file {}", plan_path))?;
    let plan: crate::app::sync::SavedPlan =
        serde_json::from_str(&raw).context("parsing plan file")?;

    if plan.schema != crate::app::sync::PLAN_SCHEMA {
        anyhow::bail!(
            "plan schema {} is unsupported (this shall speaks schema {})",
            plan.schema,
            crate::app::sync::PLAN_SCHEMA
        );
    }
    // Integrity: refuse a hand-edited plan unless forced.
    if plan.recomputed_hash() != plan.desired_hash && !yes {
        anyhow::bail!(
            "plan file looks modified (content hash mismatch). Re-generate with `shall plan`, \
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
        let current = crate::app::sync::SavedPlan::from_changes(&now.changes, &now.resources, None);
        if current.desired_hash != plan.desired_hash {
            if yes {
                warn!("apply: system has drifted from the captured plan; applying anyway (--yes).");
            } else {
                println!(
                    "WARNING: the system/manifests have drifted since this plan was captured."
                );
                // Refuse rather than decline: aborting quietly says only "Aborted", naming
                // neither the reason nor `--yes`. Found by the test that enumerates prompts
                // from the source; no review had reported it.
                let proceed = crate::core::prompt::confirm(
                    false,
                    "Apply the captured plan anyway?",
                    crate::core::prompt::Unattended::Refuse(
                        "The captured plan no longer matches this machine, and there is no \
                         terminal to confirm on. Re-run with --yes to apply it anyway, or \
                         `shall plan` to capture a fresh one.",
                    ),
                )?;
                if !proceed {
                    println!("Aborted. Run `shall plan` to capture a fresh plan.");
                    return Ok(());
                }
            }
        }
    }

    if app.config.dry_run {
        crate::would_print!(
            "would install {} and remove {} package(s), place {} and undo {} \
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
    //
    // The screen returns the plan with the deselected nodes taken out, and that IS the plan
    // from here on. It used to hand back a set of surviving keys, which the caller then used
    // to `retain` two flat lists that a second call rebuilt into a graph — a round trip
    // through a vocabulary with no edges in it, so a deselection was also the only way to
    // lose an ordering that had survived everything else.
    let mut changes = saved_plan_to_changes(&plan.installs, &plan.removals);

    // II.7c, and this is the command the rule exists for: a plan file is the one `SyncChanges`
    // that routinely crosses machines, so it is the one that can name a manager the host
    // running it does not have. Filtered here rather than left to the engine's backstop
    // because the two counts below are read *before* the engine is called — a summary saying
    // `4 installed` over a machine that got two is the same lie by a shorter route.
    changes.withdraw_what_this_machine_cannot_run(&app.registry);
    let skipped = std::mem::take(&mut changes.skipped);

    if !yes && !app.config.yes {
        use std::io::IsTerminal;
        if std::io::stdin().is_terminal() {
            let mut preview = TuiPreview::new(&changes, HashMap::new());
            if !crate::core::on_the_terminal(|| preview.run())? {
                println!("Apply cancelled.");
                return Ok(());
            }
            changes = preview.get_filtered_changes();
            if changes.is_empty() {
                println!("All changes deselected — nothing to apply.");
                return Ok(());
            }
        }
    }

    // **The frozen plan is executed by the engine that executes every other plan.**
    //
    // This was a pair of serial loops calling `Installable::install` and `::remove` a package
    // at a time, and what they skipped was not decoration. No write-ahead log — so `shall heal`,
    // which reads the journal, could not recover an interrupted `apply` at all, on the one
    // command named after review and deliberation. No transaction, so no rollback and no
    // prior-state probe. No snapshot and no health check, so `@health=` on a line in the plan
    // was checked when `sync` applied it and not when `apply` did. No batching: one
    // `apt install` per package, which the engine's own measurement puts at ten times the cost
    // of one command (V.115). And a failure was a `warn!` and a `continue`, so `apply` reported
    // `Applied plan` over a machine where half of it had failed.
    //
    // Every one of those is a property of `SyncEngine::sync`, and the freeze survives the
    // change because `sync` does not plan — it executes the `SyncChanges` it is handed, which
    // here is the one rebuilt from the file above and trimmed by the review screen. The guard
    // calls this function used to make itself are the engine's first act (`removal_pairs` over
    // the same graph, the same `GuardScope::Apply`), so they are gone from here rather than
    // duplicated: two calls to one guard is how a scope comes to disagree with itself.
    let installed = changes.total_install();
    let removed = changes.total_remove();
    let engine = app.sync_engine();
    engine
        .sync(changes, crate::app::sync::guard::GuardScope::Apply)
        .await?;

    // The resource half, through the same phase list `sync` runs (N-2). Not a second
    // implementation: `apply_non_package_phases` is the one list, and the comment above it
    // records what four separate copies of it already cost. It carries its own guard for the
    // teardown, against `app.reaping` — the removals this command has already cleared, which
    // it reads rather than being told.
    let resources = if plan.resources.is_empty() {
        0
    } else {
        crate::verbs::sync::apply_non_package_phases(
            app,
            &now.state,
            crate::app::sync::guard::GuardScope::Apply,
        )
        .await?
    };

    println!(
        "Applied plan: {} installed, {} removed, {} resource(s) reconciled.",
        installed, removed, resources
    );
    crate::verbs::sync::print_skipped(&skipped);
    perform_maintenance(app).await
}

/// Where the version pins live (II.6): in the `locks/` directory beside the hook and extras
/// ledgers, never a stray `locks.json` beside that directory.
pub fn version_lock_path(config: &Config) -> std::path::PathBuf {
    config.config_root().join("locks").join("versions.json")
}

/// The pins on disk. A missing or unreadable file is an empty set of pins — the ordinary state
/// of a machine that has never run `shall lock`, never an error.
pub fn load_version_locks(path: &std::path::Path) -> serde_json::Map<String, Value> {
    let Ok(body) = std::fs::read_to_string(path) else {
        return serde_json::Map::new();
    };
    serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|doc| doc.get("locks").and_then(Value::as_object).cloned())
        .unwrap_or_default()
}

/// Write the pins back. Returns whether the bytes reached the disk — a preview pins nothing.
pub async fn write_version_locks(
    path: &std::path::Path,
    locks: &serde_json::Map<String, Value>,
) -> Result<bool> {
    if !crate::core::dry_run::active() {
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await.ok();
        }
    }
    let doc = serde_json::json!({ "locks": locks });
    crate::utils::file::persist(path, &serde_json::to_string_pretty(&doc)?)
        .with_context(|| format!("Failed to write {}", path.display()))
}

/// The version every managed package is at *now*, keyed `backend:name`.
///
/// The live answer from the backend, falling back to recorded state. `list_installed` is memoized
/// once per run (`Queryable::list_installed`), so asking `info` per package costs one command per
/// manager, not one per package.
pub(crate) async fn scan_installed_versions(
    state: &tokio::sync::Mutex<crate::core::StateRegistry>,
    registry: &crate::backends::BackendRegistry,
) -> serde_json::Map<String, Value> {
    let mut locks = serde_json::Map::new();
    let state = state.lock().await;
    for pkg in &state.packages {
        let version = match registry
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
/// versions pinned. Shared by `shall lock versions` and by `shall heal` (which reconciles the
/// lockfile).
pub async fn build_and_write_locks(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    state: &tokio::sync::Mutex<crate::core::StateRegistry>,
) -> Result<(usize, bool)> {
    let locks = scan_installed_versions(state, registry).await;
    let count = locks.len();
    let written = write_version_locks(&version_lock_path(config), &locks).await?;
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
pub async fn refresh_version_locks(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    state: &tokio::sync::Mutex<crate::core::StateRegistry>,
) -> Result<usize> {
    // A preview moved no version, so there is nothing to re-record — and reporting a count it
    // could not write is the "would" that reads as "did".
    if crate::core::dry_run::active() {
        return Ok(0);
    }
    let path = version_lock_path(config);
    let mut locks = load_version_locks(&path);
    if locks.is_empty() {
        return Ok(0);
    }
    let current = scan_installed_versions(state, registry).await;
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
pub fn scoped_by(key: &str, names: &[String]) -> bool {
    if names.is_empty() {
        return true;
    }
    let tail = key.split_once(':').map_or(key, |(_, rest)| rest);
    names.iter().any(|n| n == key || n == tail)
}

/// The heading and verb a message uses. Every ledger **these** commands write goes through
/// `utils::file::persist`, so the answer about one of them is the answer about all: a preview
/// pins nothing, approves nothing and forgets nothing.
///
/// Scoped to these commands on purpose. `persist` is the config repo's preview policy, not the
/// program's only one — the executor diverts machine writes into a dry-run VFS instead, so that
/// a previewed command can read back what a previewed command would have written. A claim that
/// every writer goes through `persist` would be false, and this one does not make it.
fn tense(label: &str, done: &'static str, would: &'static str) -> (String, &'static str) {
    if crate::core::dry_run::active() {
        (
            format!("{} {}:", crate::core::dry_run::MARKER, label),
            would,
        )
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

/// `shall lock [AXIS] [NAME…]` — freeze what a sync would otherwise decide again (Z2).
pub async fn handle_lock(app: &App, axis: LockAxis, names: &[String], list: bool) -> Result<()> {
    if list {
        return list_locks(&app.config, &app.registry, axis);
    }
    // Scripts before either axis that resolves the model, and generators first within scripts:
    // resolving *runs* generators, so a command that resolved first could never reach the
    // generator it exists to approve (U33).
    if axis.covers(LockAxis::Scripts) {
        lock_scripts(&app.config, &app.hooks, &app.registry, names).await?;
    }
    if axis.covers(LockAxis::Versions) {
        lock_versions(&app.config, &app.registry, &app.state, names).await?;
    }
    if axis.covers(LockAxis::Backends) {
        lock_backends(&app.config, &app.registry, names).await?;
    }
    Ok(())
}

/// Pin the installed version of every managed package, or of the ones `names` picks out.
async fn lock_versions(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    state: &tokio::sync::Mutex<crate::core::StateRegistry>,
    names: &[String],
) -> Result<()> {
    let (tag, pinned) = tense("Lock", "pinned", "would pin");
    let path = version_lock_path(config);
    if names.is_empty() {
        let (count, _) = build_and_write_locks(config, registry, state).await?;
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
    for (key, version) in scan_installed_versions(state, registry).await {
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
async fn lock_backends(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    names: &[String],
) -> Result<()> {
    use crate::core::BareLock;

    let path = BareLock::path_in(&config.layout().locks_dir());
    let before = BareLock::load(&path)?;
    let resolver = crate::app::sync::resolver::StateResolver::new(config, registry.clone(), false)
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
async fn lock_scripts(
    config: &Config,
    hooks: &Arc<crate::app::LuaHooks>,
    registry: &Arc<BackendRegistry>,
    names: &[String],
) -> Result<()> {
    use crate::core::hook_lock::HookLedger;

    let ledger_path = HookLedger::path_in(&config.layout().locks_dir());
    let before = HookLedger::load(&ledger_path)?;
    let (tag, approved) = tense("Lock", "approved", "would approve");
    // Scoped runs report from the ledger afterwards: each approver counts what it read, which is
    // everything, and printing those counts beside a scope would be a false sentence.
    let scoped = !names.is_empty();

    // Generators are approved FIRST, by scanning the files — before anything calls
    // `resolve_model`, which now runs generators and would refuse an unapproved one, so the very
    // command that approves it could never resolve far enough to reach it (U33).
    let generators = approve_generate_commands(config, registry)?;
    if generators > 0 && !scoped {
        println!(
            "{} {} {} generate command(s) at their current hash.",
            tag, approved, generators
        );
    }
    // II.12: `lock` is also how you approve hooks. Record the current hash of every hook so a
    // later change to any of them stops the next sync until it is re-approved here. "Hash
    // everything, including your own scripts" — one rule, no exceptions.
    let hooks = hooks.approve_all_hooks()?;
    if hooks > 0 && !scoped {
        println!(
            "{} {} {} hook(s) at their current script hash ({}).",
            tag,
            approved,
            hooks,
            ledger_path.display()
        );
    }
    // A hook on one of Shall's own events (XIII.13) is the same surface: a script the repo
    // carries, run without anyone watching. Both of U15's locations are approved here, and
    // separately — the shared policy's approval must not cover this machine's local file.
    let events = crate::app::events::EventHooks::load(config);
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
    if let Some(file) = approve_vars_provider(config)? {
        if !scoped {
            println!(
                "{} {} the vars provider `{}` at its current hash.",
                tag, approved, file
            );
        }
    }
    // And every `adapters/` file (7a/U10). They travel with the repo, and a definition is
    // argv Shall will run, so each is approved here or it does not load.
    for name in approve_adapters(config)? {
        if !scoped {
            println!(
                "{} {} `adapters/{}` at its current hash.",
                tag, approved, name
            );
        }
    }
    // And every declared `exec:` script (XIII.3). II.12 admits no exceptions: a script the
    // configuration runs is approved by this command or it does not run.
    let model = resolve_for_approval(config, registry).await?;
    let execs = approve_exec_scripts(config, &model).await?;
    if execs > 0 && !scoped {
        println!(
            "{} {} {} exec script(s) at their current hash.",
            tag, approved, execs
        );
    }
    // And every user-declared health-check COMMAND (U31). A check is argv, run after a change,
    // so it is on the same trust model — approved here or the check counts as failed.
    let health = approve_health_checks(config, &model).await?;
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
    if crate::core::dry_run::active() {
        println!(
            "{} {} the entries matching {}",
            tag,
            approved,
            quoted(names)
        );
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
             `shall lock scripts --list` names what is approvable.",
            quoted(names)
        );
    }
    ledger.save(&ledger_path)?;
    if !hit.is_empty() {
        println!("{} {} {}", tag, approved, hit.join(", "));
    }
    Ok(())
}

/// `shall lock --list` / `shall unlock --list` — what is locked on this axis, changing nothing.
fn list_locks(config: &Config, registry: &Arc<BackendRegistry>, axis: LockAxis) -> Result<()> {
    use crate::core::hook_lock::HookLedger;
    use crate::core::BareLock;

    let locks_dir = config.layout().locks_dir();
    if axis.covers(LockAxis::Versions) {
        let locks = load_version_locks(&version_lock_path(config));
        if locks.is_empty() {
            println!("versions: nothing is pinned.");
        } else {
            for (key, version) in &locks {
                // **Say which of these can actually be put back** (`Q53`). A lockfile does two
                // jobs — reproduce, and detect drift — and only the second works on a manager
                // that cannot be asked for a version. Recording one there is still right, and it
                // is what makes `check drift` work on brew and pacman; reading the file and
                // believing every line is replayable is what is wrong. The mark is here rather
                // than in the file because the file is a record other tools read, and adding a
                // shape to it would break every reader for a sentence only a person needs.
                let replayable = key
                    .split_once(':')
                    .is_some_and(|(backend, _)| registry.pins_version(backend));
                println!(
                    "versions: {} -> {}{}",
                    key,
                    version.as_str().unwrap_or("?"),
                    if replayable {
                        ""
                    } else {
                        "  (observed; this manager cannot be asked to install it)"
                    }
                );
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
pub fn approve_generate_commands(
    config: &Config,
    registry: &Arc<BackendRegistry>,
) -> Result<usize> {
    use crate::config::grammar::{parse_document, Statement};
    use crate::core::hook_lock::{generate_id, hash_script, HookLedger};

    let layout = config.layout();
    let known = |name: &str| registry.get(name).is_some();
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
            config.config_root().join(declared)
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
pub async fn resolve_for_approval(
    config: &Config,
    registry: &Arc<BackendRegistry>,
) -> Result<crate::model::DesiredState> {
    crate::app::sync::resolver::StateResolver::new(config, registry.clone(), false)
        .await
        .resolve_model()
        .await
        .map_err(Into::into)
}

pub async fn approve_exec_scripts(
    config: &Config,
    state: &crate::model::DesiredState,
) -> Result<usize> {
    use crate::core::hook_lock::{exec_id, hash_script, HookLedger};

    if !state.has_execs() {
        return Ok(0);
    }
    let locks = config.layout().locks_dir();
    let path = HookLedger::path_in(&locks);
    let mut ledger = HookLedger::load(&path)?;
    let mut approved = 0usize;
    for (script, _opts, origin) in state.execs() {
        // A catalogued step (`H8`) has no file to hash and no approval to give: it is a row
        // compiled into this binary, the same status `builtin_backends.toml` has, and
        // `Execs::exec_plan` treats it as approved without asking. Reading it as a path sent
        // this walk after `<config>/step/rustup`, which nobody wrote — so `shall lock` failed
        // outright on any configuration that used the catalogue, taking every OTHER script's
        // approval down with it.
        if crate::model::step::named(script).is_some() {
            continue;
        }
        let declared = std::path::Path::new(script);
        let full = if declared.is_absolute() {
            declared.to_path_buf()
        } else {
            config.config_root().join(declared)
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
pub async fn approve_health_checks(
    config: &Config,
    state: &crate::model::DesiredState,
) -> Result<usize> {
    use crate::core::hook_lock::{hash_script, health_id, HookLedger};
    use crate::model::health::Probe;

    let mut commands: Vec<String> = Vec::new();
    for specs in state.packages.values() {
        for spec in specs {
            if let Some(Probe::Command(cmd)) = spec.options.one("health").and_then(Probe::parse) {
                commands.push(cmd);
            }
        }
    }
    for written in &config.health {
        if let Some(Probe::Command(cmd)) = Probe::parse(written) {
            commands.push(cmd);
        }
    }
    if commands.is_empty() {
        return Ok(0);
    }
    let path = HookLedger::path_in(&config.layout().locks_dir());
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
pub fn approve_adapters(config: &Config) -> Result<Vec<String>> {
    use crate::core::hook_lock::{adapter_id, hash_script, HookLedger};

    let layout = config.layout();
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
pub fn approve_vars_provider(config: &Config) -> Result<Option<String>> {
    use crate::core::hook_lock::{hash_script, vars_id, HookLedger};
    use crate::model::vars_provider::{self, Kind};

    let root = config.config_root();
    let Some(selected) = vars_provider::select(&root, &config.vars.source)? else {
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

/// `shall unlock [AXIS] [NAME…]` — release a lock, so the next sync decides it again (Z2).
pub async fn handle_unlock(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    resolver: &StateResolver<'_>,
    axis: LockAxis,
    names: &[String],
    list: bool,
) -> Result<()> {
    if list {
        return list_locks(config, registry, axis);
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
        resolver.require_known_spec_backends(names).await?;
    }
    if axis.covers(LockAxis::Backends) {
        unlock_backends(config, names).await?;
    }
    if axis.covers(LockAxis::Versions) {
        unlock_versions(config, names).await?;
    }
    if axis.covers(LockAxis::Scripts) {
        unlock_scripts(config, names)?;
    }
    Ok(())
}

/// Forget which manager an unpinned name resolved to, so the next sync asks again (II.6).
async fn unlock_backends(config: &Config, names: &[String]) -> Result<()> {
    let path = crate::core::BareLock::path_in(&config.layout().locks_dir());
    let mut lock = crate::core::BareLock::load(&path)?;
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
            "Run `shall sync` to re-resolve. A name that moves manager is reinstalled from \
             the new one and removed from the old."
        );
    }
    Ok(())
}

/// Drop the version pins, so the next sync takes what the managers offer.
async fn unlock_versions(config: &Config, names: &[String]) -> Result<()> {
    let path = version_lock_path(config);
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
fn unlock_scripts(config: &Config, names: &[String]) -> Result<()> {
    use crate::core::hook_lock::HookLedger;

    let path = HookLedger::path_in(&config.layout().locks_dir());
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
         `shall lock scripts` approves it again.",
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
            (
                "health:systemctl is-active nginx",
                "systemctl is-active nginx",
            ),
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
        assert_eq!(
            locks.len(),
            1,
            "an unpinned package was pinned: {:?}",
            locks
        );
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
mod frozen_plan_tests {
    use super::*;
    use crate::app::sync::saved_plan::PlanRemoval;
    use crate::core::{GraphAction, PackageSpec};

    fn spec(backend: &str, name: &str, requires: &[&str]) -> PackageSpec {
        PackageSpec {
            name: name.into(),
            backend: backend.into(),
            options: Default::default(),
            requires: requires.iter().map(|s| s.to_string()).collect(),
            present: true,
        }
    }

    /// `plan --help` promises "the exact plan you inspect is the one you later apply", and an
    /// ordering is part of a plan. This rebuilt the graph with `add_node` and no edges, so a
    /// `@requires` held when `sync` applied it and was dropped when `apply` did — silently,
    /// because an edgeless graph runs perfectly well in the wrong order.
    #[test]
    fn a_frozen_plan_keeps_the_ordering_its_specs_declare() {
        let installs = vec![
            spec("apt", "nginx", &["apt:libfoo"]),
            spec("apt", "libfoo", &[]),
        ];
        let changes = saved_plan_to_changes(&installs, &[]);

        assert_eq!(changes.graph.node_count(), 2);
        assert_eq!(
            changes.graph.edge_count(),
            1,
            "the `@requires` in the plan file was read back and dropped"
        );
        let libfoo = changes.install_map["apt:libfoo"];
        let nginx = changes.install_map["apt:nginx"];
        assert!(
            changes.graph.contains_edge(libfoo, nginx),
            "the edge must run requirement -> dependent, or the batch is ordered backwards"
        );
    }

    /// The control: a plan whose specs require nothing goes on one command line. This is the
    /// same property from the other side — an edge that should not exist costs a whole extra
    /// manager invocation, measured at ten times the batched cost (V.115).
    #[test]
    fn a_plan_with_no_requires_has_no_edges_to_split_the_batch() {
        let installs = vec![spec("apt", "htop", &[]), spec("apt", "jq", &[])];
        let changes = saved_plan_to_changes(&installs, &[]);
        assert_eq!(changes.graph.edge_count(), 0);
    }

    /// A `requires` naming something that is not in this plan is not an edge — it is already
    /// on the machine, and inventing a node for it would install a package nobody froze.
    #[test]
    fn a_requirement_outside_the_plan_adds_nothing() {
        let installs = vec![spec("apt", "nginx", &["apt:already-there"])];
        let changes = saved_plan_to_changes(&installs, &[]);
        assert_eq!(changes.graph.node_count(), 1);
        assert_eq!(changes.graph.edge_count(), 0);
    }

    /// Removals reach the graph and the tracker together. The tracker is what `declined`
    /// consults to answer "is this already scheduled", so a removal in one and not the other
    /// is a removal that can be scheduled twice.
    #[test]
    fn a_frozen_removal_is_in_the_graph_and_in_the_tracker() {
        let removals = vec![PlanRemoval {
            backend: "apt".into(),
            name: "vim".into(),
        }];
        let changes = saved_plan_to_changes(&[], &removals);

        assert_eq!(changes.total_remove(), 1);
        assert!(changes.removal_tracker.contains("apt:vim"));
        assert!(matches!(
            changes.graph.node_weights().next(),
            Some(GraphAction::Remove { name, backend }) if name == "vim" && backend == "apt"
        ));
    }

    /// And the pair the engine's guard reads: `guard::removal_pairs` over this graph must find
    /// the removals, because `apply` no longer calls the guard itself — it hands the plan to
    /// `SyncEngine::sync`, whose first act is to enforce over exactly this.
    #[test]
    fn the_engine_guard_can_see_a_frozen_plans_removals() {
        let removals = vec![
            PlanRemoval {
                backend: "apt".into(),
                name: "vim".into(),
            },
            PlanRemoval {
                backend: "brew".into(),
                name: "fd".into(),
            },
        ];
        let changes = saved_plan_to_changes(&[spec("apt", "htop", &[])], &removals);
        let mut pairs = crate::app::sync::guard::removal_pairs(&changes);
        pairs.sort();
        assert_eq!(
            pairs,
            vec![
                ("apt".to_string(), "vim".to_string()),
                ("brew".to_string(), "fd".to_string())
            ],
            "the guard the engine runs must see every removal the plan froze"
        );
    }
}

#[cfg(test)]
mod unverified_tests {
    use super::*;
    use crate::core::state::ManagedPackage;

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
            source: "test".into(),
            is_transient: false,
            session_id: None,
        }
    }

    /// Every backend the flag is legal on stays visible after the install — the download ones
    /// and, since Q5, the manager that verifies a signature itself.
    #[test]
    fn what_skipped_a_check_is_listed_whichever_backend_skipped_it() {
        let state = crate::core::StateRegistry {
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

    /// helm downloads nothing Shall can see, so the heading cannot claim it did.
    #[test]
    fn the_heading_does_not_claim_shall_downloaded_it() {
        assert!(!UNVERIFIED_HEADING.contains("downloaded"));
        assert!(UNVERIFIED_HEADING.contains("@unverified"));
    }
}
