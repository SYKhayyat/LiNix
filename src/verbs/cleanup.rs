use crate::app::sync::guard;
use crate::app::sync::resolver::StateResolver;
use crate::verbs::perform_maintenance;
use crate::verbs::prelude::*;

/// `remove-orphans` — each manager's own "no longer needed by anything" set.
///
/// The orphan set is the backend's opinion, not Shall's model, which is exactly why it gets
/// the same shape as `sync`: name every package first, put it through the guard, then ask.
/// The old `clean` ran `apt autoremove -y` / `pacman -Rs --noconfirm` across every available
/// backend with no preview and outside the guard.
pub async fn handle_remove_orphans(app: &App) -> Result<()> {
    use crate::app::sync::guard::{enforce, GuardScope};

    let mut listed: Vec<(String, Vec<String>)> = Vec::new();
    let mut cannot_say: Vec<String> = Vec::new();

    // Read-only and independent per manager; ordered so the report is stable.
    use futures::stream::StreamExt;
    let answers: Vec<(String, crate::core::Result<Vec<String>>)> =
        // What Shall uses: `remove-orphans` deletes, and a manager `priority` excludes is one
        // Shall must not be deleting through.
        futures::stream::iter(app.backends().await.usable()?)
            .filter_map(|backend| async move {
                let up = backend.as_upgradable()?.clone();
                Some((backend.name().to_string(), up))
            })
            .map(|(name, up)| async move { (name, up.list_orphans().await) })
            .buffered(app.config.max_parallel.max(1))
            .collect()
            .await;

    for (name, answer) in answers {
        match answer {
            Ok(names) if names.is_empty() => {}
            Ok(names) => listed.push((name, names)),
            Err(crate::core::Error::Unsupported(_)) => cannot_say.push(name),
            Err(e) => warn!("could not list orphans for {}: {}", name, e),
        }
    }

    // Named, every time, whether or not anything else was found: a manager silently missing
    // from a removal report reads as a manager with nothing to remove.
    if !cannot_say.is_empty() {
        println!(
            "No orphan removal for: {}. These managers cannot say what they would delete, so \
             Shall does not let them delete it.",
            cannot_say.join(", ")
        );
    }

    if listed.is_empty() {
        println!("No orphaned packages.");
        return Ok(());
    }

    let removals: Vec<(String, String)> = listed
        .iter()
        .flat_map(|(b, names)| names.iter().map(move |n| (b.clone(), n.clone())))
        .collect();

    println!("Planned changes:");
    for (backend, names) in &listed {
        println!("  {} — remove {} package(s):", backend, names.len());
        for n in names {
            println!("      {}:{}", backend, n);
        }
    }

    // The guard sees the whole set at once, so the removal count and the protected list are
    // judged against the total rather than per backend.
    // Asked here so the preview and the refusal happen before the confirmation prompt — the
    // engine asks the same question again over the same pairs, which is cheap and cannot
    // disagree with itself.
    enforce(
        &app.config,
        &app.registry,
        &removals,
        &app.reaping,
        GuardScope::RemoveOrphans,
    )
    .await?;

    if app.config.dry_run {
        println!();
        crate::would_print!("Nothing was removed.");
        return Ok(());
    }

    if !confirm_orphan_removal(&app.config)? {
        println!("Nothing removed.");
        return Ok(());
    }

    // **Executed by the one engine, not by a loop of this command's own.** This used to be a
    // per-backend `installable.remove` here, with its own journalling and no transaction — so
    // `remove-orphans` had no write-ahead recovery, no rollback, and no batching, and a kill
    // part-way through left a machine whose only account of what went was the terminal
    // scrollback. `plan.rs-499` records what that shape cost `apply`, which stopped keeping
    // its own loop for the same reasons; this is the same fix on the next command along.
    //
    // The guard already ran above, over the whole set at once. The engine asks again — through
    // `GuardScope::RemoveOrphans`, the same scope, over the same pairs — and asking a settled
    // question twice with the same inputs is cheap and cannot disagree with itself.
    execute_removals_through_the_engine(
        &app.sync_engine(),
        &removals,
        guard::GuardScope::RemoveOrphans,
    )
    .await?;
    for (backend_name, names) in &listed {
        println!("  {}: removed {} package(s)", backend_name, names.len());
    }

    perform_maintenance(app).await
}

