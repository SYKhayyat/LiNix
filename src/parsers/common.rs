use crate::core::Package;
use crate::parsers::{or_unrecognised, ParseResult};
use crate::utils::text::sanitize;

/// A generic parser for backends that return a simple space-separated list.
/// Format: "package-name version" or just "package-name"
/// Used by backends like 'apk' or internal search utilities.
pub fn parse_simple_list(output: &str, backend: &str) -> ParseResult {
    let clean = sanitize(output);
    let candidates = crate::parsers::data_lines(&clean);
    let found = candidates
        .iter()
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
        .collect();
    or_unrecognised(backend, found, &candidates)
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
pub fn parse_dash_version_list(output: &str, backend: &str) -> ParseResult {
    let clean = sanitize(output);
    let candidates = crate::parsers::data_lines(&clean);
    let found = candidates
        .iter()
        .filter_map(|line| {
            let token = line.split_whitespace().next()?;
            Some(match split_trailing_version(token) {
                Some((name, version)) => Package::with_version(name, &version, backend),
                None => Package::new(token, backend),
            })
        })
        .collect();
    or_unrecognised(backend, found, &candidates)
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
pub fn parse_delimited(output: &str, delimiter: char, backend: &str) -> ParseResult {
    let clean = sanitize(output);
    let candidates = crate::parsers::data_lines(&clean);
    let found = candidates
        .iter()
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
        .collect();
    or_unrecognised(backend, found, &candidates)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apk_search_output_yields_the_bare_name() {
        // `apk search -v` adds ` - description` after the token. Splitting on whitespace
        // alone left the name as `jq-1.7.1-r0`, which never equals the name a line asked
        // for — so apk answered "I don't have it" for everything it had.
        let res = parse_dash_version_list("jq-1.7.1-r0 - Command-line JSON processor\n", "apk")
            .expect("this fixture parses");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].name, "jq");
        assert_eq!(res[0].version.as_deref(), Some("1.7.1-r0"));
    }

    #[test]
    fn a_dash_in_the_name_is_not_mistaken_for_a_version() {
        // Alpine names really do look like this. Split blind, `xz-libs-dev` became name
        // `xz` — and this is the installed lister, so `info()` could never find the real
        // package and `remove` silently did nothing.
        let res =
            parse_dash_version_list("xz-libs-dev\npy3-requests-2.31.0-r0\nbash-5.2.15\n", "apk")
                .expect("this fixture parses");
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
        let res = parse_dash_version_list("tree\n", "apk").expect("this fixture parses");
        assert_eq!(res[0].name, "tree");
        assert_eq!(res[0].version, None);
    }
}

/// `apk version -l '<'` (`Q44`): `name-installed < available`, under a two-column header.
///
/// ```text
/// Installed:                                Available:
/// apk-tools-3.0.6-r0                      < 3.0.7-r0
/// ```
///
/// The left side is a `name-version` token, split by the same rule the installed listing uses
/// — a different rule here would produce names that match nothing the caller holds.
pub fn parse_apk_outdated(output: &str, backend: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let (installed, available) = line.split_once('<')?;
            let token = installed.split_whitespace().next()?;
            let available = available.trim();
            if available.is_empty() {
                return None;
            }
            let (name, _) = split_trailing_version(token)?;
            Some(Package::with_version(name, available, backend))
        })
        .collect()
}

#[cfg(test)]
mod apk_outdated_tests {
    use super::*;

    /// Verbatim from `apk version -l '<'` in a `shall-it-alpine` container.
    const APK: &str = "\
Installed:                                Available:
apk-tools-3.0.6-r0                      < 3.0.7-r0 
libapk-3.0.6-r0                         < 3.0.7-r0 
nodejs-24.17.0-r0                       < 24.18.1-r0 
";

    #[test]
    fn apk_splits_the_name_the_same_way_the_listing_does() {
        let p = parse_apk_outdated(APK, "apk");
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].name, "apk-tools");
        assert_eq!(p[0].version.as_deref(), Some("3.0.7-r0"));
        assert_eq!(p[2].name, "nodejs");
        assert_eq!(p[2].version.as_deref(), Some("24.18.1-r0"));
    }

    /// The header carries no `<`, so it is not a row — and a name that matched nothing would
    /// be invisible rather than loud.
    #[test]
    fn the_header_is_not_a_package() {
        assert!(!parse_apk_outdated(APK, "apk")
            .iter()
            .any(|p| p.name.contains("Installed")));
    }

    #[test]
    fn nothing_outdated_is_nothing() {
        assert!(parse_apk_outdated("", "apk").is_empty());
        assert!(parse_apk_outdated("Installed:  Available:\n", "apk").is_empty());
    }
}

/// `brew outdated --json=v2` (`Q44`).
///
/// ```json
/// {"formulae":[{"name":"jq","installed_versions":["1.7"],"current_version":"1.7.1"}],"casks":[]}
/// ```
///
/// **The JSON is found, not assumed.** brew prints `==> Auto-updating Homebrew...` and its
/// environment hints ahead of the payload, so parsing the whole output fails and would report
/// nothing outdated on precisely the machines that had not run brew recently. Casks are read
/// too: a cask is installed by the same declaration and its updates are updates.
pub fn parse_brew_outdated(output: &str) -> Vec<Package> {
    let text = sanitize(output);
    let Some(start) = text.find('{') else {
        return Vec::new();
    };
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text[start..]) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for key in ["formulae", "casks"] {
        let Some(items) = doc.get(key).and_then(|f| f.as_array()) else {
            continue;
        };
        for item in items {
            let Some(name) = item.get("name").and_then(|n| n.as_str()) else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let current = item
                .get("current_version")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty());
            out.push(match current {
                Some(v) => Package::with_version(name, v, "brew"),
                None => Package::new(name, "brew"),
            });
        }
    }
    out
}

#[cfg(test)]
mod brew_outdated_tests {
    use super::*;

    const BREW: &str = r#"{"formulae":[
        {"name":"jq","installed_versions":["1.7"],"current_version":"1.7.1"},
        {"name":"ripgrep","installed_versions":["14.1.0"],"current_version":"15.2.0"}],
      "casks":[{"name":"firefox","installed_versions":"140.0","current_version":"141.0"}]}"#;

    #[test]
    fn brew_reports_the_current_version_and_includes_casks() {
        let p = parse_brew_outdated(BREW);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].name, "jq");
        assert_eq!(p[0].version.as_deref(), Some("1.7.1"));
        assert_eq!(
            p[2].name, "firefox",
            "a cask is installed by the same declaration, so its updates are updates"
        );
        assert_eq!(p[2].version.as_deref(), Some("141.0"));
    }

    /// Measured in a `homebrew/brew` container: brew prints `==> Auto-updating Homebrew...`
    /// and its env hints ahead of the payload. Parsing the whole output fails, and a failed
    /// parse here reports nothing outdated on exactly the machines that had not run brew in a
    /// while.
    #[test]
    fn a_banner_before_the_json_does_not_hide_the_answer() {
        let noisy = format!(
            "==> Auto-updating Homebrew...\nAdjust how often this is run with $HOMEBREW…\n{}",
            BREW
        );
        assert_eq!(parse_brew_outdated(&noisy).len(), 3);
    }

    #[test]
    fn nothing_outdated_is_nothing() {
        assert!(parse_brew_outdated("").is_empty());
        assert!(parse_brew_outdated("==> Auto-updating Homebrew...\n").is_empty());
        assert!(parse_brew_outdated(r#"{"formulae":[],"casks":[]}"#).is_empty());
    }
}
