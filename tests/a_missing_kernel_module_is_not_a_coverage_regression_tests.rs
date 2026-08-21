//! A host that cannot carry a kernel module must not be recorded as having lost coverage.
//!
//! The harness already knew this fact and already drew the distinction — once. Section 13c
//! detects a kernel with no btrfs, no device-mapper or no out-of-tree ZFS module, sets a flag,
//! and `no_lifecycle_reason` reads that flag so the coverage-gap gate can tell "this machine
//! cannot" from "this run could not" (Q17). The real-lifecycle ratchet, built separately, drew
//! the same distinction through a different channel — `be-life-unmeasured`, for an install that
//! failed transiently and did not clear on a retry — and the two never met. So a correct run on
//! a kernel without ZFS did seven lifecycles against a recorded floor of eight and reported a
//! coverage collapse, on a tree where nothing had collapsed.
//!
//! The repair is to record the kernel-absent backends in their own channel and count them
//! toward the floor, which is what these tests pin. They are text gates over the shell script:
//! they check the wiring is present, not that a container behaves. Only the harness can do the
//! second, and it needs a kernel that lacks a module to do it — which is the whole difficulty.

use std::path::PathBuf;

fn harness() -> String {
    let p: PathBuf =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docker/integration/run-in-container.sh");
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
        .replace("\r\n", "\n")
}

/// The three storage backends whose availability is a property of the kernel, not of the image.
const KERNEL_BOUND: [&str; 3] = ["btrfs", "lvm", "zfs"];

/// The self-test. Every assertion below is over a scan of one file; if the flags it keys on are
/// gone or renamed, the scan finds nothing and would pass by finding nothing.
#[test]
fn the_kernel_absence_flags_are_still_what_this_scan_reads() {
    let h = harness();
    for be in KERNEL_BOUND {
        let flag = format!("STORAGE_{}_NO_KERNEL", be.to_uppercase());
        assert!(
            h.matches(&flag).count() >= 3,
            "{flag} appears {} time(s) in the harness; it should be declared, set in 13c and \
             read by no_lifecycle_reason. This scan has stopped matching the script.",
            h.matches(&flag).count()
        );
    }
}

/// The finding itself, swept across the family rather than pinned to the one backend that
/// reported it. `zfs` is what went red here, on a WSL kernel with no out-of-tree module; the
/// identical hole sat under `btrfs` and `lvm` for any host missing those.
#[test]
fn every_kernel_bound_backend_records_itself_when_its_module_is_absent() {
    let h = harness();
    for be in KERNEL_BOUND {
        let flag = format!("STORAGE_{}_NO_KERNEL=1", be.to_uppercase());
        let at = h
            .find(&flag)
            .unwrap_or_else(|| panic!("13c no longer sets {flag}, so nothing marks {be} absent"));
        // The recording belongs beside the detection: a later re-derivation would be a second
        // place that has to agree about which backends are kernel-bound.
        let window = &h[at..(at + 200).min(h.len())];
        assert!(
            window.contains(&format!("echo {be} >> \"$LEDGER/be-life-nokernel\"")),
            "13c marks {be} as having no kernel module and never records it, so the ratchet \
             counts the missing lifecycle as coverage this host used to have and lost.\n\
             saw: {window}"
        );
    }
}

/// Recording it is worth nothing if the floor comparison ignores the record.
#[test]
fn the_ratchet_counts_a_kernel_absent_backend_toward_the_floor() {
    let h = harness();
    assert!(
        h.contains("MEASURABLE=$((LIFECYCLES + EXCUSED + NOKERNEL))"),
        "the ratchet's MEASURABLE total does not include the kernel-absent backends, so a host \
         without a module still reads as a coverage regression"
    );
    // `NOKERNEL` goes in ungated and `EXCUSED` does not, and the asymmetry is the point. A
    // missing kernel module is a fact about the machine that no retry and no repair to this
    // repository can change, so it is excused unconditionally. An ecosystem that broke
    // upstream is excused only against a dated `drift` line that expires (M1) — otherwise the
    // fix for one silence installs another, and the coverage leaves while every run stays
    // green.
    assert!(
        h.contains("EXCUSED=$((EXCUSED + 1))") && h.contains(r#"drift_verdict "$HOST_CLASS""#),
        "EXCUSED no longer comes from the drift register, so an unmeasurable backend is \
         counted toward the floor for ever and the expiry gates nothing"
    );
    assert!(
        h.contains(r#"NOKERNEL=$(grep -c . "$LEDGER/be-life-nokernel.u")"#),
        "nothing counts $LEDGER/be-life-nokernel, so NOKERNEL is empty and the total above is \
         arithmetic over a variable that was never set"
    );
    assert!(
        h.contains(r#": > "$LEDGER/be-life-nokernel""#),
        "the be-life-nokernel ledger is never truncated at startup; every other ledger is, and \
         one that is not carries the previous run's answer"
    );
}

/// **The excuse has to be audible.** A run that lowers its own bar and says nothing reads
/// exactly like a run that cleared the bar.
#[test]
fn a_run_that_excuses_coverage_says_which_excuse_it_used() {
    let h = harness();
    assert!(
        h.contains("no kernel module on this host:"),
        "the ratchet excuses kernel-absent backends without naming them; silent truncation \
         reads as covered everything when it did not"
    );
    // And the two channels stay distinguishable. They have opposite remedies: a transient
    // install clears on the next run, a missing module never clears on this host at all.
    assert!(
        h.contains("unmeasurable:") && h.contains("no kernel module on this host:"),
        "the two excuse channels have collapsed into one message, so the report no longer says \
         whether running again would measure the shortfall"
    );
}

/// The floor file itself is never the repair. This is the one edit that file exists to make
/// visible in a diff, and the fix above exists so that nobody reaches for it.
#[test]
fn the_recorded_floor_for_the_storage_image_was_not_lowered() {
    let p: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/lifecycle-floor.txt");
    let text = std::fs::read_to_string(&p).expect("lifecycle-floor.txt is readable");
    let line = text
        .lines()
        .find(|l| l.trim_start().starts_with("container-linux-storage-local "))
        .expect("the storage image has a recorded floor");
    let n: usize = line
        .rsplit_once(' ')
        .and_then(|(_, n)| n.trim().parse().ok())
        .expect("the storage floor is a number");
    assert!(
        n >= 8,
        "container-linux-storage-local records {n}, and CI run 32132445664 measured 8 with zfs \
         completing a real lifecycle for the first time anywhere. A host that cannot run zfs is \
         excused by the ratchet, not by editing this number down."
    );
}
