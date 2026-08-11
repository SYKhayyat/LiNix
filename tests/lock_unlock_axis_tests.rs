//! Z2 (owner ruling, 2026-08-03): **`lock` and `unlock` name the axis they act on.**
//!
//! Three ledgers were all called "the lock", and the two verbs acted on different ones:
//!
//! | command | what it touched |
//! |---|---|
//! | `lock` | `locks/versions.json`, and every script approval in `locks/hooks.toml` |
//! | `unlock` | `locks/bare.HOST.toml` — which *manager* an unpinned bare name resolved to |
//!
//! So the obvious undo for `lock` discarded the recorded backend resolution instead, and
//! `unlock`'s own help stated the consequence — *"sync uninstalls the cargo copy"*. A harmless
//! command's obvious inverse could uninstall software.
//!
//! What this file pins is the property that fixes it: **the axis you name is the axis that
//! changes, and no other ledger moves.** Every axis is asserted in both directions, and each
//! assertion checks the two ledgers it did *not* name — the sibling check is the whole point,
//! because the bug was never in the ledger the command was about.

use std::path::{Path, PathBuf};
use std::process::Command;

struct Fresh {
    dir: PathBuf,
}

impl Fresh {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fresh = Self { dir };
        let (out, code) = fresh.run(&["init"]);
        assert_eq!(code, 0, "the fixture's own `init` failed:\n{out}");
        fresh.plant_all_three();
        fresh
    }

    fn run(&self, args: &[&str]) -> (String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_shall"))
            .args(args)
            .env("SHALL_CONFIG_DIR", self.dir.join("config"))
            .env("SHALL_DATA_DIR", self.dir.join("data"))
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

    fn locks(&self) -> PathBuf {
        self.dir.join("config").join("locks")
    }

    /// One entry in each of the three ledgers, written directly. Planting rather than earning
    /// them keeps this file about the verbs: `lock versions` needs managed packages and
    /// `lock backends` needs a manager that answers, and neither is what Z2 was about.
    fn plant_all_three(&self) {
        std::fs::create_dir_all(self.locks()).unwrap();
        std::fs::write(
            self.locks().join("versions.json"),
            r#"{"locks":{"apt:curl":"7.81.0","cargo:ripgrep":"14.1.0"}}"#,
        )
        .unwrap();
        // The bare-name lock is per host, so its filename is asked for rather than guessed.
        std::fs::write(
            shall::core::BareLock::path_in(&self.locks()),
            "[resolved]\nripgrep = \"cargo\"\njq = \"apt\"\n",
        )
        .unwrap();
        std::fs::write(
            shall::core::hook_lock::HookLedger::path_in(&self.locks()),
            "[approvals]\n\"after_install:nginx\" = \"aaa\"\n\"adapters:backends.toml\" = \"bbb\"\n",
        )
        .unwrap();
    }

    fn versions(&self) -> String {
        std::fs::read_to_string(self.locks().join("versions.json")).unwrap_or_default()
    }

    fn backends(&self) -> String {
        std::fs::read_to_string(shall::core::BareLock::path_in(&self.locks())).unwrap_or_default()
    }

    fn scripts(&self) -> String {
        std::fs::read_to_string(shall::core::hook_lock::HookLedger::path_in(&self.locks()))
            .unwrap_or_default()
    }
}

/// The bug, in one assertion. `shall unlock ripgrep` was the obvious undo for `shall lock`; it
/// forgot a backend resolution, and the next sync would have uninstalled the cargo copy. A name
/// where the axis goes is now refused, loudly, with the three axes named.
#[test]
fn a_bare_name_where_the_axis_goes_is_refused() {
    let fresh = Fresh::new("axis-bare-name");
    let (out, code) = fresh.run(&["unlock", "ripgrep"]);
    assert_ne!(code, 0, "`unlock ripgrep` was accepted:\n{out}");
    assert!(
        out.contains("versions") && out.contains("backends") && out.contains("scripts"),
        "the refusal did not name the three axes:\n{out}"
    );
    assert_eq!(
        fresh.backends(),
        "[resolved]\nripgrep = \"cargo\"\njq = \"apt\"\n",
        "a refused command still forgot a backend resolution"
    );
}

