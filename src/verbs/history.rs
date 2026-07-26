use crate::verbs::prelude::*;

pub(crate) async fn handle_snapshot(app: &App, cmd: &SnapshotCommand) -> Result<()> {
    match cmd {
        SnapshotCommand::List => {
            let list = app.snapshot_manager.list_snapshots().await?;
            if list.is_empty() {
                // Two different facts print the same empty list, and the difference is the
                // whole answer: no provider means snapshots are not available on this machine.
                if app.snapshot_manager.has_provider() {
                    println!("No snapshots yet. `sync` takes one before it changes anything.");
                } else {
                    println!(
                        "No snapshot provider on this machine (btrfs, ZFS, Timeshift or Windows \
                         System Restore) — nothing can be listed, and `sync` proceeds without a \
                         restore point. `linix doctor` says what is available here."
                    );
                }
            }
            for s in list {
                println!("{:<15} {}", s.backend, s.id);
            }
        }
        SnapshotCommand::Prune { force } => {
            app.prune_snapshots(*force).await?;
        }
    }
    Ok(())
}

/// `linix rollback <ref>` — the one rollback (owner decision, Phase 4): check out the manifests
/// at a past git commit, then `sync` the machine to match. There is no separate generation
/// history — git IS the history (II.1), so a rollback is "point the manifests at then, converge
/// now". Whole-config by nature: git checkout is all-or-nothing, which is why the old
/// per-package / with-config flags are gone.
pub(crate) async fn handle_rollback(app: &App, reference: &str) -> Result<()> {
    let git = app.git_manager();
    if !git.is_repo() {
        anyhow::bail!(
            "Rollback needs manifest history. Run `linix git init` once to start version-\
             controlling your config; after that every sync commits, and you can roll back to \
             any commit."
        );
    }
    // II.13's integrity check, on the commit being restored rather than on HEAD. Off unless
    // `require_signed_history` is set, because a fresh repo signs nothing.
    let signature = git.signature_of(reference)?;
    if app.config.guard.require_signed_history && !signature.is_verified() {
        anyhow::bail!(
            "rollback: refusing to restore {}.\n  \
             - git says it is {}\n\n\
             `require_signed_history` is on, so every commit you roll back to must carry a \
             signature git vouches for. Sign your commits (`git config commit.gpgsign true`), \
             or turn the rule off in preferences.toml.",
            reference,
            signature.describe()
        );
    }

    // The bail must come before the checkout, not after. `handle_sync` refuses unconfirmed
    // changes in a non-interactive shell, but by the time it does the manifests have already
    // been overwritten — leaving the files rolled back and the machine not.
    if !app.config.yes {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            anyhow::bail!(
                "Refusing to roll back without confirmation in a non-interactive shell. \
                 Re-run with --yes to proceed, or --dry-run to preview."
            );
        }
    }
    info!("checking out manifests at {}.", reference);
    git.checkout_files(reference)?;
    println!(
        "Manifests restored to {}. Converging the system to match…",
        reference
    );
    // The rollback is not complete until the machine matches the restored manifests.
    handle_sync(app, false, false, false).await
}

/// `linix diff <from> [to]` — what changed between two commits, in packages (Phase 4). The
/// manifests are package declarations, so a diff of the manifest files IS the package-level
/// change; git already records it. Omitting `to` compares `from` against your working tree.
pub(crate) async fn handle_diff(app: &App, from: &str, to: Option<&str>) -> Result<()> {
    let git = app.git_manager();
    if !git.is_repo() {
        anyhow::bail!(
            "`diff` compares commits of your manifest history, which is git. Run `linix git \
             init` once to start version-controlling your config."
        );
    }
    let changes = git.diff_manifest_changes(from, to)?;
    let target = to.unwrap_or("working tree");
    if changes.is_empty() {
        println!("No manifest changes between {} and {}.", from, target);
        return Ok(());
    }
    println!("Manifest changes {} → {}:", from, target);
    for line in &changes {
        println!("  {}", line);
    }
    let (added, removed) =
        changes
            .iter()
            .fold((0usize, 0usize), |(a, r), l| match l.chars().next() {
                Some('+') => (a + 1, r),
                Some('-') => (a, r + 1),
                _ => (a, r),
            });
    println!("\n{} added, {} removed.", added, removed);
    Ok(())
}

