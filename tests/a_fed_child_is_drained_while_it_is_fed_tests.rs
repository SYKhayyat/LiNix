//! **A payload larger than the pipe buffer does not deadlock the run.**
//!
//! `core::supervise::supervised_output_fed` used to write the whole feed to the child's stdin
//! *before* `wait_watched` was called — and `wait_watched` is what spawns the two tasks that
//! drain the child. The module said so, and treated it as a constraint callers would honour:
//!
//! > *"The payload is written before the output is drained, so it must be small enough for the
//! > pipe buffer — every caller here sends a JSON fact sheet of a few hundred bytes."*
//!
//! **A real caller violated it.** `Event::OnDrift` feeds `SyncReport`, one entry per install and
//! per removal, each carrying backend, name, version and source — 80–150 bytes of JSON apiece.
//! A plan of ~500 changes is ~50 KiB and ~1000 is ~100 KiB, against a default Linux pipe buffer
//! of 64 KiB; Windows anonymous pipes are commonly smaller. And this repo already knows the case
//! where a fresh config makes *every* installed package a removal.
//!
//! Past that bound, `write_all` blocked on a full pipe while nothing drained the child, the child
//! filled its own output pipe and stopped reading, and neither moved again. **No bound was
//! armed:** `command_idle_timeout()` is passed *into* `wait_watched`, and the idle clock starts
//! inside it, so the deadlock happened strictly before the thing that exists to catch it. The
//! result was a `shall sync` hung for ever with no message — the exact failure `core::supervise`
//! opens by saying supervision exists to prevent.
//!
//! **A `debug_assert` on payload length would not have been the fix.** The constraint is gone,
//! not enforced: the feed is written concurrently with the drain, so there is no size at which
//! the two sides can wedge.

use std::process::Stdio;
use std::time::Duration;

/// Comfortably past a 64 KiB pipe buffer on both sides.
const FEED_BYTES: usize = 512 * 1024;

/// A child that **never reads stdin** and writes more than its own output pipe holds.
///
/// Both halves are load-bearing. A child that ignores stdin but writes nothing cannot wedge
/// anybody: the deadlock needs Shall blocked writing the feed *and* the child blocked writing
/// its output, which is why this prints roughly 200 KiB before it says `done`.
fn a_child_that_ignores_stdin_and_talks() -> tokio::process::Command {
    let mut command = if cfg!(windows) {
        // `cmd`'s `echo` is a builtin, so 4096 of them is one process, not 4096.
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/c").arg(
            "for /L %i in (1,1,4096) do @echo \
             xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx & echo done",
        );
        c
    } else {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c")
            .arg("head -c 262144 /dev/zero | tr '\\0' 'x'; echo; echo done");
        c
    };
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_feed_larger_than_the_pipe_buffer_completes() {
    let feed = "x".repeat(FEED_BYTES);

    // The bound is the assertion. Before the fix this never returns at all — not slowly, never
    // — so any finite timeout is the whole test. Sixty seconds is far past what the child needs
    // and far short of a CI job's patience.
    let out = tokio::time::timeout(
        Duration::from_secs(60),
        shall::core::supervise::supervised_output_fed(
            a_child_that_ignores_stdin_and_talks(),
            "a hook that ignores stdin",
            false,
            &feed,
        ),
    )
    .await
    .expect("the fed child deadlocked: the feed is being written before the drain starts again")
    .expect("the child should have run");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("done"),
        "the child ran but its last line was lost; the drain is not collecting what it reads"
    );
    assert!(
        stdout.len() > 64 * 1024,
        "the child wrote well over a pipe buffer and only {} bytes were collected — the pump \
         stopped reading, which is the other way this deadlocks",
        stdout.len()
    );
}

/// **The other side of the same bug: a child that reads its whole feed gets all of it.**
///
/// Fixing a deadlock by dropping the payload would pass the test above and break every hook.
/// This asserts the bytes actually arrive, at a size the old ordering could not have delivered.
///
/// Unix only: it needs a program that counts its stdin and starts in milliseconds. `wc -c` is
/// that; the Windows equivalent is a PowerShell start-up per run, which buys a slower test for
/// the same assertion the deadlock test above already makes on both platforms.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_child_that_reads_gets_the_whole_feed() {
    let mut command = tokio::process::Command::new("sh");
    command
        .arg("-c")
        .arg("wc -c")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let feed = "y".repeat(FEED_BYTES);
    let out = tokio::time::timeout(
        Duration::from_secs(60),
        shall::core::supervise::supervised_output_fed(
            command,
            "a hook that reads stdin",
            false,
            &feed,
        ),
    )
    .await
    .expect("the fed child deadlocked")
    .expect("the child should have run");

    let counted: usize = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("`wc -c` prints a byte count");
    assert_eq!(
        counted, FEED_BYTES,
        "the hook was handed {counted} bytes of a {FEED_BYTES}-byte payload; a feed that is \
         silently truncated is worse than one that hangs, because nothing says so"
    );
}
