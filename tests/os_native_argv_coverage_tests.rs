//! Every backend's argv must be checkable on every platform's CI, not only on the OS that
//! ships its manager.
//!
//! `registry.rs`'s own doc comment records the bug: *"These registrars were
//! `#[cfg(target_os = …)]` until 2026-07-26, so `mas`'s verbs were only ever compiled on a Mac
//! and `apt`'s only on Linux — a typo in either was invisible to every other platform's CI,
//! and there is no Mac in this project at all. They are compiled everywhere now."*
//!
//! They were not all compiled everywhere. `pub mod psresource` in `src/backends/mod.rs` kept
//! its `#[cfg(target_os = "windows")]`, so on Linux and macOS the module does not exist — which
//! also makes it the one OS-native backend that *cannot* appear in the argv table, because the
//! table would not compile. And `psresource` is separately excused from a real lifecycle on the
//! Windows harness, so it had no argv check off Windows and no lifecycle anywhere: two
//! independent blindfolds on one backend, each of which looks like the other one's job.
//!
//! **Why a source scan.** The defect is a registrar that is *not in* a table. Nothing the
//! program does can enumerate what nobody wrote down; only the source can. So this counts the
//! registrars on every run and makes a new one fail until someone either gives it a row or
//! writes down why it cannot have one.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::ledger::{Entry, Ledger};

fn repo_root() -> PathBuf {
    // Compile-time, not the working directory: a test that reads `./src` passes or fails
    // depending on where `cargo test` was invoked from.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let p: PathBuf = repo_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// A registrar with no row in the argv table, and the reason it cannot have one.
///
/// The reason is the whole point. An exemption list without one is a list of things nobody
/// looked at, which is the state this test exists to end.
struct NoRow {
    registrar: &'static str,
    why: &'static str,
}

const NO_ROW: &[NoRow] = &[
    NoRow {
        registrar: "register_generic",
        why: "not a backend — the shared constructor every generic registrar below calls. \
              Its argv is whatever its caller configures, and each caller has its own row.",
    },
    NoRow {
        registrar: "backends::nixos::register",
        why: "has no per-package argv to record. Every other backend here turns a package name \
              into a command line, which is what the table checks for a typo; `nixos:` turns the \
              whole declared set into a generated Nix module and then runs one fixed \
              `nixos-rebuild switch`. There is no install or remove verb, so a row would be a \
              template nothing fills. What the table protects — a wrong verb invisible on a \
              platform that cannot run the manager — is covered instead by \
              `scripts/nix-validate.sh`, which asks a real Nix parser about the generated file.",
    },
    NoRow {
        registrar: "register_aur_helper",
        why: "takes five arguments, so it cannot be a `Registrar`. The two backends it builds \
              have rows of their own via `register_yay` and `register_paru`.",
    },
];

#[test]
fn no_backend_module_is_compiled_on_one_os_only() {
    let src = read("src/backends/mod.rs");
    let mut gated: Vec<String> = Vec::new();
    let lines: Vec<&str> = src.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if !line.trim_start().starts_with("#[cfg(target_os") {
            continue;
        }
        // The declaration the attribute applies to is the next non-attribute line.
        let decl = lines[i + 1..]
            .iter()
            .find(|l| !l.trim_start().starts_with("#["))
            .copied()
            .unwrap_or("");
        if decl.contains("mod ") {
            gated.push(format!("{}  ->  {}", line.trim(), decl.trim()));
        }
    }
    assert!(
        gated.is_empty(),
        "these backend modules are compiled on one OS only, so their code — including their \
         install and remove argv — is invisible to every other platform's CI:\n    {}\n\n\
         Register them behind `cfg!(target_os = …)` in `create_default_registry` (a runtime \
         gate on a module that always compiles), the way winget, scoop, choco, mas and apt \
         already are. That is what registry.rs's own doc comment says was done on 2026-07-26.",
        gated.join("\n    ")
    );
}

/// The text of the argv table, so a registrar named in a comment elsewhere does not count.
///
/// Read through the harness because the table lives inside `#[cfg(test)]`, which the production
/// registry reader strips — two questions about one directory, and two functions rather than one
/// that answers whichever the caller happened to mean.
fn argv_table(_src: &str) -> String {
    crate::harness::registry_argv_table()
}

/// A row satisfies `needle` only when the match is not part of a longer identifier —
/// `register_pkg` must not be satisfied by `register_pkgin`.
fn mentions(table: &str, needle: &str) -> bool {
    table.match_indices(needle).any(|(i, _)| {
        !matches!(table.as_bytes().get(i + needle.len()), Some(c) if c.is_ascii_alphanumeric() || *c == b'_')
    })
}

