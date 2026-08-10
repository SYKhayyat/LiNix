// Shared package-search helper for the Node.js managers (npm / pnpm / yarn).
//
// None of these ship a reliable CLI search: `npm search` is slow and output-unstable,
// `pnpm search` does not exist, and `yarn search` was removed in Yarn 2+ (Berry). They
// all resolve from the same npm registry, so we query the public registry search API
// directly over HTTP and tag results with the calling backend's name.

use crate::core::{Error, Package, Result};
use once_cell::sync::OnceCell;
use std::time::Duration;

/// Process-wide HTTP timeout for backend search clients, seeded from
/// `Config::network_timeout_secs` at startup. Defaults to 15s if never set.
static HTTP_TIMEOUT_SECS: OnceCell<u64> = OnceCell::new();

/// Set the global HTTP timeout (called once during startup). Subsequent calls are no-ops.
pub fn set_http_timeout(secs: u64) {
    let _ = HTTP_TIMEOUT_SECS.set(secs.max(1));
}

pub fn http_timeout() -> Duration {
    Duration::from_secs(*HTTP_TIMEOUT_SECS.get().unwrap_or(&15))
}

/// Query the npm registry search endpoint and return up to `size` matches, tagged with
/// `backend` (e.g. "npm", "pnpm", "yarn"). Network/parse failures surface as `Err`.
pub async fn registry_search(query: &str, backend: &str, size: usize) -> Result<Vec<Package>> {
    // Shared pool: npm, pnpm and yarn all route here, so one `linix search` used to build this
    // client three times and hand registry.npmjs.org three fresh TLS handshakes.
    let client = crate::core::http::api("linix-manager", http_timeout().as_secs())?;

    let url = "https://registry.npmjs.org/-/v1/search";
    let res = client
        .get(url)
        .query(&[("text", query), ("size", &size.to_string())])
        .send()
        .await
        .map_err(Error::from)?;

    if !res.status().is_success() {
        return Err(Error::Other(format!(
            "npm registry search error: {}",
            res.status()
        )));
    }

    let json: serde_json::Value = res.json().await.map_err(Error::from)?;
    Ok(parse_npm_registry(&json, backend))
}

/// Parse the npm registry search response:
/// `{ "objects": [ { "package": { name, version, description } }, ... ] }`.
pub fn parse_npm_registry(json: &serde_json::Value, backend: &str) -> Vec<Package> {
    let mut results = Vec::new();
    if let Some(objects) = json.get("objects").and_then(|o| o.as_array()) {
        for obj in objects {
            let pkg = &obj["package"];
            let Some(name) = pkg["name"].as_str() else {
                continue;
            };
            let mut p = Package::new(name, backend);
            if let Some(v) = pkg["version"].as_str() {
                p.version = Some(v.to_string());
            }
            if let Some(desc) = pkg["description"].as_str() {
                p.properties
                    .insert("description".to_string(), desc.to_string());
            }
            results.push(p);
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_npm_registry_objects() {
        let json: serde_json::Value = serde_json::from_str(r#"{
            "objects": [
                {"package": {"name": "express", "version": "4.18.2", "description": "web framework"}},
                {"package": {"name": "@types/node", "version": "20.0.0"}}
            ],
            "total": 2
        }"#).unwrap();
        let pkgs = parse_npm_registry(&json, "npm");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "express");
        assert_eq!(pkgs[0].version.as_deref(), Some("4.18.2"));
        assert_eq!(
            pkgs[0].properties.get("description").map(String::as_str),
            Some("web framework")
        );
        // scoped name preserved; missing description is fine
        assert_eq!(pkgs[1].name, "@types/node");
        assert!(!pkgs[1].properties.contains_key("description"));
    }

    #[test]
    fn npm_registry_empty_objects() {
        let json: serde_json::Value = serde_json::from_str(r#"{"objects":[]}"#).unwrap();
        assert!(parse_npm_registry(&json, "pnpm").is_empty());
    }
}