/// Hand a set of `(backend, name)` removals to `SyncEngine` rather than removing them here.
///
/// **`LX-5`: four commands removed or installed outside the planner.** Sugar that routes through
/// `sync` is the model working, and `install`/`uninstall`/`teleport`/`rollback`/`activate` all do
/// — `packages.rs` states the rule outright. `remove-orphans` and `purge-undeclared` did not:
/// each had its own preview, its own confirm, its own journalling loop, and neither ever saw
/// `ChangePlanner` or `SyncEngine`. What they lost by that is not abstract — no transaction, no
/// write-ahead recovery, no rollback, and one manager invocation per package where `Y1` measured
/// batching at 12,465 ms against 3,161 ms.
///
/// The graph is built here rather than by `ChangePlanner` because these removals are not
/// `desired − present`: the orphan set comes from each manager's own answer, and the undeclared
/// set from Shall's registry. The planner's job is deciding *what* to remove and both commands
/// have already decided; what they were missing is the engine that carries it out.
async fn execute_removals_through_the_engine(
    engine: &crate::app::SyncEngine,
    removals: &[(String, String)],
    scope: guard::GuardScope,
) -> Result<()> {
    use petgraph::stable_graph::StableDiGraph;
    let mut graph: StableDiGraph<crate::core::GraphAction, ()> = StableDiGraph::new();
    for (backend, name) in removals {
        graph.add_node(crate::core::GraphAction::Remove {
            name: name.clone(),
            backend: backend.clone(),
        });
    }
    let changes = crate::app::sync::planner::SyncChanges {
        graph,
        install_map: Default::default(),
        removal_tracker: removals
            .iter()
            .map(|(b, n)| format!("{}:{}", b, n))
            .collect(),
        skipped: Vec::new(),
    };
    Ok(engine.sync(changes, scope).await?)
}

pub fn confirm_orphan_removal(config: &Config) -> Result<bool> {
    Ok(crate::core::prompt::confirm(
        config.yes,
        "Remove these packages?",
        crate::core::prompt::Unattended::Refuse(
            "Refusing to remove orphans without confirmation in a non-interactive shell. Re-run with --yes to proceed, or --dry-run to preview.",
        ),
    )?)
}

/// `clean-cache` — downloaded archives and build caches (X.3 levels 1–2). Removes no installed
/// package, so it needs no preview and no guard: the guard protects packages, not disk space,
/// and widening it to cover caches would dilute what a refusal means (K16).
///
/// `--all` additionally clears Shall's own transient download area. It does NOT touch the
/// installed artifact directories — those hold software that is on `PATH`, and deleting them
/// is a removal (level 4), not a cache clean.
pub async fn handle_clean_cache(app: &App, all: bool) -> Result<()> {
    if app.config.dry_run {
        crate::would_print!("Would clear the package cache for every backend that has one.");
        crate::would_print!("Would forget the installed listings Shall has cached.");
        if all {
            crate::would_print!("Would also clear Shall's own download cache.");
        }
        return Ok(());
    }

    // The listings go first, and unconditionally. This is the command a user reaches for when
    // something outside Shall changed the machine and they know it before `installed_cache_secs`
    // does — so it must work even on a machine where the cache is turned off, since the files
    // could have been written by a run that had it on.
    match crate::core::installed::InstalledListings::forget_on_disk() {
        Ok(0) => {}
        Ok(n) => println!("Forgot {} cached installed listing(s).", n),
        Err(e) => warn!("could not clear the installed-listing cache: {}", e),
    }
    // Independent per manager — each clears its own cache directory and they contend for
    // nothing. `run_exclusive` still serialises anything that shares a manager lock.
    use futures::stream::StreamExt;
    // What Shall uses: clearing the download cache of a manager the user told Shall not to
    // touch is touching it.
    let cleanable: Vec<(String, bool, std::sync::Arc<dyn crate::core::Upgradable>)> = app
        .backends()
        .await
        .usable()?
        .into_iter()
        .filter_map(|b| {
            Some((
                b.name().to_string(),
                b.sudo_for_write(),
                b.as_upgradable()?.clone(),
            ))
        })
        .collect();
    let outcomes: Vec<(String, crate::core::Result<()>)> = futures::stream::iter(cleanable)
        .map(|(name, sudo, up)| async move { (name, up.clean_cache(sudo).await) })
        .buffered(app.config.max_parallel.max(1))
        .collect()
        .await;

    let mut cleaned = Vec::new();
    for (name, outcome) in outcomes {
        match outcome {
            Ok(()) => cleaned.push(name),
            Err(crate::core::Error::Unsupported(_)) => {}
            Err(e) => warn!("cache clean failed for {}: {}", name, e),
        }
    }
    if cleaned.is_empty() {
        println!("No backend on this machine has a cache to clear.");
    } else {
        println!("Cleared caches: {}.", cleaned.join(", "));
    }

    if all {
        let tmp = &app.config.tmp_dir;
        if tmp.exists() {
            match tokio::fs::remove_dir_all(tmp).await {
                Ok(()) => {
                    tokio::fs::create_dir_all(tmp).await.ok();
                    println!("Cleared Shall's download cache ({}).", tmp.display());
                }
                Err(e) => warn!("could not clear {}: {}", tmp.display(), e),
            }
        } else {
            println!("Shall's download cache is already empty.");
        }
    }

    perform_maintenance(app).await
}

