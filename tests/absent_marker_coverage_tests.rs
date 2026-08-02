//! Which managers cannot tell LiNix that a package name does not exist.
//!
//! N-1 was E1 — a name nothing can install, left in `modules/imperative.txt`, failing every
//! later command — found alive on `npm` and `github` after it had been closed on `scoop` and
//! `cargo`. The interesting part is not that two backends were missed. It is that **nothing
//! anywhere held the bound**: withdrawal read a classification only 12 of 48 backends could
//! produce, and which 12 was not written down, not counted, and not derivable from anything
//! short of reading every registration site.
//!
//! So this file derives the set from the registry and compares it against a recorded one. A
//! backend that gains an absent-name classification must leave the list; one that loses it, or
//! a new backend registered without one, must join it, and either way somebody has to look.
//! The list is a measurement with a date on it, not a target.
//!
//! An uncovered backend is not broken — an unclassified failure keeps the declaration, which
//! is the safe direction. It is *silently limited*, which is the thing this repo keeps paying
//! for: `CLAUDE.md`'s "no silent caps" and `GRADER` §2's rule that a check must be able to
//! fail both say the same thing, which is that a bounded claim has to state its bound.

use linix::core::exit_policy;
use std::sync::Arc;

/// Every installable backend that cannot yet report a missing name.
///
/// A **set**, not a count, and the reason is the finding one layer up: which backends are
/// registered depends on the platform (48 on Windows, 56 on Ubuntu), so a number would mean a
/// different thing on every runner and could pass on one while a backend regressed on another.
/// A covered backend never appears here whatever the platform, so this list shrinks as work
/// lands and goes red the moment a name joins it.
///
/// Measured 2026-07-29 on Windows for the 36 registered here, after N-1 added `npm`, `gem`,
/// `pipx`, `go` and `pixi`. The ten platform-gated entries below are the ones this host does
/// not register, read from the `cfg!(target_os)` blocks in `backends::registry`; CI on Linux
/// and macOS is what confirms them, and a name missing from this list turns those legs red
/// rather than passing quietly, which is the point.
///
/// Deleting a name is the work. Each one is a manager that can then say "no such package"
/// instead of wedging a config, and its phrasing has to come from that manager's own output —
/// see the dated captures in `src/core/exit_policy.rs`. Never a guess: a wrong marker deletes
/// a declaration whose package is real.
const CANNOT_REPORT_A_MISSING_NAME: &[&str] = &[
    // Registered and measured on this host.
    "appimage",
    "asdf",
    "btrfs",
    "bun",
    "cabal",
    "composer",
    "conda",
    "dotnet",
    "emacs",
    "flatpak",
    // helm's failures are all about names that exist — an already-installed plugin, an
    // unsignable source — so it has permanent markers and no absent ones.
    "helm",
    "krew",
    "link",
    // Deliberate, and documented at `exit_policy::luarocks`: luarocks reports an unreachable
    // index as "no results for Lua 5.5", so believing it would withdraw declarations for rocks
    // that exist. This entry is a decision, not a gap.
    //
    // Worth re-opening, but not on a guess: since 2026-08-02 a transient marker outranks an
    // absent one, and luarocks already declares `failed searching manifest`. If an unreachable
    // index prints that line *alongside* the "no results" one, the reason for this entry is
    // gone. Nobody has measured it — the rock to run is `luarocks install <name>` against a
    // dead `--server`, the same probe that settled choco.
    "luarocks",
    "lvm",
    "mise",
    "mix",
    "nix",
    "opam",
    "pip",
    "pkg",
    "pkg_add",
    "pkgin",
    "pnpm",
    "psresource",
    "pub",
    "service",
    "setting",
    "snap",
    "spack",
    "stack",
    "uv",
    "vscode",
    "web",
    "yarn",
    "zfs",
    // Platform-gated: not registered on Windows, so not measured here.
    "zypper",
    "xbps",
    "yay",
    "paru",
    "guix",
    "emerge",
    "eopkg",
    "slackpkg",
    "mas",
    "macports",
];

