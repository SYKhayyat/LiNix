//! Parser for `dotnet tool` global tables.
//!
//! Both `dotnet tool list --global` and `dotnet tool search <q>` print a two-line
//! header (a labelled row + a dashed separator) followed by rows whose first two
//! whitespace-separated columns are the package id and version. NuGet package ids
//! never contain spaces, so whitespace splitting is safe here.

use crate::core::Package;
use crate::parsers::utils::sanitize;

fn parse_tool_table(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            // Skip the labelled header row and the dashed separator beneath it.
            if trimmed.starts_with("Package") {
                return None;
            }
            if trimmed.chars().all(|c| c == '-' || c == ' ') {
                return None;
            }
            let mut cols = trimmed.split_whitespace();
            let id = cols.next()?;
            let version = cols.next().unwrap_or("");
            Some(Package::with_version(id, version, "dotnet"))
        })
        .collect()
}

/// Parses `dotnet tool list --global`.
pub fn parse_dotnet_list(output: &str) -> Vec<Package> {
    parse_tool_table(output)
}

/// Parses `dotnet tool search <query>`.
pub fn parse_dotnet_search(output: &str) -> Vec<Package> {
    parse_tool_table(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dotnet_tool_list() {
        let input = "Package Id      Version      Commands\n\
                     --------------------------------------\n\
                     dotnetsay       2.1.4        dotnetsay\n\
                     powershell      7.4.0        pwsh\n";
        let res = parse_dotnet_list(input);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "dotnetsay");
        assert_eq!(res[0].version.as_deref(), Some("2.1.4"));
        assert_eq!(res[1].name, "powershell");
        // header/separator rows must not leak through
        assert!(res
            .iter()
            .all(|p| p.name != "Package" && !p.name.starts_with('-')));
    }

    #[test]
    fn parses_dotnet_tool_search() {
        let input = "Package ID        Latest Version      Authors      Downloads\n\
                     ----------------  ------------------  -----------  ---------\n\
                     dotnet-ef         8.0.0               Microsoft    123456\n";
        let res = parse_dotnet_search(input);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].name, "dotnet-ef");
        assert_eq!(res[0].version.as_deref(), Some("8.0.0"));
    }
}
