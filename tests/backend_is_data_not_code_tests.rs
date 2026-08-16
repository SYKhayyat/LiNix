//! Is a backend data, or is it another 300 lines of Rust?
//!
//! There are two ways a backend gets written here. The good way is a `ManagerConfig` — a record
//! saying "install with these words, remove with those, list with these" — about 23 lines, no
//! logic, parsed by machinery every other data backend shares. The other way is a module with
//! its own `impl BackendCore`, its own install loop, its own remove loop, its own argv builder:
//! 200 to 400 lines, of which most is the same shape as its neighbour's.
//!
//! **The formulaic list is empty as of 2026-08-04.** It began at 29 modules; eight came out the
//! same day — `krew`, `pubdart`, `npm`, `pnpm`, `yarn`, `cargo`, `pipx`, `uv` — about 1,900
//! lines of Rust replaced by eight data rows. Three more came out on 2026-08-06: `pacman`,
//! `dnf` and `xbps`, whose reasons were refuted by the code beside them and whose hand-built
//! argv had lost the `--` terminator every backend on the data path gets for free. What remains
//! are 18 modules that are hand-written because the manager is, each named below with what the
//! generic machinery cannot express **and the line that makes that true**.
//!
//! Every one of the conversions cost the machinery a field rather than a compromise:
//! `extra_probes` (a manager reached as a plugin of another program), `upgrade_reinstall_args`
//! (no upgrade-all verb), `property_probes` (where the manager put it, asked with a second
//! command), `SearchSource` (a search that is an HTTP call), and a version pin that is a
//! trailing operand rather than a flag — which arrived as its own `VersionPin` variant and is
//! now read off the token, because three variants that built identical argv let two backends
//! disagree about the `--` terminator (Q30). The 2026-08-06 three cost four more: `CacheClean`
//! (how a manager empties its download cache — which no row could say at all, so forty
//! backends silently had none), `DependsProbe` (a dependency report whose *shape* is the
//! manager's), `OutdatedProbe::silence_is_none` (`pacman -Qu` exits 1 with nothing to mean
//! "nothing"), and `{name_component}` (a repository name that becomes a path segment, which
//! both hand-written modules validated and the shared repo path did not).
//!
//! All of them are now available to every backend, which is the difference between converting a
//! backend and deleting one.
//!
//! **This file is the ratchet, and it deliberately comes before the conversions.** The direction
//! doc's own instruction: *"Write the ratchet before converting anything — it is worth more than
//! the conversions, and it is what stops backend thirty-nine from being written by copying
//! thirty-eight."* A conversion sweep nobody finishes leaves two ways of doing things, which is
//! how this repo got two of everything. A list that can only shrink turns "convert backend #12"
//! from a refactor somebody has to justify into a chore with a visible finish line.
//!
//! So: every backend module is data-driven, or it is named below with the reason it cannot be.
//! Adding a name is allowed and requires writing the reason. Removing one is the work.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::ledger::{Entry, Ledger};

/// A backend module that is hand-written Rust rather than a `ManagerConfig`, and why.
///
/// **A reason here is a claim about the manager, not about the calendar.** "Not converted yet"
/// is not a reason; it is the absence of one, and an exemption list of those is a list of things
/// nobody looked at (E29). Each entry says what the *generic* machinery cannot express.
///
/// **And a claim is checked, which until 2026-08-06 it was not.** The only assertion on `why`
/// was that it ran past sixty characters, so three of these reasons described code that was not
/// in the module they excused. `pacman.rs` said *"the removal guard needs pacman's own
/// essential/required-by data"* and there was no `essential()` impl anywhere in it —
/// `grep -n essential src/backends/pacman.rs` returned nothing. `dnf.rs` described
/// `ManualListing::Command { format: SameAsInstalled }` in prose. The since-deleted `xbps.rs`
/// named three binaries
/// that are three existing fields. Sixty characters of fluent prose is what a check that cannot
/// fail looks like when the subject is English.
///
/// So each entry now points at the line that makes it true. [`HandWritten::proof`] must appear
/// in the module's own source; the reason and the code are checked against each other, and an
/// exemption whose evidence is not there fails the build.
///
/// **What this raises the floor to, and what it does not reach.** A grep cannot decide whether
/// a reason is *sound* — only whether it is about code that exists. The first proof written for
/// `brew.rs` was its own install argv, which is present and proves nothing, and finding that
/// out took reading the module. What the check buys is that writing the entry now requires
/// naming a line, and naming a line is where the three false claims would have come apart.
struct HandWritten {
    module: &'static str,
    why: &'static str,
    /// Text from the module that the reason is about. Not a citation and not a date — the
    /// symbol, flag or command the claim rests on, so a reader can go and disagree with it.
    proof: &'static str,
}

