//! `S87` — a cleanup uninstall reported success over a package it did not remove.
//!
//! Ownership is held in memory through a sync and serialised to `registry.json` once, at the
//! end, and only when the whole transaction succeeded. The write-ahead log is written per
//! operation. The two therefore fall out of step in one direction: a run killed after an
//! install has completed leaves the package installed, `Completed` in the log, and in no
//! registry. Nothing put it right — the entry is terminal so recovery had nothing to replay,
//! the package is present so no later sync reinstalled it, and drift removal only removes what
//! Shall manages, so the one command for removing it planned no change and answered `already
//! up to date` while the binary stayed on PATH.
//!
//! Reproduced on the `void` leg on 2026-08-11 by killing a sync the moment the log recorded
//! its first `Completed`: 3 of 3 canaries on disk, an empty registry, `heal` recovering only
//! the one operation still open, then
//!
//!     why xbps:pv          -> 'xbps:pv' is not under Shall management.
//!     uninstall xbps:pv    -> already up to date        rc=0, pv STILL ON PATH
//!
//! Killing the same sync a tenth of a second later — after the final write — left all three
//! removable, which is the whole of the intermittency that made this look like a race.

use shall::core::executor::DryRunOutput;
use shall::core::journal::JournalAction;
use shall::core::PackageSpec;

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
fn confirming(kernel: &TestKernel) -> shall::app::App {
    // Twelve fields copied by hand stood here, which is what a struct with no narrower
    // constructor costs its callers: every field added to `App` had to be added here too, and
    // forgetting one silently shared the wrong state.
    kernel.app.reconfigured(|c| c.yes = true)
}

/// This machine's resolved package set — what ownership is read from. Resolved for real rather
/// than hand-built, so a test that writes a line proves the line reaches the repair.
async fn declared(kernel: &TestKernel) -> Vec<PackageSpec> {
    shall::app::sync::resolver::StateResolver::new(
        &kernel.app.config,
        kernel.app.registry.clone(),
        false,
    )
    .await
    .resolve_model()
    .await
    .expect("the fixture's config does not resolve")
    .packages
    .into_values()
    .flatten()
    .collect()
}

/// What the active profile declares.
fn declares(kernel: &TestKernel, lines: &str) {
    std::fs::write(kernel.tmp.path().join("profiles/Main"), lines).unwrap();
}

/// The manager's listing, registered as a stub the repair must never reach. A plain stub that
/// goes unmatched proves only that nothing asked; this one answers convincingly if anything
/// does, so a repair that widened to undeclared packages would claim them and fail the
/// assertion rather than quietly pass.
fn brew_must_not_be_asked(kernel: &TestKernel, names: &[&str]) {
    let listing = names
        .iter()
        .map(|n| format!("{} 1.0\n", n))
        .collect::<String>();
    kernel.mock_executor.set_response_that_must_not_be_used(
        "brew list --versions",
        Ok(DryRunOutput {
            stdout: listing.into_bytes(),
            stderr: vec![],
        }
        .into()),
    );
}

/// The bug, and the fix: this machine declares the package, the machine has it, and nothing
/// claims it. That is not a mystery to be diagnosed later — it is a disagreement between the
/// manifest and the registry, and recovery is what makes them agree.
#[tokio::test]
async fn a_declared_package_the_registry_never_recorded_is_taken_back() {
    let kernel = TestKernel::new().await;
    declares(&kernel, "brew:orphan-pkg\n");
    brew_holds(&kernel, &["orphan-pkg"]);

    assert!(
        !manages(&kernel, "orphan-pkg").await,
        "the fixture is wrong: the registry already carries the package"
    );

    let declared = declared(&kernel).await;
    kernel
        .app
        .sync_engine()
        .await
        .heal(&declared)
        .await
        .expect("heal failed");

    assert!(
        manages(&kernel, "orphan-pkg").await,
        "a package this machine declares, and that the manager holds, is still owned by nobody \
         — so `uninstall` will plan no change and report success"
    );
}

