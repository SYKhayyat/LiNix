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
use shall::core::{
    BatchRecovery, ContinuePast, GraphAction, PackageSpec, Transaction, TransactionConfig,
};
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

// ---------------------------------------------------------------------------------------------
// `M3`: what a batch does after its command fails for a passing reason.
// ---------------------------------------------------------------------------------------------

/// Four packages on one manager, so `batches` puts them on one command line (II.19).
fn quartet(recovery: BatchRecovery) -> TransactionConfig {
    TransactionConfig {
        batch_recovery: recovery,
        ..config(ContinuePast::ClassifiedPassing)
    }
}

async fn ask_the_quartet(
    kernel: &TestKernel,
    backend: Arc<RecordingBackend>,
    recovery: BatchRecovery,
) -> Vec<String> {
    let mut registry = BackendRegistry::new();
    registry.register(capabilities(&backend));
    let mut graph = StableDiGraph::new();
    for name in ["a", "b", "c", "d"] {
        graph.add_node(install(name, DRIFTED));
    }
    let mut tx = Transaction::with_config(
        graph,
        Arc::new(registry),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        quartet(recovery),
    );
    let _ = tx.execute_with_telemetry().await;
    backend.calls()
}

/// The point of `M3`, as a command log.
///
/// `b` is the only bad member. The first command carries all four and fails, because a manager
/// fails a command line as a unit. Bisection then finds `b` in three more commands and leaves
/// `a`, `c` and `d` installed - three packages that the old behaviour lost for that run over a
/// name that was never theirs.
#[tokio::test]
async fn bisection_finds_the_one_bad_member_and_installs_the_rest() {
    let kernel = TestKernel::new().await;
    let backend = RecordingBackend::named(DRIFTED, &shared_log())
        .always_flaky("b")
        .build();

    let calls = ask_the_quartet(&kernel, backend, BatchRecovery::Bisect).await;

    assert_eq!(
        calls,
        vec![
            "mock-drifted install a b c d".to_string(),
            "mock-drifted install a b".to_string(),
            "mock-drifted install c d".to_string(),
            "mock-drifted install a".to_string(),
            "mock-drifted install b".to_string(),
        ],
        "the halves are asked in order and only a FAILING half is opened further: `c d` \
         succeeded and was never split, and `b` was not asked twice"
    );
}

/// **The stopping rule, which is what makes this affordable on the case it is named after.**
///
/// Every package fails, as they do when a registry rotates a signing key. One bad member can
/// only be in one half, so two failing halves is the manager - and the answer to every further
/// question is already in hand. Three commands, not five, and on thirty packages it is three
/// rather than sixty.
#[tokio::test]
async fn two_failing_halves_is_the_manager_and_narrowing_stops_dead() {
    let kernel = TestKernel::new().await;
    let mut b = RecordingBackend::named(DRIFTED, &shared_log());
    for name in ["a", "b", "c", "d"] {
        b = b.always_flaky(name);
    }

    let calls = ask_the_quartet(&kernel, b.build(), BatchRecovery::Bisect).await;

    assert_eq!(
        calls,
        vec![
            "mock-drifted install a b c d".to_string(),
            "mock-drifted install a b".to_string(),
            "mock-drifted install c d".to_string(),
        ],
        "both halves failed, so narrowing learned everything it was going to and stopped. If \
         this list ever grows, the manager-wide case is paying the full split to be told what \
         its first command already said"
    );
}

/// `off` is the behaviour every run had before `M3`, kept as a setting for anyone who measures
/// the narrowing costing them more than it returns.
#[tokio::test]
async fn off_asks_once_and_the_batch_fails_as_a_unit() {
    let kernel = TestKernel::new().await;
    let backend = RecordingBackend::named(DRIFTED, &shared_log())
        .always_flaky("b")
        .build();

    let calls = ask_the_quartet(&kernel, backend, BatchRecovery::Off).await;

    assert_eq!(
        calls,
        vec!["mock-drifted install a b c d".to_string()],
        "`off` means one command, and `a`, `c` and `d` wait for the next run"
    );
}

/// `every` is the thorough answer and the expensive one: it asks about each member whatever the
/// halves would have said. Five commands here against bisection's five - and on a manager-wide
/// failure it is thirty-one against three, which is the trade the setting exists to let somebody
/// make deliberately.
#[tokio::test]
async fn every_asks_once_per_member_whatever_the_halves_would_have_said() {
    let kernel = TestKernel::new().await;
    let backend = RecordingBackend::named(DRIFTED, &shared_log())
        .always_flaky("b")
        .build();

    let calls = ask_the_quartet(&kernel, backend, BatchRecovery::Every).await;

    assert_eq!(
        calls,
        vec![
            "mock-drifted install a b c d".to_string(),
            "mock-drifted install a".to_string(),
            "mock-drifted install b".to_string(),
            "mock-drifted install c".to_string(),
            "mock-drifted install d".to_string(),
        ],
        "`every` is one command per member, in order, with no halves"
    );
}

/// **Narrowing is not free and must not fire where it cannot pay.**
///
/// A run configured all-or-nothing means it. Narrowing there would install the good members of a
/// batch on a machine whose owner asked for a plan that either lands or does not - and it would
/// spend the commands to do it on a transaction that is about to end anyway.
#[tokio::test]
async fn all_or_nothing_does_not_narrow_however_the_recovery_is_set() {
    let kernel = TestKernel::new().await;
    let backend = RecordingBackend::named(DRIFTED, &shared_log())
        .always_flaky("b")
        .build();

    let mut registry = BackendRegistry::new();
    registry.register(capabilities(&backend));
    let mut graph = StableDiGraph::new();
    for name in ["a", "b", "c", "d"] {
        graph.add_node(install(name, DRIFTED));
    }
    let mut tx = Transaction::with_config(
        graph,
        Arc::new(registry),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        TransactionConfig {
            batch_recovery: BatchRecovery::Bisect,
            ..config(ContinuePast::Nothing)
        },
    );
    let _ = tx.execute_with_telemetry().await;

    assert_eq!(
        backend.calls(),
        vec!["mock-drifted install a b c d".to_string()],
        "the run was going to end at this failure, so narrowing it would spend commands filling \
         in a report nobody reaches - and install packages the owner asked not to have unless \
         all of them landed"
    );
}
