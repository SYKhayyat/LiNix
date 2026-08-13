//! **The four managers that matter on Linux had real captured `outdated` output and zero captured
//! `installed` output.** These are the installed listings, from the same containers.
//!
//! `apt.rs`, `pacman.rs`, `dnf.rs`, `dnf.rs` and `common.rs` each say
//! *"verbatim from a container"* — about the *outdated* listing. The installed listing is the one
//! the planner acts on: it decides what is already there, therefore what to install, therefore
//! what to remove. It was tested against strings somebody typed.
//!
//! The rule this pays into is written in `parsers/ecosystem.rs`, in this repo's own words:
//!
//! > *"a parser is tested against output captured from the tool it parses, and from no other
//! > tool."*
//!
//! ## Provenance
//!
//! Captured 2026-08-07 by running each manager's exact installed-listing argv — the one
//! `core/argv.rs` sends — in a stock container, and writing stdout to the fixture byte for byte.
//! Nothing was edited afterwards, including the parts that are inconvenient:
//! `zypper/search-installed.txt` opens with 52 lines of repository refresh, gpg-key prose and an
//! expired-key warning before its table starts, because that is what `zypper` prints on a cold
//! image and therefore what the parser has to survive.
//!
//! | fixture | image | argv |
//! |---|---|---|
//! | `apt/dpkg-query-installed.txt` | `ubuntu:24.04` | `dpkg-query -W -f='${Package} ${Version}\n'` |
//! | `apt/apt-mark-showmanual.txt` | `ubuntu:24.04` | `apt-mark showmanual` |
//! | `dnf/rpm-qa-installed.txt` | `fedora:41` | `rpm -qa --queryformat '%{NAME}\|%{VERSION}\n'` |
//! | `pacman/pacman-q-installed.txt` | `archlinux:base` | `pacman -Q` |
//! | `zypper/search-installed.txt` | `opensuse/leap:15` | `zypper --non-interactive search -i -t package` |
//! | `apk/info-v-installed.txt` | `alpine:3.20` | `apk info -v` |

use shall::parsers::{apt, bsd, common, dnf, pacman, ParseResult};

/// Every fixture must parse to a real listing — not to an empty one, and not to a failure.
///
/// The count is a floor rather than an exact number: a base image's package set moves between
/// releases, and pinning it exactly would make this test a chore that gets bumped without being
/// read. What cannot move is that a stock image has packages, so a parser that returns none has
/// broken.
fn assert_reads(label: &str, result: ParseResult, floor: usize, expect_name: &str) {
    let pkgs = result
        .unwrap_or_else(|e| panic!("{label}: the tool's own captured output did not parse — {e}"));
    assert!(
        pkgs.len() >= floor,
        "{label}: read {} package(s) from a stock image, expected at least {floor}",
        pkgs.len()
    );
    assert!(
        pkgs.iter().any(|p| p.name == expect_name),
        "{label}: `{expect_name}` is installed in this image and was not read. Names read: {:?}",
        pkgs.iter().take(12).map(|p| &p.name).collect::<Vec<_>>()
    );
    // The furniture must never become a package. Every one of these has been a real bug in this
    // repo in some manager or other: a header row, a separator rule, a banner, a blank.
    for p in &pkgs {
        assert!(!p.name.is_empty(), "{label}: a package with no name");
        assert!(
            !p.name.chars().all(|c| c == '-' || c == '+' || c == '='),
            "{label}: a separator rule became the package `{}`",
            p.name
        );
        assert!(
            !p.name.contains(char::is_whitespace),
            "{label}: `{}` is a sentence, not a package name",
            p.name
        );
    }
}

#[test]
fn apt_reads_its_own_dpkg_query_output() {
    const F: &str = include_str!("fixtures/apt/dpkg-query-installed.txt");
    assert_reads("apt (dpkg-query -W)", apt::parse_list(F), 90, "apt");
    // Versions are the point of this argv — `dpkg-query -W -f='${db:Status-Status} ${Package}
    // ${Version}'` exists so the planner can compare one. A read that dropped them would still
    // pass a name check.
    let pkgs = apt::parse_list(F).expect("captured output");
    assert!(
        pkgs.iter().all(|p| p.version.is_some()),
        "every installed dpkg-query row carries a version; some were read without one"
    );
    // **This capture is from a container where `figlet` was installed and then removed**, so it
    // carries the row the old lister got wrong: `config-files figlet 2.2.5-3`. The fixture is
    // the finding — a listing with no such row cannot show that B0 is fixed.
    assert!(
        F.contains("config-files figlet"),
        "the fixture was recaptured from a machine that has never removed a conffile-carrying \
         package, so it can no longer demonstrate what it exists to demonstrate"
    );
    assert!(
        !pkgs.iter().any(|p| p.name == "figlet"),
        "dpkg still knows `figlet` and `apt remove` is what left it in `config-files`; reading \
         it as installed is B0 — `list` names software that is gone and `sync` will not \
         reinstall it"
    );
}

