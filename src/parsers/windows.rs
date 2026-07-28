use crate::core::Package;
use crate::parsers::utils::sanitize;

pub fn parse_installed(backend: &str, output: &str) -> Vec<Package> {
    match backend {
        "winget" => parse_winget_list(output),
        "choco" => parse_choco_list(output),
        "scoop" => parse_scoop_list(output),
        _ => vec![],
    }
}

pub fn parse_search(backend: &str, output: &str) -> Vec<Package> {
    match backend {
        "winget" => parse_winget_search(output),
        "choco" => parse_choco_search(output),
        "scoop" => parse_scoop_search(output),
        _ => vec![],
    }
}

/// winget prints a progress spinner using bare carriage returns (without newlines)
/// before the real header, e.g. `   - \r   \ \r ... \rName  Id  ...`. `sanitize` only
/// collapses CRLF, so lone `\r` survives. Keep only the content after the last `\r`.
fn strip_cr_spinner(line: &str) -> &str {
    match line.rfind('\r') {
        Some(idx) => &line[idx + 1..],
        None => line,
    }
}

/// True for the dashed separator row winget draws under the header.
fn is_separator(line: &str) -> bool {
    let t = strip_cr_spinner(line).trim();
    !t.is_empty() && t.chars().all(|c| c == '-' || c == ' ')
}

/// Parse a winget fixed-width table, returning each data row's value for the requested
/// columns (in the requested order). winget pads every column to a fixed width and
/// Names/Ids legitimately contain spaces (e.g. "7-Zip 25.01 (x64)",
/// "ARP\\Machine\\X64\\Android Studio"), so the columns MUST be sliced by the header
/// positions — whitespace splitting corrupts multi-word fields.
fn parse_winget_table(output: &str, columns_wanted: &[&str]) -> Vec<Vec<String>> {
    // The header is the first line containing both "Name" and "Id".
    let known = ["Name", "Id", "Version", "Available", "Match", "Source"];
    slice_fixed_table(
        output,
        &known,
        |line| line.contains("Name") && line.contains("Id"),
        columns_wanted,
    )
}

/// Slice a fixed-width CLI table by its header's column offsets.
///
/// `known` names every column that may appear, `header_matches` recognizes the header
/// row, and `columns_wanted` selects which values each returned row carries, in order.
/// A column absent from this particular header yields an empty string.
///
/// Splitting such a table on whitespace instead is the fault this exists to prevent: an
/// EMPTY cell disappears, every later value shifts one place left, and the row still
/// parses — so scoop's failed-install row (no Version, no Source) read as a package
/// whose version was the date it was attempted.
fn slice_fixed_table(
    output: &str,
    known: &[&str],
    header_matches: impl Fn(&str) -> bool,
    columns_wanted: &[&str],
) -> Vec<Vec<String>> {
    let text = sanitize(output);
    let lines: Vec<&str> = text.lines().collect();

    let Some(hdr_idx) = lines
        .iter()
        .position(|l| header_matches(strip_cr_spinner(l)))
    else {
        return vec![];
    };

    let header = strip_cr_spinner(lines[hdr_idx]);
    // Locate every known column by its char-offset start in the cleaned header.
    let mut cols: Vec<(usize, &str)> = known
        .iter()
        .filter_map(|name| {
            header
                .find(name)
                .map(|b| (header[..b].chars().count(), *name))
        })
        .collect();
    cols.sort_by_key(|c| c.0);

    // A column spans from its start to the next column's start (or end of line).
    let col_range = |label: &str| -> Option<(usize, Option<usize>)> {
        let pos = cols.iter().position(|(_, l)| *l == label)?;
        Some((cols[pos].0, cols.get(pos + 1).map(|c| c.0)))
    };

    let mut rows = Vec::new();
    for line in lines.iter().skip(hdr_idx + 1) {
        if line.trim().is_empty() || is_separator(line) {
            continue;
        }
        let chars: Vec<char> = strip_cr_spinner(line).chars().collect();
        let values: Vec<String> = columns_wanted
            .iter()
            .map(|want| match col_range(want) {
                Some((start, end)) if start < chars.len() => {
                    let e = end.unwrap_or(chars.len()).min(chars.len()).max(start);
                    chars[start..e]
                        .iter()
                        .collect::<String>()
                        .trim()
                        .to_string()
                }
                _ => String::new(),
            })
            .collect();
        if values.iter().all(|v| v.is_empty()) {
            continue;
        }
        rows.push(values);
    }
    rows
}