const HAND_WRITTEN: &[HandWritten] = &[
    // ---- Not package managers at all. These do not run a manager's CLI; the generic machinery
    // has nothing to configure because there is no argv template to fill in.
    HandWritten {
        module: "link.rs",
        why: "writes symlinks (or copies) through the filesystem layer; the only program it ever \
              runs is a decryptor for an `@encrypted` source. `ManagerConfig` is a table of \
              argv, and placing a file is not one.",
        proof: "symlink_metadata",
    },
    HandWritten {
        module: "web.rs",
        why: "fetches a URL over HTTP and writes the file itself — scheme and checksum policy, \
              not argv. Nothing to template.",
        proof: "download::check_scheme",
    },
    HandWritten {
        module: "nixos.rs",
        why: "renders the machine's system configuration and runs ONE `nixos-rebuild switch` for \
              the whole batch. `ManagerConfig` is a table of per-package argv, and this backend \
              has no per-package command at all: the name never reaches a command line, it \
              reaches a generated Nix module. A data row could not express that, and one that \
              pretended to would be argv nothing ever runs.",
        proof: "nixos-rebuild",
    },
    HandWritten {
        module: "github.rs",
        why: "resolves a release through the GitHub API, selects among assets by rule, and \
              records the choice in a lock. The selection logic IS the backend; there is no \
              command line anywhere in it.",
        proof: "browser_download_url",
    },
    HandWritten {
        module: "appimage.rs",
        why: "downloads an image over HTTP and marks it executable through the filesystem \
              layer — `web.rs` one file format along, and argv-free for the same reason.",
        proof: "download::check_checksum_declared",
    },
    HandWritten {
        module: "setting.rs",
        why: "dispatches on which settings STORE the host has (registry / gsettings / …), each \
              with read-before-write semantics. It is already data — `setting_stores.toml` — \
              but of a different table than `ManagerConfig`, which has no read-then-decide step.",
        proof: "setting_stores.toml",
    },
    HandWritten {
        module: "service.rs",
        why: "dispatches on which init system the host has, from `init_providers.toml`, and a \
              declaration maps to a SEQUENCE of actions (enable, start) rather than one install \
              verb. Already data, again of a different table.",
        proof: "init_providers.toml",
    },
    HandWritten {
        module: "storage.rs",
        why: "lvm and zfs address `group/volume`, take a size, and must check existence before \
              creating. Two backends from one module, neither of which installs a package.",
        proof: "fn zfs_create",
    },
    HandWritten {
        module: "btrfs.rs",
        why: "subvolumes plus fstab editing plus an unmount ordering constraint — the mount must \
              be dropped before the subvolume, or the machine stops in the initramfs at next \
              boot. That ordering cannot live in an argv table.",
        proof: "\"umount\"",
    },
    // ---- Package managers whose shape the generic machinery does not yet cover.
    HandWritten {
        module: "nix.rs",
        why: "removes by profile INDEX, so removal must list-then-match rather than name the \
              package, and it parses two different JSON layouts across nix versions. Generic \
              removal templates a name it does not have.",
        proof: "profile\", \"list\", \"--json",
    },
    HandWritten {
        module: "go.rs",
        why: "installs a module path and removes by deleting the binary out of `go env GOPATH`. \
              Removal is a filesystem operation informed by a query, not a manager verb.",
        proof: "GOPATH",
    },
    HandWritten {
        module: "emacs.rs",
        why: "is handed an Emacs Lisp form after `--eval`, not a subcommand. The argv is one \
              long program, which is a different thing from a template with slots.",
        proof: "\"--batch\", \"--eval\"",
    },
    HandWritten {
        module: "psresource.rs",
        why: "runs PowerShell with a `-Command` script and named parameters (`-Name 'x' \
              -Scope CurrentUser`), not positional argv. `ManagerConfig` templates positions.",
        proof: "\"-Command\"",
    },
    HandWritten {
        module: "snap.rs",
        why: "probes with `snap info` before installing, because a classic snap needs \
              `--classic` and a non-classic one refuses it. Install is conditional on a read.",
        proof: "--classic",
    },
    HandWritten {
        module: "vscode.rs",
        why: "the `code` CLI is located differently per platform and per install flavour \
              (system, user, Insiders, snap), which is discovery rather than argv.",
        proof: "--install-extension",
    },
    HandWritten {
        module: "mise.rs",
        why: "manages tool VERSIONS rather than packages — `use -g name@version` where the \
              version is part of the identity — and reads its own config to list them.",
        proof: "\"use\".to_string(), \"-g\".to_string()",
    },
    HandWritten {
        module: "flatpak.rs",
        why: "`@channel` becomes part of the ref itself — `install_ref` writes `name//branch` — \
              and `ManagerConfig` has `VersionPin` for `@version` with no equivalent for a \
              channel. `Y23` made the blocker larger rather than smaller: flatpak has no channel \
              switch, so changing one is an install followed by `make-current`, conditional on \
              reading which branch the app is on — two commands decided by a query, which is the \
              same shape `snap.rs` is exempted for. Scope stopped being the blocker when `Y22` \
              renamed the key to `scope = \"user\"|\"system\"`, which a row can substitute as \
              `--{setting.scope|system}`.",
        proof: "--system",
    },
    HandWritten {
        module: "brew.rs",
        why: "`info` reads `brew info --json=v1` — a JSON document — for the keg prefix and \
              whether the formula came in as a dependency. `PropertyProbe` runs a command and \
              substitutes its whole stdout into a `{base}` template; it cannot reach \
              `installed[0].prefix`, and a listing cannot answer about a formula that is not \
              installed.",
        proof: "installed_as_dependency",
    },
];

