//! **Watching a child process, and killing it when the run that owns it goes away.**
//!
//! Split out of `executor.rs` because it is a subject in its own right and nothing else in that
//! file is about it: every function here exists because *dropping a future does not kill the
//! process it spawned*. A worker whose task is aborted — a failed node, the global timeout, a
//! Ctrl-C — leaves an `apt install` running against the same dpkg lock the rollback is about to
//! take, and whatever that install completes is in no history that could compensate it.
//!
//! [`Stopping`] is the piece that makes it structural rather than remembered: the child is
//! killed by a `Drop`, so a caller cannot forget, and a path added later inherits it.

use crate::core::executor::{command_idle_timeout, RawExecutor};
use crate::core::{Error, Result};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::Output as StdOutput;
use std::process::Stdio;
use tokio::process::Command;

/// Lock directories already created, so `create_dir_all` runs once per directory per process
/// instead of once per exclusive command.
pub(crate) static LOCK_DIRS: once_cell::sync::Lazy<dashmap::DashSet<PathBuf>> =
    once_cell::sync::Lazy::new(dashmap::DashSet::new);

/// A spawned child that is **asked** to stop before it is made to.
///
/// **SIGKILL is not a way to stop a package manager.** It cannot be caught, so nothing gets to
/// run: `dpkg`'s database is left mid-write, `pacman`'s `db.lck` is left on disk, and the next
/// Shall run on that machine — and every `apt` the user types afterwards — fails on a lock whose
/// owner is dead. That is the wedged machine `shall heal` exists to unwedge, and Shall was
/// creating it itself. SIGTERM *is* caught: apt rolls the transaction back, pacman unlinks its
/// lock, and the machine is left usable.
///
/// **And Shall's child is usually `sudo`, not the manager.** `sudo` forwards a SIGTERM to the
/// command it runs; a SIGKILL kills `sudo` alone and leaves the manager running as root with its
/// parent gone — an orphan still holding the lock, which is precisely the state that makes the
/// next run fail with a lock nobody appears to hold.
///
/// Windows has no catchable termination signal for a console process, so there `kill_on_drop`
/// keeps the job and this type only carries the child.
pub(crate) struct Stopping {
    pub(crate) child: tokio::process::Child,
}

impl Stopping {
    pub(crate) fn new(child: tokio::process::Child) -> Self {
        Self { child }
    }

    /// SIGTERM, then wait, then SIGKILL only if it is still there.
    ///
    /// The grace is the point: a manager that is cleaning up is doing the thing that keeps the
    /// machine usable, and hurrying it undoes the whole exercise.
    pub(crate) async fn stop(&mut self) {
        #[cfg(unix)]
        {
            if self.request_stop()
                && tokio::time::timeout(RawExecutor::TERMINATION_GRACE, Box::pin(self.child.wait()))
                    .await
                    .is_ok()
            {
                return;
            }
        }
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }

    /// Send SIGTERM. `false` when there is nothing to send it to — the child has already exited,
    /// or this is not Unix.
    #[cfg(unix)]
    fn request_stop(&mut self) -> bool {
        match self.child.id() {
            Some(pid) => {
                // SAFETY: `kill(2)` with a pid tokio still owns. The child has not been reaped —
                // this type owns it and no `wait` has returned — so the pid cannot have been
                // reused by another process.
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) == 0 }
            }
            None => false,
        }
    }
}

/// The abort path — a worker whose task was cancelled, the global timeout — reaches the child
/// only through `Drop`, which cannot wait for anything. It sends the signal that lets a manager
/// clean up and does not stay to watch: a package manager finishing its own transaction after
/// Shall has stopped caring is the *good* outcome, and the run after it now waits for that
/// manager rather than failing on its lock.
#[cfg(unix)]
impl Drop for Stopping {
    fn drop(&mut self) {
        if self.child.try_wait().is_ok_and(|s| s.is_some()) {
            return;
        }
        self.request_stop();
    }
}

