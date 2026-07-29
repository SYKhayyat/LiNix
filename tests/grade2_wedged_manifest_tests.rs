//! GRADER round 3, 2026-07-29 — RED. E1's family. A name a backend can never install is taken
//! back out of the manifest for two backends and left in for three, and the three wedge `sync`.
//!
//! E1 was the headline blocker of `READINESS-2026-07-27.md`: `linix install <typo>` left a line
//! nothing could satisfy in `modules/imperative.txt`, and every later command failed on it.
//! Round 1 closed it and I confirmed the closure on scoop. It was fixed at the instance.
//!
//! Measured today, one machine, one binary, four backends, each a name that cannot exist:
//!
//! | declaration                        | exit | line taken back out? |
//! |------------------------------------|------|----------------------|
//! | `scoop:linix-no-such-pkg-zzz`      |  1   | **yes**              |
//! | `cargo:linix-no-such-crate-zzz`    |  1   | **yes**              |
//! | `npm:linix-no-such-pkg-zzz-9`      |  1   | no                   |
//! | `github:linix-zzz-nope/nope`       |  1   | no                   |
//! | `luarocks:luafilesystem` (no Lua 5.5 rock here) | 1 | no        |
//!
//! `withdraw_what_can_never_succeed` (`src/verbs/packages.rs:153`) recognises exactly two
//! shapes: `Error::Unresolvable`, and `Error::CommandFailed { retry: Permanent }` **whose
//! message quotes the package name**. github raises neither — its own words are `Package
//! 'linix-zzz-nope/nope: the repo has no published release' was not found in the target
//! repository` — so the line stays. luarocks is the sharper case: LiNix retried four times,
//! compared the output, and printed *"a further retry will not help — this is not the transient
//! failure its output looks like"*, and still kept the line.
//!
//! The consequence is E1 verbatim. After the github line:
//!
//!     $ linix sync -y
//!     Error: Package 'linix-zzz-nope/nope: the repo has no published release' was not found …
//!     rc=1                                    … and identically, forever
//!
//! And the sentence the user is given is the one the code's own comment forbids:
//!
//!     WARN `github:linix-zzz-nope/nope` is still declared in …/imperative.txt, so `sync` will
//!          try it again.
//!
//! `packages.rs` says, of a kept line: *"it must not be described as a transient failure.
//! '`sync` will try it again' over a … refusal promises a retry that fails identically forever,
//! which is the sentence…"*. The rule is written down, and three of the five backends sampled
//! break it.
//!
//! **The fix is not a third pattern in the match.** Two shapes were enumerated and a third was
//! found the first time anyone asked a different backend; the shape of the answer is a backend
//! saying *"this name does not exist here"* in a way the caller can read without parsing prose.

use std::path::{Path, PathBuf};
use std::process::Command;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let f = Self { root };
        let (out, code) = f.run(&["init"]);
        assert_eq!(code, 0, "the fixture's own `init` failed:\n{out}");
        f
    }

    fn cfg(&self) -> PathBuf {
        self.root.join("config")
    }

    fn run(&self, args: &[&str]) -> (String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_linix"))
            .args(args)
            .env("LINIX_CONFIG_DIR", self.cfg())
            .env("LINIX_DATA_DIR", self.root.join("data"))
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

    fn imperative(&self) -> String {
        std::fs::read_to_string(self.cfg().join("modules").join("imperative.txt"))
            .unwrap_or_default()
    }

    fn backend_is_ready(&self, backend: &str) -> bool {
        let (out, _) = self.run(&["check", "health"]);
        out.lines()
            .any(|l| l.starts_with("[READY]") && l.split_whitespace().nth(1) == Some(backend))
    }
}

/// One declaration that cannot succeed, installed into a throwaway config.
fn probe(fixture_name: &str, backend: &str, decl: &str) -> Option<(String, String, i32)> {
    let f = Fixture::new(fixture_name);
    if !f.backend_is_ready(backend) {
        return None;
    }
    let (out, code) = f.run(&["install", decl, "-y"]);
    Some((out, f.imperative(), code))
}

