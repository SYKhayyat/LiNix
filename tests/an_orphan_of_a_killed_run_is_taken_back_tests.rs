//! `S87` — a cleanup uninstall reported success over a package it did not remove.
//!
//! Ownership is held in memory through a sync and serialised to `registry.json` once, at the
//! end, and only when the whole transaction succeeded. The write-ahead log is written per
//! operation. The two therefore fall out of step in one direction: a run killed after an
//! install has completed leaves the package installed, `Completed` in the log, and in no
//! registry. Nothing put it right — the entry is terminal so recovery had nothing to replay,
//! the package is present so no later sync reinstalled it, and drift removal only removes what
//! LiNix manages, so the one command for removing it planned no change and answered `already
//! up to date` while the binary stayed on PATH.
//!
//! Reproduced on the `void` leg on 2026-08-11 by killing a sync the moment the log recorded
//! its first `Completed`: 3 of 3 canaries on disk, an empty registry, `heal` recovering only
//! the one operation still open, then
//!
//!     why xbps:pv          -> 'xbps:pv' is not under LiNix management.
//!     uninstall xbps:pv    -> already up to date        rc=0, pv STILL ON PATH
//!
//! Killing the same sync a tenth of a second later — after the final write — left all three
//! removable, which is the whole of the intermittency that made this look like a race.

use linix::core::journal::JournalAction;
use linix::core::executor::DryRunOutput;
use linix::core::PackageSpec;

use crate::mock_providers::TestKernel;

fn spec(name: &str) -> PackageSpec {
    PackageSpec {
        name: name.into(),
        backend: "brew".into(),
        options: Default::default(),
        requires: vec![],
        present: true,
    }
}

/// An operation the log carries through to `Completed`, as a finished run would leave it.
async fn record_completed(kernel: &TestKernel, action: JournalAction) {
    let mut j = kernel.app.journal.lock().await;
    let id = j.record_start(action).expect("could not write the WAL");
    j.record_success(&id).expect("could not close the entry");
}

/// What the manager answers when asked what it holds.
fn brew_holds(kernel: &TestKernel, names: &[&str]) {
    let listing = names
        .iter()
        .map(|n| format!("{} 1.0\n", n))
        .collect::<String>();
    kernel.mock_executor.set_response(
        "brew list --versions",
        Ok(DryRunOutput {
            stdout: listing.into_bytes(),
            stderr: vec![],
        }
        .into()),
    );
}

async fn manages(kernel: &TestKernel, name: &str) -> bool {
    kernel.app.state.lock().await.is_managed("brew", name)
}

/// The same app with `--yes`, for the one test here whose plan is not empty. A removal that
/// is genuinely planned stops at the confirmation gate under a test harness, which has no
/// terminal to answer it — and the case this file exists for never reaches that gate, because
/// its plan is the empty one.
fn confirming(kernel: &TestKernel) -> linix::app::App {
    let mut config = (*kernel.app.config).clone();
    config.yes = true;
    linix::app::App {
        config: std::sync::Arc::new(config),
        registry: kernel.app.registry.clone(),
        executor: kernel.app.executor.duplicate(),
        metrics: kernel.app.metrics.clone(),
        progress: kernel.app.progress.clone(),
        hooks: kernel.app.hooks.clone(),
        state: kernel.app.state.clone(),
        snapshot_manager: kernel.app.snapshot_manager.clone(),
        journal: kernel.app.journal.clone(),
        diagnostics: kernel.app.diagnostics.clone(),
        scheduler: kernel.app.scheduler.clone(),
        reaping: kernel.app.reaping.clone(),
    }
}

/// The bug, and the fix: the log says this machine installed it, the machine still has it, and
/// nothing claims it. That is not a mystery to be diagnosed later — it is a disagreement
/// between two files, and recovery is what makes them agree.
#[tokio::test]
async fn a_completed_install_that_never_reached_the_registry_is_taken_back() {
    let kernel = TestKernel::new().await;
    record_completed(&kernel, JournalAction::Install(spec("orphan-pkg"))).await;
    brew_holds(&kernel, &["orphan-pkg"]);

    assert!(
        !manages(&kernel, "orphan-pkg").await,
        "the fixture is wrong: the registry already carries the package"
    );

    kernel
        .app
        .sync_engine()
        .await
        .heal()
        .await
        .expect("heal failed");

    assert!(
        manages(&kernel, "orphan-pkg").await,
        "a package the log says this machine installed, and that the manager still holds, is \
         still owned by nobody — so `uninstall` will plan no change and report success"
    );
}

