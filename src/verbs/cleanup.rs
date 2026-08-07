use crate::verbs::prelude::*;

/// `remove-orphans` — each manager's own "no longer needed by anything" set.
///
/// The orphan set is the backend's opinion, not LiNix's model, which is exactly why it gets
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
        futures::stream::iter(app.registry.available())
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
             LiNix does not let them delete it.",
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
    let reaped = enforce(
        &app.config,
        &app.registry,
        &removals,
        GuardScope::RemoveOrphans,
    )
    .await?;

    if app.config.dry_run {
        println!(
            "
[DRY-RUN] Nothing was removed."
        );
        return Ok(());
    }

    if !confirm_orphan_removal(app)? {
        println!("Nothing removed.");
        return Ok(());
    }

    for (backend_name, names) in &listed {
        // II.7c. Unreachable in practice — `listed` was built by asking the backends
        // themselves, so one that is not here contributed no names — but written as a reported
        // skip rather than a bare `continue` because that is the shape the rule takes
        // everywhere else, and a defensive branch that stays silent is how the reachable ones
        // came to be silent too.
        let Some(backend) = app.registry.get(backend_name).filter(|b| b.is_available()) else {
            warn!(
                "`{}` is not on this machine, so its orphans were left alone.",
                backend_name
            );
            continue;
        };
        if let Some(installable) = backend.as_installable() {
            // Remove exactly the names that were shown and guarded — not the backend's own
            // autoremove, whose set can have moved since the preview.
            //
            // One journal entry per name, not one per command: they go in one manager
            // invocation and so succeed or fail together, but a reader of an interrupted log
            // needs to know *which* packages a killed `remove-orphans` was part-way through.
            crate::core::journalled(
                &app.journal,
                crate::core::journal::removals_of(backend_name, names),
                installable.remove(names, backend.sudo_for_write(), reaped),
            )
            .await
            .with_context(|| format!("removing orphans from {}", backend_name))?;
            println!("  {}: removed {} package(s)", backend_name, names.len());
        }
    }

    perform_maintenance(app).await
}

pub fn confirm_orphan_removal(app: &App) -> Result<bool> {
    if app.config.yes {
        return Ok(true);
    }
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        return Err(crate::core::Error::Refused(
            "Refusing to remove orphans without confirmation in a non-interactive shell. Re-run with --yes to proceed, or --dry-run to preview."
        .to_string()).into());
    }
    Ok(dialoguer::Confirm::new()
        .with_prompt("Remove these packages?")
        .default(false)
        .interact()?)
}

/// `clean-cache` — downloaded archives and build caches (X.3 levels 1–2). Removes no installed
/// package, so it needs no preview and no guard: the guard protects packages, not disk space,
/// and widening it to cover caches would dilute what a refusal means (K16).
///
/// `--all` additionally clears LiNix's own transient download area. It does NOT touch the
/// installed artifact directories — those hold software that is on `PATH`, and deleting them
/// is a removal (level 4), not a cache clean.
pub async fn handle_clean_cache(app: &App, all: bool) -> Result<()> {
    if app.config.dry_run {
        println!("[DRY-RUN] Would clear the package cache for every backend that has one.");
        println!("[DRY-RUN] Would forget the installed listings LiNix has cached.");
        if all {
            println!("[DRY-RUN] Would also clear LiNix's own download cache.");
        }
        return Ok(());
    }

    // The listings go first, and unconditionally. This is the command a user reaches for when
    // something outside LiNix changed the machine and they know it before `installed_cache_secs`
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
    let cleanable: Vec<(String, bool, std::sync::Arc<dyn crate::core::Upgradable>)> = app
        .registry
        .available()
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
                    println!("Cleared LiNix's download cache ({}).", tmp.display());
                }
                Err(e) => warn!("could not clear {}: {}", tmp.display(), e),
            }
        } else {
            println!("LiNix's download cache is already empty.");
        }
    }

    perform_maintenance(app).await
}

/// How little LiNix must manage before "delete the rest" reads as a mistake (II.11).
///
/// A ratio, not a count. On Alpine, `adopt` correctly took 14 packages and a mis-scoped
/// removal scheduled all 14 — under any sane count limit, none protected, all things you
/// would cry about. The count misses it on small machines. Manage a tenth of what you are
/// about to delete and you have made a mistake, on every machine, at every scale (V.20).
pub const PURGE_RATIO: f64 = 0.1;

