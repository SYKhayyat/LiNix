//! Which managers cannot tell Shall that a package name does not exist.
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

use shall::core::exit_policy;
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
///
/// **Twelve to twenty-five on 2026-08-21, and the method matters more than the number. THREE
/// QUESTIONS PER MANAGER, not one.** An install resolves a name, an index and a version, and a
/// marker is safe only once all three have been asked - so each phrasing came from running that
/// manager's own install argv three ways: against a name that does not exist, against the same
/// name with `--network none`, and against a package that DOES exist at an impossible version.
/// A backend on `capability::CANNOT_PIN_VERSION` has no third axis by construction, which is
/// how `nix`, `krew` and `slackpkg` are safe without one.
///
/// **The third question found a marker that had already shipped wrong.** `pipx` carried
/// `no matching distribution found for`, and pip says that about a VERSION as readily as about
/// a name - so a bad `@version=` on a real package withdrew its declaration. The fixture below
/// used to be a one-line capture that stopped just above the line carrying the answer, which is
/// how a wrong marker passed the test written to catch it. `(from versions: none)` is the
/// discriminator, and `pip` inherits both the bug and the fix.
///
/// The other two passes earn their keep too: `mix` answers from a stale cache when Hex is
/// unreachable in the same words it uses for a name that never existed, and `dart pub` refuses
/// a hyphenated name before it asks pub.dev at all.
///
/// **The first sweep was scoped to `builtin_backends.toml` and reported as if it were the whole
/// board.** It was one table of two: every Rust-implemented backend was invisible to it, and
/// eight of those were sitting in the `tools` image already built. `bun`, `conda`, `dotnet`,
/// `mise`, `nix` and `pip` came from the second pass. Whatever asks this question next should
/// take its list from the registry, which is what the test below already does.
///
/// What is left here is what no machine to hand could ask. `guix`, `emerge` and `eopkg` have
/// nightly images and want that leg. `yarn` is a different shape of gap: it answers
/// `https://registry.yarnpkg.com/<name>: Not found`, which puts the package name BETWEEN the
/// two halves that would identify the sentence, so no contiguous marker matches it. `pnpm` and
/// `asdf` never reach a lookup on the install path Shall drives.
const CANNOT_REPORT_A_MISSING_NAME: &[&str] = &[
    // Registered and measured on this host.
    "appimage",
    "asdf",
    "btrfs",
    // NOT a gap - measured 2026-08-21 and left alone. `conda install <absent>` and
    // `conda install six=99.99.99` both answer `PackagesNotFoundInChannelsError: The following
    // packages are not available from current channels`. One sentence, two facts, and no line
    // above it separates them - the same shape that keeps `luarocks` here.
    "conda",
    "emacs",
    "flatpak",
    // helm's failures are all about names that exist — an already-installed plugin, an
    // unsignable source — so it has permanent markers and no absent ones.
    "helm",
    "link",
    // NOT a gap — a decision, and since 2026-08-21 a measured one. `luarocks` prints
    // `No results matching query were found for Lua 5.1.` for THREE different facts: a rock
    // that does not exist, an index it could not reach, and a rock that exists at a version
    // the user did not ask for. The transient guard separates the second; nothing separates
    // the third, so believing the summary withdraws a real declaration over a wrong version
    // pin. See `exit_policy::luarocks`.
    "luarocks",
    "lvm",
    // `nixos:` never asks nix whether an attribute exists — it writes the name into a generated
    // module and lets `nixos-rebuild` be the judge. A typo is caught at rebuild time, by nix,
    // with nix's own message about the attribute: a better error than Shall could synthesise,
    // arriving one step later than this table would prefer.
    "nixos",
    "pkg",
    "pkg_add",
    "pkgin",
    "pnpm",
    "psresource",
    "service",
    "setting",
    "snap",
    "stack",
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
    // the count is a property of Shall, and measuring it against a developer's machine is how
    // G-11's coverage audit came to report four lifecycles and call it a pass.
    let vfs = Arc::new(dashmap::DashMap::new());
    let mock = Arc::new(shall::core::executor::MockExecutor::new(vfs.clone()));
    let exec = shall::core::CommandExecutor::with_layer(
        true,
        false,
        mock,
        vfs,
        Arc::new(dashmap::DashMap::new()),
    );
    let config = shall::config::Config::default();
    let registry = shall::backends::create_default_registry(
        exec,
        &config,
        Arc::new(shall::app::hooks::LuaHooks::new(&config).expect("hooks")),
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
        "these installable backends cannot tell Shall a package name does not exist, and are \
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
        ("npm", "npm error 404 Not Found - GET https://registry.npmjs.org/shall-no-such-pkg-zzz-9 - Not found"),
        ("gem", "ERROR:  Could not find a valid gem 'shall-no-such-gem-zzz' (>= 0) in any repository"),
        ("pipx", "ERROR: Could not find a version that satisfies the requirement shall-no-such-pkg-zzz (from versions: none)\nERROR: No matching distribution found for shall-no-such-pkg-zzz"),
        ("pip", "ERROR: Could not find a version that satisfies the requirement shall-no-such-pkg-zzz (from versions: none)\nERROR: No matching distribution found for shall-no-such-pkg-zzz"),
        ("bun", "error: GET https://registry.npmjs.org/shall-no-such-pkg-zzz - 404"),
        ("dotnet", "NuGetPackageNotFoundException: shall-no-such-pkg-zzz::[*, ) is not found in NuGet feeds https://api.nuget.org/v3/index.json\"."),
        ("mise", "mise ERROR Failed to install shall-no-such-pkg-zzz@latest: shall-no-such-pkg-zzz not found in mise tool registry"),
        ("nix", "error: flake 'flake:nixpkgs' does not provide attribute 'legacyPackages.x86_64-linux.shall-no-such-pkg-zzz' or 'shall-no-such-pkg-zzz'"),
        ("go", "go: module github.com/shall-zzz-nope/nope: git ls-remote failed: remote: Repository not found."),
        ("pixi", "  \u{2570}\u{2500}\u{25b6} Cannot solve the request because of: No candidates were found for shall-no-such-pkg-zzz *."),
        ("cargo", "error: could not find `shall-no-such-crate-zzz` in registry `crates-io` with version `*`"),
        ("scoop", "ERROR Couldn't find manifest for 'shall-no-such-pkg-zzz'."),
        ("apt", "E: Unable to locate package shall-no-such-pkg-zzz"),
        ("dnf", "Error: Unable to find a match: shall-no-such-pkg-zzz"),
        ("pacman", "error: target not found: shall-no-such-pkg-zzz"),
        ("apk", "ERROR: unable to select packages:"),
        ("brew", "Error: No available formula with the name \"shall-no-such-pkg-zzz\"."),
        ("choco", "shall-no-such-pkg-zzz not installed. The package was not found with the source(s) listed."),
        ("winget", "No package found matching input criteria."),
        ("nimble", "Error:  Package not found in nimble's package list."),
        ("cabal", "cabal: Unknown package \"shall-no-such-pkg-zzz\"."),
        ("composer", "  Could not find a matching version of package shall-no-such-pkg-zzz. Check   "),
        ("opam", "[ERROR] No package named shall-no-such-pkg-zzz found."),
        ("spack", "==> Error: cannot concretize 'shall-no-such-pkg-zzz', since 'shall-no-such-pkg-zzz' does not exist"),
        ("uv", "  \u{2570}\u{2500}\u{25b6} Because shall-no-such-pkg-zzz was not found in the package registry"),
        ("krew", "plugin \"shall-no-such-pkg-zzz\" does not exist in the plugin index"),
        ("pub", "Because pub global activate depends on shall_no_such_zzz any which doesn't exist (could not find package shall_no_such_zzz at https://pub.dev), version solving failed."),
        ("mix", "** (Mix) No package with name shall_no_such_zzz (from: mix.exs) in registry"),
        ("slackpkg", "No packages match the pattern for install. Try:"),
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
        // luarocks prints one summary for three different facts, and the third is why it
        // still has no absent marker: `luarocks install luafilesystem 99.99.99` — a real rock,
        // a working index, a version that does not exist — prints exactly this, with no
        // warning above it for any guard to catch. Measured 2026-08-21.
        ("luarocks", "Error: No results matching query were found for Lua 5.1."),
        ("luarocks", "Warning: Failed searching manifest: Failed downloading https://luarocks.org/manifest-5.1\nError: No results matching query were found for Lua 5.1."),
        ("luarocks", "Warning: Failed searching manifest: Failed downloading https://luarocks.org/manifest-5.5"),
        // Hex unreachable: mix answers from a STALE CACHE in the same words it uses for a
        // name that never existed, and only the line above it says which happened.
        ("mix", "Failed to fetch record for real_pkg from registry (using cache instead)\n** (Mix) No package with name real_pkg (from: mix.exs) in registry"),
        // **The bug this method was built to find, and it was already shipped.** `pipx` carried
        // `no matching distribution found for` as its absent marker, and pip says that about a
        // VERSION as readily as about a name - so a bad `@version=` on a real package withdrew
        // the declaration for it. The line above the summary is the whole discriminator: `none`
        // means pip found nothing, a list means it found the package and not that version.
        ("pipx", "ERROR: Could not find a version that satisfies the requirement black==99.99.99 (from versions: 18.3a0, 24.1.0, 26.5.1)\nERROR: No matching distribution found for black==99.99.99"),
        ("pip", "ERROR: Could not find a version that satisfies the requirement six==99.99.99 (from versions: 1.16.0, 1.17.0)\nERROR: No matching distribution found for six==99.99.99"),
        // NuGet writes one sentence for a name it does not have and for a version it does not
        // have. `::[*, )` is an unbounded range, which is what a request with no pin looks like,
        // so a pinned request cannot match the marker.
        ("dotnet", "NuGetPackageNotFoundException: dotnetsay::99.99.99 is not found in NuGet feeds https://api.nuget.org/v3/index.json\"."),
        // bun says the package exists in so many words, which is `Permanent` and not absent -
        // the line carries a version to correct.
        ("bun", "error: No version matching \"99.99.99\" found for specifier \"left-pad\" (but package exists)"),
        // mise answers a bad version from the release API, not from its registry.
        ("mise", "mise ERROR Failed to install aqua:jqlang/jq@99.99.99: HTTP status client error (404 Not Found) for url (https://api.github.com/repos/jqlang/jq/releases/tags/jq-99.99.99)"),
        // A dart name with a hyphen is refused before pub.dev is asked at all.
        ("pub", "Not a valid package name: \"shall-no-such-pkg-zzz\""),
        // slackpkg with no mirror never reaches its pattern at all, and says so about the
        // mirror. Kept as a case because the marker above is only safe while that stays true.
        ("slackpkg", "Error downloading from http://slackware.osuosl.org/slackware64-15.0/.
Please check your mirror and try again."),
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
        // cabal cannot verify Hackage's root, so it has not looked the name up at all. The
        // package is real and the machine's trust anchor is stale.
        ("cabal", "<repo>/root.json does not have enough signatures signed with the appropriate keys"),
    ];

    for (manager, output) in cases {
        assert!(
            !exit_policy::for_manager(manager).names_an_absent_package(
                &exit_policy::ExitPolicy::haystack(output.as_bytes(), b"")
            ),
            "`{manager}` read this as a name that does not exist, so Shall would delete a \
             declaration whose package is real:\n  {output}"
        );
    }
}