/// **Nothing is interrupted here**, and that is the point. `needs_recovery` is false for a
/// machine whose log is clean, so a sync that consulted it before calling `heal` skipped the
/// repair above on every run — which is how one orphan survived a converge sync, an idempotence
/// sync and three uninstalls in the measured failure.
#[tokio::test]
async fn the_repair_does_not_wait_for_something_to_be_interrupted() {
    let kernel = TestKernel::new().await;
    declares(&kernel, "brew:orphan-pkg\n");
    brew_holds(&kernel, &["orphan-pkg"]);

    assert!(
        !kernel.app.journal.lock().await.needs_recovery(),
        "the fixture is wrong: this log has something interrupted in it"
    );

    let declared = declared(&kernel).await;
    kernel
        .app
        .sync_engine()
        .await
        .heal(&declared)
        .await
        .expect("heal failed");

    assert!(manages(&kernel, "orphan-pkg").await);
}

/// A declaration is a wish, not a fact. Claiming a package that is not there makes Shall issue a
/// removal for it on the next sync; the install that follows is what records this one.
#[tokio::test]
async fn a_declared_package_the_manager_does_not_hold_is_not_taken_back() {
    let kernel = TestKernel::new().await;
    declares(&kernel, "brew:gone-pkg\n");
    brew_holds(&kernel, &[]);

    let declared = declared(&kernel).await;
    kernel
        .app
        .sync_engine()
        .await
        .heal(&declared)
        .await
        .expect("heal failed");

    assert!(
        !manages(&kernel, "gone-pkg").await,
        "a package that is declared and not installed was claimed as owned"
    );
}

/// A manager that cannot be asked proves nothing. The alternative default — assume it is still
/// there — turns one manager having a bad day into a registry full of packages this machine
/// does not have, each of which the next sync tries to remove.
#[tokio::test]
async fn a_manager_that_cannot_answer_leaves_the_package_unclaimed() {
    let kernel = TestKernel::new().await;
    declares(&kernel, "brew:unknown-pkg\n");
    kernel.mock_executor.set_response(
        "brew list --versions",
        Err(shall::core::Error::Other("brew is wedged".into())),
    );

    let declared = declared(&kernel).await;
    kernel
        .app
        .sync_engine()
        .await
        .heal(&declared)
        .await
        .expect("heal failed");

    assert!(
        !manages(&kernel, "unknown-pkg").await,
        "a failed listing was read as `yes, it is installed`"
    );
}

/// **The boundary of the ruling of 2026-08-11.** Declaring a package you already had makes it
/// Shall's; having it and never declaring it does not. This is the software on the machine that
/// is nobody's business but the user's, and the repair must not widen into it — a machine's
/// installed set is not a manifest.
#[tokio::test]
async fn a_package_this_machine_does_not_declare_is_never_claimed() {
    let kernel = TestKernel::new().await;
    brew_must_not_be_asked(&kernel, &["hand-installed-pkg"]);

    let declared = declared(&kernel).await;
    kernel
        .app
        .sync_engine()
        .await
        .heal(&declared)
        .await
        .expect("heal failed");

    assert!(
        !manages(&kernel, "hand-installed-pkg").await,
        "Shall took ownership of software the user installed and never declared"
    );
    // And no manager was asked. Nothing is declared, so there is no candidate to ask about —
    // which is what keeps this free on the machines that have nothing to repair, since it runs
    // in front of every sync.
    assert!(
        kernel.mock_executor.get_calls().await.is_empty(),
        "an undeclared machine still cost a listing: {:?}",
        kernel.mock_executor.get_calls().await
    );
}

