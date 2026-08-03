//! A package the planner drops is a package the planner names (AU1).
//!
//! With `[guard] protected_packages = ["hello"]`, a managed-but-undeclared `hello` was dropped
//! from the removal plan by a `debug!` and a `continue`. The consequences, measured:
//!
//! | command | said | was true |
//! |---|---|---|
//! | `uninstall mytool:hello` | `already up to date`, exit 0 | the manifest line was deleted; the package was not removed |
//! | `sync` | `already up to date`, exit 0 | still installed, now declared nowhere |
//! | `check` | `ok drift  the machine matches your files` | false |
//!
//! And the state is permanently wedged: nothing declares it, the machine has it, the registry
//! manages it, and every later `sync` skips it again for the same silent reason.
//!
//! **The rule this violates is already written down, about the same situation.** `why.md:551`,
//! on `rebuild`: *"The skips are printed: a rebuild that silently dropped half its scope would
//! report success over a machine it never repaired, which is the same lie convergence was
//! already telling."* `rebuild.rs` implements it and has a test called
//! `a_protected_package_is_dropped_and_reported`. The planner is the sibling that never got it.
//!
//! The control matters here more than usual: every assertion below is paired with the same run
//! against a config protecting a DIFFERENT name, because "reports a skip" and "reports a skip
//! for everything" are indistinguishable from one assertion.

use std::path::PathBuf;
use std::process::Command;

struct Sandbox {
    root: tempfile::TempDir,
}

impl Sandbox {
    /// A repo whose registry manages one `cargo:hello` that no manifest declares — which is
    /// drift, and drift is what `sync` removes.
    fn new(protected: &str) -> Self {
        let root = tempfile::TempDir::new().unwrap();
        let cfg = root.path().join("config");
        let data = root.path().join("data");
        std::fs::create_dir_all(cfg.join("modules")).unwrap();
        std::fs::create_dir_all(&data).unwrap();

        std::fs::write(cfg.join("priority"), "cargo\n").unwrap();
        std::fs::write(cfg.join("active"), "").unwrap();
        std::fs::write(
            cfg.join("preferences.toml"),
            format!("[guard]\nprotected_packages = [\"{}\"]\n", protected),
        )
        .unwrap();

        std::fs::write(
            data.join("registry.json"),
            serde_json::json!({
                "packages": [{
                    "name": "hello",
                    "backend": "cargo",
                    "version": null,
                    "installed_at": 0,
                    "expires_at": null,
                    "options": {},
                    "source": null,
                    "is_transient": false,
                    "session_id": null
                }],
                "ghosts": {},
                "active_session_id": null,
                "suspensions": [],
                "held": []
            })
            .to_string(),
        )
        .unwrap();

        Self { root }
    }

    fn cfg(&self) -> PathBuf {
        self.root.path().join("config")
    }

    fn run(&self, args: &[&str]) -> (String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_linix"))
            .args(args)
            .env("LINIX_CONFIG_DIR", self.cfg())
            .env("LINIX_DATA_DIR", self.root.path().join("data"))
            .stdin(std::process::Stdio::null())
            .output()
            .expect("the binary should run");
        (
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
            out.status.code().unwrap_or(-1),
        )
    }
}

/// The planner drops it — that half is deliberate and stays. What must not stay is that
/// nothing says so.
#[test]
fn sync_does_not_report_success_over_a_package_it_declined_to_remove() {
    let protected = Sandbox::new("hello");
    let (out, code) = protected.run(&["-y", "sync"]);

    assert!(
        out.contains("hello"),
        "`sync` skipped `hello` and never named it. It printed:\n{}",
        out
    );
    assert!(
        out.to_lowercase().contains("protect"),
        "the skip must carry its reason — the guard already builds that sentence. Got:\n{}",
        out
    );
    assert_eq!(code, 0, "a reported skip is not a failure:\n{}", out);

    // Control: the same registry, the same undeclared package, a rule naming something else.
    let other = Sandbox::new("something-else-entirely");
    let (out, _) = other.run(&["--dry-run", "sync"]);
    assert!(
        !out.to_lowercase().contains("protect"),
        "a run with nothing protected reported a protection skip, so the assertion above \
         proves nothing:\n{}",
        out
    );
}

/// The one that makes the state unrecoverable: `check` is where a user goes to find out.
#[test]
fn check_does_not_call_the_machine_clean_while_it_holds_an_undeclared_package() {
    let protected = Sandbox::new("hello");
    let (out, _) = protected.run(&["check"]);

    assert!(
        !out.contains("the machine matches your files"),
        "`check` called it clean while `cargo:hello` is managed, installed and declared \
         nowhere:\n{}",
        out
    );
    assert!(
        out.contains("hello") || out.to_lowercase().contains("protect"),
        "`check` must name what it found:\n{}",
        out
    );
}