/// `purge-undeclared` (II.11): delete everything LiNix does not manage.
///
/// The residual risk, stated plainly because the docs must state it: `adopt` is an estimate.
/// If it missed something, this deletes it.
pub async fn handle_purge_undeclared(app: &App, allow_mass_purge: bool) -> Result<()> {
    let undeclared = app.installed_but_undeclared().await?;
    if undeclared.is_empty() {
        println!("Nothing to do: LiNix manages every installed package.");
        return Ok(());
    }

    let managed = app.state.lock().await.packages.len();
    let removals: Vec<(String, String)> = undeclared
        .iter()
        .map(|p| (p.backend.clone(), p.name.clone()))
        .collect();

    // The whole list. 576 packages is 576 lines: the pain is the feature, and a summary
    // here is a summary of what you are about to lose.
    println!(
        "LiNix manages {} package(s). This will remove {}:\n",
        managed,
        undeclared.len()
    );
    for p in &undeclared {
        println!("  {}:{}", p.backend, p.name);
    }
    println!();

    // The ratio check, before anything else asks anything.
    let ratio = managed as f64 / undeclared.len() as f64;
    if ratio < PURGE_RATIO && !allow_mass_purge {
        let sample: Vec<String> = undeclared.iter().take(3).map(|p| p.name.clone()).collect();
        return Err(crate::core::Error::Refused(format!(
            "LiNix manages {} packages.\n\
             This will remove {}, including {}.\n\
             That looks like you haven't adopted this machine yet.\n\
             Run `linix adopt` first, or --allow-mass-purge if you're sure.",
            managed,
            undeclared.len(),
            sample.join(", ")
        ))
        .into());
    }

    // `max_removals` does not apply: it catches accidents, and this is deliberate. Protection
    // and OS-essential still do — nothing overrides those (II.10, II.11).
    let reaped = crate::app::sync::guard::enforce_deliberate(
        &app.config,
        &app.registry,
        &removals,
        crate::app::sync::guard::GuardScope::PurgeUndeclared,
    )
    .await?;

    if app.config.dry_run {
        println!("[DRY-RUN] Nothing removed.");
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
        let typed: String = dialoguer::Input::new()
            .with_prompt(format!(
                "Type the number of packages to remove ({}) to confirm",
                undeclared.len()
            ))
            .allow_empty(true)
            .interact_text()?;
        if typed.trim() != undeclared.len().to_string() {
            println!("Aborted. Nothing was removed.");
            return Ok(());
        }
    }

    let (mut gone, mut failed) = (0usize, 0usize);
    for (backend_name, name) in &removals {
        // II.7c, and this one is reachable: `removals` comes from what LiNix has recorded, so
        // a package whose manager has since been uninstalled is exactly the case. It was a
        // bare `continue`, which left `purge-undeclared` printing a count that did not
        // include it and no line saying why.
        let Some(b) = app.registry.get(backend_name).filter(|b| b.is_available()) else {
            warn!(
                "`{}` is not on this machine, so {}:{} was left alone.",
                backend_name, backend_name, name
            );
            continue;
        };
        let Some(inst) = b.as_installable() else {
            continue;
        };
        // The most destructive command in the program, and until now the one with no record
        // that it had started. Journalled per package, because this loop removes one at a
        // time and a kill part-way through leaves a machine whose only account of what went
        // is the terminal scrollback.
        match crate::core::journalled(
            &app.journal,
            vec![crate::core::JournalAction::Remove {
                name: name.clone(),
                backend: backend_name.clone(),
            }],
            inst.remove(std::slice::from_ref(name), b.sudo_for_write(), reaped),
        )
        .await
        {
            Ok(_) => gone += 1,
            Err(e) => {
                failed += 1;
                warn!("purge-undeclared: {}:{} — {}", backend_name, name, e);
            }
        }
    }

    println!("\nRemoved {} package(s); {} failed.", gone, failed);
    if let Some(id) = &snapshot {
        println!(
            "Snapshot {} was taken before this ran; `linix snapshot restore` opens the gallery \
             to put the filesystem back.",
            id
        );
    }
    Ok(())
}

