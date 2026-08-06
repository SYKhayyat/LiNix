//! Parser for `dotnet tool` global tables.
//!
//! Both `dotnet tool list --global` and `dotnet tool search <q>` print a two-line
//! header (a labelled row + a dashed separator) followed by rows whose first two
//! whitespace-separated columns are the package id and version. NuGet package ids
//! never contain spaces, so whitespace splitting is safe here.

use crate::core::Package;
use crate::utils::text::sanitize;

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

/// `dotnet tool list --global --format json` (SDK 10+, `Q43`).
///
/// ```json
/// {"version":1,"data":[{"packageId":"dotnetsay","version":"3.0.3","commands":["dotnetsay"]}]}
/// ```
///
/// The table form above is read by splitting on whitespace, which is safe only because NuGet
/// ids never contain spaces — a property of NuGet, not of the format. This reads the id.
pub fn parse_dotnet_list_json(output: &str) -> Vec<Package> {
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(output) else {
        return Vec::new();
    };
    let Some(items) = doc.get("data").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|t| {
            let id = t.get("packageId")?.as_str()?.trim();
            if id.is_empty() {
                return None;
            }
            let version = t
                .get("version")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty());
            Some(match version {
                Some(v) => Package::with_version(id, v, "dotnet"),
                None => Package::new(id, "dotnet"),
            })
        })
        .collect()
}

#[cfg(test)]
mod json_tests {
    use super::*;

    /// Verbatim from `dotnet tool list --global --format json` on SDK 10.0.301.
    const REAL: &str = r#"{"version":1,"data":[{"packageId":"dotnetsay","version":"3.0.3","commands":["dotnetsay"]}]}"#;

    #[test]
    fn a_tool_is_its_package_id_and_version() {
        let pkgs = parse_dotnet_list_json(REAL);
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "dotnetsay");
        assert_eq!(pkgs[0].version.as_deref(), Some("3.0.3"));
        assert_eq!(pkgs[0].backend, "dotnet");
    }

    /// The two forms describe the same machine, so they must report the same thing. A
    /// difference here is a listing that changes shape with the installed SDK.
    #[test]
    fn the_json_and_the_table_agree_about_the_same_machine() {
        let table = "Package Id      Version      Commands \n\
                     --------------------------------------\n\
                     dotnetsay       3.0.3        dotnetsay\n";
        let a = parse_dotnet_list(table);
        let b = parse_dotnet_list_json(REAL);
        assert_eq!(
            a.iter().map(|p| (&p.name, &p.version)).collect::<Vec<_>>(),
            b.iter().map(|p| (&p.name, &p.version)).collect::<Vec<_>>(),
        );
    }

    /// An SDK too old for `--format json` fails rather than printing the table, so this never
    /// sees one — but if the negotiation ever regressed, reading a table as JSON must report
    /// nothing rather than inventing a package from a header row.
    #[test]
    fn a_table_fed_to_the_json_reader_yields_nothing() {
        assert!(parse_dotnet_list_json("Package Id  Version\n----\nx  1.0\n").is_empty());
        assert!(parse_dotnet_list_json("").is_empty());
        assert!(parse_dotnet_list_json(r#"{"version":1,"data":[]}"#).is_empty());
        assert!(parse_dotnet_list_json(r#"{"version":1}"#).is_empty());
    }
}
