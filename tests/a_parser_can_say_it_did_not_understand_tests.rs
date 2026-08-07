//! **Three layers of this program answered *"I do not understand this"* with *"there is nothing
//! here"*, and a reconciler cannot tell those apart, so it acted on the second reading.**
//!
//! `4d4a890` found this bug at one layer and fixed it beautifully — measured (sixteen concurrent
//! `winget list` from cold, three exits of `0x8A150001` in ~310 ms having written zero bytes),
//! swept across every config layer, self-critical about its own first cut. Its diagnosis names
//! the whole chain:
//!
//! > *"Through LiNix that became `Ok("")` → **a parser finding nothing** → `list_installed`
//! > answering `Ok(vec![])`. Nothing in the chain believed anything had failed."*
//!
//! It fixed `run_output`, `info`, `list` and `hook-reconcile`. **The parser — the link it named
//! itself — was not in the fix list**, not from carelessness but because that link could not be
//! fixed without changing a type, and nothing recorded that it had been skipped.
//!
//! ## Why the wrong branch is the dangerous one
//!
//! | what happened | what the planner does |
//! |---|---|
//! | the manager **fails** → `Err` → the backend is absent from `installed_sets` | `is_installed` answers true, removals stay scheduled — **safe** |
//! | the manager **succeeds, its format drifted** → present-and-empty | every declaration is planned as an install, every drift removal is dropped — **`check drift` reports the whole machine as drifted, `adopt` adopts nothing, exit 0** |
//!
//! Format drift is precisely the failure mode of the backends nobody has run.
//!
//! ## What this file gates
//!
//! Not "every parser errors on garbage" — that is not true and should not be, because a manager
//! with nothing installed is a real and common state and a parser that called *that* drift would
//! refuse to run on a clean machine. What is gated is that **the two answers are distinguishable
//! at all**, in every layer that carries them, and that the type has not quietly grown a way back
//! to conflating them.

use linix::parsers::{
    self, common, dnf, ecosystem, language, windows, ParseResult, Unrecognised,
};

// -------------------------------------------------------------------------------------------
// The distinction itself.
// -------------------------------------------------------------------------------------------

/// Nothing in, nothing out: an empty answer, not a failure.
///
/// The control for everything below. Without it, every assertion in this file would hold for a
/// parser layer that errored on absolutely everything — which would be a different bug with a
/// louder failure and no better a machine.
#[test]
fn a_manager_with_nothing_installed_says_so_and_is_believed() {
    let empty: &[(&str, ParseResult)] = &[
        ("apt", parsers::apt::parse_list("")),
        ("pacman", parsers::pacman::parse_list("")),
        ("dnf", dnf::parse_rpm_qa("", "dnf")),
        ("apk", common::parse_dash_version_list("", "apk")),
        ("spack", ecosystem::names_only("", "spack")),
        ("helm", ecosystem::ws_name_version("", "helm")),
        ("pkgin", parsers::pkgsrc::parse_pkgin("")),
        ("xbps", parsers::bsd::parse_xbps_list("")),
        ("mas", parsers::macos::parse_mas_list("")),
        ("cargo", language::parse_installed("cargo", "")),
    ];
    for (backend, result) in empty {
        let pkgs = result
            .as_ref()
            .unwrap_or_else(|e| panic!("`{backend}`: no output must read as no packages, got {e}"));
        assert!(pkgs.is_empty(), "`{backend}` invented a package out of nothing");
    }
}

/// Whitespace is still nothing. A manager that prints a trailing newline and no rows has told
/// the truth, and it must not be read as a format change.
#[test]
fn blank_output_is_still_an_empty_machine() {
    for (backend, result) in [
        ("apt", parsers::apt::parse_list("\n\n   \n")),
        ("pacman", parsers::pacman::parse_list("   \n")),
        ("spack", ecosystem::names_only("\n", "spack")),
    ] {
        assert!(
            result.expect("blank output is an empty machine").is_empty(),
            "`{backend}`"
        );
    }
}

/// A manager's own way of saying it has none is an empty machine, not an unread one.
///
/// This is the assertion that costs the most to get wrong, and it fails in the direction nobody
/// would notice: without it, every Mac with MacPorts and no ports installed, and every Windows
/// box with choco and nothing installed, would have its correct answer reported as drift.
#[test]
fn a_manager_saying_it_has_none_is_an_empty_machine() {
    assert!(
        parsers::macos::parse_macports_installed("No ports are installed.\n")
            .expect("MacPorts' own empty answer")
            .is_empty()
    );
    assert!(
        parsers::macos::parse_macports_installed(
            "The following ports are currently installed:\n"
        )
        .expect("a heading with nothing under it")
        .is_empty()
    );
    assert!(
        windows::parse_installed("choco", "0 packages installed.\n")
            .expect("choco's own empty answer")
            .is_empty()
    );
    assert!(
        ecosystem::names_only("No packages found.\n", "spack")
            .expect("spack's own empty answer")
            .is_empty()
    );
}

