//! **A backend may be unable to install an exact version. It may not be *silently* unable.**
//!
//! `@version=1.2.3` on a declaration, and every version `linix lock` records by itself, reach
//! `generic.rs` as a `version` option. There the backend's `version_pin` decides what happens:
//! a manager that has one gets its native syntax, and a manager that has none falls through the
//! match to `names.push(spec.name.clone())` — the version is dropped, the install runs at
//! whatever version the manager chose, and the command reports success. That is the failure
//! class this repo exists to remove, arrived at by omitting a struct field.
//!
//! Ten backends were in that state when this was written (`S85`), and nobody could have known
//! from reading the code: three carried a `None` with a reason in a comment, four carried a bare
//! `None`, `xbps` never named the field at all, and five rows of `builtin_backends.toml` simply
//! had no `version_pin` key. A comment is not an instrument — it cannot be enumerated, counted,
//! or made to fail.
//!
//! **What this gate does.** Every registrar in `registry.rs` that builds a `ManagerConfig`, and
//! every row of `builtin_backends.toml`, must either pin a version or appear in the ledger below
//! with a reason. The ledger separates two things a single `None` conflates:
//!
//! - [`NO_VERSION_TO_ASK_FOR`] — the manager has no mechanism. Dropping the pin is the manager's
//!   limit, not LiNix's, and the entry is permanent.
//! - [`COULD_PIN_AND_DOES_NOT`] — the manager takes a version and LiNix does not send one. Every
//!   entry here is a live `@version=` that reports success at the wrong version. The list is
//!   under a ceiling and only shrinks.
//!
//! **What it does not do.** It reads declarations, not argv. Whether a declared pin lands in the
//! right place in the command line is the existing per-backend argv assertions in `registry.rs`,
//! which cover the nine that pin. And it says nothing about `brew`, which pins through its own
//! `Installable` rather than through `version_pin` — `brew` builds a formula name that does not
//! exist, which is `S85` and awaits the ruling in `Q53`.
//!
//! **Why a source scan and not a registry walk.** `create_default_registry` registers behind
//! `cfg!(target_os = …)`, so a walk sees `apt` only on Linux and `mas` only on a Mac. That is
//! exactly how `dnf`'s exit-policy defect survived inside a passing suite until `S83`: the gate
//! that should have caught it walked the registry from Windows, where dnf does not register.
//! Reading the file finds every backend from every host.

use std::collections::BTreeMap;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Does this line answer the question, in any of the three shapes the file writes it?
///
/// `version_pin:` in a struct literal, `version_pin =` on a `ManagerConfig` already built, and
/// `cfg.version_pin =` where the registrar takes `base_config`'s default and overrides it —
/// which is how `dnf` and `conda` set theirs. The first version of this predicate knew only the
/// first two shapes and reported both of them as dropping the pin they declare.
///
/// A line that merely mentions the name inside a comment is not an answer: a reason a person can
/// read cannot be counted, and `register_pacman`'s comment is exactly the case in point.
fn names_the_pin(trimmed: &str) -> bool {
    let stripped = trimmed.trim_start_matches("cfg.");
    !trimmed.starts_with("//")
        && (stripped.starts_with("version_pin:") || stripped.starts_with("version_pin ="))
}

