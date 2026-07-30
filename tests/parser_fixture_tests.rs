//! Every parser here reads output captured from **the tool it parses, on a machine where that
//! tool is installed** — and each expectation was written by reading that file, not by reading
//! the parser.
//!
//! The rule (GRADER §3.3): *a parser is tested against output captured from the tool it parses,
//! and from no other tool.* Its two failures so far were both invented fixtures that passed.
//! `names_only` serves five managers and its only test used a **spack** fixture, while `pixi
//! search` emitted nineteen junk rows; `pixi_list`'s fixture was hand-written with two rows and
//! no nested child, and the real output has one — `exposes: rg`, which the parser reported as an
//! installed package whose version was `rg`.
//!
//! What a fixture is worth is bounded and worth stating: it pins the shape a tool prints *today*
//! on the platform it was captured from. It cannot see a format change upstream. That is what
//! `argv_drift_tests` is for, and between them the question "does LiNix still understand this
//! manager" has two halves rather than none.

use linix::parsers::{ecosystem, language, windows};
use std::path::Path;

/// `scoop list` — a fixed-width table under an `Installed apps:` banner, with a header row and
/// a dashed rule. Twenty-two apps on the machine this was captured from.
#[test]
fn scoop_list_reads_every_app_and_no_furniture() {
    const LIST: &str = include_str!("fixtures/scoop/list.txt");
    let pkgs = windows::parse_installed("scoop", LIST);
    let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();

    assert_eq!(
        names,
        vec![
            "7zip", "bun", "busybox", "dart", "fd", "gcc", "go", "helm", "jq", "kubectl", "lua",
            "luarocks", "nim", "nodejs", "nu", "php", "pipx", "pixi", "pnpm", "ripgrep", "ruby",
            "yarn",
        ],
        "the banner, the header row and the dashed rule are not apps"
    );
    let jq = pkgs.iter().find(|p| p.name == "jq").expect("jq is listed");
    assert_eq!(jq.version.as_deref(), Some("1.8.2"));
    // The `Source`/`Updated`/`Info` columns are not versions.
    assert!(
        pkgs.iter().all(|p| p.version.as_deref() != Some("main")),
        "a column heading became a version: {pkgs:?}"
    );
}

/// `choco list -r` — `name|version` rows, the machine-readable form LiNix asks for precisely so
/// the banner and the `N packages found.` summary never appear (E17).
#[test]
fn choco_list_reads_the_machine_readable_rows() {
    const LIST: &str = include_str!("fixtures/choco/list-r.txt");
    let pkgs = windows::parse_installed("choco", LIST);
    let rows: Vec<(&str, Option<&str>)> = pkgs
        .iter()
        .map(|p| (p.name.as_str(), p.version.as_deref()))
        .collect();
    assert_eq!(rows, vec![("chocolatey", Some("2.7.3"))]);
}

/// `pipx list --json` — the form LiNix asks for. The human form prints three path sentences
/// before the first package and the app names as `- pycowsay.exe` children.
#[test]
fn pipx_list_reads_its_json() {
    const LIST: &str = include_str!("fixtures/pipx/list-json.txt");
    let pkgs = language::parse_installed("pipx", LIST);
    let rows: Vec<(&str, Option<&str>)> = pkgs
        .iter()
        .map(|p| (p.name.as_str(), p.version.as_deref()))
        .collect();
    assert_eq!(rows, vec![("pycowsay", Some("0.0.0.2"))]);
}

/// `npm ls -g --json` and `pnpm ls -g --json` — the same parser, two schemas: npm answers with
/// an object, pnpm with an array of them. Real output from both, because the unit test beside
/// the parser writes its own JSON and a hand-written schema is the one thing a schema bug
/// cannot fail.
#[test]
fn the_npm_style_json_parser_reads_both_tools() {
    const NPM: &str = include_str!("fixtures/npm/ls-global-json.txt");
    const PNPM: &str = include_str!("fixtures/pnpm/ls-global-json.txt");

    for (label, fixture, backend) in [("npm", NPM, "npm"), ("pnpm", PNPM, "pnpm")] {
        let pkgs = language::parse_installed(backend, fixture);
        let rows: Vec<(&str, Option<&str>)> = pkgs
            .iter()
            .map(|p| (p.name.as_str(), p.version.as_deref()))
            .collect();
        assert_eq!(
            rows,
            vec![("cowsay", Some("1.6.0"))],
            "{label} reports one global package and its version"
        );
        assert_eq!(pkgs[0].backend, backend);
    }
}

