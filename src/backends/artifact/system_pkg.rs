//! Handing a downloaded `.deb`/`.rpm` to the system package manager (D5).
//!
//! `github:`/`web:` may resolve to a file that installs *itself* into a second package
//! database — a `.deb` to `dpkg`, an `.rpm` to `rpm`. When that happens the installing manager,
//! not LiNix, owns the files on disk: LiNix records **which** manager installed it, and removal,
//! upgrade and dedup all route back through that record (D5, owner 2026-07-24). This module is
//! the one place the argv for that hand-off is built, so github and web cannot drift on what
//! "hand it to dpkg" means, and so the argv is unit-testable without a live apt/rpm box — which
//! is the whole of what is verified here; the live round-trip is the real machine's job.

use super::format::Format;
use crate::core::{Error, Result};
use std::path::Path;

/// The system installer a given artifact format hands itself to, or `None` for a format LiNix
/// unpacks itself (a tarball) or that no manager on the supported platforms consumes as a file.
/// Only `.deb`/`.rpm` are wired: `.msi`/`.exe`/`.pkg` are `is_system_package` shapes too, but
/// their silent-install argv is per-vendor and unruled, so they stay unpacked/deployed, not
/// handed off.
pub fn installer_for(format: Format) -> Option<&'static str> {
    match format {
        Format::Deb => Some("dpkg"),
        Format::Rpm => Some("rpm"),
        _ => None,
    }
}

/// True when this format is one D5 hands to a system installer (as opposed to `is_system_package`,
/// which is the broader "installs into a database" set including the Windows/mac shapes LiNix
/// does not drive as a file yet).
pub fn is_handoff_format(format: Format) -> bool {
    installer_for(format).is_some()
}

/// The command whose presence means this machine can consume the handoff file. Same as the
/// installer name today, kept separate so a future split (apt provides dpkg) has one place.
pub fn detect_command(format: Format) -> Option<&'static str> {
    installer_for(format)
}

/// The argv that installs `file` through its system manager. `dpkg -i` upgrades in place, so it
/// doubles as the upgrade path; `rpm` uses `-U` (upgrade-or-install) rather than the ruling's
/// literal `-i`, because `rpm -i` *refuses* an already-installed package and the installing
/// backend is required to own the upgrade — `-U` is the one form that installs a new package and
/// replaces an older one with the same argv. The live exec is deferred (D5); this shape is what
/// is tested.
pub fn install_argv(format: Format, file: &Path) -> Result<Vec<String>> {
    let path = file.to_string_lossy().to_string();
    match format {
        Format::Deb => Ok(vec!["dpkg".into(), "-i".into(), path]),
        Format::Rpm => Ok(vec![
            "rpm".into(),
            "-U".into(),
            "--replacepkgs".into(),
            path,
        ]),
        other => Err(Error::Validation(format!(
            "{} is not a file LiNix hands to a system package manager",
            other
        ))),
    }
}

/// The argv that removes a package the recorded installer owns. `dpkg -r` / `rpm -e` take the
/// package *name* inside the file, not the filename — which is why the name is read at install
/// time and carried in the lock (`system_package`).
pub fn remove_argv(installer: &str, package: &str) -> Result<Vec<String>> {
    match installer {
        "dpkg" => Ok(vec!["dpkg".into(), "-r".into(), package.into()]),
        "rpm" => Ok(vec!["rpm".into(), "-e".into(), package.into()]),
        other => Err(Error::Validation(format!(
            "`{}` is not a system installer LiNix knows how to remove from",
            other
        ))),
    }
}

/// The argv that reads the package name out of a handoff file, before it is installed. That name
/// is the identity the manager will list it under, so it is what removal and dedup key on. Prints
/// exactly the name on stdout.
pub fn query_name_argv(format: Format, file: &Path) -> Result<Vec<String>> {
    let path = file.to_string_lossy().to_string();
    match format {
        Format::Deb => Ok(vec!["dpkg-deb".into(), "-f".into(), path, "Package".into()]),
        Format::Rpm => Ok(vec![
            "rpm".into(),
            "-qp".into(),
            "--queryformat".into(),
            "%{NAME}".into(),
            path,
        ]),
        other => Err(Error::Validation(format!(
            "{} carries no queryable package name",
            other
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn deb() -> PathBuf {
        PathBuf::from("/tmp/fd_10.2.0_amd64.deb")
    }
    fn rpm() -> PathBuf {
        PathBuf::from("/tmp/fd-10.2.0.x86_64.rpm")
    }

    #[test]
    fn only_deb_and_rpm_are_handoffs() {
        assert_eq!(installer_for(Format::Deb), Some("dpkg"));
        assert_eq!(installer_for(Format::Rpm), Some("rpm"));
        // A shape that installs into a database on another OS is not a handoff LiNix drives.
        assert_eq!(installer_for(Format::Msi), None);
        assert_eq!(installer_for(Format::Tarball), None);
        assert!(is_handoff_format(Format::Deb));
        assert!(!is_handoff_format(Format::Tarball));
    }

    #[test]
    fn deb_install_upgrades_in_place() {
        assert_eq!(
            install_argv(Format::Deb, &deb()).unwrap(),
            vec!["dpkg", "-i", "/tmp/fd_10.2.0_amd64.deb"]
        );
    }

    #[test]
    fn rpm_install_is_upgrade_or_install_not_bare_i() {
        // `rpm -i` refuses an already-installed package; the installing backend owns the
        // upgrade, so the argv must be the one that both installs and replaces.
        let argv = install_argv(Format::Rpm, &rpm()).unwrap();
        assert_eq!(argv[0], "rpm");
        assert_eq!(argv[1], "-U");
        assert!(argv.contains(&"/tmp/fd-10.2.0.x86_64.rpm".to_string()));
    }

    #[test]
    fn install_refuses_a_format_it_does_not_hand_off() {
        assert!(install_argv(Format::Tarball, &deb()).is_err());
    }

    #[test]
    fn remove_uses_the_package_name_not_the_file() {
        assert_eq!(remove_argv("dpkg", "fd").unwrap(), vec!["dpkg", "-r", "fd"]);
        assert_eq!(remove_argv("rpm", "fd").unwrap(), vec!["rpm", "-e", "fd"]);
        assert!(remove_argv("brew", "fd").is_err());
    }

    #[test]
    fn name_query_reads_from_the_file() {
        let argv = query_name_argv(Format::Deb, &deb()).unwrap();
        assert_eq!(argv[0], "dpkg-deb");
        assert!(argv.contains(&"Package".to_string()));
        let argv = query_name_argv(Format::Rpm, &rpm()).unwrap();
        assert_eq!(argv[0], "rpm");
        assert!(argv.contains(&"%{NAME}".to_string()));
    }
}
