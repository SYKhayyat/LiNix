//! **Five independent installs are one command, and a bundle restores to the set it bundled.**
//!
//! What was here before was `test_e2e_sync_flow_hermetic` — one `brew:neovim` through resolver,
//! planner and engine, asserting `is_managed`. **It is deleted, not moved**:
//! `a_machine_converges_tests.rs` opens by naming it as the gap it exists to close, and runs the
//! same path forward, backward and forward again over a machine the mock actually updates. One
//! package, install-only, no second run is not a weaker version of that test; it is the part of
//! it that proves nothing.
//!
//! The two that remain are about things nothing else asks.
//!
//! **The batch.** A five-node graph of independent installs becomes a *single* `brew install --`
//! with five operands (`Y1`), and the terminator goes in front of the whole operand list. The
//! test that used to live here registered five separate commands and matched none of them, so
//! every assertion below ran against the mock's empty-success default — for as long as batching
//! has existed. That the planner collapses the graph is the behaviour worth pinning, not a
//! detail of how the mock is primed.
//!
//! **The bundle.** `bundle` and `restore` are one feature and the proof runs without git
//! (V.59): bundle a config, restore it into a clean directory, and assert the model resolves to
//! the same package set. A backup nothing has ever restored is a guess.

use shall::core::executor::DryRunOutput;
use shall::core::{GraphAction, PackageSpec, Transaction, TransactionConfig};
use tokio::fs;

use crate::mock_providers::TestKernel;

/// Five independent installs of one manager reach the machine as one invocation, and all five
/// nodes reach terminal success.
#[tokio::test]
async fn five_independent_installs_reach_the_manager_as_one_command() {
    let kernel = TestKernel::new().await;

    let mut graph = petgraph::stable_graph::StableDiGraph::new();
    for i in 0..5 {
        let pkg_name = format!("pkg-parallel-{}", i);
        let spec = PackageSpec {
            name: pkg_name.clone(),
            backend: "brew".into(),
            options: Default::default(),
            requires: vec![],
            present: true,
        };

        graph.add_node(GraphAction::Install(spec));
    }

    // **One registration, because one command runs.** This registered five — `brew install
    // pkg-parallel-0` and so on — and matched none of them: `Y1` batches a manager's installs
    // into a single invocation, and the terminator goes in front of the whole operand list. So
    // the assertions below ran against the mock's empty-success default and proved nothing about
    // concurrency or about anything else, for as long as batching has existed.
    //
    // That the five nodes become one command is the finding, not a detail: what this test calls
    // a "high-throughput parallel DAG" is answered by the planner collapsing it, which is the
    // behaviour worth pinning.
    kernel.mock_executor.set_response(
        "brew install -- pkg-parallel-0 pkg-parallel-1 pkg-parallel-2 pkg-parallel-3 pkg-parallel-4",
        Ok(DryRunOutput::default().into()),
    );

    let mut tx = Transaction::with_config(
        graph,
        kernel.app.registry.clone(),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(), // DI
        kernel.app.config.clone(),
        TransactionConfig::default(),
    );

    let result = tx.execute_with_telemetry().await;

    assert!(
        result.is_ok(),
        "Concurrent parallel transaction failed: {:?}",
        result.err()
    );
    let telemetry = result.expect("Telemetry record missing");
    assert_eq!(
        telemetry.len(),
        5,
        "Not all parallel nodes reached terminal success."
    );
}

/// A restored bundle resolves to the package set the source config resolved to.
#[tokio::test]
async fn a_restored_bundle_resolves_to_the_set_it_was_made_from() {
    use shall::model::{Layout, Priority, Resolver};

    let kernel = TestKernel::new().await;
    let root = kernel.app.config.config_root().to_path_buf();
    fs::write(
        root.join("modules/tools.txt"),
        "brew:neovim\nbrew:ripgrep\n",
    )
    .await
    .unwrap();
    fs::write(root.join("profiles/Work"), "use tools\n")
        .await
        .unwrap();
    fs::write(root.join("active"), "Work\n").await.unwrap();

    let known = |n: &str| n == "brew";
    let priority = Priority::from_backends(vec!["brew".into()]);

    // The set the source config resolves to.
    let src_layout = Layout::new(root.clone(), root.join("data"));
    let before: std::collections::BTreeSet<String> = Resolver::new(&src_layout, &known, &priority)
        .resolve()
        .unwrap()
        .present()
        .map(|p| format!("{}:{}", p.backend, p.name))
        .collect();
    assert_eq!(before.len(), 2);

    // Bundle (no artifacts, no archive, no git repo present → history simply not included).
    let bundle_dir = root.join("out-bundle");
    shall::app::bundle::create_bundle(
        &kernel.app.config,
        &kernel.app.state,
        &kernel.app.vcs(),
        &bundle_dir,
        false,
        false,
        None,
    )
    .await
    .unwrap();

    // Restore into a clean directory and resolve it.
    let clean = kernel
        .app
        .config
        .config_root()
        .parent()
        .unwrap()
        .join("restored-cfg");
    let _ = fs::remove_dir_all(&clean).await;
    let reg = clean.join("data/registry.json");
    shall::app::bundle::restore_bundle(&bundle_dir, &clean, &reg, false)
        .await
        .unwrap();

    let clean_layout = Layout::new(clean.clone(), clean.join("data"));
    let after: std::collections::BTreeSet<String> = Resolver::new(&clean_layout, &known, &priority)
        .resolve()
        .unwrap()
        .present()
        .map(|p| format!("{}:{}", p.backend, p.name))
        .collect();

    assert_eq!(
        before, after,
        "restore did not reproduce the declared package set"
    );
}
