//! Parsers for Conda's `--json` output.
//!
//! `conda list --json` returns a flat array of package objects; `conda search
//! <q> --json` returns an object keyed by package name whose values are arrays of
//! candidate builds (ascending, so the last entry is the newest).

use crate::core::Package;
use crate::parsers::{or_unrecognised_json, ParseResult};
use crate::utils::text::sanitize;

/// Parses `conda env export -n <env> --from-history --json` — the packages a person
/// actually asked for, as opposed to the environment's full solved closure.
///
/// The distinction is not academic: on the stock `base` env of the test image,
/// `conda list` reports 88 packages while `--from-history` reports 4. Adopting the other
/// 84 would hand Shall an entire dependency graph to later treat as removable.
///
/// `dependencies` is an array of match-specs, not names — `"python=3.13"`,
/// `"conda[version='>=26.3.2']"`, or a bare `"pip"` — so the name is everything before
/// the first version/bracket delimiter. A nested `{"pip": [...]}` object can appear in a
/// full export; it carries pip's packages, not conda's, and is skipped.
pub fn parse_conda_history(output: &str) -> ParseResult {
    let clean = sanitize(output);
    let Some(json) = crate::parsers::json_document(&clean) else {
        return or_unrecognised_json("conda", vec![], None, "not JSON", &clean);
    };
    let deps = json.get("dependencies").and_then(|d| d.as_array());
    let found: Vec<Package> = deps
        .into_iter()
        .flatten()
        .filter_map(|d| {
            let spec = d.as_str()?;
            let name = spec
                .split(['=', '<', '>', '[', ' ', '!', '~'])
                .next()?
                .trim();
            (!name.is_empty()).then(|| Package::new(name, "conda"))
        })
        .collect();
    // An empty `dependencies` is an environment nobody asked anything of, which is real. A
    // populated one that yielded no name is a match-spec shape this does not read, and an
    // absent one is a schema change.
    or_unrecognised_json(
        "conda",
        found,
        deps.map(Vec::len),
        "JSON whose `dependencies` array is missing, or holds match-specs none of which \
         yielded a name",
        &clean,
    )
}

/// Parses `conda list -n <env> --json` — an array of `{ "name", "version", ... }`.
pub fn parse_conda_list(output: &str) -> ParseResult {
    let clean = sanitize(output);
    let Some(json) = crate::parsers::json_document(&clean) else {
        return or_unrecognised_json("conda", vec![], None, "not JSON", &clean);
    };
    let arr = json.as_array();
    let found: Vec<Package> = arr
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let name = p.get("name")?.as_str()?;
            let ver = p.get("version").and_then(|v| v.as_str()).unwrap_or("");
            Some(Package::with_version(name, ver, "conda"))
        })
        .collect();
    or_unrecognised_json(
        "conda",
        found,
        arr.map(Vec::len),
        "JSON that is not the array `conda list` returns, or an array of entries none \
         carrying a `name`",
        &clean,
    )
}

/// Parses `conda search <query> --json` — an object mapping each matching package
/// name to an array of build objects. We surface one entry per name using the newest
/// (last) build's version. A `{ "error": ... }` payload (no match) yields no results.
pub fn parse_conda_search(output: &str) -> Vec<Package> {
    let clean = sanitize(output);
    let json = crate::parsers::json_document(&clean).unwrap_or_default();
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
            .expect("this fixture parses")
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, vec!["python", "conda", "mamba", "pip"]);
    }

    #[test]
    fn history_of_an_untouched_env_is_empty_not_everything() {
        // The failure that matters: if this ever returned the full closure instead, `adopt`
        // would adopt an entire dependency graph.
        assert!(parse_conda_history(r#"{"name":"base","dependencies":[]}"#)
            .expect("an env nobody asked anything of has an empty `dependencies`")
            .is_empty());
    }

    /// The second half of that test used to be `parse_conda_history("not json").expect("this fixture parses").is_empty()`,
    /// sitting beside the assertion above under one name. An untouched environment and a conda
    /// that did not answer in JSON are opposite facts, and `adopt` acts on them in opposite
    /// directions — the first means *take nothing*, the second means *ask again, something is
    /// wrong*.
    #[test]
    fn output_that_is_not_json_is_not_an_untouched_environment() {
        let err = parse_conda_history("not json").expect_err("not an untouched environment");
        assert_eq!(err.backend, "conda");
        assert!(err.sample.starts_with("not JSON"), "{err:?}");
    }

    #[test]
    fn parses_conda_list_json() {
        let input = r#"[
            {"name": "numpy", "version": "1.26.0", "channel": "defaults"},
            {"name": "pandas", "version": "2.1.1", "channel": "defaults"}
        ]"#;
        let res = parse_conda_list(input).expect("this fixture parses");
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