/// Backends that resolve names themselves and answer with `Error::NoSuchPackage`, so they need
/// no output phrasings at all. They are covered by a different road and must not be counted as
/// gaps — `github` is the one N-1's reproduction actually used.
///
/// Each is a backend that never spawns a package manager: it queries an API or an index, gets
/// a definite answer, and says so in a value. Verified by following the constructor, not by
/// trusting this list — `the_named_exceptions_really_do_answer_structurally` below.
const RESOLVES_ITS_OWN_NAMES: &[&str] = &["github"];

#[tokio::test]
async fn every_backend_that_cannot_report_a_missing_name_is_recorded() {
    // Every backend the program registers, not the ones this host happens to have installed:
    // the count is a property of LiNix, and measuring it against a developer's machine is how
    // G-11's coverage audit came to report four lifecycles and call it a pass.
    let vfs = Arc::new(dashmap::DashMap::new());
    let mock = Arc::new(linix::core::executor::MockExecutor::new(vfs.clone()));
    let exec = linix::core::CommandExecutor::with_layer(
        true,
        false,
        mock,
        vfs,
        Arc::new(dashmap::DashMap::new()),
    );
    let config = linix::config::Config::default();
    let registry = linix::backends::create_default_registry(
        exec,
        &config,
        Arc::new(linix::app::hooks::LuaHooks::new(&config).expect("hooks")),
    )
    .await;

    let mut uncovered: Vec<String> = Vec::new();
    let mut covered: Vec<String> = Vec::new();

    for backend in registry.all() {
        // Only an installable backend can wedge a manifest: the line that stays is a line
        // something tried to install.
        if backend.as_installable().is_none() {
            continue;
        }
        let name = backend.name().to_string();
        if RESOLVES_ITS_OWN_NAMES.contains(&name.as_str()) {
            covered.push(name);
            continue;
        }
        if exit_policy::classifies_absent_names(&name) {
            covered.push(name);
        } else {
            uncovered.push(name);
        }
    }

    uncovered.sort();
    covered.sort();

    // Printed on every run, pass or fail. A bound nobody can see is the same as no bound.
    eprintln!(
        "absent-name coverage: {} of {} installable backends can report a missing name\n  \
         covered:   {}\n  uncovered: {}",
        covered.len(),
        covered.len() + uncovered.len(),
        covered.join(" "),
        uncovered.join(" ")
    );

    assert!(
        !covered.is_empty(),
        "no backend at all reports a missing name, so this test is measuring an empty registry \
         rather than the coverage it claims to measure."
    );

    let unrecorded: Vec<&String> = uncovered
        .iter()
        .filter(|n| !CANNOT_REPORT_A_MISSING_NAME.contains(&n.as_str()))
        .collect();
    assert!(
        unrecorded.is_empty(),
        "these installable backends cannot tell LiNix a package name does not exist, and are \
         not on the recorded list: {:?}\n\nEach one wedges `modules/imperative.txt` on a typo — \
         the line stays and every later command fails parsing the model, which is E1. Give it \
         an `absent_markers` entry in `src/core/exit_policy::for_manager`, taken from that \
         manager's own output and never from a guess, or add it to \
         `CANNOT_REPORT_A_MISSING_NAME` with the reason.",
        unrecorded
    );

    // The other half, and the half a list never checks about itself: an entry that is no
    // longer true. Only names registered *here* are judged, so a Linux-only manager is not
    // called stale by a Windows run.
    let stale: Vec<&&str> = CANNOT_REPORT_A_MISSING_NAME
        .iter()
        .filter(|n| covered.iter().any(|c| c == *n))
        .collect();
    assert!(
        stale.is_empty(),
        "these backends now report a missing name and are still listed as unable to: {:?}\n\
         Delete them from `CANNOT_REPORT_A_MISSING_NAME` — a list of known gaps that keeps \
         closed ones is how a bound stops meaning anything.",
        stale
    );
}

/// The exception list, tested rather than trusted. A name on it that does *not* answer
/// structurally would be a gap hiding inside the thing that excuses gaps — which is exactly
/// the shape `GRADE` §3 N-4 found in the drift gate.
#[test]
fn the_named_exceptions_really_do_answer_structurally() {
    let source = include_str!("../src/backends/github.rs");
    for name in RESOLVES_ITS_OWN_NAMES {
        assert_eq!(
            *name, "github",
            "a backend was added to the exception list without a check that it answers \
             structurally; add one before excusing it"
        );
        assert!(
            source.contains("Error::NoSuchPackage"),
            "`github` is excused from needing output phrasings because it returns \
             `NoSuchPackage` with the name it looked up. It no longer does, so the exception \
             is now a hole."
        );
    }
}

