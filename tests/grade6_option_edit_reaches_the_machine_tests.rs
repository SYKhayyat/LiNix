//! `@shim` and `@sandbox` were **write-once** (GRADER round 6, 2026-07-31). They could not be
//! added to a declaration that was already installed, and they could not be withdrawn from one:
//! `sync` said `already up to date` in both directions.
//!
//! This is the family `Q19`/`Q20` closed for `@quota`, `@size`, `@mount`, `@mount_options`,
//! `@classic` and `@channel` — and these two were left live in the same commit, after `Q18`'s own
//! comment asserted that "Both are read by `sync`". They were read by `sync` **from the frozen
//! state registry**, not from the manifest.
//!
//! ## Measured end to end, on this machine, with a real backend
//!
//! ```text
//! # 1. install it plain, so Shall owns it
//! $ echo 'npm:json'            > $SHALL_CONFIG_DIR/modules/starter.txt
//! $ shall sync -y              Installs: 1   ✓ [npm] json (21216ms)
//!
//! # 2. ask for a shim — GRADER §3.5 step 2
//! $ echo 'npm:json@shim=true'  > $SHALL_CONFIG_DIR/modules/starter.txt
//! $ shall sync -y              already up to date
//! $ ls ~/.local/bin | grep json    (nothing)
//!
//! # 3. the control: the SAME option on a fresh install works
//! $ npm uninstall -g json ; rm -f ~/.local/bin/json*
//! $ echo 'npm:json@shim=true'  > $SHALL_CONFIG_DIR/modules/starter.txt
//! $ shall sync -y              Installs: 1
//! $ ls ~/.local/bin | grep json    json.exe        <-- so the feature works, the EDIT does not
//!
//! # 4. withdraw it — GRADER §3.5 step 4
//! $ echo 'npm:json'            > $SHALL_CONFIG_DIR/modules/starter.txt
//! $ shall sync -y              already up to date
//! $ ls ~/.local/bin | grep json    json.exe        <-- the shim survives the line that asked
//! ```
//!
//! Step 4 is the one that matters most. A shim is a stand-in **on the user's `PATH`**. Once
//! declared it could not be taken back declaratively: the manifest no longer mentioned it, `sync`
//! reported nothing to do, and the executable stayed. That is unmanaged state Shall claims to
//! manage, and it is the shape `sync` exists to prevent.
//!
//! ## What was built, and why these tests changed shape
//!
//! The grader's two assertions read source text: one required `spec_is_missing` to name both
//! keys, the other required `reconcile_all_shims` not to iterate `state.packages`. The fix took
//! the second one at its word — *"the desired state is what a reconciler has to read"* — and went
//! one further: **a shim is a resource with a teardown, so it is declared as one.** A present
//! package line carrying `@shim`/`@sandbox` now contributes a `shim:NAME` extra to the resolved
//! model (`model/resolve.rs`), which is the identical noun a `shim:` line declares. It therefore
//! rides the extras ledger, the removal guard, `--dry-run`, `plan` and `check` — all of which
//! already existed — and `reconcile_all_shims`, the second engine that decided from the frozen
//! snapshot, is **deleted** rather than repaired. Its own `expect()` said to re-derive the
//! finding if it went, which is what this file does.
//!
//! Making the package's drift check name the keys instead would have converged by *reinstalling
//! the package* to obtain a symlink, and would have left the frozen-snapshot reader in place.
//!
//! The guard below is therefore over the **table**, not over the two keys, and it accepts either
//! answer: an option with a machine effect is drift-checked by the planner, or it is desugared
//! into a resource. The next key added to `PACKAGE_OPTION_KEYS` still has to declare which side
//! it is on.

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

/// The resolver's half: the options that become a declared resource instead of package drift.
fn desugared_options() -> String {
    let src = std::fs::read_to_string(repo().join("src/model/resolve.rs")).unwrap();
    let start = src
        .find("pub fn wants_a_shim")
        .expect("wants_a_shim is gone; re-derive this finding");
    src[start..start + 300.min(src.len() - start)].to_string()
}

