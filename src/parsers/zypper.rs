use crate::core::Package;

/// Parse zypper search --installed-only output
pub fn parse_zypper_list(output: &str, backend: &str) -> Vec<Package> {
    output
        .lines()
        .skip(4) // Skip headers
        .filter(|line| line.starts_with("i"))
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() >= 4 {
                Some(Package {
                    name: parts[1].trim().to_string(),
                    version: Some(parts[3].trim().to_string()),
                    backend: backend.to_string(),
                    description: None,
                    repository: parts.get(5).map(|s| s.trim().to_string()),
                    size: None,
                })
            } else {
                None
            }
        })
        .collect()
}