/// **Nothing is interrupted here**, and that is the point. `needs_recovery` is false for a
/// `Completed` entry, so a sync that consulted it before calling `heal` skipped the repair
/// above on every run — which is how one orphan survived a converge sync, an idempotence sync
/// and three uninstalls in the measured failure.
#[tokio::test]
async fn the_repair_does_not_wait_for_something_to_be_interrupted() {
    let kernel = TestKernel::new().await;
    record_completed(&kernel, JournalAction::Install(spec("orphan-pkg"))).await;
    brew_holds(&kernel, &["orphan-pkg"]);

    assert!(
        !kernel.app.journal.lock().await.needs_recovery(),
        "the fixture is wrong: this log has something interrupted in it"
    );

    kernel
        .app
        .sync_engine()
        .await
        .heal()
        .await
        .expect("heal failed");

    assert!(manages(&kernel, "orphan-pkg").await);
}

/// The other direction, and the one that would do damage: claiming a package that is not there
/// makes LiNix issue a removal for it on the next sync.
#[tokio::test]
async fn a_package_the_manager_no_longer_holds_is_not_taken_back() {
    let kernel = TestKernel::new().await;
    record_completed(&kernel, JournalAction::Install(spec("gone-pkg"))).await;
    brew_holds(&kernel, &[]);

    kernel
        .app
        .sync_engine()
        .await
        .heal()
        .await
        .expect("heal failed");

    assert!(
        !manages(&kernel, "gone-pkg").await,
        "the log records an install that is no longer on the machine, and LiNix claimed it"
    );
}

/// A manager that cannot be asked proves nothing. The alternative default — assume it is still
/// there — turns one manager having a bad day into a registry full of packages this machine
/// does not have, each of which the next sync tries to remove.
#[tokio::test]
async fn a_manager_that_cannot_answer_leaves_the_package_unclaimed() {
    let kernel = TestKernel::new().await;
    record_completed(&kernel, JournalAction::Install(spec("unknown-pkg"))).await;
    kernel.mock_executor.set_response(
        "brew list --versions",
        Err(linix::core::Error::Other("brew is wedged".into())),
    );

    kernel
        .app
        .sync_engine()
        .await
        .heal()
        .await
        .expect("heal failed");

    assert!(
        !manages(&kernel, "unknown-pkg").await,
        "a failed listing was read as `yes, it is installed`"
    );
}

/// A package LiNix installed, then removed, and that somebody put back by hand is theirs. The
/// log carries both operations for ever, so reading only the install would take it back.
#[tokio::test]
async fn an_install_the_log_later_removed_is_not_taken_back() {
    let kernel = TestKernel::new().await;
    record_completed(&kernel, JournalAction::Install(spec("returned-pkg"))).await;
    record_completed(
        &kernel,
        JournalAction::Remove {
            name: "returned-pkg".into(),
            backend: "brew".into(),
        },
    )
    .await;

    kernel
        .app
        .sync_engine()
        .await
        .heal()
        .await
        .expect("heal failed");

    assert!(
        !manages(&kernel, "returned-pkg").await,
        "LiNix gave this package up and then claimed it again from a stale log entry"
    );
    // And the manager was never asked. The log alone settles this one, which is what keeps
    // the repair free on the machines that have nothing to repair — it runs in front of every
    // sync.
    assert!(
        kernel.mock_executor.get_calls().await.is_empty(),
        "a package the log itself rules out still cost a listing: {:?}",
        kernel.mock_executor.get_calls().await
    );
}

/// A script is not a package. The log carries `exec:` and its undo as first-class entries, and
/// a reader that treats every completed entry as an install would hand their names to a
/// package manager.
#[tokio::test]
async fn a_completed_script_is_not_a_package_to_take_back() {
    let kernel = TestKernel::new().await;
    record_completed(
        &kernel,
        JournalAction::Exec {
            script: "setup.sh".into(),
            hash: "abc123".into(),
        },
    )
    .await;

    kernel
        .app
        .sync_engine()
        .await
        .heal()
        .await
        .expect("heal failed");

    assert!(
        !kernel.app.state.lock().await.is_managed("exec", "setup.sh"),
        "a completed script was recorded as a managed package"
    );
    assert!(
        kernel.mock_executor.get_calls().await.is_empty(),
        "a script's name was handed to a package manager to ask about: {:?}",
        kernel.mock_executor.get_calls().await
    );
}

