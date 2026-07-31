//! Which backends an option is legal on.
//!
//! Deliberately a static table rather than a question put to the registry. Backend
//! *existence* is host-dependent — there is no `snap` on Windows — but "snap publishes
//! channels" is true everywhere, and a module shared across a fleet has to parse the same way
//! on every machine in it. Deriving this from what is installed here would make a file legal
//! on one box and a syntax error on the next.

/// Backends where one declared name resolves to several downloadable artifacts, so `formats`,
/// `asset` and `bin` are meaningful.
///
/// `web:` is absent on purpose: a `web:URL` spec names exactly one file, so there is nothing
/// to select. It joins this list if and when a `web:` spec can resolve to several candidates.
/// `appimage:` is absent because the backend name is already the format.
const SELECTS_ARTIFACTS: &[&str] = &["github"];

/// Backends that fetch a URL, make the result executable and put it on `PATH`, so SEC2's
/// download rules — HTTPS, a checksum, and the two flags that relax them — mean something.
///
/// Every other backend asks a package manager, which has its own signed index; `@allow_http`
/// there would be a line that does nothing.
const DOWNLOADS: &[&str] = &["web", "appimage", "github"];

/// Managers that verify a signature themselves, and the argument that turns it off (Q5).
///
/// `@unverified` is not only about LiNix's own `@sha256`: a manager can be the thing doing the
/// checking, and then the line still needs a way to say "not here". helm v4 verifies plugin
/// signatures by default and **refuses outright** a source that cannot carry one — a git URL
/// has no `.prov` file — so without this there is no declaration that installs a helm plugin
/// at all.
///
/// `allow_http` deliberately has no such table. The two flags never imply each other (SEC2),
/// and helm's plain-HTTP switch addresses OCI registries LiNix does not reach.
const VERIFIES_ITSELF: &[(&str, &str)] = &[("helm", "--verify=false")];

/// Backends that publish one artifact in several version streams.
const HAS_CHANNELS: &[&str] = &["snap", "flatpak"];

/// Backends whose install command takes something other than the package's own name, and the
/// option key that carries it (U39). `helm plugin install` takes a URL while `plugin list` and
/// `plugin uninstall` speak the name in the plugin's `plugin.yaml`, so the name has to stay the
/// identity and the URL rides in `@url=`.
///
/// One table, read by both ends: the grammar decides the key is legal here and nowhere else,
/// and `backends/registry.rs` builds the backend's `install_source_option` from it.
const INSTALLS_FROM_SOURCE: &[(&str, &str)] = &[("helm", "url")];

/// Options one family of backends reads and no other backend can act on: the geometry of a
/// declared storage object (Q18), and snap's confinement. Each is legal exactly where something
/// reads it and refused by name everywhere else.
///
/// **The failure this table exists to end.** Every one of these keys was read by a backend and
/// absent from `PACKAGE_OPTION_KEYS`, so the grammar refused the only line that could reach the
/// code — `lvm:` required `@size` and no `lvm:` line could carry one, which left the backend
/// unwritable from the day it was merged, and `snap`'s `--classic` branch had never once run.
/// A key a backend reads and the grammar has never heard of is dead code that reads as a
/// feature, so the two lists are one list, asserted equal by a test below.
///
/// The third column is what the key means, and a refusal is that sentence plus the backends
/// that take it — an error that says "not here" and not where is a puzzle, not a message.
const SCOPED_OPTIONS: &[(&str, &[&str], &str)] = &[
    (
        "size",
        &["lvm"],
        "it is the size a volume is created at — a btrfs subvolume grows into its filesystem, \
         and a ZFS dataset is bounded by `@quota` instead",
    ),
    (
        "allow_shrink",
        &["lvm"],
        "it lets a smaller `@size` take space back off a volume that already exists, which is \
         the one declared change that can destroy a filesystem",
    ),
    (
        "quota",
        &["btrfs", "zfs"],
        "it caps how much a declared storage object may use",
    ),
    (
        "mount",
        &["btrfs", "zfs"],
        "it is where a declared storage object is mounted",
    ),
    (
        "mount_options",
        &["btrfs"],
        "it is what the fstab entry's option field carries; ZFS keeps its mount properties on \
         the dataset and has no such field",
    ),
    (
        "classic",
        &["snap"],
        "it installs unconfined, which is a snap concept",
    ),
];

/// Whether `key` belongs to one family of backends rather than to packages at large.
pub fn is_scoped_option(key: &str) -> bool {
    SCOPED_OPTIONS.iter().any(|(k, _, _)| *k == key)
}

/// Whether `backend` is one of the backends that reads `key`.
pub fn takes_scoped_option(backend: &str, key: &str) -> bool {
    SCOPED_OPTIONS
        .iter()
        .any(|(k, backends, _)| *k == key && backends.contains(&backend))
}

/// Why `key` exists and who takes it, for a refusal that names both.
pub fn scoped_option_reason(key: &str) -> String {
    SCOPED_OPTIONS
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(_, backends, meaning)| {
            format!(
                "{}, which only {} {}.",
                meaning,
                backends.join(", "),
                if backends.len() == 1 { "takes" } else { "take" }
            )
        })
        .unwrap_or_default()
}

/// The option key `backend` takes its install argument from, if it is not the name.
pub fn install_source_key(backend: &str) -> Option<&'static str> {
    INSTALLS_FROM_SOURCE
        .iter()
        .find(|(b, _)| *b == backend)
        .map(|(_, k)| *k)
}

