//! Parsers for the BSD package tools: FreeBSD's `pkg` and OpenBSD's `pkg_add`/`pkg_info` (U26).
//!
//! Both list installed packages as `name-version   description`, exactly the pkgsrc shape, and
//! the name itself can contain dashes (`py311-requests-2.31.0`) — so the split point is the LAST
//! dash whose right-hand side starts with a digit. This is the same heuristic `pkgsrc::parse_pkgin`
//! uses; kept in its own module because the backend labels differ and a shared label would put
//! the wrong prefix on the packages.

use crate::core::Package;
use crate::parsers::{or_unrecognised, ParseResult};
use crate::utils::text::sanitize;

/// Split a `name-version` token into `(name, version)`. `None` when no dash is followed by a
/// digit (a line with no version, e.g. a legend line).
fn split_name_version(token: &str) -> Option<(&str, &str)> {
    let bytes = token.as_bytes();
    for (idx, _) in token.rmatch_indices('-') {
        if let Some(&next) = bytes.get(idx + 1) {
            if next.is_ascii_digit() {
                return Some((&token[..idx], &token[idx + 1..]));
            }
        }
    }
    None
}

pub(crate) fn parse_with_backend(output: &str, backend: &str) -> ParseResult {
    let clean = sanitize(output);
    let candidates = crate::parsers::data_lines(&clean);
    let found = candidates
        .iter()
        .filter_map(|line| {
            let token = line.split_whitespace().next()?;
            match split_name_version(token) {
                Some((name, version)) if !name.is_empty() => {
                    Some(Package::with_version(name, version, backend))
                }
                _ => None,
            }
        })
        .collect();
    or_unrecognised(backend, found, &candidates)
}

/// FreeBSD: `pkg info` (installed) and `pkg search` (remote catalogue) share the layout.
pub fn parse_pkg(output: &str) -> ParseResult {
    parse_with_backend(output, "pkg")
}

/// OpenBSD: `pkg_info` (installed) and `pkg_info -Q` (query the remote) share the layout.
pub fn parse_pkg_add(output: &str) -> ParseResult {
    parse_with_backend(output, "pkg_add")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freebsd_pkg_info_names_and_versions() {
        let input = "pkg-1.20.9                     Package manager\n\
                     py311-requests-2.31.0          HTTP library\n\
                     vim-9.0.1897                   improved vi\n";
        let res = parse_pkg(input).expect("real `pkg info` output");
        assert_eq!(res.len(), 3);
        assert_eq!(res[0].name, "pkg");
        assert_eq!(res[0].version.as_deref(), Some("1.20.9"));
        assert_eq!(res[0].backend, "pkg");
        assert_eq!(res[1].name, "py311-requests");
        assert_eq!(res[1].version.as_deref(), Some("2.31.0"));
    }

    #[test]
    fn openbsd_pkg_info_names_and_versions_carry_the_pkg_add_label() {
        let input = "wget-1.21.4        retrieve files over HTTP/FTP\n\
                     gmake-4.4          GNU make\n";
        let res = parse_pkg_add(input).expect("real `pkg_info` output");
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "wget");
        assert_eq!(res[0].version.as_deref(), Some("1.21.4"));
        assert_eq!(
            res[0].backend, "pkg_add",
            "an OpenBSD package is a pkg_add package"
        );
    }

    #[test]
    fn a_line_that_parses_as_nothing_is_reported_rather_than_read_as_an_empty_machine() {
        // No dash-then-digit anywhere, so nothing is misread as a package — and this used to
        // end there, returning the empty vector that means *this machine has no packages*.
        // It does not mean that. A `pkg info` that printed one line and yielded no package is
        // a listing this parser did not understand, and the planner must not answer it by
        // scheduling every declared package as a fresh install.
        let err = parse_pkg("Updating FreeBSD repository catalogue...")
            .expect_err("one unreadable line is not an empty machine");
        assert_eq!(err.backend, "pkg");
        assert_eq!(err.data_lines, 1);
        assert!(err.sample.starts_with("Updating"), "{err:?}");
    }

    #[test]
    fn genuinely_empty_output_is_an_empty_machine() {
        // The control, without which the assertion above holds for a parser that errors on
        // everything: a manager with nothing installed prints nothing, and that is an answer.
        assert!(parse_pkg("").expect("no output is no packages").is_empty());
        assert!(parse_pkg("\n  \n").expect("blank output").is_empty());
    }
}