/// `linix reset` — LiNix forgets it manages anything (X.3, level 3). The packages stay; the
/// registry and snapshots go.
///
/// This is not a widening of `clean-cache`. Level 3 is a different command precisely because
/// losing the registry loses the one distinction the removal model rests on — declared vs
/// already-there — and after it every managed package looks unmanaged.
pub async fn handle_reset(app: &App, force: bool) -> Result<()> {
    let managed = app.state.lock().await.packages.len();

    // K5: forgetting the registry while the declarations remain leaves LiNix believing it
    // manages nothing and the files saying otherwise. Refuse unless the repo is gone, or the
    // user says `--force`.
    let config_root = app.config.config_root();
    let repo_exists = config_root.join("modules").exists()
        || config_root.join("profiles").exists()
        || config_root.join("active").exists();
    if repo_exists && !force {
        return Err(crate::core::Error::Refused(format!(
            "A config repo still exists at {}.\n\
             Resetting the registry while your files declare packages would leave LiNix \
             believing it manages nothing while the files say otherwise.\n\
             Delete the repo first, or pass --force if you mean to keep the files and forget \
             the registry anyway.",
            config_root.display()
        ))
        .into());
    }

    println!(
        "LiNix will forget it manages {} package(s). They stay installed.\n\
         `linix adopt` is how you get them back, and it will guess.\n\
         The registry and all snapshots are deleted. This cannot be undone.\n",
        managed
    );

    if !app.config.yes {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            return Err(crate::core::Error::Refused(
                "Refusing to reset without confirmation in a non-interactive shell. Re-run \
                 with --yes if you are certain."
                    .to_string(),
            )
            .into());
        }
        let typed: String = dialoguer::Input::new()
            .with_prompt(format!(
                "Type the number of packages to forget ({}) to confirm",
                managed
            ))
            .allow_empty(true)
            .interact_text()?;
        if typed.trim() != managed.to_string() {
            println!("Aborted. Nothing was forgotten.");
            return Ok(());
        }
    }

    let layout = app.config.layout();
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
pub async fn handle_unmanage(app: &App, packages: &[String], json: bool) -> Result<()> {
    // Q9: `unmanage nosuchbackend:foo` answered "not managed and not declared — nothing to
    // forget" at exit 0, which is what a correctly-spelled name that is genuinely unmanaged
    // also gets. `split_removal_target` below asks the registry about the prefix and falls back
    // to treating the whole string as a name, so a typo reads as a package nobody manages.
    app.require_known_spec_backends(packages).await?;
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

        // The line goes too. `forget` means LiNix never touches it again, and a package
        // still declared is a package the next `sync` re-adopts — a command that silently
        // undoes itself.
        //
        // Under `--dry-run` this reports the lines and writes none of them: the editor is in
        // `Writes::Planned`, and the `forget` above stays in memory because the save below is
        // skipped.
        let dropped = app.undeclare(spec).await?;

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

    // The registry is what LiNix believes it manages. A preview that persisted `forget` would
    // leave the package unmanaged for real while promising it had changed nothing.
    if !app.config.dry_run {
        app.state.lock().await.save()?;
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&results)?);
        return Ok(());
    }

    if app.config.dry_run {
        println!("[DRY-RUN] would stop managing:");
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
            "{}: no longer managed by LiNix. It is still installed.",
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
pub async fn handle_protected(app: &App, packages: &[String], json: bool) -> Result<()> {
    let cfg = &app.config;

    if !packages.is_empty() {
        // Same refusal every other spec-taking verb gives (N-3): a `nosuchbackend:` prefix is a
        // typo, and answering it as though it were a package name called `nosuchbackend:foo`
        // is the silence that family was closed to end. This verb was missed because the gate
        // deriving that family from `--help` exempted it as taking "nothing".
        app.require_known_spec_backends(packages).await?;

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
                crate::config::parser::split_removal_target(spec, |b| app.registry.get(b).is_some())
                    .0
            })
            .collect();
        let all_essential = crate::app::sync::guard::essential_names(
            &app.registry,
            &named_backends,
            app.config.max_parallel,
        )
        .await;

        let mut rows = Vec::new();
        for spec in packages {
            let (backend, name) = crate::config::parser::split_removal_target(spec, |b| {
                app.registry.get(b).is_some()
            });
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
        if json {
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

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "protected_packages": cfg.guard.protected_packages,
                "unprotected_packages": cfg.guard.unprotected_packages,
                "max_removals": cfg.guard.max_removals,
            }))?
        );
        return Ok(());
    }

    println!("Removal guard — what LiNix refuses to remove.\n");
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
    match cfg.guard.max_removals {
        0 => println!("\nMaximum removals in one command: unlimited (max_removals = 0)."),
        n => println!("\nMaximum removals in one command: {} (max_removals).", n),
    }

    println!(
        "\nPackages the OS itself reports as essential are also refused, on top of this list.\n\
         Every command that removes is guarded — there is no way to opt one out.\n\
         Edit `protected_packages`, `unprotected_packages` or `max_removals` under [guard] in {}.\n\
         Check one package:      linix protected apt:python3\n\
         Machine-readable:       linix protected --json\n\
         Allow a big removal:    linix <command> --allow-mass-removal (the count only —\n\
                                 it never lets a protected or essential package through)\n\
         Allow a big install:    linix <command> --allow-mass-install (answers `max_installs`,\n\
                                 off unless you set it)",
        cfg.preferences_file.display()
    );
    Ok(())
}

#[cfg(test)]
mod purge_tests {
    /// The ratio, as `handle_purge_undeclared` computes it.
    fn reads_as_a_mistake(managed: usize, to_remove: usize) -> bool {
        (managed as f64 / to_remove as f64) < super::PURGE_RATIO
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
}
