//! The resource half of the model, for every kind — not just the one that had a reproduction.
//!
//! N-2 was found with `link:` lines: `check` reported that the machine matched while a declared
//! link was missing from disk, `sync` placed three files under `already up to date`, and `plan`
//! froze an empty plan in both directions. The cause was never about links. `check`, `plan`,
//! `apply` and `sync`'s summary each asked the *package* planner and nothing else, so every
//! kind of resource was invisible to all four: `link:`, `service:`, `setting:`, `shim:`,
//! `schedule:` and `repo:`.
//!
//! `tests/grade2_check_extras_tests.rs` and `tests/grade2_plan_extras_tests.rs` pin the
//! behaviour for `link:`. This file pins the family, and the two questions the family answers
//! differently:
//!
//! - *Has this ever been applied?* — the extras ledger's to answer, identically for all six
//!   kinds. Tested here for all five that live in a module file.
//! - *Is it still in effect?* — the machine's, and only some kinds can be asked. A `link:` is a
//!   file test and a `shim:` is a file test; a `setting:` reads back through an adapter that
//!   does not report a current value. The ones that cannot be asked are **named**, not assumed
//!   converged, because "the machine matches your files" over something nobody looked at is
//!   precisely the sentence this finding is about.


use crate::harness::{decl, Fixture};

/// The shared root, plus what these tests need in it.
fn setup(name: &str) -> Fixture {
    let f = Fixture::new(name);
    std::fs::create_dir_all(f.root.join("dest")).unwrap();
    std::fs::create_dir_all(f.cfg().join("dotfiles")).unwrap();
    f
}

/// One declaration of every kind that lives in a module, and the ledger key each produces.
///
/// `schedule:` is deliberately absent: the grammar requires it in the `schedules` file, not a
/// module, so a line here would be a refusal rather than a resource. Its ledger key travels the
/// same code path as these — `extra_key` gives it one and `changes()` never looks at the kind
/// when the ledger has no record — and the `link:` and `shim:` probes are the only kind-specific
/// code there is.
fn declarations(f: &Fixture) -> (String, Vec<String>) {
    let src = f.cfg().join("dotfiles").join("f1");
    std::fs::write(&src, "content\n").unwrap();
    let dst = f.root.join("dest").join("f1");

    let mut module = format!(
        "link:{} @target={}\n\
         service:linix-test-svc @enabled=false\n\
         setting:org.linix.test/key @value=1\n\
         shim:linix-test-shim\n",
        decl(&src),
        decl(&dst)
    );
    let mut keys = vec![
        format!("link:{}", decl(&dst)),
        "service:linix-test-svc".to_string(),
        "setting:org.linix.test/key".to_string(),
        "shim:linix-test-shim".to_string(),
    ];
    // `repo:` is the one kind that cannot be a constant: it names a package manager, and
    // resolution refuses a `repo:` whose backend is not in this host's `priority` — so hardcoded
    // `scoop` made the whole module unresolvable anywhere but Windows, and every count below
    // then read `does not resolve` instead of a number. Taken from the `priority` that `init`
    // just wrote, so the line names a manager this machine actually uses.
    if let Some(be) = repo_backend(f) {
        module.push_str(&format!("repo:{be}:linix-test-bucket\n"));
        keys.push(format!("repo:{be}:linix-test-bucket"));
    } else {
        eprintln!(
            "  note: no repository-owning manager in this host's priority list, so the `repo:` \
             kind is not covered by this run — the other four are."
        );
    }
    (module, keys)
}

/// A manager on this host that owns repositories, or `None`.
fn repo_backend(f: &Fixture) -> Option<String> {
    let priority = std::fs::read_to_string(f.cfg().join("priority")).unwrap_or_default();
    ["scoop", "brew", "apt", "dnf", "pacman", "apk"]
        .iter()
        .find(|b| priority.lines().any(|l| l.trim() == **b))
        .map(|b| (*b).to_string())
}

/// A resource nothing has ever applied is work, whatever kind it is — and the two commands a
/// user reads before touching anything both have to say so.
#[test]
fn every_kind_of_undeclared_resource_is_reported_by_check_and_frozen_by_plan() {
    let f = setup("resource-family-place");
    let (module, keys) = declarations(&f);
    f.write_module(&module);

    // `check` first: this is the command whose one job is "does the machine match?".
    let (out, code) = f.run(&["check"]);
    assert!(code == 0 || code == 2, "`check` failed ({code}):\n{out}");
    assert!(
        !out.contains("the machine matches your files"),
        "`check` reported the machine matches, with {} resources declared and none applied:\n{}",
        keys.len(),
        out
    );
    assert!(
        out.contains(&format!("{} to place", keys.len())),
        "`check` did not count all {} resources:\n{}",
        keys.len(),
        out
    );

    // Then `plan`, which is the reviewable artifact and has to name each one.
    let (out, code) = f.run(&["plan"]);
    assert_eq!(code, 0, "`plan` failed:\n{out}");
    assert!(
        !out.contains("already matches desired state"),
        "`plan` froze an empty plan over {} declared resources:\n{}",
        keys.len(),
        out
    );

    let frozen = std::fs::read_to_string(f.root.join("linix-plan.json")).expect("plan file");
    for key in &keys {
        assert!(
            frozen.contains(key),
            "the frozen plan does not name `{key}` — a plan that omits work `sync` would do is \
             a review that reports nothing to see:\n{frozen}"
        );
        assert!(
            out.contains(key),
            "`plan` did not print `{key}`, so a user reading the terminal sees a count without \
             the things it counts:\n{out}"
        );
    }
}

