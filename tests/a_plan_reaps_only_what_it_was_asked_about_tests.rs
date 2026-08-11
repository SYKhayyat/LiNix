// tests/a_plan_reaps_only_what_it_was_asked_about_tests.rs
//
// **One question, asked of every path that plans a removal.** A plan's removals are computed as
// `managed − desired`, so the answer is only as good as the two sets: hand the planner a
// `desired` narrower than the machine, and every package outside it reads as drift. Eight places
// build a `ChangePlanner`; five of them used to pass `None` for a scope, which meant both "do not
// filter `desired`" and "reap every backend on the box", and four of the five wanted only the
// first.
//
// So the tests here are not four tests of four call sites. They are the property — *a plan
// removes only what its desired set was actually asked about, and only from the managers
// `priority` names* — put to the planner directly, and then to the two callers reachable from an
// integration test. `verbs/` is reachable from the library now (`lib.rs`'s `pub mod verbs;`,
// which `verbs_are_reachable_tests.rs` imports through), so the reason this is a source scan
// is not that the code cannot be called: it is that the claim is about EVERY call site, and
// calling one proves nothing about the rest. `plan`/`apply` and
// `upgrade --canary` are covered by `planner_scope_enumeration_tests` instead, which reads their
// source.
//
// Every assertion has a control beside it that removes the fix and watches the count come back.
// A "no removals were planned" test passes just as well when nothing was planned at all.

use crate::mock_providers::TestKernel;

use shall::app::sync::planner::{ChangePlanner, HostBackends, PlanScope, Scope};
use shall::app::sync::resolver::StateResolver;
use shall::core::PackageSpec;
use std::collections::HashMap;

fn spec(backend: &str, name: &str) -> PackageSpec {
    PackageSpec {
        name: name.into(),
        backend: backend.into(),
        options: Default::default(),
        requires: vec![],
        present: true,
    }
}

fn desired_of(specs: &[PackageSpec]) -> HashMap<String, Vec<PackageSpec>> {
    let mut out: HashMap<String, Vec<PackageSpec>> = HashMap::new();
    for s in specs {
        out.entry(s.backend.clone()).or_default().push(s.clone());
    }
    out
}

/// Put four packages in the registry as Shall-managed, across three managers.
async fn machine_holding_four(kernel: &TestKernel) {
    let mut state = kernel.app.state.lock().await;
    for (backend, name) in [
        ("cargo", "ripgrep"),
        ("cargo", "bat"),
        ("brew", "jq"),
        ("npm", "typescript"),
    ] {
        state.add(backend, name, None, Default::default(), "test", false);
    }
}

/// The transient shell's bug, at the layer it happened: a desired set holding only what the
/// shell was asked for, planned as though it were the machine's whole config.
///
/// `shall shell ripgrep` builds `desired = {cargo: [ripgrep]}` — it is not the config and was
/// never meant to be compared against the machine. Under the old `None` scope the other three
/// managed packages had no declaration in that map, so all three were scheduled for removal, and
/// `max_removals` was the only thing between the plan and the machine.
#[tokio::test]
async fn a_plan_over_an_explicit_package_set_removes_nothing() {
    let kernel = TestKernel::new().await;
    machine_holding_four(&kernel).await;

    let desired = desired_of(&[spec("cargo", "ripgrep")]);

    let just_these = {
        let state = kernel.app.state.lock().await;
        ChangePlanner::new(kernel.app.registry.clone(), &state, &kernel.app.config)
            .plan(&desired, PlanScope::JustThese)
            .await
            .unwrap()
    };
    assert_eq!(
        just_these.total_remove(),
        0,
        "a plan over a list that is not the config must never remove: the packages missing from \
         it are missing from the question, not from the machine's declarations"
    );

    // The control. Same registry, same desired set, the scope the shell used to pass — if this
    // does not schedule removals then the assertion above is passing on an empty machine and
    // proves nothing.
    let as_a_converge = {
        let state = kernel.app.state.lock().await;
        ChangePlanner::new(kernel.app.registry.clone(), &state, &kernel.app.config)
            .plan(&desired, PlanScope::Whole(HostBackends::default()))
            .await
            .unwrap()
    };
    assert_eq!(
        as_a_converge.total_remove(),
        3,
        "control: read as a whole-machine converge, the same input reaps the other three — \
         which is exactly what `shall shell` was doing"
    );
}

