//! Parsers for pkgsrc's binary package tool, `pkgin`.
//!
//! pkgin joins the package name and version with a dash and appends a short description, e.g.
//! `git-base-2.39.0nb1  GIT version control suite`. Because the name itself can contain dashes
//! (`py311-requests-2.31.0`), the split point is the LAST dash whose right-hand side starts with
//! a digit — the classic pkgsrc heuristic.
//!
//! **That heuristic lives in `bsd.rs` and this module calls it.** It used to be written out
//! again here, character for character, alongside a `parse_pkgin` that was
//! `bsd::parse_with_backend` with the label inlined. `bsd.rs-94` had already noticed and said
//! so out loud — *"a second copy of that rule is how `pkgsrc.rs` came to be `bsd.rs`
//! byte-for-byte"* — while the second copy went on compiling one directory away.
//!
//! `pkgin` keeps its own module because the *backend label* is the thing that differs, and a
//! shared label would put the wrong prefix on every package. One rule, two labels: that is the
//! whole of what this file is for.

use crate::parsers::ParseResult;

/// Parses the output of `pkgin list` / `pkgin search`. Both share the `name-version description`
/// layout. Trailing legend lines from `search` (`=: package is installed and up to date`, blank
/// lines) are skipped because their first token has no digit-prefixed dash.
pub fn parse_pkgin(output: &str) -> ParseResult {
    crate::parsers::bsd::parse_with_backend(output, "pkgin")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_name_and_version_with_dashed_name() {
        let input = "git-base-2.39.0nb1  GIT core tools\n\
                     py311-requests-2.31.0  HTTP library\n\
                     wget-1.21.3  Retrieves files\n";
        let res = parse_pkgin(input).expect("real `pkgin list` output");
        assert_eq!(res.len(), 3);
        assert_eq!(res[0].name, "git-base");
        assert_eq!(res[0].version.as_deref(), Some("2.39.0nb1"));
        assert_eq!(res[1].name, "py311-requests");
        assert_eq!(res[1].version.as_deref(), Some("2.31.0"));
        assert_eq!(res[2].name, "wget");
    }

    /// The label is the entire reason this module exists, so it is the thing to assert. A
    /// delegation that reached the shared rule and carried `pkg`'s prefix would pass every
    /// other test in this file and put every pkgsrc package under the wrong backend.
    #[test]
    fn a_pkgsrc_package_carries_the_pkgin_label() {
        let res = parse_pkgin("wget-1.21.3  Retrieves files\n").expect("one package");
        assert_eq!(res[0].backend, "pkgin");
    }

    #[test]
    fn skips_legend_and_status_lines() {
        let input = "vim-9.0.1  Vim editor\n\
                     =: package is installed and up to date\n\
                     >: package is installed but newer version available\n\
                     \n";
        let res = parse_pkgin(input).expect("one package plus legend");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].name, "vim");
    }

    /// Legend lines and nothing else is not an empty machine. `pkgin search` for a term that
    /// matches nothing prints exactly its legend, and a caller that read that as *"no packages
    /// installed"* would be answering a search with the machine's state.
    #[test]
    fn a_listing_of_nothing_but_legend_is_reported() {
        let err = parse_pkgin("=: package is installed and up to date\n")
            .expect_err("a legend alone is not a package listing");
        assert_eq!(err.backend, "pkgin");
    }
}