/// And the other half: output that carried something and yielded no package is reported.
///
/// One assertion per *shape* of the failure, because the shapes are what differ between the
/// layers — a line-oriented parser, a JSON reader, and a fixed-width table each have their own
/// way of not understanding, and each used to have its own way of spelling that as emptiness.
#[test]
fn output_the_parser_did_not_understand_is_reported_as_such() {
    // Line-oriented: bytes arrived, no line resolved to a package.
    let e = parsers::apt::parse_list("cannot-open-the-database\nno-such-file\n")
        .expect_err("dpkg-query output with no `name version` line is not a listing of nothing");
    assert_eq!(e.backend, "apt");
    assert_eq!(e.data_lines, 2);
    assert!(e.sample.starts_with("cannot-open"), "{e:?}");

    // JSON: not JSON at all. This is the shape `winget`, `scoop`, `dotnet`, `conda` and `pixi`
    // all shared, each through its own `unwrap_or_default()`.
    let e = ecosystem::pixi_list_json("usage: pixi global list [OPTIONS]", "pixi")
        .expect_err("a usage message is not an empty machine");
    assert_eq!(e.backend, "pixi");

    // Fixed-width table: the header row is gone, so there is nothing to slice columns against.
    // `winget list` that died before printing produces exactly this.
    let e = windows::parse_installed("winget", "\u{fffd}\u{fffd} something went wrong\n")
        .expect_err("a winget that printed no header is not a machine with no packages");
    assert_eq!(e.backend, "winget");

    // Column-delimited with no delimiter anywhere.
    let e = dnf::parse_rpm_qa("error: cannot open Packages database\n", "dnf")
        .expect_err("rpm's error is not an empty machine");
    assert_eq!(e.backend, "dnf");
}

/// **A sibling this gate found on its first run, recorded rather than quietly fixed.**
///
/// `apt::parse_list` reads `${Package} ${Version}` by splitting on the first space, so apt's own
/// error output — `E: Could not open lock file` — parses as a package named `E:` at version
/// *"Could not open lock file"*. That is the *junk* failure mode, not the empty one, and it is
/// therefore outside what `LX-1` changed: this file is about the two kinds of nothing, and no
/// return type distinguishes a wrong package from a right one.
///
/// It is asserted here as it actually behaves, so the finding cannot be lost and cannot be
/// mistaken for something this change already covered. Fixing it means constraining the name to
/// what dpkg can emit, which is a different argument with a different blast radius — every
/// backend sharing the shape, and multi-arch names that legitimately carry a colon.
#[test]
fn junk_is_a_different_failure_from_emptiness_and_this_change_does_not_address_it() {
    let pkgs = parsers::apt::parse_list("E: Could not open lock file\nE: Are you root?\n")
        .expect("this is not the empty-vs-unread failure; it produces packages");
    assert_eq!(pkgs.len(), 2, "{pkgs:?}");
    assert_eq!(pkgs[0].name, "E:", "apt's error prefix reads as a package name");
    assert_eq!(
        pkgs[0].version.as_deref(),
        Some("Could not open lock file"),
        "and the rest of the sentence reads as its version"
    );
}

/// A backend nobody wired a reader for is a question that cannot be answered, not an answer.
///
/// Both dispatches had a `_ => vec![]` arm. It is the widest spelling of the bug in the tree:
/// not one manager's format drifting, but *any* name at all reporting a bare machine.
#[test]
fn a_backend_with_no_reader_wired_is_not_an_empty_machine() {
    for (layer, result) in [
        ("language", language::parse_installed("nosuchmanager", "some output\n")),
        ("windows", windows::parse_installed("nosuchmanager", "some output\n")),
    ] {
        let e = result.expect_err("an unwired backend must not report an empty machine");
        assert_eq!(e.backend, "nosuchmanager", "{layer}");
    }
}

/// The manager that cannot list at all has a name for that, and it is not the empty vector.
///
/// `stack` installs and cannot enumerate. Its registry row read `installed_fn: |_| vec![]` —
/// character for character the most dangerous return in the region, standing in for *"this
/// question cannot be asked"*.
#[test]
fn a_manager_with_no_listing_verb_says_that_rather_than_nothing() {
    use linix::parsers::{CannotList, OutputParser};
    let p = CannotList("stack");
    let e = p
        .parse_installed("anything at all")
        .expect_err("`no listing verb` and `nothing installed` are different answers");
    assert_eq!(e.backend, "stack");
    assert!(e.sample.contains("no listing verb"), "{e:?}");
}

