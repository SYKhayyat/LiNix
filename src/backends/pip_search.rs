//! Searching PyPI, which is a `SearchSource` rather than a backend of its own.
//!
//! The legacy `pip search` was disabled upstream — PyPI withdrew the XML-RPC endpoint over
//! abuse — and there is no public full-text replacement. So `search` is an EXACT-NAME lookup
//! against `https://pypi.org/pypi/<name>/json`: the package if it exists, nothing if it does
//! not. A documented limitation: name resolution, not discovery.
//!
//! **This used to be a bespoke `impl Searchable` bolted onto pip's registration**, which meant
//! *a search that is an HTTP call* was a thing only Rust could say — `node_registry.rs` had the
//! same shape and got a `SearchSource` variant, and pip did not. A row can now say
//! `search_source = "pypi"` for the same reason it can say `"npm_registry"`.

use crate::backends::node_registry::http_timeout;
use crate::core::{Error, Package, Result};

/// Resolve `query` against PyPI, tagging the result with `backend`.
pub async fn registry_search(query: &str, backend: &str) -> Result<Vec<Package>> {
    let name = query.trim();
    if name.is_empty() {
        return Ok(vec![]);
    }

    let client = crate::core::http::api("linix-manager", http_timeout().as_secs())?;

    let url = format!("https://pypi.org/pypi/{}/json", name);
    let res = client.get(&url).send().await.map_err(Error::from)?;

    // 404 simply means "no such package" — not an error for a search.
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(vec![]);
    }
    if !res.status().is_success() {
        return Err(Error::Other(format!("PyPI API error: {}", res.status())));
    }

    let json: serde_json::Value = res.json().await.map_err(Error::from)?;
    Ok(vec![parse_pypi(&json, name, backend)])
}

/// Parse a PyPI JSON document (`https://pypi.org/pypi/<name>/json`) into a `Package`.
/// `fallback_name` is used if the document omits `info.name`.
fn parse_pypi(json: &serde_json::Value, fallback_name: &str, backend: &str) -> Package {
    let info = &json["info"];
    let pkg_name = info["name"].as_str().unwrap_or(fallback_name);
    let mut p = Package::new(pkg_name, backend);
    if let Some(v) = info["version"].as_str() {
        p.version = Some(v.to_string());
    }
    if let Some(d) = info["summary"].as_str() {
        if !d.is_empty() {
            p.properties
                .insert("description".to_string(), d.to_string());
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pypi_info() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{
            "info": {"name": "requests", "version": "2.31.0", "summary": "Python HTTP for Humans."}
        }"#,
        )
        .unwrap();
        let p = parse_pypi(&json, "requests", "pip");
        assert_eq!(p.name, "requests");
        assert_eq!(p.backend, "pip");
        assert_eq!(p.version.as_deref(), Some("2.31.0"));
        assert_eq!(
            p.properties.get("description").map(String::as_str),
            Some("Python HTTP for Humans.")
        );
    }

    #[test]
    fn pypi_falls_back_to_query_name() {
        let json: serde_json::Value = serde_json::from_str(r#"{"info": {}}"#).unwrap();
        let p = parse_pypi(&json, "somepkg", "pip");
        assert_eq!(p.name, "somepkg");
        assert!(p.version.is_none());
    }

    /// The backend tag comes from the row asking, not from the word `pip` — the same reason
    /// `node_registry` takes one and three Node managers share it.
    #[test]
    fn the_result_is_tagged_with_the_backend_that_asked() {
        let json: serde_json::Value =
            serde_json::from_str(r#"{"info": {"name": "ruff", "version": "0.16.2"}}"#).unwrap();
        assert_eq!(parse_pypi(&json, "ruff", "uv").backend, "uv");
    }
}