fn backend_modules() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/backends");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("cannot read src/backends") {
        let path = entry.expect("bad entry").path();
        let Some(name) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        if !name.ends_with(".rs") {
            continue;
        }
        // Not backends: the generic machinery itself, the registry, the runtime-defined
        // backends, the shared capability table, a shared search helper, and test files.
        if matches!(
            name,
            "generic.rs"
                | "registry.rs"
                | "onboarder.rs"
                | "capability.rs"
                | "mod.rs"
                | "node_registry.rs"
                | "pip_search.rs"
        ) || name.ends_with("_test.rs")
        {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("cannot read module");
        out.push((name.to_string(), src));
    }
    out
}

/// **The ratchet.** A backend module is data, or it is named with a reason.
#[test]
fn every_backend_is_data_or_says_why_not() {
    let modules = backend_modules();
    let hand_written: BTreeSet<String> = modules
        .iter()
        .filter(|(_, src)| src.contains("impl BackendCore for"))
        .map(|(name, _)| name.clone())
        .collect();

    Ledger::of("hand-written Rust rather than a data row", "HAND_WRITTEN")
        .exempting(HAND_WRITTEN.iter().map(|h| Entry {
            site: h.module,
            why: h.why,
        }))
        .scanning_at_least(10)
        .reason_of_at_least(60)
        .remedy(
            "Build the backend from a `ManagerConfig` — adding a backend should be adding data. \
             \"Not converted yet\" is not a reason; it is the absence of one.",
        )
        .audit(modules.len(), &hand_written);
}

