//! Every plan that may remove has to say what it is allowed to remove from.
//!
//! `ChangePlanner::plan` computes removals as `managed − desired`, so a caller that hands it a
//! `desired` narrower than the machine gets a removal planned for everything outside it. The old
//! signature took `Option<Scope>`, where `None` meant both *do not filter `desired`* and *reap
//! every backend on the box*; five of the eight call sites passed `None`, and four of the five
//! wanted only the first. `PlanScope` splits the two facts, so the compiler now makes every
//! caller answer — but the compiler cannot tell a `HostBackends` that came from `priority` from
//! one somebody assembled by hand, and that is the whole content of the rule.
//!
//! **Why a source scan and not a behavioural test.** `a_plan_reaps_only_what_it_was_asked_about`
//! proves the planner honours each variant and that the two callers reachable from an
//! integration test pass the right one. It cannot reach `src/verbs/` at all — `main.rs` declares
//! `mod verbs;`, private to the binary at the time — it is `pub mod verbs;` in `lib.rs` now —
//! and two of the four bugs were in there. A scan reaches
//! them, and it also catches the site nobody has written yet, which is the shape this defect
//! keeps taking: `plan`/`apply` sat three lines of git history from the sibling that had the
//! fix *and the comment explaining it*, in the same file, and did not get it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A production call to `ChangePlanner::plan`, and the scope it names.
#[derive(Debug)]
struct Site {
    file: String,
    line: usize,
    /// The argument as written, whitespace collapsed — e.g. `PlanScope::Whole(hosts)`.
    scope: String,
}

/// What each call site is entitled to plan, and why.
///
/// The reason is not decoration: `activate` reaped every backend on the box for as long as it
/// did because "it is a narrower operation than `sync`" sounded like a reason and was never
/// written next to the code that had to be true for it.
struct Accounted {
    file: &'static str,
    /// The `PlanScope` variants this file's planning calls may name — every one of them, so a
    /// site that gains a second, wider scope has to be written down rather than absorbed.
    variants: &'static [&'static str],
    why: &'static str,
}

const LEDGER: &[Accounted] = &[
    Accounted {
        file: "src/app/profile.rs",
        variants: &["Whole"],
        why: "`activate`/`deactivate` converge the whole config, so drift is real — bounded by \
              `priority`, which this site did not consult until 2026-08-06",
    },
    Accounted {
        file: "src/app/shell/mod.rs",
        variants: &["JustThese"],
        why: "`provision_transient_env`'s desired set holds the shell's requests and nothing \
              else; read as a converge it made every other managed package a removal",
    },
    Accounted {
        file: "src/verbs/check.rs",
        variants: &["Whole"],
        why: "`check drift` reports what a full `sync` would do, so it scopes the same way",
    },
    Accounted {
        file: "src/verbs/plan.rs",
        variants: &["Whole"],
        why: "`status` and the saved plan both answer \"what would `sync` do\" — and the saved \
              plan is applied later, so an unscoped removal in it outlives the run that made it",
    },
    Accounted {
        file: "src/verbs/setup.rs",
        variants: &["Whole", "Narrowed"],
        why: "`upgrade --canary` with no `--profile`/`--module` is a whole-machine converge \
              behind a health check; with one it narrows, which is the `match` at that site",
    },
    Accounted {
        file: "src/verbs/sync.rs",
        variants: &["Whole"],
        why: "the reconcile itself — the site every other one is measured against",
    },
    Accounted {
        file: "src/verbs/upgrade.rs",
        variants: &["Narrowed"],
        why: "mode 4 is reached only with a `--profile`/`--module`; an unscoped `upgrade` \
              returns at mode 3, and the `let Some(scope) = … else` says so in the code",
    },
];

/// The one place a [`HostBackends`] may be minted: the resolver that read `priority`.
const MINT: &str = "src/app/sync/resolver.rs";

/// What the scan records for a call whose scope it cannot read off the call itself.
const UNREADABLE: &str = "<computed elsewhere>";

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

