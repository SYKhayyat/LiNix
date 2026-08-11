use crate::verbs::plan::compute_full_changes;
use crate::verbs::prelude::*;
use crate::verbs::sync::{handle_sync, SyncMode};

pub async fn handle_snapshot(app: &App, cmd: &SnapshotCommand) -> Result<()> {
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
                         restore point. `linix check` says what is available here."
                    );
                }
            }
            for s in list {
                println!("{:<15} {}", s.backend, s.id);
            }
        }
        SnapshotCommand::Restore => {
            return handle_snapshot_restore(app).await;
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
pub async fn handle_rollback(app: &App, reference: &str) -> Result<()> {
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
        return Err(crate::core::Error::Refused(format!(
            "rollback: refusing to restore {}.\n  \
             - git says it is {}\n\n\
             `require_signed_history` is on, so every commit you roll back to must carry a \
             signature git vouches for. Sign your commits (`git config commit.gpgsign true`), \
             or turn the rule off in preferences.toml.",
            reference,
            signature.describe()
        ))
        .into());
    }

    // The bail must come before the checkout, not after. `handle_sync` refuses unconfirmed
    // changes in a non-interactive shell, but by the time it does the manifests have already
    // been overwritten — leaving the files rolled back and the machine not.
    if !app.config.yes {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            return Err(crate::core::Error::Refused(
                "Refusing to roll back without confirmation in a non-interactive shell. \
                 Re-run with --yes to proceed, or --dry-run to preview."
                    .to_string(),
            )
            .into());
        }
    }
    info!("checking out manifests at {}.", reference);
    git.checkout_files(reference)?;
    println!(
        "Manifests restored to {}. Converging the system to match…",
        reference
    );
    // The rollback is not complete until the machine matches the restored manifests.
    handle_sync(app, SyncMode::default(), Output::Human).await
}

