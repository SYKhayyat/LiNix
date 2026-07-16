//! Parsers for Conda's `--json` output.
//!
//! `conda list --json` returns a flat array of package objects; `conda search
//! <q> --json` returns an object keyed by package name whose values are arrays of
//! candidate builds (ascending, so the last entry is the newest).

use crate::core::Package;
use crate::parsers::utils::sanitize;
use serde_json::Value;

/// Parses `conda env export -n <env> --from-history --json` — the packages a person
/// actually asked for, as opposed to the environment's full solved closure.
///
/// The distinction is not academic: on the stock `base` env of the test image,
/// `conda list` reports 88 packages while `--from-history` reports 4. Adopting the other
/// 84 would hand LiNix an entire dependency graph to later treat as removable.
///
/// `dependencies` is an array of match-specs, not names — `"python=3.13"`,
/// `"conda[version='>=26.3.2']"`, or a bare `"pip"` — so the name is everything before
/// the first version/bracket delimiter. A nested `{"pip": [...]}` object can appear in a
/// full export; it carries pip's packages, not conda's, and is skipped.
pub fn parse_conda_history(output: &str) -> Vec<Package> {
    let clean = sanitize(output);
    let json: Value = serde_json::from_str(&clean).unwrap_or_default();
    json.get("dependencies")
        .and_then(|d| d.as_array())
        .map(|deps| {
            deps.iter()
                .filter_map(|d| {
                    let spec = d.as_str()?;
                    let name = spec
                        .split(['=', '<', '>', '[', ' ', '!', '~'])
                        .next()?
                        .trim();
                    (!name.is_empty()).then(|| Package::new(name, "conda"))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parses `conda list -n <env> --json` — an array of `{ "name", "version", ... }`.
pub fn parse_conda_list(output: &str) -> Vec<Package> {
    let clean = sanitize(output);
    let json: Value = serde_json::from_str(&clean).unwrap_or_default();
    json.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    let name = p.get("name")?.as_str()?;
                    let ver = p.get("version").and_then(|v| v.as_str()).unwrap_or("");
                    Some(Package::with_version(name, ver, "conda"))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parses `conda search <query> --json` — an object mapping each matching package
/// name to an array of build objects. We surface one entry per name using the newest
/// (last) build's version. A `{ "error": ... }` payload (no match) yields no results.
pub fn parse_conda_search(output: &str) -> Vec<Package> {
    let clean = sanitize(output);
    let json: Value = serde_json::from_str(&clean).unwrap_or_default();
    let Some(obj) = json.as_object() else {
        return vec![];
    };
    if obj.contains_key("error") {
        return vec![];
    }
    obj.iter()
        .map(|(name, builds)| {
            let newest = builds.as_array().and_then(|a| a.last());
            let ver = newest
                .and_then(|b| b.get("version"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Package::with_version(name, ver, "conda")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_reports_only_what_was_asked_for() {
        // Verbatim from `conda env export -n base --from-history --json` on the tools test
        // image, where `conda list` reports 88 packages and this reports these 4.
        let input = r#"{
          "name": "base",
          "channels": ["conda-forge"],
          "dependencies": [
            "python=3.13",
            "conda[version='>=26.3.2']",
            "mamba[version='>=2.5.0']",
            "pip"
          ],
          "prefix": "/opt/conda"
        }"#;
        let names: Vec<String> = parse_conda_history(input)
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, vec!["python", "conda", "mamba", "pip"]);
    }

    #[test]
    fn history_of_an_untouched_env_is_empty_not_everything() {
        // The failure that matters: if this ever returned the full closure instead, migrate
        // would adopt an entire dependency graph.
        assert!(parse_conda_history(r#"{"name":"base","dependencies":[]}"#).is_empty());
        assert!(parse_conda_history("not json").is_empty());
    }

    #[test]
    fn parses_conda_list_json() {
        let input = r#"[
            {"name": "numpy", "version": "1.26.0", "channel": "defaults"},
            {"name": "pandas", "version": "2.1.1", "channel": "defaults"}
        ]"#;
        let res = parse_conda_list(input);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "numpy");
        assert_eq!(res[0].version.as_deref(), Some("1.26.0"));
        assert_eq!(res[1].name, "pandas");
    }

    #[test]
    fn parses_conda_search_json_newest_build() {
        let input = r#"{
            "numpy": [
                {"name": "numpy", "version": "1.25.0"},
                {"name": "numpy", "version": "1.26.0"}
            ]
        }"#;
        let res = parse_conda_search(input);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].name, "numpy");
        assert_eq!(res[0].version.as_deref(), Some("1.26.0"));
    }

    #[test]
    fn conda_search_error_payload_is_empty() {
        let input =
            r#"{"error": "PackagesNotFoundError", "exception_name": "PackagesNotFoundError"}"#;
        assert!(parse_conda_search(input).is_empty());
    }
}