/// The other half of `S87`, and the owner's ruling of 2026-08-11: a removal that removed
/// nothing must say so, and say that LiNix does not own the package.
///
/// The line was declared and the line was deleted, so the check for a name no file declares
/// says nothing about this case — and the sync that follows plans no change, because drift
/// removal only removes what LiNix manages. The measured failure was three commands answering
/// `already up to date` at exit 0 with all three binaries still on PATH.
#[tokio::test]
async fn uninstalling_a_package_linix_does_not_own_says_so_instead_of_succeeding() {
    let kernel = TestKernel::new().await;
    // Declared, so `undeclare` finds a line and the "not declared anywhere" arm does not fire.
    std::fs::write(kernel.tmp.path().join("profiles/Main"), "brew:orphan-pkg\n").unwrap();
    // On the machine, and in no registry — what a killed run leaves behind.
    brew_holds(&kernel, &["orphan-pkg"]);
    assert!(!manages(&kernel, "orphan-pkg").await);

    let err = linix::verbs::packages::handle_uninstall(
        &kernel.app,
        &["brew:orphan-pkg".to_string()],
        linix::core::Output::Human,
        None,
        false,
    )
    .await
    .expect_err("uninstall reported success over a package it did not remove");

    let said = err.to_string();
    assert!(
        said.contains("nothing was uninstalled"),
        "the failure does not say that nothing was uninstalled: {said}"
    );
    assert!(
        said.contains("brew:orphan-pkg") && said.contains("no record of installing"),
        "the failure names neither the package nor the reason: {said}"
    );
    assert!(
        said.contains("adopt"),
        "the failure says what is wrong and not what to do about it: {said}"
    );
    assert!(
        said.contains("--absent"),
        "the failure names one way past it and not the other — a user who does not want to \
         own the package first has no route out of this message: {said}"
    );
}

/// The owner's ruling of 2026-08-11 on the half of `Q54` left open: a flag that removes what
/// LiNix does not own, by writing the `absent:` declaration.
///
/// Three things at once, because they are one behaviour: the module line goes, an `absent:`
/// line arrives, and the removal runs against a package no registry claims. The mock manager
/// keeps reporting the package after removing it — a static listing is all it has — so the
/// command ends by saying it is still installed. That is the `S87` rule holding on this path
/// too, and it is asserted here rather than worked around.
#[tokio::test]
async fn absent_removes_a_package_linix_does_not_own_and_declares_it_gone() {
    let kernel = TestKernel::new().await;
    std::fs::write(kernel.tmp.path().join("profiles/Main"), "brew:orphan-pkg\n").unwrap();
    brew_holds(&kernel, &["orphan-pkg"]);
    kernel.mock_executor.set_response(
        "brew uninstall -- orphan-pkg",
        Ok(DryRunOutput::default().into()),
    );
    assert!(!manages(&kernel, "orphan-pkg").await);

    let err = linix::verbs::packages::handle_uninstall(
        &confirming(&kernel),
        &["brew:orphan-pkg".to_string()],
        linix::core::Output::Human,
        None,
        true,
    )
    .await
    .expect_err("the mock manager never drops the package, so this cannot report success");

    let said = err.to_string();
    assert!(
        said.contains("declared absent") && said.contains("still installed"),
        "a removal that removed nothing reported something other than that: {said}"
    );

    let calls = kernel.mock_executor.get_calls().await;
    assert!(
        calls.iter().any(|c| c.contains("uninstall -- orphan-pkg")),
        "`--absent` never reached the manager, so it removed nothing at all: {calls:?}"
    );

    let written = std::fs::read_to_string(kernel.tmp.path().join("modules/imperative.txt"))
        .expect("`--absent` wrote no declaration");
    assert!(
        written.contains("absent:brew:orphan-pkg"),
        "the package was removed and nothing says it should stay removed: {written}"
    );
    let profile = std::fs::read_to_string(kernel.tmp.path().join("profiles/Main")).unwrap();
    assert!(
        !profile.contains("brew:orphan-pkg"),
        "the module line survived alongside the absent line, which is a config that argues \
         with itself on every sync: {profile}"
    );
}

