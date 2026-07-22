use crate::core::Package;
use crate::parsers::utils::sanitize;
use serde_json::Value;

/// Parses output from various Nix commands.
/// Supports modern JSON (nix profile list --json) and legacy nix-env formats.
pub fn parse_list(output: &str) -> Vec<Package> {
    let clean = sanitize(output);
    if clean.is_empty() {
        return vec![];
    }

    if let Ok(json) = serde_json::from_str::<Value>(&clean) {
        let mut packages = Vec::new();

        // Handle 'nix profile list --json' structure.
        if let Some(elements) = json.get("elements").and_then(|e| e.as_array()) {
            for (i, el) in elements.iter().enumerate() {
                let attr_path = el
                    .get("attrPath")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");

                let name = attr_path.split('.').next_back().unwrap_or(attr_path);
                let mut p = Package::new(name, "nix");

                // `nix profile remove` addresses entries by index, not name, so the index
                // and full attr path must survive parsing or a removal cannot be issued.
                p.properties.insert("index".into(), i.to_string());
                p.properties
                    .insert("full_attr".into(), attr_path.to_string());

                if let Some(store_paths) = el.get("storePaths").and_then(|a| a.as_array()) {
                    if let Some(first_path) = store_paths.first().and_then(|p| p.as_str()) {
                        p.properties
                            .insert("store_path".into(), first_path.to_string());
                    }
                }

                packages.push(p);
            }
            return packages;
        }

        // Handle 'nix-env -q --json' structure
        if let Some(obj) = json.as_object() {
            for (attr, data) in obj {
                let ver = data.get("version").and_then(|v| v.as_str()).unwrap_or("");
                packages.push(Package::with_version(attr, ver, "nix"));
            }
            return packages;
        }
    }

    // Fallback: `nix-env -q` text, "name-version".
    clean
        .lines()
        .map(|line| {
            // The version half must start with a digit: `gnu-config` split blind gives
            // name `gnu`, version `config`, and a package under the wrong name can never
            // be matched again. `xbps` and `pkgsrc` already parse this shape that way.
            match crate::parsers::common::split_trailing_version(line.trim()) {
                Some((name, ver)) => Package::with_version(name, &ver, "nix"),
                None => Package::new(line.trim(), "nix"),
            }
        })
        .collect()
}

/// Parses output from 'nix-env -qa' or 'nix search' results.
pub fn parse_search(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            // nix-env -qa format: "attribute-path  name-version"
            let parts: Vec<&str> = line.split_whitespace().collect();
            let full_name = parts.first()?;

            match crate::parsers::common::split_trailing_version(full_name) {
                Some((name, ver)) => Some(Package::with_version(name, &ver, "nix")),
                None => Some(Package::new(*full_name, "nix")),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nix_text_list_parsing() {
        let input = "htop-3.2.2\npython3-3.10.11\n";
        let res = parse_list(input);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "htop");
        assert_eq!(res[0].version, Some("3.2.2".into()));
    }

    #[test]
    fn test_nix_profile_json_parsing() {
        let input = r#"{
            "elements": [
                {
                    "attrPath": "legacyPackages.x86_64-linux.hello",
                    "storePaths": ["/nix/store/abc-hello-2.12.1"]
                }
            ]
        }"#;
        let res = parse_list(input);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].name, "hello");
        assert_eq!(res[0].properties.get("index").unwrap(), "0");
    }
}
