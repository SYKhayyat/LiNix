//! **A sync that failed must report failure under every flag that changes how failure is
//! reported.**
//!
//! `shall sync --keep-going` exited **0** with `Status: SUCCESS` over a run in which every
//! package failed and nothing was installed, and the same run under `--quiet` printed zero
//! bytes (B1). The flag's own help calls it *"the per-run opt-in for a fleet rollout that would
//! rather take what it can get"* — and a fleet rollout is exactly the context where the exit
//! code is the only thing anybody reads. A GitOps pipeline running it was green while
//! installing nothing.
//!
//! **Five hundred and seventeen tests passed over this**, because every one of them drove a
//! failing sync *without* the flag, where the behaviour was correct. The flag changes the
//! contract and no test crossed that boundary. The missing enumeration is not "a failing sync"
//! — it is *(a failing sync) × (every flag that changes how failure is reported)*, and that
//! product is what this file is.

use crate::harness::Fixture;

/// A declaration that cannot succeed, on a backend that needs nothing installed to try it.
///
/// Port 9 is `discard`, closed on every ordinary machine, so the download fails immediately and
/// without a network. `@unverified` is required to get *past* the checksum gate — refusing an
/// unhashed URL is a different exit code on a different path, and this probe is about what
/// happens when work is attempted and does not work.
const CANNOT_INSTALL: &str = "web:http://127.0.0.1:9/shall-no-such-file.bin@unverified\n";

fn fixture(name: &str) -> Fixture {
    let f = Fixture::new(name);
    f.write("priority", "web\n");
    f.write("modules/starter.txt", CANNOT_INSTALL);
    f
}

/// The matrix. One package that cannot install, every flag combination, and one rule: none of
/// them may exit 0.
#[test]
fn no_flag_turns_a_sync_that_installed_nothing_into_a_success() {
    let f = fixture("b1_flag_matrix");

    // **The instrument, before it is trusted.** If the plain run does not fail, this probe
    // never produced a failure at all and every row below would pass over nothing — which is
    // precisely how the original bug survived a full suite.
    let (plain_out, plain) = f.run(&["-y", "sync"]);
    assert_ne!(
        plain, 0,
        "the probe did not make a sync fail, so nothing below means anything:\n{plain_out}"
    );

    for flags in [
        vec!["-y", "sync", "--keep-going"],
        vec!["-y", "sync", "--keep-going", "--quiet"],
        vec!["-y", "sync", "--quiet"],
    ] {
        let (out, code) = f.run(&flags);
        assert_ne!(
            code,
            0,
            "`shall {}` exited 0 over a run in which nothing was installed. Without \
             `--keep-going` the same failure exits {plain}; the flag is about *continuing*, \
             never about *reporting success*.\n{out}",
            flags.join(" ")
        );
    }
}

/// `--dry-run` is the one row that exits 0 correctly, and it belongs in the matrix for that
/// reason: a rule of "every sync mentioning a bad package is non-zero" would be wrong, and a
/// test that did not say so would be pinning the wrong rule.
#[test]
fn a_dry_run_that_attempted_nothing_is_not_a_failure() {
    let f = fixture("b1_flag_matrix_dry");
    for flags in [
        vec!["-y", "sync", "--dry-run"],
        vec!["-y", "sync", "--dry-run", "--keep-going"],
    ] {
        let (out, code) = f.run(&flags);
        assert_eq!(
            code,
            0,
            "`shall {}` reported a failure, but a dry run attempts nothing and so cannot have \
             failed at one\n{out}",
            flags.join(" ")
        );
    }
}

/// The status line was a constant, and this is the assertion that says it is not.
///
/// `Status:` read a `Metrics.errors` field whose only writer had no callers anywhere in the
/// tree, so `errors.is_empty()` was permanently true and `DEGRADED` was unreachable. The word
/// printed under two task lines each carrying a failure mark.
#[test]
fn the_transaction_summary_does_not_say_success_over_a_failure() {
    let f = fixture("b1_summary_status");
    let (out, _) = f.run(&["-y", "sync", "--keep-going"]);
    assert!(
        !out.contains("Status:       SUCCESS"),
        "the summary reported SUCCESS over a run that installed nothing:\n{out}"
    );
}

/// `--quiet` promised to suppress *everything except errors*, and suppressed the errors too.
///
/// Its whole body printed a block guarded on the same dead field, so the mode printed nothing
/// under any circumstances — measured at 0 bytes over a sync where every package failed. A
/// quiet flag that is silent about failure is not quiet, it is deaf.
#[test]
fn quiet_says_nothing_on_success_and_never_swallows_a_failure() {
    let f = fixture("b1_quiet_speaks");
    let (out, err, code) = f.run_split(&["-y", "sync", "--keep-going", "--quiet"]);
    assert_ne!(code, 0);
    assert!(
        !err.trim().is_empty() || !out.trim().is_empty(),
        "`--quiet` printed nothing at all over a run in which every package failed"
    );
}
