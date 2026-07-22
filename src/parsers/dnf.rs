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

/// The architectures dnf appends to a name. Only these are stripped: taking everything
/// before the first `.` turned `python3.12.x86_64` into `python3`, and a resolver comparing
/// the parsed name against what you asked for then never matched.
const RPM_ARCHES: &[&str] = &[
    "x86_64", "i686", "i386", "noarch", "aarch64", "armv7hl", "ppc64le", "s390x", "src",
];

/// Parses the output of `dnf search`.
///
/// Two shapes, because dnf5 (Fedora 41+) rewrote the output and dnf4 is still what RHEL and
/// older Fedora run: `name.arch : summary` and `name.arch<TAB>summary`. Both are the same
/// three facts with a different separator, so one pass reads either — and a header line, which
/// has neither separator, is dropped by the same rule rather than by a list of known headers.
pub fn parse_dnf_search(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let (name_part, desc) = line
                .split_once('\t')
                .or_else(|| line.split_once(" : "))?;
            let name = strip_rpm_arch(name_part.trim());
            if name.is_empty() {
                return None;
            }
            let mut p = Package::new(name, "dnf");
            p.properties
                .insert("description".into(), desc.trim().to_string());
            Some(p)
        })
        .collect()
}

fn strip_rpm_arch(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((base, arch)) if RPM_ARCHES.contains(&arch) => base,
        _ => name,
    }
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
        assert_eq!(
            res[0].properties.get("description").unwrap(),
            "Interactive process viewer"
        );
    }

    #[test]
    fn dnf5_tab_separated_output_is_read() {
        // Fedora 41 ships dnf5, which prints a tab and indents each row, and interleaves
        // "Matched fields:" headers. Reading only dnf4's ` : ` made dnf return nothing at
        // all on Fedora — so every unpinned name skipped the system manager and landed on
        // whichever language registry happened to publish the name.
        let input = "Updating and loading repositories:\nRepositories loaded.\n\
                     Matched fields: name (exact)\n \
                     jq.x86_64\tCommand-line JSON processor\n \
                     R-jqr.x86_64\tClient for 'jq', a 'JSON' Processor\n";
        let res = parse_dnf_search(input);
        assert_eq!(res.len(), 2, "headers must not become packages");
        assert_eq!(res[0].name, "jq");
        assert_eq!(res[1].name, "R-jqr");
        assert_eq!(
            res[0].properties.get("description").unwrap(),
            "Command-line JSON processor"
        );
    }

    #[test]
    fn only_a_real_architecture_is_stripped_from_a_name() {
        // `python3.12.x86_64` is python3.12, not python3: cutting at the first dot renamed
        // the package, and a resolver matching on the name then never found it.
        let res = parse_dnf_search("python3.12.x86_64\tPython\nfoo.bar\tNot an arch\n");
        assert_eq!(res[0].name, "python3.12");
        assert_eq!(res[1].name, "foo.bar");
    }
}
