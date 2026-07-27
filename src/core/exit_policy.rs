//! What a package manager's exit code and output mean.
//!
//! This knowledge used to be a `match` on `"scoop" | "choco" | "winget"` inside
//! `CommandExecutor`, so registering a backend with its own conventions meant editing the
//! execution engine. A policy is data now, declared where the backend is registered and
//! carried by that backend's executor, and the engine only asks.

use crate::core::error::Retryability;

/// How one manager reports outcomes its exit code alone does not describe.
///
/// Marker strings are matched case-insensitively against the command's stdout and stderr
/// together. `permanent` is consulted before `transient`, so a manager that prints both
/// fails fast rather than looping.
#[derive(Debug, Clone, Default)]
pub struct ExitPolicy {
    /// Non-zero codes this manager uses for outcomes that are not failures.
    pub benign_exits: Vec<i32>,
    /// Text that means the command did nothing, even though it exited 0.
    pub failure_markers: Vec<&'static str>,
    /// Text that means the failure came from outside the request — a lock, a mirror, a
    /// network. Worth another attempt.
    pub transient_markers: Vec<&'static str>,
    /// Text that means the request itself is wrong. A second attempt reproduces it.
    pub permanent_markers: Vec<&'static str>,
}

impl ExitPolicy {
    /// A non-zero exit this manager does not mean as a failure.
    ///
    /// A command killed by a signal (`None`) is never benign: nothing chose that code.
    pub fn is_benign(&self, code: Option<i32>) -> bool {
        match code {
            Some(c) => self.benign_exits.contains(&c),
            None => false,
        }
    }

    /// A manager that exited 0 while refusing to do the work.
    pub fn signals_failure(&self, stdout: &[u8], stderr: &[u8]) -> bool {
        if self.failure_markers.is_empty() {
            return false;
        }
        let hay = Self::haystack(stdout, stderr);
        self.failure_markers.iter().any(|m| hay.contains(m))
    }

    /// Whether the same command could succeed on another attempt.
    pub fn retryability(&self, stdout: &[u8], stderr: &[u8]) -> Retryability {
        if self.permanent_markers.is_empty() && self.transient_markers.is_empty() {
            return Retryability::Unknown;
        }
        let hay = Self::haystack(stdout, stderr);
        if self.permanent_markers.iter().any(|m| hay.contains(m)) {
            return Retryability::Permanent;
        }
        if self.transient_markers.iter().any(|m| hay.contains(m)) {
            return Retryability::Transient;
        }
        Retryability::Unknown
    }

    fn haystack(stdout: &[u8], stderr: &[u8]) -> String {
        let mut hay = String::from_utf8_lossy(stdout).into_owned();
        hay.push_str(&String::from_utf8_lossy(stderr));
        hay.make_ascii_lowercase();
        hay
    }
}

/// Debian/Ubuntu. Shared by `apt` and by the `apt-get`/`dpkg-query`/`add-apt-repository`
/// programs the apt backend also runs, which is why it is bound per backend and not per
/// program name.
pub fn apt() -> ExitPolicy {
    ExitPolicy {
        transient_markers: vec![
            "could not get lock",
            "unable to acquire the dpkg frontend lock",
            "is another process using it",
            "temporary failure resolving",
            "connection timed out",
            "could not connect",
            "failed to fetch",
        ],
        permanent_markers: vec![
            "unable to locate package",
            "has no installation candidate",
            "couldn't find any package by",
        ],
        ..ExitPolicy::default()
    }
}

/// Fedora/RHEL — `dnf` and `yum`.
pub fn dnf() -> ExitPolicy {
    ExitPolicy {
        transient_markers: vec![
            "failed to synchronize cache",
            "cannot download",
            "another app is currently holding the yum lock",
            "curl error",
            "connection timed out",
        ],
        permanent_markers: vec!["no match for argument", "unable to find a match"],
        ..ExitPolicy::default()
    }
}

/// Arch — `pacman`, and the AUR helpers that speak its flags.
pub fn pacman() -> ExitPolicy {
    ExitPolicy {
        transient_markers: vec![
            "failed retrieving file",
            "could not resolve host",
            "unable to lock database",
            "connection timed out",
        ],
        permanent_markers: vec!["target not found", "could not find or read package"],
        ..ExitPolicy::default()
    }
}

/// Alpine.
pub fn apk() -> ExitPolicy {
    ExitPolicy {
        transient_markers: vec![
            "temporary error",
            "network error",
            "could not connect",
            "operation not permitted (try running as root)",
        ],
        permanent_markers: vec!["unable to select packages", "no such package"],
        ..ExitPolicy::default()
    }
}

/// macOS/Linuxbrew.
pub fn brew() -> ExitPolicy {
    ExitPolicy {
        transient_markers: vec![
            "failed to download",
            "curl: (28)",
            "operation timed out",
            "could not resolve host",
        ],
        permanent_markers: vec!["no available formula", "no formulae or casks found"],
        ..ExitPolicy::default()
    }
}