/// The `PlanScope` argument of every `.plan(` call in `text`, with the line it starts on.
///
/// Reads forward with paren balancing rather than matching one line, because rustfmt breaks
/// these calls across three lines as often as not — and a scan that only sees single-line calls
/// would have found four of the eight sites and reported the surface clean.
fn plan_calls(text: &str) -> Vec<(usize, String)> {
    let bytes: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let needle: Vec<char> = ".plan(".chars().collect();
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        if bytes[i..i + needle.len()] != needle[..] {
            i += 1;
            continue;
        }
        let line = text[..text
            .char_indices()
            .nth(i)
            .map(|(b, _)| b)
            .unwrap_or(text.len())]
            .lines()
            .count();
        // Balance from the open paren to its match.
        let mut depth = 0usize;
        let mut j = i + needle.len() - 1;
        let start = j + 1;
        while j < bytes.len() {
            match bytes[j] {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        let args: String = bytes[start..j.min(bytes.len())].iter().collect();
        out.push((line, args.split_whitespace().collect::<Vec<_>>().join(" ")));
        i = j.max(i + 1);
    }
    out
}

/// Production `.plan(` calls, by file. Scanning stops at `#[cfg(test)]` for the same reason the
/// removal-guard enumeration does: a unit test planning against a fake registry is not a path a
/// user can reach.
fn plan_sites() -> Vec<Site> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    sources(&root.join("src"), &mut files);
    files.sort();

    let mut out = Vec::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        let production = match text.find("#[cfg(test)]") {
            Some(at) => &text[..at],
            None => &text[..],
        };
        let rel = f
            .strip_prefix(root)
            .unwrap_or(&f)
            .to_string_lossy()
            .replace('\\', "/");
        for (line, args) in plan_calls(production) {
            // A planning call is one that takes a desired set and a scope. Anything else named
            // `.plan(` in `src/` is somebody else's method and not this gate's business.
            if !args.starts_with('&') {
                continue;
            }
            // **Not a `continue`.** A call whose scope is computed above it and passed as a
            // binding is one this scan cannot read — and an unreadable site is indistinguishable
            // from a safe one, which is the whole disease. `upgrade --canary` hid here for one
            // run of this gate by writing `planner.plan(&desired, scope)`.
            let scope = match args.split_once("PlanScope::") {
                Some((_, rest)) => format!("PlanScope::{}", rest.trim()),
                None => UNREADABLE.to_string(),
            };
            out.push(Site {
                file: rel.clone(),
                line,
                scope,
            });
        }
    }
    out
}

/// The variant a site names, or the whole argument if it names none.
fn variant_of(scope: &str) -> &str {
    for v in ["Whole", "Narrowed", "JustThese"] {
        if scope.starts_with(&format!("PlanScope::{}", v)) {
            return v;
        }
    }
    scope
}

