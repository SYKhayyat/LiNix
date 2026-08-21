//! One ecosystem going down upstream must not leave the rest of the machine unconverged.
//!
//! `Y15` settled the neighbouring case in August: a line pinned to a manager this host does not
//! have is not a broken config, it is the half of the config that belongs to a different
//! machine, so it is skipped and the run succeeds. It then drew the line at *a package that
//! fails still fails the command* - with two categories available, because in August every
//! failure of the third kind arrived as `Retryability::Unknown` and there was nothing to key on.
//!
//! There is now. On 2026-08-21 Hackage rotated its TUF root past the trust anchors compiled into
//! the cabal-install Ubuntu ships. Nothing about the user's config was wrong, nothing they could
//! type would fix it, and nothing about tomorrow's run is different - and under the old rule one
//! such line stopped every declaration the planner had not yet dispatched from being attempted
//! at all.
//!
//! **The mode is not `--keep-going` under another name**, and the last two tests are what say so:
//! a `Permanent` failure still ends the run, and the limit of what carrying on buys is measured
//! here rather than assumed.

use petgraph::stable_graph::StableDiGraph;
use shall::backends::BackendRegistry;
use shall::core::{ContinuePast, GraphAction, PackageSpec, Transaction, TransactionConfig};
use std::sync::Arc;
use std::time::Duration;

use crate::mock_providers::recording_backend::{
    capabilities, shared_log, CallLog, RecordingBackend,
};
use crate::mock_providers::TestKernel;

/// The manager whose ecosystem is down, and the one that is fine. TWO, because one is not enough
/// to state the claim: packages heading for the same manager in one wave share a command line
/// (II.19), so a single-backend graph measures batching rather than scheduling. The first draft
/// of this file used one, and asserted something that was not true.
const DRIFTED: &str = "mock-drifted";
const HEALTHY: &str = "mock-healthy";

fn install(name: &str, backend: &str) -> GraphAction {
    GraphAction::Install(PackageSpec {
        name: name.into(),
        backend: backend.into(),
        options: Default::default(),
        requires: vec![],
        present: true,
    })
}

/// **`max_retries: 0` is load-bearing, not tidiness.** `flaky_once` fails the first attempt with
/// `Retryability::Transient` and succeeds on the next, so with retries enabled this graph goes
/// green and measures nothing. Zero retries is how a transient failure is made to *stay* one -
/// which is the state a rotated signing key leaves a machine in until somebody fixes it upstream.
fn config(mode: ContinuePast) -> TransactionConfig {
    TransactionConfig {
        max_concurrent: 4,
        max_retries: 0,
        auto_rollback: false,
        continue_past: mode,
        node_timeout: Duration::from_secs(10),
        total_timeout: Duration::from_secs(20),
        ..TransactionConfig::default()
    }
}

fn flaky(log: &CallLog) -> Arc<RecordingBackend> {
    RecordingBackend::named(DRIFTED, log)
        .flaky_once("pkg-doomed")
        .build()
}

/// One doomed package on the drifted manager, one ordinary package on the healthy one - the
/// shape of a config with a single `cabal:` line among two hundred declarations.
///
/// Returns the healthy manager, because what it was asked to do IS the finding.
async fn run(
    kernel: &TestKernel,
    doomed: Arc<RecordingBackend>,
    mode: ContinuePast,
) -> (Arc<RecordingBackend>, Result<usize, shall::core::Error>) {
    let healthy = RecordingBackend::named(HEALTHY, &shared_log()).build();
    let mut registry = BackendRegistry::new();
    registry.register(capabilities(&doomed));
    registry.register(capabilities(&healthy));

    let mut graph = StableDiGraph::new();
    graph.add_node(install("pkg-doomed", DRIFTED));
    graph.add_node(install("pkg-ordinary", HEALTHY));

    let mut tx = Transaction::with_config(
        graph,
        Arc::new(registry),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        config(mode),
    );
    let out = tx.execute_with_telemetry().await.map(|r| r.len());
    (healthy, out)
}

/// The finding, and the reason `M2` exists.
#[tokio::test]
async fn a_failure_shall_classed_as_passing_does_not_strand_another_manager() {
    let kernel = TestKernel::new().await;
    let log = shared_log();

    let (healthy, out) = run(&kernel, flaky(&log), ContinuePast::ClassifiedPassing).await;
    let accounted = out.expect(
        "a transient failure must not end the transaction: the run finishes what it can, and \
         the caller raises the failure after the summary (`G1`)",
    );

    assert!(
        healthy
            .calls()
            .iter()
            .any(|c| c.contains("install pkg-ordinary")),
        "the healthy manager was never even asked, which is the whole bug: one drifted \
         ecosystem left the rest of the machine unconverged.\n  asked: {:?}",
        healthy.calls()
    );
    assert_eq!(
        accounted, 2,
        "both nodes have to be accounted for - a run that carries on still reports what it \
         could not do, or `sync` has nothing to exit non-zero about"
    );
}

