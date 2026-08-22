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

/// **Narrowing a batch of removals, which nothing had ever asked it to do.**
///
/// `run_one_command` hands `specs` to `install` and `names` to `remove`, and every narrowing
/// test in this file drives installs — so the `names` slice it builds was never read by anything
/// that could notice it being wrong. The mutation shard said so out loud: replacing `p + 1` with
/// `p * 1` in `narrow_batch` makes that slice **empty**, and survived, because an unused slice
/// cannot be observed to be the wrong one.
///
/// An empty name list handed to `remove` is a command that removes nothing while the run records
/// a per-member verdict saying it did — the registry and the machine then disagree, which is the
/// `S87` shape.
async fn narrow_a_removal(kernel: &TestKernel, recovery: BatchRecovery) -> Vec<String> {
    let backend = RecordingBackend::named(DRIFTED, &shared_log())
        .unremovable("c")
        .build();
    let mut registry = BackendRegistry::new();
    registry.register(capabilities(&backend));
    let mut graph = StableDiGraph::new();
    for name in ["a", "b", "c", "d"] {
        graph.add_node(GraphAction::Remove {
            name: name.into(),
            backend: DRIFTED.into(),
        });
    }
    let mut tx = Transaction::with_config(
        graph,
        Arc::new(registry),
        kernel.app.journal.clone(),
        kernel.app.diagnostics.clone(),
        kernel.app.config.clone(),
        quartet(recovery),
    )
    .guarded_by(shall::app::sync::guard::Reaped::for_reason(
        shall::app::sync::guard::GuardScope::Remove,
        "a unit test of batch narrowing over removals",
    ));
    let _ = tx.execute_with_telemetry().await;
    backend.calls()
}

/// Every command a narrowed removal issues names the packages it is removing. The assertion that
/// matters is not the shape of the bisection but that **no command is issued with an empty name
/// list** — the one thing the surviving mutant produced.
#[tokio::test]
async fn a_narrowed_removal_never_asks_the_manager_to_remove_nothing() {
    let kernel = TestKernel::new().await;

    for recovery in [BatchRecovery::Bisect, BatchRecovery::Every] {
        let calls = narrow_a_removal(&kernel, recovery).await;

        assert!(
            !calls.is_empty(),
            "{recovery:?} issued no removal command at all, so this proves nothing"
        );
        assert!(calls.len() > 1, "{recovery:?} never narrowed: {calls:?}");
        for call in &calls {
            let names = call
                .strip_prefix(&format!("{DRIFTED} remove "))
                .unwrap_or_else(|| panic!("unexpected command shape: {call}"));
            assert!(
                !names.trim().is_empty(),
                "{recovery:?} asked the manager to remove nothing: {calls:?}. A per-member \
                 command whose name list is empty removes nothing and still reports a \
                 verdict for that member."
            );
        }
    }
}

/// And the verdict itself: the one bad member is the one that fails, and the other three are
/// removed. Narrowing over removals has to earn its keep the same way it does over installs.
#[tokio::test]
async fn narrowing_a_removal_still_removes_every_good_member() {
    let kernel = TestKernel::new().await;
    let calls = narrow_a_removal(&kernel, BatchRecovery::Every).await;

    for good in ["a", "b", "d"] {
        assert!(
            calls
                .iter()
                .any(|c| c == &format!("{DRIFTED} remove {good}")),
            "`{good}` was never removed on its own, so the bad member took it down: {calls:?}"
        );
    }
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

/// **A batch of one has nothing to narrow, and a mutant found that nothing said so.**
///
/// `members > 1` became `members >= 1` and the whole suite passed. The cost is not correctness -
/// re-asking a one-package command gets the same answer - it is that the commonest failure shape
/// there is would pay for a second identical invocation. `--keep-going` produces batches of one
/// by design (`G1`), so this is not a rare corner.
#[test]
fn a_batch_of_one_is_never_narrowed() {
    let passing = shall::core::Error::CommandFailed {
        message: "`apt` could not reach its index".into(),
        retry: shall::core::Retryability::Transient,
        absent_name: false,
    };
    assert!(
        !BatchRecovery::Bisect.narrows(&passing, 1, ContinuePast::ClassifiedPassing),
        "a single-member batch was narrowed: there is nothing to tell apart, so the only thing \
         the second command can buy is the time it costs"
    );
    assert!(
        BatchRecovery::Bisect.narrows(&passing, 2, ContinuePast::ClassifiedPassing),
        "two members is the smallest batch worth splitting; if this is false the mode never fires"
    );
}

/// **The bad member in the SECOND half, which is the only way to test the midpoint.**
///
/// `mid = lo + (hi - lo) / 2` became `lo + (hi + lo) / 2` and survived, because every other test
/// here recurses into the LEFT half where `lo` is nought - and at nought the two expressions are
/// the same number. Only a recursion with `lo > 0` tells them apart.
///
/// With `c` bad: the top split clears `a b`, the right half fails, and the recursion into `(2,4)`
/// is where the arithmetic matters.
#[tokio::test]
async fn bisection_recurses_into_the_right_half_correctly() {
    let kernel = TestKernel::new().await;
    let backend = RecordingBackend::named(DRIFTED, &shared_log())
        .always_flaky("c")
        .build();

    let calls = ask_the_quartet(&kernel, backend, BatchRecovery::Bisect).await;

    assert_eq!(
        calls,
        vec![
            "mock-drifted install a b c d".to_string(),
            "mock-drifted install a b".to_string(),
            "mock-drifted install c d".to_string(),
            "mock-drifted install c".to_string(),
            "mock-drifted install d".to_string(),
        ],
        "the recursion into the right half asked the wrong packages, which is what a midpoint \
         computed as `lo + (hi + lo) / 2` does as soon as `lo` is not nought"
    );
}
