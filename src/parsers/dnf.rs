use crate::core::Package;
use crate::parsers::{or_unrecognised, ParseResult};
use crate::utils::text::sanitize;

/// Standard RPM query parser used by DNF and Zypper.
/// Command: rpm -qa --queryformat '%{NAME}|%{VERSION}\n'
/// Expected input format: "package-name|1.2.3-r1"
pub fn parse_rpm_qa(output: &str, backend: &str) -> ParseResult {
    let clean = sanitize(output);
    let candidates = crate::parsers::data_lines(&clean);
    let found = candidates
        .iter()
        .filter_map(|l| {
            let (name, ver) = l.split_once('|')?;
            Some(Package::with_version(name.trim(), ver.trim(), backend))
        })
        .collect();
    or_unrecognised(backend, found, &candidates)
}

/// The architectures dnf appends to a name. Only these are stripped: taking everything
/// before the first `.` turned `python3.12.x86_64` into `python3`, and a resolver comparing
/// the parsed name against what you asked for then never matched.
const RPM_ARCHES: &[&str] = &[
    "x86_64", "i686", "i386", "noarch", "aarch64", "armv7hl", "ppc64le", "s390x", "src",
];

/// Parses the output of `dnf search`.
///
/// Two shapes, because dnf5 (Fedora 41+) rewrote the output and dnf4 is still what RHEL and
/// older Fedora run: `name.arch : summary` and `name.arch<TAB>summary`. Both are the same
/// three facts with a different separator, so one pass reads either — and a header line, which
/// has neither separator, is dropped by the same rule rather than by a list of known headers.
pub fn parse_dnf_search(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let (name_part, desc) = line.split_once('\t').or_else(|| line.split_once(" : "))?;
            let name = strip_rpm_arch(name_part.trim());
            if name.is_empty() {
                return None;
            }
            let mut p = Package::new(name, "dnf");
            p.properties
                .insert("description".to_string(), desc.trim().to_string());
            Some(p)
        })
        .collect()
}

fn strip_rpm_arch(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((base, arch)) if RPM_ARCHES.contains(&arch) => base,
        _ => name,
    }
}