/// Does "manage this much, delete that much" read as a mistake (II.11)?
///
/// **One function, because the tests used to hold their own copy of the arithmetic** — a
/// private helper in `purge_tests` commented "the ratio, as `handle_purge_undeclared` computes
/// it", which is a claim about a second implementation rather than a test of the first.
///
/// The threshold is `[guard] purge_ratio`, and `0.0` turns the rule off for someone who means
/// it. Both counts must be taken over the **same set of managers** — see
/// [`managed_where_the_crawl_could_see`].
fn reads_as_a_mistake(cfg: &Config, managed: usize, to_remove: usize) -> bool {
    let floor = cfg.guard.purge_ratio;
    floor > 0.0 && (managed as f64 / to_remove as f64) < floor
}

/// How many packages Shall manages *through the managers the crawl actually answered for*.
///
/// **This is the other half of [`reads_as_a_mistake`], and it has to be counted the same way
/// the deletion list was.** `installed_but_undeclared` surveys `priority`'s managers only, and
/// drops any that failed to list, so a package can never reach the deletion side unless its
/// manager is in [`UndeclaredReport::answered`]. Counting the management side over the whole
/// state file therefore weighs one machine against a different one.
///
/// The mismatch errs in one direction and it is the wrong one. Every manager present in the
/// state but missing from the crawl adds to the numerator and nothing to the denominator, so
/// the ratio rises, so the refusal is withdrawn. On the macOS nightly of 2026-08-14 that is
/// exactly what happened: 43 managed against 276 undeclared reads as 0.156 and cleared the
/// bar, `purge-undeclared` proceeded, and the only thing that saved the machine was that all
/// 276 removals happened to fail.
fn managed_where_the_crawl_could_see(
    packages: &[crate::core::state::ManagedPackage],
    answered: &[String],
) -> usize {
    let seen: std::collections::HashSet<&str> = answered.iter().map(String::as_str).collect();
    packages
        .iter()
        .filter(|p| seen.contains(p.backend.as_str()))
        .count()
}

