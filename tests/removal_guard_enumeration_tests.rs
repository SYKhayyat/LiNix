//! The check that would have caught G-1: enumerate every path that removes something from
//! the machine, from the code, and require each one to be accounted for.
//!
//! `README.md` says "**every path that removes anything** goes through one guard".
//! `src/app/sync/guard.rs`'s own module doc says "*Every* path that deletes is guarded... A
//! guard on one command is a guard on nothing." Both sentences were true when written. Neither
//! was ever re-derived, and by 2026-07-28 the count was eleven sites and nine guards — the
//! `link:`/`service:`/`setting:`/`shim:`/`schedule:`/`repo:` teardown in `app/apply/extras.rs`
//! and the `shall repo remove` verb in `verbs/declare.rs` both deleted without asking.
//!
//! A sentence that quantifies over paths is only as good as the last time someone counted the
//! paths. This test does the counting on every run.
//!
//! **Why a source scan and not a behavioural test.** The finding is about a path that *exists*
//! and is *not covered*; no behaviour can enumerate the paths nobody wrote a test for — that
//! is the shape of the bug. So this asserts a structural property, and it earns its keep the
//! only way such a check can: adding a removal call anywhere in `src/` fails it until someone
//! writes down which guard stands in front of it. It is deliberately not a grep for the word
//! `guard` — `scripts/grader-red-tests.sh` was deleted for being exactly that.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A file that reaches a removal, how many such calls it holds, and what guards them.
///
/// The reason is not decoration: a site whose reason cannot be written down is a site nobody
/// has checked. Nine of these were verified by reading the call, not by trusting a list.
#[allow(dead_code)] // `guarded_by` is prose for a reader, and that is its whole job.
struct Accounted {
    file: &'static str,
    /// **No longer a count.** It was `calls: usize`, and the doc comment above still explains
    /// why that was the wrong shape: a legitimate second removal added to an already-guarded
    /// file reddened the build until somebody incremented an integer, which teaches a reader to
    /// bump the number rather than to check the call. With `Reaped` required by every effector,
    /// a new removal in a guarded file already *has* a guard — the compiler made it get one —
    /// so the number was measuring churn.
    guarded_by: &'static str,
}

const LEDGER: &[Accounted] = &[
    Accounted {
        file: "src/app/leases.rs",
        guarded_by: "guard::enforce at leases.rs:51, GuardScope::ExpirySweep",
    },
    Accounted {
        file: "src/app/apply/extras.rs",
        guarded_by: "guard::enforce_extras at extras.rs:65, over the whole drift set before \
                     any kind is dispatched (W21) — including the shim a package line asks \
                     for with `@shim`/`@sandbox`, which resolves to a `shim:` extra (G-1)",
    },
    // `src/verbs/plan.rs` used to be here, and its entry named `guard::enforce` at
    // `GuardScope::Apply` — a guard `apply` called for itself because it executed its removals
    // in a serial loop of its own. The loop is gone: a frozen plan is handed to
    // `SyncEngine::sync`, which enforces over the same graph under the same scope before the
    // first manager runs. **The gate did not move**, and it is now the same call `sync` makes
    // rather than a second one that could disagree with it.
    //
    // `src/app/sync/mod.rs` used to be here: `heal` called `handler.remove` from a serial loop
    // of its own. That loop is gone — recovery runs on the transaction engine — so the call
    // moved into `transaction.rs` below and this file now reaches no removal itself. **The gate
    // did not move**: `heal` still enforces per entry, before an interrupted removal is put in
    // the graph at all, which is what the entry below records.
    Accounted {
        file: "src/core/transaction.rs",
        guarded_by: "the purge/remove pair at :500-502 executes a plan enforced at \
                     sync/mod.rs:141 — or, for a recovery, per entry at sync/mod.rs:798 \
                     (GuardScope::Heal) before that entry becomes a node; the rollback \
                     removal at :714 is enforced at :688",
    },
    // `src/verbs/cleanup.rs` was here, and its absence is `LX-5` landing: `remove-orphans` and
    // `purge-undeclared` each kept a private removal loop, and both now build a graph and hand it
    // to `SyncEngine`. The file reaches no `inst.remove` of its own, so it is not a removal
    // surface — the guard covering it is the engine's, counted under `core/transaction.rs`.
    Accounted {
        file: "src/verbs/declare.rs",
        guarded_by: "guard::enforce_extras at declare.rs:36, GuardScope::Remove (W21) — the \
                     imperative twin of the `repo:` teardown",
    },
    Accounted {
        file: "src/verbs/packages.rs",
        guarded_by: "guard::enforce at packages.rs:612, GuardScope::Remove — and the token it                      returns is what `inst.remove` at :638 takes, so the comment above that                      call is now the compiler's to keep",
    },
    // The three new entries below are the `Reaped` change itself, and none of them is a
    // removal *site*: they are where the type is declared and where it travels.
    Accounted {
        file: "src/app/sync/guard.rs",
        guarded_by: "the guard itself: `Reaped` is declared here and minted by `enforce`,                      `enforce_extras` and `enforce_deliberate`",
    },
    Accounted {
        file: "src/app/sync/mod.rs",
        guarded_by: "guard::enforce at mod.rs:193 (the sync plan) and :872 (per interrupted                      entry, GuardScope::Heal). The engine carries the token to the executor                      rather than dropping it on the line that produced it",
    },
    Accounted {
        file: "src/app/apply/firewall.rs",
        guarded_by: "**THE FINDING.** `guard::enforce_extras` over `to_close`, before the                      first `deny_command` runs. Until 2026-08-07 the word `guard` appeared                      nowhere in this file — not an import, not a call, not a comment — while                      it closed every open port no `firewall:` line declared. `max_removals`                      did not count them, `protected` could not name them, and                      `--allow-mass-removal` was not consulted. Three bespoke refusals were                      written here instead of calling the one guard two hundred lines away",
    },
];

