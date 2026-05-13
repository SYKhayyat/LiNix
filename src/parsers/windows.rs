use crate::core::Package;
use crate::parsers::utils::sanitize;

/// Unified dispatcher for Windows-specific installed package parsing.
/// Supports Winget, Chocolatey, and Scoop.
pub fn parse_installed(backend: &str, output: &str) -> Vec<Package> {
    match backend {
        "winget" => parse_winget_list(output),
        "choco" => parse_choco_list(output),
        "scoop" => parse_scoop_list(output),
        _ => vec![],
    }
}

/// Unified dispatcher for Windows-specific search result parsing.
pub fn parse_search(backend: &str, output: &str) -> Vec<Package> {
    match backend {
        "winget" => parse_winget_search(output),
        "choco" => parse_choco_search(output),
        "scoop" => parse_scoop_search(output),
        _ => vec![],
    }
}

/// Parses output from 'winget list'.
/// Expected input contains a table with Name, Id, Version headers.
fn parse_winget_list(output: &str) -> Vec<Package> {
    sanitize(output).lines()
        .skip(2) // Skip table headers and separator line
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            // Winget list table format: Name [0] Id [1] Version [2] Source [3]
            if parts.len() >= 3 {
                Some(Package::with_version(parts[1], parts[2], "winget"))
            } else { None }
        }).collect()
}

/// Parses output from 'choco list -lo -r' (local only, readable/piped).
/// Expected input format: "name|version"
fn parse_choco_list(output: &str) -> Vec<Package> {
    sanitize(output).lines()
        .filter_map(|line| {
            let (name, ver) = line.split_once('|')?;
            Some(Package::with_version(name.trim(), ver.trim(), "choco"))
        }).collect()
}

/// Parses output from 'scoop list'.
/// Expected input contains a list of installed apps.
fn parse_scoop_list(output: &str) -> Vec<Package> {
    sanitize(output).lines()
        .filter(|l| !l.is_empty() && !l.contains("---") && !l.contains("Installed apps"))
        .filter_map(|l| {
            let parts: Vec<&str> = l.split_whitespace().collect();
            // Scoop list format: Name [0] Version [1] Source [2] Updated [3]
            if parts.len() >= 2 {
                Some(Package::with_version(parts[0], parts[1], "scoop"))
            } else { None }
        }).collect()
}

/// Parses 'winget search' output table.
fn parse_winget_search(output: &str) -> Vec<Package> {
    sanitize(output).lines()
        .skip(2)
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            // Search table: Name [0] Id [1] Version [2] Match [3] Source [4]
            if parts.len() >= 2 {
                Some(Package::new(parts[1], "winget"))
            } else { None }
        }).collect()
}

/// Parses 'choco search' results.
fn parse_choco_search(output: &str) -> Vec<Package> {
    sanitize(output).lines()
        .filter_map(|line| {
            // Choco search usually returns "name version" on each line
            let parts: Vec<&str> = line.split_whitespace().collect();
            let name = parts.get(0)?;
            let mut p = Package::new(name.trim(), "choco");
            if let Some(v) = parts.get(1) {
                p.version = Some(v.to_string());
            }
            Some(p)
        }).collect()
}

/// Parses 'scoop search' results.
fn parse_scoop_search(output: &str) -> Vec<Package> {
    sanitize(output).lines()
        .filter(|l| l.contains('(')) // Scoop search lines usually look like "name (version)"
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            Some(Package::new(parts.get(0)?, "scoop"))
        }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_winget_list_parsing() {
        let input = "Name Id Version\n---- -- -------\nPowerShell Microsoft.PowerShell 7.3.4.0\n";
        let res = parse_installed("winget", input);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].name, "Microsoft.PowerShell");
        assert_eq!(res[0].version, Some("7.3.4.0".into()));
    }

    #[test]
    fn test_choco_list_parsing() {
        let input = "git|2.40.1\ncurl|8.1.2\n";
        let res = parse_installed("choco", input);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "git");
        assert_eq!(res[1].version, Some("8.1.2".into()));
    }
}