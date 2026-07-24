//! Out-of-tree kernel modules, after a kernel changes (XIII.1).
//!
//! **LiNix builds nothing.** It drives DKMS, which is already on the machine and already knows
//! how to rebuild a module. What LiNix adds is the one thing DKMS cannot know: *that the kernel
//! just changed*, across managers.
//!
//! The distribution's own DKMS hook fires for the distribution's own package manager. LiNix's
//! whole premise is several managers at once — a kernel from `apt` and a driver installed some
//! other way, or a kernel from a manager whose hook nobody wired — so the cross-manager case is
//! the one nothing covers, and it is the one that leaves a machine without its graphics driver
//! or its wifi after the next reboot.
//!
//! **Before the reboot is the whole point.** A module that will not build is recoverable while
//! the running kernel is still the old one; after the reboot it is a machine that does not come
//! back on the network.
//!
//! Pure: recognising a kernel package, reading `dkms status`, and deciding what failed.

/// Whether a package being installed or removed is a kernel.
///
/// Matched on the name across the distributions LiNix drives, because there is no cross-manager
/// "is this a kernel" flag to ask for. Deliberately generous on the `linux-image` /
/// `kernel-core` families and deliberately silent on everything else: a false positive costs a
/// `dkms autoinstall` that finds nothing to do, and a false negative costs a driver.
pub fn is_kernel_package(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    // Arch: `linux`, `linux-lts`, `linux-zen`, `linux-hardened` (+ `-headers`).
    // Debian/Ubuntu: `linux-image-*`, `linux-headers-*`, `linux-generic`.
    // Fedora/RHEL: `kernel`, `kernel-core`, `kernel-modules`, `kernel-devel`.
    // openSUSE: `kernel-default`, `kernel-default-devel`.
    const EXACT: [&str; 6] = ["linux", "kernel", "linux-lts", "linux-zen", "linux-hardened", "linux-generic"];
    const PREFIXES: [&str; 4] = ["linux-image", "linux-headers", "kernel-", "linux-lts"];

    if EXACT.contains(&n.as_str()) {
        return true;
    }
    // `kernel-` would also match `kernel-tools`, which is harmless — see the doc comment: a
    // false positive costs a no-op rebuild, a false negative costs a driver.
    PREFIXES.iter().any(|p| n.starts_with(p))
}

/// The kernel packages a change set touches, sorted and deduplicated.
pub fn kernels_in<'a>(changed: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = changed
        .filter(|n| is_kernel_package(n))
        .map(str::to_string)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// One out-of-tree module DKMS knows about.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Module {
    pub name: String,
    pub version: String,
    /// The kernel this line is about. `dkms status` reports one line per (module, kernel), so
    /// dropping it would collapse "built for the old kernel" and "built for the new one" into
    /// one indistinguishable row — which is exactly the difference this module exists to see.
    pub kernel: String,
    /// What `dkms status` said about it — `installed`, `added`, `built`.
    pub state: String,
}

impl std::fmt::Display for Module {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.name, self.version)
    }
}

/// Parse `dkms status`.
///
/// Two formats are in the wild and both are current, because distributions ship different DKMS
/// versions:
///
/// ```text
/// nvidia, 550.78, 6.8.0-31-generic, x86_64: installed      (<= 2.8)
/// nvidia/550.78, 6.8.0-31-generic, x86_64: installed       (>= 3.0)
/// ```
///
/// A line that does not parse is skipped rather than guessed at: inventing a module out of a
/// warning would have LiNix report a rebuild of something that does not exist.
pub fn parse_status(output: &str) -> Vec<Module> {
    let mut out: Vec<Module> = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Everything after the last `:` is the state; before it, the comma-separated fields.
        let (fields, state) = match line.rsplit_once(':') {
            Some((f, s)) => (f, s.trim().to_string()),
            None => continue,
        };
        let mut parts = fields.split(',').map(str::trim);
        let Some(first) = parts.next() else { continue };

        // `name/version` (DKMS >= 3) or `name` with the version in the next field (<= 2.8).
        let (name, version) = match first.split_once('/') {
            Some((n, v)) => (n.to_string(), v.to_string()),
            None => match parts.next() {
                Some(v) => (first.to_string(), v.to_string()),
                None => continue,
            },
        };
        if name.is_empty() || version.is_empty() {
            continue;
        }
        // The kernel is the field after the version; a line that omits it (an `added` module
        // that has never been built for anything) simply has none.
        let kernel = parts.next().unwrap_or("").to_string();
        let module = Module {
            name,
            version,
            kernel,
            state,
        };
        if !out.contains(&module) {
            out.push(module);
        }
    }
    out.sort();
    out
}

