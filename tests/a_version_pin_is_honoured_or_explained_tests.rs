//! **A backend may be unable to install an exact version. It may not be *silently* unable.**
//!
//! `@version=1.2.3` on a declaration, and every version `shall lock` records by itself, reach
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
//! **What this gate does.** Every registrar in `registry.rs` that builds a `ManagerConfig`, every
//! row of `builtin_backends.toml`, and every hand-written backend module, must either pin a
//! version or appear in the ledger with a reason. Two lists, and they are not the same list:
//!
//! - `capability::CANNOT_PIN_VERSION` — the manager has no mechanism. Dropping the pin is the
//!   manager's limit, not Shall's, and the entry is permanent. It lives in the **program**
//!   because the refusal quotes it at the user.
//! - [`COULD_PIN_AND_DOES_NOT`] — the manager takes a version and Shall does not send one. Every
//!   entry here is a live `@version=` that reports success at the wrong version. A to-do, not a
//!   reason, under a ceiling that only shrinks. Empty since `Q53`.
//!
//! **What it does not do.** It reads declarations, not argv. Whether a declared pin lands in the
//! right place in the command line is the existing per-backend argv assertions in `registry.rs`.
//!
//! **What `Q53` changed.** A pin the named manager cannot express is no longer dropped: the
//! planner refuses that package by name before anything runs, and `sync --locked` treats the same
//! fact as fatal. So this gate is no longer only about a missing struct field — a backend that
//! answers `pins_version() == false` with no ledger row now produces a refusal that cannot say
//! why, which is the shape `V.42` bans.
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

/// The backend names one registrar builds.
///
/// `register_X` builds `X` for all but three, and each exception is why this mapping is written
/// out rather than derived: `register_aur_helper` builds **two** backends from one body, and the
/// two BSD registrars are named for their platform while the backend is named for its binary.
fn backends_of(registrar: &str) -> Vec<String> {
    match registrar {
        "register_aur_helper" => vec!["yay".into(), "paru".into()],
        "register_pkg_freebsd" => vec!["pkg".into()],
        "register_pkg_add_openbsd" => vec!["pkg_add".into()],
        other => vec![other.trim_start_matches("register_").to_string()],
    }
}

/// The manager takes a version and Shall does not send one. **Every entry is a live defect**: a
/// declared `@version=` is dropped and the install reports success at the wrong version.
///
/// Under a ceiling, and the ceiling only goes down. Lower it in the same change that builds one.
///
/// **Empty since `Q53` was ruled.** `pkgin`, `pkg` and `pkg_add` were built — all three spell a
/// version as the operand's suffix — and `xbps` moved to the permanent side of the ledger,
/// because `name-1.2.3` without the revision suffix names a package that does not exist, which
/// is building a name and hoping rather than pinning.
const COULD_PIN_AND_DOES_NOT: &[(&str, &str)] = &[];

// The list carried a `UNBUILT_CEILING` while it had entries, and the ceiling only ever went
// down: four when this was written (`S85`), then zero once `Q53` was ruled and the three
// buildable ones were built. At zero a ceiling is not a ratchet any more — it is `is_empty()`
// with arithmetic around it, which clippy says out loud — so the assertion below says that
// instead. Anyone who needs an entry here again re-introduces a bound deliberately, with the
// count in it, rather than inheriting a number that has stopped meaning anything.

/// Why this backend cannot pin, from the ledger the **program** reads.
///
/// **Not a second table.** The reasons live in `capability::CANNOT_PIN_VERSION` because the
/// refusal quotes them at the user; a copy here would be a list that agrees with the messages
/// until the day it does not. This asks that one, and adds only the defect list above — which is
/// a to-do, not a reason, and has no business in a user-facing message.
fn ledger_reason(key: &str) -> Option<&'static str> {
    if let Some((_, why)) = COULD_PIN_AND_DOES_NOT.iter().find(|(k, _)| *k == key) {
        return Some(why);
    }
    backends_of(key)
        .iter()
        .find_map(|b| shall::backends::capability::cannot_pin_reason(b))
}

