//! Parsers for pkgsrc's binary package tool, `pkgin`.
//!
//! pkgin joins the package name and version with a dash and appends a short
//! description, e.g. `git-base-2.39.0nb1  GIT version control suite`. Because the
//! name itself can contain dashes (`py311-requests-2.31.0`), the split point is the
//! LAST dash whose right-hand side starts with a digit — the classic pkgsrc heuristic.

use crate::core::Package;
use crate::utils::text::sanitize;

/// Splits a pkgsrc `name-version` token into `(name, version)`. Returns `None` when no
/// dash is followed by a digit (i.e. the token carries no version).
fn split_name_version(token: &str) -> Option<(&str, &str)> {
    let bytes = token.as_bytes();
    // Walk dashes from the right; the first one followed by a digit is the boundary.
    for (idx, _) in token.rmatch_indices('-') {
        if let Some(&next) = bytes.get(idx + 1) {
            if next.is_ascii_digit() {
                return Some((&token[..idx], &token[idx + 1..]));
            }
        }
    }
    None
}

/// Parses the output of `pkgin list` / `pkgin search`. Both share the
/// `name-version  description` layout. Trailing legend lines from `search`
/// (`=: package is installed and up to date`, blank lines) are skipped because their
/// first token has no digit-prefixed dash.
pub fn parse_pkgin(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let token = line.split_whitespace().next()?;
            match split_name_version(token) {
                Some((name, version)) if !name.is_empty() => {
                    Some(Package::with_version(name, version, "pkgin"))
                }
                _ => None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_name_and_version_with_dashed_name() {
        let input = "git-base-2.39.0nb1  GIT core tools\n\
                     py311-requests-2.31.0  HTTP library\n\
                     wget-1.21.3  Retrieves files\n";
        let res = parse_pkgin(input);
        assert_eq!(res.len(), 3);
        assert_eq!(res[0].name, "git-base");
        assert_eq!(res[0].version.as_deref(), Some("2.39.0nb1"));
        assert_eq!(res[1].name, "py311-requests");
        assert_eq!(res[1].version.as_deref(), Some("2.31.0"));
        assert_eq!(res[2].name, "wget");
    }

    #[test]
    fn skips_legend_and_status_lines() {
        let input = "vim-9.0.1  Vim editor\n\
                     =: package is installed and up to date\n\
                     >: package is installed but newer version available\n\
                     \n";
        let res = parse_pkgin(input);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].name, "vim");
    }
}
