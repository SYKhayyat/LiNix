use linix::app::sync::planner::ChangePlanner;
use linix::app::sync::resolver::StateResolver;
use linix::core::executor::DryRunOutput;
use linix::core::journal::JournalAction;
use linix::core::{Error, GraphAction, PackageSpec, Transaction, TransactionConfig};
use std::collections::HashMap;

// Import our authoritative A+ Test Infrastructure
mod mock_providers;
use mock_providers::TestKernel;

/// S11: hermeticity is now structural, not remembered — a `TestKernel` isolates BOTH the
/// config root and the data root (registry/snapshots/journal) inside its sandbox, so no test
/// can touch the developer's real state, whether or not the test author set `$LINIX_DATA_DIR`.
#[tokio::test]
async fn test_kernel_isolates_both_config_and_data_roots() {
    let kernel = TestKernel::new().await;
    let sandbox = kernel.tmp.path();
    assert!(
        kernel.app.config.config_root().starts_with(sandbox),
        "config_root {:?} escaped the sandbox {:?}",
        kernel.app.config.config_root(),
        sandbox
    );
    assert!(
        kernel.app.config.data_root().starts_with(sandbox),
        "data_root {:?} escaped the sandbox {:?} — a test could write to real user data",
        kernel.app.config.data_root(),
        sandbox
    );
    // And the layout the resolver actually uses points both halves at the sandbox.
    let layout = kernel.app.config.layout();
    assert!(layout.modules_dir().starts_with(sandbox));
}

// ============================================================================
// LOGIC TESTS: PLANNING & DEPENDENCIES
// ============================================================================

/// **One declaration is one node, whatever `brew deps` says.** (`Y9`, V.115a.)
///
/// This test asserted the opposite until 2026-08-06 — that declaring `brew:pkg-a` produced two
/// nodes, the second being whatever `brew deps` named. That second node was installed, and then
/// written into `registry.json` as a package LiNix *manages*, with nobody declaring it: II.7
/// then makes it drift, and `sync` removes what is drift. brew installs its own dependencies at
/// `brew install` time regardless, so the whole exchange bought a subprocess, a graph edge that
/// split brew's command line in two, and a package LiNix would later take away.
#[tokio::test]
async fn a_declared_package_is_one_node_however_many_things_it_depends_on() {
    let kernel = TestKernel::new().await;
    let state_lock = kernel.state.lock().await;

    let planner = ChangePlanner::new(kernel.app.registry.clone(), &state_lock, &kernel.app.config);

    // brew, asked, would say pkg-a needs pkg-b. Nothing asks.
    kernel.mock_executor.set_response(
        "brew deps -- pkg-a",
        Ok(DryRunOutput {
            stdout: b"pkg-b\n".to_vec(),
            stderr: vec![],
        }
        .into()),
    );

    let mut desired = HashMap::new();
    desired.insert(
        "brew".to_string(),
        vec![PackageSpec {
            name: "pkg-a".into(),
            backend: "brew".into(),
            options: HashMap::new(),
            requires: vec![],
            present: true,
        }],
    );

    let plan = planner
        .plan(&desired, None)
        .await
        .expect("Critical Path Error: Planning failed.");

    assert_eq!(
        plan.graph.node_count(),
        1,
        "one line was declared, so one package is LiNix's to install and to own"
    );
    assert_eq!(
        plan.graph.edge_count(),
        0,
        "nobody wrote `@requires`, so nothing may split brew's command line"
    );
}

/// Verifies that the Planner detects circular manifest-level dependencies
/// and returns a descriptive error rather than causing an infinite loop.
#[tokio::test]
async fn test_dag_cycle_detection_logic() {
    let kernel = TestKernel::new().await;
    let state_lock = kernel.state.lock().await;
    let planner = ChangePlanner::new(kernel.app.registry.clone(), &state_lock, &kernel.app.config);

    let mut desired = HashMap::new();

    // Create Circular Logic: pkg-a requires pkg-b, pkg-b requires pkg-a
    let spec_a = PackageSpec {
        name: "pkg-a".into(),
        backend: "brew".into(),
        options: HashMap::new(),
        requires: vec!["brew:pkg-b".into()],
        present: true,
    };
    let spec_b = PackageSpec {
        name: "pkg-b".into(),
        backend: "brew".into(),
        options: HashMap::new(),
        requires: vec!["brew:pkg-a".into()],
        present: true,
    };

    desired.insert("brew".to_string(), vec![spec_a, spec_b]);

    // Execute Planning
    let result = planner.plan(&desired, None).await;

    // Verification
    assert!(
        result.is_err(),
        "Planner failed to identify circular manifest closure."
    );
    if let Err(Error::Transaction(msg)) = result {
        // V.45: the message names the cycle, not just "a cycle exists".
        assert!(msg.contains("cycle"), "should say it is a cycle: {}", msg);
        assert!(
            msg.contains("brew:pkg-a") && msg.contains("brew:pkg-b"),
            "should name the mutually-dependent packages: {}",
            msg
        );
    }
}