/// Every registrar in `registry.rs` that builds a `ManagerConfig`, and whether it names a
/// version pin at all.
///
/// **Omitting the field counts as not pinning**, which is the point: `xbps` never wrote
/// `version_pin` and took `Default`'s `None` in silence. A scan that only looked for the literal
/// `version_pin: None` would have reported `xbps` clean.
fn registrars_and_their_pins() -> BTreeMap<String, bool> {
    let src = crate::harness::registry_source();
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
    shall::backends::onboarder::builtin_rows()
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
         saying whether the manager cannot take one or Shall does not send one: {silent:?}. Add \
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

/// Every backend the scans can see, keyed the way a *user* names it rather than the way the
/// source spells it — which is the key the ledger the program reads is written in.
fn pins_by_backend_name() -> BTreeMap<String, bool> {
    let mut out: BTreeMap<String, bool> = BTreeMap::new();
    for (registrar, pins) in registrars_and_their_pins() {
        for backend in backends_of(&registrar) {
            out.insert(backend, pins);
        }
    }
    out.extend(data_rows_and_their_pins());
    out
}

/// The other direction, and the one that rots first. An entry that stays behind after its
/// backend gained a pin makes the ledger a list of things that used to be true.
#[test]
fn the_ledger_names_nothing_that_now_pins() {
    let all = pins_by_backend_name();

    let stale: Vec<&str> = shall::backends::capability::backends_that_cannot_pin()
        .into_iter()
        .filter(|b| all.get(*b).copied().unwrap_or(false))
        .collect();
    assert!(
        stale.is_empty(),
        "these pin a version now and are still recorded as unable to: {stale:?}"
    );

    let stale_defects: Vec<&str> = COULD_PIN_AND_DOES_NOT
        .iter()
        .map(|(k, _)| *k)
        .filter(|k| {
            backends_of(k)
                .iter()
                .any(|b| all.get(b).copied().unwrap_or(false))
        })
        .collect();
    assert!(
        stale_defects.is_empty(),
        "these were built and are still listed as unbuilt defects: {stale_defects:?}"
    );

    for (key, why) in COULD_PIN_AND_DOES_NOT {
        assert!(
            why.len() > 20,
            "`{key}` is exempt with no reason worth reading: {why:?}"
        );
    }
}

/// **The ledger the program reads is the ledger the tests check.** `capability::cannot_pin_reason`
/// is what a refusal quotes at the user, so an entry there with no backend behind it is a message
/// that can never print, and a reason too short to act on is `V.42`'s narration with a different
/// hat on.
///
/// The hand-written backends are exempt from the "some scan finds it" half: `brew`, `nix`,
/// `snap`, `flatpak` and the rest build their `Installable` in their own module and appear in
/// neither `registry.rs`'s registrars nor `builtin_backends.toml`. That gap is exactly how `S85`
/// lived in `brew.rs` while a gate named "every backend pins a version or says why" passed —
/// `every_hand_written_backend_answers_the_pin_question` below is what closed it.
#[test]
fn every_reason_in_the_program_s_ledger_is_worth_printing() {
    let names = shall::backends::capability::backends_that_cannot_pin();
    assert!(
        names.len() >= 20,
        "only {} backend(s) in the ledger; it has stopped covering the hand-written ones",
        names.len()
    );
    for name in names {
        let why = shall::backends::capability::cannot_pin_reason(name)
            .expect("a name from the ledger has a reason in the ledger");
        assert!(
            why.len() > 20,
            "`{name}` is exempt with no reason worth reading: {why:?}"
        );
        assert!(
            !why.ends_with('.'),
            "`{name}`'s reason is a clause the refusal embeds mid-sentence, not its own \
             sentence: {why:?}"
        );
    }
}

/// **The gap `S85` lived in.** The two scans above read `registry.rs` and
/// `builtin_backends.toml`, and a backend that writes its own `Installable` is in neither — so a
/// gate that claimed to cover "every backend" was blind to `brew`, which did not merely drop a
/// pin but *invented* one, built `tokei@14.0.0`, and failed for ever on a version `lock` had
/// written by itself.
///
/// Source-scanned for the same reason the others are: these modules are compiled everywhere but
/// only *register* behind `cfg!(target_os = …)`, so asking the registry would ask whichever host
/// the suite happens to run on.
#[test]
fn every_hand_written_backend_answers_the_pin_question() {
    // Every module under `src/backends/` that implements `Installable` itself. Listed rather
    // than globbed: a new backend file should make somebody answer this question on purpose.
    const HAND_WRITTEN: &[&str] = &[
        "appimage",
        "brew",
        "btrfs",
        "emacs",
        "flatpak",
        "github",
        "go",
        "link",
        "mise",
        "nix",
        "psresource",
        "service",
        "setting",
        "snap",
        "storage",
        "vscode",
        "web",
    ];
    let mut unanswered: Vec<String> = Vec::new();
    for module in HAND_WRITTEN {
        let path = root().join(format!("src/backends/{module}.rs"));
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{} is in this repo: {e}", path.display()));
        assert!(
            src.contains("impl Installable for"),
            "`{module}` is listed as hand-written and implements no Installable — the list has \
             drifted from the modules"
        );
        // Omitting the method takes the trait's default (`false`), which is the safe answer and
        // the one that owes a reason. `storage` builds three backends and `firewall` one, so the
        // module name is not always the backend name — the ledger is asked per backend below.
        if src.contains("fn pins_version") {
            continue;
        }
        for backend in backends_named_by(module) {
            if shall::backends::capability::cannot_pin_reason(&backend).is_none() {
                unanswered.push(backend);
            }
        }
    }
    assert!(
        unanswered.is_empty(),
        "these hand-written backends silently drop a declared `@version=`: {unanswered:?}. \
         Either implement `pins_version`, or add a row to `capability::CANNOT_PIN_VERSION` \
         saying why the manager cannot be asked."
    );
}

/// The backend names a hand-written module registers. All but `storage` name one; it builds both
/// `zfs` and `lvm`, while `btrfs` has a module of its own.
fn backends_named_by(module: &str) -> Vec<String> {
    match module {
        "storage" => vec!["zfs".into(), "lvm".into()],
        other => vec![other.to_string()],
    }
}

/// The ratchet. `COULD_PIN_AND_DOES_NOT` is a list of live defects, not a list of decisions.
#[test]
fn the_backends_that_could_pin_and_do_not_only_get_fewer() {
    assert!(
        COULD_PIN_AND_DOES_NOT.is_empty(),
        "{} backend(s) take a version Shall does not send, which means a declared `@version=` \
         there installs the wrong version and reports success: {:?}. Build the pin, or move it \
         to `capability::CANNOT_PIN_VERSION` with the reason it cannot be built.",
        COULD_PIN_AND_DOES_NOT.len(),
        COULD_PIN_AND_DOES_NOT
            .iter()
            .map(|(k, _)| *k)
            .collect::<Vec<_>>()
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