/// Does this line reach a backend's removal?
///
/// **Keyed on `Reaped`, the type the guard mints, rather than on how the call is spelled.**
///
/// It was spelled: `.remove(`/`.purge(` with `sudo` on the line, plus `.remove_repo(`,
/// `.remove_shim(` and `.deprovision(` named outright. That predicate is what let
/// `apply/firewall.rs` close every undeclared port with `deny_command` and match none of it —
/// the word `guard` appeared nowhere in the file, and the check written to prevent exactly that
/// could not see it. **The fix for `G-1` replaced a stale list of paths with a stale list of
/// verbs**, and the staleness moved into a predicate with a passing self-test, where nobody
/// re-derives it.
///
/// The verbs are kept below because they still identify the calls; what changed is that they no
/// longer *have* to be right. An effector cannot be called without a `Reaped`, and a `Reaped`
/// cannot be obtained without asking, so a sixth removal path added tomorrow fails to compile
/// rather than failing to be noticed. This scan is now the ledger's index, not the safety
/// property.
fn is_removal_call(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with("//") || t.starts_with("///") {
        return false;
    }
    // A *declaration* names the token as a parameter with a type; a *call* passes it as an
    // argument. Twenty backends implement `remove`, and an implementation is not a path — it is
    // the far end of one. Without this the scan reports every implementor and says nothing.
    let declares = t.contains("fn ")
        || t.starts_with("reaped:")
        || t.starts_with("_reaped:")
        || t.contains("reaped: crate::app::sync::guard::Reaped")
        || t.contains("reaped: guard::Reaped")
        || t.contains("reaped: Option<");
    if declares {
        return false;
    }
    // The token names itself at every call site, whatever the method is called.
    if t.contains("reaped") || t.contains("Reaped") {
        return true;
    }
    let sudo_removal = (t.contains(".remove(") || t.contains(".purge(")) && line.contains("sudo");
    sudo_removal
        || t.contains(".remove_repo(")
        || t.contains(".remove_shim(")
        || t.contains(".deprovision(")
}

/// Every `.rs` file under `src/`.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            sources(&p, out);
        } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
            // A test module that lives in its own file under `src/` carries no in-file
            // `#[cfg(test)]` marker — the gate is on the `mod` line in its parent. Its removals
            // are a unit test's, not a path a user can reach, and counting them would make the
            // ledger track test churn instead of the safety surface, which is the same reason
            // the scan below stops at `#[cfg(test)]`.
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            if !(name.ends_with("_test.rs") || name.ends_with("_tests.rs")) {
                out.push(p);
            }
        }
    }
}

