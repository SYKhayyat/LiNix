//! The order of a sync is one list, and this is what keeps it one.
//!
//! **The bug this exists to prevent has already happened four times.** Which phase a statement
//! kind belongs to was written down in places that could not check each other — the dispatch
//! in `sync`, the dry-run branch's copy of it, `DesiredState`'s per-kind accessors, and
//! `has_non_package_work`'s chain of ors — and every kind added since was missed by one of
//! them: extras (S20), then `exec:`, then `dotfiles:`, then `firewall:`. `verbs/sync.rs`
//! records the bill in its own words.
//!
//! `Statement::phase()` is now the single answer and the compiler checks it: a new statement
//! kind cannot compile until it has been given a phase. What the compiler *cannot* check is the
//! other half — that `sync`'s dispatch has an arm for every phase — because `verbs/` is
//! declared in `main.rs` and is not reachable from any test binary. So the dispatch is gated
//! here the way the removal surface is: from the source, on every run.
//!
//! **Why a source scan.** The finding is about a phase that exists and is *not dispatched*. No
//! behavioural test can enumerate the phases nobody wired up — that is the shape of the bug.

use linix::config::grammar::{Options, Phase, Statement};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// The variant names in a `pub enum NAME { … }` block, in declaration order.
///
/// Deliberately reads the source rather than any list a human keeps: a hand-copied list of the
/// variants is the thing this test exists to make impossible.
fn variants_of_enum(source: &str, enum_name: &str) -> Vec<String> {
    let needle = format!("pub enum {} {{", enum_name);
    let start = source
        .find(&needle)
        .unwrap_or_else(|| panic!("`{}` is not declared in the source scanned", enum_name))
        + needle.len();
    let body = &source[start..];
    let end = body
        .find("\n}")
        .unwrap_or_else(|| panic!("`{}`'s body is never closed", enum_name));

    let mut out = Vec::new();
    for line in body[..end].lines() {
        let line = line.trim();
        // Doc comments, ordinary comments, attributes and blank lines are not variants.
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        let name: String = line
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        // A variant starts with an uppercase letter; a field of a struct variant does not.
        if name.chars().next().is_some_and(char::is_uppercase) {
            out.push(name);
        }
    }
    out
}

fn statement_source() -> String {
    std::fs::read_to_string(root().join("src/config/grammar/statement.rs"))
        .expect("src/config/grammar/statement.rs must be readable")
}

/// `Phase::all()` walks the `next()` chain rather than a second list — this proves the walk
/// actually reaches every variant the enum declares.
///
/// A variant added to the enum and given a successor, but that nothing points *at*, would be
/// unreachable from `Resolution` and would silently never run. That is precisely the shape of
/// the four misses, one level up.
#[test]
fn every_declared_phase_is_reachable_and_in_declaration_order() {
    let declared = variants_of_enum(&statement_source(), "Phase");
    let walked: Vec<String> = Phase::all().map(|p| format!("{:?}", p)).collect();

    assert_eq!(
        declared, walked,
        "the phases `Phase` declares and the phases `Phase::all()` walks disagree.\n  \
         declared: {:?}\n  walked:   {:?}\n\
         `all()` follows `next()`, so a variant added without being pointed at is unreachable \
         and its phase would never run.",
        declared, walked
    );
}

/// `Ord` is the sync's order, because `has_non_package_work` is a `>` comparison against
/// `Phase::Packages`. If the variants are ever reordered so that `Ord` and the run order
/// disagree, that comparison silently starts answering a different question.
#[test]
fn ord_is_the_run_order() {
    let walked: Vec<Phase> = Phase::all().collect();
    for pair in walked.windows(2) {
        assert!(
            pair[0] < pair[1],
            "{:?} runs before {:?} but does not sort before it — `phase > Phase::Packages` is \
             how `sync` asks whether there is work after the package plan, so `Ord` and the \
             run order have to be the same order",
            pair[0],
            pair[1]
        );
    }
    assert!(
        walked.contains(&Phase::Packages),
        "the comparison everything else is written against must itself be in the walk"
    );
}