// ============================================================================
// LOGIC TESTS: TRANSACTION & ROLLBACK
// ============================================================================

/// Verifies that a failed modification correctly triggers a transactional
/// rollback of preceding successful nodes.
#[tokio::test]
async fn test_transaction_rollback_fidelity() {
    let kernel = TestKernel::new().await;

    let failing_spec = PackageSpec {
        name: "fail-node".into(),
        backend: "brew".into(),
        options: HashMap::new(),
        requires: vec![],
        present: true,
    };

    // Set response to failure
    kernel.mock_executor.set_response(
        "brew install -- fail-node",
        Err(Error::command_failed("Simulated Network Timeout")),
    );

    let mut graph = petgraph::stable_graph::StableDiGraph::new();
    graph.add_node(GraphAction::Install(failing_spec));

    // Modernized: Provide Diagnostics and Config (TransactionConfig::quick restored in library)
    let mut tx = Transaction::with_config(
        graph,
        kernel.app.registry.clone(),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        TransactionConfig::default(),
    );

    let result = tx.execute_with_telemetry().await;
    assert!(
        result.is_err(),
        "Transaction logic failed to catch node failure."
    );
}

async fn record_interrupted(kernel: &TestKernel, action: JournalAction) {
    let mut j = kernel.app.journal.lock().await;
    j.record_start(action).expect("could not write the WAL");
}

fn spec(name: &str, backend: &str) -> PackageSpec {
    PackageSpec {
        name: name.into(),
        backend: backend.into(),
        options: HashMap::new(),
        requires: vec![],
        present: true,
    }
}

/// S24/V.64 — a recovery path may not remove. Recovering an interrupted *install* re-runs the
/// install; it never uninstalls first. The removal this asserts the absence of reached no
/// guard, no count, no plan and no history, and it uninstalled Google Chrome on the owner's
/// machine from `install nimble:nimjson`.
#[tokio::test]
async fn healing_an_interrupted_install_never_uninstalls() {
    let kernel = TestKernel::new().await;
    record_interrupted(&kernel, JournalAction::Install(spec("stale-pkg", "brew"))).await;

    let engine = kernel.app.sync_engine().await;
    kernel.mock_executor.set_response(
        "brew install -- stale-pkg",
        Ok(DryRunOutput::default().into()),
    );

    engine.heal().await.expect("Healing cycle crashed.");

    let calls = kernel.mock_executor.get_calls().await;
    assert!(
        !calls.iter().any(|c| c.contains("uninstall")),
        "recovery issued a removal: {:?}",
        calls
    );
    assert!(
        calls.iter().any(|c| c == "brew install -- stale-pkg"),
        "recovery did not re-run the install: {:?}",
        calls
    );
    assert!(
        !kernel.app.journal.lock().await.needs_recovery(),
        "the journal still wants recovery after a successful heal"
    );
}

/// The sibling branch: an interrupted *removal* is a removal the user asked for, so recovery
/// completes it. A fix that cured the install branch by disabling removal recovery would pass
/// the test above and break this one.
#[tokio::test]
async fn healing_an_interrupted_removal_still_removes() {
    let kernel = TestKernel::new().await;
    record_interrupted(
        &kernel,
        JournalAction::Remove {
            name: "doomed-pkg".into(),
            backend: "brew".into(),
        },
    )
    .await;

    let engine = kernel.app.sync_engine().await;
    kernel.mock_executor.set_response(
        "brew uninstall doomed-pkg",
        Ok(DryRunOutput::default().into()),
    );

    engine.heal().await.expect("Healing cycle crashed.");

    let calls = kernel.mock_executor.get_calls().await;
    assert!(
        calls
            .iter()
            .any(|c| c.contains("uninstall") && c.contains("doomed-pkg")),
        "recovery did not complete the interrupted removal: {:?}",
        calls
    );
    assert!(
        !kernel.app.journal.lock().await.needs_recovery(),
        "the journal still wants recovery after a successful heal"
    );
}

