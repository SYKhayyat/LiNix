use once_cell::sync::Lazy;
use regex::Regex;

static ANSI_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"[\u001b\u009b]\[[()#;?]*(?:[0-9]{1,4}(?:;[0-9]{0,4})*)?[0-9A-ORZcf-nqry=><]")
        .unwrap()
});

/// Collapses CRLF only; a lone `\r` (winget's progress spinner) survives and must be
/// handled by the caller.
pub fn sanitize(input: &str) -> String {
    let cleaned = ANSI_REGEX.replace_all(input, "");
    cleaned.replace("\r\n", "\n").trim().to_string()
}

/// Quoted runs stay one token: Windows managers emit names/versions containing spaces
/// ("7.3.4 (x64)"), which bare whitespace splitting would tear into separate columns.
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
