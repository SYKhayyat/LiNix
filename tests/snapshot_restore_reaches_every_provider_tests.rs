//! A provider whose row says it can put a running machine back must be able to.
//!
//! `U27` ruled that snapshot providers are data: btrfs, zfs, timeshift, lvm and Windows System
//! Restore "stop being a hardcoded list and become rows read through the one loader a user's row
//! goes through". `core/snapshot.rs` does that — `ConfigSnapshotProvider` drives whatever argv the
//! row names, and `restores_running_system` is the declared capability.
//!
//! The command a user reaches it through did not. `snapshot restore` is the only path to
//! `SnapshotProvider::restore`, and on the way it asked which provider this is **by name** — so
//! every provider outside a two-name list was refused with `Unsupported snapshot backend` before
//! the confirmation prompt, whatever its row declared. That is `zfs` (`zfs rollback -r`, live),
//! `windows_restore` (`Restore-Computer`, live), `apfs`, and every `lvm`/bcachefs row a user
//! writes: the whole point of the ruling.
//!
//! These tests quantify over *any* provider rather than over today's list, so a provider added
//! next year is covered without anyone remembering this file exists.

use async_trait::async_trait;
use shall::app::snapshot_restore::SnapshotRestore;
use shall::core::snapshot::{RestoreCapability, SnapshotProvider};
use shall::core::{Result, Snapshot, SnapshotManager, StateRegistry};
use std::sync::Arc;
use tokio::sync::Mutex;

/// A provider that is not btrfs and not timeshift, declares a live restore, and exposes no
/// readable snapshot tree — which is true of zfs, of Windows System Restore, and of every lvm row.
struct DeclaresLiveRestore {
    name: &'static str,
}

#[async_trait]
impl SnapshotProvider for DeclaresLiveRestore {
    fn name(&self) -> &str {
        self.name
    }
    async fn is_available(&self) -> bool {
        true
    }
    async fn create(&self, _label: shall::core::snapshot::SnapshotLabel) -> Result<Snapshot> {
        unreachable!("these tests never create")
    }
    async fn list(&self) -> Result<Vec<Snapshot>> {
        Ok(vec![snapshot_of(self.name)])
    }
    async fn delete(&self, _id: &str) -> Result<()> {
        Ok(())
    }
    async fn restore(&self, _id: &str) -> Result<()> {
        Ok(())
    }
    fn restore_capability(&self) -> RestoreCapability {
        RestoreCapability::Live
    }
}

fn snapshot_of(backend: &str) -> Snapshot {
    Snapshot {
        id: "shall_pre_sync_20260804120000".to_string(),
        backend: backend.to_string(),
        timestamp: "2026-08-04 12:00:00".to_string(),
        description: "Shall: pre-sync".to_string(),
    }
}

fn restorer(p: Box<dyn SnapshotProvider>) -> SnapshotRestore {
    SnapshotRestore::new(
        Arc::new(SnapshotManager::with_provider(p)),
        Arc::new(Mutex::new(StateRegistry::default())),
    )
}

/// The whole finding, as one sentence: a provider that says it restores is not turned away for
/// having a name nobody wrote down.
///
/// `restore_preamble` is the step between "you chose this snapshot" and "type RESTORE". It used to
/// be a `match` on the provider's name with two arms and an error, so this is where a declared
/// capability was silently discarded.
#[tokio::test]
async fn a_provider_that_declares_a_live_restore_is_not_refused_for_its_name() {
    for name in ["zfs", "lvm", "windows_restore", "apfs", "bcachefs"] {
        let r = restorer(Box::new(DeclaresLiveRestore { name }));
        let snap = snapshot_of(name);
        match r.restore_preamble(&snap).await {
            Ok(_) => {}
            Err(e) => panic!(
                "`{}` declares it can restore a running system and the restore path refused it \
                 before the confirmation: {e}\n\
                 U27 ruled providers are rows, not a list of names.",
                name
            ),
        }
    }
}

/// The other half, and it is the one that says *why* the first one is allowed to pass: a provider
/// with no readable snapshot tree gets no package summary, and that is a missing courtesy rather
/// than a missing capability. If this ever starts returning a diff, the test above is passing for
/// a reason that has nothing to do with the fix.
#[tokio::test]
async fn a_provider_with_no_readable_snapshot_tree_yields_no_summary_rather_than_an_error() {
    let r = restorer(Box::new(DeclaresLiveRestore { name: "zfs" }));
    let summary = r
        .restore_preamble(&snapshot_of("zfs"))
        .await
        .expect("a provider with no snapshot tree must not be an error");
    assert!(
        summary.is_none(),
        "zfs exposes no mounted snapshot tree to diff, so the summary must be absent — a summary \
         here would mean the preamble read a path from somewhere it was not given"
    );
}

/// The shipped rows, read from the file rather than retyped here. Every one that declares a live
/// restore has to be reachable — this is the assertion that fails when a *new* built-in row is
/// added with a capability the command cannot honour.
#[test]
fn every_shipped_row_that_declares_a_live_restore_names_a_restore_command() {
    let file: shall::core::snapshot::SnapshotProviderFile =
        toml::from_str(include_str!("../src/core/snapshot_builtins.toml"))
            .expect("the shipped provider rows parse");
    let mut live = 0;
    for def in &file.snapshot {
        if !def.restores_running_system {
            // Create-only is the safe default, and the refusal has to say how to restore by
            // hand — a blank refusal is the V.60 footgun with better manners.
            assert!(
                def.restore_how.is_some(),
                "`{}` is create-only and says nothing about how to restore it by hand",
                def.name
            );
            continue;
        }
        live += 1;
        assert!(
            !def.restore.is_empty(),
            "`{}` declares it restores a running system and names no restore command",
            def.name
        );
    }
    assert!(
        live >= 2,
        "the shipped rows should include at least zfs and Windows System Restore as live \
         restorers; found {live} — if that changed, this file's premise did too"
    );
}
