use crate::core::Package;

/// Parse apk list --installed output
pub fn parse_apk_list(output: &str, backend: &str) -> Vec<Package> {
    output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if !parts.is_empty() {
                let (name, version) = parse_apk_name_version(parts[0]);
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

fn parse_apk_name_version(s: &str) -> (String, Option<String>) {
    let parts: Vec<&str> = s.rsplitn(3, '-').collect();
    
    if parts.len() >= 2 {
        let potential_version = parts[1];
        if potential_version.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            let version = format!("{}-{}", parts[1], parts[0]);
            let name = parts[2..].join("-");
            return (if name.is_empty() { parts[1].to_string() } else { name }, Some(version));
        }
    }
    
    (s.to_string(), None)
}