// -------------------------------------------------------------------------------------------
// The judgement, made once.
// -------------------------------------------------------------------------------------------

/// `or_unrecognised` is the whole rule, so it is tested directly rather than only through the
/// sixty parsers that call it.
#[test]
fn the_shared_judgement_is_the_rule_and_nothing_more() {
    use linix::core::Package;
    let one = || vec![Package::new("jq", "test")];

    // Found something: always fine, whatever else was in the output.
    assert!(parsers::or_unrecognised("test", one(), &[]).is_ok());
    assert!(parsers::or_unrecognised("test", one(), &["noise", "more"]).is_ok());
    // Found nothing out of nothing: an empty machine.
    assert!(parsers::or_unrecognised("test", vec![], &[])
        .expect("no candidates is an empty machine")
        .is_empty());
    // Found nothing out of something: the failure.
    let e = parsers::or_unrecognised("test", vec![], &["a line nobody read"])
        .expect_err("candidates that yielded nothing");
    assert_eq!(e.data_lines, 1);
    assert_eq!(e.sample, "a line nobody read");
}

/// The prose filter decides which lines even count as candidates, so getting it wrong moves the
/// boundary above without touching a single parser.
#[test]
fn a_managers_prose_is_not_evidence_that_anything_went_unread() {
    for prose in [
        "The following ports are currently installed:",
        "No ports are installed.",
        "No global environments found.",
        "Nothing to list here.",
        "0 packages installed.",
        "Installed apps:",
    ] {
        assert!(parsers::is_prose_line(prose), "{prose:?} is prose");
    }
    for data in [
        "jq 1.7.1",
        "ripgrep|15.2.0",
        "py311-requests-2.31.0  HTTP library",
        "7zip     26.00     main",
        // A package whose name begins with "no" is a package.
        "nodejs 22.1.0",
        // A description may end in a full stop; the line still opens with a name.
        "curl - transfer a URL.",
    ] {
        assert!(!parsers::is_prose_line(data), "{data:?} is data");
    }
}

// -------------------------------------------------------------------------------------------
// The oracle: the type must not be able to grow a way back.
// -------------------------------------------------------------------------------------------

/// `Unrecognised` must carry enough to act on.
///
/// A failure that says only *"parse error"* sends the reader back to reproduce it, which for a
/// format change on somebody else's distro is the one thing they cannot do for us. The manager,
/// the count and the first unread line are what turn a bug report into a fixture.
#[test]
fn a_failure_names_the_manager_and_the_bytes() {
    let e = Unrecognised {
        backend: "zypper".into(),
        data_lines: 3,
        sample: "S  | Name | Summary".into(),
    };
    let text = e.to_string();
    assert!(text.contains("zypper"), "{text}");
    assert!(text.contains('3'), "{text}");
    assert!(text.contains("S  | Name | Summary"), "{text}");
    // And it must say what it is refusing to do, because that is the part a reader would
    // otherwise have to know this file to understand.
    assert!(text.contains("empty machine"), "{text}");
}

/// The error crossing into the program's own error type keeps its words, and lands in the
/// variant whose retry policy is right.
#[test]
fn the_failure_survives_the_crossing_into_the_programs_error_type() {
    use linix::core::{Error, Retryability};
    let e: Error = Unrecognised {
        backend: "opam".into(),
        data_lines: 9,
        sample: "opam: unknown option '--short'".into(),
    }
    .into();
    assert!(matches!(e, Error::Unreadable(_)));
    assert!(e.to_string().contains("opam"), "{e}");
    // A manager will print the same bytes next time and the parser will fail to recognise them
    // the same way. Retrying an output-format change is time spent proving it did not change
    // back.
    assert_eq!(e.retryability(), Retryability::Permanent);
}

/// `parse_search` is deliberately **not** fallible, and the asymmetry is the finding rather than
/// an oversight — so it is asserted, not left to be rediscovered as an inconsistency and
/// "fixed".
///
/// A search that returns nothing is a fact the user asked for and can see on their screen. An
/// installed listing that returns nothing is a fact the *planner* acts on, invisibly, in the
/// direction of installing everything and removing nothing.
#[test]
fn only_the_installed_side_is_fallible_and_that_is_deliberate() {
    use linix::core::Package;
    fn assert_is_vec(_: Vec<Package>) {}
    fn assert_is_result(_: ParseResult) {}

    assert_is_vec(parsers::apt::parse_search("nothing at all"));
    assert_is_vec(windows::parse_search("winget", "nothing at all"));
    assert_is_result(parsers::apt::parse_list("nothing at all"));
    assert_is_result(windows::parse_installed("winget", "nothing at all"));
}
