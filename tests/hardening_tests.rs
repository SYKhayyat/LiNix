// tests/hardening_tests.rs
//
// Integration coverage for behavior added/changed during the v5 hardening pass:
//  - resolver merges multiple sources for one package (was first-write-wins)
//  - scoped `upgrade` is non-destructive end-to-end (resolver -> planner)
//  - RepoManager dispatch issues the right backend command
//
// All hermetic via the shared TestKernel (MockExecutor + temp sandbox); OS-independent,
// so these also exercise the relevant paths on Linux at CI time.

use crate::mock_providers::TestKernel;

use shall::app::sync::planner::{ChangePlanner, HostBackends, PlanScope, Scope};
use shall::app::sync::resolver::StateResolver;
use shall::core::executor::DryRunOutput;
use shall::core::PackageSpec;

/// Build a PackageSpec with a pinned version option.
fn pinned_spec(backend: &str, name: &str, version: &str) -> PackageSpec {
    let mut options = shall::config::grammar::Options::default();
    options.set("version", version.to_string());
    PackageSpec {
        name: name.into(),
        backend: backend.into(),
        options,
        requires: vec![],
        present: true,
    }
}

/// A package a profile declares directly AND a module it uses also declares must end up
/// belonging to BOTH scopes, so it stays visible to each one.
///
/// Declaring the same thing twice is not a disagreement, so the two declarations merge —
/// but the loser's scope is not thereby untrue. Keep only the winner's and `upgrade
/// --module dev` stops finding a package module `dev` really does declare.
#[tokio::test]
async fn resolver_records_every_scope_a_package_belongs_to() {
    let kernel = TestKernel::new().await;
    let root = kernel.app.config.config_root();

    // The profile holds the package directly and also uses a module holding the same one.
    tokio::fs::write(root.join("profiles/Work"), "cargo:ripgrep\nuse dev\n")
        .await
        .unwrap();
    tokio::fs::write(root.join("modules/dev.txt"), "cargo:ripgrep\n")
        .await
        .unwrap();
    tokio::fs::write(root.join("active"), "Work\n")
        .await
        .unwrap();

    let resolver = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), false).await;
    let desired = resolver.resolve_desired_state().await.unwrap();

    let spec = desired
        .get("cargo")
        .and_then(|specs| specs.iter().find(|s| s.name == "ripgrep"))
        .expect("ripgrep should be resolved under cargo");
    assert_eq!(
        desired.get("cargo").map(|s| s.len()),
        Some(1),
        "one package, declared twice — not two packages"
    );

    // A list, since `LX-10`. It was one `;`-joined string that this test split back apart.
    let scopes = spec.options.all("__scopes");
    assert!(!scopes.is_empty(), "__scopes should be tagged");
    assert!(
        scopes.iter().any(|s| s == "profile:Work"),
        "missing profile scope in {scopes:?}"
    );
    assert!(
        scopes.iter().any(|s| s == "module:dev"),
        "missing module scope in {scopes:?}"
    );
}

/// End-to-end: a targeted `upgrade --module dev` must never schedule removals for managed
/// packages outside that scope — while an UNSCOPED plan would remove the same drift.
#[tokio::test]
async fn scoped_upgrade_is_non_destructive_end_to_end() {
    let kernel = TestKernel::new().await;
    let root = kernel.app.config.config_root();
    tokio::fs::write(root.join("profiles/Work"), "use dev\n")
        .await
        .unwrap();
    tokio::fs::write(root.join("modules/dev.txt"), "cargo:ripgrep\n")
        .await
        .unwrap();
    tokio::fs::write(root.join("active"), "Work\n")
        .await
        .unwrap();

    // A managed package that no active module declares == drift.
    {
        let mut state = kernel.app.state.lock().await;
        state.add(
            "cargo",
            "out-of-scope-pkg",
            None,
            Default::default(),
            "manifest:other",
            false,
        );
    }

    let resolver = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), false).await;
    let desired = resolver.resolve_desired_state().await.unwrap();

    // Scoped plan: zero removals.
    let scoped = {
        let state = kernel.app.state.lock().await;
        let planner = ChangePlanner::new(kernel.app.registry.clone(), &state, &kernel.app.config);
        planner
            .plan(&desired, PlanScope::Narrowed(Scope::Module("dev".into())))
            .await
            .unwrap()
    };
    assert_eq!(
        scoped.total_remove(),
        0,
        "scoped upgrade must not remove out-of-scope packages"
    );

    // Unscoped plan: the same drift IS scheduled for removal (proves the guard is what
    // prevents it, not e.g. protection).
    let unscoped = {
        let state = kernel.app.state.lock().await;
        let planner = ChangePlanner::new(kernel.app.registry.clone(), &state, &kernel.app.config);
        planner
            .plan(&desired, PlanScope::Whole(HostBackends::default()))
            .await
            .unwrap()
    };
    assert!(
        unscoped.total_remove() >= 1,
        "unscoped sync should remove the drift package"
    );
}

