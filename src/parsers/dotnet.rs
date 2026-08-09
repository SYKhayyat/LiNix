//! Parser for `dotnet tool` global tables.
//!
//! Both `dotnet tool list --global` and `dotnet tool search <q>` print a two-line
//! header (a labelled row + a dashed separator) followed by rows whose first two
//! whitespace-separated columns are the package id and version. NuGet package ids
//! never contain spaces, so whitespace splitting is safe here.

use crate::core::Package;
use crate::parsers::{or_unrecognised, ParseResult};
use crate::utils::text::sanitize;

fn parse_tool_table(output: &str) -> ParseResult {
    let clean = sanitize(output);
    // The labelled header row and the dashed separator beneath it are this format's own
    // furniture: understood, skipped, and not evidence that anything went unread.
    let candidates: Vec<&str> = clean
        .lines()
        .map(str::trim)
        .filter(|t| {
            !t.is_empty()
                && !t.starts_with("Package")
                && !t.chars().all(|c| c == '-' || c == ' ')
                && !crate::parsers::is_prose_line(t)
        })
        .collect();
    let found = candidates
        .iter()
        .filter_map(|trimmed| {
            let mut cols = trimmed.split_whitespace();
            let id = cols.next()?;
            let version = cols.next().unwrap_or("");
            Some(Package::with_version(id, version, "dotnet"))
        })
        .collect();
    or_unrecognised("dotnet", found, &candidates)
}

/// Parses `dotnet tool list --global`.
pub fn parse_dotnet_list(output: &str) -> ParseResult {
    parse_tool_table(output)
}

/// Parses `dotnet tool search <query>`.
pub fn parse_dotnet_search(output: &str) -> ParseResult {
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
        let res = parse_dotnet_list(input).expect("this fixture parses");
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
        let res = parse_dotnet_search(input).expect("this fixture parses");
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
///
/// **Both early returns used to be `Vec::new()`.** This is the negotiated `--format json` path
/// (`Q43`), reached by asking a tool that may be too old for the flag — so the usage message an
/// SDK 9 prints was being read as *"no global tools are installed"*, for exactly the users least
/// likely to notice. The `MachineListing` doc four hundred lines away in `generic.rs` describes
/// that failure precisely and calls it `Q40` reproduced; the parser it hands the bytes to went
/// on reproducing it.
pub fn parse_dotnet_list_json(output: &str) -> ParseResult {
    let Some(doc) = crate::parsers::json_document(output) else {
        return crate::parsers::or_unrecognised_json(
            "dotnet",
            vec![],
            None,
            "not JSON — most likely a usage message from an SDK without the flag",
            output,
        );
    };
    let items = doc.get("data").and_then(|d| d.as_array());
    let found = items
        .into_iter()
        .flatten()
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
        .collect::<Vec<_>>();
    // An empty `data` array is dotnet saying no global tools are installed, which is true on
    // most machines. A populated one none of whose entries carried a `packageId` is a schema
    // change, and an absent one is another; none of the three may share a return value.
    crate::parsers::or_unrecognised_json(
        "dotnet",
        found,
        items.map(Vec::len),
        "JSON with no `data` array, or a `data` array none of whose entries carry a `packageId`",
        output,
    )
}

#[cfg(test)]
mod json_tests {
    use super::*;

    /// Verbatim from `dotnet tool list --global --format json` on SDK 10.0.301.
    const REAL: &str = r#"{"version":1,"data":[{"packageId":"dotnetsay","version":"3.0.3","commands":["dotnetsay"]}]}"#;

    #[test]
    fn a_tool_is_its_package_id_and_version() {
        let pkgs = parse_dotnet_list_json(REAL).expect("this fixture parses");
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
        let a = parse_dotnet_list(table).expect("this fixture parses");
        let b = parse_dotnet_list_json(REAL).expect("this fixture parses");
        assert_eq!(
            a.iter().map(|p| (&p.name, &p.version)).collect::<Vec<_>>(),
            b.iter().map(|p| (&p.name, &p.version)).collect::<Vec<_>>(),
        );
    }

    /// An SDK too old for `--format json` fails rather than printing the table, so this never
    /// sees one — but if the negotiation ever regressed, reading a table as JSON must report
    /// nothing rather than inventing a package from a header row.
    #[test]
    fn an_empty_data_array_is_an_empty_machine_and_everything_else_is_unread() {
        assert!(
            parse_dotnet_list_json(r#"{"version":1,"data":[]}"#)
                .expect("dotnet with no global tools returns an empty `data` array")
                .is_empty()
        );
        // The table form is what an SDK *without* `--format json` prints, and the empty string
        // is what it prints when the command fails outright. Both used to arrive as "no global
        // tools are installed" — for exactly the users on older tooling, who are the least
        // likely to notice. `{"version":1}` with no `data` is the third: a schema this does not
        // read.
        for broken in [
            "Package Id  Version\n----\nx  1.0\n",
            "",
            r#"{"version":1}"#,
        ] {
            let err = parse_dotnet_list_json(broken)
                .expect_err("output this reader could not read is not an empty machine");
            assert_eq!(err.backend, "dotnet", "{broken}");
        }
    }
}
