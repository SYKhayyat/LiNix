use crate::core::Package;
use crate::parsers::{or_unrecognised, OutputParser, ParseResult};
use crate::utils::text::sanitize;

/// apt's parser. A dedicated struct rather than a `LambdaParser` because apt is the one
/// backend that can also report which packages dpkg itself refuses to lose.
pub struct AptParser;

impl OutputParser for AptParser {
    fn parse_installed(&self, output: &str) -> ParseResult {
        parse_list(output)
    }
    fn parse_search(&self, output: &str) -> Vec<Package> {
        parse_search(output)
    }
    fn parse_essential(&self, output: &str) -> Vec<String> {
        parse_essential(output)
    }
}

/// Parses `dpkg-query -W -f='${Essential} ${Priority} ${Package}\n'`, keeping only the
/// packages Debian marks as undeletable.
///
/// `Essential: yes` is enforced by dpkg itself ("the package management system will refuse
/// to remove the package"), and `Priority: required` means "necessary for the proper
/// functioning of the system". Both are read live from the running system, so this stays
/// correct across distro releases without a hand-maintained name list.
///
/// Expected input format: "yes required base-files" / "no optional python3".
///
/// The name is read from the END and the flags from the front, deliberately: `Priority` is
/// an optional dpkg field, and a package that omits it yields two tokens rather than
/// three. Counting positions from the front would then read the *name* as the priority and
/// drop the package — silently un-protecting something marked `Essential: yes`. This query
/// exists to keep systems alive, so it must fail closed.
pub fn parse_essential(output: &str) -> Vec<String> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            let name = parts.last()?;
            let flags = &parts[..parts.len() - 1];
            (flags.contains(&"yes") || flags.contains(&"required")).then(|| name.to_string())
        })
        .collect()
}

/// Every status word dpkg can put on a package, and whether it means the software is on the
/// machine.
///
/// The list is exhaustive on purpose: it is what tells a real row from a line of apt's error
/// output, and a status dpkg grows tomorrow should be an unreadable line rather than a silent
/// "absent".
const DPKG_STATUS_WORDS: [(&str, bool); 8] = [
    ("installed", true),
    // Installed, with a trigger deferred — not a partial install. Reading these as absent
    // would report a working package as missing and make `sync` reinstall it, which is B0's
    // mistake pointed the other way.
    ("triggers-awaited", true),
    ("triggers-pending", true),
    // The software is not usable. `config-files` is what `apt remove` leaves behind.
    ("not-installed", false),
    ("config-files", false),
    ("half-installed", false),
    ("unpacked", false),
    ("half-configured", false),
];

/// One row of `dpkg-query -W -f='${db:Status-Status} ${Package} ${Version}\n'`, as
/// `(is_installed, name, version)`. `None` is a line this format cannot have produced.
///
/// A removed package's `${Version}` is empty, so the version is optional and its absence is
/// not a parse failure.
fn read_row(line: &str) -> Option<(bool, &str, &str)> {
    let mut parts = line.split_whitespace();
    let status = parts.next()?;
    let installed = DPKG_STATUS_WORDS
        .iter()
        .find_map(|(word, present)| (*word == status).then_some(*present))?;
    let name = parts.next()?;
    Some((installed, name, parts.next().unwrap_or("")))
}

/// Parses output from the debian/ubuntu package database query.
/// Command: dpkg-query -W -f='${db:Status-Status} ${Package} ${Version}\n'
/// Expected input format: "installed curl 7.81.0-1ubuntu1.16"
///
/// **The status field is what this argv exists for.** `dpkg-query -W` alone lists every package
/// dpkg knows about, and dpkg keeps knowing about one after `apt remove`: it enters
/// `deinstall ok config-files`, meaning the software is gone and its configuration was kept.
/// Shall removes with `remove` and not `purge`, which is the correct and safe choice — so every
/// conffile-carrying package Shall removed was minted into a state the old listing reported as
/// installed. `list` named a package that was not there, `check` saw no drift, and `sync`
/// refused to put it back, permanently (B0). The lister was the bug, not the remover.
///
/// **It also settles what a line that is not a package looks like.** apt's own error output
/// (`E: Could not open lock file`) used to read as a package named `E:` at version *"Could not
/// open lock file"*, because any line with a space in it was a package. A row now has to open
/// with a status word dpkg can actually emit, so junk is unreadable rather than believed.
pub fn parse_list(output: &str) -> ParseResult {
    let clean = sanitize(output);
    let candidates = crate::parsers::data_lines(&clean);
    let found = candidates
        .iter()
        .filter_map(|line| {
            let (installed, name, ver) = read_row(line)?;
            installed.then(|| match ver.is_empty() {
                // An installed package always carries a version, but `Some("")` compares as a
                // version and would make every plan see a change it cannot explain.
                true => Package::new(name, "apt"),
                false => Package::with_version(name, ver, "apt"),
            })
        })
        .collect();
    // Only the lines this format could not have produced count towards "the parser did not
    // understand this". A row read correctly and dropped for saying `config-files` is an
    // answer, and a machine whose every known package has been removed is a real machine —
    // reporting that as unrecognised would refuse to run on it.
    let unreadable: Vec<&str> = candidates
        .iter()
        .copied()
        .filter(|line| read_row(line).is_none())
        .collect();
    or_unrecognised("apt", found, &unreadable)
}

