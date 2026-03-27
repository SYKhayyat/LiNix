use crate::core::Package;

/// Parse pip list --format=json output
pub fn parse_pip_list(output: &str, backend: &str) -> Vec<Package> {
    let packages: Vec<serde_json::Value> = serde_json::from_str(output).unwrap_or_default();
    
    packages
        .into_iter()
        .filter_map(|pkg| {
            let name = pkg.get("name")?.as_str()?.to_string();
            let version = pkg.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
            
            Some(Package {
                name,
                version,
                backend: backend.to_string(),
                description: None,
                repository: None,
                size: None,
            })
        })
        .collect()
}
