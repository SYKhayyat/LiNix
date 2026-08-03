//! What a command costs before it has done anything.
//!
//! `linix path` measured **272 ms** on a release build against a 61 ms process-spawn baseline,
//! and `--timings` said `no child commands — this run asked no package manager anything`. All of
//! it was fixed overhead: `create_default_registry` runs for every subcommand, and one backend
//! built a rate limiter in its constructor whose clock performs a 200 ms TSC calibration on
//! first construction (AU3).
//!
//! The reason it survived is the one `AU3` states plainly: **nothing measured the part of a run
//! that spawns no child.** `latency.rs` budgets a whole command at seconds, which a fifth of a
//! second of start-up never crosses, and every per-backend timing instrument measures child
//! processes — of which this has none.
//!
//! So this file budgets the two halves separately:
//!
//! * a wall-clock ceiling on building the registry, which is the class — *any* eagerly
//!   constructed expensive object lands here, whatever the cause;
//! * the invariant behind the specific one, asserted deterministically: a rate limiter costs
//!   nothing until something is rate limited.

use linix::backends::create_default_registry;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Generous by an order of magnitude against the ~20 ms a lazy debug build measures, because a
/// loaded runner is not a defect and a gate that reddens on load is one people learn to ignore
/// (`latency.rs` says the same about its own numbers). It is still a tenth of what one eager
/// TSC calibration costs, which is the collapse this exists to catch.
const REGISTRY_BUDGET: Duration = Duration::from_millis(120);

#[tokio::test]
async fn building_the_registry_asks_nothing_and_therefore_costs_almost_nothing() {
    let vfs = Arc::new(dashmap::DashMap::new());
    let mock = Arc::new(linix::core::executor::MockExecutor::new(vfs.clone()));
    let exec = linix::core::CommandExecutor::with_layer(
        true,
        false,
        mock,
        vfs,
        Arc::new(dashmap::DashMap::new()),
    );
    let config = linix::config::Config::default();
    let hooks = Arc::new(linix::app::hooks::LuaHooks::new(&config).expect("hooks"));

    // The FIRST construction in this process is the one that matters: a one-off global
    // calibration is free on every subsequent call, so a warm-up here would measure the fix
    // into existence.
    let started = Instant::now();
    let registry = create_default_registry(exec, &config, hooks).await;
    let elapsed = started.elapsed();

    assert!(
        !registry.all().is_empty(),
        "the registry registered nothing, so the measurement below is of nothing"
    );
    assert!(
        elapsed <= REGISTRY_BUDGET,
        "registering {} backends took {:.1?}, over the {:.1?} budget.\n\
         Registration runs for every subcommand and asks no manager anything, so this is fixed \
         overhead on `linix path` as much as on `linix sync`. Something is being CONSTRUCTED \
         eagerly that is only USED conditionally — build it where it is used, as `web.rs` and \
         `appimage.rs` build their HTTP clients.",
        registry.all().len(),
        elapsed,
        REGISTRY_BUDGET,
    );
}

#[tokio::test]
async fn a_rate_limiter_costs_nothing_until_something_is_rate_limited() {
    // Every constructor on the type, not just the one that was found eager: the defect was a
    // backend calling `RateLimiter::github()` in `new`, and the sibling call site
    // (`vscode.rs`) does exactly the same thing.
    for limiter in [
        linix::core::RateLimiter::github(),
        linix::core::RateLimiter::github_authenticated(),
        linix::core::RateLimiter::vscode_marketplace(),
        linix::core::RateLimiter::new(10, "test"),
    ] {
        assert!(
            !limiter.is_engaged(),
            "`{}` built its limiter in the constructor. Nothing has been rate limited yet, so \
             nothing should have been built — a backend's `new` runs for every subcommand.",
            limiter.description()
        );
        limiter.wait().await.expect("first permit is immediate");
        assert!(
            limiter.is_engaged(),
            "`{}` waited for a permit without a limiter to issue one",
            limiter.description()
        );
    }
}

/// The clones share one limiter, which is the whole point of rate limiting: two backends holding
/// copies of the same quota must not get one quota each. Laziness is easy to add in a way that
/// quietly breaks this — a per-clone cell would.
#[tokio::test]
async fn clones_share_the_quota_they_were_cloned_from() {
    let original = linix::core::RateLimiter::new(5, "shared");
    let clone = original.clone();

    clone.wait().await.expect("first permit is immediate");

    assert!(
        original.is_engaged(),
        "the clone built its own limiter, so the two hold separate quotas and the limit is \
         twice what it says it is"
    );
}