/// `purge-undeclared` (II.11): delete everything Shall does not manage.
///
/// The residual risk, stated plainly because the docs must state it: `adopt` is an estimate.
/// If it missed something, this deletes it.
pub async fn handle_purge_undeclared(app: &App, allow_mass_purge: bool) -> Result<()> {
    let crawl = app.inventory().await.installed_but_undeclared().await?;
    let undeclared = crawl.packages;
    // A manager that could not be listed is safe for the *deletion* — nothing it has can end
    // up on the list, so this removes less and never more — and unsafe for the *sentence*.
    // Said before the list either way, because a user about to delete on the strength of it is
    // entitled to know the survey has a hole in it (B4's sibling).
    if !crawl.unanswered.is_empty() {
        println!(
            "! {} manager(s) could not be listed, so nothing they have appears below:",
            crawl.unanswered.len()
        );
        for who in &crawl.unanswered {
            println!("    {}", who);
        }
    }
    if undeclared.is_empty() {
        match crawl.unanswered.is_empty() {
            true => println!("Nothing to do: Shall manages every installed package."),
            false => println!(
                "Nothing to delete from the managers that answered. Whether Shall manages \
                 every installed package is not known — see above."
            ),
        }
        return Ok(());
    }

    let managed =
        managed_where_the_crawl_could_see(&app.state.lock().await.packages, &crawl.answered);
    let removals: Vec<(String, String)> = undeclared
        .iter()
        .map(|p| (p.backend.clone(), p.name.clone()))
        .collect();

    // The whole list. 576 packages is 576 lines: the pain is the feature, and a summary
    // here is a summary of what you are about to lose.
    println!(
        "Shall manages {} package(s). This will remove {}:\n",
        managed,
        undeclared.len()
    );
    for p in &undeclared {
        println!("  {}:{}", p.backend, p.name);
    }
    println!();

    // The ratio check, before anything else asks anything.
    if reads_as_a_mistake(&app.config, managed, undeclared.len()) && !allow_mass_purge {
        let sample: Vec<String> = undeclared.iter().take(3).map(|p| p.name.clone()).collect();
        return Err(crate::core::Error::Refused(format!(
            "Shall manages {} packages.\n\
             This will remove {}, including {}.\n\
             That looks like you haven't adopted this machine yet.\n\
             Run `shall adopt` first, or --allow-mass-purge if you're sure.",
            managed,
            undeclared.len(),
            sample.join(", ")
        ))
        .into());
    }

    // `max_removals` does not apply: it catches accidents, and this is deliberate. Protection
    // and OS-essential still do — nothing overrides those (II.10, II.11).
    //
    // Asked here so a refusal lands before the confirmation prompt, and asked again by the
    // engine that carries the removal out. Two asks, one rule: `SyncEngine::sync` dispatches
    // `GuardScope::PurgeUndeclared` to this same `enforce_deliberate`, so the second ask cannot
    // answer differently from the first.
    crate::app::sync::guard::enforce_deliberate(
        &app.config,
        &app.registry,
        &removals,
        &app.reaping,
        crate::app::sync::guard::GuardScope::PurgeUndeclared,
    )
    .await?;

    if app.config.dry_run {
        crate::would_print!("Nothing removed.");
        return Ok(());
    }

    // Snapshots first, automatically. If none can be taken, say so — "there is no undo for
    // this" is the most important sentence this command can print (II.11).
    let snapshot = match app
        .snapshot_manager
        .auto_snapshot(crate::core::snapshot::SnapshotLabel::PurgeUndeclared)
        .await
    {
        Ok(Some(snap)) => {
            println!("Snapshot taken: {}. That is your undo.\n", snap.id);
            Some(snap.id)
        }
        Ok(None) => {
            println!(
                "This cannot be undone.\n  \
                 This machine has no snapshot provider (btrfs, ZFS or Timeshift), so nothing \
                 removed here can be brought back.\n"
            );
            None
        }
        Err(e) => {
            println!(
                "This cannot be undone.\n  \
                 The snapshot failed ({}), so nothing removed here can be brought back.\n",
                e
            );
            None
        }
    };

    if !app.config.yes {
        use std::io::IsTerminal;
        // The most destructive command in the program, and the only prompt of the eight that
        // could not say why it stopped. dialoguer answers a closed stdin with `IO error: not a
        // terminal`, so it did fail safe — and a scripted user got that sentence instead of
        // the one naming the flag that would have worked.
        if !std::io::stdin().is_terminal() {
            return Err(crate::core::Error::Refused(
                "Refusing to purge undeclared packages without confirmation in a \
                 non-interactive shell. Re-run with --yes to proceed, or --dry-run to preview."
                    .to_string(),
            )
            .into());
        }
        // Waiting on a person, not on work: `block_in_place` so the runtime's other tasks move
        // off this worker rather than queueing behind someone reading a number off the screen.
        let typed: String = crate::core::on_the_terminal(|| {
            dialoguer::Input::new()
                .with_prompt(format!(
                    "Type the number of packages to remove ({}) to confirm",
                    undeclared.len()
                ))
                .allow_empty(true)
                .interact_text()
        })?;
        if typed.trim() != undeclared.len().to_string() {
            println!("Aborted. Nothing was removed.");
            return Ok(());
        }
    }

    // **Executed by the one engine.** This was a `for` loop calling `inst.remove` one package
    // at a time — no transaction, no batching, no rollback — in the most destructive command in
    // the program. `plan.rs-499` is the best paragraph in `src/verbs/` on exactly what that
    // shape cost `apply`, and nobody applied it here.
    //
    // The guard's scope carries `II.11` with it: `SyncEngine::sync` dispatches
    // `GuardScope::PurgeUndeclared` to `enforce_deliberate`, so the count check stays off for
    // this command while `protected_packages` and OS-essential stay on. That ruling now lives in
    // one place instead of being the reason this loop could not use the engine.
    let planned = removals.len();
    let (gone, failed) = match execute_removals_through_the_engine(
        &app.sync_engine(),
        &removals,
        guard::GuardScope::PurgeUndeclared,
    )
    .await
    {
        Ok(()) => (planned, 0usize),
        Err(e) => {
            warn!("purge-undeclared: {}", e);
            (0usize, planned)
        }
    };

    println!("\nRemoved {} package(s); {} failed.", gone, failed);
    if let Some(id) = &snapshot {
        println!(
            "Snapshot {} was taken before this ran; `shall snapshot restore` opens the gallery \
             to put the filesystem back.",
            id
        );
    }
    Ok(())
}