// ---------------------------------------------------------------------------
// Void Linux (`xbps`). Here rather than in its own module because the split point is the same
// question `split_name_version` above already answers — a `pkgver` token whose version starts
// after the last dash — and a second copy of that rule is how `pkgsrc.rs` came to be `bsd.rs`
// byte-for-byte.
// ---------------------------------------------------------------------------

/// Split an XBPS `pkgver` token (`<name>-<version>_<revision>`, e.g. `bash-5.2.15_2`) into
/// `(name, version)`. The version always begins with a digit after the final `-`, which is what
/// separates it from names that themselves contain dashes (`xbps-triggers-0.128_1`).
fn split_pkgver(tok: &str) -> Option<(&str, &str)> {
    let (name, ver) = tok.rsplit_once('-')?;
    if ver.chars().next()?.is_ascii_digit() {
        Some((name, ver))
    } else {
        None
    }
}

/// `xbps-query -l` / `xbps-query -m`. `-l` lines carry a two-character state flag
/// (`ii <pkgver> <desc>`); `-m` lines are the bare pkgver. Either way the pkgver is the first
/// token that parses, so this reads both.
pub fn parse_xbps_list(output: &str) -> ParseResult {
    let clean = sanitize(output);
    let candidates = crate::parsers::data_lines(&clean);
    let mut packages = Vec::new();
    for line in &candidates {
        if let Some((name, ver)) = line.split_whitespace().find_map(split_pkgver) {
            packages.push(Package::with_version(name, ver, "xbps"));
        }
    }
    or_unrecognised("xbps", packages, &candidates)
}

/// `xbps-query -Rs <query>`: `[-] bash-5.2.15_2   The GNU Bourne Again Shell`, where `[*]`
/// means installed.
pub fn parse_xbps_search(output: &str) -> Vec<Package> {
    let mut packages = Vec::new();
    for line in sanitize(output).lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let Some(pos) = tokens.iter().position(|t| split_pkgver(t).is_some()) else {
            continue;
        };
        let Some((name, ver)) = split_pkgver(tokens[pos]) else {
            continue;
        };
        let mut pkg = Package::with_version(name, ver, "xbps");
        let desc = tokens[pos + 1..].join(" ");
        if !desc.is_empty() {
            pkg.properties.insert("description".to_string(), desc);
        }
        packages.push(pkg);
    }
    packages
}

/// `xbps-query -x <pkg>` — run-time dependencies as version-constrained patterns
/// (`glibc>=2.36_1`), one per line. The constraint is not part of the name a later command
/// would have to use.
pub fn parse_xbps_dependencies(output: &str) -> Vec<String> {
    sanitize(output)
        .lines()
        .filter_map(|l| l.split(['>', '<', '=']).next())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod xbps_tests {
    use super::*;

    #[test]
    fn a_state_flag_is_not_part_of_the_package() {
        let out = "\
ii bash-5.2.15_2            The GNU Bourne Again Shell
ii xbps-triggers-0.128_1    XBPS triggers for Void Linux
";
        let pkgs = parse_xbps_list(out).expect("real xbps -l output");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "bash");
        assert_eq!(pkgs[0].version.as_deref(), Some("5.2.15_2"));
        assert_eq!(pkgs[0].backend, "xbps");
        // A name with an internal dash must not be split at the wrong hyphen.
        assert_eq!(pkgs[1].name, "xbps-triggers");
        assert_eq!(pkgs[1].version.as_deref(), Some("0.128_1"));
    }

    #[test]
    fn a_bare_pkgver_listing_reads_the_same_way() {
        let pkgs = parse_xbps_list("curl-8.4.0_1\ngit-2.42.0_1\n").expect("bare pkgver listing");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "curl");
        assert_eq!(pkgs[1].name, "git");
    }

    #[test]
    fn a_search_carries_its_install_marker_and_its_description() {
        let out = "[*] bash-5.2.15_2   The GNU Bourne Again Shell\n[-] zsh-5.9_2   The Z shell\n";
        let pkgs = parse_xbps_search(out);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "bash");
        assert_eq!(
            pkgs[0].properties.get("description").map(String::as_str),
            Some("The GNU Bourne Again Shell")
        );
        assert_eq!(pkgs[1].name, "zsh");
        assert_eq!(pkgs[1].version.as_deref(), Some("5.9_2"));
    }

    #[test]
    fn a_dependency_constraint_is_not_part_of_the_name() {
        assert_eq!(
            parse_xbps_dependencies("glibc>=2.36_1\noniguruma>=6.9\n\n"),
            vec!["glibc", "oniguruma"]
        );
    }
}