/// The manager cannot be asked for a version. Permanent entries.
///
/// Keyed by the registrar or the `builtin_backends.toml` row, because that is what the scan
/// finds; `register_aur_helper` builds both `yay` and `paru` from one body and has no single
/// backend name to key on.
///
/// **Four of these are documented rather than measured** — `macports`, `krew`, `slackpkg` and
/// `eopkg` are read from their managers' documentation, not from a container that ran them.
/// The `gentoo` image can settle `emerge`; there is no image here for the other four.
const NO_VERSION_TO_ASK_FOR: &[(&str, &str)] = &[
    (
        "register_pacman",
        "Arch is rolling: the repositories publish one version of a package and there is no \
         flag that asks for another",
    ),
    (
        "register_aur_helper",
        "yay and paru speak pacman's flags over the same rolling repositories, so they inherit \
         pacman's answer",
    ),
    (
        "register_scoop",
        "scoop pins through a versioned manifest in a bucket, not through an install flag",
    ),
    (
        "register_mas",
        "the Mac App Store serves the current published version of an app and no other",
    ),
    (
        "register_macports",
        "a Portfile carries its own version; installing an older one means checking out an \
         older ports tree, which is not something an install argument can express",
    ),
    (
        "krew",
        "the kubectl plugin index serves the current version of a plugin only",
    ),
    (
        "slackpkg",
        "slackpkg installs what the configured mirror carries and takes no version",
    ),
    (
        "eopkg",
        "Solus is rolling: the repository holds one version and eopkg has no flag for another",
    ),
    (
        "emerge",
        "Portage pins with an atom (`=category/name-version`), which needs the category as well \
         as the version — a bare `@version=` cannot be turned into a valid atom",
    ),
];

/// The manager takes a version and LiNix does not send one. **Every entry is a live defect**: a
/// declared `@version=` is dropped and the install reports success at the wrong version.
///
/// Under a ceiling, and the ceiling only goes down. Lower it in the same change that builds one.
const COULD_PIN_AND_DOES_NOT: &[(&str, &str)] = &[
    (
        "register_xbps",
        "`xbps-install name-1.2.3_1` takes a version, and the field is not named here at all — \
         note the pin also needs the `_revision` suffix that `@version=` does not carry",
    ),
    (
        "register_pkgin",
        "`pkgin install name-1.2.3` takes a version",
    ),
    (
        "register_pkg_freebsd",
        "`pkg install name-1.2.3` takes a version",
    ),
    (
        "register_pkg_add_openbsd",
        "`pkg_add name-1.2.3` takes a version",
    ),
];

/// It only shrinks. Four when this was written (`S85`).
const UNBUILT_CEILING: usize = 4;

fn ledger_reason(key: &str) -> Option<&'static str> {
    NO_VERSION_TO_ASK_FOR
        .iter()
        .chain(COULD_PIN_AND_DOES_NOT.iter())
        .find(|(k, _)| *k == key)
        .map(|(_, why)| *why)
}

/// Every registrar in `registry.rs` that builds a `ManagerConfig`, and whether it names a
/// version pin at all.
///
/// **Omitting the field counts as not pinning**, which is the point: `xbps` never wrote
/// `version_pin` and took `Default`'s `None` in silence. A scan that only looked for the literal
/// `version_pin: None` would have reported `xbps` clean.
fn registrars_and_their_pins() -> BTreeMap<String, bool> {
    let src = std::fs::read_to_string(root().join("src/backends/registry.rs"))
        .expect("registry.rs is in this repo");
    let mut out: BTreeMap<String, bool> = BTreeMap::new();
    let mut builds_a_config: BTreeMap<String, bool> = BTreeMap::new();
    let mut current = String::new();
    for line in src.lines() {
        if let Some(rest) = line.strip_prefix("fn register_") {
            current = format!("register_{}", rest.split('(').next().unwrap_or(""));
            out.entry(current.clone()).or_insert(false);
            builds_a_config.entry(current.clone()).or_insert(false);
            continue;
        }
        // A top-level `fn` that is not a registrar ends the one we were in — `base_config`'s own
        // `version_pin: None` is the default every registrar may override, not a backend's answer.
        if line.starts_with("fn ") || line.starts_with("pub fn ") {
            current.clear();
            continue;
        }
        if current.is_empty() {
            continue;
        }
        if line.contains("ManagerConfig {") || line.contains("base_config(") {
            builds_a_config.insert(current.clone(), true);
        }
        // `version_pin:` in a struct literal, `version_pin =` in a later override. A line that
        // only mentions the name inside a comment is not an answer.
        let trimmed = line.trim_start();
        if names_the_pin(trimmed) {
            out.insert(current.clone(), !trimmed.contains("None"));
        }
    }
    out.into_iter()
        .filter(|(name, _)| builds_a_config.get(name).copied().unwrap_or(false))
        .collect()
}