/// And the half that makes the count mean something: a covered backend must actually classify
/// its own real output. Fixtures captured from each tool, on the dates in `exit_policy.rs`.
#[test]
fn a_covered_manager_recognises_its_own_words_for_a_missing_name() {
    let cases = [
        ("npm", "npm error 404 Not Found - GET https://registry.npmjs.org/linix-no-such-pkg-zzz-9 - Not found"),
        ("gem", "ERROR:  Could not find a valid gem 'linix-no-such-gem-zzz' (>= 0) in any repository"),
        ("pipx", "ERROR: No matching distribution found for linix-no-such-pkg-zzz"),
        ("go", "go: module github.com/linix-zzz-nope/nope: git ls-remote failed: remote: Repository not found."),
        ("pixi", "  \u{2570}\u{2500}\u{25b6} Cannot solve the request because of: No candidates were found for linix-no-such-pkg-zzz *."),
        ("cargo", "error: could not find `linix-no-such-crate-zzz` in registry `crates-io` with version `*`"),
        ("scoop", "ERROR Couldn't find manifest for 'linix-no-such-pkg-zzz'."),
        ("apt", "E: Unable to locate package linix-no-such-pkg-zzz"),
        ("dnf", "Error: Unable to find a match: linix-no-such-pkg-zzz"),
        ("pacman", "error: target not found: linix-no-such-pkg-zzz"),
        ("apk", "ERROR: unable to select packages:"),
        ("brew", "Error: No available formula with the name \"linix-no-such-pkg-zzz\"."),
        ("choco", "linix-no-such-pkg-zzz not installed. The package was not found with the source(s) listed."),
        ("winget", "No package found matching input criteria."),
        ("nimble", "Error:  Package not found in nimble's package list."),
    ];

    for (manager, output) in cases {
        let policy = exit_policy::for_manager(manager);
        assert!(
            policy.names_an_absent_package(&exit_policy::ExitPolicy::haystack(
                output.as_bytes(),
                b""
            )),
            "`{manager}` does not recognise its own words for a name that is not there, so a \
             typo behind `{manager}:` wedges the config:\n  {output}"
        );
        assert!(
            exit_policy::classifies_absent_names(manager),
            "`{manager}` recognised the output but reports itself as unable to — the coverage \
             count and the behaviour disagree"
        );
    }
}

/// The other direction, and the one that keeps the markers from becoming a blunt instrument:
/// output that is *not* about a missing name must not be read as one, or the fix for a wedged
/// config becomes a program that deletes declarations.
#[test]
fn a_failure_about_a_name_that_exists_is_not_read_as_absent() {
    let cases = [
        // luarocks names the wrong cause: the rock exists and the downloader is broken. This
        // is the one manager deliberately left with no absent markers at all.
        ("luarocks", "Error: No results matching query were found for Lua 5.5."),
        ("luarocks", "Warning: Failed searching manifest: Failed downloading https://luarocks.org/manifest-5.5"),
        // helm: the plugin is there twice over, and the source is real but unsignable.
        ("helm", "Error: plugin already exists"),
        ("helm", "Error: plugin source does not support verification. Use --verify=false"),
        // A real crate that ships no program, and a real gem whose version pin is wrong.
        ("cargo", "error: there are no binaries in package `serde`"),
        ("nimble", "Error:  Version not found for package jsony"),
        // scoop declining to remove software that is not installed says nothing about the
        // bucket, and must never withdraw the line.
        ("scoop", "ERROR 'jq' isn't installed."),
        // A held lock is the classic transient, and the reason withdrawal reads existence
        // rather than permanence.
        ("apt", "E: Could not get lock /var/lib/dpkg/lock-frontend"),
    ];

    for (manager, output) in cases {
        assert!(
            !exit_policy::for_manager(manager).names_an_absent_package(
                &exit_policy::ExitPolicy::haystack(output.as_bytes(), b"")
            ),
            "`{manager}` read this as a name that does not exist, so LiNix would delete a \
             declaration whose package is real:\n  {output}"
        );
    }
}
