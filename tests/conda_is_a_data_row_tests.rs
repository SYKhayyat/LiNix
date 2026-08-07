//! conda's argv, after it stopped being 319 lines of Rust.
//!
//! `backend_is_data_not_code_tests.rs` exempted `conda.rs` with this reason:
//!
//! > *environment-scoped: every verb carries `-n <env>` resolved at call time from the
//! > declaration and config, so the argv is not fixed at registration.*
//!
//! True, and it was the only thing standing in the way — `conda.rs` overrode `essential()`,
//! `purge`, `tracks_manual`, `Enumerable` and `RepoManager` exactly zero times, so being bespoke
//! bought it nothing and cost it eight capabilities. `ManagerConfig::resolve_settings` and the
//! `{setting.env|base}` placeholder are what the row was missing.
//!
//! **This file is the deleted module's own test, kept.** `conda.rs:267` asserted exactly these
//! three commands through a mock; the conversion is only correct if the same three still come
//! out, so the assertion moved here rather than going with the module. The default and the
//! blank-value cases came with it, because a placeholder with a default is precisely the thing
//! `resolve_env` used to do by hand and the two must not disagree.

use linix::config::Config;
use linix::core::executor::MockExecutor;
use linix::core::{CommandExecutor, PackageSpec};
use std::collections::HashMap;
use std::sync::Arc;

fn config_with_env(env: Option<&str>) -> Config {
    let mut cfg = Config::default();
    if let Some(env) = env {
        let mut conda = HashMap::new();
        conda.insert("env".to_string(), env.to_string());
        cfg.backend_settings.insert("conda".to_string(), conda);
    }
    cfg
}

/// Every conda command this run would issue, in order, for a given `[backend_settings.conda]`.
async fn conda_calls(cfg: Config) -> Vec<String> {
    let vfs = Arc::new(dashmap::DashMap::new());
    let mock = Arc::new(MockExecutor::new(vfs.clone()));
    let exec = CommandExecutor::with_layer(
        false,
        false,
        mock.clone(),
        vfs,
        Arc::new(dashmap::DashMap::new()),
    );
    let hooks = Arc::new(linix::app::LuaHooks::new(&cfg).expect("hooks init"));
    let reg = linix::backends::create_default_registry(exec, &cfg, hooks).await;
    let conda = reg.get("conda").expect("conda registers");

    let mut options = HashMap::new();
    options.insert("version".to_string(), "1.2.3".to_string());
    let inst = conda.as_installable().expect("conda installs");
    inst.install(
        &[PackageSpec {
            name: "numpy".into(),
            backend: "conda".into(),
            options,
            ..Default::default()
        }],
        false,
    )
    .await
    .expect("install");
    inst.remove(
        &["numpy".to_string()],
        false,
        linix::app::sync::guard::Reaped::for_reason(
            linix::app::sync::guard::GuardScope::Remove,
            "a unit test of the effector itself, not of the guard",
        ),
    )
    .await
    .expect("remove");
    conda
        .as_searchable()
        .expect("conda searches")
        .search("numpy")
        .await
        .expect("search");

    mock.allow_unmatched_registrations();
    mock.get_calls().await
}

/// The named environment reaches every verb that is scoped to one — and **not `search`**, which
/// spans the configured channels rather than an environment. The hand-written backend said that
/// in a comment; the row says it by not naming the placeholder in `search_args`.
#[tokio::test]
async fn the_environment_setting_reaches_every_scoped_verb_and_not_search() {
    let calls = conda_calls(config_with_env(Some("ml"))).await;
    assert_eq!(
        calls,
        vec![
            "conda install -n ml -y -- numpy=1.2.3",
            "conda remove -n ml -y -- numpy",
            "conda search --json -- numpy",
        ],
        "the data row does not issue the argv the hand-written backend did"
    );
}

/// `{setting.env|base}` with nothing set. This is `resolve_env`'s default, and the two must not
/// disagree — a machine with no `[backend_settings.conda]` block is the common case, so a wrong
/// answer here is a wrong answer for almost everybody.
#[tokio::test]
async fn no_setting_means_the_base_environment() {
    let calls = conda_calls(config_with_env(None)).await;
    assert_eq!(calls[0], "conda install -n base -y -- numpy=1.2.3");
    assert_eq!(calls[1], "conda remove -n base -y -- numpy");
}

/// A key set to whitespace is a key that was not set. `resolve_env` filtered on
/// `!e.trim().is_empty()`, and the substitution has to keep doing it: `conda list -n  --json`
/// asks conda about a flag rather than an environment, and conda would answer *something*.
#[tokio::test]
async fn a_blank_setting_is_not_an_environment_name() {
    let calls = conda_calls(config_with_env(Some("   "))).await;
    assert_eq!(calls[0], "conda install -n base -y -- numpy=1.2.3");
}

/// The manual set is a different question from the installed set, and on conda it has a
/// different *shape*: `env export --from-history --json` returns a `dependencies` array of
/// match-specs, while `list --json` returns package objects. `ManualFormat::Read` is what lets
/// one row carry both, and this pins that the export command is still the one that runs.
#[tokio::test]
async fn the_manual_set_comes_from_the_history_export() {
    let cfg = config_with_env(Some("ml"));
    let vfs = Arc::new(dashmap::DashMap::new());
    let mock = Arc::new(MockExecutor::new(vfs.clone()));
    let exec = CommandExecutor::with_layer(
        false,
        false,
        mock.clone(),
        vfs,
        Arc::new(dashmap::DashMap::new()),
    );
    let hooks = Arc::new(linix::app::LuaHooks::new(&cfg).expect("hooks init"));
    let reg = linix::backends::create_default_registry(exec, &cfg, hooks).await;
    let conda = reg.get("conda").expect("conda registers");
    let q = conda.as_queryable().expect("conda queries");

    assert!(
        q.tracks_manual(),
        "conda can tell a chosen package from a solved one, and the row must still say so — \
         `tracks_manual() == false` makes `adopt` skip the backend entirely"
    );
    assert!(
        q.manual_source().contains("--from-history"),
        "the manual source is `{}`, which is not the export that answers the question",
        q.manual_source()
    );

    let _ = q.list_manual().await;
    mock.allow_unmatched_registrations();
    let calls = mock.get_calls().await;
    assert!(
        calls
            .iter()
            .any(|c| c.contains("env export -n ml --from-history --json")),
        "the history export did not run: {calls:?}"
    );
}
