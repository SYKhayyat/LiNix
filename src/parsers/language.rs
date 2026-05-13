use crate::core::Package;
use crate::parsers::utils::sanitize;
use serde_json::Value;

/// Multi-backend dispatcher for language-specific package manager formats.
/// Follows the Open/Closed Principle: new language tools can be added by 
/// implementing a specific parser function and adding it to this dispatcher.
pub fn parse_installed(backend: &str, output: &str) -> Vec<Package> {
    let clean = sanitize(output);
    if clean.is_empty() { return vec![]; }

    match backend {
        "npm" | "pnpm" | "bun" => parse_npm_style_json(&clean, backend),
        "pip" => parse_pip_json(&clean),
        "pipx" => parse_pipx_json(&clean),
        "cargo" => parse_cargo_list(&clean),
        "yarn" => parse_yarn_list(&clean),
        "gem" => parse_gem_list(&clean),
        "composer" => parse_composer_json(&clean),
        "go" => parse_go_list(&clean),
        _ => vec![],
    }
}

/// Dispatches search result parsing for language-specific repositories.
pub fn parse_search(backend: &str, output: &str) -> Vec<Package> {
    let clean = sanitize(output);
    match backend {
        "cargo" => parse_cargo_search(&clean),
        "gem" => parse_gem_search(&clean),
        "composer" => parse_composer_search(&clean),
        _ => vec![], 
    }
}

/// Handles JSON dependencies for NPM, PNPM, and Bun global lists.
fn parse_npm_style_json(output: &str, backend: &str) -> Vec<Package> {
    let json: Value = serde_json::from_str(output).unwrap_or_default();
    let mut res = vec![];
    if let Some(deps) = json.get("dependencies").and_then(|d| d.as_object()) {
        for (name, val) in deps {
            let version = val.get("version").and_then(|v| v.as_str()).unwrap_or("unknown");
            res.push(Package::with_version(name, version, backend));
        }
    }
    res
}

/// Parses the flat JSON array output of `pip list --format=json`.
fn parse_pip_json(output: &str) -> Vec<Package> {
    let json: Value = serde_json::from_str(output).unwrap_or_default();
    json.as_array().unwrap_or(&vec![]).iter().filter_map(|p| {
        let name = p.get("name")?.as_str()?;
        let ver = p.get("version")?.as_str()?;
        Some(Package::with_version(name, ver, "pip"))
    }).collect()
}

/// Parses the complex JSON object of `pipx list --json`.
fn parse_pipx_json(output: &str) -> Vec<Package> {
    let json: Value = serde_json::from_str(output).unwrap_or_default();
    let mut res = vec![];
    if let Some(venvs) = json.get("venvs").and_then(|v| v.as_object()) {
        for (name, data) in venvs {
            let ver = data.get("metadata")
                .and_then(|m| m.get("main_package"))
                .and_then(|p| p.get("version"))
                .and_then(|v| v.as_str());
            res.push(Package::with_version(name, ver.unwrap_or("unknown"), "pipx"));
        }
    }
    res
}

/// Parses the formatted text list of `cargo install --list`.
/// Format: "name v1.2.3:"
fn parse_cargo_list(output: &str) -> Vec<Package> {
    output.lines()
        .filter(|l| l.contains(" v") && l.ends_with(':'))
        .filter_map(|l| {
            let parts: Vec<&str> = l.split_whitespace().collect();
            if parts.len() >= 2 {
                Some(Package::with_version(
                    parts[0], 
                    parts[1].trim_matches(|c| c == 'v' || c == ':'), 
                    "cargo"
                ))
            } else { None }
        }).collect()
}

/// Parses the ASCII tree output of `yarn global list`.
fn parse_yarn_list(output: &str) -> Vec<Package> {
    output.lines()
        .filter(|l| l.contains('@') && !l.contains("info"))
        .filter_map(|l| {
            let cleaned = l.trim()
                .trim_start_matches("├── ")
                .trim_start_matches("└── ")
                .trim();
            let (name, ver) = cleaned.rsplit_once('@')?;
            Some(Package::with_version(name, ver, "yarn"))
        }).collect()
}

/// Parses the text output of `gem list --local`.
/// Format: "name (1.2.3, 1.1.0)"
fn parse_gem_list(output: &str) -> Vec<Package> {
    output.lines()
        .filter(|l| !l.is_empty() && !l.starts_with("***"))
        .filter_map(|line| {
            let (name, rest) = line.split_once(' ')?;
            let ver = rest.trim().trim_matches(|c| c == '(' || c == ')').split(',').next()?;
            Some(Package::with_version(name.trim(), ver.trim(), "gem"))
        }).collect()
}

/// Parses the JSON output of `composer global show --format=json`.
fn parse_composer_json(output: &str) -> Vec<Package> {
    let json: Value = serde_json::from_str(output).unwrap_or_default();
    let mut res = vec![];
    if let Some(installed) = json.get("installed").and_then(|i| i.as_array()) {
        for pkg in installed {
            let name = pkg.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let version = pkg.get("version").and_then(|v| v.as_str()).unwrap_or("");
            if !name.is_empty() {
                res.push(Package::with_version(name, version, "composer"));
            }
        }
    }
    res
}

/// Parses simple binary names from Go-related outputs.
fn parse_go_list(output: &str) -> Vec<Package> {
    output.lines()
        .filter(|l| !l.is_empty())
        .map(|l| Package::new(l.trim(), "go"))
        .collect()
}

/// Specialized parser for `cargo search`.
fn parse_cargo_search(output: &str) -> Vec<Package> {
    output.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|l| {
            let (name, _) = l.split_once('=')?;
            Some(Package::new(name.trim(), "cargo"))
        }).collect()
}

/// Specialized parser for `gem search`.
fn parse_gem_search(output: &str) -> Vec<Package> {
    output.lines()
        .filter(|l| !l.is_empty() && !l.starts_with("***"))
        .filter_map(|l| {
            let (name, _) = l.split_once(' ')?;
            Some(Package::new(name.trim(), "gem"))
        }).collect()
}

/// Specialized parser for `composer search`.
fn parse_composer_search(output: &str) -> Vec<Package> {
    let json: Value = serde_json::from_str(output).unwrap_or_default();
    json.as_array().unwrap_or(&vec![]).iter().filter_map(|p| {
        let name = p.get("name")?.as_str()?;
        Some(Package::new(name, "composer"))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cargo_list_parsing() {
        let input = "ripgrep v13.0.0:\nexa v0.10.1:\n";
        let res = parse_cargo_list(input);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "ripgrep");
        assert_eq!(res[1].version, Some("0.10.1".into()));
    }

    #[test]
    fn test_composer_json_parsing() {
        let input = r#"{"installed": [{"name": "laravel/installer", "version": "v4.0.0"}]}"#;
        let res = parse_composer_json(input);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].name, "laravel/installer");
        assert_eq!(res[0].version, Some("v4.0.0".into()));
    }
}