/// Parses the table-based output of 'zypper search'.
/// Zypper output includes status indicators like 'i+' for installed.
///
/// **This is the parser whose failure mode the whole `Unrecognised` type exists for**, and its
/// comment below has said so since the `skip_while` bug: it is zypper's *installed* lister as
/// well as its search, so an output it cannot read is a mass-removal input. It can now say it
/// could not read one.
pub fn parse_zypper_search(output: &str) -> ParseResult {
    let clean = sanitize(output);
    let candidates: Vec<&str> = clean
        .lines()
        // The rule under the header is skipped where it appears, rather than used as the
        // gate that starts reading. `skip_while(|l| !l.contains("---"))` consumed the WHOLE
        // output when zypper printed no rule — and this parser is wired as zypper's
        // installed lister as well as its search, so "no rule" meant "nothing is installed",
        // which is not a bad search result but a mass-removal input.
        .filter(|l| {
            !l.trim_start().starts_with("---")
                && !l.trim().is_empty()
                && !crate::parsers::is_prose_line(l)
        })
        .collect();
    let found = candidates
        .iter()
        .filter_map(|line| {
            // Table format: S | Name | Summary | Type
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() >= 3 {
                let status = parts[0].trim();
                let name = parts[1].trim();
                let summary = parts[2].trim();
                // The header row names its own columns; without the rule to skip past, it
                // would otherwise become a package called `Name`.
                if name.is_empty() || name == "Name" {
                    return None;
                }

                let mut p = Package::new(name, "zypper");
                p.properties
                    .insert("summary".to_string(), summary.to_string());
                p.properties
                    .insert("status_raw".to_string(), status.to_string());

                // If status contains 'i', it's already installed
                if status.contains('i') {
                    p.properties.insert("installed".to_string(), "true".into());
                }

                Some(p)
            } else {
                None
            }
        })
        .collect();
    or_unrecognised("zypper", found, &candidates)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rpm_qa_parsing() {
        let input = "kernel|6.3.5\ngit|2.40.1\n";
        let res = parse_rpm_qa(input, "dnf").expect("this fixture parses");
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "kernel");
        assert_eq!(res[1].version, Some("2.40.1".into()));
    }

    #[test]
    fn test_dnf_search_parsing() {
        let input = "htop.x86_64 : Interactive process viewer\npython3.noarch : Python programming language\n";
        let res = parse_dnf_search(input);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "htop");
        assert_eq!(
            res[0].properties.get("description").unwrap(),
            "Interactive process viewer"
        );
    }

    #[test]
    fn dnf5_tab_separated_output_is_read() {
        // Fedora 41 ships dnf5, which prints a tab and indents each row, and interleaves
        // "Matched fields:" headers. Reading only dnf4's ` : ` made dnf return nothing at
        // all on Fedora — so every unpinned name skipped the system manager and landed on
        // whichever language registry happened to publish the name.
        let input = "Updating and loading repositories:\nRepositories loaded.\n\
                     Matched fields: name (exact)\n \
                     jq.x86_64\tCommand-line JSON processor\n \
                     R-jqr.x86_64\tClient for 'jq', a 'JSON' Processor\n";
        let res = parse_dnf_search(input);
        assert_eq!(res.len(), 2, "headers must not become packages");
        assert_eq!(res[0].name, "jq");
        assert_eq!(res[1].name, "R-jqr");
        assert_eq!(
            res[0].properties.get("description").unwrap(),
            "Command-line JSON processor"
        );
    }

    #[test]
    fn zypper_rows_are_read_with_or_without_the_header_rule() {
        // This parser is zypper's installed lister as well as its search, so returning
        // nothing is not a bad search result — it is "nothing is installed", which is a
        // mass-removal input. Waiting for a `---` rule before reading anything meant one
        // missing rule produced exactly that.
        let with_rule = "S | Name | Summary | Type\n\
                         --+------+---------+-----\n\
                         i | jq   | JSON    | package\n\
                           | htop | Viewer  | package\n";
        let without_rule = "S | Name | Summary | Type\n\
                            i | jq   | JSON    | package\n\
                              | htop | Viewer  | package\n";
        for (label, input) in [("with", with_rule), ("without", without_rule)] {
            let res = parse_zypper_search(input).expect("this fixture parses");
            assert_eq!(res.len(), 2, "{} the rule", label);
            assert_eq!(res[0].name, "jq");
            assert_eq!(
                res[0].properties.get("installed").map(String::as_str),
                Some("true")
            );
            assert_eq!(res[1].name, "htop");
            assert_eq!(res[1].properties.get("installed"), None);
        }
    }

    #[test]
    fn only_a_real_architecture_is_stripped_from_a_name() {
        // `python3.12.x86_64` is python3.12, not python3: cutting at the first dot renamed
        // the package, and a resolver matching on the name then never found it.
        let res = parse_dnf_search("python3.12.x86_64\tPython\nfoo.bar\tNot an arch\n");
        assert_eq!(res[0].name, "python3.12");
        assert_eq!(res[1].name, "foo.bar");
    }
}

/// `dnf check-update -q` (`Q44`).
///
/// ```text
/// Upgrades
/// audit-libs.x86_64                  4.2.1-1.fc44     updates
/// coreutils.x86_64                   9.10-5.fc44      updates
/// ```
///
/// **The arch is stripped, because the installed listing strips it** — a row reported as
/// `audit-libs.x86_64` matches nothing the caller holds, so it would silently report zero
/// updates on a machine that has them.
///
/// `dnf check-update` exits **100** when it finds updates. That is an answer, not a fault, and
/// it is why this is read through a reader that tolerates a non-zero exit that still spoke.
pub fn parse_dnf_outdated(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let name_arch = f.next()?;
            let version = f.next()?;
            // A section banner ("Upgrades", "Obsoleting Packages") has no second field.
            let _repo = f.next()?;
            let name = strip_rpm_arch(name_arch);
            // dnf wraps a long name onto its own line; the continuation has no dot-arch and
            // reads as a package with the version of whatever followed it.
            if name.is_empty() || !name_arch.contains('.') {
                return None;
            }
            Some(Package::with_version(name, version, "dnf"))
        })
        .collect()
}

