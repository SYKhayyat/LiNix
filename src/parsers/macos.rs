use crate::core::Package;
use crate::parsers::utils::sanitize;

/// Parses the output from the 'mas list' command.
/// 'mas' (Mac App Store CLI) output format: "identifier Name (Version)"
/// Example: "497799835 Xcode (14.3.1)"
pub fn parse_mas_list(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let (id_name, ver_part) = line.rsplit_once(' ')?;
            let (id, name) = id_name.split_once(' ')?;

            let mut p = Package::with_version(
                id.trim(),
                ver_part.trim_matches(|c| c == '(' || c == ')'),
                "mas",
            );

            // Store the human-readable name in properties as 'mas' packages
            // are primary identified by their numeric ID.
            p.properties
                .insert("human_name".into(), name.trim().to_string());
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
            if parts.len() < 2 {
                return None;
            }

            let id = parts[0];
            let name = parts[1..].join(" ");

            let mut p = Package::new(id, "mas");
            p.properties.insert("human_name".into(), name);
            Some(p)
        })
        .collect()
}

/// Parses the output of `port installed` (MacPorts).
/// Format:
///   The following ports are currently installed:
///     git @2.39.0_0+doc (active)
///     wget @1.21.3_0
/// The leading `@` marks the version; a trailing `+variant` and `(active)` tag are
/// dropped so the version is clean (e.g. "2.39.0_0").
pub fn parse_macports_installed(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            // Skip the header and any line that doesn't carry a "@version" token.
            if line.is_empty() || !line.contains('@') {
                return None;
            }
            let mut parts = line.split_whitespace();
            let name = parts.next()?;
            let ver_token = parts.find(|t| t.starts_with('@'))?;
            let version = clean_macports_version(ver_token);
            Some(Package::with_version(name, &version, "macports"))
        })
        .collect()
}

/// Parses the output of `port search <query>` (MacPorts).
/// Format: "git @2.39.0 (devel, net): Distributed version control system".
pub fn parse_macports_search(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            // Result rows contain a " @version"; the trailing "Found N ports." does not.
            if !line.contains(" @") {
                return None;
            }
            let mut parts = line.split_whitespace();
            let name = parts.next()?;
            let ver_token = parts.find(|t| t.starts_with('@'))?;
            let version = clean_macports_version(ver_token);
            let mut p = Package::with_version(name, &version, "macports");
            if let Some((_, desc)) = line.split_once("): ") {
                let desc = desc.trim();
                if !desc.is_empty() {
                    p.properties.insert("description".into(), desc.to_string());
                }
            }
            Some(p)
        })
        .collect()
}

/// Strips MacPorts decoration from a `@version` token: drops the leading `@` and any
/// `+variant` suffix, leaving the bare version (e.g. "@2.39.0_0+doc" -> "2.39.0_0").
fn clean_macports_version(token: &str) -> String {
    let no_at = token.trim_start_matches('@');
    match no_at.find('+') {
        Some(idx) => no_at[..idx].to_string(),
        None => no_at.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_macports_installed_parsing() {
        let input = "The following ports are currently installed:\n  \
                     git @2.39.0_0+doc (active)\n  wget @1.21.3_0\n";
        let res = parse_macports_installed(input);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "git");
        assert_eq!(res[0].version.as_deref(), Some("2.39.0_0"));
        assert_eq!(res[1].name, "wget");
        assert_eq!(res[1].version.as_deref(), Some("1.21.3_0"));
        // The header line must not become a package.
        assert!(res.iter().all(|p| p.name != "The"));
    }

    #[test]
    fn test_macports_search_parsing() {
        let input = "git @2.39.0 (devel, net): Distributed version control system\n\
                     Found 1 ports.\n";
        let res = parse_macports_search(input);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].name, "git");
        assert_eq!(res[0].version.as_deref(), Some("2.39.0"));
        assert_eq!(
            res[0].properties.get("description").map(String::as_str),
            Some("Distributed version control system")
        );
    }

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
        assert_eq!(
            res[1].properties.get("human_name").unwrap(),
            "The Unarchiver"
        );
    }
}