/// `shall reset` — Shall forgets it manages anything (X.3, level 3). The packages stay; the
/// registry and snapshots go.
///
/// This is not a widening of `clean-cache`. Level 3 is a different command precisely because
/// losing the registry loses the one distinction the removal model rests on — declared vs
/// already-there — and after it every managed package looks unmanaged.
pub async fn handle_reset(
    config: &Config,
    state: &tokio::sync::Mutex<crate::core::StateRegistry>,
    force: bool,
) -> Result<()> {
    let managed = state.lock().await.packages.len();

    // K5: forgetting the registry while the declarations remain leaves Shall believing it
    // manages nothing and the files saying otherwise. Refuse unless the repo is gone, or the
    // user says `--force`.
    let config_root = config.config_root();
    let repo_exists = config_root.join("modules").exists()
        || config_root.join("profiles").exists()
        || config_root.join("active").exists();
    if repo_exists && !force {
        return Err(crate::core::Error::Refused(format!(
            "A config repo still exists at {}.\n\
             Resetting the registry while your files declare packages would leave Shall \
             believing it manages nothing while the files say otherwise.\n\
             Delete the repo first, or pass --force if you mean to keep the files and forget \
             the registry anyway.",
            config_root.display()
        ))
        .into());
    }

    println!(
        "Shall will forget it manages {} package(s). They stay installed.\n\
         `shall adopt` is how you get them back, and it will guess.\n\
         The registry and all snapshots are deleted. This cannot be undone.\n",
        managed
    );

    if !config.yes {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            return Err(crate::core::Error::Refused(
                "Refusing to reset without confirmation in a non-interactive shell. Re-run \
                 with --yes if you are certain."
                    .to_string(),
            )
            .into());
        }
        let typed: String = crate::core::on_the_terminal(|| {
            dialoguer::Input::new()
                .with_prompt(format!(
                    "Type the number of packages to forget ({}) to confirm",
                    managed
                ))
                .allow_empty(true)
                .interact_text()
        })?;
        if typed.trim() != managed.to_string() {
            println!("Aborted. Nothing was forgotten.");
            return Ok(());
        }
    }

    let layout = config.layout();
    let registry = layout.registry_file();
    let snapshots = layout.snapshots_dir();

    let mut removed = Vec::new();
    if registry.exists() {
        tokio::fs::remove_file(&registry)
            .await
            .with_context(|| format!("could not delete {}", registry.display()))?;
        removed.push(registry.display().to_string());
    }
    if snapshots.exists() {
        tokio::fs::remove_dir_all(&snapshots)
            .await
            .with_context(|| format!("could not delete {}", snapshots.display()))?;
        removed.push(snapshots.display().to_string());
    }

    if removed.is_empty() {
        println!("Nothing to reset: no registry or snapshots were on disk.");
    } else {
        println!("Reset. Deleted:");
        for r in &removed {
            println!("  {}", r);
        }
    }
    Ok(())
}

/// Stop managing packages without uninstalling them.
///
/// This exists because deleting a manifest line means "uninstall this", not "stop managing
/// this" — so the obvious way to trim `adopt`'s output (keep 15 lines, delete 85) is in
/// fact an order to purge 85 packages. Forgetting has to be its own verb.
///
/// It drops the package from managed state AND from any manifest that declares it. Doing
/// only the first would be undone by the next `sync`, which would see the declaration and
/// re-adopt it.
pub async fn handle_unmanage(app: &App, packages: &[String], out: Output) -> Result<()> {
    // Q9: `unmanage nosuchbackend:foo` answered "not managed and not declared — nothing to
    // forget" at exit 0, which is what a correctly-spelled name that is genuinely unmanaged
    // also gets. `split_removal_target` below asks the registry about the prefix and falls back
    // to treating the whole string as a name, so a typo reads as a package nobody manages.
    app.resolver()
        .await
        .require_known_spec_backends(packages)
        .await?;
    let mut results = Vec::new();

    for spec in packages {
        let (backend, name) =
            crate::config::parser::split_removal_target(spec, |b| app.registry.get(b).is_some());

        // Forget every backend's copy when the target is unqualified, mirroring how
        // `remove` searches all backends for a bare name.
        let mut forgotten = Vec::new();
        {
            let mut state = app.state.lock().await;
            let managed: Vec<(String, String)> = state
                .packages
                .iter()
                .filter(|p| p.name == name)
                .filter(|p| backend.as_deref().is_none_or(|b| b == p.backend))
                .map(|p| (p.backend.clone(), p.name.clone()))
                .collect();
            for (b, n) in managed {
                if state.remove(&b, &n) {
                    forgotten.push(format!("{}:{}", b, n));
                }
            }
        }

        // The line goes too, and it is what makes the forgetting stick: ownership is read from
        // what this machine declares (II.56), so a package still declared is a package the next
        // `sync` re-adopts — a command that silently undoes itself.
        //
        // Under `--dry-run` this reports the lines and writes none of them: the editor is in
        // `Writes::Planned`, and the `forget` above stays in memory because the save below is
        // skipped.
        let dropped = app.declarations().undeclare(spec).await?;

        results.push(serde_json::json!({
            "package": spec,
            "forgotten": forgotten,
            "lines_removed": dropped
                .iter()
                .map(|e| serde_json::json!({
                    "file": e.file.display().to_string(),
                    "line": e.line,
                }))
                .collect::<Vec<_>>(),
            "still_installed": true,
        }));
    }

    // The registry is what Shall believes it manages. A preview that persisted `forget` would
    // leave the package unmanaged for real while promising it had changed nothing.
    if !app.config.dry_run {
        crate::core::save_off_the_runtime(&app.state).await?;
    }

    if out.is_json() {
        println!("{}", serde_json::to_string_pretty(&results)?);
        return Ok(());
    }

    if app.config.dry_run {
        crate::would_print!("would stop managing:");
    }

    for r in &results {
        let spec = r["package"].as_str().unwrap_or_default();
        let forgotten = r["forgotten"].as_array().map(|a| a.len()).unwrap_or(0);
        let lines = r["lines_removed"].as_array().map(|a| a.len()).unwrap_or(0);
        if forgotten == 0 && lines == 0 {
            println!(
                "{}: not managed and not declared — nothing to forget.",
                spec
            );
            continue;
        }
        println!(
            "{}: no longer managed by Shall. It is still installed.",
            spec
        );
        for f in r["forgotten"].as_array().into_iter().flatten() {
            println!("  dropped from managed state: {}", f.as_str().unwrap_or(""));
        }
        for l in r["lines_removed"].as_array().into_iter().flatten() {
            println!(
                "  removed declaration `{}` from {}",
                l["line"].as_str().unwrap_or(""),
                l["file"].as_str().unwrap_or("")
            );
        }
    }
    Ok(())
}