/// The same refusal on the other verb. `lock` and `unlock` are one grammar, and a rule that
/// held on one of them would be the next Z2.
#[test]
fn the_same_refusal_applies_to_lock() {
    let fresh = Fresh::new("axis-bare-name-lock");
    let (out, code) = fresh.run(&["lock", "ripgrep"]);
    assert_ne!(code, 0, "`lock ripgrep` was accepted:\n{out}");
}

#[test]
fn unlock_versions_drops_the_pins_and_leaves_the_other_two_alone() {
    let fresh = Fresh::new("axis-unlock-versions");
    let before_backends = fresh.backends();
    let before_scripts = fresh.scripts();

    let (out, code) = fresh.run(&["unlock", "versions"]);
    assert_eq!(code, 0, "{out}");

    assert!(
        !fresh.versions().contains("apt:curl"),
        "{}",
        fresh.versions()
    );
    assert!(
        !fresh.versions().contains("cargo:ripgrep"),
        "{}",
        fresh.versions()
    );
    assert_eq!(fresh.backends(), before_backends, "the backend lock moved");
    assert_eq!(fresh.scripts(), before_scripts, "the approvals moved");
}

/// Z2's exact symptom, inverted: the command that forgets backend resolutions must not touch
/// the version pins — and must not touch the approvals either, which nobody had noticed was
/// the same question.
#[test]
fn unlock_backends_forgets_resolutions_and_leaves_the_other_two_alone() {
    let fresh = Fresh::new("axis-unlock-backends");
    let before_versions = fresh.versions();
    let before_scripts = fresh.scripts();

    let (out, code) = fresh.run(&["unlock", "backends"]);
    assert_eq!(code, 0, "{out}");

    assert!(
        !fresh.backends().contains("ripgrep"),
        "{}",
        fresh.backends()
    );
    assert_eq!(fresh.versions(), before_versions, "the version pins moved");
    assert_eq!(fresh.scripts(), before_scripts, "the approvals moved");
}

#[test]
fn unlock_scripts_withdraws_approval_and_leaves_the_other_two_alone() {
    let fresh = Fresh::new("axis-unlock-scripts");
    let before_versions = fresh.versions();
    let before_backends = fresh.backends();

    let (out, code) = fresh.run(&["unlock", "scripts"]);
    assert_eq!(code, 0, "{out}");

    assert!(
        !fresh.scripts().contains("after_install:nginx"),
        "{}",
        fresh.scripts()
    );
    assert!(
        !fresh.scripts().contains("adapters:backends.toml"),
        "{}",
        fresh.scripts()
    );
    assert_eq!(fresh.versions(), before_versions, "the version pins moved");
    assert_eq!(fresh.backends(), before_backends, "the backend lock moved");
}