/// Parses the output of 'apt-cache search' for remote package discovery.
/// Expected input format: "package-name - human readable description"
pub fn parse_search(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            // Apt-cache search usually uses " - " as a separator between name and description
            let (name, desc) = line.split_once(" - ")?;
            let mut p = Package::new(name.trim(), "apt");
            p.properties
                .insert("description".to_string(), desc.trim().to_string());
            Some(p)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apt_list_parsing() {
        let input = "installed bash 5.1-6ubuntu1\ninstalled coreutils 8.32-4.1ubuntu1\n";
        let res = parse_list(input).expect("this fixture parses");
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "bash");
        assert_eq!(res[0].version, Some("5.1-6ubuntu1".into()));
    }

    /// A machine dpkg knows only removed packages on is a machine with nothing installed — not
    /// a listing the parser failed to read. The two are different answers and reading the first
    /// as the second would refuse to run on it.
    #[test]
    fn every_row_removed_is_an_empty_machine_and_not_an_unread_one() {
        let res = parse_list("config-files figlet 2.2.5-3\nnot-installed sl \n")
            .expect("rows that were read and dropped are an answer");
        assert!(res.is_empty());
    }

    /// The bug in one assertion: the row `apt remove` leaves behind is not an installed package.
    #[test]
    fn a_package_apt_remove_left_behind_is_not_installed() {
        let res = parse_list("installed bash 5.1-6ubuntu1\nconfig-files figlet 2.2.5-3\n")
            .expect("this fixture parses");
        assert_eq!(res.len(), 1, "{res:?}");
        assert_eq!(res[0].name, "bash");
    }

    #[test]
    fn essential_keeps_only_undeletable_packages() {
        // Real shape of `dpkg-query -W -f='${Essential} ${Priority} ${Package}\n'`,
        // sampled from the ubuntu test image.
        let input = "no required apt\nyes required base-files\nyes required bash\n\
                     no optional binutils\nno optional python3\n";
        let res = parse_essential(input);
        assert_eq!(res, vec!["apt", "base-files", "bash"]);
        // python3 is `no optional` on a stock Ubuntu — dpkg will NOT protect it, which is
        // why the static protected list has to carry it.
        assert!(!res.contains(&"python3".to_string()));
    }

    #[test]
    fn essential_survives_a_missing_priority_field() {
        // `Priority` is optional in dpkg, so an unset field collapses the line to two
        // tokens. Counting from the front would read "base-files" as the priority and drop
        // the package — silently un-protecting something marked Essential: yes.
        let res = parse_essential("yes  base-files\n required login\nno optional jq\n");
        assert!(res.contains(&"base-files".to_string()), "{:?}", res);
        assert!(res.contains(&"login".to_string()), "{:?}", res);
        assert!(!res.contains(&"jq".to_string()));
    }

    #[test]
    fn a_package_named_like_a_flag_is_not_confused_for_one() {
        // The name is excluded from the flags before matching, so a package called "yes"
        // is only protected if its own flags say so.
        assert!(parse_essential("no optional yes\n").is_empty());
        assert_eq!(parse_essential("yes required yes\n"), vec!["yes"]);
    }

    #[test]
    fn test_apt_search_parsing() {
        let input = "htop - interactive processes viewer\nvim - Vi IMproved - enhanced vi editor\n";
        let res = parse_search(input);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "htop");
        assert_eq!(
            res[0].properties.get("description").unwrap(),
            "interactive processes viewer"
        );
    }
}

/// `apt list --upgradable` (`Q44`).
///
/// ```text
/// Listing...
/// gzip/noble-updates,noble-security 1.12-1ubuntu3.2 amd64 [upgradable from: 1.12-1ubuntu3.1]
/// ```
///
/// The name is what precedes the `/`; everything after it is the suite list, not part of the
/// name. The version taken is the *first* one — the second, inside the brackets, is what is
/// installed, and reading that one reports every package as already current.
pub fn parse_apt_outdated(output: &str) -> Vec<Package> {
    crate::utils::text::sanitize(output)
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            // apt's own progress banner, and its warning about an unstable CLI.
            if line.is_empty() || line.starts_with("Listing") || line.starts_with("WARNING") {
                return None;
            }
            let (name, rest) = line.split_once('/')?;
            let version = rest.split_whitespace().nth(1)?;
            if name.is_empty() || version.is_empty() {
                return None;
            }
            Some(Package::with_version(name.trim(), version, "apt"))
        })
        .collect()
}

#[cfg(test)]
mod outdated_tests {
    use super::*;

    /// Verbatim from `apt list --upgradable` in an `ubuntu:24.04` container.
    const APT: &str = "\
Listing...
gzip/noble-updates,noble-security 1.12-1ubuntu3.2 amd64 [upgradable from: 1.12-1ubuntu3.1]
libc-bin/noble-updates,noble-security 2.39-0ubuntu8.8 amd64 [upgradable from: 2.39-0ubuntu8.7]
libpam-runtime/noble-updates,noble-security 1.5.3-5ubuntu5.6 all [upgradable from: 1.5.3-5ubuntu5.5]
";

    #[test]
    fn apt_reports_the_available_version_not_the_installed_one() {
        let p = parse_apt_outdated(APT);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].name, "gzip");
        assert_eq!(
            p[0].version.as_deref(),
            Some("1.12-1ubuntu3.2"),
            "the bracketed version is what is INSTALLED; reading it reports everything as \
             already up to date"
        );
        assert_eq!(p[2].name, "libpam-runtime");
        assert_eq!(p[2].version.as_deref(), Some("1.5.3-5ubuntu5.6"));
    }

    /// The suite list is not part of the name, and `Listing...` is not a package.
    #[test]
    fn the_suite_and_the_banner_are_not_packages() {
        let p = parse_apt_outdated(APT);
        assert!(!p.iter().any(|x| x.name.contains('/')), "{:?}", p);
        assert!(!p.iter().any(|x| x.name.starts_with("Listing")), "{:?}", p);
    }

    #[test]
    fn nothing_upgradable_is_nothing() {
        assert!(parse_apt_outdated("").is_empty());
        assert!(parse_apt_outdated("Listing...\n").is_empty());
    }
}
