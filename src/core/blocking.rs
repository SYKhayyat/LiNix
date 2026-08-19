//! Work that blocks a thread, run from a command that is `async`.
//!
//! **Shall's slowest waits are not on the network — they are on a person, or on a program.** A
//! confirm sits at a prompt until someone types; a TUI reads keys for as long as they browse;
//! `git commit` runs after every sync; a `btrfs subvolume snapshot` takes as long as it takes.
//! Every one of those was a plain blocking call reached straight from an `async fn`, which parks
//! a tokio worker for the whole of it. One worker of several is survivable rather than fatal,
//! which is exactly why it lasted this long — and why the next one was written the same way.
//!
//! Two shapes, two primitives:
//!
//! * [`on_the_terminal`] — waiting where the call cannot move: it owns the terminal, or its
//!   caller is synchronous. `block_in_place` moves the runtime's other tasks off this worker and
//!   lets the call stay where it is. [`command_output`] and [`command_status`] are that, spelled
//!   for the `std::process::Command` sites.
//! * [`off_the_runtime`] — work that *can* move, because nothing about it is tied to this
//!   thread: unpacking an archive, hashing a file, waiting out a file lock. That belongs on the
//!   blocking pool, where it neither parks a worker nor competes with one.

/// Wait where the call cannot move, without parking a runtime worker.
///
/// `block_in_place` panics on a current-thread runtime, and Shall builds one of those as a
/// fallback in `rhai_stdlib`, so the flavour is asked rather than assumed. Nothing reaches a
/// prompt from there today; a check that costs nothing is cheaper than a panic that depends on
/// that staying true — and cheaper still than one that only fires for whoever writes the hook
/// that does reach it.
pub fn on_the_terminal<T>(wait: impl FnOnce() -> T) -> T {
    use tokio::runtime::{Handle, RuntimeFlavor};
    match Handle::try_current().map(|h| h.runtime_flavor()) {
        Ok(RuntimeFlavor::MultiThread) => tokio::task::block_in_place(wait),
        // Either no runtime at all — a unit test, `main` before it starts one — or the
        // single-threaded fallback, where there is no other worker to protect.
        _ => wait(),
    }
}

/// Do blocking work on the blocking pool instead of on a runtime worker.
///
/// For work with no tie to this thread: `tar`, `flate2`, `xz2`, `zip`, a `sha2` pass over a
/// downloaded file, a `flock` wait. These are not milliseconds — a release tarball is seconds to
/// minutes, and the data-directory lock waits up to two — and they ran on the same worker that
/// was supposed to be driving everything else.
pub async fn off_the_runtime<T, F>(work: F) -> crate::core::Result<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        // A `JoinError` here is the work having panicked. It is reported rather than resumed,
        // because a panic that unwinds into a package manager's retry loop is a panic nobody
        // can attribute to the thing that caused it.
        .map_err(|e| crate::core::Error::Other(format!("a background step failed: {e}")))
}

/// Run a `std::process::Command` to completion without parking a runtime worker.
///
/// **The third door.** `core::executor`'s two are for `tokio` children, whose hazard is that
/// dropping the future detaches the process. A `std::process::Command` has the opposite shape:
/// it cannot be abandoned at all, because the call does not return until the child has exited —
/// which is precisely the problem. `git commit` after every sync, a `btrfs subvolume` snapshot,
/// a `--help` probe, an external vars provider: each of them held a worker for its whole run,
/// from an `async fn`, with nothing saying so.
///
/// These stay synchronous rather than becoming `tokio` children because their callers are
/// synchronous — `GitManager` is a sync API used from a dozen places — and rewriting those to
/// async to fix a threading problem would be a far larger change than the problem is.
pub fn command_output(
    command: &mut std::process::Command,
) -> std::io::Result<std::process::Output> {
    on_the_terminal(|| command.output())
}

