//! 7b / XIII.3 — the `exec:` lifecycle, end to end through the real resolver and apply path.
//!
//! The exit condition, in one file: a script runs once, does not run on the next sync, runs
//! again after one byte of it changes, and the plan states the decision before any of it
//! happens. Plus the two rules that are easy to get wrong — II.12 approval is required, and a
//! failed script is not recorded as having run.

use shall::core::hook_lock::{exec_id, hash_script, HookLedger};
use shall::core::ExecLedger;
use shall::core::LockFile;

use crate::mock_providers::TestKernel;

/// Write a module declaring one `exec:` line, plus the script it names.
async fn declare_exec(kernel: &TestKernel, line: &str, script_rel: &str, body: &str) {
    let root = kernel.app.config.config_root();
    let script = root.join(script_rel);
    if let Some(dir) = script.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(&script, body).unwrap();
    std::fs::write(root.join("modules/tools.txt"), format!("{}\n", line)).unwrap();
    std::fs::write(root.join("profiles/Main"), "use tools\n").unwrap();
}

/// What `shall lock` does: record the script's current hash as approved.
fn approve(kernel: &TestKernel, script_rel: &str, body: &str) {
    let locks = kernel.app.config.layout().locks_dir();
    let path = HookLedger::path_in(&locks);
    let mut ledger = HookLedger::load(&path).unwrap();
    ledger.approve(&exec_id(script_rel), &hash_script(body));
    ledger.save(&path).unwrap();
}

fn runs_of(kernel: &TestKernel, body: &str) -> u32 {
    let locks = kernel.app.config.layout().locks_dir();
    ExecLedger::load(&ExecLedger::path_in(&locks))
        .unwrap()
        .count(&hash_script(body))
}

/// One resolution, as a fresh invocation would do it.
///
/// The `new_resolution()` is what makes a second call here mean a second run rather than a
/// second look inside one run: variables resolve once per invocation (IX.6), so a test that
/// edits `vars` between two resolutions has to say that a new invocation began — the same thing
/// `reconcile` says at the top of every `sync` and every `watch` tick.
async fn resolve(kernel: &TestKernel) -> shall::model::DesiredState {
    shall::app::sync::resolver::new_resolution();
    shall::app::sync::resolver::StateResolver::new(
        &kernel.app.config,
        kernel.app.registry.clone(),
        false,
    )
    .await
    .resolve_model()
    .await
    .expect("the model resolves")
}

/// The whole exit condition: runs once, not twice, and again after an edit.
#[tokio::test]
async fn a_script_runs_once_then_not_again_until_its_content_changes() {
    let kernel = TestKernel::new().await;
    let v1 = "echo one\n";
    declare_exec(&kernel, "exec:./bin/setup.sh", "./bin/setup.sh", v1).await;
    approve(&kernel, "./bin/setup.sh", v1);

    // First sync: it runs, and the run is recorded.
    let state = resolve(&kernel).await;
    assert!(state.has_execs(), "the exec: line did not reach the model");
    kernel
        .app
        .execs()
        .apply(&state, shall::model::exec::Verb::Sync)
        .await
        .expect("first run");
    assert_eq!(runs_of(&kernel, v1), 1, "the first run was not recorded");

    // Second sync: the same content is at its ceiling, so nothing runs.
    kernel
        .app
        .execs()
        .apply(&state, shall::model::exec::Verb::Sync)
        .await
        .expect("second pass");
    assert_eq!(runs_of(&kernel, v1), 1, "the script ran twice");

    // One byte changes: a different content, never run, so it runs — and the old row survives.
    let v2 = "echo two\n";
    declare_exec(&kernel, "exec:./bin/setup.sh", "./bin/setup.sh", v2).await;
    approve(&kernel, "./bin/setup.sh", v2);
    let state = resolve(&kernel).await;
    kernel
        .app
        .execs()
        .apply(&state, shall::model::exec::Verb::Sync)
        .await
        .expect("post-edit run");
    assert_eq!(runs_of(&kernel, v2), 1, "the edited script did not run");
    assert_eq!(runs_of(&kernel, v1), 1, "the old content's count was lost");
}

/// `@runs=always` is the explicit opt-out — and being explicit is the point, so it must
/// actually opt out.
#[tokio::test]
async fn runs_always_runs_every_time() {
    let kernel = TestKernel::new().await;
    let body = "echo tick\n";
    declare_exec(&kernel, "exec:./tick.sh@runs=always", "./tick.sh", body).await;
    approve(&kernel, "./tick.sh", body);

    let state = resolve(&kernel).await;
    for expected in 1..=3 {
        kernel
            .app
            .execs()
            .apply(&state, shall::model::exec::Verb::Sync)
            .await
            .expect("always runs");
        assert_eq!(runs_of(&kernel, body), expected);
    }
}

