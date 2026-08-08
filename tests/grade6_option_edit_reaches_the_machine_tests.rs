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
//! # 1. install it plain, so LiNix owns it
//! $ echo 'npm:json'            > $LINIX_CONFIG_DIR/modules/starter.txt
//! $ linix sync -y              Installs: 1   ✓ [npm] json (21216ms)
//!
//! # 2. ask for a shim — GRADER §3.5 step 2
//! $ echo 'npm:json@shim=true'  > $LINIX_CONFIG_DIR/modules/starter.txt
//! $ linix sync -y              already up to date
//! $ ls ~/.local/bin | grep json    (nothing)
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
//! $ ls ~/.local/bin | grep json    json.exe        <-- the shim survives the line that asked
//! ```
//!
//! Step 4 is the one that matters most. A shim is a stand-in **on the user's `PATH`**. Once
//! declared it could not be taken back declaratively: the manifest no longer mentioned it, `sync`
//! reported nothing to do, and the executable stayed. That is unmanaged state LiNix claims to
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

use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Options whose value is a fact about the machine that can be *observed and re-applied* after
/// the install. Each one is a promise that editing the line changes the box.
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
    let unchecked: Vec<String> = MACHINE_EFFECT_KEYS
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
        MACHINE_EFFECT_KEYS.len(),
        unchecked.join("\n  ")
    );
}

/// The other end of the same defect, stated as behaviour: what the module says **today** is what
/// decides whether the stand-in exists. Driven through the shipped binary, not the source.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let f = Self { root };
        let (out, code) = f.run(&["init"]);
        assert_eq!(code, 0, "the fixture's own `init` failed:\n{out}");
        // One backend, so the model resolves the same way on every host and no run is at the
        // mercy of what this box happens to have installed (G-11's shape).
        std::fs::write(f.cfg().join("priority"), "cargo\n").unwrap();
        f
    }

    fn cfg(&self) -> PathBuf {
        self.root.join("config")
    }

    fn run(&self, args: &[&str]) -> (String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_linix"))
            .args(args)
            .current_dir(&self.root)
            .env("LINIX_CONFIG_DIR", self.cfg())
            .env("LINIX_DATA_DIR", self.root.join("data"))
            .stdin(std::process::Stdio::null())
            .output()
            .expect("the binary should run");
        (
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
            out.status.code().unwrap_or(-1),
        )
    }

    fn write_module(&self, body: &str) {
        std::fs::write(self.cfg().join("modules/starter.txt"), body).unwrap();
    }

    /// Pre-seed the applied-extras ledger, so the withdrawal case can be put in the
    /// "already applied" state without installing anything.
    fn seed_ledger(&self, keys: &[&str]) {
        let locks = self.cfg().join("locks");
        std::fs::create_dir_all(&locks).unwrap();
        let body = format!(
            "applied = [{}]\n",
            keys.iter()
                .map(|k| format!("{:?}", k))
                .collect::<Vec<_>>()
                .join(", ")
        );
        std::fs::write(locks.join("extras.toml"), body).unwrap();
    }
}

const CANARY: &str = "linix-shim-canary";

/// `linix eval` renders the resolved model as JSON, so the question is asked of the structure
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
    let f = Fixture::new("grade6-shim-edit");

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
    assert_eq!(code, 0, "`plan` failed:\n{out}");
    assert!(
        out.contains(&format!("shim:{CANARY}")),
        "the line stopped asking for a stand-in and nothing plans to remove it. A shim is an \
         executable on the user's PATH: unremovable by editing the file that asked for it, it is \
         unmanaged state LiNix claims to manage:\n{out}"
    );

    // And the control: with the option back, that teardown must NOT be planned — otherwise the
    // assertion above would pass on a tree that tears the shim down unconditionally.
    f.write_module(&format!("cargo:{CANARY}@shim=true\n"));
    let (out, code) = f.run(&["plan"]);
    assert_eq!(code, 0, "`plan` failed:\n{out}");
    assert!(
        !out.contains("no longer declared"),
        "a declared `@shim=true` was planned for teardown:\n{out}"
    );
}

/// `@sandbox` is the same promise plus confinement, and it was broken in exactly the same way.
#[test]
fn sandbox_carries_the_stand_in_too() {
    let f = Fixture::new("grade6-sandbox-edit");
    f.write_module(&format!("cargo:{CANARY}@sandbox=true\n"));
    let (out, code) = f.run(&["eval"]);
    assert_eq!(code, 0, "`eval` failed:\n{out}");
    assert!(
        declares_a_shim(&out),
        "`@sandbox=true` declared no stand-in; `linix run` confines the tool THROUGH the shim, \
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
        !MACHINE_EFFECT_KEYS.iter().any(|(k, _)| *k == "unverified"),
        "`@unverified` must not be in the machine-effect table: it records what an install did, \
         not what the machine looks like now."
    );
}