/// Whether `key` is any backend's install-source key — what tells `@url` on `apt` apart from a
/// misspelling.
pub fn is_source_key(key: &str) -> bool {
    INSTALLS_FROM_SOURCE.iter().any(|(_, k)| *k == key)
}

/// The backends that take `key` as their install source, for a refusal that names them.
pub fn source_backends(key: &str) -> String {
    INSTALLS_FROM_SOURCE
        .iter()
        .filter(|(_, k)| *k == key)
        .map(|(b, _)| *b)
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn selects_artifacts(backend: &str) -> bool {
    SELECTS_ARTIFACTS.contains(&backend)
}

pub fn downloads(backend: &str) -> bool {
    DOWNLOADS.contains(&backend)
}

pub fn download_backends() -> String {
    DOWNLOADS.join(", ")
}

/// The argument that turns off `backend`'s own signature check, if it has one.
pub fn unverified_arg(backend: &str) -> Option<&'static str> {
    VERIFIES_ITSELF
        .iter()
        .find(|(b, _)| *b == backend)
        .map(|(_, a)| *a)
}

/// Whether `@unverified` says anything on `backend` — LiNix's checksum, or the manager's own
/// signature check. Wider than [`downloads`], which is `@allow_http`'s set alone.
pub fn accepts_unverified(backend: &str) -> bool {
    downloads(backend) || unverified_arg(backend).is_some()
}

/// The backends `@unverified` is legal on, for a refusal that names them.
pub fn unverified_backends() -> String {
    DOWNLOADS
        .iter()
        .copied()
        .chain(VERIFIES_ITSELF.iter().map(|(b, _)| *b))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn has_channels(backend: &str) -> bool {
    HAS_CHANNELS.contains(&backend)
}

pub fn artifact_backends() -> String {
    SELECTS_ARTIFACTS.join(", ")
}

pub fn channel_backends() -> String {
    HAS_CHANNELS.join(", ")
}

/// The comparable part of a channel string. A snap channel is `track/risk`
/// (`latest/stable`), and the user usually writes just the risk (`stable`), so the two must
/// compare equal or a channel change would fire on every sync (D13).
pub fn channel_risk(channel: &str) -> &str {
    channel.rsplit('/').next().unwrap_or(channel).trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_axes_do_not_overlap() {
        for b in SELECTS_ARTIFACTS {
            assert!(
                !has_channels(b),
                "{} would accept both formats and channel",
                b
            );
        }
    }

    #[test]
    fn a_backend_whose_ecosystem_chose_the_file_has_neither() {
        for b in ["apt", "dnf", "cargo", "npm", "pacman"] {
            assert!(!selects_artifacts(b));
            assert!(!has_channels(b));
        }
    }

    #[test]
    fn appimage_does_not_select_a_format_because_it_is_one() {
        assert!(!selects_artifacts("appimage"));
    }

    /// The two ends of U39's one table. A source key the grammar has never heard of is
    /// rejected as a misspelling, and the backend then refuses every line that carries it —
    /// which is how the fix shipped the first time, and it took a real helm to notice.
    #[test]
    fn every_install_source_key_is_a_legal_option_key() {
        for (backend, key) in INSTALLS_FROM_SOURCE {
            assert!(
                crate::config::grammar::statement::PACKAGE_OPTION_KEYS.contains(key),
                "`@{}` is {}'s install source and the grammar would refuse it",
                key,
                backend
            );
            assert_eq!(install_source_key(backend), Some(*key));
            assert!(is_source_key(key));
        }
    }

    /// The same two ends as the test above, for the wider table (Q18). Five keys were read by a
    /// backend and refused by the parser at once, so this asserts the join rather than any one
    /// of them: a key that reaches no line is a feature nobody can use.
    #[test]
    fn every_scoped_option_is_a_legal_option_key() {
        for (key, backends, meaning) in SCOPED_OPTIONS {
            assert!(
                crate::config::grammar::statement::PACKAGE_OPTION_KEYS.contains(key),
                "`@{}` is read by {} and the grammar would refuse it",
                key,
                backends.join(", ")
            );
            assert!(!backends.is_empty(), "`@{}` is legal on nothing", key);
            assert!(!meaning.is_empty(), "`@{}` refuses without saying why", key);
            for b in *backends {
                assert!(takes_scoped_option(b, key));
            }
            assert!(is_scoped_option(key));
            let reason = scoped_option_reason(key);
            assert!(reason.contains(backends[0]));
            // One backend takes it; several take it. A refusal is read by a person.
            assert!(
                reason.ends_with(if backends.len() == 1 {
                    "takes."
                } else {
                    "take."
                }),
                "`@{}`: {}",
                key,
                reason
            );
        }
    }

    #[test]
    fn a_scoped_option_is_refused_on_a_backend_that_cannot_read_it() {
        for b in ["apt", "cargo", "npm", "github", "brew"] {
            for (key, _, _) in SCOPED_OPTIONS {
                assert!(!takes_scoped_option(b, key), "{} takes @{}", b, key);
            }
        }
        // The neighbours inside the family are the ones worth naming: each storage backend
        // takes a different subset, and a table that let them share everything would put
        // `@size` on a subvolume that has no size to set.
        assert!(!takes_scoped_option("zfs", "size"));
        assert!(!takes_scoped_option("btrfs", "size"));
        assert!(!takes_scoped_option("zfs", "mount_options"));
        assert!(!takes_scoped_option("lvm", "quota"));
    }

    #[test]
    fn a_backend_that_installs_by_name_has_no_source_key() {
        for b in ["apt", "cargo", "npm", "krew", "github", "web"] {
            assert!(install_source_key(b).is_none(), "{}", b);
        }
    }
}