/// Show what the removal guard will refuse to touch. The guard is only trustworthy if its
/// rules are inspectable, so this reports the effective rules — and, given package names,
/// answers the question people actually have ("will this be protected?") along with the
/// rule that decides it.
pub async fn handle_protected(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    resolver: &StateResolver<'_>,
    packages: &[String],
    out: Output,
) -> Result<()> {
    let cfg = config;

    if !packages.is_empty() {
        // Same refusal every other spec-taking verb gives (N-3): a `nosuchbackend:` prefix is a
        // typo, and answering it as though it were a package name called `nosuchbackend:foo`
        // is the silence that family was closed to end. This verb was missed because the gate
        // deriving that family from `--help` exempted it as taking "nothing".
        resolver.require_known_spec_backends(packages).await?;

        // Query mode. This MUST reach the same answer as a real removal, so it calls the
        // guard's own decision function rather than re-implementing the rules — an
        // inspector that contradicts the enforcer is worse than none, because it is
        // believed. "backend:name" consults that backend's essential list; a bare name is
        // checked against the config rules only, and says so, because the OS's list is keyed
        // by backend and there is no honest way to answer it from a name alone.
        // The OS's essential set does not change partway through one command, and it costs a
        // subprocess per backend to fetch. This asked for it *inside* the per-package loop, so
        // checking 40 packages ran the whole per-backend essential query 40 times over for the
        // same answer. Asked once, for every backend the request names.
        let named_backends: std::collections::HashSet<String> = packages
            .iter()
            .filter_map(|spec| {
                crate::config::parser::split_removal_target(spec, |b| registry.get(b).is_some()).0
            })
            .collect();
        let all_essential = crate::app::sync::guard::essential_names(
            registry,
            &named_backends,
            config.max_parallel,
        )
        .await;

        let mut rows = Vec::new();
        for spec in packages {
            let (backend, name) =
                crate::config::parser::split_removal_target(spec, |b| registry.get(b).is_some());
            // A bare name is checked against the config rules only: the OS's list is keyed by
            // backend and there is no honest way to answer it from a name alone.
            let os_essential = match &backend {
                Some(b) => all_essential
                    .iter()
                    .filter(|k| k.split_once(':').is_some_and(|(kb, _)| kb == b))
                    .cloned()
                    .collect(),
                None => std::collections::HashSet::new(),
            };
            let (protected, reason) = match crate::app::sync::guard::protection_of(
                cfg,
                backend.as_deref(),
                &name,
                &os_essential,
            ) {
                Some(p) => (true, p.reason()),
                None => match cfg.unprotect_rule(&name) {
                    Some(rule) => (
                        false,
                        format!("exempted by unprotected_packages rule `{}`", rule),
                    ),
                    None => (
                        false,
                        match &backend {
                            Some(_) => "no rule matches".to_string(),
                            None => format!(
                                "no config rule matches (no backend named, so this machine's \
                                 essential list was not consulted — ask `<backend>:{}` for that)",
                                name
                            ),
                        },
                    ),
                },
            };
            rows.push((spec.clone(), protected, reason));
        }
        if out.is_json() {
            let out: Vec<_> = rows
                .iter()
                .map(|(p, prot, why)| {
                    serde_json::json!({ "package": p, "protected": prot, "reason": why })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            println!("{:<30} {:<10} REASON", "PACKAGE", "PROTECTED");
            for (p, prot, why) in rows {
                println!("{:<30} {:<10} {}", p, if prot { "yes" } else { "no" }, why);
            }
        }
        return Ok(());
    }

    if out.is_json() {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "protected_packages": cfg.guard.protected_packages,
                "unprotected_packages": cfg.guard.unprotected_packages,
                // Every ceiling, not the one this command was written before the others
                // existed: a consumer asking "what will this machine refuse" got a third of
                // the answer and no way to tell it was a third.
                "max_removals": cfg.guard.max_removals,
                "max_extra_removals": cfg.guard.max_extra_removals,
                "max_port_closures": cfg.guard.max_port_closures,
                "max_installs": cfg.guard.max_installs,
                "max_total_changes": cfg.guard.max_total_changes,
                // Not a ceiling — a proportion — and it refuses removals all the same. It was
                // omitted here on the day it stopped being a private constant and became a
                // setting, which is the same third-of-an-answer the comment above describes.
                "purge_ratio": cfg.guard.purge_ratio,
            }))?
        );
        return Ok(());
    }

    println!("Removal guard — what Shall refuses to remove.\n");
    println!(
        "Protected packages ({}):",
        cfg.guard.protected_packages.len()
    );
    for p in &cfg.guard.protected_packages {
        match p.strip_suffix('*') {
            Some(prefix) => println!("  {:<24} (any name starting with '{}')", p, prefix),
            None => println!("  {}", p),
        }
    }
    if cfg.guard.unprotected_packages.is_empty() {
        println!("\nExemptions: none.");
    } else {
        println!(
            "\nExemptions ({}) — these override the list above:",
            cfg.guard.unprotected_packages.len()
        );
        for p in &cfg.guard.unprotected_packages {
            println!("  {}", p);
        }
    }
    // Every ceiling, in one column, because a user reading this to answer "what stops a big
    // change" needs the one that will stop theirs — and which one that is depends on what they
    // are changing.
    println!("\nCeilings for one command:");
    for (key, value) in [
        ("max_removals", cfg.guard.max_removals),
        ("max_extra_removals", cfg.guard.max_extra_removals),
        ("max_port_closures", cfg.guard.max_port_closures),
        ("max_installs", cfg.guard.max_installs),
        ("max_total_changes", cfg.guard.max_total_changes),
    ] {
        match value {
            0 => println!("  {:<20} unlimited (0)", key),
            n => println!("  {:<20} {}", key, n),
        }
    }
    // Separate from the ceilings because it is not one: it compares a sweep against what Shall
    // manages rather than against a fixed number, which is the case a count cannot catch — on a
    // machine managing fourteen packages, removing all fourteen is under every ceiling above.
    match cfg.guard.purge_ratio {
        r if r <= 0.0 => println!("\nProportion rule:\n  purge_ratio          off (0)"),
        r => println!(
            "\nProportion rule:\n  purge_ratio          {r} — refuses a sweep removing more than \
             {:.0}x what Shall manages",
            1.0 / r
        ),
    }

    println!(
        "\nPackages the OS itself reports as essential are also refused, on top of this list.\n\
         Every command that removes is guarded — there is no way to opt one out.\n\
         Edit `protected_packages`, `unprotected_packages` or any ceiling under [guard] in {}.\n\
         Check one package:      shall protected apt:python3\n\
         Machine-readable:       shall protected --json\n\
         Allow a big removal:    shall <command> --allow-mass-removal (the count only —\n\
                                 it never lets a protected or essential package through)\n\
         Allow a big install:    shall <command> --allow-mass-install (answers `max_installs`,\n\
                                 off unless you set it)\n\
         Either flag answers `max_total_changes`; neither answers a protected name.",
        cfg.preferences_file.display()
    );
    Ok(())
}

