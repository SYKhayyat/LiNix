use crate::core::Package;
use crate::parsers::{or_unrecognised, ParseResult};
use crate::utils::text::sanitize;

/// Parses output from 'pacman -Q' for installed packages.
/// Expected input format: "name version"
pub fn parse_list(output: &str) -> ParseResult {
    parse_list_for(output, "pacman")
}

/// Like [`parse_list`], but stamps a caller-supplied backend name. AUR helpers
/// (`yay`, `paru`) speak pacman's `-Q` syntax verbatim but must label their packages
/// with their own backend so state tracking stays per-backend correct.
pub fn parse_list_for(output: &str, backend: &str) -> ParseResult {
    let clean = sanitize(output);
    let candidates = crate::parsers::data_lines(&clean);
    let found = candidates
        .iter()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let name = parts.next()?;
            let ver = parts.next()?;
            Some(Package::with_version(name, ver, backend))
        })
        .collect();
    or_unrecognised(backend, found, &candidates)
}

/// Parses the multi-line output of 'pacman -Ss' for remote searching.
/// Pacman search output typically has the Name/Version on one line and the Description on the next.
pub fn parse_search(output: &str) -> Vec<Package> {
    parse_search_for(output, "pacman")
}

/// Like [`parse_search`], but stamps a caller-supplied backend name (see [`parse_list_for`]).
pub fn parse_search_for(output: &str, backend: &str) -> Vec<Package> {
    let clean = sanitize(output);
    let mut packages = Vec::new();
    let mut lines = clean.lines().peekable();

    while let Some(line) = lines.next() {
        // An indented line is a description continuation, not a package.
        if line.starts_with(' ') || line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        // Format is usually: core/bash 5.1.016-1 (base) [installed]
        if let Some(repo_name) = parts.first() {
            let name = repo_name.split('/').next_back().unwrap_or(repo_name);
            let mut p = Package::new(name, backend);

            if let Some(version) = parts.get(1) {
                p.version = Some(version.to_string());
            }

            if let Some(desc_line) = lines.peek() {
                if desc_line.starts_with("    ") {
                    p.properties
                        .insert("description".to_string(), desc_line.trim().to_string());
                    lines.next(); // Consume the description line
                }
            }
            packages.push(p);
        }
    }
    packages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pacman_list_parsing() {
        let input = "bash 5.1.016-1\nlinux 6.3.5.arch1-1\n";
        let res = parse_list(input).expect("this fixture parses");
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "bash");
        assert_eq!(res[1].version, Some("6.3.5.arch1-1".into()));
    }

    #[test]
    fn test_pacman_search_parsing() {
        let input = "core/bash 5.1.016-1 (base)\n    The GNU Bourne Again Shell\nextra/vim 9.0.1583-1\n    Vi Improved, a highly configurable, improved version of the Vi real-time editor\n";
        let res = parse_search(input);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "bash");
        assert_eq!(
            res[0].properties.get("description").unwrap(),
            "The GNU Bourne Again Shell"
        );
        assert_eq!(res[1].name, "vim");
    }
}

/// The `Depends On` row of a `pacman -Si` report, as package names.
///
/// **Matched by key, not by column.** This stripped the literal `"Depends On     :"` — five
/// spaces — and Arch prints six, so the prefix never matched and `pacman:` answered every
/// dependency query with an empty list for the whole life of the backend. Nothing caught it,
/// because the only consumer was a planner that installed whatever came back and was better off
/// with nothing (`Y9`); now that the answer is only ever *shown*, an empty one is a lie a user
/// reads. Captured from `pacman -Si jq` in the Arch integration image, 2026-08-06:
///
/// ```text
/// Depends On      : glibc  oniguruma
/// ```
///
/// `Optional Deps` and `Conflicts With` are rows of the same shape, which is why the key is
/// compared whole rather than by prefix. And the row carries SEVERAL names, which is why the
/// labelled parser in `generic.rs` — written for apt's one-name-per-line shape — cannot read it.
pub fn parse_depends_on(output: &str) -> Vec<String> {
    let mut deps = Vec::new();
    for line in crate::utils::text::sanitize(output).lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.trim() != "Depends On" {
            continue;
        }
        for part in value.split_whitespace() {
            // pacman writes `None` where there are no dependencies at all.
            if part == "None" {
                continue;
            }
            // `glibc>=2.36` and `sh=5` are one dependency each; the constraint is not part of
            // the name a later command would have to use.
            let name = part.split(['>', '<', '=']).next().unwrap_or(part);
            if !name.is_empty() {
                deps.push(name.to_string());
            }
        }
    }
    deps
}

