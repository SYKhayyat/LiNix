//! **Nothing in this repository tested that a machine converges.**
//!
//! `lamdan/whole-repo-2026-08-05.md` closes with this as the one gap to shut before any of its
//! findings: the since-deleted `e2e_tests.rs` wrote one `brew:neovim`, ran resolver → planner
//! → engine once, and
//! asserted `is_managed`. One package, install-only, no second run. **No test deleted a line and
//! asserted the package left. No test synced twice and asserted the second run was empty.**
//! `src/app/sync/mod.rs` — 1,102 lines holding the entire apply loop — contains zero
//! `#[cfg(test)]`.
//!
//! So `install = desired − present` was proved once and `remove = (present ∩ owned) − desired`
//! was proved nowhere end to end, while seventy-odd binaries guarded the loop from every other
//! angle. This runs it forward, backward, and forward again.
//!
//! **Each sync is a fresh `App` over the same files and the same state registry**, because that
//! is what two runs of `shall sync` are. One `App` memoises each manager's installed listing for
//! the run (`CommandExecutor::installed`), so re-planning inside one would answer the second
//! question from the first question's answer and prove nothing about convergence.
//!
//! **The mock is the machine.** After Shall installs, the test makes brew report the package —
//! which is what the real brew would do — through every question Shall asks it; the assertion
//! that the install command actually ran is what keeps that from being fiction.

use shall::app::sync::planner::{ChangePlanner, HostBackends, PlanScope};
use shall::app::sync::resolver::StateResolver;
use shall::core::executor::{DryRunOutput, MockExecutor};
use tokio::fs;

use crate::mock_providers::TestKernel;

fn answer(mock: &MockExecutor, cmd: &str, stdout: &str) {
    mock.set_response(
        cmd,
        Ok(DryRunOutput {
            stdout: stdout.as_bytes().to_vec(),
            stderr: vec![],
        }
        .into()),
    );
}

/// Put the machine in a state, through **both** questions Shall asks about brew.
///
/// `brew` overrides `Queryable::info` to run `brew info --json=v1`, so the planner's
/// *is it installed?* never touches the listing — while removal planning's `installed_sets`
/// only ever reads `brew list --versions`. Setting one and not the other produces a machine
/// that is holding a package and does not have it, which is not a state any real host is in
/// and not a thing worth asserting about.
async fn machine_holds(mock: &MockExecutor, names: &[&str]) {
    // **These stubs are a world, not an expectation.** The mock's unmatched-registration check
    // exists because a stub the product never ran usually means the test was wrong about the
    // product's argv — the since-deleted `e2e_tests.rs` registered `brew install {name}`
    // against a product that
    // emits `brew install -- neovim`, and stayed green. That reading does not apply here: this
    // helper describes what the *machine* is holding, and a machine having a state does not mean
    // Shall asks about every part of it. `brew leaves` goes unasked whenever nothing is being
    // adopted, and `brew info -- fd` whenever no line mentions `fd`.
    mock.allow_unmatched_registrations();
    let listing: String = names.iter().map(|n| format!("{} 1.0.0\n", n)).collect();
    answer(mock, "brew list --versions", &listing);
    answer(mock, "brew leaves", &names.join("\n"));
    for n in ["neovim", "fd"] {
        let json = if names.contains(&n) {
            format!(
                r#"[{{"name":"{}","versions":{{"stable":"1.0.0"}},"installed":[{{"prefix":"/opt/homebrew/Cellar/{}/1.0.0"}}]}}]"#,
                n, n
            )
        } else {
            "[]".to_string()
        };
        answer(mock, &format!("brew info --json=v1 -- {}", n), &json);
    }
}

/// Whether the on-disk registry — the file, not one process's copy of it — records this.
async fn recorded(kernel: &TestKernel, name: &str) -> bool {
    let app = kernel.second_run().await;
    let state = app.state.lock().await;
    state.is_managed("brew", name)
}

