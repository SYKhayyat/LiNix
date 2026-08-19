//! A `rust-toolchain.toml` in this repository silently replaces the CI version matrix.
//!
//! `e7c9e75` added one pinning `channel = "1.97.1"`, to make the tree "still build years from
//! now". It made four jobs red and one job dishonest, and the dishonest one is the reason this
//! test exists rather than a note in a commit message.
//!
//! **Red.** `ci.yml` installs the toolchain with `dtolnay/rust-toolchain`, passing
//! `targets: ${{ matrix.target }}` — so `stable` gets the cross target's `std`. A pin file then
//! redirects every `cargo` call to the toolchain named `1.97.1`, which rustup installs on demand
//! with the components the file lists and **no targets at all**. `stable` and `1.97.1` are one
//! compiler and two rustup toolchains. Every `native: false` row died on
//! `error[E0463]: can't find crate for std`; the native rows survived because the host's own
//! `std` is always present.
//!
//! **Dishonest.** The `msrv` job reads `rust-version` from `Cargo.toml`, installs that compiler
//! and runs `cargo check`. The pin overrides it, so the job built on 1.97.1 and passed — a gate
//! reporting on a toolchain it did not use. The same override applies to every `rust: stable`
//! row, which is the whole point of a version matrix: a pin file makes it a matrix of one. The
//! commit did not only break four jobs, it disabled the gate that would have reported the rest.
//!
//! **Where reproducibility actually lives here, all three already committed:** `Cargo.lock` pins
//! the dependency graph, `rust-version` in `Cargo.toml` declares the compiler floor, and the CI
//! matrix *builds on that floor and on stable* so the claim is measured rather than asserted. A
//! pin file is a fourth mechanism answering the same question by overriding the other three.
//!
//! Adding one back means deleting this test, and deleting a test is a thing a reviewer sees.

use std::path::PathBuf;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Both spellings rustup honours, and both must stay absent: the extensionless file is the older
/// form and overrides just as completely.
const PIN_FILES: [&str; 2] = ["rust-toolchain.toml", "rust-toolchain"];

#[test]
fn no_toolchain_pin_file_overrides_the_ci_matrix() {
    for name in PIN_FILES {
        let path = repo().join(name);
        assert!(
            !path.exists(),
            "{name} exists. It overrides the toolchain CI installs, so the cross-compile rows \
             lose their target's std and the msrv job stops measuring the msrv. Pin the \
             dependency graph in Cargo.lock and the compiler floor in Cargo.toml's rust-version, \
             both of which the matrix already builds against."
        );
    }
}

/// The floor has to be a real version for the `msrv` job's `grep`/`cut` to hand rustup something
/// installable; an empty output there installs nothing and the job checks the default toolchain,
/// which is the same silent pass by another route.
#[test]
fn the_declared_msrv_is_a_version_the_msrv_job_can_install() {
    let manifest = std::fs::read_to_string(repo().join("Cargo.toml")).expect("Cargo.toml");
    let line = manifest
        .lines()
        .find(|l| l.starts_with("rust-version"))
        .expect("Cargo.toml declares rust-version; the msrv job greps for it at line start");
    let version = line.split('"').nth(1).expect("rust-version is quoted");
    let mut parts = version.split('.');
    let major: u32 = parts.next().and_then(|p| p.parse().ok()).expect("major");
    let minor: u32 = parts.next().and_then(|p| p.parse().ok()).expect("minor");
    assert_eq!(major, 1, "rust-version {version} is not a Rust 1.x release");
    assert!(minor > 0, "rust-version {version} names no minor release");
}

/// The matrix installs the cross target through the action's `targets:` input. If that input is
/// dropped, `rustup target add` never runs and the same `E0463` returns without a pin file in
/// sight — the second way this failure is reachable, so it is gated beside the first.
#[test]
fn ci_installs_the_matrix_target_with_the_toolchain() {
    let ci = std::fs::read_to_string(repo().join(".github/workflows/ci.yml")).expect("ci.yml");
    assert!(
        ci.contains("targets: ${{ matrix.target }}"),
        "the build matrix must pass its target to the toolchain install, or a cross row \
         compiles against a std that was never downloaded"
    );
}

fn ci_yml() -> String {
    std::fs::read_to_string(repo().join(".github/workflows/ci.yml")).expect("ci.yml")
}

/// The pin that replaced the pin file: `RUST_PINNED` in `ci.yml` chooses the compiler where the
/// target is also chosen, so a cross row still receives its `std`. Named once, because a second
/// literal is how one of them goes stale and a gate quietly measures a compiler nobody meant.
#[test]
fn ci_names_the_pinned_compiler_exactly_once() {
    let ci = ci_yml();
    let declarations: Vec<&str> = ci
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("RUST_PINNED:"))
        .collect();
    assert_eq!(
        declarations.len(),
        1,
        "RUST_PINNED is declared {} times: {declarations:?}",
        declarations.len()
    );
    let version = declarations[0]
        .split('"')
        .nth(1)
        .expect("RUST_PINNED names a quoted version");
    assert!(
        version.starts_with("1.") && version.split('.').count() >= 2,
        "RUST_PINNED is {version:?}, which is not an exact Rust release. A channel name here floats, and the pin then buys nothing"
    );
}

/// Exactly one job may install a floating toolchain, and it is the one that is allowed to fail.
/// Any other `@stable` install is a gate a Rust release can turn red on a tree nobody touched,
/// which is the failure this pin exists to prevent, reintroduced one job at a time.
#[test]
fn only_the_advisory_job_installs_a_floating_toolchain() {
    let ci = ci_yml();
    let floating = ci.matches("dtolnay/rust-toolchain@stable").count();
    assert_eq!(
        floating, 1,
        "{floating} job(s) install a floating toolchain. Only `newest-rust` may, because only it is continue-on-error; every other job takes the pinned toolchain"
    );
    assert!(
        ci.contains("  newest-rust:"),
        "the advisory drift job is gone, which leaves the pin a debt with the statement switched off"
    );
}

/// The advisory job has to stay advisory. Dropping `continue-on-error` hands a Rust release the
/// power to block every merge again, which is the arrangement the pin replaced.
#[test]
fn the_drift_detector_cannot_block_a_merge() {
    let ci = ci_yml();
    let after = ci
        .split_once("  newest-rust:")
        .expect("the advisory job exists")
        .1;
    let body = after.split_once("\n  msrv:").map_or(after, |(b, _)| b);
    assert!(
        body.contains("continue-on-error: true"),
        "`newest-rust` is not continue-on-error, so one new clippy lint blocks every merge"
    );
}
