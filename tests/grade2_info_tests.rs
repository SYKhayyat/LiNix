//! GRADER round 3, 2026-07-29 — RED. `info` answers about the resolver, not about the machine.
//!
//! Two findings, one root. `App::get_info` (`src/app/context.rs:541`) hands `backend:name` to
//! `resolve_spec` and then either asks exactly the backends that came back, or — when the string
//! does not resolve at all — falls through to asking **every** available backend for a package
//! literally named `nosuchbackend:foo`. Neither branch ever asks the question `install` and
//! `list` both ask: *is that prefix a backend at all?*
//!
//! **H-2 — an unknown backend prefix costs 83 seconds and gets the wrong answer.** Measured on
//! Windows, back to back, same config:
//!
//!     $ linix install nosuchbackend:foo -y
//!     Error: Configuration error: `nosuchbackend` is not a backend LiNix uses
//!       add `nosuchbackend` to your `priority` file, or check the spelling.
//!     rc=1                                                             215 ms
//!
//!     $ linix list -b nosuchbackend
//!     Error: Configuration error: `nosuchbackend` is not a backend LiNix uses …
//!     rc=1                                                             fast
//!
//!     $ linix info nosuchbackend:foo
//!     'nosuchbackend:foo' is not installed on this machine, so there is nothing to describe.
//!       `linix search foo` looks for it in the managers you use.
//!     rc=0                                             83 522 ms, twice, reproducibly
//!
//! `list -b <typo>` was G-7 and it was fixed — `require_known_backend` is called there and the
//! `--backend` flag family (`rebuild`, `upgrade`, `repo list`) was swept with it. The
//! `backend:name` *spec* form was not, and it is the form the same user types next.
//!
//! **H-3 — `info <bare name>` denies what `list` reports.** A bare name is resolved by
//! `priority` order, not by where the package actually is, and `get_info` then asks only the
//! backend the resolver picked and returns `Ok(None)`. Measured, same machine, same binary,
//! with a control between the two:
//!
//!     $ linix list -b cargo    →  cargo  ripgrep  15.2.0
//!     $ linix info cargo:ripgrep →  Package: ripgrep  Backend: cargo  Version: 15.2.0
//!     $ linix info ripgrep     →  'ripgrep' is not installed on this machine …
//!     $ linix list -b cargo    →  cargo  ripgrep  15.2.0        (unchanged)
//!
//! `linix eval` shows the mechanism: bare `hexyl` resolves to `choco:hexyl` because `choco` is
//! first in `priority` and choco's index carries the name — so `info hexyl` asks choco, choco
//! does not have it installed, and LiNix reports the package absent while `list` reports it
//! present at 0.17.0 under cargo.
//!
//! This is E6's class ("a `list` that disagrees with the machine breaks the one thing it
//! promises") on the `info` verb, and READINESS graded that one as the reason the backend layer
//! was a D. The rule the fix has to satisfy: **a read command answers about the machine, and two
//! read commands never contradict each other about it.**

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

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

    fn run(&self, args: &[&str]) -> (String, i32) {
        self.timed(args).0
    }

    fn timed(&self, args: &[&str]) -> ((String, i32), u128) {
        let started = Instant::now();
        let out = Command::new(env!("CARGO_BIN_EXE_linix"))
            .args(args)
            .env("LINIX_CONFIG_DIR", self.root.join("config"))
            .env("LINIX_DATA_DIR", self.root.join("data"))
            .stdin(std::process::Stdio::null())
            .output()
            .expect("the binary should run");
        let ms = started.elapsed().as_millis();
        (
            (
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                ),
                out.status.code().unwrap_or(-1),
            ),
            ms,
        )
    }
}

/// H-2. One question — "is `nosuchbackend` a manager?" — answered three ways by one binary.
#[test]
fn info_refuses_an_unknown_backend_prefix_the_way_install_does() {
    let f = Fixture::new("grade2-info-typo");

    // Control: the verb that gets it right, so a red `info` cannot be blamed on the fixture.
    let ((out, code), install_ms) = f.timed(&["install", "nosuchbackend:foo", "-y"]);
    assert_eq!(
        code, 1,
        "the control failed — `install` no longer refuses an unknown backend:\n{out}"
    );
    assert!(
        out.contains("is not a backend LiNix uses"),
        "the control failed — `install`'s refusal changed wording:\n{out}"
    );

    let ((out, code), info_ms) = f.timed(&["info", "nosuchbackend:foo"]);

    assert!(
        !out.contains("is not installed on this machine"),
        "`info nosuchbackend:foo` reported the package absent. The package is not absent; the \
         *manager* does not exist, and saying otherwise sends the user looking for a package \
         instead of a typo.\n{out}"
    );
    assert!(
        out.contains("is not a backend LiNix uses"),
        "`info` did not name the unknown backend. `install` says `nosuchbackend` is not a \
         backend LiNix uses; `list -b nosuchbackend` says the same; `info` says the package is \
         not installed. One question, three answers.\n{out}"
    );
    assert_ne!(
        code, 0,
        "`info` exited 0 for a backend that does not exist — a script cannot tell this from a \
         successful lookup.\n{out}"
    );

    // The cost of not asking: an unresolvable string is handed to every available backend, so
    // the wrong answer is also the slowest one in the program. 83.5 s measured, twice.
    assert!(
        info_ms < 10_000,
        "`info nosuchbackend:foo` took {info_ms} ms while `install` refused the same string in \
         {install_ms} ms. The whole cost is asking every manager on the machine for a package \
         named `nosuchbackend:foo`, which none of them can have."
    );
}

/// H-3. Two read commands, one machine, opposite answers.
#[test]
fn info_agrees_with_list_about_what_is_installed() {
    let f = Fixture::new("grade2-info-bare");

    let (listing, code) = f.run(&["list"]);
    assert_eq!(code, 0, "`list` failed:\n{listing}");

    // BACKEND PACKAGE VERSION, whitespace-separated. Take the first row that has a name; a
    // machine with nothing installed cannot answer this question and says so rather than
    // passing.
    let row = listing
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
        .find(|c| c.len() >= 2 && !c[0].starts_with('-'));
    let Some(cols) = row else {
        panic!(
            "`linix list` reported no installed package on this machine, so there is nothing to \
             cross-examine `info` about. This test needs a host with at least one package under \
             any backend.\n{listing}"
        );
    };
    let (backend, name) = (cols[0].to_string(), cols[1].to_string());

    // Control: the qualified form agrees with `list`, which is what makes the bare form's
    // answer a contradiction rather than a missing feature.
    let qualified = format!("{backend}:{name}");
    let (out, code) = f.run(&["info", &qualified]);
    assert_eq!(code, 0, "`info {qualified}` failed:\n{out}");
    assert!(
        !out.contains("is not installed on this machine"),
        "the control failed — `info {qualified}` denies a package `list` just reported, so the \
         listing and the fixture disagree before the bare form is even asked:\n{out}"
    );

    let (out, code) = f.run(&["info", &name]);
    assert_eq!(code, 0, "`info {name}` failed:\n{out}");
    assert!(
        !out.contains("is not installed on this machine"),
        "`linix list` reports `{name}` installed under `{backend}`, and `info {qualified}` \
         confirms it — but `info {name}` says it is not installed on this machine.\n\
         A bare name is resolved by `priority` order rather than by where the package is, and \
         `get_info` then asks only the backend the resolver picked. The machine is not \
         consulted, and the user is told a package they have is absent.\n{out}"
    );
}