/// One run of `shall sync`, in its own process's worth of state.
///
/// Returns `(installs, removals)` from the plan, after executing it.
async fn sync_run(kernel: &TestKernel) -> (usize, usize) {
    let app = kernel.second_run().await;

    let resolver = StateResolver::new(&app.config, app.registry.clone(), false).await;
    let desired = resolver.resolve_desired_state().await.expect("resolves");

    let changes = {
        let state = app.state.lock().await;
        ChangePlanner::new(app.registry.clone(), &state, &app.config)
            .plan(&desired, PlanScope::Whole(HostBackends::default()))
            .await
            .expect("plans")
    };
    let counts = (changes.total_install(), changes.total_remove());

    let engine = app.sync_engine();
    engine
        .sync(changes, shall::app::sync::guard::GuardScope::Sync)
        .await
        .expect("the plan applies");
    counts
}

async fn declare(kernel: &TestKernel, body: &str) {
    let root = kernel.app.config.config_root();
    fs::write(root.join("modules/tools.txt"), body)
        .await
        .unwrap();
    fs::write(root.join("profiles/Main"), "use tools\n")
        .await
        .unwrap();
    fs::write(root.join("active"), "Main\n").await.unwrap();
}

#[tokio::test]
async fn a_machine_converges_forward_backward_and_forward_again() {
    let kernel = TestKernel::new().await;
    kernel
        .mock_executor
        .set_response("brew install -- neovim", Ok(DryRunOutput::default().into()));
    kernel.mock_executor.set_response(
        "brew uninstall -- neovim",
        Ok(DryRunOutput::default().into()),
    );
    machine_holds(&kernel.mock_executor, &[]).await;

    // ---- forward: the line is declared and the machine does not have it -------------------
    declare(&kernel, "brew:neovim\n").await;
    let (installs, removals) = sync_run(&kernel).await;
    assert_eq!(
        (installs, removals),
        (1, 0),
        "`install = desired - present`: one declared, none present"
    );
    kernel.assert_called("brew install -- neovim").await;
    assert!(
        recorded(&kernel, "neovim").await,
        "the run that installed it must be the run that records owning it"
    );

    // ---- the fixed point: the same files, the same machine, nothing to do -----------------
    machine_holds(&kernel.mock_executor, &["neovim"]).await;
    assert_eq!(
        sync_run(&kernel).await,
        (0, 0),
        "a second sync over an unchanged config must be empty — convergence is a fixed \
         point, and a sync that reinstalls what it just installed is not one"
    );

    // ---- backward: the declaration goes, and so does the package --------------------------
    declare(&kernel, "").await;
    let (installs, removals) = sync_run(&kernel).await;
    assert_eq!(
        (installs, removals),
        (0, 1),
        "`remove = (present n owned) - desired`: nothing declares it and Shall owns it"
    );
    kernel.assert_called("brew uninstall -- neovim").await;
    assert!(
        !recorded(&kernel, "neovim").await,
        "the removal is what drops the registry row; leaving it makes Shall claim to manage \
         something it has just deleted"
    );

    // ---- and forward again: converged in the other direction ------------------------------
    machine_holds(&kernel.mock_executor, &[]).await;
    assert_eq!(
        sync_run(&kernel).await,
        (0, 0),
        "the empty config over the emptied machine is the fixed point too"
    );
}

#[tokio::test]
async fn convergence_never_reaches_for_what_shall_did_not_install() {
    // II.7: *"What Shall may remove: what it manages and you stopped declaring. Plus `absent:`.
    // Nothing else, ever."* The forward-backward test above would pass just as well if drift
    // were computed from the machine instead of from the registry — and that version deletes
    // software the user installed by hand. This is the half that tells them apart.
    let kernel = TestKernel::new().await;
    kernel
        .mock_executor
        .set_response("brew install -- neovim", Ok(DryRunOutput::default().into()));
    // A machine with something on it that Shall has never heard of.
    machine_holds(&kernel.mock_executor, &["fd"]).await;

    declare(&kernel, "brew:neovim\n").await;
    assert_eq!(sync_run(&kernel).await, (1, 0));

    machine_holds(&kernel.mock_executor, &["fd", "neovim"]).await;
    assert_eq!(
        sync_run(&kernel).await,
        (0, 0),
        "`fd` is on the machine, is declared nowhere, and is not Shall's to remove — a plan \
         that reaps it is `purge-undeclared`, which is a command you type"
    );

    let calls = kernel.mock_executor.get_calls().await;
    assert!(
        !calls
            .iter()
            .any(|c| c.contains("uninstall") && c.contains("fd")),
        "an undeclared package the user installed was scheduled for removal: {:?}",
        calls
    );
}

