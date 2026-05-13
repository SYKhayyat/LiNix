use crate::core::Package;
use crate::parsers::utils::sanitize;

/// Parses the output from the 'mas list' command.
/// 'mas' (Mac App Store CLI) output format: "identifier Name (Version)"
/// Example: "497799835 Xcode (14.3.1)"
pub fn parse_mas_list(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            // We use rsplit_once to isolate the version part inside parentheses
            let (id_name, ver_part) = line.rsplit_once(' ')?;
            // Split the identifier and the human-readable name
            let (id, name) = id_name.split_once(' ')?;
            
            let mut p = Package::with_version(
                id.trim(), 
                ver_part.trim_matches(|c| c == '(' || c == ')'), 
                "mas"
            );
            
            // Store the human-readable name in properties as 'mas' packages 
            // are primary identified by their numeric ID.
            p.properties.insert("human_name".into(), name.trim().to_string());
            Some(p)
        })
        .collect()
}

/// Parses the output from the 'mas search' command.
/// Expected format: "identifier Name"
pub fn parse_mas_search(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 { return None; }
            
            // First part is the App Store numeric ID
            let id = parts[0];
            // The rest is the App name
            let name = parts[1..].join(" ");
            
            let mut p = Package::new(id, "mas");
            p.properties.insert("human_name".into(), name);
            Some(p)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mas_list_parsing() {
        let input = "497799835 Xcode (14.3.1)\n1284863847 Unarchiver (3.35.2)\n";
        let res = parse_mas_list(input);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "497799835");
        assert_eq!(res[0].version, Some("14.3.1".into()));
        assert_eq!(res[0].properties.get("human_name").unwrap(), "Xcode");
    }

    #[test]
    fn test_mas_search_parsing() {
        let input = "497799835 Xcode\n1284863847 The Unarchiver\n";
        let res = parse_mas_search(input);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "497799835");
        assert_eq!(res[1].properties.get("human_name").unwrap(), "The Unarchiver");
    }
}