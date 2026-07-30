//! GRADER round 4, 2026-07-30 — RED. `pixi`'s `list` parser invents a package.
//!
//! E16 was "pixi's *search* parser emits 19 junk rows". It was closed, with two fixtures captured
//! from the tool. The `list` parser in the same file, for the same backend, still has none — and
//! the output it is given on a real machine has a nested row the invented fixture does not:
//!
//!     $ pixi global list
//!     Global environments as specified in 'C:\Users\...\pixi-global.toml'
//!     └── ripgrep: 15.2.0
//!         └─ exposes: rg
//!
//!     $ linix list -b pixi
//!     pixi         ripgrep                          15.2.0
//!     pixi         exposes                          rg           <- not a package
//!
//! `exposes` is pixi's word for "this environment puts these binaries on PATH". LiNix reports it
//! as an installed package whose version is `rg`. The unit test in `src/parsers/ecosystem.rs`
//! passes because its fixture was written by hand — two rows, no `exposes:` child — which is
//! GRADER §3.3's rule in one instance: *a parser is tested against output captured from the tool
//! it parses, and from no other tool.*
//!
//! The fixture beside this test is `pixi global list` on the grading host, byte for byte.

use linix::parsers::ecosystem::pixi_list;

const FIXTURE: &str = include_str!("fixtures/pixi/list-one-tool.txt");

/// One tool installed → one package. The `exposes:` line is a property of that tool, not a
/// second tool.
#[test]
fn pixi_list_reads_one_tool_as_one_package() {
    let pkgs = pixi_list(FIXTURE, "pixi");
    let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["ripgrep"],
        "`pixi global list` reports one installed tool; the parser reported {:?}. A row LiNix \
         invents here is a package `check` counts, `list` prints, `adopt` can write into a \
         manifest and `sync` will then try to install.",
        names
    );
    assert_eq!(pkgs[0].version.as_deref(), Some("15.2.0"));
}

/// The empty case, from the same tool: pixi prints the banner and nothing under it. Three of the
/// four cases GRADER §3.3 names — empty, single, not-found, error — are where junk rows come from.
#[test]
fn pixi_list_with_nothing_installed_is_empty() {
    let empty =
        "Global environments as specified in 'C:\\Users\\u\\.pixi\\manifests\\pixi-global.toml'\n";
    let pkgs = pixi_list(empty, "pixi");
    let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
    assert!(
        pkgs.is_empty(),
        "an empty pixi reported {:?} installed package(s)",
        names
    );
}