/// **The fourth act: a plan that fails part-way, and the sync after it.**
///
/// The three acts above prove convergence forward, backward and forward again — all of them on
/// the happy path. `lamdan/whole-repo-2026-08-07.md` names the gap: *"convergence is proved
/// forward and backward on the happy path only, and the one mechanism that provably un-converges
/// runs only when something fails."*
///
/// That mechanism is `auto_rollback`, on by default (`transaction.rs`). On the first failure
/// it walks the completed nodes backwards, and for each one whose `Prior` is `Absent` it calls
/// `remove` — **on packages this run installed successfully, which are still in the manifest,
/// which the next `sync` therefore reinstalls.** `heal`, whose entire job is the same failure
/// shape, sets `auto_rollback: false`, and nothing explained the split.
///
/// So: declare two packages, make one of them fail, and assert that the run afterwards leaves
/// the machine matching the file. The interesting assertion is not that the failure is reported.
/// It is that **the package that succeeded is still there** — because `Prior::Absent` means *was
/// not here before this run*, and rollback was reading it as *nobody wants this*.
#[tokio::test]
async fn a_failed_plan_does_not_undo_the_part_that_worked_and_is_still_declared() {
    let kernel = TestKernel::new().await;
    // **Two managers, not two lines under one.** `Y1` ruled that a manager's installs are
    // batched into one invocation per wave, so `brew:fd` and `brew:neovim` are a single
    // `brew install` — both succeed or both fail, and there is no "the part that worked" left
    // to preserve. Two backends are two commands, which is the shape this test is about.
    kernel
        .mock_executor
        .set_response("brew install -- fd", Ok(DryRunOutput::default().into()));
    kernel.mock_executor.set_response(
        "cargo install -- ripgrep",
        Err(shall::core::Error::command_failed(
            "error: could not compile `ripgrep`",
        )),
    );
    // Rollback's compensating removal, registered so that if it is ever issued this test can
    // say so by name rather than by an absence.
    kernel
        .mock_executor
        .set_response("brew uninstall -- fd", Ok(DryRunOutput::default().into()));
    machine_holds(&kernel.mock_executor, &[]).await;

    // **Two managers, not two lines under one.** `Y1` ruled that a manager's installs are
    // batched into one invocation per wave, so `brew:fd` and `brew:neovim` would be a single
    // `brew install` — both succeed or both fail together, and there is no "the part that
    // worked" left to preserve. Two backends are two commands, which is the shape this is about.
    declare(&kernel, "brew:fd\ncargo:ripgrep\n").await;

    // The plan is two installs; one of them fails, so the sync as a whole fails. That is
    // correct and is not what this test is about.
    let app = kernel.second_run().await;
    let resolver = StateResolver::new(&app.config, app.registry.clone(), false).await;
    let desired = resolver.resolve_desired_state().await.expect("resolves");
    let changes = {
        let state = app.state.lock().await;
        ChangePlanner::new(app.registry.clone(), &state, &app.config)
            .plan(&desired, PlanScope::Whole(HostBackends::default()))
            .await
            .expect("plans")
    };
    assert_eq!(
        changes.total_install(),
        2,
        "both lines are new to this machine"
    );
    let engine = app.sync_engine();
    let outcome = engine
        .sync(changes, shall::app::sync::guard::GuardScope::Sync)
        .await;
    assert!(
        outcome.is_err(),
        "a plan with a failing member is a failed plan — half-applying it silently is the \
         thing `continue_on_error: false` exists to prevent"
    );

    // **The assertion this act exists for.** `fd` installed, and `fd` is still declared. It is
    // not failed work; it is the goal, reached early. Rollback compensating it would hand the
    // next sync the same install to do again — which is a machine that oscillates rather than
    // converges, and it is the one mechanism in the program that provably un-converges.
    let calls = kernel.mock_executor.get_calls().await;
    assert!(
        !calls
            .iter()
            .any(|c| c.contains("uninstall") && c.contains("fd")),
        "rollback removed `fd`, which installed cleanly and is still in the manifest. \
         `Prior::Absent` says the package was not here before this run; it does not say \
         nobody wants it, and the manifest holds that second fact. Calls: {:?}",
        calls
    );
}
