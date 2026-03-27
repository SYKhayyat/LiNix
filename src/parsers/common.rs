use crate::core::Package;
use once_cell::sync::Lazy;
use regex::Regex;

/// Parse a simple "name version" format
pub fn parse_simple_list(output: &str, backend: &str) -> Vec<Package> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                Some(Package {
                    name: parts[0].to_string(),
                    version: Some(parts[1].to_string()),
                    backend: backend.to_string(),
                    description: None,
                    repository: None,
                    size: None,
                })
            } else if parts.len() == 1 {
                Some(Package {
                    name: parts[0].to_string(),
                    version: None,
                    backend: backend.to_string(),
                    description: None,
                    repository: None,
                    size: None,
                })
            } else {
                None
            }
        })
        .collect()
}

/// Parse a "name|version" format (used by chocolatey)
pub fn parse_pipe_separated(output: &str, backend: &str) -> Vec<Package> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }

            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() >= 2 {
                Some(Package {
                    name: parts[0].trim().to_string(),
                    version: Some(parts[1].trim().to_string()),
                    backend: backend.to_string(),
                    description: parts.get(2).map(|s| s.trim().to_string()),
                    repository: None,
                    size: None,
                })
            } else {
                None
            }
        })
        .collect()
}

/// Parse a "name - description" format (used by apt search)
pub fn parse_dash_separated(output: &str, backend: &str) -> Vec<Package> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }

            let parts: Vec<&str> = line.splitn(2, " - ").collect();
            if !parts.is_empty() {
                Some(Package {
                    name: parts[0].trim().to_string(),
                    version: None,
                    backend: backend.to_string(),
                    description: parts.get(1).map(|s| s.trim().to_string()),
                    repository: None,
                    size: None,
                })
            } else {
                None
            }
        })
        .collect()
}

/// Parse a tab-separated format
pub fn parse_tab_separated(output: &str, backend: &str, skip_header: bool) -> Vec<Package> {
    output
        .lines()
        .skip(if skip_header { 1 } else { 0 })
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }

            let parts: Vec<&str> = line.split('\t').collect();
            if !parts.is_empty() {
                Some(Package {
                    name: parts[0].trim().to_string(),
                    version: parts.get(1).map(|s| s.trim().to_string()),
                    backend: backend.to_string(),
                    description: parts.get(2).map(|s| s.trim().to_string()),
                    repository: None,
                    size: None,
                })
            } else {
                None
            }
        })
        .collect()
}

/// Parse JSON array format (used by pip, npm)
pub fn parse_json_array(output: &str, backend: &str) -> Vec<Package> {
    let packages_json: Vec<serde_json::Value> = match serde_json::from_str(output) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    packages_json
        .into_iter()
        .filter_map(|pkg| {
            let name = pkg.get("name")?.as_str()?.to_string();
            let version = pkg
                .get("version")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let description = pkg
                .get("description")
                .and_then(|d| d.as_str())
                .map(|s| s.to_string());

            Some(Package {
                name,
                version,
                backend: backend.to_string(),
                description,
                repository: None,
                size: None,
            })
        })
        .collect()
}

/// Extract name and version from "name@version" format
pub fn parse_at_version(s: &str) -> (String, Option<String>) {
    let parts: Vec<&str> = s.rsplitn(2, '@').collect();
    if parts.len() == 2 {
        (parts[1].to_string(), Some(parts[0].to_string()))
    } else {
        (s.to_string(), None)
    }
}

/// Extract name and version from "name-version-release" format (APK style)
pub fn parse_dash_version(s: &str) -> (String, Option<String>) {
    static VERSION_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(.+)-(\d+[.\d]*.*?)$").unwrap());

    if let Some(caps) = VERSION_REGEX.captures(s) {
        let name = caps
            .get(1)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        let version = caps.get(2).map(|m| m.as_str().to_string());
        (name, version)
    } else {
        (s.to_string(), None)
    }
}

/// Parse key-value output (like "Name: value")
pub fn parse_key_value(output: &str, keys: &[&str]) -> std::collections::HashMap<String, String> {
    let mut result = std::collections::HashMap::new();

    for line in output.lines() {
        let line = line.trim();
        for key in keys {
            let prefix = format!("{}: ", key);
            if let Some(value) = line.strip_prefix(&prefix) {
                result.insert(key.to_string(), value.trim().to_string());
            } else {
                let prefix_alt = format!("{}:", key);
                if let Some(value) = line.strip_prefix(&prefix_alt) {
                    result.insert(key.to_string(), value.trim().to_string());
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_list() {
        let output = "vim 8.2\ncurl 7.81\n";
        let packages = parse_simple_list(output, "test");
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].name, "vim");
        assert_eq!(packages[0].version, Some("8.2".to_string()));
    }

    #[test]
    fn test_parse_pipe_separated() {
        let output = "vim|8.2\ncurl|7.81\n";
        let packages = parse_pipe_separated(output, "test");
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].name, "vim");
    }

    #[test]
    fn test_parse_at_version() {
        let (name, version) = parse_at_version("package@1.2.3");
        assert_eq!(name, "package");
        assert_eq!(version, Some("1.2.3".to_string()));

        let (name, version) = parse_at_version("package");
        assert_eq!(name, "package");
        assert_eq!(version, None);
    }

    #[test]
    fn test_parse_key_value() {
        let output = "Name: vim\nVersion: 8.2\nDescription: Text editor";
        let result = parse_key_value(output, &["Name", "Version", "Description"]);
        assert_eq!(result.get("Name"), Some(&"vim".to_string()));
        assert_eq!(result.get("Version"), Some(&"8.2".to_string()));
    }
}