/// `linix diff <from> [to]` — what changed between two commits, in packages (Phase 4). The
/// manifests are package declarations, so a diff of the manifest files IS the package-level
/// change; git already records it. Omitting `to` compares `from` against your working tree.
pub async fn handle_diff(app: &App, from: &str, to: Option<&str>) -> Result<()> {
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

pub async fn handle_git(app: &App, cmd: &GitCommand) -> Result<()> {
    // Asked once, for every subcommand. Only `init` used to ask, so on a machine without
    // git the others answered from `.git`'s absence: `log` printed an empty history, and
    // `status` advised running `git init`, which could only refuse.
    crate::core::GitManager::require()?;
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
                    crate::core::git::Signature::Unsigned => {
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

pub async fn handle_shell(app: &App, packages: &[String]) -> Result<()> {
    app.shell().enter(packages).await.map_err(|e| e.into())
}

/// `linix run --packages X -- cmd arg…`
///
/// **One rule, both spellings.** The first positional may still carry a whole command line, which
/// is what the quoted form (`-- "jq -r .name"`) has always meant; everything after it is an
/// argument, verbatim. The second half is new: `command` was a lone positional, so clap refused
/// `-- jq -r .name` outright — and with it `src/bin/shim.rs`, which builds exactly that argv and
/// is the entire mechanism behind a `@shim=true` line.
pub async fn handle_run(
    app: &App,
    packages: &[String],
    command: &str,
    trailing: &[String],
) -> Result<()> {
    let mut parts: Vec<String> = command.split_whitespace().map(str::to_string).collect();
    parts.extend(trailing.iter().cloned());
    let Some((bin, args)) = parts.split_first() else {
        return Err(
            crate::core::Error::Validation("`linix run` needs a command to run".into()).into(),
        );
    };
    app.runner()
        .run(packages, bin, args)
        .await
        .map_err(|e| e.into())
}

pub async fn handle_adopt(app: &App, backends: Vec<String>, enabled_only: bool) -> Result<()> {
    // A name that reaches no backend is refused rather than silently adopting nothing: `linix
    // adopt srvice` answering "Adopted 0 declaration(s)" is byte-identical to a correct name
    // with nothing to take, so a typo cannot be told from a no-op (Q9).
    //
    // Through `require_known_backend` and not a message of its own: `install`'s wording is the
    // one refusal, and a second spelling of it is how E18's family started.
    for name in &backends {
        app.require_known_backend(Some(name))?;
    }
    let scope = crate::app::adopt::AdoptScope {
        backends,
        enabled_only,
    };
    app.adopter()
        .adopt_scoped(&scope)
        .await
        .map_err(|e| e.into())
}

pub async fn handle_snapshot_restore(app: &App) -> Result<()> {
    app.snapshot_restore()
        .run_interactive()
        .await
        .map_err(|e| e.into())
}

pub async fn handle_history(app: &App) -> Result<()> {
    use crate::app::ui::{CommitView, HistoryAction, HistoryBrowser};

    // A TUI needs a terminal to draw on. Without this, a piped or scheduled `linix history`
    // reached `enable_raw_mode` and failed with an OS error about a console handle — the same
    // hole `sync` and `rollback` already close, on the one command that is only ever a TUI.
    {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
            return Err(crate::core::Error::Refused(
                "`linix history` is an interactive browser and needs a terminal. \
                 For the same timeline without one, use `linix git log`; to go back, \
                 `linix rollback <ref> --yes`."
                    .to_string(),
            )
            .into());
        }
    }

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

    let action = crate::core::on_the_terminal(|| HistoryBrowser::new(commits).run())?;
    match action {
        HistoryAction::Quit => Ok(()),
        HistoryAction::Rollback { reference } => {
            // **`history` is `LockScope::Deferred` and this is where the deferral ends.** The
            // browser is a TUI a person reads for as long as they like, so locking the whole
            // command would stop every other LiNix on the machine for the length of a reading
            // session — the `edit`-blocks-on-$EDITOR problem AU6 records. But this arm reaches
            // `handle_rollback` → `handle_sync`: the entire install/remove path, `state.save()`
            // and all. The same function through `Commands::Rollback` is locked; through this
            // door it was not, so one function had two locking regimes decided by which one the
            // user happened to walk through.
            let _data_lock =
                crate::core::datalock::DataLock::for_one_step("history rollback").await?;
            println!("Rolling back to {reference}…");
            handle_rollback(app, &reference).await
        }
    }
}

pub async fn handle_audit(app: &App, out: Output) -> Result<()> {
    let report = crate::app::insight::audit(app).await?;
    crate::app::insight::print_audit(&report, out).map_err(|e| e.into())
}

pub async fn handle_sbom(app: &App) -> Result<()> {
    println!("{}", crate::app::insight::sbom(app).await?);
    Ok(())
}

pub async fn handle_export(
    app: &App,
    format: Option<&str>,
    out: &str,
    stdout: bool,
    force: bool,
) -> Result<()> {
    use crate::app::export::{export, Format, Outcome};
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
                println!(
                    "  {} would write {}",
                    crate::core::dry_run::MARKER,
                    path.display()
                )
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

pub async fn handle_bundle(app: &App, out: &str, artifacts: bool, archive: bool) -> Result<()> {
    let out_path = std::path::PathBuf::from(out);

    // Freeze a plan so the target can review/apply it offline. Computed up front so it can be
    // written into the bundle (and captured inside the archive) by create_bundle.
    let plan_json = match compute_full_changes(app, None).await {
        Ok(full) => {
            let mut plan = crate::app::sync::SavedPlan::from_changes(
                &full.changes,
                &full.resources,
                Some(chrono::Utc::now().timestamp()),
            );
            plan.vars = full.state.vars;
            Some(serde_json::to_string_pretty(&plan)?)
        }
        Err(_) => None,
    };

    let report =
        crate::app::bundle::create_bundle(app, &out_path, artifacts, archive, plan_json.as_deref())
            .await?;

    // The tense comes from the writer, not from asking the flag a second time (Q15/V.105).
    // `--dry-run bundle` wrote all nine files and said "Bundle written to X" — a preview that
    // manufactured the artifact it was asked to describe, and reported it in the past tense.
    let lead = if report.previewed {
        format!("{} would write a bundle to", crate::core::dry_run::MARKER)
    } else {
        "Bundle written to".to_string()
    };
    println!(
        "{} {} — {} config file(s), {} package(s).",
        lead,
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
            "Artifacts: {} {}, {} skipped.",
            report.artifacts_fetched.len(),
            if report.previewed {
                "would be fetched"
            } else {
                "fetched"
            },
            report.artifacts_skipped.len()
        );
        // Honest reporting: never let a skipped backend read as "bundled everything".
        for s in &report.artifacts_skipped {
            println!("  skipped {}", s);
        }
    }
    if let Some((path, size)) = &report.archive {
        if report.previewed {
            println!("Archive: {} would be written.", path.display());
        } else {
            println!(
                "Archive: {} ({:.1} KiB) — copy this one file to an air-gapped host.",
                path.display(),
                *size as f64 / 1024.0
            );
        }
    }
    if report.previewed {
        println!("Nothing was written. Run without `--dry-run` to produce the bundle.");
    } else {
        println!(
            "See {}/RESTORE.md for offline restore steps.",
            report.out.display()
        );
    }
    Ok(())
}

pub async fn handle_restore(app: &App, dir: &str, force: bool) -> Result<()> {
    let bundle_dir = std::path::PathBuf::from(dir);
    let config_root = app.config.config_root();
    let registry_path = { app.state.lock().await.path.clone() };

    let report =
        crate::app::bundle::restore_bundle(&bundle_dir, &config_root, &registry_path, force)
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

pub async fn handle_why(app: &App, package: &str, out: Output) -> Result<()> {
    // Q9: `why nosuchbackend:foo` reported it "not under LiNix management" at exit 0 — true of
    // the string and useless, because the manager is the part that does not exist.
    app.require_known_spec_backends(std::slice::from_ref(&package.to_string()))
        .await?;
    crate::app::insight::why(app, package, out)
        .await
        .map_err(|e| e.into())
}