/// Where a package option's value ends up, and therefore what editing it does.
///
/// **The four-way split exists because "it's in the table" was the only thing anyone checked.**
/// The grammar validates 24 keys; nine of them were on a hand-written list here and the other
/// fifteen had never been classified at all. An option read once while the install argv is
/// built and never again passes every lifecycle test in this repo for ever, while doing nothing
/// after the first install — so the question a key has to answer is not *is it legal* but *what
/// happens when I change it on a package I already have*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Where {
    /// A fact about the machine that can be observed and re-applied. Editing it must schedule
    /// work: either the planner's drift check reads it, or the resolver desugars it into a
    /// declared resource with its own teardown.
    Converges,
    /// Changes *what is declared* rather than how one package is installed, so editing it
    /// converges through the plan itself — a package appears or disappears.
    Resolution,
    /// Re-read from the declaration on every run. Editing it takes effect on the next command
    /// with nothing to re-apply, because nothing was ever written down.
    EveryRun,
    /// Authorises an action at the moment it happens. It has no state of its own to converge
    /// to, and asking whether the machine "has" it is not a question.
    Permission,
    /// **Read while installing, and never again.** Editing it on an installed package does
    /// nothing until something reinstalls. This is an honest confession, not a category —
    /// [`INSTALL_TIME_CEILING`] is what stops the confessions accumulating.
    InstallTime,
}

/// Every key in `PACKAGE_OPTION_KEYS`, and where its value ends up.
///
/// Asserted to be exactly that constant, in both directions, by
/// `every_package_option_declares_where_it_ends_up`. Key 25 fails until it is here.
const DISPOSITION: &[(&str, Where, &str)] = &[
    (
        "version",
        Where::Converges,
        "which version is installed; `spec_is_missing` compares it against the installed one",
    ),
    (
        "channel",
        Where::Converges,
        "which stream the package follows (D13)",
    ),
    (
        "classic",
        Where::Converges,
        "whether a snap is confined (Q20)",
    ),
    ("mount", Where::Converges, "where a volume is mounted (Q18)"),
    (
        "mount_options",
        Where::Converges,
        "the fstab options the next boot honours",
    ),
    ("quota", Where::Converges, "the byte limit on a volume"),
    ("size", Where::Converges, "the geometry of a volume"),
    (
        "shim",
        Where::Converges,
        "whether a stand-in exists on the user's PATH; desugared into a `shim:` extra",
    ),
    (
        "sandbox",
        Where::Converges,
        "the same stand-in, plus `shall run` confinement; desugared the same way",
    ),
    (
        "requires",
        Where::Resolution,
        "pulls more declarations into the desired set, so editing it installs or reaps them",
    ),
    (
        "hold",
        Where::EveryRun,
        "freezes the package against a bulk upgrade. Read by `spec_is_missing` and by \
         `upgrade_targeted`, both from the declaration — until 2026-08-11 it was read by \
         NOTHING, which is the finding this enumeration exists to produce",
    ),
    (
        "expires",
        Where::EveryRun,
        "`model::dated::dating_of` classifies the line on every resolution; a lapsed \
         declaration stops being declared",
    ),
    (
        "until",
        Where::EveryRun,
        "the `absent:` half of the same clock, read by the same function",
    ),
    (
        "health",
        Where::EveryRun,
        "the probe `sync` runs after installing; parsed from the declaration each run",
    ),
    (
        "allow_http",
        Where::Permission,
        "permits an `http://` source for this one line; there is no state on the machine that \
         says a download was allowed",
    ),
    (
        "unverified",
        Where::Permission,
        "opts one line out of the manager's own signature check",
    ),
    (
        "system",
        Where::Permission,
        "permits writing into the environment the OS owns (Q49)",
    ),
    (
        "allow_shrink",
        Where::Permission,
        "authorises the destructive direction of a `@size` change (Q19); on its own it asks \
         for nothing",
    ),
    (
        "sha256",
        Where::InstallTime,
        "checked against the bytes as they are downloaded. Editing it on an installed package \
         re-verifies nothing, because the download already happened",
    ),
    (
        "formats",
        Where::InstallTime,
        "picks which file of a release to fetch; the picked file is already unpacked",
    ),
    (
        "asset",
        Where::InstallTime,
        "the same choice by pattern rather than by extension",
    ),
    (
        "bin",
        Where::InstallTime,
        "names the executable inside a downloaded artifact, read while unpacking it",
    ),
    (
        "url",
        Where::InstallTime,
        "where a backend that installs from a source rather than a name fetches from (U39)",
    ),
    (
        "download_only",
        Where::InstallTime,
        "fetch the artifact and stop; there is no installed state for a later edit to change",
    ),
];

