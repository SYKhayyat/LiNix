//! Parsers for Conda's `--json` output.
//!
//! `conda list --json` returns a flat array of package objects; `conda search
//! <q> --json` returns an object keyed by package name whose values are arrays of
//! candidate builds (ascending, so the last entry is the newest).

use crate::core::Package;
use crate::parsers::utils::sanitize;
use serde_json::Value;

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