/// An `absent:` line is a declaration that the package must **not** be here. Claiming it would
/// have Shall take ownership of something it is under orders to remove — and every declaration
/// arrives through the same map, so the presence flag is the only thing separating the two.
#[tokio::test]
async fn an_absent_declaration_is_not_a_claim() {
    let kernel = TestKernel::new().await;
    // In a module, not the profile: a profile's `absent:` is refused by the parser, because
    // there `-<package>` means "leave it out of this profile" and the two are different claims.
    std::fs::write(
        kernel.tmp.path().join("modules/banned.txt"),
        "absent:brew:banned-pkg\n",
    )
    .unwrap();
    declares(&kernel, "use banned\n");
    // The filter drops it before any manager is asked, so the listing must go unreached — and
    // it answers convincingly if the filter ever breaks, rather than letting the claim fail for
    // the unrelated reason that nothing was stubbed.
    brew_must_not_be_asked(&kernel, &["banned-pkg"]);

    let declared = declared(&kernel).await;
    assert!(
        declared.iter().any(|s| s.name == "banned-pkg"),
        "the fixture is wrong: the absent line never reached the resolved set, so this test \
         would pass without the filter it exists to check"
    );

    kernel
        .app
        .sync_engine()
        .await
        .heal(&declared)
        .await
        .expect("heal failed");

    assert!(
        !manages(&kernel, "banned-pkg").await,
        "Shall claimed ownership of a package it is declared to remove"
    );
}