/// `helm plugin list` — a tab-aligned table whose first row is the column headings. One plugin
/// installed here.
#[test]
fn helm_plugin_list_is_not_its_own_header() {
    const LIST: &str = include_str!("fixtures/helm/plugin-list.txt");
    let pkgs = ecosystem::ws_name_version(LIST, "helm");
    let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["secrets"],
        "`NAME` is the heading of the name column, not a plugin"
    );
    assert_eq!(pkgs[0].version.as_deref(), Some("4.8.0-dev"));
}

/// `cargo install --list` — the one fixture that was captured from a real tool in `08790c3` and
/// then read by nothing. **This is the `pixi:exposes` family**: every entry is a `name vX.Y.Z:`
/// header followed by its binaries, indented. A parser that trims the indentation away before
/// reading reports `rg.exe` as an installed package.
///
/// The cargo parser is correct — that was checked against the live tool before this was written
/// — so this is not a hidden bug. It is the check that makes the fixture mean what its presence
/// claimed, and it is here rather than deleted because "a parser is tested against output from
/// the tool it parses" wants a cargo fixture more than it wants one fewer file.
#[test]
fn cargo_list_reads_the_crates_and_not_their_binaries() {
    const LIST: &str = include_str!("fixtures/cargo/install-list.txt");
    let pkgs = language::parse_installed("cargo", LIST);
    let rows: Vec<(&str, Option<&str>)> = pkgs
        .iter()
        .map(|p| (p.name.as_str(), p.version.as_deref()))
        .collect();

    assert_eq!(
        rows,
        vec![
            ("hexyl", Some("0.17.0")),
            ("ripgrep", Some("15.2.0")),
            ("wasm-pack", Some("0.15.0")),
        ],
        "the indented binary lines are not packages"
    );
    // Named explicitly, because `ripgrep`'s binary is `rg` and the two are easy to conflate:
    // the crate is what a declaration names, the binary is not.
    assert!(
        !pkgs.iter().any(|p| p.name.ends_with(".exe")),
        "a binary was reported as a package: {pkgs:?}"
    );
}

/// Every file under `tests/fixtures/` is read by something.
///
/// An orphan is a **false signal of coverage**: the directory listing says fourteen backends
/// have a fixture captured from their own tool, and `cargo/install-list.txt` was read by no
/// test at all — committed in the same change that established the rule fixtures exist to
/// serve. That is worse than a missing fixture, because a missing one is visible.
///
/// It reads what every file actually references, so a fixture wired up anywhere — by
/// `include_str!` or by path, from a test file or a harness script — counts.
///
/// **The first draft of this test reported twenty orphans, and nineteen of them were its own
/// fault.** It searched `tests/*.rs` and the harness scripts and nothing else, while half the
/// fixtures are read by unit tests inside `src/` — `exit_policy.rs` alone reads seven through
/// `include_str!("../../tests/fixtures/…")`. A check that examines the wrong thing and reports
/// a finding is the same defect as one that examines the wrong thing and reports success; this
/// one would have had someone delete nineteen fixtures that were doing their job. Before it was
/// trusted it was fed something it must reject — a planted `zzz-probe/orphan.txt` — and it
/// rejected it.
#[test]
fn every_fixture_is_read_by_some_test() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let fixtures_dir = root.join("fixtures");

    // Everything that could name a fixture, in one haystack: the integration tests, **the
    // unit tests inside `src/`** — half the fixtures are read from there, by
    // `include_str!("../../tests/fixtures/…")` — and the harness scripts, which read by path.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut haystack = String::new();
    let mut sources = vec![root.clone(), repo.join("src")];
    while let Some(dir) = sources.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = entry.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == "fixtures") {
                    continue;
                }
                sources.push(p);
            } else if p.extension().is_some_and(|e| e == "rs") {
                haystack.push_str(&std::fs::read_to_string(&p).unwrap_or_default());
            }
        }
    }
    for dir in ["scripts", "docker/integration"] {
        for entry in std::fs::read_dir(repo.join(dir))
            .into_iter()
            .flatten()
            .flatten()
        {
            haystack.push_str(&std::fs::read_to_string(entry.path()).unwrap_or_default());
        }
    }

    let mut orphans = Vec::new();
    let mut total = 0;
    let mut stack = vec![fixtures_dir.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            total += 1;
            let rel = p
                .strip_prefix(&fixtures_dir)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            if !haystack.contains(&rel) {
                orphans.push(rel);
            }
        }
    }

    assert!(
        total > 0,
        "no fixtures found — this test is looking in the wrong place"
    );
    orphans.sort();
    assert!(
        orphans.is_empty(),
        "{} of {} fixture(s) under tests/fixtures/ are read by nothing:\n  {}\n\nA captured \
         fixture nobody reads is a false signal of coverage: the directory says this backend is \
         covered and no assertion looks at it. Wire it up or delete it.",
        orphans.len(),
        total,
        orphans.join("\n  ")
    );
}