/// II.12, no exceptions: a script the configuration carries does not run until a human has
/// approved it, and `-y` cannot approve. An unapproved script is a REFUSAL, not a skip.
#[tokio::test]
async fn an_unapproved_script_refuses_the_sync_and_never_runs() {
    let kernel = TestKernel::new().await;
    let body = "echo untrusted\n";
    declare_exec(&kernel, "exec:./evil.sh", "./evil.sh", body).await;
    // Deliberately NOT approved.

    let state = resolve(&kernel).await;
    let err = kernel
        .app
        .execs()
        .apply(&state, shall::model::exec::Verb::Sync)
        .await
        .expect_err("an unapproved script must stop the sync");
    let msg = err.to_string();
    assert!(msg.contains("never been approved"), "{}", msg);
    assert!(msg.contains("shall lock"), "{}", msg);
    assert_eq!(runs_of(&kernel, body), 0, "it ran anyway");
}

/// The other half of II.12: approved once, then edited. The changed content is refused until
/// it is approved again — this is the case the ledger exists for.
#[tokio::test]
async fn a_script_edited_after_approval_is_refused_until_reapproved() {
    let kernel = TestKernel::new().await;
    let v1 = "echo safe\n";
    declare_exec(&kernel, "exec:./setup.sh", "./setup.sh", v1).await;
    approve(&kernel, "./setup.sh", v1);

    // Swap the content without re-approving — the supply-chain case.
    let v2 = "echo tampered\n";
    std::fs::write(kernel.app.config.config_root().join("./setup.sh"), v2).unwrap();

    let state = resolve(&kernel).await;
    let err = kernel
        .app
        .execs()
        .apply(&state, shall::model::exec::Verb::Sync)
        .await
        .expect_err("changed");
    let msg = err.to_string();
    assert!(msg.contains("changed since you approved"), "{}", msg);
    assert_eq!(runs_of(&kernel, v2), 0, "the tampered content ran");
}

/// A `when` that is false means nothing runs and nothing is undone — and, critically, the
/// ledger row survives, so a condition that flaps does not re-run the script each time it
/// swings back (XIII.3's three-state table).
#[tokio::test]
async fn a_false_when_runs_nothing_and_keeps_the_count() {
    let kernel = TestKernel::new().await;
    let body = "echo once\n";
    let root = kernel.app.config.config_root();
    std::fs::write(root.join("./enroll.sh"), body).unwrap();
    approve(&kernel, "./enroll.sh", body);

    // Gate it on a variable that is currently "no".
    std::fs::write(root.join("vars"), "enrolled = no\n").unwrap();
    std::fs::write(
        root.join("modules/tools.txt"),
        "when $enrolled == no {\n  exec:./enroll.sh\n}\n",
    )
    .unwrap();
    std::fs::write(root.join("profiles/Main"), "use tools\n").unwrap();

    let state = resolve(&kernel).await;
    assert!(
        state.has_execs(),
        "the gated exec: should be present when true"
    );
    kernel
        .app
        .execs()
        .apply(&state, shall::model::exec::Verb::Sync)
        .await
        .expect("runs");
    assert_eq!(runs_of(&kernel, body), 1);

    // The script "succeeded", so its own condition is now false: the line drops out entirely.
    std::fs::write(root.join("vars"), "enrolled = yes\n").unwrap();
    let state = resolve(&kernel).await;
    assert!(!state.has_execs(), "a false `when` must drop the line");
    kernel
        .app
        .execs()
        .apply(&state, shall::model::exec::Verb::Sync)
        .await
        .expect("no-op");

    // The count survives — this is what stops a flapping condition re-running it.
    assert_eq!(runs_of(&kernel, body), 1, "the ledger row was lost");
}

/// The command Shall issues for a script, matching `App::run_exec_script`, so a test can prime
/// the mock executor to fail exactly that invocation.
fn exec_command_for(path: &std::path::Path) -> String {
    let script = path.to_string_lossy().to_string();
    if cfg!(windows) {
        format!(
            "powershell -NoProfile -ExecutionPolicy Bypass -File {}",
            script
        )
    } else {
        format!("sh {}", script)
    }
}

