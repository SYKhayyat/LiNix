//! Which registered backends have never had a real lifecycle *anywhere*.
//!
//! Each sweep audits only its own registry, and that is not a small gap — it is the gap that
//! hid the worst case. `winget`, `choco` and `psresource` exist only on Windows and were
//! excused there; they are absent from the Linux registry entirely. So the question *"is
//! `winget` lifecycled anywhere?"* was asked by nothing at all, and the answer was no. An
//! excuse on the only harness that can run a backend is indistinguishable from coverage.
//!
//! Measured on 2026-07-30 across the union of both registries — 60 distinct backends — **20 had
//! never completed a real lifecycle in any harness and 12 were in neither table of either one.**
//!
//! This test is the only place that sees both harnesses at once without running either, which
//! is why it reads their tables from source rather than from a run. It answers one question:
//! **is this backend reachable by a real install → list → remove somewhere in the matrix?**
//!
//! It deliberately does *not* answer "did a lifecycle ever pass" — that is a fact about CI
//! history, and `lifecycle-floor.txt` is the ratchet for it. A canary means the harness *could*.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::ledger::{Entry, Ledger};

fn read(rel: &str) -> String {
    let p: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// The `case` labels of one shell function, which is how both harnesses hold their tables.
///
/// `alpine|arch)` is two labels; `*)` is not a backend. A body that produces nothing for a
/// label — `btrfs) [ -n "$STORAGE_BTRFS" ] && echo …` — still counts as *named*, because the
/// question here is whether anyone has considered the backend, not whether today's host can.
fn case_labels(script: &str, func: &str, min: usize) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(body) = script.split_once(&format!("\n{func}() {{\n")) else {
        panic!("{func}() is not in this script, or its shape changed");
    };
    let body = body.1.split_once("\n}\n").expect("unterminated function").0;
    for line in body.lines() {
        let t = line.trim();
        // A label line is `name)` or `a|b)` at the head of a case arm, never a comment.
        if t.starts_with('#') {
            continue;
        }
        let Some((head, arm)) = t.split_once(')') else {
            continue;
        };
        // A label is coverage only if its arm actually YIELDS a canary. `web) echo "" ;;` is a
        // row that says "there is nothing here", and counting it made this gate overstate
        // coverage by every empty row in both tables — the check examining its own shape
        // rather than the thing it names. Rows that echo a variable (`$STORAGE_BTRFS/canary`)
        // are real; only a literal empty string is the absence.
        if func == "canary" && (arm.contains(r#"echo "" "#) || arm.trim() == r#"echo "" ;;"#) {
            continue;
        }
        if head.is_empty() || head.contains(' ') || head.contains('$') || head.contains('(') {
            continue;
        }
        for name in head.split('|') {
            if name == "*" || name.is_empty() {
                continue;
            }
            if name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            {
                out.insert(name.to_string());
            }
        }
    }
    // **The floor is per table, because the tables are different sizes on purpose.**
    // A hard `> 3` is right for `canary`, which names dozens; it is wrong for
    // `dependent_lifecycle`, which today names exactly one statement that is really
    // driven. Asserting the wrong floor would force the table to be padded with
    // statements nobody drives, which is the failure this whole file is about.
    assert!(
        out.len() >= min,
        "{func}() yielded only {} labels, fewer than the {min} it must have — the
         scan is broken, not the table",
        out.len()
    );
    out
}

/// Every backend either harness could register, on any platform.
///
/// Checked in rather than derived, because no single platform's registry holds all of them —
/// that is the whole point of this file. `every_backend_this_host_registers_is_in_the_universe`
/// keeps it from rotting: a new backend fails on whichever platform registers it.
const UNIVERSE: &[&str] = &[
    "apk",
    "appimage",
    "apt",
    "asdf",
    "brew",
    "btrfs",
    "bun",
    "cabal",
    "cargo",
    "choco",
    "composer",
    "conda",
    "dnf",
    "dotnet",
    "emacs",
    "emerge",
    "eopkg",
    "flatpak",
    "gem",
    "github",
    "go",
    "guix",
    "helm",
    "krew",
    "link",
    "luarocks",
    "lvm",
    "mas",
    "macports",
    "mise",
    "mix",
    "nimble",
    "nix",
    "nixos",
    "npm",
    "opam",
    "pacman",
    "paru",
    "pip",
    "pipx",
    "pixi",
    "pkg",
    "pkg_add",
    "pkgin",
    "pnpm",
    "psresource",
    "pub",
    "scoop",
    "service",
    "setting",
    "slackpkg",
    "snap",
    "spack",
    "stack",
    "uv",
    "vscode",
    "web",
    "winget",
    "xbps",
    "yarn",
    "yay",
    "zfs",
    "zypper",
];

/// A backend no harness can reach with a real lifecycle, and why.
///
/// **This table moved into `src/backends/proving.rs`, and this test now reads it.** It used to
/// live here, which meant the repository knew which backends had never met their manager and
/// the program could not say it — a fact known only to a test is a fact no user gets. `check
/// health` now marks them, and the same table answers both questions.
///
/// Reading it rather than keeping a copy is `F7`'s lesson applied instead of repeated: the gate
/// that checked the latency class table read a hand-typed transcription of it, so the one name
/// that had gone stale was the one the copy omitted.
use shall::backends::proving::UNPROVEN;

struct Nowhere {
    backend: &'static str,
    why: &'static str,
}

fn nowhere() -> Vec<Nowhere> {
    UNPROVEN
        .iter()
        .map(|(backend, why)| Nowhere { backend, why })
        .collect()
}

/// **What this gate cannot see, stated because `nix` proved it.** It reads the two harnesses'
/// TABLES, not their runs — deliberately, since no single run observes every image. So a canary
/// row for a manager that is not installed anywhere counts as coverage: `nix` had a row here the
/// whole time, `no_lifecycle_reason` had no excuse for it, and the manager itself was missing
/// from the image because its installer refused to run as root. Both gates said fine and nothing
/// ran for months.
///
/// The run-time half of that question now exists and lives in the sweep: each image writes what
/// it actually ships to `/etc/shall-image-managers`, and the coverage audit reports a manager
/// that failed to install as MISSING rather than impossible. This test and that check answer
/// different halves — *is it claimed anywhere* and *is it really there* — and neither is the
/// whole answer alone.
///
/// May only go DOWN. Raising it is `Q4`'s item 4 happening — *no new backend is added until the
/// current set passes* — and the failure says so.
///
/// **15 → 13 on 2026-08-14**, when `yay` and `paru` stopped being unreachable. Their exemption
/// had named its own closing move — *"closable with a non-root leg on the arch image"* — and the
/// arch image now runs the sweep as an unprivileged user, which is the only condition an AUR
/// helper ever asked for. Neither tool changed; the harness did.
///
/// **12 → 10 the same day**, when `service` and `setting` did. Each had been excused on the
/// evidence of one row out of five: `service` on systemd's, while SysVinit needs no running init
/// to enable or start anything, and `setting` on dconf's, while the Windows store is `reg` and
/// wants no bus. Neither statement changed either.
/// **9 → 8** the same day, when `guix` did. Its reason had already been rewritten twice as each
/// version of it was measured false; the third was *"the manager works, Shall has not driven
/// it"*, and now Shall drives it. What the image had to be taught was not about guix — a profile
/// that starts empty owes the harness a `git` and a protected package, and a manager that tells
/// you to put its bin on PATH yourself will not do it for you.
///
/// **10 → 9**, when `slackpkg` did. Its reason had been *"Slackware images exist but are
/// community-built and ship a Rust too old to build Shall in-image"*, and both halves were wrong:
/// `Dockerfile.slackware` bootstraps the toolchain through slackpkg itself and the Rust it
/// installs builds Shall. Like the four above it, the tool never changed — the harness did.
// Raised 8 -> 9 on 2026-08-16 for `nixos`, by the owner, and this is the only entry that has
// ever raised it. The rule above says it may only go down, and the remedy says a reason must be
// an impossibility rather than a cost; `nixos` is a cost. What was traded for it: the full
// lifecycle WAS driven, by hand, on NixOS 26.05 under WSL — install, list, binary on PATH,
// remove, machine still buildable — and that run found four defects the hermetic layers had all
// passed. So the number buys a backend that is measured but not gated, and the debt is a NixOS
// CI leg. Retire it by building one; do not retire it by deleting the row.
const NOWHERE_CEILING: usize = 9;

fn covered_somewhere() -> BTreeSet<String> {
    let win = read("scripts/integration-windows.sh");
    let con = read("docker/integration/run-in-container.sh");
    let mut c = case_labels(&win, "canary", 4);
    c.extend(case_labels(&con, "canary", 4));
    // **The dependent statements, which have their own table because they have their own
    // assertions.** `canary()` describes a `backend:name` package lifecycle; a `link:` is checked
    // with `test -L` and a content read, so it could never be a row there. Reading only `canary`
    // is what made `link` look permanently unreachable — this gate was measuring the shape of one
    // table rather than whether anything drives the statement.
    c.extend(case_labels(&con, "dependent_lifecycle", 1));
    // **And the Windows sweep's own table, because a dependent statement can be unreachable on
    // one platform and trivial on the other.** `setting:` is the case that proves it: on Linux
    // the store is dconf and needs a session bus no image runs, and on Windows it is `reg`,
    // which needs nothing at all. Reading only the container's table would have kept `setting`
    // permanently exempt on the evidence of the platform that cannot do it — which is the same
    // error as `winget` being excused on the only harness that could run it.
    c.extend(case_labels(&win, "dependent_lifecycle", 1));
    // A distro's own manager is lifecycled by section 5 of the image built for it — a real
    // lifecycle, on a different run of the same script, which no single run can observe.
    c.extend(case_labels(&con, "primary_manager_image", 4));
    c
}

#[test]
fn every_backend_is_reachable_somewhere_or_named_as_unreachable() {
    let covered = covered_somewhere();
    let uncovered: BTreeSet<String> = UNIVERSE
        .iter()
        .filter(|b| !covered.contains(**b))
        .map(|b| b.to_string())
        .collect();

    Ledger::of(
        "without a canary in EITHER harness",
        "`UNPROVEN` in src/backends/proving.rs (and lower NOWHERE_CEILING with it)",
    )
    .exempting(nowhere().iter().map(|n| Entry {
        site: n.backend,
        why: n.why,
    }))
    .scanning_at_least(40)
    .remedy(
        "Q4: a backend with no real lifecycle in an automated gate is a release blocker, not a \
         caption. Give it a canary, or write a reason that is an impossibility rather than a cost.",
    )
    .audit(UNIVERSE.len(), &uncovered);

    assert!(
        uncovered.len() <= NOWHERE_CEILING,
        "{} backends are unreachable and the ceiling is {NOWHERE_CEILING}. It may only go down.",
        uncovered.len()
    );
}

/// The half [`Ledger`] cannot check: an exemption for something that is not a backend at all.
/// Its stale check would report it as "reachable now", which is the opposite of true.
#[test]
fn every_exemption_names_a_backend() {
    for n in &nowhere() {
        assert!(
            UNIVERSE.contains(&n.backend),
            "{} is exempted and is not a backend",
            n.backend
        );
    }
}

/// The universe list is hand-written, so it rots the moment a backend is added. This is what
/// stops that: whatever platform runs the suite checks its own registry against the list, and
/// between Windows and Linux CI every entry is covered.
#[tokio::test]
async fn every_backend_this_host_registers_is_in_the_universe() {
    let config = shall::config::Config::default();
    let hooks = shall::app::LuaHooks::new(&config).expect("hooks for a default config");
    let registry = shall::backends::registry::create_default_registry(
        shall::core::CommandExecutor::new(true, false),
        &config,
        std::sync::Arc::new(hooks),
    )
    .await;

    // `all()`, not `available()`: the question is what is REGISTERED on this platform, and a
    // manager the host does not happen to have installed is still a backend that needs a
    // lifecycle somewhere. Filtering by availability would make the check weaker on exactly
    // the machines that have the least installed.
    let missing: Vec<String> = registry
        .all()
        .into_iter()
        .map(|b| b.name().to_string())
        .filter(|n| !UNIVERSE.contains(&n.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "this host registers backends the coverage universe does not list: {missing:?}\n\
         Add them to UNIVERSE — and to a canary table, or to `UNPROVEN` with a reason."
    );
}
