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
/// `@unverified` is not only about Shall's own `@sha256`: a manager can be the thing doing the
/// checking, and then the line still needs a way to say "not here". helm v4 verifies plugin
/// signatures by default and **refuses outright** a source that cannot carry one — a git URL
/// has no `.prov` file — so without this there is no declaration that installs a helm plugin
/// at all.
///
/// `allow_http` deliberately has no such table. The two flags never imply each other (SEC2),
/// and helm's plain-HTTP switch addresses OCI registries Shall does not reach.
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

/// Managers that install into an environment the operating system may own, and the argument
/// that says *write into it anyway* (`Q49`, owner ruling 2026-08-10).
///
/// **PEP 668.** Debian, Ubuntu, Alpine, openSUSE and Fedora ship a marker file next to their
/// Python — `EXTERNALLY-MANAGED` — that tells pip the interpreter belongs to the distro's own
/// package manager. pip then refuses every install, `--user` included, and it is right to: two
/// package managers writing the same site-packages is how a system python ends up unbootable.
///
/// So the default is the refusal, with `pipx:` named in it — Shall ships pipx, pipx exists for
/// exactly this, and it works on all of those distros. This flag is the escape hatch for
/// someone who means it, per line, never as a global switch.
const OS_OWNED_ENV: &[(&str, &str)] = &[("pip", "--break-system-packages")];

/// The argument that lets `backend` write into an OS-owned environment, if it has one.
pub fn os_owned_env_arg(backend: &str) -> Option<&'static str> {
    OS_OWNED_ENV
        .iter()
        .find(|(b, _)| *b == backend)
        .map(|(_, a)| *a)
}

/// Whether `@system` says anything on `backend`.
pub fn accepts_system(backend: &str) -> bool {
    os_owned_env_arg(backend).is_some()
}

/// The backends `@system` is legal on, for a refusal that names them.
pub fn system_backends() -> String {
    OS_OWNED_ENV
        .iter()
        .map(|(b, _)| *b)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether `@unverified` says anything on `backend` — Shall's checksum, or the manager's own
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

/// Why a backend cannot turn `@version=1.2.3` into an install argument (`Q53`).
///
/// **The ledger exists so that a backend can be unable to pin but never *silently* unable.**
/// [`Installable::pins_version`](crate::core::manager::Installable::pins_version) defaults to
/// `false`, so a backend that answers nothing refuses every pin — and this table is what turns
/// that refusal into a sentence a person can act on instead of a shrug. Every `false` needs a
/// row here, asserted by `a_version_pin_is_honoured_or_explained_tests`.
///
/// A row is a **reason**, not a to-do. A backend that could pin and simply has not been built
/// does not belong here: it belongs in that test's `COULD_PIN_AND_DOES_NOT`, which is a list of
/// live defects under a ceiling that only goes down.
///
/// **Read by the refusal, not only by the test**, which is why it is here and not beside the
/// scan. A message that says "cannot be met" without saying why is a puzzle (`V.42`).
const CANNOT_PIN_VERSION: &[(&str, &str)] = &[
    // Rolling repositories: one published version, and no flag that asks for another.
    (
        "pacman",
        "Arch is rolling — the repositories publish one version of a package and pacman has no \
         flag that asks for another",
    ),
    (
        "yay",
        "yay speaks pacman's flags over the same rolling repositories, so it inherits pacman's \
         answer",
    ),
    (
        "paru",
        "paru speaks pacman's flags over the same rolling repositories, so it inherits pacman's \
         answer",
    ),
    (
        "eopkg",
        "Solus is rolling — the repository holds one version and eopkg has no flag for another",
    ),
    (
        "slackpkg",
        "slackpkg installs what the configured mirror carries and takes no version",
    ),
    // The manager pins, but through something a bare version string cannot be turned into.
    (
        "brew",
        "Homebrew's `name@version` is a *different formula's name* (`python@3.12`), not a version \
         selector — a full version built into one names a formula that does not exist, and the \
         install fails permanently",
    ),
    (
        "scoop",
        "scoop pins through a versioned manifest in a bucket, not through an install flag",
    ),
    (
        "emerge",
        "Portage pins with an atom (`=category/name-version`), which needs the category as well \
         as the version — a bare `@version=` cannot be turned into a valid atom",
    ),
    (
        "macports",
        "a Portfile carries its own version, so installing an older one means checking out an \
         older ports tree — not something an install argument can express",
    ),
    (
        "xbps",
        "`xbps-install name-1.2.3_1` needs the package's revision suffix as well as its version, \
         which `@version=` does not carry and cannot derive — and Void is rolling, so there is \
         one version in the repository to select anyway",
    ),
    (
        "snap",
        "snap selects a channel or a revision number, and neither can be derived from a version",
    ),
    (
        "flatpak",
        "flatpak selects a branch or a commit hash, and neither can be derived from a version",
    ),
    (
        "nix",
        "a nix flake URI carries its own revision, so pinning means naming a different URI rather \
         than passing a version",
    ),
    // The index serves exactly one version, so there is nothing to choose between.
    (
        "mas",
        "the Mac App Store serves the current published version of an app and no other",
    ),
    (
        "krew",
        "the kubectl plugin index serves the current version of a plugin only",
    ),
    (
        "emacs",
        "package.el installs what the configured archive currently carries and takes no version",
    ),
    // The declaration already names the exact artifact, so a version would be a second answer
    // to a question that is already answered.
    (
        "web",
        "a `web:` line names one URL, which already is the exact thing to install",
    ),
    (
        "appimage",
        "an `appimage:` line names one file, which already is the exact thing to install",
    ),
    (
        "link",
        "a `link:` line creates a symlink, which has no version to install",
    ),
    // Not packages at all: these converge a machine's state, and state has no version.
    (
        "service",
        "a `service:` line enables a unit the system already has — the version is the package's, \
         not the service's",
    ),
    (
        "setting",
        "a `setting:` line writes a configuration value, which has no version",
    ),
    (
        "btrfs",
        "a `btrfs:` line creates a subvolume, which has no version",
    ),
    (
        "zfs",
        "a `zfs:` line creates a dataset, which has no version",
    ),
    (
        "lvm",
        "a `lvm:` line creates a volume, which has no version",
    ),
];

/// Why `backend` cannot honour an exact version, or `None` if it can.
///
/// **Asked of the ledger, not of the backend.** The backend answers *whether*
/// ([`Installable::pins_version`](crate::core::manager::Installable::pins_version)); this
/// answers *why*, and the two are checked against each other rather than derived from one
/// another — a backend that starts pinning and leaves its row behind makes the ledger a list of
/// things that used to be true, which the test catches from this side.
pub fn cannot_pin_reason(backend: &str) -> Option<&'static str> {
    CANNOT_PIN_VERSION
        .iter()
        .find(|(b, _)| *b == backend)
        .map(|(_, why)| *why)
}

/// Every backend the ledger excuses, for a test to check against the backends themselves.
pub fn backends_that_cannot_pin() -> Vec<&'static str> {
    CANNOT_PIN_VERSION.iter().map(|(b, _)| *b).collect()
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