/// A converge reaps only the managers `priority` names (II.6).
///
/// "Listed means Shall uses it. Not listed means Shall does not touch it at all" is the sentence
/// a new user reads when `priority` is missing. Four commands broke it by planning removals with
/// no list at all.
#[tokio::test]
async fn a_converge_reaps_only_the_backends_priority_names() {
    let kernel = TestKernel::new().await;
    machine_holding_four(&kernel).await;

    // Nothing is declared, so every managed package is drift — the maximal case, on purpose.
    let nothing = HashMap::new();

    let only_cargo = HostBackends::from_priority(vec!["cargo".into()]);
    let scoped = {
        let state = kernel.app.state.lock().await;
        ChangePlanner::new(kernel.app.registry.clone(), &state, &kernel.app.config)
            .plan(&nothing, PlanScope::Whole(only_cargo))
            .await
            .unwrap()
    };
    assert_eq!(
        scoped.total_remove(),
        2,
        "only the two `cargo` packages may go; `brew:jq` and `npm:typescript` are on managers this \
         host does not list"
    );
    // …and the ones it declined are reported, not dropped in silence (AU1).
    assert_eq!(
        scoped.skipped.len(),
        2,
        "a package left alone because its backend is not in `priority` is a decision the user \
         hears about"
    );

    // The control: without the list, all four go.
    let unscoped = {
        let state = kernel.app.state.lock().await;
        ChangePlanner::new(kernel.app.registry.clone(), &state, &kernel.app.config)
            .plan(&nothing, PlanScope::Whole(HostBackends::default()))
            .await
            .unwrap()
    };
    assert_eq!(
        unscoped.total_remove(),
        4,
        "control: an empty host list still means every backend, so this test is measuring the \
         list and not something else"
    );
}

/// A narrowed plan removes nothing — the rule `upgrade --module` has always relied on, restated
/// here because `PlanScope::Narrowed` is now what carries it.
#[tokio::test]
async fn a_narrowed_plan_removes_nothing() {
    let kernel = TestKernel::new().await;
    machine_holding_four(&kernel).await;

    let desired = desired_of(&[spec("cargo", "ripgrep")]);
    let narrowed = {
        let state = kernel.app.state.lock().await;
        ChangePlanner::new(kernel.app.registry.clone(), &state, &kernel.app.config)
            .plan(&desired, PlanScope::Narrowed(Scope::Module("dev".into())))
            .await
            .unwrap()
    };
    assert_eq!(
        narrowed.total_remove(),
        0,
        "a plan narrowed to one module must not reap the packages outside it"
    );
}

/// End to end, through the caller: `provision_transient_env` leaves the machine's packages alone.
///
/// The planner test above proves `JustThese` is safe; this proves the shell passes it. Both are
/// needed — the bug was never in the planner, it was in what the caller said.
#[tokio::test]
async fn the_transient_shell_plans_no_removal_of_the_machines_packages() {
    let kernel = TestKernel::new().await;
    machine_holding_four(&kernel).await;

    kernel
        .app
        .shell()
        .provision_transient_env(&["cargo:fd".into()], "shell-test")
        .await
        .expect("provisioning a transient shell must succeed");

    // Nothing the machine held was touched. Read from the registry rather than from the plan,
    // because the question is about the machine and a plan is only a claim about it.
    let state = kernel.app.state.lock().await;
    for (backend, name) in [
        ("cargo", "ripgrep"),
        ("cargo", "bat"),
        ("brew", "jq"),
        ("npm", "typescript"),
    ] {
        assert!(
            state.is_managed(backend, name),
            "`shall shell` must not remove `{}:{}` — it was never asked about it",
            backend,
            name
        );
    }
}

/// `activate` confines removals the way `sync` does.
///
/// The narrower-sounding command was the more destructive one: `sync` consulted `priority` and
/// `activate` did not, so activating a profile could reap a manager the host had never listed.
#[tokio::test]
async fn activating_a_profile_reaps_only_the_backends_priority_names() {
    let kernel = TestKernel::new().await;
    let root = kernel.app.config.config_root();

    // `priority` lists cargo alone: brew and npm are managers this host does not use.
    tokio::fs::write(root.join("priority"), "cargo\n")
        .await
        .unwrap();
    tokio::fs::write(root.join("profiles/Work"), "cargo:ripgrep\n")
        .await
        .unwrap();
    tokio::fs::write(root.join("active"), "\n").await.unwrap();

    machine_holding_four(&kernel).await;

    // The resolver's answer is the list `activate` must plan against. Asserted here rather than
    // trusted, so a `priority` this fixture failed to write cannot make the test vacuous.
    let resolver = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), false).await;
    assert_eq!(
        resolver.host_backends().await,
        HostBackends::from_priority(vec!["cargo".into()]),
        "the fixture's `priority` must name exactly one manager for this test to mean anything"
    );

    kernel
        .app
        .profile_manager()
        .activate(&["Work".into()], false)
        .await
        .expect("activating a profile must succeed");

    let state = kernel.app.state.lock().await;
    for (backend, name) in [("brew", "jq"), ("npm", "typescript")] {
        assert!(
            state.is_managed(backend, name),
            "`activate` must not reap `{}:{}`: `priority` does not name that manager",
            backend,
            name
        );
    }
}