/// A script that failed did not happen. Recording it would mean the next sync skips a step
/// that never completed — the quiet half-configured machine this whole model exists to avoid.
#[tokio::test]
async fn a_failed_script_is_not_recorded_and_runs_again() {
    let kernel = TestKernel::new().await;
    let body = "exit 1\n";
    declare_exec(&kernel, "exec:./fails.sh", "./fails.sh", body).await;
    approve(&kernel, "./fails.sh", body);

    let script = kernel.app.config.config_root().join("./fails.sh");
    kernel.mock_executor.set_response(
        &exec_command_for(&script),
        Err(shall::core::Error::command_failed("the script exited 1")),
    );

    let state = resolve(&kernel).await;
    let err = kernel
        .app
        .execs()
        .apply(&state, shall::model::exec::Verb::Sync)
        .await
        .expect_err("a failing script must surface, not be swallowed");
    assert!(err.to_string().contains("exited 1"), "{}", err);
    assert_eq!(
        runs_of(&kernel, body),
        0,
        "a script that failed was recorded as having run, so it will never be retried"
    );
}

// ---------------------------------------------------------------------------
// U3 — what removing an `exec:` line means.
// ---------------------------------------------------------------------------

/// A line that declared `@undo=` has it run when the line goes away, and is then forgotten.
/// The undo is read from the LEDGER, not from the config — by the time it is needed the
/// declaration has been deleted, so reading the files would find nothing and do nothing.
#[tokio::test]
async fn removing_an_exec_runs_the_undo_it_declared() {
    let kernel = TestKernel::new().await;
    let body = "echo enrol\n";
    declare_exec(
        &kernel,
        "exec:./enrol.sh@undo=echo unenrol",
        "./enrol.sh",
        body,
    )
    .await;
    approve(&kernel, "./enrol.sh", body);

    let state = resolve(&kernel).await;
    kernel
        .app
        .execs()
        .apply(&state, shall::model::exec::Verb::Sync)
        .await
        .unwrap();
    assert_eq!(runs_of(&kernel, body), 1);

    // The line is deleted — the declaration no longer exists anywhere.
    let root = kernel.app.config.config_root();
    std::fs::write(root.join("modules/tools.txt"), "# gone\n").unwrap();
    let state = resolve(&kernel).await;
    assert!(!state.has_execs());

    kernel
        .app
        .execs()
        .apply(&state, shall::model::exec::Verb::Sync)
        .await
        .unwrap();

    let calls = kernel.mock_executor.get_calls().await;
    assert!(
        calls.iter().any(|c| c.contains("unenrol")),
        "the undo did not run: {:?}",
        calls
    );
    // Forgotten, so it does not run again on the next sync.
    assert_eq!(
        runs_of(&kernel, body),
        0,
        "the row survived a completed undo"
    );
}

/// A script that declared no `@undo=` is forgotten and nothing is run. Shall cannot invent an
/// inverse for a script, and pretending to would be worse than saying nothing.
#[tokio::test]
async fn removing_an_exec_without_an_undo_runs_nothing() {
    let kernel = TestKernel::new().await;
    let body = "echo once\n";
    declare_exec(&kernel, "exec:./once.sh", "./once.sh", body).await;
    approve(&kernel, "./once.sh", body);

    let state = resolve(&kernel).await;
    kernel
        .app
        .execs()
        .apply(&state, shall::model::exec::Verb::Sync)
        .await
        .unwrap();
    let before = kernel.mock_executor.get_calls().await.len();

    std::fs::write(
        kernel.app.config.config_root().join("modules/tools.txt"),
        "# gone\n",
    )
    .unwrap();
    let state = resolve(&kernel).await;
    kernel
        .app
        .execs()
        .apply(&state, shall::model::exec::Verb::Sync)
        .await
        .unwrap();

    assert_eq!(
        kernel.mock_executor.get_calls().await.len(),
        before,
        "something ran for a line that declared no undo"
    );
    assert_eq!(runs_of(&kernel, body), 0, "the row was not dropped");
}

/// A `when` that went false is NOT a removal — the line is still declared, so its undo must
/// not run. This is XIII.3's three-state rule meeting U3: getting it wrong un-enrols the TPM
/// on the sync after the one that enrolled it.
#[tokio::test]
async fn a_false_when_does_not_run_the_undo() {
    let kernel = TestKernel::new().await;
    let body = "echo enrol\n";
    let root = kernel.app.config.config_root();
    std::fs::write(root.join("./enrol.sh"), body).unwrap();
    approve(&kernel, "./enrol.sh", body);
    std::fs::write(root.join("vars"), "enrolled = no\n").unwrap();
    std::fs::write(
        root.join("modules/tools.txt"),
        "when $enrolled == no {\n  exec:./enrol.sh@undo=echo unenrol\n}\n",
    )
    .unwrap();
    std::fs::write(root.join("profiles/Main"), "use tools\n").unwrap();

    let state = resolve(&kernel).await;
    kernel
        .app
        .execs()
        .apply(&state, shall::model::exec::Verb::Sync)
        .await
        .unwrap();
    assert_eq!(runs_of(&kernel, body), 1);

    // The script succeeded, so its own condition is now false — but the LINE is still there.
    std::fs::write(root.join("vars"), "enrolled = yes\n").unwrap();
    let state = resolve(&kernel).await;
    kernel
        .app
        .execs()
        .apply(&state, shall::model::exec::Verb::Sync)
        .await
        .unwrap();

    let calls = kernel.mock_executor.get_calls().await;
    assert!(
        !calls.iter().any(|c| c.contains("unenrol")),
        "a false `when` ran the undo — the script would un-do itself every sync: {:?}",
        calls
    );
    assert_eq!(runs_of(&kernel, body), 1, "the count was lost");
}

