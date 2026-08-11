//! `readme.md`: "A write-ahead log records **every mutation** before it runs. If Shall is
//! killed mid-transaction, the next run heals it." This test does the counting that sentence
//! quantifies over.
//!
//! The sibling of `removal_guard_enumeration_tests.rs`, and written for the same reason one
//! command over. That file asks "what stands in front of this removal"; this one asks "what
//! records this mutation before it happens" — and until 2026-08-06 the answer for eight files,
//! eleven call sites between them, was nothing. A review named one of them (`apply`) and called
//! it *the* path recovery cannot reach. It was one of eight, which is what an enumeration is
//! for and a reading is not.
//!
//! **Why a source scan.** The defect is a path that exists and is not covered; no behavioural
//! test can enumerate the paths nobody wrote a test for — that is the shape of the bug. So this
//! asserts a structural property: a call that mutates a package through a backend fails the
//! build until someone writes down what records it.
//!
//! **What a mutation needs a record for is not "it changes the machine".** II.19 draws the line
//! and this ledger keeps it: a resource — `link:`, `service:`, `setting:`, `firewall:`,
//! `schedule:`, `repo:` — is converged from a declaration, so an interrupted one is finished by
//! the next `sync` reading the machine and seeing the declaration unmet. Recomputing is a
//! *better* recovery than replaying, because it also corrects drift the log never saw. A
//! package is not that: an interrupted `apt install` wedges dpkg in a state no declaration
//! describes. So resource sites are accounted `Recomputed`, and they are accounted rather than
//! filtered out — the reason has to be written down, because "it is a resource" is a claim
//! about the call, and the three that were wrong about themselves were wrong in prose.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// How a file's package mutations survive being killed half-way through.
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
enum Recovery {
    /// The call is inside the transaction engine, which writes the WAL entry itself.
    Transaction,
    /// The call is wrapped in `journal::journalled`, so an interruption is `heal`'s to finish.
    Journalled,
    /// The call converges a resource from a declaration; the next `sync` recomputes it (II.19).
    /// Not a gap — a deliberate exclusion, and the one this ledger exists to keep honest.
    Recomputed,
}

/// A file that mutates the machine through a backend, how many such calls it holds, and what
/// makes an interruption recoverable.
///
/// The `how` is not decoration. `apply`'s entry could have been written as prose about the
/// guard it calls — it does call one — and the guard has nothing to do with recovery. Naming
/// the *mechanism* is what makes the claim checkable by reading one line of the file.
struct Accounted {
    file: &'static str,
    calls: usize,
    recovery: Recovery,
    how: &'static str,
}

const LEDGER: &[Accounted] = &[
    // ---- The engine. Every package `sync`, `apply`, `rebuild`, `prune` and `heal` schedules
    // arrives here, and `record_start` runs before the manager is invoked.
    Accounted {
        file: "src/core/transaction.rs",
        calls: 5,
        recovery: Recovery::Transaction,
        how: "record_start per node before the batch runs; a WAL write that fails makes the \
              batch stillborn rather than letting it run unrecorded",
    },
    // ---- Package mutations outside the engine. Each one is a command that reaches a manager
    // without a plan behind it, so it carries its own record.
    Accounted {
        file: "src/app/leases.rs",
        calls: 2,
        recovery: Recovery::Journalled,
        how: "journalled() around the expiry sweep's removal and the suspension restore's \
              reinstall",
    },
    Accounted {
        file: "src/app/run.rs",
        calls: 1,
        recovery: Recovery::Journalled,
        how: "journalled() around the auto-provision install — `shall run` installs a real \
              package with a real manager, and calling it temporary describes the intent, not \
              what dpkg is left holding",
    },
    Accounted {
        file: "src/app/shell/mod.rs",
        calls: 1,
        recovery: Recovery::Journalled,
        how: "journalled() around the ephemeral-shell install",
    },
    // `src/app/diagnostics.rs` was here, accounted `Journalled` for "the remediation
    // install". `Y14` wrapped that install in a write-ahead record while the function holding
    // it — `handle_failure`/`remediate`, 115 lines that prompted and then installed packages —
    // already had **zero callers**, and F-6 deleted it. The file mutates nothing now, so it is
    // out of the ledger rather than accounted for a call it no longer makes. This entry going
    // is the deletion landing, not the scan breaking.
    // `src/verbs/cleanup.rs` was here, accounted `Journalled` for two hand-rolled loops. `LX-5`
    // routed `remove-orphans` and `purge-undeclared` through `SyncEngine`, which journals every
    // node it runs — so the file mutates nothing directly, and its entry going is that routing
    // landing rather than the scan breaking. The journalling is counted under
    // `core/transaction.rs` now, with everything else the engine carries out.
    Accounted {
        file: "src/verbs/packages.rs",
        calls: 1,
        recovery: Recovery::Journalled,
        how: "journalled() around the suspend removal",
    },
    Accounted {
        file: "src/verbs/upgrade.rs",
        calls: 1,
        recovery: Recovery::Journalled,
        how: "journalled() around `upgrade_one`'s install",
    },
    // ---- Resources. II.19: converged from a declaration, so the next sync recomputes them.
    Accounted {
        file: "src/app/apply/dependents.rs",
        calls: 1,
        recovery: Recovery::Recomputed,
        how: "`service:`, `link:` and `setting:` reached through Installable because that is \
              the trait the backends implement — each is a read-then-write converge from a \
              line in the config, and the next sync finishes an interrupted one. One call site \
              rather than three: `LX-9` collapsed three byte-identical match arms into \
              `apply_through_backend`, which is the same behaviour written once",
    },
    Accounted {
        file: "src/app/apply/dotfiles.rs",
        calls: 1,
        recovery: Recovery::Recomputed,
        how: "a tree expands into the `link:` lines it stands for, and `link:` carries T6's \
              backup-and-restore; the placement is recomputed from the declaration",
    },
    Accounted {
        file: "src/app/apply/extras.rs",
        calls: 1,
        recovery: Recovery::Recomputed,
        how: "the teardown of an extra whose declaration is gone; `reconcile` computes the \
              drift again on the next run, which is why a kill mid-teardown retries rather \
              than replays",
    },
    Accounted {
        file: "src/verbs/declare.rs",
        calls: 1,
        recovery: Recovery::Recomputed,
        how: "`shall service enable` writes a `service:` line and converges it — the \
              imperative twin of the Dependents phase above",
    },
];