/// The half the compiler cannot reach: `verbs/` is private to the binary (`main.rs`), so no
/// test binary can call `apply_non_package_phases`. The `match` inside it is exhaustive, so
/// the compiler does force an arm per phase — but only for whoever builds the binary, and a
/// phase can still be dispatched to nothing by being folded into the ignored arm. This asserts
/// each post-package phase is named on its own.
#[test]
fn syncs_dispatch_names_every_phase_after_the_packages() {
    let source = std::fs::read_to_string(root().join("src/verbs/sync.rs"))
        .expect("src/verbs/sync.rs must be readable");
    let body = dispatch_body(&source);

    let mut missing = Vec::new();
    for phase in Phase::after_packages() {
        if !dispatches(&body, phase) {
            missing.push(format!("{:?}", phase));
        }
    }

    assert!(
        missing.is_empty(),
        "`apply_non_package_phases` has no arm of its own for: {}.\n\
         Every phase after the package plan is work `sync` must dispatch. A phase folded into \
         the ignored arm — or given an arm with an empty body — is a phase that silently does \
         nothing, which is how extras, `exec:`, `dotfiles:` and `firewall:` were each missed \
         in turn.",
        missing.join(", ")
    );
}

/// Whether `body` dispatches `phase` to actual work.
///
/// **Three ways to look dispatched without being dispatched, and it must catch all three.** A
/// mention in a comment; an arm with an empty body (`Phase::Execs => {}`); and the one that
/// caught this test out when it was first written — being the last alternative of the ignored
/// or-pattern, `A | B | Phase::Execs => {}`, where a plain substring search for
/// `Phase::Execs =>` matches and reports the phase as covered. So the arm must *open* its line
/// and its body must not be empty.
fn dispatches(body: &str, phase: Phase) -> bool {
    let arm = format!("Phase::{:?} =>", phase);
    body.lines().any(|line| {
        let line = line.trim();
        if line.starts_with("//") {
            return false;
        }
        let Some(rest) = line.strip_prefix(&arm) else {
            return false;
        };
        let rest = rest.trim().trim_end_matches(',').trim();
        !rest.is_empty() && rest != "{}"
    })
}

/// The dispatch loop's body, from the `for phase in` to the end of its `match`.
fn dispatch_body(source: &str) -> String {
    let start = source
        .find("pub(crate) async fn apply_non_package_phases")
        .expect("`apply_non_package_phases` must still exist — it is the one phase list");
    let rest = &source[start..];
    let end = rest
        .find("\n}")
        .expect("`apply_non_package_phases` must be a closed function");
    rest[..end].to_string()
}

