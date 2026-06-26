use crate::core::Package;
use crate::parsers::utils::sanitize;

/// A generic parser for backends that return a simple space-separated list.
/// Format: "package-name version" or just "package-name"
/// Used by backends like 'apk' or internal search utilities.
pub fn parse_simple_list(output: &str, backend: &str) -> Vec<Package> {
    sanitize(output).lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 { 
                Some(Package::with_version(parts[0], parts[1], backend)) 
            }
            else if parts.len() == 1 { 
                Some(Package::new(parts[0], backend)) 
            }
            else { 
                None 
            }
        }).collect()
}

/// Parses a list where the version is attached to the name via a dash.
/// Example: "package-name-1.2.3-r1" -> Name: "package-name", Version: "1.2.3-r1"
/// Common in older RPM-based systems or Alpine (APK) info commands.
pub fn parse_dash_version_list(output: &str, backend: &str) -> Vec<Package> {
    sanitize(output).lines().map(|line| {
        // We assume the last two parts after splitting by dash are part of the version (version-revision)
        let parts: Vec<&str> = line.rsplitn(3, '-').collect();
        if parts.len() >= 3 {
            let name = parts[2].to_string();
            let version = format!("{}-{}", parts[1], parts[0]);
            Package::with_version(&name, &version, backend)
        } else {
            Package::new(line, backend)
        }
    }).collect()
}

/// A strict CSV-style parser for backends that support delimited output.
pub fn parse_delimited(output: &str, delimiter: char, backend: &str) -> Vec<Package> {
    sanitize(output).lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split(delimiter).collect();
            if parts.len() >= 2 {
                Some(Package::with_version(parts[0].trim(), parts[1].trim(), backend))
            } else if !parts[0].is_empty() {
                Some(Package::new(parts[0].trim(), backend))
            } else {
                None
            }
        })
        .collect()
}