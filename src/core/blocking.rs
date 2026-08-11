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

/// The same, for a command whose streams are inherited and whose answer is its exit status.
pub fn command_status(
    command: &mut std::process::Command,
) -> std::io::Result<std::process::ExitStatus> {
    on_the_terminal(|| command.status())
}

#[cfg(test)]
mod tests {
    use super::*;

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