/// AU1's first row: the manifest line goes, the package stays, and the summary says neither.
#[test]
fn uninstall_says_what_it_did_not_do() {
    let sandbox = Sandbox::new("hello");
    // Declared this time, so `uninstall` has a line to delete — the exact sequence measured.
    std::fs::create_dir_all(sandbox.cfg().join("profiles")).unwrap();
    std::fs::write(sandbox.cfg().join("modules/dev.txt"), "cargo:hello\n").unwrap();
    std::fs::write(sandbox.cfg().join("profiles/Main"), "use dev\n").unwrap();
    std::fs::write(sandbox.cfg().join("active"), "Main\n").unwrap();

    let (out, _) = sandbox.run(&["-y", "uninstall", "cargo:hello"]);

    assert!(
        !out.trim().ends_with("already up to date"),
        "`uninstall` deleted the declaration, removed nothing, and reported that the machine \
         was up to date:\n{}",
        out
    );
    assert!(
        out.to_lowercase().contains("protect"),
        "it must say why the package is still there:\n{}",
        out
    );
}

/// `--dry-run` is where a user looks BEFORE the wedge. A preview that omits the skip sends
/// them into it.
#[test]
fn the_preview_names_the_skip_too() {
    let sandbox = Sandbox::new("hello");
    let (out, _) = sandbox.run(&["--dry-run", "sync"]);
    assert!(
        out.contains("hello"),
        "the preview did not mention the package it will silently leave behind:\n{}",
        out
    );
}

/// Every reason a removal can be dropped, not just the one that was reported. The planner had
/// two drop sites and both were a bare `continue`; a fix that reports one of them leaves the
/// other exactly as it was.
#[test]
fn a_managed_package_whose_backend_left_priority_is_reported_too() {
    let sandbox = Sandbox::new("nothing-here");
    // `cargo` is no longer listed, so drift for it is deliberately NOT reaped (II.6) — and
    // that, too, is a package the machine has and nothing declares.
    std::fs::write(sandbox.cfg().join("priority"), "npm\n").unwrap();

    let (out, _) = sandbox.run(&["--dry-run", "sync"]);
    assert!(
        out.contains("cargo"),
        "a managed package left alone because its backend is not in `priority` was dropped \
         silently — the same shape as AU1, one branch over:\n{}",
        out
    );
}

/// Fixture check: the sandbox really does describe drift, or every assertion above passes for
/// a machine with nothing to do.
#[test]
fn the_fixture_is_drift_when_nothing_protects_it() {
    let sandbox = Sandbox::new("something-else-entirely");
    let (out, _) = sandbox.run(&["--dry-run", "sync"]);
    assert!(
        out.contains("remove 1") || out.contains("hello"),
        "with nothing protected, this fixture must plan one removal:\n{}",
        out
    );
}

/// The invariant, asked of the type rather than of the two sites I happened to find.
///
/// Grade §9's cure for a family is *"invariant tests that quantify over sites rather than naming
/// one"*, and its first example is this: **every code path that drops something from a plan
/// appends to a reported list.** The tests above are behavioural and name two paths; this one
/// is exhaustive over `Declined`, so a third reason added tomorrow **fails to compile here**
/// until someone says whether the user hears about it.
///
/// A `_ => ` arm would defeat it entirely, which is why there is not one.
#[test]
fn every_reason_a_removal_is_declined_says_whether_the_user_hears_about_it() {
    use linix::app::sync::planner::Declined;

    for declined in [
        Declined::AlreadyScheduled,
        Declined::StillDeclared,
        Declined::BackendNotInPriority("cargo".into()),
        Declined::Protected("hello".into()),
    ] {
        let reported = declined.reported();
        let must_report = match declined {
            // The removal is happening, or there is nothing to remove: the machine and the
            // files agree when this run ends, so there is nothing to tell anyone.
            Declined::AlreadyScheduled | Declined::StillDeclared => false,
            // The machine keeps software nothing declares and no `sync` will ever take. That
            // is a standing disagreement and it must be said out loud.
            Declined::BackendNotInPriority(_) | Declined::Protected(_) => true,
        };

        assert_eq!(
            reported.is_some(),
            must_report,
            "{:?} reports {:?}, which is not what this test says it owes the user",
            declined,
            reported
        );
        if let Some(text) = reported {
            assert!(
                text.len() > 20 && !text.ends_with(':'),
                "{:?}'s reason is not a sentence a user can act on: {:?}",
                declined,
                text
            );
        }
    }
}
