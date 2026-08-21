//! One config, three machines. A line pinned to a manager this host does not have is not a
//! broken config — it is the half of the config that belongs to a different machine.
//!
//! The grammar learned this already: `app/vocab.rs` folds `priority` into the backend
//! vocabulary *specifically* so `apt:curl` parses on Windows, and says so in its own header.
//! Nothing downstream was told. `spec_is_missing` turned an unregistered backend into
//! `BackendNotFound` and failed the whole plan, so a shared `modules/` file with an `apt:` line
//! and a `winget:` line could not sync on either machine.
//!
//! The other half of the rule is here too, because a rule with only its permissive half is how
//! `AU1` starts: a package that *fails* still fails the command. Absence is a property of the
//! machine; a failed install is a property of the run.

use shall::app::sync::planner::{ChangePlanner, HostBackends, PlanScope};
use shall::app::sync::resolver::StateResolver;
use shall::core::ManagedPackage;
use tokio::fs;

use crate::mock_providers::TestKernel;

/// A manager that is in `priority`, is a name Shall knows, and is not on this host.
///
/// `zypper` is registered only on Linux, so on Windows this exercises the "no such backend in
/// the registry" branch and on Linux the "registered, binary absent" branch — which is the
/// point: **both are the same answer to the user**, and a test that only ever ran one of them
/// would pass on one CI leg while the other stayed broken.
const NOT_HERE: &str = "zypper";

async fn kernel_with(module_body: &str) -> TestKernel {
    let kernel = TestKernel::new().await;
    let root = kernel.app.config.config_root();
    fs::write(root.join("priority"), "apt\nbrew\ncargo\nzypper\n")
        .await
        .unwrap();
    fs::write(root.join("modules/travelling.txt"), module_body)
        .await
        .unwrap();
    fs::write(root.join("profiles/Main"), "use travelling\n")
        .await
        .unwrap();
    fs::write(root.join("active"), "Main\n").await.unwrap();
    // Whatever the host really is, the manager under test is not on it.
    kernel.mock_executor.set_command_exists(NOT_HERE, false);
    kernel
}

async fn plan_of(kernel: &TestKernel) -> shall::app::sync::planner::SyncChanges {
    let resolver = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), false).await;
    let desired = resolver
        .resolve_desired_state()
        .await
        .expect("the config resolves");
    let state_guard = kernel.state.lock().await;
    ChangePlanner::new(
        kernel.app.registry.clone(),
        &state_guard,
        &kernel.app.config,
    )
    .plan(&desired, PlanScope::Whole(HostBackends::default()))
    .await
    .expect("a manager this machine lacks must not fail the plan")
}

#[tokio::test]
async fn a_pin_to_a_manager_this_machine_lacks_is_skipped_not_failed() {
    let kernel = kernel_with(&format!("{}:jq\n", NOT_HERE)).await;
    let changes = plan_of(&kernel).await;

    assert_eq!(
        changes.total_install(),
        0,
        "nothing can be installed through a manager that is not here"
    );
    assert!(
        changes.skipped.iter().any(|s| s.key.contains("jq")),
        "the line has to appear in `skipped` — a plan that drops it in silence reports \
         success over a machine it did not change (AU1). Got: {:?}",
        changes.skipped
    );
    assert!(
        changes
            .skipped
            .iter()
            .any(|s| s.reason.contains(NOT_HERE) && s.reason.contains("machine")),
        "the reason has to name the manager and say it is the machine that lacks it, so the \
         user can tell this from a typo. Got: {:?}",
        changes.skipped
    );
}

#[tokio::test]
async fn the_half_of_the_config_this_machine_can_do_still_happens() {
    // The whole point of the rule: one file, both lines, each machine doing its half.
    let kernel = kernel_with(&format!("brew:neovim\n{}:jq\n", NOT_HERE)).await;
    let changes = plan_of(&kernel).await;

    assert_eq!(
        changes.total_install(),
        1,
        "the manager that IS here must still install its package"
    );
    assert_eq!(changes.skipped.len(), 1, "{:?}", changes.skipped);
}

#[tokio::test]
async fn the_report_counts_a_skip_apart_from_a_change() {
    let kernel = kernel_with(&format!("brew:neovim\n{}:jq\n", NOT_HERE)).await;
    let report = plan_of(&kernel).await.generate_report();

    assert_eq!(
        report.change_count, 1,
        "a skip is not a change — counting it as one makes `1 installed` describe a machine \
         that got one package and was told about two"
    );
    assert_eq!(report.install.len(), 1);
    assert_eq!(report.skipped.len(), 1, "{:?}", report.skipped);
}