/// Removal calls per file, in production code only.
///
/// Scanning stops at `#[cfg(test)]`: a unit test that removes a repo from a fake registry is
/// not a path a user can reach, and counting it would make the ledger track test churn instead
/// of the safety surface.
fn removal_sites() -> BTreeMap<String, Vec<(usize, String)>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    sources(&root.join("src"), &mut files);
    files.sort();

    let mut found: BTreeMap<String, Vec<(usize, String)>> = BTreeMap::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        let rel = f
            .strip_prefix(root)
            .unwrap_or(&f)
            .to_string_lossy()
            .replace('\\', "/");
        for (i, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("#[cfg(test)]") {
                break;
            }
            if is_removal_call(line) {
                found
                    .entry(rel.clone())
                    .or_default()
                    .push((i + 1, line.trim().to_string()));
            }
        }
    }
    found
}

#[test]
fn every_path_that_removes_anything_is_accounted_for() {
    let found = removal_sites();
    let ledger: BTreeMap<&str, &Accounted> = LEDGER.iter().map(|a| (a.file, a)).collect();

    let mut problems = Vec::new();

    for (file, sites) in &found {
        if !ledger.contains_key(file.as_str()) {
            problems.push(format!(
                "UNACCOUNTED: {} reaches a removal at {:?} and is in no ledger entry.\n    \
                 Add it to LEDGER in this file with the guard that stands in front of it — \
                 or, if nothing does, put a guard there first. This is exactly how G-1 \
                 survived: the path existed and the sentence about it was never re-counted.",
                file,
                sites.iter().map(|(l, _)| *l).collect::<Vec<_>>()
            ));
        }
    }

    // The other half, and the half that rots: a ledger entry naming a file that no longer
    // removes anything is a guard nobody needs, and it is also how a list comes to describe a
    // program that has moved on. READINESS §5.3: a list is an assertion about what is absent,
    // and nothing verifies that half.
    for acc in LEDGER {
        if !found.contains_key(acc.file) {
            problems.push(format!(
                "STALE: LEDGER names {} but it reaches no removal any more. Delete the entry.",
                acc.file
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "the removal surface has moved since it was last counted:\n\n{}\n\n\
         README.md says every path that removes anything goes through one guard. That \
         sentence is only true while this list is.",
        problems.join("\n\n")
    );
}

/// The oracle test: this enumeration must be able to see a site that is really there.
///
/// GRADE §"Do not test your own oracle by assuming it works" — "All 24 READY backends answer
/// `list`" was measured, true, and meaningless, because a backend that does not exist answers
/// the same way. So before trusting the scan above, feed it something it must catch.
#[test]
fn the_enumeration_can_actually_see_a_removal() {
    assert!(
        is_removal_call(
            "            inst.remove(std::slice::from_ref(&id.to_string()), b.sudo_for_write())"
        ),
        "the scan missed the exact line G-1 was about"
    );
    assert!(is_removal_call("    handler.purge(one, sudo).await?;"));
    assert!(is_removal_call(
        "    mgr.remove_repo(name, b.sudo_for_write()).await?;"
    ));
    assert!(is_removal_call(
        "  self.scheduler.deprovision(self.executor, id).await,"
    ));

    // And the controls, or the assertions above would pass for a scan that returns true always.
    assert!(!is_removal_call("        self.packages.remove(pos);"));
    assert!(!is_removal_call("        store.remove(key);"));
    assert!(!is_removal_call(
        "    // mgr.remove_repo(name, b.sudo_for_write()) is guarded"
    ));

    // And it must find something in the real tree: a scan whose patterns silently stopped
    // matching would report an empty map and pass the test above for the worst reason.
    //
    // The floor is a floor and not a count — it is the "did the scan still work" question, not
    // the "is every site accounted for" one, which the ledger above answers exactly. It came
    // down from 8 to 7 when `heal` stopped issuing its own removals and started scheduling
    // them through the engine, and from 7 to 6 when `apply` did the same: one fewer *file*
    // reaching a removal is the fix landing, not the scan breaking. Twice now, for the same
    // reason — a command that stopped keeping its own copy of the loop.
    let found = removal_sites();
    assert!(
        found.len() >= 6,
        "the scan found only {} file(s) with removals, which is fewer than this program has: {:?}",
        found.len(),
        found.keys().collect::<Vec<_>>()
    );
}
