use crate::core::Package;

/// Parse scoop list output
pub fn parse_scoop_list(output: &str, backend: &str) -> Vec<Package> {
    output
        .lines()
        .skip(2) // Skip headers
        .filter_map(|line| {
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
            } else {
                None
            }
        })
        .collect()
}