/// `apt-mark showmanual` is the *explicit* set, and its shape is bare names with no versions —
/// which is why routing it through the `name version` parser silently returned nothing.
#[test]
fn apt_reads_its_own_showmanual_output() {
    const F: &str = include_str!("fixtures/apt/apt-mark-showmanual.txt");
    let pkgs = shall::parsers::parse_bare_names(F, "apt").expect("captured output");
    assert!(pkgs.len() >= 50, "read {} manual package(s)", pkgs.len());
    assert!(pkgs.iter().any(|p| p.name == "apt"));
    assert!(
        pkgs.iter().all(|p| p.version.is_none()),
        "this listing has no versions and none must be invented"
    );
}

#[test]
fn dnf_reads_its_own_rpm_qa_output() {
    const F: &str = include_str!("fixtures/dnf/rpm-qa-installed.txt");
    assert_reads("dnf (rpm -qa)", dnf::parse_rpm_qa(F, "dnf"), 100, "libgcc");
}

#[test]
fn pacman_reads_its_own_query_output() {
    const F: &str = include_str!("fixtures/pacman/pacman-q-installed.txt");
    assert_reads("pacman (-Q)", pacman::parse_list(F), 120, "acl");
}

/// **The fixture with the noise in it, which is the reason to capture rather than to type.**
///
/// `zypper --non-interactive search -i` on a cold image prints 52 lines before its table: a
/// repository refresh, gpg key details, and an expired-key warning. A hand-written fixture would
/// have started at the header row, and the parser that consumed the whole output looking for a
/// `---` rule — the `skip_while` bug this parser's own comment records — would have passed it.
#[test]
fn zypper_reads_its_own_table_out_of_a_page_of_repository_noise() {
    const F: &str = include_str!("fixtures/zypper/search-installed.txt");
    assert_reads(
        "zypper (search -i)",
        dnf::parse_zypper_search(F),
        100,
        "zypper",
    );
    let pkgs = dnf::parse_zypper_search(F).expect("captured output");
    // None of the prose above the table may become a package.
    for junk in ["Looking", "Building", "Warning", "Key", "Rpm", "gpgkey"] {
        assert!(
            !pkgs.iter().any(|p| p.name.starts_with(junk)),
            "repository noise became a package starting `{junk}`: {:?}",
            pkgs.iter()
                .filter(|p| p.name.starts_with(junk))
                .map(|p| &p.name)
                .collect::<Vec<_>>()
        );
    }
    // And the status column must not be read as the name.
    assert!(!pkgs.iter().any(|p| p.name == "i" || p.name == "i+"));
}

#[test]
fn apk_reads_its_own_info_output() {
    const F: &str = include_str!("fixtures/apk/info-v-installed.txt");
    assert_reads(
        "apk (info -v)",
        common::parse_dash_version_list(F, "apk"),
        10,
        "alpine-baselayout",
    );
    // The whole reason `parse_dash_version_list` requires a digit after the dash: `busybox-binsh`
    // must not split into `busybox` at version `binsh`, and a name that legitimately contains a
    // dash must survive whole.
    let pkgs = common::parse_dash_version_list(F, "apk").expect("captured output");
    assert!(
        pkgs.iter().any(|p| p.name == "alpine-baselayout-data"),
        "a hyphenated name was split at the wrong dash: {:?}",
        pkgs.iter().map(|p| &p.name).collect::<Vec<_>>()
    );
    assert!(pkgs.iter().all(|p| p.version.is_some()));
}