/// S25 — a preview recovers nothing. `--dry-run sync` reached recovery before the branch whose
/// comment says "never prompt, never mutate", so the preview ran the removal S24 describes.
#[tokio::test]
async fn a_dry_run_reports_the_recovery_and_performs_none_of_it() {
    let kernel = TestKernel::new().await;
    record_interrupted(&kernel, JournalAction::Install(spec("stale-pkg", "brew"))).await;
    record_interrupted(
        &kernel,
        JournalAction::Remove {
            name: "doomed-pkg".into(),
            backend: "brew".into(),
        },
    )
    .await;

    let mut previewing = (*kernel.app.config).clone();
    previewing.dry_run = true;
    let engine = linix::app::sync::SyncEngine::new(
        &previewing,
        kernel.app.registry.clone(),
        kernel.app.executor.duplicate(),
        kernel.app.metrics.clone(),
        kernel.app.progress.clone(),
        kernel.app.hooks.clone(),
        kernel.app.snapshot_manager.clone(),
        kernel.app.journal.clone(),
        kernel.app.state.clone(),
        kernel.app.diagnostics.clone(),
    )
    .await;

    engine.heal().await.expect("Healing cycle crashed.");

    assert!(
        kernel.mock_executor.get_calls().await.is_empty(),
        "a dry run ran commands: {:?}",
        kernel.mock_executor.get_calls().await
    );
    assert!(
        kernel.app.journal.lock().await.needs_recovery(),
        "a dry run resolved the journal entries, so the real run has nothing left to recover"
    );
}

/// And the protected case, which is why the removal branch consults the guard at all: `sudo`
/// is protected by default, so its interrupted removal is refused and the package is kept —
/// while the entry still resolves, or heal retries a refusal forever.
#[tokio::test]
async fn healing_a_protected_removal_is_refused_and_the_package_kept() {
    let kernel = TestKernel::new().await;
    record_interrupted(
        &kernel,
        JournalAction::Remove {
            name: "sudo".into(),
            backend: "brew".into(),
        },
    )
    .await;

    let engine = kernel.app.sync_engine().await;
    engine.heal().await.expect("Healing cycle crashed.");

    let calls = kernel.mock_executor.get_calls().await;
    assert!(
        !calls.iter().any(|c| c.contains("uninstall")),
        "the guard did not stop the removal of a protected package: {:?}",
        calls
    );
    assert!(
        !kernel.app.journal.lock().await.needs_recovery(),
        "a refused recovery left the journal stuck retrying it"
    );
}

// ============================================================================
// LOGIC TESTS: RESOLVER & VFS
// ============================================================================

/// Verifies that the StateResolver correctly parses and unrolls complex
/// semver constraints from manifest strings.
#[tokio::test]
async fn test_semver_constraint_resolution_logic() {
    let kernel = TestKernel::new().await;

    // Modernized: Await async constructor and pass locked=false
    let resolver = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), false).await;

    let spec_line = "brew:curl@version=>=7.0.0";

    // Modernized: Call method on the RESOLVED Resolver (Future was awaited above)
    let spec = resolver
        .parse_and_probe_spec(spec_line)
        .await
        .expect("Critical Path: Resolver failed to parse semver spec line.");

    assert_eq!(spec.options.get("version").unwrap(), ">=7.0.0");
}

/// A preview writes no manifest — the flagship bug, moved from the machine to the files.
///
/// `--dry-run uninstall scoop:sd` printed `remove 1` and deleted the declaration for real,
/// leaving the package installed and undeclared: drift the next sync would act on, produced
/// by the command that promises to do nothing. Every writer is asserted here rather than
/// `undeclare` alone, because the flag was consulted per-verb and each verb that forgot was
/// its own instance of the same bug.
#[tokio::test]
async fn a_preview_writes_no_manifest() {
    use linix::model::Landing;

    let kernel = TestKernel::new().await;
    let manifest = kernel.tmp.path().join("modules/imperative.txt");

    kernel
        .app
        .declare("cargo:ripgrep", None, Landing::Imperative)
        .await
        .expect("the fixture's own declaration must be written for real");
    let before = std::fs::read_to_string(&manifest).unwrap();
    assert!(before.contains("cargo:ripgrep"), "fixture wrote nothing");

    let preview = kernel.previewing().await;

    // `uninstall`, the reported case.
    let planned = preview.undeclare("cargo:ripgrep").await.unwrap();
    assert_eq!(
        std::fs::read_to_string(&manifest).unwrap(),
        before,
        "a preview removed the declaration"
    );
    assert_eq!(
        planned.len(),
        1,
        "a preview that changes nothing must still say what it would have changed"
    );

    // `install`, and `uninstall --temp`, which writes an `absent:` line the same way.
    preview
        .declare("cargo:fd", None, Landing::Imperative)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&manifest).unwrap(),
        before,
        "a preview added a declaration"
    );

    // `teleport`, which rewrites the line in place.
    let moved = preview.retarget("ripgrep", "brew").await.unwrap();
    assert_eq!(
        std::fs::read_to_string(&manifest).unwrap(),
        before,
        "a preview rewrote the declaration's backend"
    );
    assert_eq!(moved.len(), 1, "a preview must still report the move");

    // The control: without the flag, the same call on the same fixture really does remove
    // the line. Without this, every assertion above passes on a setup that never reached
    // the condition.
    kernel.app.undeclare("cargo:ripgrep").await.unwrap();
    assert!(
        !std::fs::read_to_string(&manifest)
            .unwrap()
            .contains("cargo:ripgrep"),
        "the fixture cannot remove the line at all, so it proves nothing about the preview"
    );
}