/// `H6` — a step says which verb runs it, and neither verb runs the other's steps.
///
/// **Both directions, because the whole ruling is that this is not inherited.** A widening that
/// only proved "`upgrade` runs the step" would pass just as well if `upgrade` had been taught to
/// run *every* `exec:` — which is the change that was rejected, and which would hand a verb that
/// has never executed a user script every script in every manifest that already exists. So the
/// assertion that matters most is the silent one: a plain line, approved and ready, that
/// `upgrade` leaves alone.
#[tokio::test]
async fn a_step_names_the_verb_that_runs_it_and_the_other_verb_leaves_it_alone() {
    use shall::model::exec::Verb;

    let kernel = TestKernel::new().await;
    let root = kernel.app.config.config_root();
    std::fs::create_dir_all(root.join("bin")).unwrap();

    let plain = "echo plain\n";
    let stepped = "echo firmware\n";
    std::fs::write(root.join("bin/plain.sh"), plain).unwrap();
    std::fs::write(root.join("bin/firmware.sh"), stepped).unwrap();
    std::fs::write(
        root.join("modules/tools.txt"),
        // **Both `@runs=always`, and that is load-bearing.** With the default run-once
        // ceiling the plain line's count stays 1 whether or not `upgrade` reached for it, so
        // the assertion below passed against the very widening it exists to reject. Watched
        // doing exactly that before this option was added.
        "exec:./bin/plain.sh @runs=always\nexec:./bin/firmware.sh @runs=always,on=upgrade\n",
    )
    .unwrap();
    std::fs::write(root.join("profiles/Main"), "use tools\n").unwrap();
    approve(&kernel, "./bin/plain.sh", plain);
    approve(&kernel, "./bin/firmware.sh", stepped);

    // `sync` runs the plain line and not the upgrade-only one.
    let state = resolve(&kernel).await;
    kernel
        .app
        .execs()
        .apply(&state, Verb::Sync)
        .await
        .expect("sync runs its steps");
    assert_eq!(
        runs_of(&kernel, plain),
        1,
        "`sync` did not run a plain line"
    );
    assert_eq!(
        runs_of(&kernel, stepped),
        0,
        "`sync` ran a step declared `@on=upgrade`, so the option decides nothing"
    );

    // `upgrade` runs the step and does NOT reach for the plain line — the half that would still
    // pass if the verb had simply been widened to every script.
    let state = resolve(&kernel).await;
    kernel
        .app
        .execs()
        .apply(&state, Verb::Upgrade)
        .await
        .expect("upgrade runs its steps");
    assert_eq!(
        runs_of(&kernel, stepped),
        1,
        "`upgrade` did not run a step that named it, which is the whole finding"
    );
    assert_eq!(
        runs_of(&kernel, plain),
        1,
        "`upgrade` ran a script that never asked for it — approving a line for `sync` is not \
         approving every verb to run it"
    );
}

/// `@on=both` is the third case, and it is not two options written twice.
#[tokio::test]
async fn a_step_can_belong_to_both_verbs() {
    use shall::model::exec::Verb;

    let kernel = TestKernel::new().await;
    let body = "echo both\n";
    declare_exec(
        &kernel,
        "exec:./bin/both.sh @runs=always,on=both",
        "./bin/both.sh",
        body,
    )
    .await;
    approve(&kernel, "./bin/both.sh", body);

    let state = resolve(&kernel).await;
    kernel.app.execs().apply(&state, Verb::Sync).await.unwrap();
    assert_eq!(runs_of(&kernel, body), 1);

    let state = resolve(&kernel).await;
    kernel
        .app
        .execs()
        .apply(&state, Verb::Upgrade)
        .await
        .unwrap();
    assert_eq!(
        runs_of(&kernel, body),
        2,
        "`@on=both` ran under one verb and not the other"
    );
}