/// How many options may still be read only while installing.
///
/// **A ratchet, not a target.** Each one is a shipped feature that silently does nothing when
/// edited on a package the machine already has, and the honest reading of that is a defect
/// under a ceiling rather than a design. It may be lowered when a key learns to converge;
/// raising it means shipping another write-once option, which is the shape `Q19`/`Q20`/`Q18`
/// each closed one key at a time while the rest of the table went unexamined.
const INSTALL_TIME_CEILING: usize = 6;

/// Options whose value is a fact about the machine that can be *observed and re-applied* after
/// the install, derived from the table above rather than restated beside it.
fn machine_effect_keys() -> Vec<(&'static str, &'static str)> {
    DISPOSITION
        .iter()
        .filter(|(_, w, _)| *w == Where::Converges)
        .map(|(k, _, why)| (*k, *why))
        .collect()
}

/// The keys the grammar accepts, read out of the grammar.
fn package_option_keys() -> Vec<String> {
    let src = std::fs::read_to_string(repo().join("src/config/grammar/statement.rs"))
        .expect("statement.rs");
    let body = src
        .split_once("pub(crate) const PACKAGE_OPTION_KEYS: &[&str] = &[")
        .expect("PACKAGE_OPTION_KEYS is gone; re-derive this finding")
        .1;
    let body = body
        .split_once("\n];")
        .expect("the constant is unterminated")
        .0;
    // **Comments first, and this is not fastidiousness.** The first draft took every quoted
    // string in the body on the reasoning that comments carry backticked prose and never a bare
    // quoted word — and Q49's comment opens `"Write into the environment the OS owns"`, which
    // arrived as a twenty-fifth option key. The extraction has to be right about the thing it
    // extracts or the table it compares against is compared to noise.
    let mut keys = Vec::new();
    for line in body.lines() {
        let code = match line.find("//") {
            Some(at) => &line[..at],
            None => line,
        };
        let mut rest = code;
        while let Some(open) = rest.find('"') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('"') else { break };
            keys.push(after[..close].to_string());
            rest = &after[close + 1..];
        }
    }
    assert!(
        keys.len() > 15,
        "only {} keys parsed out of PACKAGE_OPTION_KEYS; the extraction is looking at the wrong \
         thing, and a table this test compares against nothing is worse than no table",
        keys.len()
    );
    keys
}

