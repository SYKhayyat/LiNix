//! **Time travel: the snapshots, the git history behind the config repo, and the two verbs
//! that move between them.**
//!
//! This file used to hold fifteen handlers, seven of which had nothing to do with history —
//! `shell`, `run`, `adopt`, `sbom`, `export`, `bundle` and `why`. Modules organised by size
//! rather than by subject: a reader looking for `why` had no reason to open a file called
//! `history`, and a reader opening `history` met four unrelated subjects before reaching one.

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
                         restore point. `shall check` says what is available here."
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

/// `shall rollback <ref>` — the one rollback (owner decision, Phase 4): check out the manifests
/// at a past git commit, then `sync` the machine to match. There is no separate generation
/// history — git IS the history (II.1), so a rollback is "point the manifests at then, converge
/// now". Whole-config by nature: git checkout is all-or-nothing, which is why the old
/// per-package / with-config flags are gone.
pub async fn handle_rollback(app: &App, reference: &str) -> Result<()> {
    let git = app.git_manager();
    if !git.is_repo() {
        anyhow::bail!(
            "Rollback needs manifest history. Run `shall git init` once to start version-\
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

/// `shall diff <from> [to]` — what changed between two commits, in packages (Phase 4). The
/// manifests are package declarations, so a diff of the manifest files IS the package-level
/// change; git already records it. Omitting `to` compares `from` against your working tree.
pub async fn handle_diff(app: &App, from: &str, to: Option<&str>) -> Result<()> {
    let git = app.git_manager();
    if !git.is_repo() {
        anyhow::bail!(
            "`diff` compares commits of your manifest history, which is git. Run `shall git \
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
            let first = git.commit_all("shall: config at the time history was enabled")?;
            println!(
                "Initialized manifest version control at {}.\n\
                 Shall will now auto-commit config/manifest changes after each command.",
                git.root().display()
            );
            match first {
                Some(hash) => println!("Your config as it stands is committed as {}.", hash),
                None => println!("There was nothing to commit yet."),
            }
        }
        GitCommand::Status => {
            if !git.is_repo() {
                println!("Not a git repo yet. Run `shall git init` to enable manifest history.");
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
                println!("Not a git repo yet. Run `shall git init` first.");
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
                anyhow::bail!("Not a git repo. Run `shall git init` first.");
            }
            git.checkout_files(reference)?;
            println!(
                "Manifests restored to {}. Installed packages are unchanged — run `shall sync` \
                 to converge the system to these manifests.",
                reference
            );
        }
    }
    Ok(())
}

pub async fn handle_snapshot_restore(app: &App) -> Result<()> {
    app.snapshot_restore()
        .run_interactive()
        .await
        .map_err(|e| e.into())
}

pub async fn handle_history(app: &App) -> Result<()> {
    use crate::app::ui::{CommitView, HistoryAction, HistoryBrowser};

    // A TUI needs a terminal to draw on. Without this, a piped or scheduled `shall history`
    // reached `enable_raw_mode` and failed with an OS error about a console handle — the same
    // hole `sync` and `rollback` already close, on the one command that is only ever a TUI.
    {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
            return Err(crate::core::Error::Refused(
                "`shall history` is an interactive browser and needs a terminal. \
                 For the same timeline without one, use `shall git log`; to go back, \
                 `shall rollback <ref> --yes`."
                    .to_string(),
            )
            .into());
        }
    }

    let git = app.git_manager();
    if !git.is_repo() {
        println!(
            "The history browses your manifest history, which is git. Run `shall git init` \
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
        println!("No commits yet. Run `shall sync` (it commits after each successful change).");
        return Ok(());
    }

    let action = crate::core::on_the_terminal(|| HistoryBrowser::new(commits).run())?;
    match action {
        HistoryAction::Quit => Ok(()),
        HistoryAction::Rollback { reference } => {
            // **`history` is `LockScope::Deferred` and this is where the deferral ends.** The
            // browser is a TUI a person reads for as long as they like, so locking the whole
            // command would stop every other Shall on the machine for the length of a reading
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