pub(crate) async fn handle_git(app: &App, cmd: &GitCommand) -> Result<()> {
    // Asked once, for every subcommand. Only `init` used to ask, so on a machine without
    // git the others answered from `.git`'s absence: `log` printed an empty history, and
    // `status` advised running `git init`, which could only refuse.
    linix::core::GitManager::require()?;
    let git = app.git_manager();
    match cmd {
        GitCommand::Init => {
            git.init()?;
            // Without this commit there is no HEAD, so `diff` and `rollback` answer with
            // git's "unknown revision" until some later command happens to commit. History
            // has to be usable from the moment it is switched on.
            let first = git.commit_all("linix: config at the time history was enabled")?;
            println!(
                "Initialized manifest version control at {}.\n\
                 LiNix will now auto-commit config/manifest changes after each command.",
                git.root().display()
            );
            match first {
                Some(hash) => println!("Your config as it stands is committed as {}.", hash),
                None => println!("There was nothing to commit yet."),
            }
        }
        GitCommand::Status => {
            if !git.is_repo() {
                println!("Not a git repo yet. Run `linix git init` to enable manifest history.");
                return Ok(());
            }
            let status = git.status_porcelain()?;
            if status.trim().is_empty() {
                println!("Manifests are clean (no uncommitted changes).");
            } else {
                println!("{}", status);
            }
        }
        GitCommand::Log { limit } => {
            if !git.is_repo() {
                println!("Not a git repo yet. Run `linix git init` first.");
                return Ok(());
            }
            let commits = git.log(*limit)?;
            if commits.is_empty() {
                println!("No commits yet.");
            }
            for c in commits {
                // The signature is named only when there is one: a repo nobody signs would
                // otherwise carry "unsigned" on every row, which is noise, not a finding.
                match &c.signature {
                    linix::core::git::Signature::Unsigned => {
                        println!("{}  {}  {}", c.short, c.date, c.subject)
                    }
                    sig => println!(
                        "{}  {}  {}  [{}]",
                        c.short,
                        c.date,
                        c.subject,
                        sig.describe()
                    ),
                }
            }
        }
        GitCommand::Commit { message } => {
            git.init().ok(); // ensure a repo exists so `commit` is a one-step action
            match git.commit_all(message)? {
                Some(hash) => println!("Committed {} — {}", &hash[..hash.len().min(8)], message),
                None => println!("Nothing to commit; manifests are already up to date."),
            }
        }
        GitCommand::Checkout { reference } => {
            if !git.is_repo() {
                anyhow::bail!("Not a git repo. Run `linix git init` first.");
            }
            git.checkout_files(reference)?;
            println!(
                "Manifests restored to {}. Installed packages are unchanged — run `linix sync` \
                 to converge the system to these manifests.",
                reference
            );
        }
    }
    Ok(())
}

pub(crate) async fn handle_shell(app: &App, packages: &[String]) -> Result<()> {
    app.shell().enter(packages).await.map_err(|e| e.into())
}

pub(crate) async fn handle_run(app: &App, packages: &[String], command: &str) -> Result<()> {
    let parts: Vec<&str> = command.split_whitespace().collect();
    let bin = parts.first().unwrap_or(&"");
    let args: Vec<String> = parts.iter().skip(1).map(|s| s.to_string()).collect();
    app.runner()
        .run(packages, bin, &args)
        .await
        .map_err(|e| e.into())
}

pub(crate) async fn handle_adopt(app: &App) -> Result<()> {
    app.adopter().adopt().await.map_err(|e| e.into())
}

pub(crate) async fn handle_undo(app: &App) -> Result<()> {
    app.undo_manager()
        .run_interactive()
        .await
        .map_err(|e| e.into())
}

pub(crate) async fn handle_history(app: &App) -> Result<()> {
    use linix::app::ui::{CommitView, HistoryAction, HistoryBrowser};

    let git = app.git_manager();
    if !git.is_repo() {
        println!(
            "The history browses your manifest history, which is git. Run `linix git init` \
             once; after that every `sync` commits, and the history shows the timeline."
        );
        return Ok(());
    }

    // The timeline is the commit log; each row carries the manifest lines that commit changed.
    let commits: Vec<CommitView> = git
        .log(200)?
        .into_iter()
        .map(|c| {
            let changes = git.commit_manifest_changes(&c.hash).unwrap_or_default();
            CommitView {
                short: c.short,
                date: c.date,
                subject: c.subject,
                full_hash: c.hash,
                changes,
                signature: c.signature.describe(),
            }
        })
        .collect();

    if commits.is_empty() {
        println!("No commits yet. Run `linix sync` (it commits after each successful change).");
        return Ok(());
    }

    let action = HistoryBrowser::new(commits).run()?;
    match action {
        HistoryAction::Quit => Ok(()),
        HistoryAction::Rollback { reference } => {
            println!("Rolling back to {reference}…");
            handle_rollback(app, &reference).await
        }
    }
}

pub(crate) async fn handle_audit(app: &App, json: bool) -> Result<()> {
    let report = linix::app::insight::audit(app).await?;
    linix::app::insight::print_audit(&report, json).map_err(|e| e.into())
}

