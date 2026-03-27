use crate::core::Package;

/// Parse gem list --local output
pub fn parse_gem_list(output: &str, backend: &str) -> Vec<Package> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with("***") {
                return None;
            }

            let parts: Vec<&str> = line.splitn(2, '(').collect();
            if parts.len() >= 2 {
                let name = parts[0].trim().to_string();
                let version = parts[1]
                    .trim_end_matches(')')
                    .split(',')
                    .next()
                    .map(|v| v.trim().to_string());

                Some(Package {
                    name,
                    version,
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