/// The modules DKMS holds that are not installed for any kernel.
///
/// **Asked after `dkms autoinstall`, not before.** A module still sitting at `added` or `built`
/// once autoinstall has run is one that did not make it — the fact worth failing on.
///
/// Deliberately NOT "not installed for the new kernel". LiNix cannot reliably name the release
/// that was just installed: `linux-image-6.8.0-35-generic` carries it and Arch's plain `linux`
/// does not, and the *running* kernel is still the old one at this point — so a check against a
/// release string would be right on one distribution and quietly vacuous on another. This asks
/// a question that is well-defined everywhere.
pub fn not_installed(output: &str) -> Vec<Module> {
    let installed: std::collections::HashSet<String> = parse_status(output)
        .into_iter()
        .filter(|m| m.state == "installed")
        .map(|m| m.name)
        .collect();

    let mut stuck: Vec<Module> = parse_status(output)
        .into_iter()
        .filter(|m| !installed.contains(&m.name))
        .collect();
    stuck.sort();
    stuck.dedup_by(|a, b| a.name == b.name);
    stuck
}

/// The refusal for modules that would not build.
///
/// Loud, and specific about *why now*: the recoverable moment is before the reboot, while the
/// running kernel is still the one whose modules are built.
pub fn failed_to_build(failed: &[Module], kernel: &str) -> String {
    let named: Vec<String> = failed.iter().map(|m| m.to_string()).collect();
    format!(
        "{} out-of-tree kernel module(s) did not build for {}: {}.\n  \
         The running kernel still has them, so this is recoverable right now and will not be \
         after a reboot — a module that is missing then is a graphics driver or a network card \
         that does not come back.\n  \
         Look at /var/lib/dkms/<module>/<version>/build/make.log, or hold the kernel \
         (`@hold` on its line) until the module supports it.",
        failed.len(),
        kernel,
        named.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kernels_of_every_distribution_are_recognised() {
        for name in [
            "linux",
            "linux-lts",
            "linux-zen",
            "linux-hardened",
            "linux-image-6.8.0-31-generic",
            "linux-headers-amd64",
            "linux-generic",
            "kernel",
            "kernel-core",
            "kernel-default",
            "kernel-devel",
        ] {
            assert!(is_kernel_package(name), "`{}` is a kernel package", name);
        }
    }

    /// A false negative costs a driver, so the matcher is generous — but not so generous that
    /// ordinary software trips it.
    #[test]
    fn ordinary_packages_are_not_kernels() {
        for name in ["jq", "ripgrep", "linuxbrew", "vlinux", "nginx", "python3"] {
            assert!(!is_kernel_package(name), "`{}` is not a kernel", name);
        }
    }

    #[test]
    fn kernels_are_picked_out_of_a_change_set_sorted() {
        let changed = ["jq", "linux-image-6.8", "nginx", "linux-headers-6.8", "jq"];
        assert_eq!(
            kernels_in(changed.into_iter()),
            vec!["linux-headers-6.8".to_string(), "linux-image-6.8".to_string()]
        );
    }

    #[test]
    fn nothing_kernel_shaped_is_no_kernel_change() {
        assert!(kernels_in(["jq", "nginx"].into_iter()).is_empty());
    }

    /// Both `dkms status` formats are current, because distributions ship different DKMS
    /// versions. Parsing only one would silently find no modules on half of them — and finding
    /// no modules is indistinguishable from having none.
    #[test]
    fn both_dkms_status_formats_parse() {
        let old = "nvidia, 550.78, 6.8.0-31-generic, x86_64: installed";
        let new = "nvidia/550.78, 6.8.0-31-generic, x86_64: installed";
        for out in [old, new] {
            let mods = parse_status(out);
            assert_eq!(mods.len(), 1, "{:?} from {}", mods, out);
            assert_eq!(mods[0].name, "nvidia");
            assert_eq!(mods[0].version, "550.78");
            assert_eq!(mods[0].state, "installed");
            assert_eq!(mods[0].kernel, "6.8.0-31-generic");
        }
    }

    /// One module built for two kernels is two lines and two rows — the kernel is part of what
    /// the line says, and collapsing it would hide "built for the old one, not the new one".
    #[test]
    fn several_modules_and_kernels_parse() {
        let out = "\
nvidia/550.78, 6.8.0-31-generic, x86_64: installed
nvidia/550.78, 6.8.0-35-generic, x86_64: installed
v4l2loopback/0.12.7, 6.8.0-31-generic, x86_64: installed
";
        let mods = parse_status(out);
        assert_eq!(mods.len(), 3, "{:?}", mods);
    }

    /// A line that is not a module must not become one. `dkms status` prints warnings, and a
    /// phantom module would have LiNix report rebuilding something that does not exist.
    #[test]
    fn noise_does_not_become_a_module() {
        assert!(parse_status("").is_empty());
        assert!(parse_status("Error! Could not locate dkms.conf\n").is_empty());
        assert!(parse_status("just some words\n").is_empty());
    }

    /// A module still at `added` or `built` after autoinstall has run is one that did not make
    /// it — the fact worth failing on.
    #[test]
    fn a_module_that_never_reached_installed_is_reported() {
        let out = "\
nvidia/550.78, 6.8.0-35-generic, x86_64: built
v4l2loopback/0.12.7, 6.8.0-35-generic, x86_64: installed
";
        let stuck = not_installed(out);
        assert_eq!(stuck.len(), 1, "{:?}", stuck);
        assert_eq!(stuck[0].name, "nvidia");
    }

    /// A module installed for ONE kernel is fine even when another row has it merely `built`:
    /// DKMS keeps a row per kernel, and an older kernel's row is not a failure.
    #[test]
    fn a_module_installed_for_any_kernel_is_not_stuck() {
        let out = "\
nvidia/550.78, 6.8.0-31-generic, x86_64: installed
nvidia/550.78, 6.8.0-35-generic, x86_64: built
";
        assert!(not_installed(out).is_empty(), "{:?}", not_installed(out));
    }

    #[test]
    fn everything_installed_is_nothing_to_report() {
        let out = "\
nvidia/550.78, 6.8.0-35-generic, x86_64: installed
v4l2loopback/0.12.7, 6.8.0-35-generic, x86_64: installed
";
        assert!(not_installed(out).is_empty());
    }

    /// A machine with no DKMS modules has nothing to rebuild and nothing to report.
    #[test]
    fn no_modules_is_not_a_failure() {
        assert!(not_installed("").is_empty());
    }

    /// The message has to say why *now* matters, or a reader postpones it until after the
    /// reboot — which is the one time it cannot be fixed from the machine itself.
    #[test]
    fn the_failure_says_why_before_the_reboot_matters() {
        let msg = failed_to_build(
            &[Module {
                name: "nvidia".into(),
                version: "550.78".into(),
                kernel: "6.8.0-35-generic".into(),
                state: "added".into(),
            }],
            "6.8.0-35-generic",
        );
        assert!(msg.contains("nvidia/550.78"), "{}", msg);
        assert!(msg.contains("6.8.0-35-generic"), "{}", msg);
        assert!(msg.contains("reboot"), "{}", msg);
        assert!(msg.contains("make.log"), "{}", msg);
    }
}