/// II.7 phase 3: a config whose only work is dependent extras (a `service:`, a `link:`, a
/// `shim:`) still resolves them, and `apply` walks them without error. The
/// dry-run kernel makes this a preview: it touches nothing, so it also proves the dependent
/// phase does not reach for the real filesystem or `~/.local/bin` when only previewing.
#[tokio::test]
async fn dependents_only_config_resolves_and_applies_the_dependent_phase() {
    let kernel = TestKernel::new().await;
    let root = kernel.app.config.config_root();
    tokio::fs::write(root.join("profiles/Work"), "use svc\n")
        .await
        .unwrap();
    // A module with no package line at all — only dependents. `apt:nginx` is omitted on
    // purpose so the "package plan is empty but there is still work" path is what runs.
    tokio::fs::write(
        root.join("modules/svc.txt"),
        "service:nginx@enabled=true\n\
         link:~/.config/app.conf@target=~/.config/app.conf,content=hello\n\
         shim:rg\n",
    )
    .await
    .unwrap();
    tokio::fs::write(root.join("active"), "Work\n")
        .await
        .unwrap();

    let resolver = StateResolver::new(&kernel.app.config, kernel.app.registry.clone(), false).await;
    let state = resolver.resolve_model().await.unwrap();

    assert!(
        state.packages.values().all(|v| v.is_empty()),
        "the fixture declares no packages"
    );
    assert!(
        state.has_dependents(),
        "the three extras are the dependent phase's work"
    );
    assert_eq!(state.dependents().count(), 3);

    // Previews cleanly (dry-run kernel): resolution -> dependent dispatch, no panic, no touch.
    kernel.app.dependents().apply(&state).await.unwrap();
}

/// A backend's RepoManager must issue the backend's real "add source" command.
#[tokio::test]
async fn repo_manager_dispatches_add_command() {
    let kernel = TestKernel::new().await;
    kernel.mock_executor.set_command_exists("gem", true);

    let gem = kernel
        .app
        .registry
        .get("gem")
        .expect("gem should be registered");
    let repo = gem
        .as_repo_manager()
        .expect("gem should support RepoManager");
    repo.add_repo("myrepo", "https://gems.example.com/", false)
        .await
        .unwrap();

    let calls = kernel.mock_executor.get_calls().await;
    assert!(
        calls
            .iter()
            .any(|c| c.contains("sources -a https://gems.example.com/")),
        "expected a `gem sources -a <url>` call, got: {:?}",
        calls
    );
}