/// `zypper --non-interactive list-updates` (`Q44`).
///
/// ```text
/// S  | Repository | Name         | Current Version | Available Version | Arch
/// ---+------------+--------------+-----------------+-------------------+-------
/// v  | repo-oss   | curl         | 8.17.0-1.1      | 8.18.0-1.1        | x86_64
/// ```
///
/// Six pipe-separated columns, and the one wanted is *Available*, not *Current*. Both are
/// versions and they sit next to each other, which is exactly the shape that gets read off by
/// one column and reports every package as already up to date.
pub fn parse_zypper_outdated(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter(|l| !l.trim_start().starts_with("---"))
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() < 5 {
                return None;
            }
            let name = parts[2].trim();
            let available = parts[4].trim();
            // The header names its own columns and would otherwise become a package.
            if name.is_empty() || name == "Name" || available.is_empty() {
                return None;
            }
            Some(Package::with_version(name, available, "zypper"))
        })
        .collect()
}

#[cfg(test)]
mod outdated_tests {
    use super::*;

    /// Verbatim from `dnf check-update -q` in a `fedora:latest` container.
    const DNF: &str = "\
Upgrades
audit-libs.x86_64                  4.2.1-1.fc44     updates
coreutils.x86_64                   9.10-5.fc44      updates
curl.x86_64                        8.18.0-8.fc44    updates
";

    #[test]
    fn dnf_strips_the_arch_so_the_name_matches_the_listing() {
        let p = parse_dnf_outdated(DNF);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].name, "audit-libs");
        assert_eq!(p[0].version.as_deref(), Some("4.2.1-1.fc44"));
        assert!(
            !p.iter().any(|x| x.name.contains(".x86_64")),
            "an arch-qualified name matches nothing the caller holds: {:?}",
            p
        );
    }

    /// `Upgrades` is a section banner, not a package.
    #[test]
    fn a_dnf_section_banner_is_not_a_package() {
        assert!(!parse_dnf_outdated(DNF).iter().any(|p| p.name == "Upgrades"));
    }

    /// Header shape verbatim from `zypper list-updates` in a `linix-it-opensuse` container.
    const ZYPPER: &str = "\
S  | Repository                 | Name               | Current Version | Available Version | Arch
---+----------------------------+--------------------+-----------------+-------------------+-------
v  | repo-oss                   | curl               | 8.17.0-1.1      | 8.18.0-1.1        | x86_64
v  | repo-oss                   | glibc              | 2.42-1.1        | 2.43-1.1          | x86_64
";

    #[test]
    fn zypper_reads_available_and_not_current() {
        let p = parse_zypper_outdated(ZYPPER);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].name, "curl");
        assert_eq!(
            p[0].version.as_deref(),
            Some("8.18.0-1.1"),
            "Current and Available sit next to each other; reading the wrong one reports \
             every package as already up to date"
        );
        assert_eq!(p[1].version.as_deref(), Some("2.43-1.1"));
    }

    #[test]
    fn a_zypper_header_row_is_not_a_package() {
        assert!(!parse_zypper_outdated(ZYPPER)
            .iter()
            .any(|p| p.name == "Name"));
    }

    #[test]
    fn nothing_outdated_is_nothing() {
        assert!(parse_dnf_outdated("").is_empty());
        assert!(parse_zypper_outdated("").is_empty());
        // zypper prints its header and nothing beneath when everything is current.
        assert!(parse_zypper_outdated(
            "S  | Repository | Name | Current Version | Available Version | Arch\n"
        )
        .is_empty());
    }
}

/// `dnf repoquery --requires --resolve --queryformat %{name} <pkg>` — one bare package name per
/// line, and nothing else.
///
/// **A different shape from a labelled report, and so a different function.** `generic.rs`'s
/// labelled parser reads `Depends:`/`Requires:` rows and would answer this output with an empty
/// list, because there is no label on any line. One parser made lenient enough for both is one
/// parser that reads a malformed answer in either shape as a valid answer in the other — the
/// same rule `MachineListing` states for its own reader.
pub fn parse_bare_dependency_names(output: &str) -> Vec<String> {
    crate::utils::text::sanitize(output)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod bare_dependency_tests {
    use super::parse_bare_dependency_names;

    #[test]
    fn one_name_per_line_and_blank_lines_are_not_packages() {
        assert_eq!(
            parse_bare_dependency_names("glibc\n\noniguruma\n  \n"),
            vec!["glibc", "oniguruma"]
        );
    }

    #[test]
    fn a_package_with_nothing_to_report_reports_nothing() {
        assert!(parse_bare_dependency_names("").is_empty());
    }
}
