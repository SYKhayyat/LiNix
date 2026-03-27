use crate::core::Package;

/// Parse pacman -Q output
pub fn parse_pacman_list(output: &str, backend: &str) -> Vec<Package> {
    output
        .lines()
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

/// Parse pacman -Ss output
pub fn parse_pacman_search(output: &str, backend: &str) -> Vec<Package> {
    let mut packages = Vec::new();
    let mut lines = output.lines().peekable();
    
    while let Some(line) = lines.next() {
        if line.starts_with(' ') {
            continue;
        }
        
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let name_part = parts[0];
            let name = name_part.split('/').last().unwrap_or(name_part);
            
            let description = lines.peek()
                .filter(|l| l.starts_with("    "))
                .map(|l| l.trim().to_string());
            
            if description.is_some() {
                lines.next();
            }
            
            packages.push(Package {
                name: name.to_string(),
                version: Some(parts[1].to_string()),
                backend: backend.to_string(),
                description,
                repository: None,
                size: None,
            });
        }
    }
    
    packages
}