/// Run an outside tool Shall does not otherwise supervise, under the same ownership and the same
/// bound as everything else it spawns.
///
/// **A child spawned outside `RawExecutor` used to have neither.** Awaiting `Command::output()`
/// and then dropping that future does not kill the process — tokio detaches it — so every
/// abandoned operation left a program running with nothing watching it: a `generate:` command
/// after the sync that asked for it failed, a hook after the node that fired it was rolled back,
/// a secret decrypt after its own timeout expired, under a comment promising the process would
/// not be left hung. And none of them was bounded at all, so a `generate:` command that blocks
/// on a prompt blocks every sync on that machine, forever, with no message.
///
/// `stdin` is closed unless `feed` gives it something. A tool that needs one otherwise is a tool
/// asking a question nobody will answer, and a child sharing Shall's stdin eats input meant for
/// Shall.
///
/// `mirror` echoes the tool's output to stderr as it arrives, for the callers whose tool used to
/// inherit the terminal — a hook and the bisect oracle both printed as they ran, and capturing
/// that silently would be a regression dressed as a fix. Never stdout: that carries Shall's own
/// answer, and a child's chatter interleaved with it is not parseable by whoever piped us.
pub async fn supervised_output(command: Command, what: &str, mirror: bool) -> Result<StdOutput> {
    supervise(command, what, mirror, None).await
}

/// The same, for a tool that is handed something on stdin and then sees it close.
///
/// The payload is written before the output is drained, so it must be small enough for the pipe
/// buffer — every caller here sends a JSON fact sheet of a few hundred bytes. A large one would
/// deadlock against a child that will not read until it has written.
pub async fn supervised_output_fed(
    command: Command,
    what: &str,
    mirror: bool,
    feed: &str,
) -> Result<StdOutput> {
    supervise(command, what, mirror, Some(feed)).await
}

/// The other door: a child that **takes the terminal**, run to completion and owned all the same.
///
/// `shall run`, the ephemeral shell, an interpreter a user is watching. Its streams are inherited
/// rather than captured, because the point is that the person is looking at it, and there is no
/// idle bound for the same reason — a shell sitting at a prompt is not a hung command. What it
/// does get is an owner: abandoning the future used to leave the child holding the terminal after
/// Shall was gone, which is a mess nobody can attribute to anything.
pub async fn supervised_status(
    mut command: Command,
    what: &str,
) -> Result<std::process::ExitStatus> {
    #[cfg(windows)]
    command.kill_on_drop(true);
    #[cfg(unix)]
    command.kill_on_drop(false);
    let child = command
        .spawn()
        .map_err(|e| Error::command_failed(format!("could not start {what}: {e}")))?;
    let mut child = Stopping::new(child);
    child
        .child
        .wait()
        .await
        .map_err(|e| Error::command_failed(format!("waiting for {what}: {e}")))
}

async fn supervise(
    mut command: Command,
    what: &str,
    mirror: bool,
    feed: Option<&str>,
) -> Result<StdOutput> {
    command
        .stdin(if feed.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.kill_on_drop(true);
    #[cfg(unix)]
    command.kill_on_drop(false);
    let mut child = command
        .spawn()
        .map_err(|e| Error::command_failed(format!("could not start {what}: {e}")))?;
    if let (Some(feed), Some(mut pipe)) = (feed, child.stdin.take()) {
        use tokio::io::AsyncWriteExt;
        // A tool that ignores stdin closes the pipe, and writing to a closed pipe is that tool's
        // choice rather than an error: it was told, and it may not care.
        let _ = pipe.write_all(feed.as_bytes()).await;
        let _ = pipe.shutdown().await;
    }
    RawExecutor::wait_watched(
        child,
        what,
        mirror && std::io::stderr().is_terminal(),
        command_idle_timeout(),
    )
    .await
}
