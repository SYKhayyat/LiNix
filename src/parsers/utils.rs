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

    /// A line with nothing quoted must split exactly as whitespace splitting would, or the
    /// `quoted = true` option would be a behaviour change for every row that set it.
    #[test]
    fn an_unquoted_line_splits_the_ordinary_way() {
        for line in ["git 2.40.0 installed", "  ripgrep\t13.0.0  ", "solo"] {
            let mine: Vec<String> = split_columns(line);
            let theirs: Vec<&str> = line.split_whitespace().collect();
            assert_eq!(mine, theirs, "{line:?}");
        }
    }

    /// Both bracket shapes, because the pattern accepts both and only one has a caller today.
    #[test]
    fn a_version_is_taken_from_either_bracket() {
        assert_eq!(
            extract_version_bracketed("Xcode (14.3.1)").as_deref(),
            Some("14.3.1")
        );
        assert_eq!(
            extract_version_bracketed("pkg [1.2.3]").as_deref(),
            Some("1.2.3")
        );
        assert_eq!(
            extract_version_bracketed("bundler (default: 4.0.10)").as_deref(),
            Some("default: 4.0.10")
        );
    }

    /// The whole reason the two callers extract instead of trimming: a trim on a line with no
    /// brackets hands the line back, and the caller writes it down as a version.
    #[test]
    fn a_line_with_no_brackets_has_no_version_rather_than_all_of_it() {
        assert_eq!(extract_version_bracketed("Xcode 14.3.1"), None);
        assert_eq!(extract_version_bracketed(""), None);
        // An opening bracket alone is not a bracketed run.
        assert_eq!(extract_version_bracketed("pkg (14.3.1"), None);
    }
}