/// The invariant, stated once: a declaration the backend has just refused as non-existent does
/// not stay in the manifest, because every later command parses the manifest.
#[test]
fn a_name_no_backend_can_install_is_never_left_in_the_manifest() {
    // Control first. Without a backend that demonstrably *does* withdraw, a red run below could
    // mean the withdrawal mechanism is gone rather than incomplete, and the finding is that it
    // is incomplete.
    let control = probe(
        "grade2-wedge-control",
        "cargo",
        "cargo:linix-no-such-crate-zzz",
    );
    let Some((cout, cmanifest, ccode)) = control else {
        panic!(
            "`cargo` is not READY on this machine, so the control cannot run. This test needs \
             one backend that withdraws and one that does not; run it on a host with cargo."
        );
    };
    assert_eq!(ccode, 1, "the control's install did not fail:\n{cout}");
    assert!(
        !cmanifest.contains("linix-no-such-crate-zzz"),
        "the control failed — `cargo` no longer withdraws an impossible name, so this test \
         cannot tell an incomplete mechanism from a missing one:\n{cout}"
    );

    let cases = [
        ("npm", "npm:linix-no-such-pkg-zzz-9"),
        ("github", "github:linix-zzz-nope/nope"),
    ];

    let mut wedged: Vec<String> = Vec::new();
    let mut examined = 0usize;

    for (backend, decl) in cases {
        let Some((out, manifest, code)) = probe(&format!("grade2-wedge-{backend}"), backend, decl)
        else {
            // A named skip, never a silent pass.
            eprintln!("skipped: {backend} is not READY on this machine");
            continue;
        };
        examined += 1;
        assert_eq!(
            code, 1,
            "`install {decl}` was expected to fail; if this name became installable, pick \
             another:\n{out}"
        );
        if manifest.contains(decl) {
            wedged.push(format!(
                "`{decl}` — still in modules/imperative.txt after the install failed. \
                 Every later `sync` fails on it.\n      {}",
                out.lines()
                    .filter(|l| l.contains("still declared") || l.starts_with("Error:"))
                    .collect::<Vec<_>>()
                    .join("\n      ")
            ));
        }
    }

    assert!(
        examined > 0,
        "neither npm nor github is READY here, so this test examined nothing — a named skip, \
         not a pass."
    );
    assert!(
        wedged.is_empty(),
        "a permanently-failed declaration was left in the manifest:\n  {}\n\n\
         `cargo` and `scoop` withdraw it; these do not. `withdraw_what_can_never_succeed` \
         matches `Unresolvable` and `CommandFailed{{Permanent}}` whose message quotes the name, \
         and every other way a backend says \"no such package\" wedges the config — which is E1, \
         the blocker this assessment opened with.",
        wedged.join("\n  ")
    );
}

/// The second half: what the user is told about the line that stayed.
#[test]
fn a_kept_line_is_not_described_as_something_sync_will_retry() {
    let Some((out, manifest, _)) = probe(
        "grade2-wedge-wording",
        "github",
        "github:linix-zzz-nope/nope",
    ) else {
        panic!("`github` is not READY on this machine; run this on a host where it is.");
    };
    assert!(
        manifest.contains("linix-zzz-nope/nope"),
        "the control failed — the line was withdrawn, so there is no kept line to describe. \
         That would mean the first test in this file is fixed and this one needs a new case.\
         \n{out}"
    );
    assert!(
        !out.contains("`sync` will try it again"),
        "the repo has no published release and never will within this run, and LiNix tells the \
         user `sync` will try it again. `src/verbs/packages.rs` writes the rule itself: a kept \
         line must not be described as a transient failure, because that promises a retry which \
         fails identically forever.\n{out}"
    );
}
