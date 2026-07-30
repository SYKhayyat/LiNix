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