#[test]
fn every_plan_names_the_scope_its_ledger_entry_records() {
    let sites = plan_sites();
    assert!(
        sites.len() >= LEDGER.len(),
        "the scan found {} planning site(s) and the ledger records {} — a scan that reads \
         fewer sites than are written down is measuring itself, not the program",
        sites.len(),
        LEDGER.len()
    );

    let ledger: BTreeMap<&str, &Accounted> = LEDGER.iter().map(|a| (a.file, a)).collect();
    let mut problems = Vec::new();

    for site in &sites {
        match ledger.get(site.file.as_str()) {
            None => problems.push(format!(
                "UNACCOUNTED: {}:{} plans with {} and is in no ledger entry.\n    \
                 Add it here with the reason that site is allowed to remove what it removes. \
                 A plan is `managed − desired`: if `desired` is not this machine's whole \
                 declaration set, `Whole` reaps everything the caller did not ask about.",
                site.file, site.line, site.scope
            )),
            Some(_) if site.scope == UNREADABLE => problems.push(format!(
                "UNREADABLE: {}:{} passes a scope this gate cannot see — it is computed above \
                 the call and handed over as a binding.\n    \
                 Name the variant at the call. A scope the gate cannot read is one it reports \
                 as clean, and `upgrade --canary` spent a run hidden behind exactly this.",
                site.file, site.line
            )),
            Some(acc) if !acc.variants.contains(&variant_of(&site.scope)) => {
                problems.push(format!(
                    "SCOPE MOVED: {}:{} names `{}`, which is not among the recorded {:?}.\n    \
                 Reason on record: {}.\n    \
                 Widening a plan's scope widens what it deletes. Check it, then update this \
                 entry — do not update the entry to match the code.",
                    site.file, site.line, site.scope, acc.variants, acc.why
                ))
            }
            Some(_) => {}
        }
    }

    // The half that rots: an entry naming a file that no longer plans is a rule about nothing,
    // and it is also how a list comes to describe a program that has moved on.
    for acc in LEDGER {
        if !sites.iter().any(|s| s.file == acc.file) {
            problems.push(format!(
                "STALE: the ledger names {} but it plans nothing any more. Delete the entry.",
                acc.file
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "the set of paths that can plan a removal has moved since it was last counted:\n\n{}",
        problems.join("\n\n")
    );
}

/// A `Whole` plan must be handed the host's real backend list, never a default.
///
/// `HostBackends::default()` is empty, and empty means every backend — the right answer for a
/// host whose `priority` could not be read, and the wrong one for a caller that simply did not
/// ask. The four bugs were all this: a reap with no list, which is a reap of everything.
#[test]
fn no_production_converge_reaps_with_a_default_backend_list() {
    let offenders: Vec<String> = plan_sites()
        .into_iter()
        .filter(|s| variant_of(&s.scope) == "Whole")
        .filter(|s| {
            let inner = s.scope.trim_start_matches("PlanScope::Whole").trim();
            inner.contains("default()") || inner == "()" || inner.contains("Default::default")
        })
        .map(|s| format!("{}:{} — {}", s.file, s.line, s.scope))
        .collect();

    assert!(
        offenders.is_empty(),
        "these sites converge the whole machine against an empty backend list, which allows \
         every manager on the box — including the ones `priority` deliberately does not name:\
         \n\n{}\n\nHand them `host_backends()`.",
        offenders.join("\n")
    );
}

/// `HostBackends` is minted in exactly one place: the resolver that read `priority`.
///
/// The newtype is only worth having while this is true. A second constructor is a second answer
/// to "which managers are this host's own", and the day it disagrees with the file it wins
/// silently — which is the defect `priority` itself was introduced to end (V.15).
#[test]
fn only_the_resolver_mints_a_host_backend_list() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    sources(&root.join("src"), &mut files);

    let mut minters = Vec::new();
    for f in &files {
        let Ok(text) = std::fs::read_to_string(f) else {
            continue;
        };
        let production = match text.find("#[cfg(test)]") {
            Some(at) => &text[..at],
            None => &text[..],
        };
        let rel = f
            .strip_prefix(root)
            .unwrap_or(f)
            .to_string_lossy()
            .replace('\\', "/");
        for (i, line) in production.lines().enumerate() {
            if line.trim_start().starts_with("//") || line.trim_start().starts_with("///") {
                continue;
            }
            if line.contains("HostBackends::from_priority") {
                minters.push(format!("{}:{}", rel, i + 1));
            }
        }
    }

    assert_eq!(
        minters.len(),
        1,
        "`HostBackends::from_priority` must be called exactly once in `src/`, from {} — \
         found: {:?}",
        MINT,
        minters
    );
    assert!(
        minters[0].starts_with(MINT),
        "the one call is at {} and belongs in {}",
        minters[0],
        MINT
    );
}

/// The oracle test: prove the scanner can see what it claims to see, before trusting it.
///
/// GRADE's rule — "do not test your own oracle by assuming it works". A scan that returns
/// nothing passes every assertion above, and would report a program with eight unscoped reaps
/// as clean.
#[test]
fn the_enumeration_can_actually_read_a_planning_call() {
    // The single-line form.
    let one = plan_calls("    planner.plan(&desired, PlanScope::JustThese).await?;");
    assert_eq!(one.len(), 1, "missed a single-line call");
    assert!(one[0].1.contains("PlanScope::JustThese"));

    // The form rustfmt actually produces, which is why this balances parens instead of
    // matching a line: split across three, with a nested call in the argument.
    let split = plan_calls(
        "        planner\n            .plan(&state.packages, PlanScope::Whole(hosts))\n            .await?",
    );
    assert_eq!(split.len(), 1, "missed a call rustfmt broke across lines");
    assert_eq!(
        variant_of(&format!(
            "PlanScope::{}",
            split[0].1.split("PlanScope::").nth(1).unwrap()
        )),
        "Whole"
    );

    // Nested parens inside the argument must not end the scan early.
    let nested = plan_calls(".plan(&d, PlanScope::Narrowed(Scope::Module(\"dev\".into())))");
    assert_eq!(nested.len(), 1);
    assert!(
        nested[0].1.contains("Scope::Module"),
        "the scan stopped at the first `)` and truncated the argument: {:?}",
        nested[0].1
    );

    // The hole this gate found in itself on its first run: a scope bound above the call.
    // `plan_sites` must record that as unreadable, never skip it.
    let hidden = plan_calls("        planner.plan(&desired, scope).await?");
    assert_eq!(hidden.len(), 1, "missed a call whose scope is a binding");
    assert!(
        !hidden[0].1.contains("PlanScope::"),
        "this is the shape that must be reported unreadable, and it is not"
    );

    // And the controls, or the assertions above would pass for a scan that matches anything.
    assert!(
        plan_calls("    let plan = self.plan_name.clone();").is_empty(),
        "matched an identifier that merely starts with `plan`"
    );
    assert_eq!(variant_of("PlanScope::Whole(hosts)"), "Whole");
    assert_eq!(variant_of("PlanScope::JustThese"), "JustThese");
    assert_ne!(
        variant_of("PlanScope::Narrowed(s)"),
        "Whole",
        "the variant reader cannot tell the scopes apart"
    );
}
