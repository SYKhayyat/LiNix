use crate::core::Package;
use crate::parsers::utils::sanitize;

/// Parses output from 'pacman -Q' for installed packages.
/// Expected input format: "name version"
pub fn parse_list(output: &str) -> Vec<Package> {
    parse_list_for(output, "pacman")
}

/// Like [`parse_list`], but stamps a caller-supplied backend name. AUR helpers
/// (`yay`, `paru`) speak pacman's `-Q` syntax verbatim but must label their packages
/// with their own backend so state tracking stays per-backend correct.
pub fn parse_list_for(output: &str, backend: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let name = parts.next()?;
            let ver = parts.next()?;
            Some(Package::with_version(name, ver, backend))
        })
        .collect()
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
                        .insert("description".into(), desc_line.trim().to_string());
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
        let res = parse_list(input);
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
