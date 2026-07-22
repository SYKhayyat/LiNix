use crate::core::Package;
use crate::parsers::utils::sanitize;

/// A generic parser for backends that return a simple space-separated list.
/// Format: "package-name version" or just "package-name"
/// Used by backends like 'apk' or internal search utilities.
pub fn parse_simple_list(output: &str, backend: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                Some(Package::with_version(parts[0], parts[1], backend))
            } else if parts.len() == 1 {
                Some(Package::new(parts[0], backend))
            } else {
                None
            }
        })
        .collect()
}

/// Parses a list where the version is attached to the name via a dash.
///
/// `package-name-1.2.3-r1` -> name `package-name`, version `1.2.3-r1`. Alpine's `apk info -v`
/// and `apk search -v` both answer this way; the search form adds ` - description` after the
/// token, so only the first whitespace-separated field is read.
///
/// **The version half must start with a digit.** Package names contain dashes — `xz-libs-dev`
/// split blind gives name `xz` and version `libs-dev`, and since this is apk's installed
/// lister, a package under the wrong name can never be matched by `info()`, so `remove`
/// silently does nothing. `xbps` and `pkgsrc` already parse this shape with the digit check;
/// this is the same rule, not a new one.
pub fn parse_dash_version_list(output: &str, backend: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let token = line.split_whitespace().next()?;
            Some(match split_trailing_version(token) {
                Some((name, version)) => Package::with_version(name, &version, backend),
                None => Package::new(token, backend),
            })
        })
        .collect()
}

/// `name-1.2.3-r1` -> `("name", "1.2.3-r1")`, and `None` when no trailing field looks like a
/// version. The scan is right-to-left so the *last* dash-joined version wins.
pub(crate) fn split_trailing_version(token: &str) -> Option<(&str, String)> {
    let starts_with_digit = |s: &str| s.chars().next().is_some_and(|c| c.is_ascii_digit());

    // `name-1.2.3-r1`: the revision is not itself a version, so try two fields before one.
    if let Some((head, rev)) = token.rsplit_once('-') {
        if let Some((name, ver)) = head.rsplit_once('-') {
            if starts_with_digit(ver) && !name.is_empty() {
                return Some((name, format!("{}-{}", ver, rev)));
            }
        }
        if starts_with_digit(rev) && !head.is_empty() {
            return Some((head, rev.to_string()));
        }
    }
    None
}

/// A strict CSV-style parser for backends that support delimited output.
pub fn parse_delimited(output: &str, delimiter: char, backend: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split(delimiter).collect();
            if parts.len() >= 2 {
                Some(Package::with_version(
                    parts[0].trim(),
                    parts[1].trim(),
                    backend,
                ))
            } else if !parts[0].is_empty() {
                Some(Package::new(parts[0].trim(), backend))
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apk_search_output_yields_the_bare_name() {
        // `apk search -v` adds ` - description` after the token. Splitting on whitespace
        // alone left the name as `jq-1.7.1-r0`, which never equals the name a line asked
        // for — so apk answered "I don't have it" for everything it had.
        let res = parse_dash_version_list("jq-1.7.1-r0 - Command-line JSON processor\n", "apk");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].name, "jq");
        assert_eq!(res[0].version.as_deref(), Some("1.7.1-r0"));
    }

    #[test]
    fn a_dash_in_the_name_is_not_mistaken_for_a_version() {
        // Alpine names really do look like this. Split blind, `xz-libs-dev` became name
        // `xz` — and this is the installed lister, so `info()` could never find the real
        // package and `remove` silently did nothing.
        let res = parse_dash_version_list("xz-libs-dev\npy3-requests-2.31.0-r0\nbash-5.2.15\n", "apk");
        assert_eq!(res[0].name, "xz-libs-dev");
        assert_eq!(res[0].version, None);
        assert_eq!(res[1].name, "py3-requests");
        assert_eq!(res[1].version.as_deref(), Some("2.31.0-r0"));
        // No `-rN` revision: the version is still a version.
        assert_eq!(res[2].name, "bash");
        assert_eq!(res[2].version.as_deref(), Some("5.2.15"));
    }

    #[test]
    fn a_name_with_no_version_survives_whole() {
        let res = parse_dash_version_list("tree\n", "apk");
        assert_eq!(res[0].name, "tree");
        assert_eq!(res[0].version, None);
    }
}