#[tokio::test]
async fn a_managed_package_whose_manager_is_gone_is_not_reaped() {
    // The removal side of the same rule. Shall installed it through zypper on the Linux box;
    // the config now travels to a machine with no zypper. Scheduling the removal would run a
    // command that cannot exist, and failing is not what the user asked for either.
    let kernel = kernel_with("brew:neovim\n").await;
    {
        let mut state = kernel.state.lock().await;
        state.manage(ManagedPackage {
            name: "jq".into(),
            backend: NOT_HERE.into(),
            version: None,
            installed_at: 0,
            expires_at: None,
            options: Default::default(),
            source: "sync".into(),
            is_transient: false,
            session_id: None,
        });
    }
    let changes = plan_of(&kernel).await;

    assert!(
        !changes.removal_tracker.iter().any(|k| k.contains("jq")),
        "a manager that is not here cannot remove anything: {:?}",
        changes.removal_tracker
    );
    assert!(
        changes
            .skipped
            .iter()
            .any(|s| s.key.contains("jq") && s.reason.contains(NOT_HERE)),
        "and the standing disagreement is still reported. Got: {:?}",
        changes.skipped
    );
}

#[tokio::test]
async fn an_absent_declaration_for_a_manager_that_is_gone_is_skipped() {
    let kernel = kernel_with(&format!("absent:{}:jq\n", NOT_HERE)).await;
    let changes = plan_of(&kernel).await;

    assert!(
        !changes.removal_tracker.iter().any(|k| k.contains("jq")),
        "`absent:` through a manager that is not here is already satisfied: {:?}",
        changes.removal_tracker
    );
}

#[tokio::test]
async fn a_package_that_fails_still_fails_the_command() {
    // The other half, and the reason this is a rule about *absence* rather than a licence to
    // continue past anything. A manager that is here and says no is a fact about this run.
    let kernel = TestKernel::new().await;
    let root = kernel.app.config.config_root();
    fs::write(root.join("modules/m.txt"), "brew:neovim\n")
        .await
        .unwrap();
    fs::write(root.join("profiles/Main"), "use m\n")
        .await
        .unwrap();

    let changes = plan_of(&kernel).await;
    assert_eq!(changes.total_install(), 1);

    kernel.mock_executor.set_response(
        "brew install -- neovim",
        Err(shall::core::Error::Other("no such formula".into())),
    );

    let engine = kernel.app.sync_engine();
    let result = engine
        .sync(changes, shall::app::sync::guard::GuardScope::Sync)
        .await;

    assert!(
        result.is_err(),
        "a package that could not be installed must fail the command — warning and exiting 0 \
         over a machine that did not get the package is exactly what AU1 bans"
    );
}

#[tokio::test]
async fn only_the_flag_carries_on_past_a_failure_about_the_config() {
    use shall::core::{ContinuePast, TransactionConfig};
    let kernel = TestKernel::new().await;
    assert_eq!(
        TransactionConfig::default().continue_past,
        ContinuePast::Nothing,
        "the library default stays all-or-nothing; what a real run does is `from_config`'s to \
         say, and recovery builds its own"
    );

    // **`M2` moved this line and did not erase it.** A stock machine now carries on past a
    // failure Shall itself classified as passing — a rotated key, a held lock — because that
    // is not a fact about the config. It still stops at everything else, which is the half
    // `Y15` ruled on and the half `AU1` is about: a package that fails still fails the command.
    let mut config = (*kernel.app.config).clone();
    assert_eq!(
        TransactionConfig::from_config(&config).continue_past,
        ContinuePast::ClassifiedPassing,
        "the default is to finish what it can past a passing failure"
    );
    assert!(
        !TransactionConfig::from_config(&config)
            .continue_past
            .carries_on(false),
        "and NOT past a failure nothing classified — otherwise the key is `--keep-going` for \
         everybody, which is exactly what nobody typed"
    );
    config.keep_going_this_run = true;
    assert_eq!(
        TransactionConfig::from_config(&config).continue_past,
        ContinuePast::AnyFailure,
        "`--keep-going` has to reach the transaction, or the flag is decoration"
    );
}
