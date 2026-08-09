//! One module per verb family. `main.rs` is the entry point and the argv it is
//! handed; everything a subcommand actually does lives here.

pub mod check;
pub mod cleanup;
pub mod declare;
pub mod history;
pub mod packages;
pub mod plan;
pub mod setup;
pub mod sync;
pub mod upgrade;

/// Write a file in the config repo — unless this run only says what it would do.
///
/// The verbs that edit `modules/` and `schedules` by hand do not go through the model's
/// [`Editor`](crate::model::Editor), which is where `--dry-run` became an editing mode rather
/// than a flag six callers remember. They reach the disk here instead, so the guard is in one
/// place for them too. Returns the word to print, because a message that says "Added" after a
/// write that did not happen is the bug wearing different clothes.
pub async fn write_unless_previewing(
    app: &crate::app::App,
    path: &std::path::Path,
    body: &str,
    done: &'static str,
    planned: &'static str,
) -> anyhow::Result<String> {
    if app.config.dry_run {
        return Ok(format!("{} {}", crate::core::dry_run::MARKER, planned));
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    tokio::fs::write(path, body).await?;
    Ok(done.to_string())
}

/// What every verb module needs. A glob rather than a list per file because the
/// alternative is nine import blocks that drift apart.
use crate::app::App;
use anyhow::Result;
use tracing::warn;

/// The tail every state-changing verb runs: expire what has expired, commit if asked, prune.
///
/// It lived in `main.rs` and was reached through the prelude, so moving `verbs` into the
/// library left eight call sites pointing at a function in the other crate. It belongs here
/// anyway — it is what a verb does after it has changed the machine, not part of reading argv.
pub async fn perform_maintenance(app: &App) -> Result<()> {
    app.journal.lock().await.cleanup()?;
    // Reclaim expired temporary installs so leases are enforced on every state-changing run.
    if let Err(e) = app.leases().sweep_expired().await {
        warn!("Maintenance: lease sweep failed: {}", e);
    }
    // Restore temporary uninstalls whose timer has elapsed (mirror of the lease sweep).
    if let Err(e) = app.leases().sweep_due_suspensions().await {
        warn!("Maintenance: suspension sweep failed: {}", e);
    }
    // Version-control the manifests/config if the user opted in via `linix git init`.
    app.git_autocommit("linix: sync manifest state").await;
    if app.config.snapshot_retention().prunes() {
        app.prune_snapshots(false).await?;
    }
    Ok(())
}

pub mod prelude {
    pub use crate::app::sync::planner::PlanScope;
    pub use crate::app::sync::planner::Scope as PlannerScope;
    pub use crate::app::{ui::TuiPreview, App};
    pub use anyhow::{Context, Result};
    // The ledger file rules are a trait, so `HookLedger::load` needs it in scope. In the
    // prelude rather than per verb for the reason the prelude exists.
    pub use crate::cli::{
        Cli, ConfigCommand, GitCommand, HooksCommand, LockAxis, ModuleCommand, ProfileCommand,
        RepoCommand, ScheduleCommand, ServiceCommand, SnapshotCommand,
    };
    pub use crate::core::LockFile;
    // Every handler that takes `--json` takes it as this, so it belongs here for the same
    // reason `Result` does.
    pub use crate::core::Output;
    pub use serde_json::Value;
    pub use std::collections::HashMap;
    pub use tracing::{info, warn};

    // **The nine `verbs::*` globs and `crate::*` used to be here, and their absence is the
    // point.** With them, there was no module boundary inside `verbs/` at all: 8,587 lines in
    // one namespace stored in nine files, where moving a function between them was a no-op and
    // no rule about where a handler belongs could be enforced — or even stated. That is why
    // `history.rs` holds `handle_export`, `handle_shell`, `handle_adopt`, `handle_bundle` and
    // `handle_why`; why `plan.rs` holds `lock`/`unlock`; why `setup.rs` holds `handle_policy`
    // and `handle_try`. **Nothing can enforce a name that costs nothing to violate.**
    //
    // What is left is what a prelude is for: the handful of types every handler in this
    // directory genuinely uses. A cross-file reference is now an import, which means the
    // compiler can be asked where things are — and a rule about file layout has something to
    // describe.
}
