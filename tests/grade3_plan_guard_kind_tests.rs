//! GRADER round 4, 2026-07-30 — RED. `plan` predicts a refusal `apply` does not make, in the
//! vocabulary of package names, about a `link:`.
//!
//! Round 3's N-2 was "`check`, `plan` and `apply` cannot see the extras family". They can now, and
//! that half is closed. But the guard preview added with it runs the *package* rules over resource
//! keys — `src/verbs/plan.rs:259` merges `extra_removal_pairs(...)` into `guard::inspect(...)`, and
//! `inspect` is `RemovalKind::Package`. Three `link:` lines, undeclared, on a default config:
//!
//!     $ linix plan
//!     Wrote plan to linix-plan.json — 0 install(s), 0 removal(s), 0 resource(s) to place, 3 to undo.
//!       - link:…/dest/s1 (no longer declared)
//!
//!     WARNING: `linix apply` will refuse this plan.
//!     apply: refusing this removal.
//!       - link:…/dest/s1 would be removed (its manager reports a name no package line can hold,
//!         so LiNix cannot manage it — and removing what you cannot declare is not something you
//!         asked for)
//!
//!     $ linix apply linix-plan.json -y
//!     Applied plan: 0 installed, 0 removed, 3 resource(s) reconciled.     rc=0
//!
//! Every file was removed. `sync -y` does the same, also rc=0, no refusal.
//!
//! The trap is documented in the guard, and there is a unit test asserting it cannot happen —
//! `src/app/sync/guard.rs:973`, `no_extra_is_refused_merely_for_not_being_a_package_line`:
//!
//!     // `protection_of`'s declarability test asks whether a package line could hold the name,
//!     // and no extras key can — `link:/home/u/.vimrc` is not a package line and never parses as
//!     // one. Running that test over extras marks all six kinds `Undeclarable` and refuses every
//!     // teardown on every machine forever…
//!
//! That test passes. It exercises `inspect_removals(..., RemovalKind::Extra, …)`; the product
//! calls `inspect(...)`, which is `Package`. And `app/apply/extras.rs:146` states the property
//! this violates, in the other direction: *"a preview that skipped the guard would report a
//! teardown the real run then refuses, and the two must never disagree about the same machine."*
//!
//! Cost: the one warning a user is told to trust before a removal cries wolf on the ordinary case
//! — undeclaring a dotfile — and explains itself with a sentence about package names. A preview
//! that is wrong in the harmless direction is the same code that is wrong in the other.

use std::path::{Path, PathBuf};
use std::process::Command;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("dest")).unwrap();
        let f = Self { root };
        let (out, code) = f.run(&["init"]);
        assert_eq!(code, 0, "the fixture's own `init` failed:\n{out}");
        let profile = f.cfg().join("profiles").join("Main");
        let mut p = std::fs::read_to_string(&profile).unwrap();
        p.push_str("\nuse extras\n");
        std::fs::write(&profile, p).unwrap();
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

    /// `n` `link:` lines, placed for real, then undeclared — the state a user reaches by deleting
    /// lines from a module.
    fn placed_then_undeclared(&self, n: usize) -> Vec<PathBuf> {
        let mut lines = String::new();
        let mut targets = Vec::new();
        for i in 0..n {
            let src = self.root.join("src").join(format!("s{i}"));
            let dest = self.root.join("dest").join(format!("s{i}"));
            std::fs::write(&src, format!("content-{i}\n")).unwrap();
            lines.push_str(&format!(
                "link:{} @target={}\n",
                src.display().to_string().replace('\\', "/"),
                dest.display().to_string().replace('\\', "/")
            ));
            targets.push(dest);
        }
        let module = self.cfg().join("modules").join("extras.txt");
        std::fs::write(&module, &lines).unwrap();
        let (out, code) = self.run(&["sync", "-y"]);
        assert_eq!(code, 0, "the fixture's own sync failed:\n{out}");
        for t in &targets {
            assert!(t.exists(), "sync did not place {}:\n{out}", t.display());
        }
        std::fs::write(&module, "").unwrap();
        targets
    }
}

#[test]
fn plan_does_not_predict_a_refusal_apply_will_not_make() {
    let f = Fixture::new("grade3-plan-guard-kind");
    let targets = f.placed_then_undeclared(3);

    let (plan_out, plan_code) = f.run(&["plan"]);
    assert_eq!(plan_code, 0, "{plan_out}");
    // The control: the preview must actually see the teardown, or a green assertion below would
    // only mean `plan` had gone blind again (round 3's N-2).
    assert!(
        plan_out.contains("3 to undo"),
        "`plan` did not see the teardown at all, so this test is not measuring the prediction:\n\
         {plan_out}"
    );

    let (apply_out, apply_code) = f.run(&["apply", "linix-plan.json", "-y"]);
    let refused_for_real = apply_code == 3 || apply_out.contains("refusing this removal");
    let gone = targets.iter().all(|t| !t.exists());

    assert!(
        // The implication, unchanged: if `plan` predicted the refusal, `apply` has to make it.
        !plan_out.contains("will refuse this plan") || refused_for_real,
        "`plan` warned `WARNING: linix apply will refuse this plan` and `apply` performed it \
         (rc={apply_code}, every target removed: {gone}).\n\nplan said:\n{}\n\napply said:\n{}",
        plan_out
            .lines()
            .filter(|l| l.contains("refus") || l.contains("no package line"))
            .collect::<Vec<_>>()
            .join("\n"),
        apply_out.trim(),
    );
}

/// The same disagreement on the path most users take. `sync` never refuses this; `plan` says it
/// will.
#[test]
fn plan_and_sync_agree_about_a_resource_teardown() {
    let f = Fixture::new("grade3-plan-vs-sync");
    let targets = f.placed_then_undeclared(3);

    let (plan_out, _) = f.run(&["plan"]);
    let (sync_out, sync_code) = f.run(&["sync", "-y"]);
    let gone = targets.iter().all(|t| !t.exists());
    assert!(
        gone && sync_code == 0,
        "the fixture's own sync did not tear down:\nrc={sync_code}\n{sync_out}"
    );

    assert!(
        !plan_out.contains("will refuse this plan"),
        "`sync` tore down three resources at rc=0 without a word of objection, and `plan` \
         predicted a refusal for the same machine:\n{}",
        plan_out
            .lines()
            .filter(|l| l.contains("refus") || l.contains("no package line"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// And the reason string, which is the half a user reads. Nothing about a `link:` is a manager
/// reporting an unwritable package name.
#[test]
fn a_resource_is_never_explained_as_an_unwritable_package_name() {
    let f = Fixture::new("grade3-plan-guard-reason");
    f.placed_then_undeclared(3);
    let (plan_out, _) = f.run(&["plan"]);
    assert!(
        !plan_out.contains("no package line can hold"),
        "a `link:` teardown was explained with the guard's package-name refusal:\n{}",
        plan_out
            .lines()
            .filter(|l| l.contains("no package line"))
            .take(1)
            .collect::<Vec<_>>()
            .join("\n")
    );
}