/// A bare name is resolved by asking who *holds* it, not who could supply it. `install`
/// resolves the other way, and borrowing that here would write an `absent:` line naming a
/// manager that never had the package — a line that then outlives the run that guessed.
#[tokio::test]
async fn absent_names_the_manager_that_actually_holds_a_bare_name() {
    let kernel = TestKernel::new().await;
    brew_holds(&kernel, &["orphan-pkg"]);
    kernel.mock_executor.set_response(
        "brew uninstall -- orphan-pkg",
        Ok(DryRunOutput::default().into()),
    );

    let _ = linix::verbs::packages::handle_uninstall(
        &confirming(&kernel),
        &["orphan-pkg".to_string()],
        linix::core::Output::Human,
        None,
        true,
    )
    .await;

    let written = std::fs::read_to_string(kernel.tmp.path().join("modules/imperative.txt"))
        .expect("`--absent` wrote no declaration for a bare name");
    assert!(
        written.contains("absent:brew:orphan-pkg"),
        "a bare name did not resolve to the manager holding it: {written}"
    );
}

/// And a bare name nobody holds is refused, not guessed at. There is no manager to name, and
/// picking one would write a permanent line about a package that manager never had.
#[tokio::test]
async fn absent_refuses_a_bare_name_no_manager_holds() {
    let kernel = TestKernel::new().await;
    brew_holds(&kernel, &[]);

    let err = linix::verbs::packages::handle_uninstall(
        &confirming(&kernel),
        &["ghost-pkg".to_string()],
        linix::core::Output::Human,
        None,
        true,
    )
    .await
    .expect_err("`--absent` invented a manager for a package nothing holds");

    let said = err.to_string();
    assert!(
        said.contains("nothing to declare absent"),
        "the refusal does not say why it refused: {said}"
    );
    assert!(
        said.contains("ghost-pkg"),
        "the refusal does not name the package: {said}"
    );
    assert!(
        !kernel.tmp.path().join("modules/imperative.txt").exists()
            || !std::fs::read_to_string(kernel.tmp.path().join("modules/imperative.txt"))
                .unwrap()
                .contains("ghost-pkg"),
        "the refusal still wrote the line it refused to write"
    );
}

/// `--temp` says *bring it back*; `--absent` says *keep it gone*. Together they are two
/// declarations about the same package pointing opposite ways, so the parser refuses the pair
/// rather than letting whichever branch runs second decide.
#[test]
fn absent_and_temp_cannot_be_combined() {
    use clap::Parser;
    linix::cli::args::Cli::try_parse_from([
        "linix",
        "uninstall",
        "brew:pkg",
        "--absent",
        "--temp=2h",
    ])
    .expect_err("a package cannot be scheduled to return and declared permanently gone");
}

/// And the ordinary removal still succeeds. The check above asks the manager one question and
/// only about names the registry does not carry, so a package LiNix owns pays for none of it —
/// verified here rather than assumed, because a verification that fires on the happy path
/// turns every uninstall into a listing.
#[tokio::test]
async fn uninstalling_a_package_linix_owns_is_unaffected() {
    let kernel = TestKernel::new().await;
    std::fs::write(kernel.tmp.path().join("profiles/Main"), "brew:owned-pkg\n").unwrap();
    {
        let mut state = kernel.app.state.lock().await;
        state.add("brew", "owned-pkg", None, Default::default(), "sync", false);
    }
    kernel.mock_executor.set_response(
        "brew uninstall -- owned-pkg",
        Ok(DryRunOutput::default().into()),
    );

    linix::verbs::packages::handle_uninstall(
        &confirming(&kernel),
        &["brew:owned-pkg".to_string()],
        linix::core::Output::Human,
        None,
        false,
    )
    .await
    .expect("removing a package LiNix owns failed");

    let calls = kernel.mock_executor.get_calls().await;
    assert!(
        calls.iter().any(|c| c.contains("uninstall")),
        "the removal never ran: {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.contains("list --versions")),
        "an ordinary uninstall paid for a listing it did not need: {calls:?}"
    );
}