/// The owner's ruling on the bare form: it does all three. It is the one command in this family
/// that can move software, so it is the one you have to type without an axis to get.
#[test]
fn a_bare_unlock_releases_all_three() {
    let fresh = Fresh::new("axis-unlock-all");
    let (out, code) = fresh.run(&["unlock"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        !fresh.versions().contains("apt:curl"),
        "{}",
        fresh.versions()
    );
    assert!(
        !fresh.backends().contains("ripgrep"),
        "{}",
        fresh.backends()
    );
    assert!(
        !fresh.scripts().contains("after_install:nginx"),
        "{}",
        fresh.scripts()
    );
}

/// `unlock all` and a bare `unlock` are the same command spelled two ways, so they must do the
/// same thing — a default that drifts from the value it defaults to is its own bug.
#[test]
fn the_explicit_all_matches_the_bare_form() {
    let fresh = Fresh::new("axis-unlock-all-explicit");
    let (out, code) = fresh.run(&["unlock", "all"]);
    assert_eq!(code, 0, "{out}");
    assert!(!fresh.versions().contains("apt:curl"));
    assert!(!fresh.backends().contains("ripgrep"));
    assert!(!fresh.scripts().contains("after_install:nginx"));
}

/// Scoping, on every axis: the named entry goes and its neighbour in the same ledger stays.
/// One name per axis rather than one axis, because the scope is one shared rule and a rule
/// tested on one ledger is a rule tested nowhere.
#[test]
fn a_scope_takes_the_named_entry_and_leaves_its_neighbour() {
    let fresh = Fresh::new("axis-scope");

    let (out, code) = fresh.run(&["unlock", "versions", "curl"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        !fresh.versions().contains("apt:curl"),
        "{}",
        fresh.versions()
    );
    assert!(
        fresh.versions().contains("cargo:ripgrep"),
        "a scoped unpin took its neighbour too:\n{}",
        fresh.versions()
    );

    let (out, code) = fresh.run(&["unlock", "backends", "ripgrep"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        !fresh.backends().contains("ripgrep"),
        "{}",
        fresh.backends()
    );
    assert!(
        fresh.backends().contains("jq"),
        "a scoped forget took its neighbour too:\n{}",
        fresh.backends()
    );

    let (out, code) = fresh.run(&["unlock", "scripts", "nginx"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        !fresh.scripts().contains("after_install:nginx"),
        "{}",
        fresh.scripts()
    );
    assert!(
        fresh.scripts().contains("adapters:backends.toml"),
        "a scoped withdrawal took its neighbour too:\n{}",
        fresh.scripts()
    );
}

/// The whole key works where the tail does, so a user who has two entries with the same tail
/// can still say which one they meant.
#[test]
fn a_scope_accepts_the_whole_key_too() {
    let fresh = Fresh::new("axis-scope-full-key");
    let (out, code) = fresh.run(&["unlock", "versions", "apt:curl"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        !fresh.versions().contains("apt:curl"),
        "{}",
        fresh.versions()
    );
    assert!(fresh.versions().contains("cargo:ripgrep"));
}

/// A name that picks nothing out changes nothing and says so. Silence here is the failure mode
/// Principle I forbids: an unpin that unpinned nothing must not read like an unpin that worked.
#[test]
fn a_name_that_matches_nothing_changes_nothing_and_says_so() {
    let fresh = Fresh::new("axis-scope-miss");
    let before = fresh.versions();
    let (out, code) = fresh.run(&["unlock", "versions", "shall-never-pinned-zzz"]);
    assert_eq!(code, 0, "a miss is not an error:\n{out}");
    assert!(
        out.contains("nothing unpinned"),
        "the miss was silent:\n{out}"
    );
    assert_eq!(fresh.versions(), before);
}

/// `--list` reports and changes nothing, on the axis it was given and on the bare form.
#[test]
fn list_reports_every_axis_and_writes_nothing() {
    let fresh = Fresh::new("axis-list");
    let (before_v, before_b, before_s) = (fresh.versions(), fresh.backends(), fresh.scripts());

    let (out, code) = fresh.run(&["lock", "--list"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("apt:curl"),
        "the version pins are missing:\n{out}"
    );
    assert!(
        out.contains("ripgrep"),
        "the backend lock is missing:\n{out}"
    );
    assert!(
        out.contains("after_install:nginx"),
        "the approvals are missing:\n{out}"
    );

    let (scoped, code) = fresh.run(&["unlock", "backends", "--list"]);
    assert_eq!(code, 0, "{scoped}");
    assert!(scoped.contains("ripgrep"), "{scoped}");
    assert!(
        !scoped.contains("apt:curl"),
        "a scoped --list reported another axis:\n{scoped}"
    );

    assert_eq!(fresh.versions(), before_v, "--list wrote a version pin");
    assert_eq!(fresh.backends(), before_b, "--list wrote the backend lock");
    assert_eq!(fresh.scripts(), before_s, "--list wrote an approval");
}

/// A preview releases nothing, on every axis. `--dry-run lock` writing the ledgers is a bug
/// this repo has already had once.
#[test]
fn a_previewed_unlock_releases_nothing() {
    let fresh = Fresh::new("axis-dry-run");
    let (before_v, before_b, before_s) = (fresh.versions(), fresh.backends(), fresh.scripts());

    let (out, code) = fresh.run(&["--dry-run", "unlock"]);
    assert_eq!(code, 0, "{out}");

    assert_eq!(fresh.versions(), before_v, "a preview dropped a pin");
    assert_eq!(fresh.backends(), before_b, "a preview forgot a resolution");
    assert_eq!(fresh.scripts(), before_s, "a preview withdrew an approval");
}
