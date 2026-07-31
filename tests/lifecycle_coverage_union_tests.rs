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

fn read(rel: &str) -> String {
    let p: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// The `case` labels of one shell function, which is how both harnesses hold their tables.
///
/// `alpine|arch)` is two labels; `*)` is not a backend. A body that produces nothing for a
/// label — `btrfs) [ -n "$STORAGE_BTRFS" ] && echo …` — still counts as *named*, because the
/// question here is whether anyone has considered the backend, not whether today's host can.
fn case_labels(script: &str, func: &str) -> BTreeSet<String> {
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
        let Some((head, _)) = t.split_once(')') else {
            continue;
        };
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
    assert!(
        out.len() > 3,
        "{func}() yielded only {} labels — the scan is broken, not the table",
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
/// **The reason must be something a harness genuinely cannot do** — no such userland, no such
/// device, no account to sign in with (`Q17`). A cost is not a reason: "it downloads 2 GB" is
/// an argument for baking it into an image, not for an exemption. "It touches the real machine"
/// is not a reason either; every package manager does.
struct Nowhere {
    backend: &'static str,
    why: &'static str,
}

const NOWHERE: &[Nowhere] = &[
    // `brew` is deliberately NOT here. It has no container canary and the container harness
    // counts it in its own gap — but it HAS one in `integration-windows.sh`, which the macOS
    // CI leg runs, and this file asks whether a backend is reachable ANYWHERE. My first draft
    // listed it and this test rejected the entry, which is the check disagreeing with its
    // author and being right.
    Nowhere {
        backend: "emerge",
        why: "Gentoo is SMOKE_ONLY by design: a source-building install→remove costs hours, so \
              its image installs nothing and crediting it would be a caption, not coverage.",
    },
    Nowhere {
        backend: "eopkg",
        why: "no Solus image exists on any public registry — probed 2026-07-30, \
              getsolus/solus:latest is not published.",
    },
    Nowhere {
        backend: "guix",
        why: "no published base image; Guix installs via a script that needs a running \
              guix-daemon. Closable with an image built from that script.",
    },
    Nowhere {
        backend: "slackpkg",
        why: "Slackware images exist but are community-built and ship a Rust too old to build \
              LiNix in-image. Closable by copying in a statically-linked binary.",
    },
    Nowhere {
        backend: "yay",
        why: "AUR helpers refuse to run as root (needs_root = false) and the container sweep \
              runs as root. Closable with a non-root leg on the arch image.",
    },
    Nowhere {
        backend: "paru",
        why: "the same as yay, and it closes with the same non-root leg.",
    },
    Nowhere {
        backend: "pkg",
        why: "FreeBSD userland. A container shares the host's LINUX kernel, so this needs a VM \
              and not an image.",
    },
    Nowhere {
        backend: "pkg_add",
        why: "OpenBSD userland — a VM, for the same reason as pkg.",
    },
    Nowhere {
        backend: "pkgin",
        why: "NetBSD/SmartOS userland — a VM, for the same reason as pkg.",
    },
    Nowhere {
        backend: "mas",
        why: "needs a signed-in Mac App Store account on real Apple hardware. No container and \
              no VM can hold one legitimately.",
    },
    Nowhere {
        backend: "macports",
        why: "needs a real Mac. Apple's licence forbids virtualising macOS off Apple hardware, \
              so this is a runner we do not have rather than a thing we have not done.",
    },
    Nowhere {
        backend: "link",
        why: "not a package statement. `link:SRC @target=…` is its own grammar branch, so the \
              harness's `lifecycle()` — which builds a `backend:name` package declaration — \
              cannot express one. Closable with a lifecycle function for dependent statements; \
              covered today by link_teardown_test.rs and the plan-smoke.",
    },
    Nowhere {
        backend: "service",
        why: "a dependent statement like link, AND starting one needs an init system a plain \
              container does not run. Two independent blocks, and the second is real.",
    },
    Nowhere {
        backend: "setting",
        why: "a dependent statement like link, AND it writes to a live desktop settings store \
              (dconf/gsettings) that no image here runs a bus for.",
    },
    Nowhere {
        backend: "stack",
        why: "its first install downloads a whole GHC toolchain (~2 GB). That is a COST and not \
              an impossibility — Q17 says so — and the fix is the one every other manager in \
              the tools image already got: bake the toolchain in at build time. Named here so \
              the ceiling counts it rather than hiding it in a harness exemption.",
    },
];

/// May only go DOWN. Raising it is `Q4`'s item 4 happening — *no new backend is added until the
/// current set passes* — and the failure says so.
const NOWHERE_CEILING: usize = 15;

fn covered_somewhere() -> BTreeSet<String> {
    let win = read("scripts/integration-windows.sh");
    let con = read("docker/integration/run-in-container.sh");
    let mut c = case_labels(&win, "canary");
    c.extend(case_labels(&con, "canary"));
    // A distro's own manager is lifecycled by section 5 of the image built for it — a real
    // lifecycle, on a different run of the same script, which no single run can observe.
    c.extend(case_labels(&con, "primary_manager_image"));
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
    let named: BTreeSet<String> = NOWHERE.iter().map(|n| n.backend.to_string()).collect();

    let unexplained: Vec<&String> = uncovered.difference(&named).collect();
    assert!(
        unexplained.is_empty(),
        "these backends have no canary in EITHER harness and no written reason they cannot have \
         one: {unexplained:?}\n\nQ4: a backend with no real lifecycle in an automated gate is a \
         release blocker, not a caption. Give it a canary, or add it to NOWHERE with a reason \
         that is an impossibility rather than a cost."
    );

    let stale: Vec<&String> = named.difference(&uncovered).collect();
    assert!(
        stale.is_empty(),
        "NOWHERE names backends that ARE reachable now — delete the entry and lower \
         NOWHERE_CEILING to {}: {stale:?}",
        uncovered.len()
    );

    assert!(
        uncovered.len() <= NOWHERE_CEILING,
        "{} backends are unreachable and the ceiling is {NOWHERE_CEILING}. It may only go down.",
        uncovered.len()
    );
}

#[test]
fn every_exemption_gives_a_reason_and_not_a_cost() {
    for n in NOWHERE {
        assert!(
            n.why.len() > 40,
            "{}'s exemption has no reason worth the name: {:?}",
            n.backend,
            n.why
        );
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
    let config = linix::config::Config::default();
    let hooks = linix::app::LuaHooks::new(&config).expect("hooks for a default config");
    let registry = linix::backends::registry::create_default_registry(
        linix::core::CommandExecutor::new(true, false),
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
         Add them to UNIVERSE — and to a canary table, or to NOWHERE with a reason."
    );
}