/// The oracle: before trusting either scan above, feed it something it must catch and
/// something it must not.
///
/// A scan that silently stopped matching would report nothing missing and pass every assertion
/// above for the worst possible reason.
#[test]
fn the_scans_can_actually_see_what_they_look_for() {
    // The variant reader, on a body carrying every shape the real enum has: doc comments,
    // attributes, plain variants, and a struct variant whose fields are lowercase.
    let synthetic = r#"
#[derive(Debug)]
pub enum Phase {
    /// A doc comment.
    First,
    // An ordinary comment.
    #[allow(dead_code)]
    Second,
    Third {
        field: String,
        other: usize,
    },
}
"#;
    assert_eq!(
        variants_of_enum(synthetic, "Phase"),
        vec!["First", "Second", "Third"],
        "the variant reader must skip doc comments, comments, attributes and struct fields"
    );

    // And on the real file, it must find the phases rather than an empty list.
    let real = variants_of_enum(&statement_source(), "Phase");
    assert!(
        real.len() >= 8,
        "the variant reader found {} phases in the real source — it has stopped matching",
        real.len()
    );
    assert!(real.contains(&"Packages".to_string()));

    // The dispatch scan, against every way an undispatched phase can look dispatched.
    assert!(
        dispatches(
            "            Phase::Execs => app.execs().apply(state).await?,",
            Phase::Execs
        ),
        "the scan must see a real arm — this is the line it is written to find"
    );
    assert!(
        dispatches(
            "Phase::Execs => {\n    app.execs().apply(state).await?\n}",
            Phase::Execs
        ),
        "an arm opening a block is still an arm"
    );

    // And the controls, each of which this scan got wrong before a mutation caught it.
    assert!(
        !dispatches(
            "            // Phase::Execs => handled elsewhere",
            Phase::Execs
        ),
        "a commented-out arm must not read as a dispatched phase"
    );
    assert!(
        !dispatches(
            "            Phase::Resolution | Phase::Repositories | Phase::Execs => {}",
            Phase::Execs
        ),
        "a phase folded into the ignored or-pattern is not dispatched — a substring search for \
         `Phase::Execs =>` matches this line, which is how this scan first shipped unable to \
         fail"
    );
    assert!(
        !dispatches("            Phase::Execs => {}", Phase::Execs),
        "an arm with an empty body dispatches the phase to nothing"
    );
    assert!(
        !dispatches(
            "            Phase::Dependents => app.dependents().apply(state).await?,",
            Phase::Execs
        ),
        "one phase's arm must not answer for another's"
    );
}

/// Every statement kind lands in the phase II.7 says it does.
///
/// Written out per kind rather than derived, because this is the assertion — deriving it from
/// `phase()` would be the function agreeing with itself.
#[test]
fn each_statement_kind_declares_the_phase_it_runs_in() {
    let opts = Options::default;
    let cases: Vec<(Statement, Phase)> = vec![
        (
            Statement::Repo {
                backend: "apt".into(),
                spec: "ppa:x/y".into(),
            },
            Phase::Repositories,
        ),
        (Statement::Shim("rg".into(), opts()), Phase::Dependents),
        (
            Statement::Service("nginx".into(), opts()),
            Phase::Dependents,
        ),
        (Statement::Link("vimrc".into(), opts()), Phase::Dependents),
        (
            Statement::Setting("org.gnome/foo".into(), opts()),
            Phase::Dependents,
        ),
        (
            Statement::Dotfiles("dotfiles".into(), opts()),
            Phase::Dotfiles,
        ),
        (
            Statement::Firewall("22/tcp".into(), opts()),
            Phase::Firewall,
        ),
        (
            Statement::Schedule("clean".into(), opts()),
            Phase::Schedules,
        ),
        (Statement::Exec("setup.sh".into(), opts()), Phase::Execs),
        (
            Statement::Generate("gen.sh".into(), opts()),
            Phase::Resolution,
        ),
        (Statement::Subtract("vim".into()), Phase::Resolution),
        (Statement::Expr("(Work | dev)".into()), Phase::Resolution),
        (
            Statement::Var {
                name: "role".into(),
                value: "desktop".into(),
            },
            Phase::Resolution,
        ),
        (
            Statement::Param {
                name: "user".into(),
                default: None,
            },
            Phase::Resolution,
        ),
    ];

    for (stmt, want) in cases {
        assert_eq!(
            stmt.phase(),
            want,
            "`{}` runs in {:?}, not {:?}",
            stmt.key(),
            want,
            stmt.phase()
        );
    }
}

