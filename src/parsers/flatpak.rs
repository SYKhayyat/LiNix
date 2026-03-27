use crate::core::Package;

/// Parse flatpak list output
pub fn parse_flatpak_list(output: &str, backend: &str) -> Vec<Package> {
    output
        .lines()
        .skip(1) // Skip header
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 2 {
                Some(Package {
                    name: parts[0].trim().to_string(),
                    version: Some(parts[1].trim().to_string()),
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