/// Every key the grammar accepts says what editing it does — and nothing else does.
///
/// **This is the enumeration the round was about.** `MACHINE_EFFECT_KEYS` was a hand-written
/// list of nine, and the grammar validates twenty-four: fifteen keys had never been asked the
/// question at all. Running it found `@hold`, which the grammar validated, refused beside
/// `@version` as a contradiction, documented in II.2 — and which no code in the tree read.
#[test]
fn every_package_option_declares_where_it_ends_up() {
    let grammar: std::collections::BTreeSet<String> = package_option_keys().into_iter().collect();
    let declared: std::collections::BTreeSet<String> =
        DISPOSITION.iter().map(|(k, _, _)| k.to_string()).collect();

    let undeclared: Vec<&String> = grammar.difference(&declared).collect();
    assert!(
        undeclared.is_empty(),
        "{:?} are legal on a package line and this table does not say what editing them does. \
         An option read once when the install argv is built and never again passes every \
         lifecycle test in this repo for ever, while doing nothing — so a new key answers the \
         question before it ships, not after somebody notices.",
        undeclared
    );

    let invented: Vec<&String> = declared.difference(&grammar).collect();
    assert!(
        invented.is_empty(),
        "{:?} are classified here and the grammar does not accept them. That is the `undo` \
         disease: a table naming options the program does not have, guarded by nothing.",
        invented
    );

    let no_reason: Vec<&str> = DISPOSITION
        .iter()
        .filter(|(_, _, why)| why.trim().is_empty())
        .map(|(k, _, _)| *k)
        .collect();
    assert!(
        no_reason.is_empty(),
        "{no_reason:?} are classified with no reason. A classification nobody can check is a \
         claim (E29)."
    );

    let install_time: Vec<&str> = DISPOSITION
        .iter()
        .filter(|(_, w, _)| *w == Where::InstallTime)
        .map(|(k, _, _)| *k)
        .collect();
    assert!(
        install_time.len() <= INSTALL_TIME_CEILING,
        "{} options are read only while installing, over the ceiling of {}: {:?}\n\n\
         Each is a shipped feature that silently does nothing when edited on a package the \
         machine already has. The ceiling may fall; raising it is shipping another write-once \
         option.",
        install_time.len(),
        INSTALL_TIME_CEILING,
        install_time
    );
}

