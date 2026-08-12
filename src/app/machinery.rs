//! Everything it takes to change this machine, carried as one value.
//!
//! **This is the god object's shadow.** `App` stopped being passed around, and what replaced it
//! at the three places that converge — `SyncEngine`, `ProfileManager`, `EphemeralShell` — was
//! the same eleven collaborators spelled out by hand: **33 parameter declarations in three
//! different orders**, and 55 arguments across the five sites that called them, each `new`
//! wearing an `#[allow(clippy::too_many_arguments)]` to say so. Passing twelve fields one at a
//! time is not an improvement on passing the struct that holds them; it is the same coupling
//! with more places to get it wrong, and two of the three orders put `config` in a different
//! position from the other.
//!
//! The list is not arbitrary and that is the point: these eleven are exactly what a *change* to
//! the machine needs — the managers, the process runner, the log, the snapshot, the removal
//! budget — so they travel together. A reader that only queries takes [`Inventory`]; a caller
//! that only edits files takes [`Declarations`]. This is the third set, and it has one home.
//!
//! [`Inventory`]: crate::app::Inventory
//! [`Declarations`]: crate::app::Declarations

use crate::app::diagnostics::FailureDiagnosticEngine;
use crate::app::sync::guard;
use crate::app::{LuaHooks, MetricsCollector};
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::core::{CommandExecutor, Journal, SnapshotManager, StateRegistry};
use crate::utils::progress::ProgressReporter;
use std::sync::Arc;
use tokio::sync::Mutex;

/// The collaborators a state-changing command runs on.
///
/// Every field is an `Arc` or an executor handle, so cloning this shares one run's state rather
/// than forking it — which is the property the three engines depend on. The removal budget in
/// particular is a budget *for the command*: two clones that did not share it would each get the
/// full ceiling, and the guard would be a guard on nothing.
#[derive(Clone)]
pub struct Machinery {
    pub config: Arc<Config>,
    pub registry: Arc<BackendRegistry>,
    pub executor: CommandExecutor,
    pub metrics: MetricsCollector,
    pub progress: Arc<dyn ProgressReporter>,
    pub hooks: Arc<LuaHooks>,
    pub snapshot_manager: Arc<SnapshotManager>,
    pub journal: Arc<Mutex<Journal>>,
    pub state: Arc<Mutex<StateRegistry>>,
    pub diagnostics: Arc<FailureDiagnosticEngine>,
    pub reaping: Arc<guard::Reaping>,
}
