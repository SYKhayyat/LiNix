//! One module per verb family. `main.rs` is the entry point and the argv it is
//! handed; everything a subcommand actually does lives here.

pub(crate) mod check;
pub(crate) mod cleanup;
pub(crate) mod declare;
pub(crate) mod history;
pub(crate) mod packages;
pub(crate) mod plan;
pub(crate) mod setup;
pub(crate) mod sync;
pub(crate) mod upgrade;

/// Write a file in the config repo — unless this run only says what it would do.
///
/// The verbs that edit `modules/` and `schedules` by hand do not go through the model's
/// [`Editor`](linix::model::Editor), which is where `--dry-run` became an editing mode rather
/// than a flag six callers remember. They reach the disk here instead, so the guard is in one
/// place for them too. Returns the word to print, because a message that says "Added" after a
/// write that did not happen is the bug wearing different clothes.
pub(crate) async fn write_unless_previewing(
    app: &linix::app::App,
    path: &std::path::Path,
    body: &str,
    done: &'static str,
    planned: &'static str,
) -> anyhow::Result<&'static str> {
    if app.config.dry_run {
        return Ok(planned);
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    tokio::fs::write(path, body).await?;
    Ok(done)
}

/// What every verb module needs. A glob rather than a list per file because the
/// alternative is nine import blocks that drift apart.
pub(crate) mod prelude {
    pub(crate) use anyhow::{Context, Result};
    pub(crate) use linix::app::sync::planner::Scope as PlannerScope;
    pub(crate) use linix::app::{ui::TuiPreview, App};
    pub(crate) use linix::cli::{
        Cli, ConfigCommand, GitCommand, HooksCommand, LockAxis, ModuleCommand, ProfileCommand,
        RepoCommand, ScheduleCommand, ServiceCommand, SnapshotCommand,
    };
    pub(crate) use serde_json::Value;
    pub(crate) use std::collections::HashMap;
    pub(crate) use tracing::{info, warn};

    pub(crate) use crate::verbs::check::*;
    pub(crate) use crate::verbs::cleanup::*;
    pub(crate) use crate::verbs::declare::*;
    pub(crate) use crate::verbs::history::*;
    pub(crate) use crate::verbs::packages::*;
    pub(crate) use crate::verbs::plan::*;
    pub(crate) use crate::verbs::setup::*;
    pub(crate) use crate::verbs::sync::*;
    pub(crate) use crate::verbs::upgrade::*;
    #[allow(unused_imports)]
    pub(crate) use crate::*;
}