/// Every row of `builtin_backends.toml`, and whether it declares a version pin.
///
/// Parsed by the program's own loader rather than by matching text, so a row this test reads is
/// the row the backend is built from.
fn data_rows_and_their_pins() -> BTreeMap<String, bool> {
    linix::backends::onboarder::builtin_rows()
        .into_iter()
        // A row with an install *source* takes a URL or a path where a package manager takes a
        // name, so there is no version to pin — `github:owner/repo` carries its release tag in
        // the source itself.
        .filter(|row| row.install_source_option.is_none())
        .map(|row| (row.name.clone(), row.version_pin.is_some()))
        .collect()
}

#[test]
fn every_registrar_that_builds_a_config_pins_a_version_or_says_why() {
    let found = registrars_and_their_pins();
    // II.23's floor. A scan that stopped matching `registry.rs` would otherwise report clean.
    assert!(
        found.len() >= 18,
        "only {} registrar(s) build a ManagerConfig; this scan has stopped matching registry.rs",
        found.len()
    );
    assert!(
        found.values().filter(|pins| **pins).count() >= 8,
        "no registrar was found to pin a version at all, so the scan is measuring nothing"
    );

    let silent: Vec<&String> = found
        .iter()
        .filter(|(name, pins)| !**pins && ledger_reason(name).is_none())
        .map(|(name, _)| name)
        .collect();
    assert!(
        silent.is_empty(),
        "these registrars drop a declared `@version=` and report success, with nothing recorded \
         saying whether the manager cannot take one or LiNix does not send one: {silent:?}. Add \
         a `version_pin`, or an entry to NO_VERSION_TO_ASK_FOR / COULD_PIN_AND_DOES_NOT."
    );
}

#[test]
fn every_data_defined_backend_pins_a_version_or_says_why() {
    let found = data_rows_and_their_pins();
    assert!(
        found.len() >= 18,
        "only {} row(s) came back from builtin_rows(); the table or the filter has moved",
        found.len()
    );
    assert!(
        found.values().filter(|pins| **pins).count() >= 15,
        "no row was found to pin a version, so the scan is measuring nothing"
    );

    let silent: Vec<&String> = found
        .iter()
        .filter(|(name, pins)| !**pins && ledger_reason(name).is_none())
        .map(|(name, _)| name)
        .collect();
    assert!(
        silent.is_empty(),
        "these rows of builtin_backends.toml drop a declared `@version=` and report success, \
         with no reason recorded: {silent:?}"
    );
}

/// The other direction, and the one that rots first. An entry that stays behind after its
/// backend gained a pin makes the ledger a list of things that used to be true.
#[test]
fn the_ledger_names_nothing_that_now_pins() {
    let mut all = registrars_and_their_pins();
    all.extend(data_rows_and_their_pins());

    let stale: Vec<&str> = NO_VERSION_TO_ASK_FOR
        .iter()
        .chain(COULD_PIN_AND_DOES_NOT.iter())
        .map(|(k, _)| *k)
        .filter(|k| all.get(*k).copied().unwrap_or(false))
        .collect();
    assert!(
        stale.is_empty(),
        "these pin a version now and are still recorded as unable to: {stale:?}"
    );

    let unknown: Vec<&str> = NO_VERSION_TO_ASK_FOR
        .iter()
        .chain(COULD_PIN_AND_DOES_NOT.iter())
        .map(|(k, _)| *k)
        .filter(|k| !all.contains_key(*k))
        .collect();
    assert!(
        unknown.is_empty(),
        "these are in the ledger and neither scan finds them — a renamed registrar or a deleted \
         row leaves an exemption covering nothing: {unknown:?}"
    );

    for (key, why) in NO_VERSION_TO_ASK_FOR.iter().chain(COULD_PIN_AND_DOES_NOT) {
        assert!(
            why.len() > 20,
            "`{key}` is exempt with no reason worth reading: {why:?}"
        );
    }
}

