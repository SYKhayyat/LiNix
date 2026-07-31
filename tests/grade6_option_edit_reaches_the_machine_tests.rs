//! GRADER round 6, 2026-07-31 — RED. `@shim` and `@sandbox` are **write-once**. They cannot be
//! added to a declaration that is already installed, and they cannot be withdrawn from one.
//! `sync` says `already up to date` in both directions.
//!
//! This is the family `Q19`/`Q20` closed for `@quota`, `@size`, `@mount`, `@mount_options`,
//! `@classic` and `@channel` on 2026-07-31 — and these two were left live in the same commit,
//! after `Q18`'s own comment asserted that "Both are read by `sync`". They are read by `sync`
//! **from the frozen state registry**, not from the manifest.
//!
//! ## Measured end to end, on this machine, with a real backend
//!
//! ```text
//! # 1. install it plain, so LiNix owns it
//! $ echo 'npm:json'            > $LINIX_CONFIG_DIR/modules/starter.txt
//! $ linix sync -y              Installs: 1   ✓ [npm] json (21216ms)
//! $ grep options registry.json "options": {"__source": "...starter.txt:1", "__scopes": "..."}
//!
//! # 2. ask for a shim — GRADER §3.5 step 2
//! $ echo 'npm:json@shim=true'  > $LINIX_CONFIG_DIR/modules/starter.txt
//! $ linix sync -y              already up to date
//! $ ls ~/.local/bin | grep json    (nothing)
//! $ grep options registry.json     still no `shim` key
//!
//! # 3. the control: the SAME option on a fresh install works
//! $ npm uninstall -g json ; rm -f ~/.local/bin/json*
//! $ echo 'npm:json@shim=true'  > $LINIX_CONFIG_DIR/modules/starter.txt
//! $ linix sync -y              Installs: 1
//! $ ls ~/.local/bin | grep json    json.exe        <-- so the feature works, the EDIT does not
//!
//! # 4. withdraw it — GRADER §3.5 step 4
//! $ echo 'npm:json'            > $LINIX_CONFIG_DIR/modules/starter.txt
//! $ linix sync -y              already up to date
//! $ ls ~/.local/bin | grep json    json.exe        <-- the shim survives the line that asked for it
//! ```
//!
//! Step 4 is the one that matters most. A shim is a stand-in **on the user's `PATH`**. Once
//! declared it cannot be taken back declaratively: the manifest no longer mentions it, `sync`
//! reports nothing to do, and the executable stays. That is unmanaged state LiNix claims to
//! manage, and it is the shape `sync` exists to prevent.
//!
//! ## The mechanism
//!
//! `ChangePlanner::spec_is_missing` (`src/app/sync/planner.rs`) is the whole drift check. It
//! consults seven option keys. `reconcile_all_shims` (`src/app/sync/mod.rs:594`) decides whether
//! a shim is wanted from `pkg.options`, where `pkg` is a `ManagedPackage` out of the
//! `StateRegistry` — and `StateRegistry::add` is the only writer of that map, called only when a
//! package is installed. So a manifest edit that schedules no install can never change it.
//!
//! ## Why the shape matters more than the two keys
//!
//! A lifecycle is `install → list → remove` and by construction never edits a declaration, so an
//! option read once at install passes every lifecycle, every plan-smoke and every unit test for
//! ever. `Q19`/`Q20` were found by driving that fourth step by hand; these two were in the same
//! table and were not driven. The guard below is therefore over the *table*, not over the two
//! keys — so the next option added to `PACKAGE_OPTION_KEYS` has to declare which side it is on.

use std::path::PathBuf;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The body of the one function that decides whether a declared spec needs work.
fn drift_check() -> String {
    let src = std::fs::read_to_string(repo().join("src/app/sync/planner.rs")).unwrap();
    let start = src
        .find("async fn spec_is_missing")
        .expect("spec_is_missing is gone; re-derive this finding");
    let tail = &src[start..];
    let end = tail
        .find("\n    async fn ")
        .or_else(|| tail.find("\n    fn "))
        .unwrap_or(tail.len());
    tail[..end].to_string()
}