/// The other half of `S87`, and the owner's ruling of 2026-08-11: a removal that removed
/// nothing must say so, and say that Shall does not own the package.
///
/// The line was declared and the line was deleted, so the check for a name no file declares
/// says nothing about this case — and the sync that follows plans no change, because drift
/// removal only removes what Shall manages. The measured failure was three commands answering
/// `already up to date` at exit 0 with all three binaries still on PATH.
#[tokio::test]
async fn uninstalling_a_package_shall_does_not_own_says_so_instead_of_succeeding() {
    let kernel = TestKernel::new().await;
    // Declared, so `undeclare` finds a line and the "not declared anywhere" arm does not fire.
    std::fs::write(kernel.tmp.path().join("profiles/Main"), "brew:orphan-pkg\n").unwrap();
    // On the machine, and in no registry — what a killed run leaves behind.
    brew_holds(&kernel, &["orphan-pkg"]);
    assert!(!manages(&kernel, "orphan-pkg").await);

    let err = shall::verbs::packages::handle_uninstall(
        &kernel.app,
        &["brew:orphan-pkg".to_string()],
        shall::core::Output::Human,
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
/// Shall does not own, by writing the `absent:` declaration.
///
/// Three things at once, because they are one behaviour: the module line goes, an `absent:`
/// line arrives, and the removal runs against a package no registry claims. The mock manager
/// keeps reporting the package after removing it — a static listing is all it has — so the
/// command ends by saying it is still installed. That is the `S87` rule holding on this path
/// too, and it is asserted here rather than worked around.
#[tokio::test]
async fn absent_removes_a_package_shall_does_not_own_and_declares_it_gone() {
    let kernel = TestKernel::new().await;
    std::fs::write(kernel.tmp.path().join("profiles/Main"), "brew:orphan-pkg\n").unwrap();
    brew_holds(&kernel, &["orphan-pkg"]);
    kernel.mock_executor.set_response(
        "brew uninstall -- orphan-pkg",
        Ok(DryRunOutput::default().into()),
    );
    assert!(!manages(&kernel, "orphan-pkg").await);

    let err = shall::verbs::packages::handle_uninstall(
        &confirming(&kernel),
        &["brew:orphan-pkg".to_string()],
        shall::core::Output::Human,
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

    let _ = shall::verbs::packages::handle_uninstall(
        &confirming(&kernel),
        &["orphan-pkg".to_string()],
        shall::core::Output::Human,
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

    let err = shall::verbs::packages::handle_uninstall(
        &confirming(&kernel),
        &["ghost-pkg".to_string()],
        shall::core::Output::Human,
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
    shall::cli::args::Cli::try_parse_from([
        "shall",
        "uninstall",
        "brew:pkg",
        "--absent",
        "--temp=2h",
    ])
    .expect_err("a package cannot be scheduled to return and declared permanently gone");
}

/// And the ordinary removal still succeeds. The check above asks the manager one question and
/// only about names the registry does not carry, so a package Shall owns pays for none of it —
/// verified here rather than assumed, because a verification that fires on the happy path
/// turns every uninstall into a listing.
#[tokio::test]
async fn uninstalling_a_package_shall_owns_is_unaffected() {
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

    shall::verbs::packages::handle_uninstall(
        &confirming(&kernel),
        &["brew:owned-pkg".to_string()],
        shall::core::Output::Human,
        None,
        false,
    )
    .await
    .expect("removing a package Shall owns failed");

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
/// `shall uninstall jq` into a listing from every package manager on the box.
#[tokio::test]
async fn a_bare_name_shall_owns_costs_no_listing_either() {
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

    shall::verbs::packages::handle_uninstall(
        &confirming(&kernel),
        &["owned-pkg".to_string()],
        shall::core::Output::Human,
        None,
        false,
    )
    .await
    .expect("removing a package Shall owns, named without its manager, failed");

    let calls = kernel.mock_executor.get_calls().await;
    assert!(
        !calls.iter().any(|c| c.contains("list --versions")),
        "a bare name Shall already owns still cost a listing: {calls:?}"
    );
}

/// **`unmanage` has to survive the repair, or the repair uninstalls people's software.**
///
/// `unmanage` means *stop watching this, leave it installed*: it drops the registry entry and
/// the manifest line and touches the machine not at all. Dropping the line is what makes that
/// stick, now that ownership is read from what the machine declares — so this asserts the two
/// halves together, because `unmanage` that dropped only the registry entry would be re-adopted
/// by the next sync, found declared nowhere, and uninstalled.
#[tokio::test]
async fn a_package_the_user_told_shall_to_forget_stays_forgotten() {
    let kernel = TestKernel::new().await;
    declares(&kernel, "brew:kept-pkg\n");
    brew_must_not_be_asked(&kernel, &["kept-pkg"]);
    {
        let mut state = kernel.app.state.lock().await;
        state.add("brew", "kept-pkg", None, Default::default(), "sync", false);
    }

    shall::verbs::cleanup::handle_unmanage(
        &kernel.app,
        &["brew:kept-pkg".to_string()],
        shall::core::Output::Human,
    )
    .await
    .expect("unmanage failed");
    assert!(
        !manages(&kernel, "kept-pkg").await,
        "the fixture is wrong: unmanage did not drop the registry entry"
    );

    // Resolved after the forgetting, which is the whole point: the line is gone, so the package
    // is not declared, so the repair below has no candidate to take back.
    let declared = declared(&kernel).await;
    assert!(
        declared.is_empty(),
        "unmanage left the declaration in place, and the repair is about to re-adopt it"
    );
    kernel
        .app
        .sync_engine()
        .await
        .heal(&declared)
        .await
        .expect("heal failed");

    assert!(
        !manages(&kernel, "kept-pkg").await,
        "the repair took back a package the user had explicitly told Shall to forget — the \
         next sync would find it declared nowhere and uninstall it"
    );
    assert!(
        kernel.mock_executor.get_calls().await.is_empty(),
        "a forgotten package was still a candidate worth asking a manager about: {:?}",
        kernel.mock_executor.get_calls().await
    );
}

/// And forgetting leaves the log alone entirely. An `InProgress` entry is the record that
/// something on this machine is half-done, and a package being forgotten is not a reason to
/// lose the evidence that its install never completed. `unmanage` used to clear the package's
/// finished entries, because the repair read them as ownership; the repair reads the manifest
/// now, so that clearing was deleted rather than left as a defence against nothing.
#[tokio::test]
async fn forgetting_a_package_keeps_the_record_of_work_still_open() {
    let kernel = TestKernel::new().await;
    {
        let mut j = kernel.app.journal.lock().await;
        j.record_start(JournalAction::Install(spec("half-done-pkg")))
            .expect("could not write the WAL");
    }

    shall::verbs::cleanup::handle_unmanage(
        &kernel.app,
        &["brew:half-done-pkg".to_string()],
        shall::core::Output::Human,
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
    declares(&kernel, "brew:orphan-pkg\n");
    brew_holds(&kernel, &["orphan-pkg"]);

    let mut previewing = (*kernel.app.config).clone();
    previewing.dry_run = true;
    let engine = shall::app::sync::SyncEngine::new(
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
    let declared = declared(&kernel).await;
    engine.heal(&declared).await.expect("heal failed");

    assert!(
        !manages(&kernel, "orphan-pkg").await,
        "a `--dry-run` wrote an ownership record"
    );
}