#[cfg(test)]
mod purge_tests {
    use super::managed_where_the_crawl_could_see;
    use crate::config::Config;
    use crate::core::state::ManagedPackage;

    /// The rule at its shipped threshold.
    fn reads_as_a_mistake(managed: usize, to_remove: usize) -> bool {
        super::reads_as_a_mistake(&Config::default(), managed, to_remove)
    }

    fn managed_pkg(backend: &str, name: &str) -> ManagedPackage {
        ManagedPackage {
            name: name.into(),
            backend: backend.into(),
            version: None,
            installed_at: 0,
            expires_at: None,
            options: Default::default(),
            source: "test".into(),
            is_transient: false,
            session_id: None,
        }
    }

    #[test]
    fn manage_three_delete_576_is_a_mistake_at_any_scale() {
        // II.11's example, and V.20's rule: a count cannot catch this on a small machine.
        assert!(reads_as_a_mistake(3, 576));
    }

    #[test]
    fn the_ratio_catches_the_small_machine_a_count_misses() {
        // Alpine: adopt correctly took 14 packages, and a mis-scoped removal scheduled all
        // 14 — under any count limit, none protected, all things you would cry about.
        assert!(reads_as_a_mistake(1, 14));
        // And an adopted Alpine is fine: 14 managed, a handful of strays to clear.
        assert!(!reads_as_a_mistake(14, 20));
    }

    #[test]
    fn an_adopted_machine_may_purge_the_rest() {
        // Ubuntu after `adopt`: ~103 manual packages managed, the dependency closure and
        // whatever else is lying around unmanaged. That is the command working as intended.
        assert!(!reads_as_a_mistake(103, 476));
    }

    // ---- The two sides of the ratio count the same managers. ------------------------------

    /// A manager the crawl never surveyed contributes nothing to the count it is weighed
    /// against.
    ///
    /// `installed_but_undeclared` asks `priority`'s managers only, so a package managed
    /// through one `priority` omits can never appear on the deletion side. Counting it on the
    /// management side is how 43-against-276 cleared a bar that 12-against-276 would not
    /// have.
    #[test]
    fn a_manager_the_crawl_never_asked_is_on_neither_side() {
        let state = vec![
            managed_pkg("brew", "ripgrep"),
            managed_pkg("brew", "fd"),
            managed_pkg("gem", "rails"),
            managed_pkg("npm", "typescript"),
        ];
        // Only brew answered — `priority` does not name gem or npm, or they failed to list.
        let answered = vec!["brew".to_string()];
        assert_eq!(
            managed_where_the_crawl_could_see(&state, &answered),
            2,
            "only the two brew packages are comparable with a brew-only deletion list"
        );
    }

