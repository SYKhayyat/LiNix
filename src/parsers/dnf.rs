use crate::core::Package;
use crate::parsers::utils::sanitize;

/// Standard RPM query parser used by DNF and Zypper.
/// Command: rpm -qa --queryformat '%{NAME}|%{VERSION}\n'
/// Expected input format: "package-name|1.2.3-r1"
pub fn parse_rpm_qa(output: &str, backend: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|l| {
            let (name, ver) = l.split_once('|')?;
            Some(Package::with_version(name.trim(), ver.trim(), backend))
        })
        .collect()
}

/// Parses the output of 'dnf search'.
/// Expected input format: "package-name.x86_64 : human readable description"
pub fn parse_dnf_search(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let (name_part, desc) = line.split_once(" : ")?;
            // DNF usually includes the architecture in the search name (e.g., .noarch or .x86_64)
            let name = name_part.split('.').next()?.trim();
            let mut p = Package::new(name, "dnf");
            p.properties.insert("description".into(), desc.trim().to_string());
            Some(p)
        })
        .collect()
}

/// Parses the table-based output of 'zypper search'.
/// Zypper output includes status indicators like 'i+' for installed.
pub fn parse_zypper_search(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        // Zypper search output has several header lines
        .skip_while(|l| !l.contains("---"))
        .skip(1)
        .filter_map(|line| {
            // Table format: S | Name | Summary | Type
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() >= 3 {
                let status = parts[0].trim();
                let name = parts[1].trim();
                let summary = parts[2].trim();
                
                let mut p = Package::new(name, "zypper");
                p.properties.insert("summary".into(), summary.to_string());
                p.properties.insert("status_raw".into(), status.to_string());
                
                // If status contains 'i', it's already installed
                if status.contains('i') {
                    p.properties.insert("installed".into(), "true".into());
                }
                
                Some(p)
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rpm_qa_parsing() {
        let input = "kernel|6.3.5\ngit|2.40.1\n";
        let res = parse_rpm_qa(input, "dnf");
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "kernel");
        assert_eq!(res[1].version, Some("2.40.1".into()));
    }

    #[test]
    fn test_dnf_search_parsing() {
        let input = "htop.x86_64 : Interactive process viewer\npython3.noarch : Python programming language\n";
        let res = parse_dnf_search(input);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "htop");
        assert_eq!(res[0].properties.get("description").unwrap(), "Interactive process viewer");
    }
}