/// Chocolatey, which surfaces MSI conventions in its own exit status: 1641 reboot
/// initiated, 3010 reboot required, 1605/1614/1618 already-removed or uninstall-in-progress.
pub fn choco() -> ExitPolicy {
    ExitPolicy {
        benign_exits: vec![1605, 1614, 1618, 1641, 3010],
        permanent_markers: vec!["the package was not found with the source"],
        ..ExitPolicy::default()
    }
}

/// winget's HRESULT-style "success but noteworthy": no applicable upgrade, already
/// installed, no installed package found.
pub fn winget() -> ExitPolicy {
    ExitPolicy {
        benign_exits: vec![-1978335189, -1978335212, -1978335215],
        permanent_markers: vec!["no package found matching input criteria"],
        ..ExitPolicy::default()
    }
}

/// crates.io. `cargo install` names the two things that cannot be fixed by trying again:
/// a crate with no program in it, and a name the registry does not carry.
pub fn cargo() -> ExitPolicy {
    ExitPolicy {
        transient_markers: vec![
            "network failure",
            "spurious network error",
            "failed to fetch",
            "connection timed out",
        ],
        permanent_markers: vec![
            "no binaries",
            "nothing to install",
            "could not find",
            "does not have these features",
        ],
        ..ExitPolicy::default()
    }
}

/// scoop exits 0 after installing nothing when the manifest does not exist, so a bogus
/// install would otherwise be trusted. The tail of "Couldn't find manifest for 'x'." is
/// stable across scoop versions.
pub fn scoop() -> ExitPolicy {
    ExitPolicy {
        failure_markers: vec!["find manifest for"],
        permanent_markers: vec!["find manifest for"],
        ..ExitPolicy::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_policy_with_no_markers_classifies_nothing() {
        let p = ExitPolicy::default();
        assert_eq!(p.retryability(b"anything", b""), Retryability::Unknown);
        assert!(!p.signals_failure(b"Couldn't find manifest for 'x'.", b""));
        assert!(!p.is_benign(Some(3010)));
    }

    #[test]
    fn a_signal_kill_is_never_benign() {
        assert!(!choco().is_benign(None));
        assert!(!winget().is_benign(None));
    }

    #[test]
    fn benign_codes_are_per_backend_and_do_not_leak() {
        for code in [1605, 1614, 1618, 1641, 3010] {
            assert!(choco().is_benign(Some(code)));
            assert!(!winget().is_benign(Some(code)));
            assert!(!apt().is_benign(Some(code)));
        }
        assert!(!choco().is_benign(Some(1)));
        assert!(winget().is_benign(Some(-1978335189)));
        assert!(winget().is_benign(Some(-1978335212)));
        assert!(!winget().is_benign(Some(1)));
        assert!(!apt().is_benign(Some(100)));
        assert!(!apk().is_benign(Some(1)));
        assert!(!dnf().is_benign(Some(1)));
    }

    #[test]
    fn scoop_reports_a_missing_manifest_as_a_failure_and_never_retries_it() {
        let p = scoop();
        assert!(p.signals_failure(b"Couldn't find manifest for 'nope'.", b""));
        assert!(p.signals_failure(b"", b"couldn't find manifest for 'nope'."));
        assert!(!p.signals_failure(b"Installing 'jq'...", b""));
        assert_eq!(
            p.retryability(b"Couldn't find manifest for 'nope'.", b""),
            Retryability::Permanent
        );
    }

    #[test]
    fn a_held_lock_is_transient_and_a_missing_name_is_not() {
        assert_eq!(
            apt().retryability(b"", b"E: Could not get lock /var/lib/dpkg/lock-frontend"),
            Retryability::Transient
        );
        assert_eq!(
            apt().retryability(b"", b"E: Unable to locate package nosuchpkg"),
            Retryability::Permanent
        );
        assert_eq!(
            apt().retryability(b"", b"E: Something nobody classified"),
            Retryability::Unknown
        );
    }

    /// Every distro manager answers both questions, or the fast-fail only helps Debian.
    #[test]
    fn each_distro_policy_tells_a_held_lock_from_a_missing_name() {
        let cases = [
            (
                dnf(),
                "Another app is currently holding the yum lock",
                "No match for argument: nope",
            ),
            (
                pacman(),
                "error: unable to lock database",
                "error: target not found: nope",
            ),
            (
                apk(),
                "ERROR: temporary error (try again later)",
                "ERROR: unable to select packages:",
            ),
            (
                brew(),
                "curl: (28) Operation timed out",
                "Error: No available formula with the name \"nope\"",
            ),
        ];
        for (policy, transient, permanent) in cases {
            assert_eq!(
                policy.retryability(b"", transient.as_bytes()),
                Retryability::Transient,
                "not transient: {transient}"
            );
            assert_eq!(
                policy.retryability(b"", permanent.as_bytes()),
                Retryability::Permanent,
                "not permanent: {permanent}"
            );
        }
    }

    /// A manager that prints both must not loop on the half that cannot succeed.
    #[test]
    fn permanent_wins_over_transient() {
        let p = apt();
        let both = b"E: Could not get lock; E: Unable to locate package nope" as &[u8];
        assert_eq!(p.retryability(b"", both), Retryability::Permanent);
    }
}
