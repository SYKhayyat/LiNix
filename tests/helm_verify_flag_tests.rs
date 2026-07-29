//! E11 reopened / G-8: `--verify=false` is helm 4's flag, and it was emitted unconditionally.
//!
//! `capability.rs`'s `VERIFIES_ITSELF = [("helm", "--verify=false")]` was E11's fix, taken from
//! helm's own error text — "Use --verify=false to skip verification" — on a machine running
//! helm v4.2.3. helm 3 has no such flag and rejects it:
//!
//! ```text
//! Error: `helm` failed (exit 1): Error: unknown flag: --verify
//! ```
//!
//! So `@unverified` worked on helm 4 and broke every helm 3: one argv defect traded for
//! another, from a fix derived from one machine and shipped everywhere. The gate that exists
//! for exactly this could not see it — `tests/argv_drift_tests.rs` says in its own words that
//! it examines "a token that is a subcommand rather than a flag", so 72 subcommands were
//! verified against live tools and zero flags were.
//!
//! Fixture-driven, against both helps rather than against whichever helm this host has:
//!
//!   * `tests/fixtures/helm/plugin-install-help-v4.txt` — captured from the real helm v4.2.3
//!     on this machine, and it documents `--verify`;
//!   * `tests/fixtures/helm/plugin-install-help-v3.txt` — helm 3's, which does not.
//!
//! The property: LiNix asks the installed tool, so it builds a different argv against each.

use std::path::{Path, PathBuf};

/// A fake manager on PATH whose `--help` prints `fixture`.
///
/// The point of the exercise is that the answer comes from the tool, so the test has to supply
/// a tool. Two distinct names, because the probe caches per program.
fn fake_tool(name: &str, fixture: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("helm-flag-fakes");
    std::fs::create_dir_all(&dir).unwrap();
    let help = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/helm")
            .join(fixture),
    )
    .unwrap_or_else(|e| panic!("fixture {fixture} must exist: {e}"));

    let script = dir.join(if cfg!(windows) {
        format!("{name}.cmd")
    } else {
        name.to_string()
    });
    let body = if cfg!(windows) {
        // `type` the help from a file beside the script: batch quoting cannot carry this text.
        let data = dir.join(format!("{name}.help.txt"));
        std::fs::write(&data, &help).unwrap();
        format!("@echo off\r\ntype \"{}\"\r\n", data.display())
    } else {
        let data = dir.join(format!("{name}.help.txt"));
        std::fs::write(&data, &help).unwrap();
        format!("#!/bin/sh\ncat '{}'\n", data.display())
    };
    std::fs::write(&script, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(&script).unwrap().permissions();
        p.set_mode(0o755);
        std::fs::set_permissions(&script, p).unwrap();
    }
    dir
}

fn with_dir_on_path(dir: &Path) {
    let old = std::env::var("PATH").unwrap_or_default();
    let sep = if cfg!(windows) { ";" } else { ":" };
    if !old.starts_with(&*dir.to_string_lossy()) {
        std::env::set_var("PATH", format!("{}{}{}", dir.display(), sep, old));
    }
}

#[test]
fn the_unverified_flag_is_emitted_only_where_the_tool_takes_it() {
    let chain = vec!["plugin".to_string(), "install".to_string()];

    let dir = fake_tool("helmfake4", "plugin-install-help-v4.txt");
    fake_tool("helmfake3", "plugin-install-help-v3.txt");
    with_dir_on_path(&dir);

    let v4 = linix::core::tool_help::accepts_flag("helmfake4", &chain, "--verify=false");
    let v3 = linix::core::tool_help::accepts_flag("helmfake3", &chain, "--verify=false");

    assert!(
        v4,
        "helm 4's own `plugin install --help` documents `--verify`, so LiNix must still use it \
         — otherwise this fix would have removed E11's fix instead of conditioning it"
    );
    assert!(
        !v3,
        "helm 3's `plugin install --help` has no `--verify`, and LiNix passed it anyway: \
         `Error: unknown flag: --verify`. The flag came from helm 4's error text on one \
         machine and was shipped to every machine."
    );
}

/// The trap that made this need a real matcher rather than `contains`.
#[test]
fn a_longer_flag_is_not_mistaken_for_the_one_being_asked_about() {
    let chain = vec!["plugin".to_string(), "install".to_string()];
    let dir = fake_tool("helmfake3b", "plugin-install-help-v3.txt");
    with_dir_on_path(&dir);

    // helm 3's help carries `--kube-insecure-skip-tls-verify`. A substring search for
    // `--verify` finds nothing there, but a search for `verify` finds it — and a naive
    // matcher for `--kube` would find `--kube-context`. The oracle has to reject both.
    let help = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/helm/plugin-install-help-v3.txt"),
    )
    .unwrap();
    assert!(
        help.contains("--kube-insecure-skip-tls-verify"),
        "the fixture no longer contains the flag this test is about; re-capture it"
    );
    assert!(
        !linix::core::tool_help::accepts_flag("helmfake3b", &chain, "--verify=false"),
        "`--kube-insecure-skip-tls-verify` was read as `--verify`"
    );
    assert!(
        linix::core::tool_help::accepts_flag("helmfake3b", &chain, "--kube-context"),
        "the control failed: a flag helm 3 really does document was not found, so the \
         assertion above would pass for a probe that always says no"
    );
}
