use crate::core::Package;
use crate::parsers::utils::sanitize;

/// Parses output from 'brew list --versions' for installed packages.
/// Expected input format: "package-name 1.2.3"
pub fn parse_list(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            // Homebrew versions output is space-separated: "name version"
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                Some(Package::with_version(parts[0], parts[1], "brew"))
            } else {
                None
            }
        })
        .collect()
}

/// Parses the output of 'brew search' for remote package discovery.
/// This parser filters out Homebrew's visual headers (e.g. "==> Formulae").
pub fn parse_search(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with("==>"))
        .map(|l| {
            let name = l.trim();
            Package::new(name, "brew")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_brew_list_parsing() {
        let input = "openssl@3 3.1.1\nripgrep 13.0.0\npython@3.11 3.11.3\n";
        let res = parse_list(input);
        assert_eq!(res.len(), 3);
        assert_eq!(res[0].name, "openssl@3");
        assert_eq!(res[0].version, Some("3.1.1".into()));
        assert_eq!(res[1].name, "ripgrep");
        assert_eq!(res[2].version, Some("3.11.3".into()));
    }

    #[test]
    fn test_brew_search_parsing() {
        let input = "==> Formulae\nhtop\nbtop\n\n==> Casks\nvisual-studio-code\n";
        let res = parse_search(input);
        assert_eq!(res.len(), 3);
        assert_eq!(res[0].name, "htop");
        assert_eq!(res[1].name, "btop");
        assert_eq!(res[2].name, "visual-studio-code");
        assert_eq!(res[2].backend, "brew");
    }
}