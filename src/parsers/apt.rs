use crate::core::Package;
use crate::parsers::utils::sanitize;

/// Parses output from the debian/ubuntu package database query.
/// Command: dpkg-query -W -f='${Package} ${Version}\n'
/// Expected input format: "curl 7.81.0-1ubuntu1.16"
pub fn parse_list(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let (name, ver) = line.split_once(' ')?;
            Some(Package::with_version(name.trim(), ver.trim(), "apt"))
        })
        .collect()
}

/// Parses the output of 'apt-cache search' for remote package discovery.
/// Expected input format: "package-name - human readable description"
pub fn parse_search(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            // Apt-cache search usually uses " - " as a separator between name and description
            let (name, desc) = line.split_once(" - ")?;
            let mut p = Package::new(name.trim(), "apt");
            p.properties
                .insert("description".into(), desc.trim().to_string());
            Some(p)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apt_list_parsing() {
        let input = "bash 5.1-6ubuntu1\ncoreutils 8.32-4.1ubuntu1\n";
        let res = parse_list(input);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "bash");
        assert_eq!(res[0].version, Some("5.1-6ubuntu1".into()));
    }

    #[test]
    fn test_apt_search_parsing() {
        let input = "htop - interactive processes viewer\nvim - Vi IMproved - enhanced vi editor\n";
        let res = parse_search(input);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "htop");
        assert_eq!(
            res[0].properties.get("description").unwrap(),
            "interactive processes viewer"
        );
    }
}
