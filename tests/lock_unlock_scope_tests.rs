//! Z2 (owner ruling, 2026-08-03): **`lock` and `unlock` name what they act on.**
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
//! What this file pins is the property that fixes it: **what you name is what changes, and no
//! other ledger moves.** Every scope is asserted in both directions, and each assertion checks
//! the ledgers it did *not* name — the sibling check is the whole point, because the bug was
//! never in the ledger the command was about.
//!
//! J4 (owner ruling, 2026-08-16) widened the vocabulary from three axes to nine kinds in three
//! groups, with a sub-category below the kind and an exclusion list beside it. The property is
//! unchanged and the surface it has to hold over is much larger, so the granular forms are
//! asserted here too — against the same three planted ledgers, because "nothing else moved" is
//! the assertion that does not get easier as the vocabulary grows.

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
/// where the scope goes is now refused, loudly, with the vocabulary named.
#[test]
fn a_bare_name_where_the_scope_goes_is_refused() {
    let fresh = Fresh::new("scope-bare-name");
    let (out, code) = fresh.run(&["unlock", "ripgrep"]);
    assert_ne!(code, 0, "`unlock ripgrep` was accepted:\n{out}");
    assert!(
        out.contains("versions") && out.contains("backends") && out.contains("scripts"),
        "the refusal did not name the groups:\n{out}"
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
    let fresh = Fresh::new("scope-bare-name-lock");
    let (out, code) = fresh.run(&["lock", "ripgrep"]);
    assert_ne!(code, 0, "`lock ripgrep` was accepted:\n{out}");
}

#[test]
fn unlock_versions_drops_the_pins_and_leaves_the_other_two_alone() {
    let fresh = Fresh::new("scope-unlock-versions");
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
    let fresh = Fresh::new("scope-unlock-backends");
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
    let fresh = Fresh::new("scope-unlock-scripts");
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
    let fresh = Fresh::new("scope-unlock-bare");
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

/// `unlock everything` and a bare `unlock` are the same command spelled two ways, so they must
/// do the same thing — a default that drifts from the value it defaults to is its own bug.
#[test]
fn the_explicit_everything_matches_the_bare_form() {
    let fresh = Fresh::new("scope-unlock-everything");
    let (out, code) = fresh.run(&["unlock", "everything"]);
    assert_eq!(code, 0, "{out}");
    assert!(!fresh.versions().contains("apt:curl"));
    assert!(!fresh.backends().contains("ripgrep"));
    assert!(!fresh.scripts().contains("after_install:nginx"));
}

/// **One word, not two.** `all` was never a spelling of this and must not quietly become one:
/// two words for one scope is how three ledgers came to be called "the lock" in the first
/// place. The refusal has to teach the vocabulary, or a user who guessed wrong learns only
/// that they guessed wrong.
#[test]
fn a_near_miss_for_everything_is_refused_and_teaches_the_vocabulary() {
    let fresh = Fresh::new("scope-unlock-all-is-not-a-word");
    let before = (fresh.versions(), fresh.backends(), fresh.scripts());
    let (out, code) = fresh.run(&["unlock", "all"]);
    assert_ne!(code, 0, "`all` was accepted as a scope:\n{out}");
    for taught in ["everything", "packages", "scripts", "versions", "backends"] {
        assert!(
            out.contains(taught),
            "the refusal never said `{taught}`:\n{out}"
        );
    }
    assert_eq!(
        (fresh.versions(), fresh.backends(), fresh.scripts()),
        before,
        "a refused command still moved a ledger"
    );
}

/// A list of kinds takes exactly those kinds. The group words are a convenience over this, not
/// the other way round, so the list form is the one that has to hold.
#[test]
fn a_list_of_kinds_takes_those_kinds_and_no_others() {
    let fresh = Fresh::new("scope-unlock-list");
    let before_scripts = fresh.scripts();

    let (out, code) = fresh.run(&["unlock", "versions,backends"]);
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
    assert_eq!(
        fresh.scripts(),
        before_scripts,
        "a list naming two package kinds withdrew an approval"
    );
}

/// `everything --except versions` — the form the owner asked for by name. What is subtracted
/// stays put; everything else goes.
#[test]
fn an_exclusion_keeps_what_it_names_and_takes_the_rest() {
    let fresh = Fresh::new("scope-unlock-except");
    let before_versions = fresh.versions();

    let (out, code) = fresh.run(&["unlock", "everything", "--except", "versions"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        fresh.versions(),
        before_versions,
        "the excluded kind was released anyway"
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

/// The sub-category: `versions:apt` is apt's pins and nobody else's. This is the granularity
/// the ruling turned on — a manager scope that a `--backend` flag could express only in the
/// inclusion direction, and never inside an `--except`.
#[test]
fn a_sub_category_takes_one_managers_entries_and_leaves_the_others() {
    let fresh = Fresh::new("scope-unlock-qualifier");

    let (out, code) = fresh.run(&["unlock", "versions:apt"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        !fresh.versions().contains("apt:curl"),
        "{}",
        fresh.versions()
    );
    assert!(
        fresh.versions().contains("cargo:ripgrep"),
        "`versions:apt` took cargo's pin too:\n{}",
        fresh.versions()
    );
}

/// The form that has no spelling as a flag, which is the whole argument for putting the scope
/// in the word: keep one manager's pins and release every other lock there is.
#[test]
fn an_exclusion_can_name_a_sub_category() {
    let fresh = Fresh::new("scope-unlock-except-qualifier");

    let (out, code) = fresh.run(&["unlock", "everything", "--except", "versions:apt"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        fresh.versions().contains("apt:curl"),
        "the excluded manager's pin went with the rest:\n{}",
        fresh.versions()
    );
    assert!(
        !fresh.versions().contains("cargo:ripgrep"),
        "the exclusion kept a manager it did not name:\n{}",
        fresh.versions()
    );
    assert!(
        !fresh.backends().contains("ripgrep"),
        "{}",
        fresh.backends()
    );
}

/// A script kind on its own. `scripts` is seven kinds, and the point of naming them separately
/// is that one of them can move without the other six.
#[test]
fn one_script_kind_moves_without_its_six_siblings() {
    let fresh = Fresh::new("scope-unlock-one-script-kind");
    let before_versions = fresh.versions();

    // `adapters:backends.toml` is an adapters approval; `after_install:nginx` is a hook.
    let (out, code) = fresh.run(&["unlock", "adapters"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        !fresh.scripts().contains("adapters:backends.toml"),
        "{}",
        fresh.scripts()
    );
    assert!(
        fresh.scripts().contains("after_install:nginx"),
        "`unlock adapters` withdrew a hook approval:\n{}",
        fresh.scripts()
    );
    assert_eq!(fresh.versions(), before_versions, "the version pins moved");
}

/// A kind that does not subdivide says so, rather than accepting a qualifier and ignoring it.
/// An accepted-and-ignored scope is the silent wrong answer: the user believes they narrowed
/// the command and the command did everything.
#[test]
fn a_qualifier_on_a_flat_kind_is_refused() {
    let fresh = Fresh::new("scope-unlock-flat-qualifier");
    let before = (fresh.versions(), fresh.backends(), fresh.scripts());
    let (out, code) = fresh.run(&["unlock", "exec:anything"]);
    assert_ne!(code, 0, "a qualifier on a flat kind was accepted:\n{out}");
    assert_eq!(
        (fresh.versions(), fresh.backends(), fresh.scripts()),
        before,
        "a refused command still moved a ledger"
    );
}

/// Scoping by name, on every ledger: the named entry goes and its neighbour in the same
/// ledger stays. One name per ledger rather than one, because the scope is one shared rule
/// and a rule tested on one ledger is a rule tested nowhere.
#[test]
fn a_scope_takes_the_named_entry_and_leaves_its_neighbour() {
    let fresh = Fresh::new("scope-names");

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
    let fresh = Fresh::new("scope-full-key");
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
    let fresh = Fresh::new("scope-miss");
    let before = fresh.versions();
    let (out, code) = fresh.run(&["unlock", "versions", "shall-never-pinned-zzz"]);
    assert_eq!(code, 0, "a miss is not an error:\n{out}");
    assert!(
        out.contains("nothing unpinned"),
        "the miss was silent:\n{out}"
    );
    assert_eq!(fresh.versions(), before);
}

/// `--list` reports and changes nothing, on the scope it was given and on the bare form.
#[test]
fn list_reports_every_ledger_and_writes_nothing() {
    let fresh = Fresh::new("scope-list");
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
        "a scoped --list reported another ledger:\n{scoped}"
    );

    assert_eq!(fresh.versions(), before_v, "--list wrote a version pin");
    assert_eq!(fresh.backends(), before_b, "--list wrote the backend lock");
    assert_eq!(fresh.scripts(), before_s, "--list wrote an approval");
}

/// A preview releases nothing, on any ledger. `--dry-run lock` writing the ledgers is a bug
/// this repo has already had once.
#[test]
fn a_previewed_unlock_releases_nothing() {
    let fresh = Fresh::new("scope-dry-run");
    let (before_v, before_b, before_s) = (fresh.versions(), fresh.backends(), fresh.scripts());

    let (out, code) = fresh.run(&["--dry-run", "unlock"]);
    assert_eq!(code, 0, "{out}");

    assert_eq!(fresh.versions(), before_v, "a preview dropped a pin");
    assert_eq!(fresh.backends(), before_b, "a preview forgot a resolution");
    assert_eq!(fresh.scripts(), before_s, "a preview withdrew an approval");
}
