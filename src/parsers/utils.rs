use once_cell::sync::Lazy;
use regex::Regex;

static ANSI_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"[\u001b\u009b]\[[()#;?]*(?:[0-9]{1,4}(?:;[0-9]{0,4})*)?[0-9A-ORZcf-nqry=><]")
        .unwrap()
});

/// Collapses CRLF only; a lone `\r` (winget's progress spinner) survives and must be
/// handled by the caller.
///
/// Runs on every command's output. The common case on Linux is text with no escapes and no
/// CRLF, where this still allocated three `String`s — one for `replace_all`, one for
/// `replace`, one for `trim().to_string()`. That case allocates one now, and only because the
/// signature promises an owned value.
pub fn sanitize(input: &str) -> String {
    let cleaned = ANSI_REGEX.replace_all(input, "");
    match cleaned {
        std::borrow::Cow::Borrowed(s) if !s.contains("\r\n") => s.trim().to_string(),
        other => other.replace("\r\n", "\n").trim().to_string(),
    }
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
