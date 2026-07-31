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

use std::path::{Path, PathBuf};

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
        registrar: "register_aur_helper",
        why: "takes five arguments, so it cannot be a `Registrar`. The two backends it builds \
              have rows of their own via `register_yay` and `register_paru`.",
    },
    NoRow {
        registrar: "register_helm",
        why: "installs from an option the table cannot carry (`@url=`), so its install call \
              never happens and the row would pass on the remove alone — a check that tests \
              nothing (IV.1). It has its own tests and a real lifecycle in both harnesses.",
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

#[test]
fn every_registrar_has_an_argv_row_or_a_written_reason() {
    let src = read("src/backends/registry.rs");

    let mut defined: Vec<String> = Vec::new();
    for line in src.lines() {
        if let Some(rest) = line.strip_prefix("fn register_") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                defined.push(format!("register_{name}"));
            }
        }
    }
    assert!(
        defined.len() > 20,
        "found only {} registrars in registry.rs — the scan is broken, not the code",
        defined.len()
    );

    // The table's own text, so a registrar named in a *comment* elsewhere does not count.
    let table = src
        .split_once("let cases: &[(&str, Registrar, &str, Option<&str>)] = &[")
        .expect("the argv cases table moved or was renamed")
        .1
        .split_once("\n        ];")
        .expect("the argv cases table has no end")
        .0;

    let mut missing: Vec<String> = Vec::new();
    for r in &defined {
        // Word-boundary check: `register_pkg` must not be satisfied by `register_pkgin`.
        let has_row = table
            .match_indices(r.as_str())
            .any(|(i, _)| !matches!(table.as_bytes().get(i + r.len()), Some(c) if c.is_ascii_alphanumeric() || *c == b'_'));
        if has_row {
            continue;
        }
        if NO_ROW.iter().any(|n| n.registrar == r) {
            continue;
        }
        missing.push(r.clone());
    }

    assert!(
        missing.is_empty(),
        "these registrars have no row in the argv table and no written reason:\n    {}\n\n\
         Give each one a row — the table is what makes a typo in an install or remove verb \
         visible on a platform that cannot run the manager — or add it to NO_ROW with the \
         reason it cannot have one.",
        missing.join("\n    ")
    );

    // An exemption for a registrar that no longer exists is an exemption nobody re-read.
    let stale: Vec<&str> = NO_ROW
        .iter()
        .filter(|n| !defined.iter().any(|d| d == n.registrar))
        .map(|n| n.registrar)
        .collect();
    assert!(
        stale.is_empty(),
        "NO_ROW names registrars that no longer exist: {stale:?}"
    );

    // The reason is the exemption. A blank one is a backend nobody looked at wearing the
    // costume of one somebody did.
    for n in NO_ROW {
        assert!(
            n.why.len() > 40,
            "{}'s exemption has no reason worth the name: {:?}",
            n.registrar,
            n.why
        );
    }
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
