// Search capability for pip. The legacy `pip search` command was disabled upstream
// (PyPI removed the XML-RPC search endpoint due to abuse) and PyPI exposes no public
// search API. We therefore implement `search` as an EXACT-NAME lookup against the
// PyPI JSON API (https://pypi.org/pypi/<name>/json): it returns the package if it
// exists, or an empty result otherwise. This is a documented limitation — pip search
// is name resolution, not full-text discovery.

use crate::backends::node_registry::http_timeout;
use crate::core::{Error, Package, Result, Searchable};
use async_trait::async_trait;

pub struct PipSearchable;

#[async_trait]
impl Searchable for PipSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
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
        Ok(vec![parse_pypi(&json, name)])
    }
}

/// Parse a PyPI JSON document (`https://pypi.org/pypi/<name>/json`) into a `Package`.
/// `fallback_name` is used if the document omits `info.name`.
fn parse_pypi(json: &serde_json::Value, fallback_name: &str) -> Package {
    let info = &json["info"];
    let pkg_name = info["name"].as_str().unwrap_or(fallback_name);
    let mut p = Package::new(pkg_name, "pip");
    if let Some(v) = info["version"].as_str() {
        p.version = Some(v.to_string());
    }
    if let Some(d) = info["summary"].as_str() {
        if !d.is_empty() {
            p.properties.insert("description".into(), d.to_string());
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
        let p = parse_pypi(&json, "requests");
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
        let p = parse_pypi(&json, "somepkg");
        assert_eq!(p.name, "somepkg");
        assert!(p.version.is_none());
    }
}