/// Options whose value is a fact about the machine that can be *observed and re-applied* after
/// the install. Each one is a promise that editing the line changes the box.
///
/// Seven of these were closed by Q18/Q19/Q20. Two were not, and they are in the same table, on
/// the same `sync`, with the same failure.
const MACHINE_EFFECT_KEYS: &[(&str, &str)] = &[
    ("version", "which version is installed"),
    ("channel", "which stream the package follows"),
    ("classic", "whether a snap is confined"),
    ("mount", "where a volume is mounted"),
    ("mount_options", "the fstab options the next boot honours"),
    ("quota", "the byte limit on a volume"),
    ("size", "the geometry of a volume"),
    ("shim", "whether a stand-in exists on the user's PATH"),
    (
        "sandbox",
        "whether a stand-in exists AND `linix run` is confined",
    ),
];

#[test]
fn every_option_with_a_machine_effect_is_checked_for_drift() {
    let body = drift_check();
    // Without this the test would pass on a tree where the extraction broke — the G2 shape.
    assert!(
        body.contains(r#"spec.options.get("version")"#),
        "the drift check no longer reads `@version`; this extraction is looking at the wrong \
         function"
    );

    // Two spellings, because the check uses both: a named lookup for the keys with their own
    // rule, and `for key in ["quota", "size"]` for the pair that share one. Matching only the
    // first reported `@quota` and `@size` as unchecked when they are checked — a false finding,
    // which is the one thing worse than a missing one.
    let checked = |k: &str| {
        body.contains(&format!(r#"spec.options.get("{k}")"#))
            || body.contains(&format!(r#""{k}""#)) && body.contains("for key in [")
    };
    let unchecked: Vec<String> = MACHINE_EFFECT_KEYS
        .iter()
        .filter(|(k, _)| !checked(k))
        .map(|(k, effect)| format!("@{k} — {effect}"))
        .collect();

    assert!(
        unchecked.is_empty(),
        "{} of {} options with a machine effect are invisible to the drift check:\n  {}\n\n\
         Declaring one on a package that is already installed is a no-op: `sync` reports \
         `already up to date`, and deleting it again is a no-op too. Measured with `npm:json` \
         and `@shim=true` — the shim is not created when the option is added, and not removed \
         when it is withdrawn, while the identical option on a FRESH install works.",
        unchecked.len(),
        MACHINE_EFFECT_KEYS.len(),
        unchecked.join("\n  ")
    );
}

/// The other end of the same defect, stated where it actually lives: the shim reconciler reads
/// the recorded options, and nothing ever refreshes them from the manifest.
#[test]
fn the_shim_reconciler_does_not_decide_from_a_frozen_snapshot() {
    let sync = std::fs::read_to_string(repo().join("src/app/sync/mod.rs")).unwrap();
    let start = sync
        .find("async fn reconcile_all_shims")
        .expect("reconcile_all_shims is gone; re-derive this finding");
    let body = &sync[start..start + 900.min(sync.len() - start)];

    // It iterates `state.packages` — a `ManagedPackage` whose `options` map is written once, by
    // `StateRegistry::add`, at install time.
    let reads_state = body.contains("state.packages") && body.contains("pkg.options.get");
    assert!(
        !reads_state,
        "reconcile_all_shims decides `needs_shim` from state.packages[].options.\n\n\
         `StateRegistry::add` is the only writer of that map and sync only calls it when a \
         package is installed, so a manifest that gains or loses `@shim=true` on an \
         already-installed package never reaches this decision. The desired state — what the \
         module says today — is what a reconciler has to read."
    );
}

/// Not every option is supposed to be re-applied, and saying so is half the finding: a guard that
/// demanded all 21 keys be drift-checked would be wrong and would be silenced rather than fixed.
///
/// `@unverified` is the clearest case and it was checked before this file was written:
/// `verbs/plan.rs::unverified_packages` reads the *recorded* option on purpose, because the
/// install really did skip a verification and editing the line later cannot change that history.
/// That is correct, it is documented where it happens, and it is not in the table above.
#[test]
fn the_historical_options_are_deliberately_not_drift_checked() {
    let plan = std::fs::read_to_string(repo().join("src/verbs/plan.rs")).unwrap();
    assert!(
        plan.contains(r#"p.options.get("unverified")"#),
        "`@unverified` no longer reads the recorded option. If it became drift-checked, the \
         reasoning above needs re-deriving rather than this assertion deleting."
    );
    assert!(
        !MACHINE_EFFECT_KEYS.iter().any(|(k, _)| *k == "unverified"),
        "`@unverified` must not be in the machine-effect table: it records what an install did, \
         not what the machine looks like now."
    );
}