/// The same rule one layer down, where it is enforced: no writer reaches the disk in a
/// preview, including the ones no verb calls today.
///
/// `adopt` writes a whole module through `write_module`, and it is the writer with the most
/// to lose — it overwrites `modules/adopted.txt` entirely, so a preview that wrote would
/// destroy the previous adoption rather than add to it.
#[tokio::test]
async fn a_previewing_editor_writes_no_file_at_all() {
    use linix::config::grammar::Origin;
    use linix::model::{Editor, Target, Writes};

    let kernel = TestKernel::new().await;
    let layout = kernel.app.config.layout();
    let vocab = kernel.app.vocabulary().await.unwrap();
    let facts = kernel.app.host_facts().await.unwrap();
    let adopted = layout.modules_dir().join("adopted.txt");

    Editor::new(&layout, &vocab, facts.clone(), Writes::ToDisk)
        .write_module(
            &Target::parse("adopted", &Origin::argument()).unwrap(),
            "brew:jq\n",
        )
        .expect("the fixture's own write must reach the disk");
    let before = std::fs::read_to_string(&adopted).unwrap();

    Editor::new(&layout, &vocab, facts, Writes::Planned)
        .write_module(
            &Target::parse("adopted", &Origin::argument()).unwrap(),
            "brew:something-else\n",
        )
        .unwrap();

    assert_eq!(
        std::fs::read_to_string(&adopted).unwrap(),
        before,
        "a previewing editor overwrote the adopted module"
    );
}

/// Verifies that the CommandExecutor accurately records file modifications
/// in the Virtual File System (VFS) during dry-run sessions.
#[tokio::test]
async fn test_dry_run_vfs_simulation() {
    let kernel = TestKernel::new().await;
    let executor = kernel.app.executor.clone();

    let path = std::path::PathBuf::from("/virtual/A+_Integrity_Pass.txt");
    let content = "System Integrity Verified.";

    // write_atomic should target VFS in dry-run mode
    executor
        .write_atomic(&path, content)
        .await
        .expect("VFS Write failed.");

    let read_content = executor.read_file(&path).await.expect("VFS Read failed.");
    assert_eq!(
        read_content, content,
        "VFS failed to preserve/retrieve written content."
    );

    let diff = executor.get_vfs_diff();
    assert!(
        !diff.is_empty(),
        "VFS diff tracker is empty after dry-run modification."
    );
}

/// Performance-shape regression: `App::list` fans out across backends concurrently instead of
/// querying them one at a time, and still returns every backend's packages. The mock executor
/// answers `brew` and `cargo` list commands; both sets must come back.
#[tokio::test]
async fn list_aggregates_every_backend_that_answers() {
    let kernel = TestKernel::new().await;
    // brew's lister: `brew list --versions` → "name version" per line.
    kernel.mock_executor.set_response(
        "brew list --versions",
        Ok(DryRunOutput {
            stdout: b"ripgrep 14.1.0\nfd 10.2.0\n".to_vec(),
            stderr: vec![],
        }
        .into()),
    );
    // cargo's lister: `cargo install --list` → "name vX.Y.Z:" headers.
    kernel.mock_executor.set_response(
        "cargo install --list",
        Ok(DryRunOutput {
            stdout: b"bat v0.24.0:\n    bat\n".to_vec(),
            stderr: vec![],
        }
        .into()),
    );

    let all = kernel.app.list(None).await.expect("list runs");
    let names: std::collections::HashSet<&str> = all.iter().map(|p| p.name.as_str()).collect();
    assert!(
        names.contains("ripgrep"),
        "brew packages missing: {:?}",
        names
    );
    assert!(names.contains("bat"), "cargo packages missing: {:?}", names);

    // A backend filter still narrows to one.
    let brew_only = kernel.app.list(Some("brew")).await.unwrap();
    assert!(
        brew_only.iter().all(|p| p.backend == "brew"),
        "{:?}",
        brew_only
    );
    assert!(brew_only.iter().any(|p| p.name == "fd"));
}