#[cfg(test)]
mod depends_tests {
    use super::parse_depends_on;

    /// Verbatim from `pacman -Si` in the Arch integration image, 2026-08-06 — the whole point
    /// being the column width, which the old fixed-width `strip_prefix` was one space short of.
    const SI_JQ: &str = "\
Repository      : extra
Name            : jq
Version         : 1.8.1-1
Depends On      : glibc  oniguruma
Optional Deps   : None
Conflicts With  : None
Description     : Command-line JSON processor
";

    #[test]
    fn a_depends_row_is_matched_by_its_key_and_not_by_its_column() {
        assert_eq!(parse_depends_on(SI_JQ), vec!["glibc", "oniguruma"]);
    }

    #[test]
    fn a_row_of_the_same_shape_that_is_not_a_dependency_is_left_alone() {
        // `Optional Deps` and `Conflicts With` sit in the same column and are not dependencies;
        // matching by prefix on `Depends` would have taken `Optional Deps` too once the width
        // was right.
        let deps = parse_depends_on(SI_JQ);
        assert!(!deps.iter().any(|d| d == "None"), "{:?}", deps);
        assert!(!deps.iter().any(|d| d == "Command-line"), "{:?}", deps);
    }

    #[test]
    fn a_version_constraint_is_not_part_of_the_name() {
        let out = "Depends On      : glibc>=2.36  sh=5  libc.so\n";
        assert_eq!(parse_depends_on(out), vec!["glibc", "sh", "libc.so"]);
    }

    #[test]
    fn a_package_with_no_dependencies_reports_none_of_them() {
        assert!(parse_depends_on("Depends On      : None\n").is_empty());
        assert!(parse_depends_on("Name            : tree\n").is_empty());
    }
}

/// `pacman -Qu` (`Q44`): `name installed -> available`, one per line.
pub fn parse_pacman_outdated(output: &str) -> Vec<Package> {
    crate::utils::text::sanitize(output)
        .lines()
        .filter_map(|line| {
            let (head, available) = line.split_once("->")?;
            let name = head.split_whitespace().next()?;
            let available = available.trim();
            if name.is_empty() || available.is_empty() {
                return None;
            }
            Some(Package::with_version(name, available, "pacman"))
        })
        .collect()
}

#[cfg(test)]
mod outdated_tests {
    use super::*;

    /// Verbatim from `pacman -Qu` in a `linix-it-arch` container.
    const PACMAN: &str = "\
archlinux-keyring 20260707.1-1 -> 20260727-1
audit 4.1.4-2 -> 4.2.1-1
gcc 16.1.1+r581+gb73ad535acaa-1 -> 16.1.1+r595+g171d15ac6959-1
";

    #[test]
    fn pacman_reads_the_right_side_of_the_arrow() {
        let p = parse_pacman_outdated(PACMAN);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].name, "archlinux-keyring");
        assert_eq!(p[0].version.as_deref(), Some("20260727-1"));
        // A version containing `+r581+g...` must survive intact — it is arch's own shape for
        // a VCS build and splitting it differently loses the identity.
        assert_eq!(p[2].version.as_deref(), Some("16.1.1+r595+g171d15ac6959-1"));
    }

    #[test]
    fn nothing_upgradable_is_nothing() {
        assert!(parse_pacman_outdated("").is_empty());
        // `pacman -Qu` prints nothing at all when everything is current.
        assert!(parse_pacman_outdated("\n\n").is_empty());
    }
}
