//! How many backends have actually been driven, stated in a document, checked against the file
//! that measures it.
//!
//! `SPEC.md` said *"23 have ever been run against a real package manager — 7 per distro image, 18
//! in the `tools` image"*. `scripts/lifecycle-floor.txt` — which is written by the harness itself
//! and ratcheted upward by passing runs — recorded 26 for that image and 13 for the Windows
//! runner. The README's list of driven managers implied a third number again. Three documents,
//! one fact, no two agreeing: the same failure this repository has now had over the backend
//! count, the decision register's totals, and the exit-code table.
//!
//! **The distinguishing feature of this one is that a green run raises the number.** The floor is
//! a ratchet, so every successful sweep can move it — which makes a hand-typed copy wrong not
//! through neglect but through success. That is the strongest possible argument for deriving it,
//! and it is why the prose now cites the file and this test compares the two.
//!
//! It checks the direction that can mislead a reader: a document must not claim **more** coverage
//! than the ratchet records. Claiming less is merely out of date; claiming more is a promise the
//! project cannot keep, and this section of the README exists precisely to avoid making one.

use std::collections::BTreeMap;
use std::path::PathBuf;

fn read(rel: &str) -> String {
    let p: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
        .replace("\r\n", "\n")
}

/// `container-linux-tools-local 26` -> `("container-linux-tools-local", 26)`.
fn floors() -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for line in read("scripts/lifecycle-floor.txt").lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((class, n)) = line.rsplit_once(' ') else {
            continue;
        };
        if let Ok(n) = n.trim().parse::<usize>() {
            out.insert(class.trim().to_string(), n);
        }
    }
    out
}

/// The self-test: the file this whole gate reads must still be the shape it thinks it is.
#[test]
fn the_ratchet_is_readable_and_not_empty() {
    let floors = floors();
    assert!(
        floors.len() >= 8,
        "read {} host classes from scripts/lifecycle-floor.txt; the parser or the file has \
         changed shape, and every assertion below is over what it managed to read",
        floors.len()
    );
    assert!(
        floors.values().all(|n| *n > 0),
        "a host class records a floor of zero, which is not a floor: {floors:?}"
    );
}

/// No document claims more round-tripped backends than any host class has ever achieved.
///
/// The ceiling is the best single host, because a backend is round-tripped *somewhere* rather
/// than everywhere — no runner has every manager on it, and summing the classes would count apt
/// six times.
#[test]
fn no_document_claims_more_lifecycle_coverage_than_the_ratchet_records() {
    let floors = floors();
    let best = *floors.values().max().expect("the ratchet has entries");

    // Every number the README's coverage table states, read from the table itself.
    let readme = read("README.md");
    let table = readme
        .split_once("| host class | backends round-tripped |")
        .unwrap_or_else(|| {
            panic!(
                "README.md no longer contains the lifecycle coverage table this gate reads. If \
                 the wording changed, change it here too rather than deleting the assertion."
            )
        })
        .1;
    let table = table.split("\n\n").next().unwrap_or("");

    let mut stated: Vec<usize> = Vec::new();
    for row in table.lines().filter(|l| l.starts_with('|')) {
        if let Some(cell) = row.rsplit('|').nth(1) {
            if let Ok(n) = cell.trim().parse::<usize>() {
                stated.push(n);
            }
        }
    }

    // The self-test again: a table that parsed to nothing would pass every check below.
    assert!(
        stated.len() >= 5,
        "parsed {} numbers out of the README's coverage table; the reader is broken, not the \
         table",
        stated.len()
    );

    let overclaimed: Vec<usize> = stated.iter().copied().filter(|n| *n > best).collect();
    assert!(
        overclaimed.is_empty(),
        "README.md's coverage table states {overclaimed:?} round-tripped backends, and the best \
         host class in scripts/lifecycle-floor.txt has ever reached {best}. A document may lag \
         the ratchet; it may not promise more than the harness has ever measured."
    );

    // And every stated number must actually be one the ratchet records, not a plausible
    // neighbour of one — the failure mode a `<=` check alone cannot see.
    let recorded: Vec<usize> = floors.values().copied().collect();
    let invented: Vec<usize> = stated
        .iter()
        .copied()
        .filter(|n| !recorded.contains(n))
        .collect();
    assert!(
        invented.is_empty(),
        "README.md's coverage table states {invented:?}, which no host class in \
         scripts/lifecycle-floor.txt records. The recorded floors are {recorded:?} — update the \
         table from the file rather than from memory."
    );
}