/// The question the chain of ors got wrong four times, asked of every kind.
///
/// A config whose only line is a `service:`, a `dotfiles:`, a `firewall:`, a `schedule:` or an
/// `exec:` has work to do even with an empty package plan. A config whose only line is a
/// `repo:` does not — its phase ran *before* the package plan, so by the time this is asked
/// there is nothing left for it, and answering yes would keep a settled machine off the
/// settled path. That distinction used to be an omission; it is a comparison now.
#[test]
fn has_non_package_work_covers_every_phase_after_the_packages() {
    let opts = Options::default;
    let after_packages: Vec<Statement> = vec![
        Statement::Shim("rg".into(), opts()),
        Statement::Service("nginx".into(), opts()),
        Statement::Link("vimrc".into(), opts()),
        Statement::Setting("org.gnome/foo".into(), opts()),
        Statement::Dotfiles("dotfiles".into(), opts()),
        Statement::Firewall("22/tcp".into(), opts()),
        Statement::Schedule("clean".into(), opts()),
        Statement::Exec("setup.sh".into(), opts()),
    ];

    // Every phase after the packages is represented above — otherwise this test would pass by
    // not asking about the kind that was added last, which is the failure it is written for.
    let covered: std::collections::BTreeSet<Phase> =
        after_packages.iter().map(Statement::phase).collect();
    let expected: std::collections::BTreeSet<Phase> = Phase::after_packages().collect();
    assert_eq!(
        covered, expected,
        "this test does not have an example of every post-package phase, so it cannot claim to \
         cover them. Add one for the phase that is missing."
    );

    for stmt in after_packages {
        let state = state_with(vec![stmt.clone()]);
        assert!(
            state.has_non_package_work(),
            "a config whose only line is `{}` has work to do, and the \"nothing to do\" exit \
             must not take it",
            stmt.key()
        );
    }

    // The controls. Without these the assertions above would pass for a function that
    // returns `true` always — which is the other way to get this wrong, and it costs a
    // converged machine the settled path on every run.
    assert!(
        !state_with(vec![]).has_non_package_work(),
        "an empty config has no work after the package plan"
    );
    assert!(
        !state_with(vec![Statement::Repo {
            backend: "apt".into(),
            spec: "ppa:x/y".into(),
        }])
        .has_non_package_work(),
        "`repo:` is phase 1 and has already been applied by the time this is asked"
    );
}

/// A `DesiredState` holding nothing but these statements.
fn state_with(statements: Vec<Statement>) -> linix::model::DesiredState {
    let origin = linix::config::grammar::Origin {
        file: PathBuf::from("modules/test.txt"),
        line: 1,
    };
    linix::model::DesiredState {
        extras: statements
            .into_iter()
            .map(|s| (s, origin.clone()))
            .collect(),
        ..Default::default()
    }
}

/// `dependents()` and the dependent phase are the same list, which they were not.
#[test]
fn the_dependent_accessor_and_the_dependent_phase_agree() {
    let opts = Options::default;
    let state = state_with(vec![
        Statement::Shim("rg".into(), opts()),
        Statement::Service("nginx".into(), opts()),
        Statement::Link("vimrc".into(), opts()),
        Statement::Setting("org.gnome/foo".into(), opts()),
        // And the ones that must NOT be dependents, one per neighbouring phase.
        Statement::Dotfiles("dotfiles".into(), opts()),
        Statement::Firewall("22/tcp".into(), opts()),
        Statement::Schedule("clean".into(), opts()),
        Statement::Exec("setup.sh".into(), opts()),
        Statement::Repo {
            backend: "apt".into(),
            spec: "ppa:x/y".into(),
        },
    ]);

    let dependents: Vec<String> = state.dependents().map(|(s, _)| s.key()).collect();
    assert_eq!(
        dependents,
        vec![
            "shim:rg",
            "service:nginx",
            "link:vimrc",
            "setting:org.gnome/foo"
        ],
        "the dependent phase is exactly shim/service/link/setting — a `dotfiles:` or a \
         `firewall:` applied here would run before the phase that owns it"
    );

    let by_phase: Vec<String> = state
        .in_phase(Phase::Dependents)
        .map(|(s, _)| s.key())
        .collect();
    assert_eq!(
        dependents, by_phase,
        "`dependents()` must be the dependent phase and nothing else, or the two lists can \
         drift the way they already did"
    );
}
