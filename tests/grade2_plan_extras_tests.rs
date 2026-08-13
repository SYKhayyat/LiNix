//! GRADER round 3, 2026-07-29 — RED. `plan`/`apply` cannot see the extras family at all.
//!
//! G-1 was closed properly: `sync` now counts a `link:`/`service:`/`setting:`/`shim:`/
//! `schedule:`/`repo:` teardown, refuses past `max_removals`, refuses on a protected name, and
//! names each one in `--dry-run`. Measured on Windows, all four.
//!
//! `plan` and `apply` were not part of that sweep, and they are the *reviewable* half of the
//! product — `shall plan --help` calls itself Terraform-style, "so the exact plan you inspect is
//! the one you later `apply`". Measured, on a fresh config with three `link:` lines declared and
//! nothing on disk:
//!
//!     $ shall plan
//!     Wrote plan to shall-plan.json — system already matches desired state (no changes).
//!     $ cat shall-plan.json
//!     { … "installs": [], "removals": [], … }
//!     $ shall apply shall-plan.json -y
//!     Plan is empty — nothing to apply.
//!     $ ls dest/
//!     (empty)
//!
//! And in the other direction — three `link:` lines applied, then undeclared:
//!
//!     $ shall plan
//!     Wrote plan to shall-plan.json — system already matches desired state (no changes).
//!     $ shall --dry-run sync
//!     WARN [DRY-RUN] `link:…/dest/g1` is no longer declared — sync would undo it.   (×3)
//!
//! So the two previews disagree about the same machine, and the one a user is told to trust for
//! review is the blind one. Worse, it is the guard's own refusal text that sends them there:
//!
//!     sync: refusing this removal.
//!       - it removes 5 managed resources, over the limit of 1 ([guard] max_removals)
//!       What to do:
//!         shall plan                     see exactly what would be undone
//!
//! `shall plan` shows nothing at all. A user who follows that line sees "no changes" and
//! concludes the refusal was spurious.
//!
//! This is G-1's third failure — "the removal is invisible; nothing in `plan`, `--dry-run` or
//! the summary names it" — with the `--dry-run` half fixed and the `plan` half still live.

use std::path::PathBuf;

use crate::harness::{decl, Fixture};

/// The shared root, plus what these tests need in it.
fn setup(name: &str) -> Fixture {
    let f = Fixture::new(name);
    std::fs::create_dir_all(f.root.join("dest")).unwrap();
    f
}

impl Fixture {
    /// Declare `n` `link:` lines and return their targets.
    fn declare_links(&self, n: usize) -> Vec<PathBuf> {
        let sources = self.cfg().join("dotfiles");
        std::fs::create_dir_all(&sources).unwrap();
        let mut module = String::new();
        let mut targets = Vec::new();
        for i in 1..=n {
            let src = sources.join(format!("p{i}"));
            let dst = self.root.join("dest").join(format!("p{i}"));
            std::fs::write(&src, format!("content-{i}\n")).unwrap();
            module.push_str(&format!("link:{} @target={}\n", decl(&src), decl(&dst)));
            targets.push(dst);
        }
        std::fs::write(self.cfg().join("modules/starter.txt"), &module).unwrap();
        targets
    }
}

/// A plan that omits work `sync` would do is worse than no plan: it is a review that reports
/// "nothing to see" over changes to the filesystem.
#[test]
fn plan_names_the_extras_it_would_place() {
    let f = setup("grade2-plan-place");
    let targets = f.declare_links(3);

    // Control: the same model, through the preview that does work, so a green run cannot be
    // explained by "the declarations never resolved".
    let (dry, code) = f.run(&["--dry-run", "sync"]);
    assert_eq!(code, 0, "the fixture's own `--dry-run sync` failed:\n{dry}");
    for t in &targets {
        assert!(
            !t.exists(),
            "`--dry-run sync` placed {} — this fixture cannot tell a blind plan from a busy \
             one:\n{dry}",
            t.display()
        );
    }

    let (out, code) = f.run(&["plan"]);
    // `H2` (owner, 2026-08-13): a read-only command that finds work exits **2**, and `plan`
    // is one. Both 0 and 2 are successful runs of it; 1 is a failure. The content
    // assertions below are what carry this test's meaning either way.
    assert!(matches!(code, 0 | 2), "`plan` failed:\n{out}");
    assert!(
        !out.contains("already matches desired state"),
        "`shall plan` reported the system already matches, while three `link:` declarations \
         are unapplied and `sync` would place them.\n\
         `plan --help` promises the plan you inspect is the one you later `apply`; a plan that \
         cannot see the extras family promises that about half the model.\n{out}"
    );

    let plan = std::fs::read_to_string(f.root.join("shall-plan.json")).expect("plan file");
    assert!(
        targets
            .iter()
            .all(|t| plan.contains(&t.file_name().unwrap().to_string_lossy().to_string())),
        "the frozen plan does not name the resources it would place:\n{plan}"
    );

    // `apply` executes the frozen plan, so an empty plan is an apply that does nothing.
    let (out, code) = f.run(&["apply", "shall-plan.json", "-y"]);
    assert_eq!(code, 0, "`apply` failed:\n{out}");
    for t in &targets {
        assert!(
            t.exists(),
            "`apply` of a freshly frozen plan did not place {}; the file says `Plan is empty`.\
             \n{out}",
            t.display()
        );
    }
}

