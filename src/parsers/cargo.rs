use crate::core::Package;

/// Parse cargo install --list output
pub fn parse_cargo_list(output: &str, backend: &str) -> Vec<Package> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with('-') {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    return Some(Package {
                        name: parts[0].to_string(),
                        version: Some(parts[1].trim_matches(':').to_string()),
                        backend: backend.to_string(),
                        description: None,
                        repository: None,
                        size: None,
                    });
                }
            }
            None
        })
        .collect()
}