/// `SPEC.md` cites the ratchet rather than copying it, and names the best figure correctly.
#[test]
fn the_spec_defers_to_the_ratchet_instead_of_restating_it() {
    let spec = read("docs/SPEC.md");
    let best = *floors().values().max().expect("the ratchet has entries");

    assert!(
        spec.contains("scripts/lifecycle-floor.txt"),
        "SPEC.md's readiness paragraph no longer points at the file that measures lifecycle \
         coverage. It stated the number itself once and the number went stale by being beaten."
    );
    assert!(
        spec.contains(&format!("reaches **{best}**")),
        "SPEC.md does not name {best} as the best-covered image, which is what the ratchet \
         currently records."
    );
}

/// The README's "argv-tested only" table names exactly the backends `UNPROVEN` names.
///
/// **The dangerous direction is the omission.** A manager that is in `UNPROVEN` and missing from
/// the table reads to a user as one of the many that *are* driven — the README's own sentence
/// above the table promises the difference is "taken from the harnesses' own tables rather than
/// from anybody's memory", and memory is exactly what a hand-maintained copy is. `mas` was
/// missing from it on the day this test was written.
///
/// The other direction fails too: a backend listed here that has since been driven turns a
/// released claim into an out-of-date apology for work that was done.
#[test]
fn the_readme_names_exactly_the_backends_no_harness_has_driven() {
    use std::collections::BTreeSet;

    let readme = read("README.md");
    let table = readme
        .split_once("| backend | why nothing has driven it |")
        .unwrap_or_else(|| {
            panic!(
                "README.md no longer contains the argv-only table this gate reads. If the wording \
                 changed, change it here too rather than deleting the assertion."
            )
        })
        .1;
    let table = table.split("\n\n").next().unwrap_or("");

    // The names live in the first cell, backticked, and a cell may hold several
    // (`pkg`, `pkg_add`, `pkgin`).
    let mut stated: BTreeSet<String> = BTreeSet::new();
    for row in table.lines().filter(|l| l.starts_with('|')) {
        let Some(first) = row.split('|').nth(1) else {
            continue;
        };
        for piece in first.split('`').skip(1).step_by(2) {
            let name = piece.trim();
            if !name.is_empty() {
                stated.insert(name.to_string());
            }
        }
    }
    assert!(
        stated.len() >= 5,
        "parsed {} names out of the README's argv-only table; the reader is broken, not the table",
        stated.len()
    );

    let unproven: BTreeSet<String> = shall::backends::proving::UNPROVEN
        .iter()
        .map(|(name, _)| name.to_string())
        .collect();

    let missing: Vec<&String> = unproven.difference(&stated).collect();
    assert!(
        missing.is_empty(),
        "src/backends/proving.rs says no harness has driven {missing:?}, and the README's \
         argv-only table does not name them — so a reader counts them among the driven."
    );

    // **The table is legitimately a SUPERSET of `UNPROVEN`, and the difference is not slack.**
    // `UNPROVEN` answers *can any harness reach this at all*; a backend can have a canary row —
    // so the union gate counts it reachable — and still be refused by every image it appears on,
    // which each harness records per-image in `no_lifecycle_reason`. `snap` is exactly that: a
    // canary in the container table, and `snapd is a Linux daemon over systemd ... no image in
    // the matrix runs systemd either` in the Windows one.
    //
    // So an extra row must be a backend some harness refuses BY NAME. One that is in neither
    // place is a claim with nothing behind it — which is what `yay` and `paru` became the hour
    // the arch image started running the sweep unprivileged.
    let refused: BTreeSet<String> = [
        "docker/integration/run-in-container.sh",
        "scripts/integration-windows.sh",
    ]
    .iter()
    .flat_map(|f| {
        let body = read(f);
        let body = body
            .split_once("\nno_lifecycle_reason() {\n")
            .map(|(_, rest)| rest.split_once("\n}\n").map(|(b, _)| b.to_string()))
            .unwrap_or_default()
            .unwrap_or_default();
        body.lines()
            .map(str::trim)
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.split_once(')').map(|(head, _)| head.to_string()))
            .flat_map(|head| {
                head.split('|')
                    .map(str::trim)
                    .filter(|n| {
                        !n.is_empty()
                            && n.chars()
                                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                    })
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    })
    .collect();
    assert!(
        refused.len() >= 10,
        "read {} refusal labels out of the two harnesses; the reader is broken, not the tables",
        refused.len()
    );

    let unbacked: Vec<&String> = stated
        .difference(&unproven)
        .filter(|n| !refused.contains(*n))
        .collect();
    assert!(
        unbacked.is_empty(),
        "the README's argv-only table names {unbacked:?}, which `UNPROVEN` does not list and no \
         harness refuses by name — so nothing in this repository still says they are undriven, \
         and something probably drove them."
    );
}