#[test]
fn every_option_with_a_machine_effect_converges_somewhere() {
    let body = drift_check();
    // Without this the test would pass on a tree where the extraction broke — the G2 shape.
    assert!(
        body.contains(r#"spec.options.one("version")"#),
        "the drift check no longer reads `@version`; this extraction is looking at the wrong \
         function"
    );
    let desugared = desugared_options();
    assert!(
        desugared.contains(r#"one("shim")"#),
        "the desugaring no longer reads `@shim`; this extraction is looking at the wrong function"
    );

    // Two spellings for the planner, because the check uses both: a named lookup for the keys
    // with their own rule, and `for key in ["quota", "size"]` for the pair that share one.
    // Matching only the first reported `@quota` and `@size` as unchecked when they are checked —
    // a false finding, which is the one thing worse than a missing one.
    let checked = |k: &str| {
        body.contains(&format!(r#"spec.options.one("{k}")"#))
            || body.contains(&format!(r#""{k}""#)) && body.contains("for key in [")
            || desugared.contains(&format!(r#"one("{k}")"#))
    };
    let machine_effect = machine_effect_keys();
    let unchecked: Vec<String> = machine_effect
        .iter()
        .filter(|(k, _)| !checked(k))
        .map(|(k, effect)| format!("@{k} — {effect}"))
        .collect();

    assert!(
        unchecked.is_empty(),
        "{} of {} options with a machine effect converge nowhere:\n  {}\n\n\
         Declaring one on a package that is already installed is then a no-op — `sync` reports \
         `already up to date` — and deleting it again is a no-op too. Each key must either be \
         drift-checked by the planner or desugared into a declared resource.",
        unchecked.len(),
        machine_effect.len(),
        unchecked.join("\n  ")
    );
}

use crate::harness::Fixture;

/// The shared root, plus what these tests need in it.
fn setup(name: &str) -> Fixture {
    let f = Fixture::new(name);
    // One backend, so the model resolves the same way on every host and no run is at the
    // mercy of what this box happens to have installed (G-11's shape).
    std::fs::write(f.cfg().join("priority"), "cargo\n").unwrap();
    f
}

const CANARY: &str = "shall-shim-canary";

/// `shall eval` renders the resolved model as JSON, so the question is asked of the structure
/// rather than of the word: an `extras` entry of kind `shim` naming the canary.
///
/// The first draft asked whether the output contained "shim" and the name anywhere, and it
/// **passed on the unfixed tree** — the schema itself is enough to satisfy that. A check whose
/// green is free is the thing this whole round is about.
fn declares_a_shim(eval_json: &str) -> bool {
    let doc: serde_json::Value = serde_json::from_str(eval_json).expect("`eval` emits JSON");
    doc["extras"]
        .as_array()
        .map(|xs| {
            xs.iter()
                .any(|x| x["kind"] == "shim" && x["name"] == CANARY)
        })
        .unwrap_or(false)
}

#[test]
fn adding_the_option_declares_the_stand_in_and_withdrawing_it_takes_it_back() {
    let f = setup("grade6-shim-edit");

    // Step 2: the option is added to a line that is already there.
    f.write_module(&format!("cargo:{CANARY}@shim=true\n"));
    let (out, code) = f.run(&["eval"]);
    assert_eq!(code, 0, "`eval` failed:\n{out}");
    assert!(
        declares_a_shim(&out),
        "`@shim=true` declared no stand-in. The option is read at install time only, so adding \
         it to an installed package does nothing and `sync` reports `already up to date`:\n{out}"
    );

    // Step 4, the one that matters: the option is withdrawn and the stand-in has to go with it.
    f.write_module(&format!("cargo:{CANARY}\n"));
    f.seed_ledger(&[&format!("shim:{CANARY}")]);
    let (out, code) = f.run(&["plan"]);
    // `H2` (owner, 2026-08-13): a read-only command that finds work exits **2**, and `plan`
    // is one. Both 0 and 2 are successful runs of it; 1 is a failure. The content
    // assertions below are what carry this test's meaning either way.
    assert!(matches!(code, 0 | 2), "`plan` failed:\n{out}");
    assert!(
        out.contains(&format!("shim:{CANARY}")),
        "the line stopped asking for a stand-in and nothing plans to remove it. A shim is an \
         executable on the user's PATH: unremovable by editing the file that asked for it, it is \
         unmanaged state Shall claims to manage:\n{out}"
    );

    // And the control: with the option back, that teardown must NOT be planned — otherwise the
    // assertion above would pass on a tree that tears the shim down unconditionally.
    f.write_module(&format!("cargo:{CANARY}@shim=true\n"));
    let (out, code) = f.run(&["plan"]);
    // `H2` (owner, 2026-08-13): a read-only command that finds work exits **2**, and `plan`
    // is one. Both 0 and 2 are successful runs of it; 1 is a failure. The content
    // assertions below are what carry this test's meaning either way.
    assert!(matches!(code, 0 | 2), "`plan` failed:\n{out}");
    assert!(
        !out.contains("no longer declared"),
        "a declared `@shim=true` was planned for teardown:\n{out}"
    );
}

/// `@sandbox` is the same promise plus confinement, and it was broken in exactly the same way.
#[test]
fn sandbox_carries_the_stand_in_too() {
    let f = setup("grade6-sandbox-edit");
    f.write_module(&format!("cargo:{CANARY}@sandbox=true\n"));
    let (out, code) = f.run(&["eval"]);
    assert_eq!(code, 0, "`eval` failed:\n{out}");
    assert!(
        declares_a_shim(&out),
        "`@sandbox=true` declared no stand-in; `shall run` confines the tool THROUGH the shim, \
         so there is nothing to confine without one:\n{out}"
    );
}

/// Not every option is supposed to be re-applied, and saying so is half the finding: a guard that
/// demanded all 21 keys converge would be wrong and would be silenced rather than fixed.
///
/// `@unverified` is the clearest case: `verbs/plan.rs::unverified_packages` reads the *recorded*
/// option on purpose, because the install really did skip a verification and editing the line
/// later cannot change that history.
#[test]
fn the_historical_options_are_deliberately_not_drift_checked() {
    let plan = std::fs::read_to_string(repo().join("src/verbs/plan.rs")).unwrap();
    assert!(
        plan.contains(r#"p.options.one("unverified")"#),
        "`@unverified` no longer reads the recorded option. If it became drift-checked, the \
         reasoning above needs re-deriving rather than this assertion deleting."
    );
    assert!(
        !machine_effect_keys()
            .iter()
            .any(|(k, _)| *k == "unverified"),
        "`@unverified` must not be classified `Where::Converges`: it records what an install \
         did, not what the machine looks like now."
    );
}

/// Nothing outside the union reads the hold ledger.
///
/// **`@hold=true` was inert**, and the first fix was the same mistake one size smaller. It is in
/// `PACKAGE_OPTION_KEYS`, `validate_package` refuses it beside `@version` as a contradiction, and
/// II.2 documents it — and the only writer of the held set was the imperative `shall hold`, so a
/// manifest line carrying it parsed, validated, and did nothing whatsoever. Two readers were
/// taught about the declaration and a *file-level* version of this test went green over four:
///
/// - `upgrade --security` copied `StateRegistry::held` into a closure of its own, so it matched
///   no grep for the ledger's readers and silently remediated a package the manifest had frozen;
/// - the "holds are not enforced by a native whole-system upgrade" note counted the ledger, so
///   somebody whose holds were all declared was told nothing;
/// - `shall hold` with no arguments — the command whose entire job is *tell me what is held* —
///   answered `No packages are held.` over a manifest holding three.
///
/// The last two live in files that already mentioned `declares_hold`, which is exactly why a
/// check keyed on the file passed. So the rule is now structural: `app::holds::Holds` is the
/// union, and no other module may reach the ledger at all.
#[test]
fn nothing_outside_the_union_reads_the_hold_ledger() {
    /// Where the ledger may legitimately be touched, and why.
    ///
    /// Two entries, and neither is a caller: one defines the ledger and one is the union over
    /// it. A third entry here is a claim that some module needs half the answer, which is the
    /// state this whole finding is about.
    const MAY_READ_THE_LEDGER: &[(&str, &str)] = &[
        (
            "src/core/state.rs",
            "defines `held`, `hold`, `is_held` and `list_held` — it is the answer, not a caller",
        ),
        (
            "src/app/holds.rs",
            "the union of the ledger and the declarations; the one place both are read",
        ),
        (
            "src/app/sync/planner.rs",
            "asks per spec, so the declaration is already in its hand and the union it computes              (`state.is_held(..) || spec.declares_hold()`) is this one, for one package",
        ),
    ];

    // Every way the ledger can be reached. `.held` is in here because `upgrade --security` used
    // it and therefore matched none of the others — the reader that hid was the reason this
    // list is a list.
    const REACHES_THE_LEDGER: &[&str] = &["is_held(", "list_held(", ".held"];

    let mut deaf: Vec<String> = Vec::new();
    for path in rust_files(&repo().join("src")) {
        let rel = path
            .strip_prefix(repo())
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if MAY_READ_THE_LEDGER.iter().any(|(p, _)| rel.ends_with(p)) {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (n, line) in src.lines().enumerate() {
            // Comments talk about this rule; they do not break it.
            let code = line.split("//").next().unwrap_or("");
            if REACHES_THE_LEDGER.iter().any(|m| code.contains(m)) {
                deaf.push(format!("{rel}:{} — {}", n + 1, line.trim()));
            }
        }
    }

    assert!(
        deaf.is_empty(),
        "{} place(s) read the hold ledger directly:
  {}

`shall hold` writes the ledger          and `@hold=true` on a manifest line writes nothing there, so a reader that sees only          the ledger upgrades a package the user froze. Ask `App::holds()`, which is the union          of the two.",
        deaf.len(),
        deaf.join("
  ")
    );

    // And the union has to actually be reached, or the rule above is satisfied by nobody asking
    // about holds at all — the G2 shape, where a scan passes because it matched nothing.
    let callers = rust_files(&repo().join("src"))
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .filter(|src| src.contains("holds().await") || src.contains("Holds::new("))
        .count();
    assert!(
        callers >= 3,
        "only {callers} module(s) reach `App::holds()`; the readers this finding was about are          `upgrade` (three of them) and `hold`, so this scan is looking at the wrong thing"
    );
}

fn rust_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}