/// Does this line mutate the machine through a backend?
///
/// Keyed on the `sudo` argument every backend write carries, so `HashMap::remove` and
/// `Vec::remove` — of which this codebase has many — do not drown the signal. `.install(` is
/// matched on its own: nothing else in the tree is called `install`, and the one install site
/// that took no sudo argument is exactly the kind of thing a sudo-only rule would miss.
fn is_mutation_call(line: &str) -> bool {
    is_mutation_call_in(line, "")
}

/// Whether `line` opens a mutation, reading `rest` — the few lines after it — for the argument
/// that tells a manager's `remove` from a collection's.
///
/// **`sudo` is on the same *call*, not always the same *line*.** The original predicate required
/// both in one string, which made the audit sensitive to line wrapping: `apply/extras.rs` put
/// `inst.remove(` and `b.sudo_for_write()` on separate lines and vanished from the mutation
/// surface — the ledger then reported it as an entry naming a file that mutates nothing, when
/// what had changed was the formatting. **A gate that a rustfmt pass can blind is not a gate.**
fn is_mutation_call_in(line: &str, rest: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with("//") || t.starts_with("///") {
        return false;
    }
    if t.contains(".install(") {
        return true;
    }
    // `sudo` is what separates a manager's removal from `Vec::remove`/`HashMap::remove`, which
    // take an index or a key and no privilege.
    (t.contains(".remove(") || t.contains(".purge("))
        && (line.contains("sudo") || rest.contains("sudo"))
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

/// Mutation calls per file, in production code only.
///
/// Scanning stops at `#[cfg(test)]`, and skips `src/backends/`: a backend module IS the
/// manager's argv, so its own `install`/`remove` are the implementation of the call this
/// ledger tracks, not another instance of it. Counting them would make the ledger a list of
/// backends.
fn mutation_sites() -> BTreeMap<String, Vec<(usize, String)>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    sources(&root.join("src"), &mut files);
    files.sort();

    let mut found: BTreeMap<String, Vec<(usize, String)>> = BTreeMap::new();
    for f in files {
        let rel = f
            .strip_prefix(root)
            .unwrap_or(&f)
            .to_string_lossy()
            .replace('\\', "/");
        if rel.starts_with("src/backends/") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("#[cfg(test)]") {
                break;
            }
            // The call, not the line: an argument list wrapped across lines is the same call.
            let rest: String = text
                .lines()
                .skip(i + 1)
                .take(4)
                .take_while(|l| !l.trim_start().starts_with("fn "))
                .collect::<Vec<_>>()
                .join(
                    "
",
                );
            if is_mutation_call_in(line, &rest) {
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
fn every_package_mutation_is_recorded_before_it_runs() {
    let found = mutation_sites();
    let ledger: BTreeMap<&str, &Accounted> = LEDGER.iter().map(|a| (a.file, a)).collect();

    let mut problems = Vec::new();

    for (file, sites) in &found {
        match ledger.get(file.as_str()) {
            None => problems.push(format!(
                "UNRECORDED: {} mutates the machine at {:?} and is in no ledger entry.\n    \
                 Wrap it in `journal::journalled` and add it here — or, if it converges a \
                 resource from a declaration, account it `Recomputed` and say which \
                 declaration. A mutation nothing recorded is a mutation `heal` cannot see.",
                file,
                sites.iter().map(|(l, _)| *l).collect::<Vec<_>>()
            )),
            Some(acc) if acc.calls != sites.len() => problems.push(format!(
                "COUNT MOVED: {} was recorded with {} mutation(s) recovered by {:?} — {} — \
                 and now has {}, at lines {:?}.\n    A new mutation in an already-accounted \
                 file is not automatically recorded: check it against that sentence, then \
                 update the count.",
                file,
                acc.calls,
                acc.recovery,
                acc.how,
                sites.len(),
                sites.iter().map(|(l, _)| *l).collect::<Vec<_>>()
            )),
            Some(_) => {}
        }
    }

    // The half that rots. A ledger entry naming a file that no longer mutates anything is a
    // sentence about a program that has moved on — and it is how `apply` came to be described
    // by four documents as journalled while it was not.
    for acc in LEDGER {
        if !found.contains_key(acc.file) {
            problems.push(format!(
                "STALE: LEDGER names {} but it mutates nothing any more. Delete the entry.",
                acc.file
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "the mutation surface has moved since it was last counted:\n\n{}\n\n\
         readme.md says a write-ahead log records every mutation before it runs. That \
         sentence is only true while this list is.",
        problems.join("\n\n")
    );
}

/// Every file the ledger says is journalled must actually contain the call it claims.
///
/// The reason this exists: the three false backend exemptions were sixty characters of fluent
/// prose about code that was not in the module they excused, and the check on them asserted
/// only that the prose was long enough. A `how` that names a mechanism is checkable — so check
/// it, rather than shipping a second list whose entries are English.
#[test]
fn a_journalled_entry_names_a_file_that_journals() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut missing = Vec::new();
    for acc in LEDGER.iter().filter(|a| a.recovery == Recovery::Journalled) {
        let text = std::fs::read_to_string(root.join(acc.file)).unwrap_or_default();
        if !text.contains("journalled(") {
            missing.push(acc.file);
        }
    }
    assert!(
        missing.is_empty(),
        "these files are accounted `Journalled` and call no `journalled(`: {:?}",
        missing
    );
}

/// The oracle: this enumeration must be able to see a site that is really there.
///
/// GRADE §"Do not test your own oracle by assuming it works". The gate written for the phase
/// dispatch searched for a substring that survived the deletion it was meant to catch, so it
/// shipped unable to fail — twice, in two rulings. Before trusting the scan above, feed it the
/// exact lines it exists to find.
#[test]
fn the_enumeration_can_actually_see_a_mutation() {
    // The two lines `apply` walked serially, with no WAL behind either of them.
    assert!(is_mutation_call(
        "                .install(std::slice::from_ref(spec), b.sudo_for_write())"
    ));
    assert!(is_mutation_call(
        "                .remove(std::slice::from_ref(&r.name), b.sudo_for_write())"
    ));
    // A batch removal, which is what `remove-orphans` issues.
    assert!(is_mutation_call(
        "            installable.remove(names, backend.sudo_for_write())"
    ));
    assert!(is_mutation_call(
        "        handler.purge(&names, sudo).await"
    ));
    // An install with no `sudo` word on the line at all — `run.rs` passes it in a variable, and
    // a sudo-keyed rule would have skipped the one call site nothing else covers.
    assert!(is_mutation_call(
        "                    installer.install(std::slice::from_ref(spec), sudo).await?;"
    ));

    // And the controls, or every assertion above would pass for a scan that returns true.
    assert!(!is_mutation_call("        self.packages.remove(pos);"));
    assert!(!is_mutation_call("        in_progress.remove(&idx);"));
    assert!(!is_mutation_call(
        "    // inst.install(std::slice::from_ref(&spec), b.sudo_for_write()) is journalled"
    ));
    assert!(!is_mutation_call(
        "    /// Reinstall a single package by backend + name."
    ));

    // And it must find something in the real tree: a scan whose patterns silently stopped
    // matching would report an empty map and pass the ledger test for the worst reason.
    let found = mutation_sites();
    assert!(
        // Nine, not ten: `verbs/cleanup.rs` left the surface when `LX-5` routed its two removal
        // loops through the engine. Still a real floor — a scan whose patterns silently stopped
        // matching would report an empty map and pass the ledger test for the worst reason.
        found.len() >= 9,
        "the scan found mutation sites in only {} file(s) — it has stopped matching",
        found.len()
    );
    assert!(
        found.contains_key("src/core/transaction.rs"),
        "the scan cannot see the transaction engine's own install/remove"
    );
}