/// Parses output from 'winget list' (Name / Id / Version / Available / Source).
/// The Id is the canonical identity used by `winget install`, so prefer it (falling
/// back to the display Name for rows that lack an Id).
fn parse_winget_list(output: &str) -> Vec<Package> {
    parse_winget_table(output, &["Id", "Name", "Version"])
        .into_iter()
        .filter_map(|row| {
            let ident = if !row[0].is_empty() { &row[0] } else { &row[1] };
            if ident.is_empty() {
                return None;
            }
            let mut p = Package::new(ident, "winget");
            if !row[2].is_empty() {
                p.version = Some(row[2].clone());
            }
            Some(p)
        })
        .collect()
}

/// Parses output from 'choco list -lo -r' (local only, readable/piped).
/// Expected input format: "name|version"
fn parse_choco_list(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let (name, ver) = line.split_once('|')?;
            Some(Package::with_version(name.trim(), ver.trim(), "choco"))
        })
        .collect()
}

/// Parses output from 'scoop list' (Name / Version / Source / Updated / Info).
///
/// Sliced by header offsets, not whitespace — scoop leaves Version and Source EMPTY for
/// an app whose install failed, and it keeps that row in `list` forever:
///
/// ```text
/// Name     Version         Source Updated             Info
/// 7zip     26.00           main   2026-04-19 07:09:55
/// jq                              2026-07-21 13:48:29 Install failed
/// ```
///
/// A row like that is not an installed package. Read by splitting on whitespace it was
/// one — named `jq`, versioned `2026-07-21` — so `sync` believed there was nothing to
/// do, `adopt` wrote it into a manifest, and no `jq` was ever on PATH.
fn parse_scoop_list(output: &str) -> Vec<Package> {
    let known = ["Name", "Version", "Source", "Updated", "Info"];
    slice_fixed_table(
        output,
        &known,
        |line| line.contains("Name") && line.contains("Version"),
        &["Name", "Version", "Info"],
    )
    .into_iter()
    .filter_map(|row| {
        let (name, version, info) = (&row[0], &row[1], &row[2]);
        if name.is_empty() {
            return None;
        }
        // scoop reports the outcome in Info and nowhere else; the row itself stays.
        if info.to_ascii_lowercase().contains("failed") {
            return None;
        }
        // No version means scoop has a directory for it and no installed manifest —
        // the same half-state by a different route.
        if version.is_empty() {
            return None;
        }
        Some(Package::with_version(name, version, "scoop"))
    })
    .collect()
}

/// Parses 'winget search' output table (Name / Id / Version / Match / Source).
fn parse_winget_search(output: &str) -> Vec<Package> {
    parse_winget_table(output, &["Id", "Name", "Version"])
        .into_iter()
        .filter_map(|row| {
            let ident = if !row[0].is_empty() { &row[0] } else { &row[1] };
            if ident.is_empty() {
                return None;
            }
            let mut p = Package::new(ident, "winget");
            if !row[2].is_empty() {
                p.version = Some(row[2].clone());
            }
            Some(p)
        })
        .collect()
}

