//! Parsers for the BSD package tools: FreeBSD's `pkg` and OpenBSD's `pkg_add`/`pkg_info` (U26).
//!
//! Both list installed packages as `name-version   description`, exactly the pkgsrc shape, and
//! the name itself can contain dashes (`py311-requests-2.31.0`) — so the split point is the LAST
//! dash whose right-hand side starts with a digit. This is the same heuristic `pkgsrc::parse_pkgin`
//! uses; kept in its own module because the backend labels differ and a shared label would put
//! the wrong prefix on the packages.

use crate::core::Package;
use crate::parsers::utils::sanitize;

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

fn parse_with_backend(output: &str, backend: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let token = line.split_whitespace().next()?;
            match split_name_version(token) {
                Some((name, version)) if !name.is_empty() => {
                    Some(Package::with_version(name, version, backend))
                }
                _ => None,
            }
        })
        .collect()
}

/// FreeBSD: `pkg info` (installed) and `pkg search` (remote catalogue) share the layout.
pub fn parse_pkg(output: &str) -> Vec<Package> {
    parse_with_backend(output, "pkg")
}

/// OpenBSD: `pkg_info` (installed) and `pkg_info -Q` (query the remote) share the layout.
pub fn parse_pkg_add(output: &str) -> Vec<Package> {
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
        let res = parse_pkg(input);
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
        let res = parse_pkg_add(input);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "wget");
        assert_eq!(res[0].version.as_deref(), Some("1.21.4"));
        assert_eq!(res[0].backend, "pkg_add", "an OpenBSD package is a pkg_add package");
    }

    #[test]
    fn a_legend_or_versionless_line_is_skipped() {
        // No dash-then-digit anywhere, so nothing is misread as a package.
        assert!(parse_pkg("Updating FreeBSD repository catalogue...").is_empty());
    }
}
