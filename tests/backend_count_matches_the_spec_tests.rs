//! How many backends are there?
//!
//! Three documents answered and no two agreed: `SPEC.md` said 52, `GRADE-2026-07-28` said 48
//! registered on Windows and 56 on Ubuntu, and the registration list came to something else
//! again. None of them was lying. **"Registered" meant two different things** — how many
//! backends the build *contains*, and how many `create_default_registry` actually *registers*
//! on the host you ran it on, which differs because the OS-native ones sit behind
//! `cfg!(target_os = …)`.
//!
//! The per-host numbers were right and belong in a grade, which is a dated measurement of one
//! machine. The build-wide number belongs in `SPEC.md`, and it was stale — a number in prose is
//! a copy of a fact, and this copy had been wrong long enough that nobody could say which of the
//! three was the stale one. So it is asserted.
//!
//! **Four documents, in fact.** `README.md` states the same number in its own words, and this
//! file gated only `SPEC.md` — so when the spec was corrected from 52 to 62, the README kept the
//! stale one, in the section that exists to say what has and has not actually been driven. A
//! gate aimed at one copy of a fact leaves the other copies exactly as free to drift as they
//! were; it just makes them harder to notice, because the file everyone quotes now agrees with
//! the code.
//!
//! The authority is the argv table in `src/backends/registry.rs`, and it is the right authority
//! for a reason beyond convenience: `os_native_argv_coverage_tests.rs` already fails the build
//! if a registrar has no row there. So "every backend has a row" and "the row count is the
//! backend count" hold each other up — a new backend cannot change one without the other.

use std::path::PathBuf;

/// Read a file of the repo, with line endings normalised where the text enters the scanner.
///
/// **This one passes today by luck.** Its marker spans a line break — `" backends\nexist across
/// all platforms"` — and a CRLF copy of `SPEC.md` contains no such sequence, so the scan would
/// panic that the sentence had been reworded. `SPEC.md` happened to be LF in the working tree
/// where four of its neighbours in `docs/spec/` were CRLF, which is the only reason this gate
/// ran while `grammar_table_matches_the_spec_tests` sat dark. Same fix, same boundary, before
/// the coin lands the other way.
fn read(rel: &str) -> String {
    let p: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
        .replace("\r\n", "\n")
}

/// Backends the build contains, counted from the argv table's rows.
fn backends_in_the_build() -> usize {
    let table = crate::harness::registry_argv_table();
    let n = table.matches("ArgvCase::pkg(").count() + table.matches("ArgvCase::shaped(").count();
    assert!(
        n > 40,
        "counted only {n} argv rows — the scan is broken, not the code"
    );
    n
}

/// The number `README.md` states, read from the one sentence that states it.
///
/// A different sentence from the spec's, deliberately: the README is prose for a user and the
/// spec is a specification, and forcing them to share a wording is how one of them ends up
/// reading badly. What they must share is the number.
fn backends_claimed_by_the_readme() -> usize {
    let readme = read("README.md");
    let marker = " backends.";
    let head = "Shall ships ";
    let at = readme.find(head).unwrap_or_else(|| {
        panic!(
            "README.md no longer contains the sentence this test reads. It must say \
             `Shall ships <N> backends.`; if the wording changed, change it here too rather \
             than deleting the assertion."
        )
    });
    let rest = &readme[at + head.len()..];
    let end = rest.find(marker).unwrap_or_else(|| {
        panic!("README.md says `Shall ships` but not `<N> backends.` after it: {rest:.60}")
    });
    rest[..end].parse().unwrap_or_else(|_| {
        panic!(
            "README.md's backend count is not a number: {:?}",
            &rest[..end]
        )
    })
}

/// The number `SPEC.md` states, read from the one sentence that states it.
fn backends_claimed_by_the_spec() -> usize {
    let spec = read("docs/SPEC.md");
    let marker = " backends\nexist across all platforms";
    let at = spec.find(marker).unwrap_or_else(|| {
        panic!(
            "SPEC.md no longer contains the sentence this test reads. It must say \
             `**<N> backends\\nexist across all platforms**`; if the wording changed, change it \
             here too rather than deleting the assertion."
        )
    });
    let before = &spec[..at];
    let digits: String = before
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits
        .parse()
        .unwrap_or_else(|_| panic!("no number before {marker:?} in SPEC.md; found {digits:?}"))
}

#[test]
fn the_spec_states_the_number_of_backends_the_build_actually_has() {
    let real = backends_in_the_build();
    let claimed = backends_claimed_by_the_spec();
    assert_eq!(
        claimed, real,
        "SPEC.md says {claimed} backends exist across all platforms; the argv table has {real} \
         rows, and every registrar is required to have one.\n\n\
         Update the sentence in SPEC.md. Do NOT update the grade documents — those are dated \
         measurements of one host, they say `registered on Windows`/`on Ubuntu`, and both of \
         those are different questions with different right answers."
    );
}

/// The README's copy of the same fact.
///
/// Split from the spec's assertion rather than folded into it so a failure names which document
/// is stale — the whole difficulty the header describes was that three numbers disagreed and
/// nobody could say which one had moved.
#[test]
fn the_readme_states_the_number_of_backends_the_build_actually_has() {
    let real = backends_in_the_build();
    let claimed = backends_claimed_by_the_readme();
    assert_eq!(
        claimed, real,
        "README.md says Shall ships {claimed} backends; the argv table has {real} rows. The \
         README kept 52 for as long as this file gated only SPEC.md, in the section that exists \
         to state honestly what has been driven."
    );
}

/// The grades measured one host each and said so. That is not the same claim as the spec's, and
/// a future reader must not "reconcile" them into one wrong number — which is how 52 survived.
#[test]
fn a_per_host_count_is_never_the_build_wide_count() {
    let real = backends_in_the_build();
    // Windows omits the Linux- and macOS-native managers; Linux omits the Windows ones. Neither
    // host registers everything, so any host count must be strictly smaller.
    assert!(
        real > 56,
        "the build-wide count ({real}) is not larger than the largest per-host count ever \
         measured (56, Ubuntu). Either a backend was lost, or the two numbers have been \
         conflated again."
    );
}

/// A gate that has never failed is a claim, not a check.
#[test]
fn the_count_scan_can_actually_fail() {
    let spec = read("docs/SPEC.md");
    assert!(
        spec.contains(" backends\nexist across all platforms"),
        "the sentence the scan reads is gone"
    );
    // The scan reads digits immediately before the marker, so a changed number is seen.
    assert_eq!(backends_claimed_by_the_spec(), backends_in_the_build());
    assert!(
        !spec.contains("**52 backends"),
        "the stale 52 is back in SPEC.md"
    );
}