/// The ratchet. `COULD_PIN_AND_DOES_NOT` is a list of live defects, not a list of decisions.
#[test]
fn the_backends_that_could_pin_and_do_not_only_get_fewer() {
    assert!(
        COULD_PIN_AND_DOES_NOT.len() <= UNBUILT_CEILING,
        "{} backend(s) take a version LiNix does not send, and the ceiling is {}. Build one, or \
         move it to NO_VERSION_TO_ASK_FOR with the reason it cannot be built.",
        COULD_PIN_AND_DOES_NOT.len(),
        UNBUILT_CEILING
    );
}

/// **Does the scan fail on the shape that shipped?** `xbps` named no `version_pin` at all and a
/// scan looking for the literal `version_pin: None` called it clean. Both shapes are fed to the
/// parser here, because the one that hid for longest is the one with nothing to match.
#[test]
fn the_scan_sees_both_shapes_of_not_pinning() {
    // Mirrors the parser above rather than calling it, because the real one reads a fixed path.
    fn pins_in(body: &str) -> BTreeMap<String, bool> {
        let mut out: BTreeMap<String, bool> = BTreeMap::new();
        let mut builds: BTreeMap<String, bool> = BTreeMap::new();
        let mut current = String::new();
        for line in body.lines() {
            if let Some(rest) = line.strip_prefix("fn register_") {
                current = format!("register_{}", rest.split('(').next().unwrap_or(""));
                out.entry(current.clone()).or_insert(false);
                builds.entry(current.clone()).or_insert(false);
                continue;
            }
            if line.starts_with("fn ") || line.starts_with("pub fn ") {
                current.clear();
                continue;
            }
            if current.is_empty() {
                continue;
            }
            if line.contains("ManagerConfig {") || line.contains("base_config(") {
                builds.insert(current.clone(), true);
            }
            let t = line.trim_start();
            if names_the_pin(t) {
                out.insert(current.clone(), !t.contains("None"));
            }
        }
        out.into_iter()
            .filter(|(n, _)| builds.get(n).copied().unwrap_or(false))
            .collect()
    }

    let sample = "\
fn register_good(reg: &mut R, e: &E) {
    let cfg = ManagerConfig {
        version_pin: Some(VersionPin::Inline(\"{name}={version}\".into())),
    };
}
fn register_explicit_none(reg: &mut R, e: &E) {
    let cfg = ManagerConfig {
        version_pin: None,
    };
}
fn register_never_says(reg: &mut R, e: &E) {
    let cfg = ManagerConfig {
        install_args: vec![],
    };
}
fn register_only_a_comment(reg: &mut R, e: &E) {
    // helpers that share this syntax carry no `version_pin` either.
    let cfg = ManagerConfig {
        install_args: vec![],
    };
}
fn register_overrides_the_default(reg: &mut R, e: &E) {
    let mut cfg = base_config(\"dnf\");
    cfg.version_pin = Some(VersionPin::Inline(\"{name}-{version}\".into()));
}
fn base_config(name: &str) -> ManagerConfig {
    ManagerConfig {
        version_pin: None,
    }
}
";
    let seen = pins_in(sample);
    assert_eq!(
        seen.get("register_good"),
        Some(&true),
        "a declared pin was not seen"
    );
    assert_eq!(
        seen.get("register_explicit_none"),
        Some(&false),
        "an explicit `None` was not caught"
    );
    assert_eq!(
        seen.get("register_never_says"),
        Some(&false),
        "a registrar that never names the field was not caught — this is the xbps shape, and \
         the whole reason the scan cannot look for `version_pin: None`"
    );
    assert_eq!(
        seen.get("register_only_a_comment"),
        Some(&false),
        "a reason in a comment was read as an answer; a comment cannot be enumerated"
    );
    assert_eq!(
        seen.get("register_overrides_the_default"),
        Some(&true),
        "a `cfg.version_pin = Some(…)` override was read as no pin — the shape `dnf` and `conda` \
         use, and the one this predicate got wrong first time round"
    );
    assert!(
        !seen.contains_key("base_config"),
        "base_config is the default every registrar may override, not a backend's answer"
    );
}
