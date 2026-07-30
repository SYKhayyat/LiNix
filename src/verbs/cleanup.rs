use crate::verbs::prelude::*;

/// `remove-orphans` — each manager's own "no longer needed by anything" set.
///
/// The orphan set is the backend's opinion, not LiNix's model, which is exactly why it gets
/// the same shape as `sync`: name every package first, put it through the guard, then ask.
/// The old `clean` ran `apt autoremove -y` / `pacman -Rs --noconfirm` across every available
/// backend with no preview and outside the guard.
pub(crate) async fn handle_remove_orphans(app: &App) -> Result<()> {
    use linix::app::sync::guard::{enforce, GuardScope};

    let mut listed: Vec<(String, Vec<String>)> = Vec::new();
    let mut cannot_say: Vec<String> = Vec::new();

    for backend in app.registry.available() {
        let up = match backend.as_upgradable() {
            Some(u) => u,
            None => continue,
        };
        match up.list_orphans().await {
            Ok(names) if names.is_empty() => {}
            Ok(names) => listed.push((backend.name().to_string(), names)),
            Err(linix::core::Error::Unsupported(_)) => cannot_say.push(backend.name().to_string()),
            Err(e) => warn!("could not list orphans for {}: {}", backend.name(), e),
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
    enforce(
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
        let backend = match app.registry.get(backend_name) {
            Some(b) => b,
            None => continue,
        };
        if let Some(installable) = backend.as_installable() {
            // Remove exactly the names that were shown and guarded — not the backend's own
            // autoremove, whose set can have moved since the preview.
            installable
                .remove(names, backend.sudo_for_write())
                .await
                .with_context(|| format!("removing orphans from {}", backend_name))?;
            println!("  {}: removed {} package(s)", backend_name, names.len());
        }
    }

    perform_maintenance(app).await
}

pub(crate) fn confirm_orphan_removal(app: &App) -> Result<bool> {
    if app.config.yes {
        return Ok(true);
    }
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        return Err(linix::core::Error::Refused(
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
pub(crate) async fn handle_clean_cache(app: &App, all: bool) -> Result<()> {
    if app.config.dry_run {
        println!("[DRY-RUN] Would clear the package cache for every backend that has one.");
        if all {
            println!("[DRY-RUN] Would also clear LiNix's own download cache.");
        }
        return Ok(());
    }
    let mut cleaned = Vec::new();
    for backend in app.registry.available() {
        let up = match backend.as_upgradable() {
            Some(u) => u,
            None => continue,
        };
        match up.clean_cache(backend.sudo_for_write()).await {
            Ok(()) => cleaned.push(backend.name().to_string()),
            Err(linix::core::Error::Unsupported(_)) => {}
            Err(e) => warn!("cache clean failed for {}: {}", backend.name(), e),
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
pub(crate) const PURGE_RATIO: f64 = 0.1;

/// `purge-unmanaged` (II.11): delete everything LiNix does not manage.
///
/// The residual risk, stated plainly because the docs must state it: `adopt` is an estimate.
/// If it missed something, this deletes it.
pub(crate) async fn handle_purge_unmanaged(app: &App, allow_mass_purge: bool) -> Result<()> {
    let unmanaged = app.installed_but_unmanaged().await?;
    if unmanaged.is_empty() {
        println!("Nothing to do: LiNix manages every installed package.");
        return Ok(());
    }

    let managed = app.state.lock().await.packages.len();
    let removals: Vec<(String, String)> = unmanaged
        .iter()
        .map(|p| (p.backend.clone(), p.name.clone()))
        .collect();

    // The whole list. 576 packages is 576 lines: the pain is the feature, and a summary
    // here is a summary of what you are about to lose.
    println!(
        "LiNix manages {} package(s). This will remove {}:\n",
        managed,
        unmanaged.len()
    );
    for p in &unmanaged {
        println!("  {}:{}", p.backend, p.name);
    }
    println!();

    // The ratio check, before anything else asks anything.
    let ratio = managed as f64 / unmanaged.len() as f64;
    if ratio < PURGE_RATIO && !allow_mass_purge {
        let sample: Vec<String> = unmanaged.iter().take(3).map(|p| p.name.clone()).collect();
        return Err(linix::core::Error::Refused(format!(
            "LiNix manages {} packages.\n\
             This will remove {}, including {}.\n\
             That looks like you haven't adopted this machine yet.\n\
             Run `linix adopt` first, or --allow-mass-purge if you're sure.",
            managed,
            unmanaged.len(),
            sample.join(", ")
        ))
        .into());
    }

    // `max_removals` does not apply: it catches accidents, and this is deliberate. Protection
    // and OS-essential still do — nothing overrides those (II.10, II.11).
    linix::app::sync::guard::enforce_deliberate(
        &app.config,
        &app.registry,
        &removals,
        linix::app::sync::guard::GuardScope::PurgeUnmanaged,
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
        .auto_snapshot(linix::core::snapshot::SnapshotLabel::PurgeUnmanaged)
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
            return Err(linix::core::Error::Refused(
                "Refusing to purge unmanaged packages without confirmation in a \
                 non-interactive shell. Re-run with --yes to proceed, or --dry-run to preview."
                    .to_string(),
            )
            .into());
        }
        let typed: String = dialoguer::Input::new()
            .with_prompt(format!(
                "Type the number of packages to remove ({}) to confirm",
                unmanaged.len()
            ))
            .allow_empty(true)
            .interact_text()?;
        if typed.trim() != unmanaged.len().to_string() {
            println!("Aborted. Nothing was removed.");
            return Ok(());
        }
    }

    let (mut gone, mut failed) = (0usize, 0usize);
    for (backend_name, name) in &removals {
        let Some(b) = app.registry.get(backend_name) else {
            continue;
        };
        let Some(inst) = b.as_installable() else {
            continue;
        };
        match inst
            .remove(std::slice::from_ref(name), b.sudo_for_write())
            .await
        {
            Ok(_) => gone += 1,
            Err(e) => {
                failed += 1;
                warn!("purge-unmanaged: {}:{} — {}", backend_name, name, e);
            }
        }
    }

    println!("\nRemoved {} package(s); {} failed.", gone, failed);
    if let Some(id) = &snapshot {
        println!("Undo with `linix undo {}`.", id);
    }
    Ok(())
}

/// `linix reset` — LiNix forgets it manages anything (X.3, level 3). The packages stay; the
/// registry and snapshots go.
///
/// This is not a widening of `clean-cache`. Level 3 is a different command precisely because
/// losing the registry loses the one distinction the removal model rests on — declared vs
/// already-there — and after it every managed package looks unmanaged.
pub(crate) async fn handle_reset(app: &App, force: bool) -> Result<()> {
    let managed = app.state.lock().await.packages.len();

    // K5: forgetting the registry while the declarations remain leaves LiNix believing it
    // manages nothing and the files saying otherwise. Refuse unless the repo is gone, or the
    // user says `--force`.
    let config_root = app.config.config_root();
    let repo_exists = config_root.join("modules").exists()
        || config_root.join("profiles").exists()
        || config_root.join("active").exists();
    if repo_exists && !force {
        return Err(linix::core::Error::Refused(format!(
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
            return Err(linix::core::Error::Refused(
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
pub(crate) async fn handle_unmanage(app: &App, packages: &[String], json: bool) -> Result<()> {
    // Q9: `unmanage nosuchbackend:foo` answered "not managed and not declared — nothing to
    // forget" at exit 0, which is what a correctly-spelled name that is genuinely unmanaged
    // also gets. `split_removal_target` below asks the registry about the prefix and falls back
    // to treating the whole string as a name, so a typo reads as a package nobody manages.
    app.require_known_spec_backends(packages).await?;
    let mut results = Vec::new();

    for spec in packages {
        let (backend, name) =
            linix::config::parser::split_removal_target(spec, |b| app.registry.get(b).is_some());

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
                if state.forget(&b, &n) {
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
pub(crate) async fn handle_protected(app: &App, packages: &[String], json: bool) -> Result<()> {
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
        let mut rows = Vec::new();
        for spec in packages {
            let (backend, name) = linix::config::parser::split_removal_target(spec, |b| {
                app.registry.get(b).is_some()
            });
            let os_essential = match &backend {
                Some(b) => {
                    let mut set = std::collections::HashSet::new();
                    set.insert(b.clone());
                    linix::app::sync::guard::essential_names(&app.registry, &set).await
                }
                None => std::collections::HashSet::new(),
            };
            let (protected, reason) = match linix::app::sync::guard::protection_of(
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
    /// The ratio, as `handle_purge_unmanaged` computes it.
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
