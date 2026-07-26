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

/// What every verb module needs. A glob rather than a list per file because the
/// alternative is nine import blocks that drift apart.
pub(crate) mod prelude {
    pub(crate) use anyhow::{Context, Result};
    pub(crate) use linix::app::sync::planner::Scope as PlannerScope;
    pub(crate) use linix::app::{ui::TuiPreview, App};
    pub(crate) use linix::cli::{
        Cli, ConfigCommand, GitCommand, HooksCommand, ModuleCommand, ProfileCommand, RepoCommand,
        ScheduleCommand, ServiceCommand, SnapshotCommand,
    };
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