/// The other direction, for every kind: applied before, declared nowhere now.
///
/// This is the half the guard already counts — a teardown past `max_removals` is refused — and
/// `plan` is where the refusal's own text tells the user to look.
#[test]
fn every_kind_of_undeclared_resource_is_reported_as_a_teardown() {
    let f = setup("resource-family-undo");
    let (_, keys) = declarations(&f);
    let borrowed: Vec<&str> = keys.iter().map(String::as_str).collect();
    f.seed_ledger(&borrowed);
    // Declared nowhere: an empty module against a ledger that records all five.
    f.write_module("");

    let (out, code) = f.run(&["plan"]);
    assert_eq!(code, 0, "`plan` failed:\n{out}");
    assert!(
        !out.contains("already matches desired state"),
        "`plan` reported a match while {} applied resources are no longer declared:\n{}",
        keys.len(),
        out
    );
    for key in &keys {
        assert!(
            out.contains(key),
            "`plan` did not name `{key}` as a teardown. The guard's refusal text sends the user \
             to `linix plan` to see exactly what would be undone.\n{out}"
        );
    }

    let (out, code) = f.run(&["check"]);
    assert!(code == 0 || code == 2, "`check` failed ({code}):\n{out}");
    assert!(
        out.contains(&format!("{} to undo", keys.len())),
        "`check` did not count the {} teardowns:\n{}",
        keys.len(),
        out
    );
}

/// A resource LiNix placed and can read back: it must report converged, or `check` is red
/// forever on a machine that matches.
///
/// This is the positive half of the probe, and without it the two tests above would pass on an
/// implementation that simply called everything drift.
#[test]
fn a_resource_that_is_applied_and_present_is_not_reported_as_drift() {
    let f = setup("resource-family-converged");
    let src = f.cfg().join("dotfiles").join("f1");
    std::fs::write(&src, "content\n").unwrap();
    let dst = f.root.join("dest").join("f1");
    f.write_module(&format!(
        "link:{} @target={}\n",
        decl(&src),
        decl(&dst)
    ));

    let (out, code) = f.run(&["sync", "-y"]);
    assert_eq!(code, 0, "`sync` failed:\n{out}");
    assert!(
        dst.exists(),
        "setup did not place {}:\n{out}",
        dst.display()
    );

    let (out, code) = f.run(&["check"]);
    assert!(code == 0 || code == 2, "`check` failed ({code}):\n{out}");
    assert!(
        out.contains("the machine matches your files"),
        "a `link:` that is declared, applied and on disk was reported as drift — `check` would \
         then be red on every converged machine:\n{out}"
    );
    assert!(!out.contains("to place"), "the same, counted:\n{out}");

    let (out, code) = f.run(&["plan"]);
    assert_eq!(code, 0, "`plan` failed:\n{out}");
    assert!(
        out.contains("already matches desired state"),
        "`plan` found work on a converged machine:\n{out}"
    );
}

/// And the bound, stated out loud. A `setting:` cannot be read back through its adapter, so
/// LiNix does not know whether it is still in effect — and says so rather than reporting a
/// match it did not verify.
///
/// This is the assertion that separates "converged" from "unexamined". Without it, returning
/// "in effect" for every kind LiNix cannot probe would pass every other test in this file.
#[test]
fn a_resource_that_cannot_be_read_back_is_named_rather_than_assumed() {
    let f = setup("resource-family-unverifiable");
    f.write_module("setting:org.linix.test/key @value=1\n");
    f.seed_ledger(&["setting:org.linix.test/key"]);

    let (out, code) = f.run(&["check"]);
    assert!(code == 0 || code == 2, "`check` failed ({code}):\n{out}");
    assert!(
        out.contains("cannot read back"),
        "`check` reported a plain match over a `setting:` whose current value LiNix never asked \
         for. An unstated bound on what `check` means is the whole shape of this finding.\n{out}"
    );
    assert!(
        out.contains("setting:org.linix.test/key"),
        "the unreadable resource is counted but not named, so nobody can tell which one it \
         is:\n{out}"
    );
    // And it is not drift: "I cannot tell" must not make `check` permanently red.
    assert!(
        !out.contains("to place"),
        "an unreadable resource was counted as work, which would make `check` red forever on \
         any machine declaring a `setting:`:\n{out}"
    );
}
