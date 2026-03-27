use crate::core::Package;

/// Parse dnf list installed output
pub fn parse_dnf_list(output: &str, backend: &str) -> Vec<Package> {
    output
        .lines()
        .skip(1) // Skip header
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let name = parts[0].split('.').next()?;
                Some(Package {
                    name: name.to_string(),
                    version: Some(parts[1].to_string()),
                    backend: backend.to_string(),
                    description: None,
                    repository: parts.get(2).map(|s| s.to_string()),
                    size: None,
                })
            } else {
                None
            }
        })
        .collect()
}