/// Parse `choco search`, in either the machine form (`-r`, `name|version`) or the human one.
///
/// It took the first token of every line, so choco's own banner became a package named
/// `Chocolatey` at version `v2.7.3` and its own summary line `5 packages found.` became a
/// package named `5` at version `packages`. Both were offered to a user choosing what to
/// install. `list` had already been given `-r` for a related reason; `search` had not, which
/// is the twin-path half of the same bug.
fn parse_choco_search(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            // `-r` output. Unambiguous, and the reason `search` now asks for it.
            if let Some((name, version)) = line.split_once('|') {
                let mut p = Package::new(name.trim(), "choco");
                let v = version.trim();
                if !v.is_empty() {
                    p.version = Some(v.to_string());
                }
                return Some(p);
            }
            // The human form, still parsed so a `-r` that stops working is a wrong answer
            // rather than an empty one.
            let mut parts = line.split_whitespace();
            let name = parts.next()?;
            let version = parts.next();
            // choco's own words about itself, not packages: the `Chocolatey v2.7.3` banner,
            // the `N packages found.` / `N validations performed.` summaries, and the
            // "did you know" marketing footer.
            if name.eq_ignore_ascii_case("chocolatey") || name.parse::<u64>().is_ok() {
                return None;
            }
            // A real row's second column is a version. A prose line's is a word.
            let version = version.filter(|v| v.starts_with(|c: char| c.is_ascii_digit()))?;
            let mut p = Package::new(name, "choco");
            p.version = Some(version.to_string());
            Some(p)
        })
        .collect()
}

