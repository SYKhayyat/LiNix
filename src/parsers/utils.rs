use once_cell::sync::Lazy;
use regex::Regex;

/// A production-grade regular expression to identify and strip ANSI escape codes.
/// Used to ensure that CLI output from backends is clean before parsing.
static ANSI_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"[\u001b\u009b]\[[()#;?]*(?:[0-9]{1,4}(?:;[0-9]{0,4})*)?[0-9A-ORZcf-nqry=><]")
        .unwrap()
});

/// Cleans raw CLI output by removing ANSI color codes, normalizing CRLF to LF,
/// and trimming leading/trailing whitespace.
/// Essential for consistent cross-platform parsing.
pub fn sanitize(input: &str) -> String {
    let cleaned = ANSI_REGEX.replace_all(input, "");
    cleaned.replace("\r\n", "\n").trim().to_string()
}

/// Splits a string into columns based on whitespace, but handles quoted strings
/// as single tokens. Useful for Windows managers like Winget that use spaces in names.
pub fn split_columns(line: &str) -> Vec<String> {
    let mut columns = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for c in line.chars() {
        match c {
            '"' => in_quotes = !in_quotes,
            ' ' | '\t' if !in_quotes => {
                if !current.is_empty() {
                    columns.push(current.clone());
                    current.clear();
                }
            }
            _ => current.push(c),
        }
    }

    if !current.is_empty() {
        columns.push(current);
    }

    columns
}

/// Extracts a version string from a line that likely contains "name version" or "name (version)".
pub fn extract_version_bracketed(input: &str) -> Option<String> {
    let re = Regex::new(r"[\(\[](.*?)[\)\]]").ok()?;
    re.captures(input).map(|cap| cap[1].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_ansi() {
        let input = "\u{1b}[32mSuccessfully installed\u{1b}[0m package-1.2.3\r\n";
        assert_eq!(sanitize(input), "Successfully installed package-1.2.3");
    }

    #[test]
    fn test_split_columns_quoted() {
        let input = "Microsoft.PowerShell \"7.3.4 (x64)\" installed";
        let cols = split_columns(input);
        assert_eq!(cols.len(), 3);
        assert_eq!(cols[1], "7.3.4 (x64)");
    }
}
