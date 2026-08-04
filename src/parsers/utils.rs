use once_cell::sync::Lazy;
use regex::Regex;

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

/// A literal pattern, so it is compiled once for the process rather than once per package
/// line parsed — which is what this used to do, on every line of every listing.
static BRACKETED: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\(\[](.*?)[\)\]]").unwrap());

pub fn extract_version_bracketed(input: &str) -> Option<String> {
    BRACKETED.captures(input).map(|cap| cap[1].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_columns_quoted() {
        let input = "Microsoft.PowerShell \"7.3.4 (x64)\" installed";
        let cols = split_columns(input);
        assert_eq!(cols.len(), 3);
        assert_eq!(cols[1], "7.3.4 (x64)");
    }
}