pub(crate) async fn handle_sbom(app: &App) -> Result<()> {
    println!("{}", linix::app::insight::sbom(app).await?);
    Ok(())
}

pub(crate) async fn handle_export(
    app: &App,
    format: Option<&str>,
    out: &str,
    stdout: bool,
    force: bool,
) -> Result<()> {
    use linix::app::export::{export, Format, Outcome};
    let fmt = match format {
        Some(s) => Some(
            Format::parse(s)
                .with_context(|| format!("unknown export format '{}' (brew|pip|npm|apt)", s))?,
        ),
        None => None,
    };
    if stdout && fmt.is_none() {
        anyhow::bail!("--stdout needs a single --format (brew|pip|npm|apt).");
    }
    let out_dir = std::path::PathBuf::from(out);
    let results = export(app, fmt, &out_dir, stdout, force, app.config.dry_run).await?;
    for (file, outcome) in &results {
        match outcome {
            Outcome::NoPackages => println!("  skipped {} (no matching packages)", file),
            Outcome::Wrote(path) => println!("  wrote   {}", path.display()),
            Outcome::WouldWrite(path) => {
                println!("  [DRY-RUN] would write {}", path.display())
            }
            Outcome::WroteBeside { taken, renamed } => {
                println!("  wrote   {}", renamed.display());
                println!(
                    "          ({} already exists and was left alone; re-run with --force to replace it)",
                    taken.display()
                );
            }
        }
    }
    Ok(())
}

pub(crate) async fn handle_bundle(
    app: &App,
    out: &str,
    artifacts: bool,
    archive: bool,
) -> Result<()> {
    let out_path = std::path::PathBuf::from(out);

    // Freeze a plan so the target can review/apply it offline. Computed up front so it can be
    // written into the bundle (and captured inside the archive) by create_bundle.
    let plan_json = match compute_full_changes(app, None).await {
        Ok((changes, vars)) => {
            let mut plan = linix::app::sync::SavedPlan::from_changes(
                &changes,
                Some(chrono::Utc::now().timestamp()),
            );
            plan.vars = vars;
            Some(serde_json::to_string_pretty(&plan)?)
        }
        Err(_) => None,
    };

    let report =
        linix::app::bundle::create_bundle(app, &out_path, artifacts, archive, plan_json.as_deref())
            .await?;

    println!(
        "Bundle written to {} — {} config file(s), {} package(s).",
        report.out.display(),
        report.files_copied,
        report.package_count
    );
    // Honest per-part reporting: say plainly what did and did NOT make it into the bundle.
    println!(
        "  manifest history (git bundle): {}",
        if report.git_history_included {
            "included (config.bundle) — `git clone` it to roll back to any past commit"
        } else {
            "NOT included — the config is not a git repo (or has no commits); run `linix git init`"
        }
    );
    println!(
        "  ownership registry (registry.json): {}",
        if report.registry_included {
            "included"
        } else {
            "NOT included — none found"
        }
    );
    if artifacts {
        println!(
            "Artifacts: {} fetched, {} skipped.",
            report.artifacts_fetched.len(),
            report.artifacts_skipped.len()
        );
        // Honest reporting: never let a skipped backend read as "bundled everything".
        for s in &report.artifacts_skipped {
            println!("  skipped {}", s);
        }
    }
    if let Some((path, size)) = &report.archive {
        println!(
            "Archive: {} ({:.1} KiB) — copy this one file to an air-gapped host.",
            path.display(),
            *size as f64 / 1024.0
        );
    }
    println!(
        "See {}/RESTORE.md for offline restore steps.",
        report.out.display()
    );
    Ok(())
}

pub(crate) async fn handle_restore(app: &App, dir: &str, force: bool) -> Result<()> {
    let bundle_dir = std::path::PathBuf::from(dir);
    let config_root = app.config.config_root();
    let registry_path = { app.state.lock().await.path.clone() };

    let report =
        linix::app::bundle::restore_bundle(&bundle_dir, &config_root, &registry_path, force)
            .await?;

    println!(
        "Restored {} config file(s) into {}.",
        report.config_files,
        config_root.display()
    );
    println!(
        "  ownership registry: {}",
        if report.registry_restored {
            "restored"
        } else {
            "not in the bundle — a first `sync` will rebuild it"
        }
    );
    if report.git_history_present {
        println!(
            "  manifest history: `config.bundle` is in {} — `git clone` it there to keep the \
             history, or `linix sync --locked` to reproduce the current state.",
            bundle_dir.display()
        );
    }
    println!("Run `linix sync --locked` to reproduce the exact package set.");
    Ok(())
}

pub(crate) async fn handle_why(app: &App, package: &str, json: bool) -> Result<()> {
    linix::app::insight::why(app, package, json)
        .await
        .map_err(|e| e.into())
}