/// `H8` — a catalogued step runs from a name, with no script and no approval.
///
/// **The approval asymmetry is the assertion, not a side effect.** A script the user writes is
/// refused until `shall lock` has seen it, and `an_unapproved_script_refuses_the_sync_and_never_
/// runs` above pins that. A shipped step is a row compiled into this binary — the same status
/// `builtin_backends.toml` and `firewall_adapters.toml` have, whose header settled the question
/// in those words: *"this file is compiled into the binary, so there is no II.12 question to ask
/// about it."* If a catalogued step needed approving too, the catalogue would buy nothing: the
/// user would still have to go and look at something before it ran.
#[tokio::test]
async fn a_catalogued_step_runs_from_its_name_without_a_script_or_an_approval() {
    use shall::model::exec::Verb;

    let kernel = TestKernel::new().await;
    let root = kernel.app.config.config_root();
    std::fs::write(root.join("modules/tools.txt"), "exec:step/rustup\n").unwrap();
    std::fs::write(root.join("profiles/Main"), "use tools\n").unwrap();
    // Deliberately no `approve(...)`, and deliberately no file at `<config>/step/rustup`.

    let state = resolve(&kernel).await;
    assert!(state.has_execs(), "the step line did not reach the model");

    // The row says `on = "upgrade"`, so `sync` must leave it alone without the user writing
    // `@on=` at all — that default is what makes a name shorter than a script.
    kernel
        .app
        .execs()
        .apply(&state, Verb::Sync)
        .await
        .expect("sync runs its own steps");
    let calls = kernel.mock_executor.get_calls().await;
    assert!(
        !calls.iter().any(|c| c.starts_with("rustup")),
        "`sync` ran an upgrade step, so the row's own `on` decided nothing: {calls:?}"
    );

    kernel
        .app
        .execs()
        .apply(&state, Verb::Upgrade)
        .await
        .expect("a shipped step needs no approval — that is the whole point of shipping it");
    let calls = kernel.mock_executor.get_calls().await;
    assert!(
        calls.iter().any(|c| c == "rustup update"),
        "`upgrade` did not run the catalogued step: {calls:?}"
    );
}

/// A step for a tool this machine does not have is skipped, and says so.
///
/// One config, many machines: the laptop has `rustup` and the server does not, and the server
/// has *nothing to do* rather than *something to report*. A hard failure here would make a
/// shared config unusable, which is the opposite of what a catalogue is for.
#[tokio::test]
async fn a_step_whose_tool_is_absent_is_skipped_rather_than_failed() {
    use shall::model::exec::Verb;

    let kernel = TestKernel::new().await;
    kernel.mock_executor.set_command_exists("rustup", false);
    let root = kernel.app.config.config_root();
    std::fs::write(root.join("modules/tools.txt"), "exec:step/rustup\n").unwrap();
    std::fs::write(root.join("profiles/Main"), "use tools\n").unwrap();

    let state = resolve(&kernel).await;
    kernel
        .app
        .execs()
        .apply(&state, Verb::Upgrade)
        .await
        .expect("an absent tool is not an error");
    let calls = kernel.mock_executor.get_calls().await;
    assert!(
        !calls.iter().any(|c| c.starts_with("rustup")),
        "a step ran for a tool that is not on this machine: {calls:?}"
    );
}

/// A name the catalogue does not have is refused when the config is read, not when it runs.
///
/// `exec:step/rustupp` would otherwise reach the runtime as *"cannot read the script at
/// <config>/step/rustupp"* — sending a reader to look for a file they never meant to write. The
/// refusal names what this machine actually offers, because a catalogue you cannot discover from
/// its own error is one you read the source for.
#[tokio::test]
async fn an_unknown_step_name_is_refused_with_the_list_of_real_ones() {
    let kernel = TestKernel::new().await;
    let root = kernel.app.config.config_root();
    std::fs::write(root.join("modules/tools.txt"), "exec:step/rustupp\n").unwrap();
    std::fs::write(root.join("profiles/Main"), "use tools\n").unwrap();

    shall::app::sync::resolver::new_resolution();
    let err = shall::app::sync::resolver::StateResolver::new(
        &kernel.app.config,
        kernel.app.registry.clone(),
        false,
    )
    .await
    .resolve_model()
    .await
    .expect_err("an unknown step name is a configuration error");
    let msg = err.to_string();
    assert!(msg.contains("names no step Shall ships"), "{msg}");
    assert!(
        msg.contains("rustup"),
        "the refusal did not list what this machine offers: {msg}"
    );
}