/// The same graph with the key off, which is what `[sync] continue_past_transient = false` buys
/// somebody who wants a plan to be all-or-nothing.
#[tokio::test]
async fn turning_the_key_off_restores_all_or_nothing() {
    let kernel = TestKernel::new().await;
    let log = shared_log();

    let (_, out) = run(&kernel, flaky(&log), ContinuePast::Nothing).await;
    let err = out.expect_err("with the key off the first failure ends the transaction, as always");
    assert!(
        format!("{err}").contains("pkg-doomed"),
        "the failure that stopped the run has to name itself: {err}"
    );
}

/// **The cell that keeps the mode honest.** A `Permanent` failure is not a window that moves; it
/// says this plan cannot be applied, and the rest of the plan is built on it. If this ever passes
/// by carrying on, the mode has quietly become `--keep-going` for everybody, turned on by default
/// - which is the destructive default `--keep-going` was denied a file key to avoid.
#[tokio::test]
async fn a_permanent_failure_still_ends_the_run_under_the_same_key() {
    let kernel = TestKernel::new().await;
    let log = shared_log();
    let doomed = RecordingBackend::named(DRIFTED, &log)
        .failing("pkg-doomed")
        .build();

    let (_, out) = run(&kernel, doomed, ContinuePast::ClassifiedPassing).await;
    let err = out.expect_err(
        "`ClassifiedPassing` reads the classification. A Permanent failure is about the request, \
         and carrying on past it half-applies a plan already known to be wrong",
    );
    assert!(
        format!("{err}").contains("pkg-doomed"),
        "the failure that stopped the run has to name itself: {err}"
    );
}

/// **The limit of what this buys, measured rather than assumed - and it is not small.**
///
/// Packages heading for one manager in one wave share a command line (II.19), so a batch fails as
/// a unit. Carrying on past that failure rescues every OTHER manager's packages and none of the
/// batch's own: the twenty-nine that would have installed fine are not installed this run, and
/// wait for the next one.
///
/// `--keep-going` does not have this limit, because `G1` cuts its batch to one package per
/// command - deliberately, since a name no repository carries is a fact about one member. That is
/// not free, and making it the default would undo II.19 for every sync on every machine.
///
/// Written down as a test because a bounded claim has to state its bound. This is the shape of
/// run where the mode does less than the sentence describing it suggests, and a reader should
/// meet it here rather than on a machine.
#[tokio::test]
async fn a_batch_still_fails_as_one_and_that_is_the_documented_cost() {
    let kernel = TestKernel::new().await;
    let log = shared_log();
    let doomed = flaky(&log);

    let mut registry = BackendRegistry::new();
    registry.register(capabilities(&doomed));
    let mut graph = StableDiGraph::new();
    graph.add_node(install("pkg-doomed", DRIFTED));
    graph.add_node(install("pkg-beside-it", DRIFTED));

    let mut tx = Transaction::with_config(
        graph,
        Arc::new(registry),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        config(ContinuePast::ClassifiedPassing),
    );
    tx.execute_with_telemetry()
        .await
        .expect("the run still carries on: it is the batch that failed, not the transaction");

    assert_eq!(
        doomed.calls(),
        vec!["mock-drifted install pkg-doomed pkg-beside-it".to_string()],
        "one command for the pair is II.19 working as designed. If this ever becomes two \
         commands the batch has been split and the cost documented above is gone - which would \
         be an improvement, and this assertion is where somebody has to say so on purpose"
    );
}

/// The rule as a table, lifted out of `core::transaction` when that file reached the 3,000-line
/// gate. It belongs here anyway: the three tests above drive the same rule through the engine,
/// and this is the same rule asked directly.
///
/// Six cells - three modes against two answers to "was every failure in this round one Shall
/// classed as passing" - and the interesting one is `ClassifiedPassing` with something else in
/// the round, which STOPS. Without that cell the mode is `--keep-going` under a longer name.
#[test]
fn carrying_on_reads_the_classification_and_not_merely_the_mode() {
    for every_passing in [true, false] {
        assert!(
            ContinuePast::AnyFailure.carries_on(every_passing),
            "`--keep-going` carries on past anything, which is what it is for"
        );
        assert!(
            !ContinuePast::Nothing.carries_on(every_passing),
            "all-or-nothing stops at the first failure, whatever it was"
        );
    }
    assert!(
        ContinuePast::ClassifiedPassing.carries_on(true),
        "a round of failures Shall classed as passing is not a reason to strand the rest of \
         the plan"
    );
    assert!(
        !ContinuePast::ClassifiedPassing.carries_on(false),
        "a Permanent or an unclassified failure says the plan itself is wrong, and the rest of \
         the plan is built on it - this is the cell that keeps the mode honest"
    );
}