/// The same, with a bound — for a child whose command line came from a *script*.
///
/// **[`command_output`] returns when the child does and never before**, which is the right
/// shape for `git commit` and for a `--help` probe and the wrong one for the two doors that run
/// user-authored code: an external `vars.<ext>` provider, and `sh()` in the Rhai stdlib. Both of
/// those sit at step 0 of resolution — before any package manager is asked and before any plan
/// exists — so they are on the path of *every* command, `check` and `list` and `plan` included.
/// A `vars.py` reading from a network mount that went away, or a `vars.shall` calling
/// `sh("git fetch")` against an unreachable remote, hung Shall for ever with no output and no
/// way out but Ctrl-C. On a scheduled run there is nobody to press it.
///
/// The reasoning was already written down twice — `events.rs` bounds a hook because it is
/// "arbitrary code fired by a timer, with nobody at the terminal", and `rhai_stdlib`'s
/// operation cap names this exact gap: *"it counts Rhai operations, not seconds — a hook whose
/// `sh()` runs for ten minutes is one."* This is the seconds.
///
/// A **whole-command** bound rather than the idle bound the `tokio` doors use: those watch a
/// stream they are already draining, and a synchronous child's output is drained by the reader
/// threads below, not by the waiter. `0` in `command_idle_timeout_secs` still means no bound,
/// so the escape hatch is the one users already know.
pub fn command_output_bounded(
    command: &mut std::process::Command,
    what: &str,
) -> std::io::Result<std::process::Output> {
    command_output_within(command, what, crate::core::executor::command_idle_timeout())
}

/// The body of [`command_output_bounded`] with the bound passed in.
///
/// Split out so a test can name its own: the process-wide one is a `OnceCell` seeded at
/// startup, and a test that set it would decide the value for every other test in the binary.
fn command_output_within(
    command: &mut std::process::Command,
    what: &str,
    limit: Option<std::time::Duration>,
) -> std::io::Result<std::process::Output> {
    let Some(limit) = limit else {
        return command_output(command);
    };
    on_the_terminal(|| {
        use std::io::Read;
        use std::process::Stdio;
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // Drained on their own threads. A child that fills its output pipe stops writing, and a
        // waiter that is not reading would then be waiting on a child that is waiting on it —
        // a deadlock the timeout would report as a hang the script did not cause.
        let mut out = child.stdout.take();
        let mut err = child.stderr.take();
        let out_thread = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(pipe) = out.as_mut() {
                let _ = pipe.read_to_end(&mut buf);
            }
            buf
        });
        let err_thread = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(pipe) = err.as_mut() {
                let _ = pipe.read_to_end(&mut buf);
            }
            buf
        });

        let deadline = std::time::Instant::now() + limit;
        let status = loop {
            match child.try_wait()? {
                Some(status) => break status,
                None if std::time::Instant::now() >= deadline => {
                    // Killed rather than asked: this is not a package manager mid-transaction
                    // with a database to unwind, it is a script of the user's that stopped
                    // answering, and leaving it running is what "unbounded" already meant.
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!(
                            "{} did not finish within {}s and was stopped. It runs before \
                             anything else Shall does, so a wait here is a wait on every \
                             command. Raise `command_idle_timeout_secs`, or set it to 0 to \
                             remove the bound.",
                            what,
                            limit.as_secs()
                        ),
                    ));
                }
                None => std::thread::sleep(std::time::Duration::from_millis(25)),
            }
        };
        Ok(std::process::Output {
            status,
            stdout: out_thread.join().unwrap_or_default(),
            stderr: err_thread.join().unwrap_or_default(),
        })
    })
}