/// The assertion [`Ledger`] cannot make: a reason may be long and still be a schedule.
#[test]
fn every_reason_says_what_the_generic_machinery_cannot_express() {
    for h in HAND_WRITTEN {
        assert!(
            !h.why.to_lowercase().contains("not converted yet")
                || h.why.contains("What blocks it")
                || h.why.contains("blocks it"),
            "{}'s reason is a schedule, not a constraint: {:?}",
            h.module,
            h.why
        );
    }
}

/// **The reason and the code are checked against each other.**
///
/// This is the assertion the list did not have, and its absence is why three exemptions
/// described code that was not in the module they excused for months. Length is not evidence:
/// `pacman.rs`'s claim that the removal guard needed pacman's essential data ran to 118
/// characters and there was no `essential()` impl in the file.
#[test]
fn every_reason_points_at_a_line_that_is_actually_there() {
    let modules = backend_modules();
    let mut unproven: Vec<String> = Vec::new();
    for h in HAND_WRITTEN {
        assert!(
            h.proof.len() >= 6 && h.proof != h.module.trim_end_matches(".rs"),
            "{}'s proof is the module's own name or too short to identify anything: {:?}",
            h.module,
            h.proof
        );
        let Some((_, src)) = modules.iter().find(|(n, _)| n == h.module) else {
            continue; // `the_hand_written_list_has_no_stale_entries` owns that failure.
        };
        if !src.contains(h.proof) {
            unproven.push(format!("{} claims `{}`", h.module, h.proof));
        }
    }
    assert!(
        unproven.is_empty(),
        "these exemptions rest on code that is not in the module:\n    {}\n\n\
         Either the module lost the thing that made it exempt — in which case it is a row now \
         — or the reason was never true. Both were the case here on 2026-08-06: `pacman.rs` \
         named essential data it had no impl for, and `dnf.rs` described \
         `ManualListing::Command {{ format: SameAsInstalled }}` in prose.",
        unproven.join("\n    ")
    );
}

/// A gate that has never failed is a claim, not a check.
///
/// So the instrument is run against a planted falsehood before it is trusted with the real
/// list — `grade3_resource_idempotency_tests` self-tests its mtime comparison the same way, and
/// this file's own history is the argument for it.
#[test]
fn the_proof_check_can_actually_fail() {
    let modules = backend_modules();
    let (_, brew) = modules
        .iter()
        .find(|(n, _)| n == "brew.rs")
        .expect("brew.rs is a hand-written backend");
    assert!(
        brew.contains("\"list\", \"--versions\""),
        "the real proof must be found, or the check passes by matching nothing"
    );
    assert!(
        !brew.contains("fn essential"),
        "a proof that is not in the module must NOT be found — this is the exact shape of \
         pacman's false claim, asserted against a module that never had it"
    );
}

/// How far there is to go, printed rather than asserted.
///
/// **Not a threshold.** A test that failed when the count rose would be gamed by adding a
/// reason, and one that failed when it fell would have to be edited on every conversion. The
/// number is here to be read — `cargo test --test backend_is_data_not_code_tests -- --nocapture`
/// — because the direction doc asks each phase to say what it bought, and a count nobody can see
/// is a count nobody writes down.
#[test]
fn report_the_distance_to_go() {
    let modules = backend_modules();
    let hand = modules
        .iter()
        .filter(|(_, s)| s.contains("impl BackendCore for"))
        .count();
    let to_convert = HAND_WRITTEN
        .iter()
        .filter(|h| h.why.starts_with("TO CONVERT"))
        .count();
    eprintln!(
        "backend modules: {} total, {hand} hand-written, {to_convert} of those marked TO CONVERT",
        modules.len()
    );
    assert!(hand >= to_convert);
}