/// The same, written the other way a user writes it. A bare name means *the one I have*, so one
/// manager owning it settles the question — widening it to every manager would turn an ordinary
/// `linix uninstall jq` into a listing from every package manager on the box.
#[tokio::test]
async fn a_bare_name_linix_owns_costs_no_listing_either() {
    let kernel = TestKernel::new().await;
    std::fs::write(kernel.tmp.path().join("profiles/Main"), "brew:owned-pkg\n").unwrap();
    {
        let mut state = kernel.app.state.lock().await;
        state.add("brew", "owned-pkg", None, Default::default(), "sync", false);
    }
    kernel.mock_executor.set_response(
        "brew uninstall -- owned-pkg",
        Ok(DryRunOutput::default().into()),
    );

    linix::verbs::packages::handle_uninstall(
        &confirming(&kernel),
        &["owned-pkg".to_string()],
        linix::core::Output::Human,
        None,
        false,
    )
    .await
    .expect("removing a package LiNix owns, named without its manager, failed");

    let calls = kernel.mock_executor.get_calls().await;
    assert!(
        !calls.iter().any(|c| c.contains("list --versions")),
        "a bare name LiNix already owns still cost a listing: {calls:?}"
    );
}

/// **`unmanage` has to survive the repair, or the repair uninstalls people's software.**
///
/// `unmanage` means *stop watching this, leave it installed*: it drops the registry entry and
/// the manifest line and touches the machine not at all. From the registry alone that is
/// indistinguishable from an ownership record a killed run never wrote — so a repair that reads
/// only the registry takes the package back, the next sync finds it declared nowhere, and drift
/// removal takes away software the user asked LiNix only to forget. The log is the third record
/// of the same relationship and `unmanage` now clears it too.
#[tokio::test]
async fn a_package_the_user_told_linix_to_forget_stays_forgotten() {
    let kernel = TestKernel::new().await;
    std::fs::write(kernel.tmp.path().join("profiles/Main"), "brew:kept-pkg\n").unwrap();
    record_completed(&kernel, JournalAction::Install(spec("kept-pkg"))).await;
    {
        let mut state = kernel.app.state.lock().await;
        state.add("brew", "kept-pkg", None, Default::default(), "sync", false);
    }

    linix::verbs::cleanup::handle_unmanage(
        &kernel.app,
        &["brew:kept-pkg".to_string()],
        linix::core::Output::Human,
    )
    .await
    .expect("unmanage failed");
    assert!(
        !manages(&kernel, "kept-pkg").await,
        "the fixture is wrong: unmanage did not drop the registry entry"
    );

    // The machine still has it — that is what `unmanage` promises — and the repair does not
    // even get as far as asking, because the log no longer claims the package either.
    kernel
        .app
        .sync_engine()
        .await
        .heal()
        .await
        .expect("heal failed");

    assert!(
        !manages(&kernel, "kept-pkg").await,
        "the repair took back a package the user had explicitly told LiNix to forget — the \
         next sync would find it declared nowhere and uninstall it"
    );
    assert!(
        kernel.mock_executor.get_calls().await.is_empty(),
        "a forgotten package was still a candidate worth asking a manager about: {:?}",
        kernel.mock_executor.get_calls().await
    );
}

/// And forgetting drops only what is finished. An `InProgress` entry is the record that
/// something on this machine is half-done, and a package being forgotten is not a reason to
/// lose the evidence that its install never completed.
#[tokio::test]
async fn forgetting_a_package_keeps_the_record_of_work_still_open() {
    let kernel = TestKernel::new().await;
    {
        let mut j = kernel.app.journal.lock().await;
        j.record_start(JournalAction::Install(spec("half-done-pkg")))
            .expect("could not write the WAL");
    }

    linix::verbs::cleanup::handle_unmanage(
        &kernel.app,
        &["brew:half-done-pkg".to_string()],
        linix::core::Output::Human,
    )
    .await
    .expect("unmanage failed");

    assert!(
        kernel.app.journal.lock().await.needs_recovery(),
        "forgetting a package threw away the record that its install was interrupted"
    );
}

/// S25's rule, one repair further on: a preview changes nothing. This one writes the registry,
/// which is the file every later run reads to decide what it may remove.
#[tokio::test]
async fn a_preview_takes_nothing_back() {
    let kernel = TestKernel::new().await;
    record_completed(&kernel, JournalAction::Install(spec("orphan-pkg"))).await;
    brew_holds(&kernel, &["orphan-pkg"]);

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
        kernel.app.reaping.clone(),
    )
    .await;
    engine.heal().await.expect("heal failed");

    assert!(
        !manages(&kernel, "orphan-pkg").await,
        "a `--dry-run` wrote an ownership record"
    );
}
