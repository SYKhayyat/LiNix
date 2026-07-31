//! GRADER round 6, 2026-07-31 — RED. `assert_binary_reachable` asks "is a binary of this name on
//! PATH", never "did *this backend* put it there" — so another manager's copy of the same name
//! scores the check green.
//!
//! **Measured, not argued.** From CI run 30566924407, the `tools` image — the widest live
//! coverage in this repo, 25 real lifecycles:
//!
//! ```text
//!   PASS  cabal: hello is on PATH
//!   soft  go: hello already resolves to /root/.cabal/bin/hello — the removal check compares
//!         against that, not against absence
//!   PASS  go: hello is on PATH          <-- cabal's binary, scored as go's
//! ```
//!
//! The harness *computed the exact value that would have caught this*. `lifecycle()` reads
//! `_prepath="$(path_of "$cbin")"` before the install, says so out loud in a `soft`, and then
//! hands it to `assert_binary_gone "$be" "$cbin" "$_prepath"` — and **not** to
//! `assert_binary_reachable "$be" "$cbin" /tmp/life.out`. One half of the lifecycle is protected
//! against the collision the other half announces.
//!
//! `go: hello is on PATH` would pass on that image if the go install had done nothing at all.
//! That is one of the three legs the `READINESS` §8.1 **A** bar names — "install → `list` →
//! binary → remove, asserted while installed".
//!
//! This is the repo's own case study, one function over. `CLAUDE.md` opens its "fix the whole
//! family" section with `assert_binary_gone` passing because it asked the shell's hash table, and
//! with the twin bug sitting untouched in the Windows script "for another hour". Both harnesses
//! carry this one, identically, at two call sites each.
//!
//! The fix is not this test's business, but the shape is forced: `assert_binary_reachable` has to
//! take `_prepath` and, when it is non-empty, resolve the binary again and require the answer to
//! have *changed* — or fail, naming the manager that already owned the name.

use std::path::PathBuf;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

const HARNESSES: &[&str] = &[
    "scripts/integration-windows.sh",
    "docker/integration/run-in-container.sh",
];

/// The premise, checked rather than assumed: both harnesses do compute the pre-existing
/// resolution and do give it to the removal half. If that ever stops being true this file is
/// diagnosing something that no longer exists, and it should say so instead of failing vaguely.
#[test]
fn the_premise_the_harness_already_knows_the_name_was_taken() {
    for h in HARNESSES {
        let body = std::fs::read_to_string(repo().join(h)).unwrap();
        assert!(
            body.contains(r#"_prepath="$(path_of "$cbin")""#),
            "{h} no longer reads the pre-existing resolution; re-derive this finding"
        );
        assert!(
            body.contains(r#"assert_binary_gone "$be" "$cbin" "$_prepath""#),
            "{h} no longer hands _prepath to assert_binary_gone; re-derive this finding"
        );
    }
}

/// Every `assert_binary_reachable` call must be told what already owned the name, exactly as its
/// twin is. A call that is not told cannot distinguish "this backend installed it" from
/// "something else did".
#[test]
fn every_binary_reachable_call_is_told_what_already_owned_the_name() {
    let mut blind = Vec::new();

    for h in HARNESSES {
        let body = std::fs::read_to_string(repo().join(h)).unwrap();
        for (i, line) in body.lines().enumerate() {
            let t = line.trim();
            // The definition and the comment above it are not calls.
            if t.starts_with('#') || t.starts_with("assert_binary_reachable()") {
                continue;
            }
            if !t.contains("assert_binary_reachable ") {
                continue;
            }
            // The fourth argument, and that it names a prior resolution — asked of the argument
            // position rather than of the whole line, so a call cannot satisfy this by
            // mentioning the value in a comment or by passing it somewhere else. Case-insensitive
            // because the lifecycle's local is `$_prepath` and section 5's global is
            // `$PKG_PREPATH`; the value is the point, not the spelling.
            let args: Vec<&str> = t
                .split_whitespace()
                .skip_while(|w| !w.contains("assert_binary_reachable"))
                .skip(1)
                .collect();
            if args.len() >= 4 && args[3].to_ascii_lowercase().contains("prepath") {
                continue;
            }
            blind.push(format!("{}:{}: {}", h, i + 1, t));
        }
    }

    assert!(
        blind.is_empty(),
        "{} call(s) to assert_binary_reachable are never told which binary already owned the \
         name, while assert_binary_gone on the very next lines is:\n  {}\n\n\
         Measured on the tools image (CI 30566924407): `go: hello is on PATH` passed against \
         /root/.cabal/bin/hello, which cabal installed four lifecycles earlier. The harness \
         printed the collision as a `soft` in the same run and scored the check green anyway.",
        blind.len(),
        blind.join("\n  ")
    );
}

/// And the parameter has to reach the function, not merely the call site — a value passed and
/// dropped is the same blindness with more punctuation.
#[test]
fn assert_binary_reachable_actually_reads_what_it_is_given() {
    let mut deaf = Vec::new();

    for h in HARNESSES {
        let body = std::fs::read_to_string(repo().join(h)).unwrap();
        let Some(start) = body.find("assert_binary_reachable() {") else {
            panic!("{h} has no assert_binary_reachable; re-derive this finding");
        };
        let tail = &body[start..];
        let end = tail.find("\n}\n").map(|e| e + 2).unwrap_or(tail.len());
        let func = &tail[..end];

        // `on_path "$_rbin"` alone answers "is SOME binary of this name reachable". The function
        // has to compare against the prior owner to answer "did this backend put it there".
        let compares = func.contains("_prepath") || func.contains("_rprev");
        if !compares {
            deaf.push(*h);
        }
    }

    assert!(
        deaf.is_empty(),
        "assert_binary_reachable in {:?} decides on `on_path` alone. It has no way to tell a \
         binary this backend installed from one that was already there under the same name, so \
         the check passes whether or not the install did anything.",
        deaf
    );
}