/// Every registrar this build can reach, in the two shapes they are written in.
///
/// **Both halves, which is the whole point of this scan.** Until 2026-08-04 it collected only
/// `fn register_*` — the generic registrars written in `registry.rs` — and the twenty-eight
/// backends that register from their own modules were invisible to it. The scan that existed
/// to stop a backend going uncovered was itself covering half the family, which is the defect
/// class `CLAUDE.md` opens with. `brew`, `npm`, `nix`, `snap`, `pacman` and twenty-three others
/// had no argv row, no exemption, and a green gate saying otherwise.
fn registrars(src: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    // Shape one: `fn register_apt(...)` defined in registry.rs.
    for line in src.lines() {
        if let Some(rest) = line.strip_prefix("fn register_") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                out.push(format!("register_{name}"));
            }
        }
    }

    // Shape two: `crate::backends::brew::register(...)` called from `create_default_registry`.
    // Read from the production function only — the test module calls registrars too, and a
    // registrar reachable *only* from a test is not something this build ships.
    let production = src
        .split_once("pub async fn create_default_registry(")
        .expect("create_default_registry moved or was renamed")
        .1
        .split_once("\n}")
        .expect("create_default_registry has no end")
        .0;
    for chunk in production.split("crate::backends::").skip(1) {
        let module: String = chunk
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        let after = &chunk[module.len()..];
        if after.starts_with("::register(") && !module.is_empty() {
            let entry = format!("backends::{module}::register");
            if !out.contains(&entry) {
                out.push(entry);
            }
        }
    }
    out
}

#[test]
fn every_registrar_has_an_argv_row_or_a_written_reason() {
    let src = crate::harness::registry_source();
    let defined = registrars(&src);

    // Floors on the SCAN, not counts of the backends. A parse that finds nothing passes every
    // assertion below it, and that is what these guard. They sit far under the real numbers on
    // purpose: a conversion moves registrars from the second list to the first, and a floor
    // that has to be edited each time is one more thing the sweep has to remember. It was
    // `> 20` on both, and converting `dnf`, `pacman` and `xbps` on 2026-08-06 took the module
    // half to exactly 20 — a green gate failing on a change that made the code better.
    assert!(
        defined
            .iter()
            .filter(|r| r.starts_with("register_"))
            .count()
            > 10,
        "found too few generic registrars — the scan is broken, not the code"
    );
    assert!(
        defined
            .iter()
            .filter(|r| r.starts_with("backends::"))
            .count()
            > 10,
        "found too few module registrars — the scan is broken, not the code. This half was \
         missing entirely until 2026-08-04 and a zero here reads exactly like the bug."
    );

    let table = argv_table(&src);

    // **Before the ledger, because the ledger's stale check would pre-empt this one with a
    // vaguer sentence.** A registrar with BOTH a row and an exemption saying it cannot have one
    // is a contradiction that survives silently: the scan takes the row and never reads the
    // reason. `helm` was exempt on the grounds that a row "would pass on the remove alone",
    // which stopped being true the moment rows could carry options, and nothing said so.
    let contradicted: Vec<&str> = NO_ROW
        .iter()
        .filter(|n| mentions(&table, n.registrar))
        .map(|n| n.registrar)
        .collect();
    assert!(
        contradicted.is_empty(),
        "these registrars have BOTH an argv row and a NO_ROW exemption saying they cannot have \
         one: {contradicted:?}\n\n\
         Delete the exemption. A reason that is no longer true is worse than no reason: it \
         reads as considered."
    );

    let rowless: BTreeSet<String> = defined
        .iter()
        .filter(|r| !mentions(&table, r))
        .cloned()
        .collect();

    Ledger::of("a registrar with no row in the argv table", "NO_ROW")
        .exempting(NO_ROW.iter().map(|n| Entry {
            site: n.registrar,
            why: n.why,
        }))
        .scanning_at_least(30)
        .remedy(
            "Give each one a row — the table is what makes a typo in an install or remove verb \
             visible on a platform that cannot run the manager.",
        )
        .audit(defined.len(), &rowless);
}

/// A gate that has never failed is a claim, not a check.
#[test]
fn the_registrar_scan_can_actually_fail() {
    let src = crate::harness::registry_source();

    // Both shapes are found in the real file.
    let found = registrars(&src);
    assert!(
        found.iter().any(|r| r == "register_apt"),
        "the generic-registrar scan stopped finding `register_apt`"
    );
    assert!(
        found.iter().any(|r| r == "backends::brew::register"),
        "the module-registrar scan stopped finding `backends::brew::register` — this is the \
         half that was missing, so a silent zero here is the original bug returning"
    );

    // A registrar with no row is reported rather than passed over.
    let table = argv_table(&src);
    assert!(
        !mentions(&table, "backends::shall_nonexistent::register"),
        "the table cannot mention a module that does not exist"
    );

    // Word-boundary: a longer name must not satisfy a shorter one.
    assert!(!mentions("register_pkgin", "register_pkg"));
    assert!(mentions("register_pkg,", "register_pkg"));
}

#[test]
fn the_scan_can_actually_fail() {
    // The two tests above are greps, and a grep that matches nothing passes. This proves the
    // module-gate scan sees a gate when there is one, using a fixture rather than the real
    // file — IV.1: a check whose only evidence is the right answer is not evidence.
    let planted = "pub mod alpha;\n#[cfg(target_os = \"plan9\")]\npub mod beta;\n";
    let lines: Vec<&str> = planted.lines().collect();
    let found = lines.iter().enumerate().any(|(i, l)| {
        l.trim_start().starts_with("#[cfg(target_os")
            && lines
                .get(i + 1)
                .map(|d| d.contains("mod "))
                .unwrap_or(false)
    });
    assert!(
        found,
        "the module-gate scan cannot see a gate that is there"
    );

    let _ = Path::new("");
}