/// **`names_only` is the installed lister for `opam` and `emerge`, and had zero fixtures between
/// them.** This is opam's, from `ocaml/opam:debian-12-ocaml-5.2`.
///
/// The function's only test was hand-typed and labelled `"spack"` — a manager it does not serve
/// as an installed lister — which is the exact shape `ecosystem.rs` names: *"it passed, and
/// said nothing whatever about pixi, which is exactly where it was wrong."*
///
/// `emerge` still has none, and cannot get one from here: the Gentoo image bakes `SMOKE_ONLY=1`,
/// so emerge installs nothing and lists nothing, anywhere in this repo's matrix. Saying that is
/// better than inventing a fixture for it.
#[test]
fn opam_reads_its_own_short_listing() {
    use shall::parsers::ecosystem;
    const F: &str = include_str!("fixtures/opam/list-installed-short.txt");
    let pkgs = ecosystem::names_only(F, "opam").expect("captured `opam list --installed --short`");
    assert_eq!(pkgs.len(), 10, "{pkgs:?}");
    assert!(pkgs.iter().any(|p| p.name == "ocaml-base-compiler"));
    assert!(
        pkgs.iter().all(|p| p.version.is_none()),
        "`--short` prints names only; no version may be invented"
    );
    // A hyphenated name is one name. `names_only` takes the whole first token, which is what
    // makes it right here and wrong for a manager whose first token is a field label.
    assert!(pkgs.iter().any(|p| p.name == "ocaml-options-vanilla"));
}

/// The instrument, tested before it is trusted.
///
/// Every assertion above holds for an `assert_reads` that asserts nothing. This is the planted
/// falsehood: a fixture that is not the tool's output must fail, or the six tests above are
/// decoration.
#[test]
fn the_fixture_check_can_fail() {
    let hand_typed = "Package    Version\n---------  -------\n";
    assert!(
        apt::parse_list(hand_typed).is_err()
            || apt::parse_list(hand_typed).is_ok_and(|p| p.len() < 90),
        "a two-line invention must not satisfy the floor a real image meets"
    );
    // And the shared reader must not be satisfied by another tool's output — the exact rule
    // `ecosystem.rs` states, applied to itself. apk's output is `name-version` with no space;
    // apt's parser needs a space, so it reads nothing and now says so.
    const APK: &str = include_str!("fixtures/apk/info-v-installed.txt");
    assert!(
        apt::parse_list(APK).is_err(),
        "apk's listing fed to apt's parser must not read as an apt machine"
    );
    // The reverse, which is the direction that used to be silent: apt's output through apk's
    // parser produces junk rather than nothing, so this asserts the junk rather than pretending
    // the check caught it.
    const APT: &str = include_str!("fixtures/apt/dpkg-query-installed.txt");
    let crossed = common::parse_dash_version_list(APT, "apk").expect("it parses, wrongly");
    assert!(
        crossed
            .iter()
            .any(|p| p.name == "base" || p.version.is_none()),
        "cross-fed output should look wrong on inspection: {:?}",
        crossed.iter().take(5).map(|p| &p.name).collect::<Vec<_>>()
    );

    // **And the case that makes `ecosystem.rs`'s rule concrete — recorded as it now
    // behaves, and it changed for a reason worth keeping.**
    //
    // `bsd::parse_pkg` fed apt's listing used to *succeed*: it read 7 of the 92 lines — every
    // one a name containing a dash followed by a digit — and every one of the seven was wrong.
    // `libbz2-1.0` became the package `libbz2` at version `1.0`; `gcc-14-base` became `gcc` at
    // `14-base`. It passed, it was silent, and the packages it named could not be removed
    // because no such package exists.
    //
    // It now refuses, and nobody set out to fix it. apt's lister gained a leading
    // `${db:Status-Status}` field so that a package `apt remove` left in `config-files` stops
    // reading as installed (B0) — and a line that opens with a status word is a line the BSD
    // reader cannot get a `name-version` out of. **A format that carries a field only its own
    // tool emits is a format another tool's parser cannot silently half-read**, which is worth
    // more than the specific bug it was bought for.
    let e =
        bsd::parse_pkg(APT).expect_err("apt's listing must not read as a BSD one, however wrongly");
    assert_eq!(e.backend, "pkg");
    let real = apt::parse_list(APT).expect("captured output");
    assert!(
        real.iter().any(|p| p.name == "libbz2-1.0"),
        "and apt's own parser reads the whole name, which is the difference the rule is about"
    );
}
