//! GRADER round 3, 2026-07-29 — the flag half of the argv-drift gate cannot fail on the flag
//! it was built for.
//!
//! G-8 was: `VERIFIES_ITSELF = [("helm", "--verify=false")]` was derived from one machine's
//! helm 4 error message and emitted unconditionally, and helm 3 answers `unknown flag:
//! --verify`. The gate that exists to catch an argv upstream will not accept could not see it,
//! because it only examined subcommands. The round-2 fix added a flag half to
//! `tests/argv_drift_tests.rs` and a runtime probe (`tool_help::accepts_flag`) that withholds
//! the flag when the tool's help does not document it.
//!
//! The probe defeats the gate. Measured on this Windows host, where `linix check health` reports
//! `[READY] helm` and `helm version --short` is `v4.2.3`:
//!
//!     # table as shipped
//!     $ DRIFT_DUMP=1 cargo test --test argv_drift_tests -- --include-ignored
//!     CALL: helm plugin install --verify=false -- https://example.invalid/linix-drift-probe
//!     test result: ok. 1 passed
//!
//!     # one character of drift planted in the capability table
//!     $ sed -i 's/--verify=false/--linix-bogus-flag-zzz/' src/backends/artifact/capability.rs
//!     $ DRIFT_DUMP=1 cargo test --test argv_drift_tests -- --include-ignored
//!     CALL: helm plugin install -- https://example.invalid/linix-drift-probe
//!     test result: ok. 1 passed
//!
//! The bogus flag never reaches an argv, so the gate — which reads argvs — has nothing to
//! check and reports success. **The gate is green whether the table is right or wrong**, which
//! is this repo's recurring defect in the check that was built to end it: `accepts_flag`
//! returns `Some(false)`, LiNix silently drops the flag, and no one is told.
//!
//! The user-visible half is not hypothetical. When helm renames the flag, LiNix withholds it,
//! helm refuses the unsignable source, and `explain_verification` tells the user to *"Add
//! `@unverified` to the line"* — which is the line they already wrote. Silent drift becomes an
//! error message that asks for what is already there.
//!
//! This file is the assertion the gate is missing: **a flag the capability table names for a
//! backend that is installed here must survive into the argv.** It passes on the table as
//! shipped and goes red on the mutation above — the mirror image of the gate, which passes on
//! both. Verified in both directions before committing.

use std::sync::Arc;

/// Every `(backend, flag)` the capability table would contribute, for backends available here.
///
/// Read from the argv LiNix actually builds, never from the table alone: the point of the
/// finding is that the two can disagree, and a test that reads the table would agree with
/// itself.
#[tokio::test]
async fn a_capability_flag_survives_into_the_argv_or_is_named() {
    use dashmap::DashMap;
    use linix::core::executor::MockExecutor;
    use linix::core::{CommandExecutor, PackageSpec};

    let vfs = Arc::new(DashMap::new());
    let mock = Arc::new(MockExecutor::new(vfs.clone()));
    let exec =
        CommandExecutor::with_layer(true, false, mock.clone(), vfs, Arc::new(DashMap::new()));
    let config = linix::config::Config::default();
    let registry = linix::backends::create_default_registry(
        exec,
        &config,
        Arc::new(linix::app::hooks::LuaHooks::new(&config).expect("hooks")),
    )
    .await;

    let mut checked = 0usize;
    let mut missing: Vec<String> = Vec::new();

    for backend in registry.available() {
        let name = backend.name().to_string();
        let Some(flag) = linix::backends::artifact::capability::unverified_arg(&name) else {
            continue;
        };
        let Some(installable) = backend.as_installable() else {
            continue;
        };

        let mut spec = PackageSpec {
            name: "jq".into(),
            backend: name.clone(),
            ..Default::default()
        };
        spec.options
            .insert("unverified".to_string(), "true".to_string());
        // The install source, where the backend demands one — helm's `plugin install` takes a
        // URL, and without it the call fails before it builds an argv at all.
        if let Some(key) = linix::backends::artifact::capability::install_source_key(&name) {
            spec.options.insert(
                key.to_string(),
                "https://example.invalid/linix-drift-probe".to_string(),
            );
        }

        let before = mock.get_calls().await.len();
        let _ = installable
            .install(std::slice::from_ref(&spec), false)
            .await;
        let calls = mock.get_calls().await;
        let produced: Vec<String> = calls.into_iter().skip(before).collect();

        assert!(
            !produced.is_empty(),
            "`{name}` produced no argv for an `@unverified` install, so this check would pass \
             by testing nothing — the same way the drift gate did before the install source was \
             threaded through it."
        );

        checked += 1;
        if !produced.iter().any(|c| c.contains(flag)) {
            missing.push(format!(
                "`{name}`: the capability table says `@unverified` adds `{flag}`, and the argv \
                 LiNix built carries no such flag:\n      {}",
                produced.join("\n      ")
            ));
        }
    }

    assert!(
        checked > 0,
        "no backend in `VERIFIES_ITSELF` is installed on this machine, so this gate examined \
         nothing. That is a named skip, not a pass — run it on a host with helm."
    );

    assert!(
        missing.is_empty(),
        "a flag the capability table promises was dropped before it reached the command:\n  {}\n\n\
         `tool_help::accepts_flag` withholds a flag the tool's help does not document. That is \
         a reasonable thing to do and a terrible thing to do silently: the withheld flag is \
         exactly the drifted one, so the argv-drift gate — which inspects argvs — is \
         structurally unable to see the case it was built for. Drift has to be named, not \
         absorbed.",
        missing.join("\n  ")
    );
}