/// `unmanaged` reports installed packages not under Shall management (and excludes
/// managed ones).
#[tokio::test]
async fn installed_but_undeclared_lists_the_dependency_closure_too() {
    let kernel = TestKernel::new().await;
    // cargo reports two installed crates...
    kernel.mock_executor.set_response(
        "cargo install --list",
        Ok(DryRunOutput {
            stdout: b"ripgrep v13.0.0:\n    rg\nexa v0.10.1:\n    exa\n".to_vec(),
            stderr: vec![],
        }
        .into()),
    );
    // ...but only ripgrep is under management.
    {
        let mut state = kernel.app.state.lock().await;
        state.add(
            "cargo",
            "ripgrep",
            None,
            Default::default(),
            "manifest:base",
            false,
        );
    }

    let unmanaged = kernel.app.installed_but_undeclared().await.unwrap();
    assert!(
        unmanaged
            .packages
            .iter()
            .any(|p| p.backend == "cargo" && p.name == "exa"),
        "exa should be reported as unmanaged, got: {:?}",
        unmanaged
            .packages
            .iter()
            .map(|p| (&p.backend, &p.name))
            .collect::<Vec<_>>()
    );
    assert!(
        !unmanaged.packages.iter().any(|p| p.name == "ripgrep"),
        "ripgrep is managed and must not be listed as unmanaged"
    );
    // **The other half of the same answer, and this fixture demonstrates why it had to become
    // one.** Four real backends on the host — conda, dotnet, pixi, scoop — are reached by this
    // crawl and answer with output their parsers refuse. They contribute no packages, which is
    // correct and safe for `purge-undeclared`; the empty vector they contributed was also, until
    // now, indistinguishable from "nothing here is unmanaged", and `check drift` printed a clean
    // bill over it (B4). Not asserted as non-empty — which managers are installed is a fact
    // about the host — but every entry must name the backend it is about, or the report cannot
    // be acted on.
    for who in &unmanaged.unanswered {
        assert!(
            who.contains(':'),
            "an unanswered manager must be named: {who}"
        );
    }
}

/// Reproducible installs: a pinned version reaches the backend command in its native
/// syntax — inline (`pip install requests==2.31.0`) for generic backends, and a flag
/// (`cargo install ripgrep --version 13.0.0`) for cargo.
#[tokio::test]
async fn pinned_version_reaches_install_command() {
    let kernel = TestKernel::new().await;

    // generic inline pin (pip == syntax)
    let pip = kernel.app.registry.get("pip").expect("pip registered");
    pip.as_installable()
        .unwrap()
        .install(&[pinned_spec("pip", "requests", "2.31.0")], false)
        .await
        .unwrap();

    // bespoke flag pin (cargo --version)
    let cargo = kernel.app.registry.get("cargo").expect("cargo registered");
    cargo
        .as_installable()
        .unwrap()
        .install(&[pinned_spec("cargo", "ripgrep", "13.0.0")], false)
        .await
        .unwrap();

    let calls = kernel.mock_executor.get_calls().await;
    assert!(
        calls
            .iter()
            .any(|c| c.contains("install -- requests==2.31.0")),
        "pip should pin with ==, got: {:?}",
        calls
    );
    assert!(
        calls
            .iter()
            .any(|c| c.contains("install --version 13.0.0 -- ripgrep")),
        "cargo should pin with --version, ahead of the terminator, got: {:?}",
        calls
    );
}

/// A floating version ("latest") must NOT be pinned — it installs the bare name.
#[tokio::test]
async fn floating_version_is_not_pinned() {
    let kernel = TestKernel::new().await;
    let pip = kernel.app.registry.get("pip").unwrap();
    pip.as_installable()
        .unwrap()
        .install(&[pinned_spec("pip", "requests", "latest")], false)
        .await
        .unwrap();
    let calls = kernel.mock_executor.get_calls().await;
    assert!(
        calls
            .iter()
            .any(|c| c.contains("install -- requests") && !c.contains("==")),
        "latest should install bare name, got: {:?}",
        calls
    );
}

// ---------------------------------------------------------------------------
// A write that the model will reject must never reach a file.
//
// `install` writes the line first and syncs second on purpose (S15), so a spec naming
// a backend `priority` does not list used to land in `modules/imperative.txt` and only
// then fail — and from that moment every command that parses the model was a hard
// error, until a human edited the file. Found by the container harness: one
// `install dnf:jq` on an image without dnf wedged `status`, `why`, `upgrade`,
// `conflicts`, `activate` and every later install for the rest of the run.
//
// The family is every writer that goes through `App::declare` (install, `absent:@until`,
// `service enable`, hook-record, `init --interactive`'s starter packages) plus
// `retarget` (`teleport`), which rewrites a line to a backend the same way.
// ---------------------------------------------------------------------------

