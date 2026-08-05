//! The check that would have caught G-1: enumerate every path that removes something from
//! the machine, from the code, and require each one to be accounted for.
//!
//! `readme.md` says "**every path that removes anything** goes through one guard".
//! `src/app/sync/guard.rs`'s own module doc says "*Every* path that deletes is guarded... A
//! guard on one command is a guard on nothing." Both sentences were true when written. Neither
//! was ever re-derived, and by 2026-07-28 the count was eleven sites and nine guards — the
//! `link:`/`service:`/`setting:`/`shim:`/`schedule:`/`repo:` teardown in `app/apply/extras.rs`
//! and the `linix repo remove` verb in `verbs/declare.rs` both deleted without asking.
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
struct Accounted {
    file: &'static str,
    calls: usize,
    guarded_by: &'static str,
}

const LEDGER: &[Accounted] = &[
    Accounted {
        file: "src/app/leases.rs",
        calls: 1,
        guarded_by: "guard::enforce at leases.rs:51, GuardScope::ExpirySweep",
    },
    Accounted {
        file: "src/app/apply/extras.rs",
        calls: 4,
        guarded_by: "guard::enforce_extras at extras.rs:65, over the whole drift set before \
                     any kind is dispatched (W21) — including the shim a package line asks \
                     for with `@shim`/`@sandbox`, which resolves to a `shim:` extra (G-1)",
    },
    // `src/app/sync/mod.rs` used to be here: `heal` called `handler.remove` from a serial loop
    // of its own. That loop is gone — recovery runs on the transaction engine — so the call
    // moved into `transaction.rs` below and this file now reaches no removal itself. **The gate
    // did not move**: `heal` still enforces per entry, before an interrupted removal is put in
    // the graph at all, which is what the entry below records.
    Accounted {
        file: "src/core/transaction.rs",
        calls: 3,
        guarded_by: "the purge/remove pair at :500-502 executes a plan enforced at \
                     sync/mod.rs:141 — or, for a recovery, per entry at sync/mod.rs:798 \
                     (GuardScope::Heal) before that entry becomes a node; the rollback \
                     removal at :714 is enforced at :688",
    },
    Accounted {
        file: "src/verbs/cleanup.rs",
        calls: 2,
        guarded_by: "enforce at cleanup.rs:57 (RemoveOrphans) and enforce_deliberate at :220 \
                     (PurgeUndeclared)",
    },
    Accounted {
        file: "src/verbs/declare.rs",
        calls: 1,
        guarded_by: "guard::enforce_extras at declare.rs:36, GuardScope::Remove (W21) — the \
                     imperative twin of the `repo:` teardown",
    },
    Accounted {
        file: "src/verbs/packages.rs",
        calls: 1,
        guarded_by: "guard::enforce at packages.rs:423, GuardScope::Remove",
    },
    Accounted {
        file: "src/verbs/plan.rs",
        calls: 1,
        guarded_by: "guard::enforce at plan.rs:368, GuardScope::Apply",
    },
];

/// Does this line reach a backend's removal?
///
/// Keyed on the `sudo` argument every backend removal carries rather than on the method name
/// alone, so `HashMap::remove` and `Vec::remove` — of which this codebase has many — do not
/// drown the signal. The three resource removals that take no `sudo` are named outright.
fn is_removal_call(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with("//") {
        return false;
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
            out.push(p);
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
        match ledger.get(file.as_str()) {
            None => problems.push(format!(
                "UNACCOUNTED: {} reaches a removal at {:?} and is in no ledger entry.\n    \
                 Add it to LEDGER in this file with the guard that stands in front of it — \
                 or, if nothing does, put a guard there first. This is exactly how G-1 \
                 survived: the path existed and the sentence about it was never re-counted.",
                file,
                sites.iter().map(|(l, _)| *l).collect::<Vec<_>>()
            )),
            Some(acc) if acc.calls != sites.len() => problems.push(format!(
                "COUNT MOVED: {} was recorded with {} removal call(s) guarded by [{}], and \
                 now has {} — at lines {:?}.\n    A new removal in an already-guarded file is \
                 not automatically guarded: check it, then update the count.",
                file,
                acc.calls,
                acc.guarded_by,
                sites.len(),
                sites.iter().map(|(l, _)| *l).collect::<Vec<_>>()
            )),
            Some(_) => {}
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
         readme.md says every path that removes anything goes through one guard. That \
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
    // them through the engine: one fewer *file* reaching a removal is the fix landing, not the
    // scan breaking.
    let found = removal_sites();
    assert!(
        found.len() >= 7,
        "the scan found only {} file(s) with removals, which is fewer than this program has: {:?}",
        found.len(),
        found.keys().collect::<Vec<_>>()
    );
}