/// The same, for a command whose streams are inherited and whose answer is its exit status.
pub fn command_status(
    command: &mut std::process::Command,
) -> std::io::Result<std::process::ExitStatus> {
    on_the_terminal(|| command.status())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A script that never finishes is stopped, and the stop says so.**
    ///
    /// The two doors that run user-authored code — an external `vars.<ext>` provider and
    /// `sh()` in the Rhai stdlib — both sit before any package manager is asked, so an
    /// unbounded wait there is an unbounded wait on `check`, `list`, `plan` and every
    /// scheduled `sync`.
    #[test]
    fn a_command_that_never_finishes_is_stopped_and_named() {
        let mut cmd = if cfg!(windows) {
            let mut c = std::process::Command::new("cmd");
            c.args(["/c", "ping -n 30 127.0.0.1 > NUL"]);
            c
        } else {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", "sleep 30"]);
            c
        };
        let started = std::time::Instant::now();
        let err = command_output_within(
            &mut cmd,
            "the wedged provider",
            Some(std::time::Duration::from_millis(300)),
        )
        .expect_err("a command that outlives its bound is not a result");
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        assert!(
            err.to_string().contains("the wedged provider"),
            "the message must name what hung: {err}"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "the bound did not bound anything"
        );
    }

    /// The bound must not cost the ordinary case its output. A command inside the bound comes
    /// back whole — both streams, and the exit status.
    #[test]
    fn a_command_inside_the_bound_returns_its_output() {
        let mut cmd = if cfg!(windows) {
            let mut c = std::process::Command::new("cmd");
            c.args(["/c", "echo hello"]);
            c
        } else {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", "echo hello"]);
            c
        };
        let out = command_output_within(
            &mut cmd,
            "a probe",
            Some(std::time::Duration::from_secs(30)),
        )
        .expect("a fast command");
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hello");
    }

    /// `0` means no bound, all the way down — the escape hatch users already know from
    /// `command_idle_timeout_secs`.
    #[test]
    fn no_bound_still_runs_the_command() {
        let mut cmd = if cfg!(windows) {
            let mut c = std::process::Command::new("cmd");
            c.args(["/c", "echo unbounded"]);
            c
        } else {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", "echo unbounded"]);
            c
        };
        let out =
            command_output_within(&mut cmd, "a probe", None).expect("no bound refuses nothing");
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "unbounded");
    }

    /// The no-runtime case: a plain call, not a panic. Every unit test in this repo is one.
    #[test]
    fn without_a_runtime_it_simply_runs_the_work() {
        assert_eq!(on_the_terminal(|| 6 * 7), 42);
    }

    /// The single-threaded case, which is the one `block_in_place` panics on. `rhai_stdlib`
    /// builds exactly this runtime, so the flavour check is what stands between a hook that
    /// reaches a prompt and an abort.
    #[test]
    fn a_current_thread_runtime_does_not_panic() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a current-thread runtime");
        assert_eq!(rt.block_on(async { on_the_terminal(|| "asked") }), "asked");
    }

    /// And the case it exists for: on a multi-thread runtime the work still runs, and returns.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_multi_thread_runtime_runs_it_in_place() {
        assert_eq!(on_the_terminal(|| "asked"), "asked");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blocking_work_comes_back_from_the_blocking_pool() {
        assert_eq!(off_the_runtime(|| 6 * 7).await.expect("no panic"), 42);
    }

    /// A panic in the work is an error, not a panic in the caller: this runs inside a package
    /// manager's retry loop, and an unwind there is a failure nobody can trace to its cause.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_panic_in_the_work_is_reported_rather_than_resumed() {
        let out = off_the_runtime(|| panic!("the archive was truncated")).await;
        assert!(out.is_err(), "a panicking job must not look like success");
    }

    /// The third door works, and the answer comes back. Run under a multi-thread runtime,
    /// because that is the flavour whose worker was being parked.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_blocking_command_still_answers_through_the_door() {
        let mut cmd = if cfg!(windows) {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "echo through-the-door"]);
            c
        } else {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", "echo through-the-door"]);
            c
        };
        let out = command_output(&mut cmd).expect("the shell ran");
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("through-the-door"));
    }
}