/// The fixture's `priority` is apt/brew/cargo, so `npm` is a backend Shall does not use.
#[tokio::test]
async fn declaring_an_unlisted_backend_writes_nothing() {
    let kernel = TestKernel::new().await;
    let root = kernel.app.config.config_root();

    let err = kernel
        .app
        .declare("npm:cowsay", None, shall::model::Landing::Imperative)
        .await
        .expect_err("a backend not in `priority` must be refused");
    assert!(
        err.to_string().contains("priority"),
        "the refusal must name the rule, got: {err}"
    );

    let imperative = root.join("modules/imperative.txt");
    assert!(
        !imperative.exists()
            || !tokio::fs::read_to_string(&imperative)
                .await
                .unwrap()
                .contains("npm"),
        "the refused line reached the manifest anyway"
    );
}

/// Every landing writes through the same door, so none of them can poison the model —
/// this is the sibling check the reported symptom (an `install`) does not cover.
#[tokio::test]
async fn no_landing_can_write_an_unlisted_backend() {
    let kernel = TestKernel::new().await;
    let root = kernel.app.config.config_root();

    for (line, landing) in [
        ("npm:cowsay", shall::model::Landing::Imperative),
        ("absent:npm:cowsay", shall::model::Landing::Imperative),
        ("npm:cowsay", shall::model::Landing::Hooks),
        ("npm:cowsay", shall::model::Landing::Adopted),
    ] {
        assert!(
            kernel.app.declare(line, None, landing).await.is_err(),
            "`{line}` was written despite naming a backend `priority` does not list"
        );
    }

    let mut wrote_npm = false;
    let mut dir = tokio::fs::read_dir(root.join("modules")).await.unwrap();
    while let Ok(Some(entry)) = dir.next_entry().await {
        if tokio::fs::read_to_string(entry.path())
            .await
            .unwrap_or_default()
            .contains("npm")
        {
            wrote_npm = true;
        }
    }
    assert!(!wrote_npm, "a refused declaration still landed in modules/");
}

/// A backend that IS listed still goes through — the check refuses the unusable line,
/// not every line. Without this the fix could be "refuse everything" and still pass.
#[tokio::test]
async fn declaring_a_listed_backend_still_writes() {
    let kernel = TestKernel::new().await;
    let root = kernel.app.config.config_root();

    kernel
        .app
        .declare("cargo:ripgrep", None, shall::model::Landing::Imperative)
        .await
        .expect("cargo is in `priority`, so the line belongs in a file");

    let text = tokio::fs::read_to_string(root.join("modules/imperative.txt"))
        .await
        .expect("the manifest was not written");
    assert!(text.contains("cargo:ripgrep"), "got: {text}");
}

/// A bare name carries no backend, so nothing static can reject it — and it must not be
/// probed here either. The pre-write check is the priority rule, not a resolution.
#[tokio::test]
async fn a_bare_name_is_not_rejected_before_the_write() {
    let kernel = TestKernel::new().await;
    kernel
        .app
        .declare("ripgrep", None, shall::model::Landing::Imperative)
        .await
        .expect("a bare name has no backend to refuse");
}

/// `teleport` is the second writer: it rewrites a declared line to another manager, and
/// a move to an unlisted one would leave the file naming a backend nothing parses — with
/// the original already gone.
#[tokio::test]
async fn teleport_to_an_unlisted_backend_leaves_the_line_alone() {
    let kernel = TestKernel::new().await;
    let root = kernel.app.config.config_root();
    tokio::fs::write(root.join("modules/dev.txt"), "cargo:ripgrep\n")
        .await
        .unwrap();
    tokio::fs::write(root.join("profiles/Main"), "use dev\n")
        .await
        .unwrap();

    kernel
        .app
        .retarget("ripgrep", "npm")
        .await
        .expect_err("npm is not in `priority`, so there is nowhere to move it to");

    let text = tokio::fs::read_to_string(root.join("modules/dev.txt"))
        .await
        .unwrap();
    assert!(
        text.contains("cargo:ripgrep") && !text.contains("npm"),
        "the line was rewritten to an unusable backend, got: {text}"
    );
}
