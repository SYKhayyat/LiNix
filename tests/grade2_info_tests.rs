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
//!     rc=0                              5 523 – 83 522 ms, measured repeatedly across a day
//!
//! The spread is the network, not the build: two consecutive debug runs agreed at 82.5 s and
//! 83.5 s, and later runs of the same command on the same host ranged down to 5.5 s. The stable
//! fact is the shape — `install` refuses without asking anything, and `info` asks every manager
//! on the machine for a package none of them can have. Only the shape is asserted below.
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

use crate::harness::Fixture;

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

    // The cost of not asking, reported and deliberately NOT asserted on. An unresolvable string
    // is handed to every available backend, so the wrong answer is also the slow one: measured
    // between 5.5 s and 83.5 s across a day on this host, against `install`'s ~0.2 s, which is
    // stable because it never reaches the network. A threshold over a number that moves by 10×
    // with the weather is an assertion that passes or fails by luck — the mirror of one that
    // cannot fail — so this prints and the assertions above are what decide the test.
    eprintln!(
        "timing (not asserted): info {info_ms} ms vs install {install_ms} ms for the same \
         unknown backend"
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

// ---------------------------------------------------------------------------------------
// BUILDER round 6, W37 / R-4. The test above takes the FIRST row `list` prints, so which
// backend it examines is whatever that host happens to list first. On macOS that was a
// launchd agent and it went red; on Windows it is a package and the same defect was invisible.
//
// One row per backend, so the platform decides how much is covered and never which half.
// ---------------------------------------------------------------------------------------

/// Every backend `list` reports at least one row for, with the first row's name.
fn one_row_per_backend(listing: &str) -> Vec<(String, String)> {
    let mut seen: Vec<(String, String)> = Vec::new();
    for line in listing.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 2 || cols[0].starts_with('-') {
            continue;
        }
        // **A log line is not a listing row.** `list` warns on stderr when a manager's output
        // stops parsing (`LX-1`), and the fixture merges the two streams — so the first token of
        // `\x1b[33m WARN\x1b[0m …` was scraped as a backend name and `info` was asked about
        // `\x1b[33m:WARN`. The failure then read as "a backend contradicts its own listing",
        // which is the opposite of what happened. A backend name is lowercase and unadorned.
        if !cols[0]
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            continue;
        }
        if !seen.iter().any(|(b, _)| b == cols[0]) {
            seen.push((cols[0].to_string(), cols[1].to_string()));
        }
    }
    seen
}

#[test]
fn info_agrees_with_list_about_every_backend_not_only_the_first() {
    let f = Fixture::new("grade2-info-every-backend");

    let (listing, code) = f.run(&["list"]);
    assert_eq!(code, 0, "`list` failed:\n{listing}");

    let rows = one_row_per_backend(&listing);
    assert!(
        !rows.is_empty(),
        "`linix list` reported nothing at all, so there is nothing to cross-examine `info` \
         about:\n{listing}"
    );

    let mut denied = Vec::new();
    for (backend, name) in &rows {
        let qualified = format!("{backend}:{name}");
        let (out, code) = f.run(&["info", &qualified]);
        if code != 0 {
            denied.push(format!(
                "`info {qualified}` exited {code}:\n      {}",
                out.trim()
            ));
            continue;
        }
        if out.contains("is not installed on this machine") {
            denied.push(format!(
                "`info {qualified}` denies a row `list` just printed under `{backend}`"
            ));
        }
    }

    assert!(
        denied.is_empty(),
        "{} of {} backend(s) contradict their own listing:\n  {}\n\n\
         `service`, `link` and `setting` are each a grammar prefix AND a registered backend, so \
         a string copied out of a listing parses as a typed resource statement rather than as \
         `backend:name` — and everything downstream understood only packages. A `list` that \
         disagrees with the machine breaks the one thing it promises.\n\nbackends examined: {}",
        denied.len(),
        rows.len(),
        denied.join("\n  "),
        rows.iter()
            .map(|(b, _)| b.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
}

/// GRADER §4: *flag every place internal vocabulary leaks.* `info` rendered every leftover
/// property as an aligned field, so `linix info service:Appinfo` printed
///
///     status raw:    [SC] QueryServiceConfig SUCCESS
///
/// — an internal key with its underscore swapped for a space, holding the whole of `sc qc`'s
/// multi-line output squeezed into a 14-column row. Two faults in one line: a key shown as a
/// label, and a tool's raw dump shown as a value.
///
/// Swept over every backend that lists something, for the same reason the test above is: which
/// backend carries a `*_raw` property is an accident of the platform, and on Windows it is the
/// one the grader could not reach.
#[test]
fn info_shows_no_internal_property_key_as_a_field_label() {
    let f = Fixture::new("grade2-info-internals");

    let (listing, code) = f.run(&["list"]);
    assert_eq!(code, 0, "`list` failed:\n{listing}");
    let rows = one_row_per_backend(&listing);
    assert!(
        !rows.is_empty(),
        "nothing installed, so nothing to inspect:\n{listing}"
    );

    let mut leaks = Vec::new();
    for (backend, name) in &rows {
        let (out, code) = f.run(&["info", &format!("{backend}:{name}")]);
        if code != 0 {
            continue;
        }
        for line in out.lines() {
            // A field label is `word[ word]:` in the first column. An internal key reaches it
            // either as a `__`-prefixed tag or as a `_raw` dump whose underscore became a space.
            let Some((label, _)) = line.split_once(':') else {
                continue;
            };
            if line.starts_with(' ') || label.is_empty() {
                continue;
            }
            if label.starts_with("__") || label.ends_with(" raw") || label.contains("__") {
                leaks.push(format!(
                    "`info {backend}:{name}` printed the field `{label}:`"
                ));
            }
        }
    }

    assert!(
        leaks.is_empty(),
        "internal property keys reached the user as field labels:\n  {}\n\nA manager's own \
         output belongs under a heading that says whose words they are, not in a 14-column row \
         pretending to be a value.",
        leaks.join("\n  ")
    );
}
