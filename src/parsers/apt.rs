use crate::core::Package;
use crate::parsers::OutputParser;
use crate::utils::text::sanitize;

/// apt's parser. A dedicated struct rather than a `LambdaParser` because apt is the one
/// backend that can also report which packages dpkg itself refuses to lose.
pub struct AptParser;

impl OutputParser for AptParser {
    fn parse_installed(&self, output: &str) -> Vec<Package> {
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

/// Parses output from the debian/ubuntu package database query.
/// Command: dpkg-query -W -f='${Package} ${Version}\n'
/// Expected input format: "curl 7.81.0-1ubuntu1.16"
pub fn parse_list(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let (name, ver) = line.split_once(' ')?;
            Some(Package::with_version(name.trim(), ver.trim(), "apt"))
        })
        .collect()
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
                .insert("description".into(), desc.trim().to_string());
            Some(p)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apt_list_parsing() {
        let input = "bash 5.1-6ubuntu1\ncoreutils 8.32-4.1ubuntu1\n";
        let res = parse_list(input);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "bash");
        assert_eq!(res[0].version, Some("5.1-6ubuntu1".into()));
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