/// Parses 'scoop search' results.
fn parse_scoop_search(output: &str) -> Vec<Package> {
    // Modern scoop (0.5+) prints a table:
    //   Results from local buckets...
    //
    //   Name    Version Source Binaries
    //   ----    ------- ------ --------
    //   ripgrep 15.1.0  main
    // Sliced by header offsets like `list`, and for the same reason: an empty Binaries
    // or Source cell must not shift the row's other values one place left.
    let known = ["Name", "Version", "Source", "Binaries"];
    slice_fixed_table(
        output,
        &known,
        |line| line.contains("Name") && line.contains("Version"),
        &["Name", "Version"],
    )
    .into_iter()
    .filter_map(|row| {
        let (name, version) = (&row[0], &row[1]);
        if name.is_empty() {
            return None;
        }
        if version.is_empty() {
            return Some(Package::new(name, "scoop"));
        }
        Some(Package::with_version(name, version, "scoop"))
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real `scoop list` from a machine that had a failed install sitting in it. The
    /// row keeps its Name and Updated and has NO Version and NO Source, so a
    /// whitespace-split read it as `jq` at version `2026-07-21`.
    fn scoop_list_fixture() -> String {
        let row = |name: &str, ver: &str, src: &str, updated: &str, info: &str| {
            format!("{:<9}{:<16}{:<7}{:<20}{}", name, ver, src, updated, info)
        };
        [
            "Installed apps:".to_string(),
            String::new(),
            row("Name", "Version", "Source", "Updated", "Info"),
            row("----", "-------", "------", "-------", "----"),
            row("7zip", "26.00", "main", "2026-04-19 07:09:55", ""),
            row("jq", "", "", "2026-07-21 13:48:29", "Install failed"),
            row("ripgrep", "15.1.0", "main", "2026-07-08 15:38:44", ""),
        ]
        .join("\n")
    }

    #[test]
    fn scoop_list_drops_a_failed_install() {
        let res = parse_scoop_list(&scoop_list_fixture());
        let names: Vec<&str> = res.iter().map(|p| p.name.as_str()).collect();
        assert!(
            !names.contains(&"jq"),
            "a row whose Info says the install failed is not an installed package: {:?}",
            names
        );
        assert!(
            names.contains(&"7zip") && names.contains(&"ripgrep"),
            "{:?}",
            names
        );
    }

    #[test]
    fn scoop_list_reads_versions_from_the_version_column() {
        let res = parse_scoop_list(&scoop_list_fixture());
        let seven = res.iter().find(|p| p.name == "7zip").unwrap();
        assert_eq!(seven.version.as_deref(), Some("26.00"));
        // The date must never reach a version field — that is what the shifted read did.
        assert!(
            res.iter().all(|p| !p
                .version
                .as_deref()
                .unwrap_or_default()
                .starts_with("2026-")),
            "an Updated timestamp was parsed as a version: {:?}",
            res.iter()
                .map(|p| (&p.name, &p.version))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn scoop_list_ingests_no_header_or_separator() {
        let names: Vec<String> = parse_scoop_list(&scoop_list_fixture())
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert!(
            !names
                .iter()
                .any(|n| n == "Name" || n.starts_with('-') || n == "Installed"),
            "{:?}",
            names
        );
    }

    /// The same empty-cell shift on the search table: a row with no Binaries must not
    /// borrow the next column's value, and one with no Version must not borrow Source.
    #[test]
    fn scoop_search_survives_empty_trailing_columns() {
        let row = |name: &str, ver: &str, src: &str, bins: &str| {
            format!("{:<8}{:<8}{:<7}{}", name, ver, src, bins)
        };
        let out = [
            "Results from local buckets...".to_string(),
            String::new(),
            row("Name", "Version", "Source", "Binaries"),
            row("----", "-------", "------", "--------"),
            row("rga", "0.10.9", "main", "ripgrep-all.exe"),
            row("ripgrep", "15.1.0", "main", ""),
        ]
        .join("\n");
        let res = parse_scoop_search(&out);
        let rg = res.iter().find(|p| p.name == "ripgrep").unwrap();
        assert_eq!(rg.version.as_deref(), Some("15.1.0"));
        let rga = res.iter().find(|p| p.name == "rga").unwrap();
        assert_eq!(rga.version.as_deref(), Some("0.10.9"));
        assert_eq!(res.len(), 2, "got {:?}", res);
    }

    #[test]
    fn scoop_search_parses_modern_table() {
        // Real `scoop search ripgrep` output (0.5.x).
        let out = "Results from local buckets...\n\nName    Version Source Binaries\n----    ------- ------ --------\nrga     0.10.9  main   ripgrep-all.exe\nripgrep 15.1.0  main\n";
        let res = parse_scoop_search(out);
        let names: Vec<&str> = res.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"ripgrep"), "got {:?}", names);
        assert!(names.contains(&"rga"));
        // header/separator/chatter must not leak in as packages
        assert!(!names
            .iter()
            .any(|n| n.starts_with('-') || *n == "Name" || *n == "Results"));
        let rg = res.iter().find(|p| p.name == "ripgrep").unwrap();
        assert_eq!(rg.version.as_deref(), Some("15.1.0"));
    }

    // Build a fixed-width winget row so the test fixtures match real `winget list`
    // column alignment (the previous fixture used single spaces, which is why it
    // passed while real multi-word output was mis-parsed).
    fn wrow(name: &str, id: &str, ver: &str, avail: &str, src: &str) -> String {
        format!("{:<24}{:<40}{:<14}{:<14}{}", name, id, ver, avail, src)
    }

    fn winget_list_fixture() -> String {
        let header = wrow("Name", "Id", "Version", "Available", "Source");
        let sep = "-".repeat(110);
        let rows = [
            wrow("7-Zip 25.01 (x64)", "7zip.7zip", "25.01", "26.01", "winget"),
            wrow(
                "Android Studio",
                "ARP\\Machine\\X64\\Android Studio",
                "2025.1",
                "",
                "",
            ),
            wrow("Git", "Git.Git", "2.54.0", "", "winget"),
        ];
        format!("{}\n{}\n{}\n", header, sep, rows.join("\n"))
    }

    #[test]
    fn winget_list_uses_columns_not_whitespace() {
        let res = parse_installed("winget", &winget_list_fixture());
        assert_eq!(
            res.len(),
            3,
            "should parse exactly 3 rows, no header/garbage"
        );

        // multi-word display name must NOT corrupt identity/version
        let sevenz = res
            .iter()
            .find(|p| p.name == "7zip.7zip")
            .expect("7zip.7zip present");
        assert_eq!(sevenz.version.as_deref(), Some("25.01"));

        // ARP (non-winget) app: Id carries spaces+backslashes, parsed intact
        let studio = res
            .iter()
            .find(|p| p.name == "ARP\\Machine\\X64\\Android Studio")
            .expect("ARP id parsed whole");
        assert_eq!(studio.version.as_deref(), Some("2025.1"));

        // none of the old garbage fragments should appear as packages
        for bad in ["Studio", "(x64)", "25.01", "Name", "HDR"] {
            assert!(
                !res.iter().any(|p| p.name == bad),
                "unexpected garbage row: {bad}"
            );
        }
    }

    #[test]
    fn winget_list_handles_cr_spinner_header() {
        // Prepend the bare-\r progress spinner winget draws before the header.
        let fixture = winget_list_fixture();
        let with_spinner = format!("  - \r  \\ \r  / \r{}", fixture);
        let res = parse_installed("winget", &with_spinner);
        assert_eq!(res.len(), 3);
        assert!(res.iter().any(|p| p.name == "Git.Git"));
    }

    #[test]
    fn winget_search_parses_columns() {
        let header = wrow("Name", "Id", "Version", "Match", "Source");
        let sep = "-".repeat(110);
        let row = wrow(
            "Visual Studio Code",
            "Microsoft.VisualStudioCode",
            "1.85.0",
            "",
            "winget",
        );
        let input = format!("{}\n{}\n{}\n", header, sep, row);
        let res = parse_search("winget", &input);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].name, "Microsoft.VisualStudioCode");
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

#[cfg(test)]
mod real_output_tests {
    use super::*;

    /// Captured from Chocolatey v2.7.3 on this machine.
    const CHOCO_HUMAN: &str = include_str!("../../tests/fixtures/choco/search-ripgrep.txt");
    const CHOCO_MACHINE: &str =
        include_str!("../../tests/fixtures/choco/search-ripgrep-limitoutput.txt");
    const CHOCO_NOT_FOUND: &str = include_str!("../../tests/fixtures/choco/search-not-found.txt");

    /// choco's own banner and its own summary line were becoming packages: a package named
    /// `Chocolatey` at version `v2.7.3`, and one named `5` at version `packages`. Neither is a
    /// package and both were offered to a user choosing what to install.
    #[test]
    fn choco_search_yields_packages_and_never_the_banner_or_the_summary() {
        for (case, out) in [("human", CHOCO_HUMAN), ("machine", CHOCO_MACHINE)] {
            let names: Vec<String> = parse_choco_search(out)
                .into_iter()
                .map(|p| p.name)
                .collect();
            assert!(
                names.iter().any(|n| n == "ripgrep"),
                "{case}: lost the real package: {names:?}"
            );
            assert!(
                !names.iter().any(|n| n.eq_ignore_ascii_case("chocolatey")),
                "{case}: the version banner became a package: {names:?}"
            );
            assert!(
                !names.iter().any(|n| n == "5"),
                "{case}: the `N packages found.` summary became a package: {names:?}"
            );
        }
    }

    /// The version has to survive the fix, or the cure removes the answer with the junk.
    #[test]
    fn choco_search_keeps_the_version() {
        let found = parse_choco_search(CHOCO_MACHINE);
        let rg = found.iter().find(|p| p.name == "ripgrep").expect("ripgrep");
        assert_eq!(rg.version.as_deref(), Some("14.1.0"));
    }

    /// The empty case. `names_only`'s only test used a spack fixture and said nothing about
    /// the four other managers routed through it; this is the same trap one file over.
    #[test]
    fn choco_search_finding_nothing_yields_nothing() {
        assert!(parse_choco_search(CHOCO_NOT_FOUND).is_empty());
        assert!(parse_choco_search("").is_empty());
    }
}