/// The direction the guard's own refusal text points at. `sync: refusing this removal` tells the
/// user to run `shall plan` to "see exactly what would be undone", and `plan` shows nothing.
#[test]
fn plan_names_the_extras_it_would_tear_down() {
    let f = setup("grade2-plan-teardown");
    let targets = f.declare_links(3);

    let (out, code) = f.run(&["sync", "-y"]);
    assert_eq!(code, 0, "applying the link declarations failed:\n{out}");
    for t in &targets {
        assert!(t.exists(), "setup did not apply {}:\n{out}", t.display());
    }

    std::fs::write(f.cfg().join("modules/starter.txt"), "").unwrap();

    // Control: `--dry-run sync` sees the teardown, so the model genuinely has three fewer
    // resources than the machine. Without this, a blind `plan` and an empty model look alike.
    let (dry, code) = f.run(&["--dry-run", "sync"]);
    assert_eq!(code, 0, "`--dry-run sync` failed:\n{dry}");
    assert_eq!(
        dry.matches("is no longer declared").count(),
        3,
        "the control failed: `--dry-run sync` did not report three teardowns, so this test \
         would prove nothing about `plan`:\n{dry}"
    );

    let (out, code) = f.run(&["plan"]);
    // `H2` (owner, 2026-08-13): a read-only command that finds work exits **2**, and `plan`
    // is one. Both 0 and 2 are successful runs of it; 1 is a failure. The content
    // assertions below are what carry this test's meaning either way.
    assert!(matches!(code, 0 | 2), "`plan` failed:\n{out}");
    assert!(
        !out.contains("already matches desired state"),
        "`shall plan` reported the system already matches, while `--dry-run sync` on the same \
         tree named three resources it would tear down.\n\
         The guard's refusal text sends the user to `shall plan` to see exactly what would be \
         undone — and it shows them nothing.\n{out}"
    );
}

/// `H6` — the preview shows every declared script, including the ones this command will not run.
///
/// **The same hole as the rest of this file, one option later.** `@on=` gave `exec:` lines two
/// audiences, and the first version of the preview iterated the *running* list — so a step
/// declared `@on=upgrade` was code in the configuration that no preview anywhere showed. `plan`
/// previews `sync`; nothing previews `upgrade`; a filtered preview meant nobody could review the
/// script before it ran. A summary built from the actor's list rather than the reader's is `F12`
/// exactly, and it is why this asserts on the line being *present and labelled* rather than on
/// it being absent.
#[test]
fn the_plan_shows_a_step_it_will_not_run_and_says_which_verb_runs_it() {
    let f = setup("h6-plan-shows-upgrade-steps");
    let cfg = f.cfg();
    std::fs::create_dir_all(cfg.join("bin")).unwrap();
    std::fs::write(cfg.join("bin/sync-step.sh"), "echo sync\n").unwrap();
    std::fs::write(cfg.join("bin/firmware.sh"), "echo firmware\n").unwrap();
    std::fs::write(
        cfg.join("modules/starter.txt"),
        "exec:./bin/sync-step.sh\nexec:./bin/firmware.sh @on=upgrade\n",
    )
    .unwrap();

    let (out, code) = f.run(&["plan"]);
    assert!(matches!(code, 0 | 2), "`plan` failed:\n{out}");
    assert!(
        out.contains("sync-step.sh"),
        "the control failed — `plan` printed no script section at all, so this proves \
         nothing:\n{out}"
    );
    assert!(
        out.contains("firmware.sh"),
        "`plan` hid a declared `exec:` line because this command would not run it. It is still \
         code in the configuration, and `plan` is where a user reviews it before it runs:\n{out}"
    );
    assert!(
        out.contains("not this command"),
        "`plan` listed a step it will not run without saying so, which is worse than hiding \
         it — the reader takes it for work this command is about to do:\n{out}"
    );
}
