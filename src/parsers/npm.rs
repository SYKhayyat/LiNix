use crate::core::Package;

/// Parse npm list -g --json output
pub fn parse_npm_list(output: &str, backend: &str) -> Vec<Package> {
    let json: serde_json::Value = serde_json::from_str(output).unwrap_or_default();
    
    let mut packages = Vec::new();
    
    if let Some(dependencies) = json.get("dependencies").and_then(|d| d.as_object()) {
        for (name, data) in dependencies {
            let version = data.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
            
            packages.push(Package {
                name: name.clone(),
                version,
                backend: backend.to_string(),
                description: None,
                repository: None,
                size: None,
            });
        }
    }
    
    packages
}