    /// The whole state counts when the whole state was surveyed — the control that makes the
    /// test above a measurement rather than an artefact of filtering.
    #[test]
    fn every_manager_that_answered_counts() {
        let state = vec![
            managed_pkg("brew", "ripgrep"),
            managed_pkg("gem", "rails"),
            managed_pkg("npm", "typescript"),
        ];
        let answered = vec!["brew".to_string(), "gem".to_string(), "npm".to_string()];
        assert_eq!(managed_where_the_crawl_could_see(&state, &answered), 3);
    }

    /// The regression, end to end and in its own numbers.
    ///
    /// macOS nightly, 2026-08-14: 43 packages in the state file, 276 undeclared found through
    /// the managers `priority` names. Scoped, the management side is what those same managers
    /// account for — and the refusal comes back.
    #[test]
    fn the_macos_nightly_refuses_once_both_sides_count_the_same_machine() {
        // The unscoped count is what shipped, and it cleared the bar.
        assert!(
            !reads_as_a_mistake(43, 276),
            "43/276 is 0.156 — this is the number that let the purge through"
        );
        // Scoped to the managers that answered, the same machine reads as a mistake. Any
        // numerator below 27.6 does; the point is that dropping the managers which cannot
        // appear on the deletion side can only move it down.
        assert!(reads_as_a_mistake(12, 276));
    }

    /// The threshold is the owner's to move, in both directions.
    ///
    /// **`0.0` is off, and that is a deliberate escape hatch rather than an accident of the
    /// arithmetic** — `managed / to_remove < 0.0` is false for every non-negative input, so a
    /// bare comparison would have disabled the rule at zero by luck. It is written down and
    /// tested because someone purging a machine on purpose, repeatedly, should be able to say
    /// so once in a file instead of remembering a flag every time.
    #[test]
    fn the_purge_ratio_is_a_setting_and_zero_turns_it_off() {
        let strict = Config {
            guard: crate::config::GuardSettings {
                purge_ratio: 0.5,
                ..Default::default()
            },
            ..Default::default()
        };
        // Passes at the shipped 0.1, refused once the owner asks for half.
        assert!(!reads_as_a_mistake(43, 276));
        assert!(super::reads_as_a_mistake(&strict, 43, 276));

        let off = Config {
            guard: crate::config::GuardSettings {
                purge_ratio: 0.0,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(
            !super::reads_as_a_mistake(&off, 0, 576),
            "zero is off — managing nothing and deleting everything is allowed on request"
        );
    }

    /// The platform lists name packages the managers actually report.
    ///
    /// They used to read `kernel32`, `ntdll.dll`, `win32`, `xnu` — the OS's vocabulary, not any
    /// manager's, so nothing ever matched them and outside Linux the protected set was three
    /// names. A default list that matches nothing is indistinguishable from no list, and this
    /// is what tells the two apart.
    #[test]
    fn the_protected_defaults_match_names_a_manager_would_report() {
        let cfg = Config::default();
        let protected = |name: &str| cfg.protection_rule(name).is_some();

        // Shared, on every platform.
        assert!(protected("sudo") && protected("bash") && protected("shall"));

        #[cfg(target_os = "windows")]
        {
            // Exactly as `winget list` and `choco list` print them.
            assert!(protected("Microsoft.VCRedist.2015+.x64"));
            assert!(protected("vcredist140"));
            assert!(protected("Microsoft.DotNet.DesktopRuntime.8"));
            assert!(protected("Git.Git"));
            // And a name nobody should be stopped from managing.
            assert!(!protected("GitHub.cli"));
            assert!(!protected("dotPDN.PaintDotNet"));
        }
        #[cfg(target_os = "macos")]
        {
            assert!(protected("ca-certificates"));
            assert!(protected("openssl@3"), "the family, under its versions");
            // brew's git and curl are a preference, not a dependency: macOS ships its own in
            // /usr/bin, so protecting them would be friction bought with no safety.
            assert!(!protected("git") && !protected("curl"));
        }
    }

    /// A crawl that answered for nothing cannot license a deletion.
    ///
    /// Zero managed over any deletion list is a ratio of zero, which is below any threshold —
    /// asserted rather than assumed, because `0.0 / n` is the one input where a float
    /// comparison could plausibly have been written the other way round.
    #[test]
    fn managing_nothing_is_always_a_mistake() {
        assert!(reads_as_a_mistake(0, 1));
        assert!(reads_as_a_mistake(0, 576));
        assert_eq!(managed_where_the_crawl_could_see(&[], &[]), 0);
    }
}
