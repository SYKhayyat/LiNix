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
    let text = sanitize(output);
    let lines: Vec<&str> = text.lines().collect();

    // The header is the first line containing both "Name" and "Id".
    let Some(hdr_idx) = lines.iter().position(|l| {
        let c = strip_cr_spinner(l);
        c.contains("Name") && c.contains("Id")
    }) else {
        return vec![];
    };

    let header = strip_cr_spinner(lines[hdr_idx]);
    // Locate every known column by its char-offset start in the cleaned header.
    let known = ["Name", "Id", "Version", "Available", "Match", "Source"];
    let mut cols: Vec<(usize, &str)> = known
        .iter()
        .filter_map(|name| header.find(name).map(|b| (header[..b].chars().count(), *name)))
        .collect();
    cols.sort_by_key(|c| c.0);

    // A column spans from its start to the next column's start (or end of line).
    let col_range = |label: &str| -> Option<(usize, Option<usize>)> {
        let pos = cols.iter().position(|(_, l)| *l == label)?;
        Some((cols[pos].0, cols.get(pos + 1).map(|c| c.0)))
    };

    let mut rows = Vec::new();
    for line in lines.iter().skip(hdr_idx + 1) {
        if line.trim().is_empty() || is_separator(line) { continue; }
        let chars: Vec<char> = strip_cr_spinner(line).chars().collect();
        let values: Vec<String> = columns_wanted
            .iter()
            .map(|want| match col_range(want) {
                Some((start, end)) if start < chars.len() => {
                    let e = end.unwrap_or(chars.len()).min(chars.len()).max(start);
                    chars[start..e].iter().collect::<String>().trim().to_string()
                }
                _ => String::new(),
            })
            .collect();
        if values.iter().all(|v| v.is_empty()) { continue; }
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
            if ident.is_empty() { return None; }
            let mut p = Package::new(ident, "winget");
            if !row[2].is_empty() { p.version = Some(row[2].clone()); }
            Some(p)
        })
        .collect()
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

/// Parses 'winget search' output table (Name / Id / Version / Match / Source).
fn parse_winget_search(output: &str) -> Vec<Package> {
    parse_winget_table(output, &["Id", "Name", "Version"])
        .into_iter()
        .filter_map(|row| {
            let ident = if !row[0].is_empty() { &row[0] } else { &row[1] };
            if ident.is_empty() { return None; }
            let mut p = Package::new(ident, "winget");
            if !row[2].is_empty() { p.version = Some(row[2].clone()); }
            Some(p)
        })
        .collect()
}

/// Parses 'choco search' results.
fn parse_choco_search(output: &str) -> Vec<Package> {
    sanitize(output).lines()
        .filter_map(|line| {
            // Choco search usually returns "name version" on each line
            let parts: Vec<&str> = line.split_whitespace().collect();
            // Fix E0277: Dereference &&str
            let name = parts.first()?;
            let mut p = Package::new(*name, "choco");
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
            // Fix E0277: Dereference &&str
            let name = parts.first()?;
            Some(Package::new(*name, "scoop"))
        }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
            wrow("Android Studio", "ARP\\Machine\\X64\\Android Studio", "2025.1", "", ""),
            wrow("Git", "Git.Git", "2.54.0", "", "winget"),
        ];
        format!("{}\n{}\n{}\n", header, sep, rows.join("\n"))
    }

    #[test]
    fn winget_list_uses_columns_not_whitespace() {
        let res = parse_installed("winget", &winget_list_fixture());
        assert_eq!(res.len(), 3, "should parse exactly 3 rows, no header/garbage");

        // multi-word display name must NOT corrupt identity/version
        let sevenz = res.iter().find(|p| p.name == "7zip.7zip").expect("7zip.7zip present");
        assert_eq!(sevenz.version.as_deref(), Some("25.01"));

        // ARP (non-winget) app: Id carries spaces+backslashes, parsed intact
        let studio = res.iter().find(|p| p.name == "ARP\\Machine\\X64\\Android Studio")
            .expect("ARP id parsed whole");
        assert_eq!(studio.version.as_deref(), Some("2025.1"));

        // none of the old garbage fragments should appear as packages
        for bad in ["Studio", "(x64)", "25.01", "Name", "HDR"] {
            assert!(!res.iter().any(|p| p.name == bad), "unexpected garbage row: {bad}");
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
        let row = wrow("Visual Studio Code", "Microsoft.VisualStudioCode", "1.85.0", "", "winget");
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