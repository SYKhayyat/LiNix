//! **The managers that come with a language runtime: pip, gem, bun, dotnet, conda.**
//!
//! Hand-written rather than rows in `builtin_backends.toml` because each needs something the
//! table cannot say — a machine-readable listing to try first, an export format, a config-driven
//! root. The ones that fit the table are in the table, and `a_backend_is_a_row_tests` is what
//! keeps that boundary from drifting into "whatever was easiest that day".

// src/backends/registry.rs

use crate::backends::generic::{CacheClean, RepoListing};
use crate::backends::generic::{
    GenericBackendCore, GenericInstallable, GenericQueryable, GenericRepoManager,
    GenericSearchable, GenericUpgradable, ManagerConfig, ManualListing, SearchSource, VersionPin,
};
use crate::backends::generic::{MachineListing, ManualFormat, OutdatedProbe};
use crate::config::Config;
use crate::core::{BackendCapabilities, CommandExecutor};
use crate::parsers::LambdaParser;
use std::sync::Arc;

use super::{base_config, register_generic, with_manager_policy, BackendRegistry};

pub(super) fn register_pip(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    // Generic for install/list; Searchable is a bespoke PyPI JSON lookup
    // (pip's own `search` was disabled upstream).
    let core = Arc::new(GenericBackendCore {
        name: "pip".into(),
        executor: executor.clone(),
        config: ManagerConfig {
            name: "pip".into(),
            binary: None,
            remove_binary: None,
            version_pin: Some(VersionPin::Inline("{name}=={version}".into())),
            install_args: vec!["install".into()],
            remove_args: vec!["uninstall".into(), "-y".into()],
            purge_args: None,
            list_args: vec!["list".into(), "--format=json".into()],
            // `pip list` includes every pulled-in dependency and pip keeps no record of
            // which distributions a person actually asked for. (`--not-required` reports
            // leaves, which is a different question: a leaf may still be a dependency.)
            manual: ManualListing::Unsupported,
            essential_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec!["install".into(), "--upgrade".into()],
            update_args: None,
            orphan_dry_run: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            repo_binary: None,
            repo_list_binary: None,
            repo_remove_binary: None,
            repo_list_shape: RepoListing::Columns,
            depends: None,
            // `pip cache purge` — the wheel cache under `~/.cache/pip`. pip 20.1 and later.
            clean_cache: Some(CacheClean {
                binary: None,
                args: vec!["cache".into(), "purge".into()],
            }),
            needs_root: false,
            is_exclusive: false,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: Some(OutdatedProbe {
                binary: None,
                args: vec!["list".into(), "--outdated".into(), "--format=json".into()],
                parse: std::sync::Arc::new(crate::parsers::language::parse_pip_outdated),
                silence_is_none: false,
            }),
            // PyPI over HTTP, because `pip search` is gone. Declared here rather than bolted
            // on as a bespoke `Searchable`, so it is the same mechanism `npm_registry` uses
            // and a row can ask for it.
            search_source: SearchSource::PyPi,
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::language::parse_installed("pip", o),
            search_fn: |_| vec![],
        }),
    });
    let core = with_manager_policy(core);
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
            .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

pub(super) fn register_gem(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "gem".into(),
        executor: executor.clone(),
        config: ManagerConfig {
            name: "gem".into(),
            binary: None,
            remove_binary: None,
            version_pin: Some(VersionPin::after(vec!["-v".into(), "{version}".into()])),
            install_args: vec!["install".into()],
            remove_args: vec!["uninstall".into()],
            purge_args: None,
            list_args: vec!["list".into(), "--local".into()],
            // `gem list --local` mixes user-installed gems with their dependencies, and
            // RubyGems records no explicit-install marker.
            manual: ManualListing::Unsupported,
            essential_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec!["update".into()],
            update_args: None,
            orphan_dry_run: None,
            repo_add_args: Some(vec!["sources".into(), "-a".into(), "{url}".into()]),
            repo_remove_args: Some(vec!["sources".into(), "-r".into(), "{url}".into()]),
            repo_list_args: Some(vec!["sources".into()]),
            repo_binary: None,
            repo_list_binary: None,
            repo_remove_binary: None,
            repo_list_shape: RepoListing::Columns,
            depends: None,
            clean_cache: None,
            needs_root: false,
            is_exclusive: false,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: Some(OutdatedProbe {
                binary: None,
                args: vec!["outdated".into()],
                parse: std::sync::Arc::new(crate::parsers::language::parse_gem_outdated),
                silence_is_none: false,
            }),
            search_source: SearchSource::Command,
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::language::parse_installed("gem", o),
            search_fn: |o| crate::parsers::language::parse_search("gem", o),
        }),
    });
    let core = with_manager_policy(core);
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
            .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }))
            .with_repo_manager(Arc::new(GenericRepoManager { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

pub(super) fn register_bun(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "bun".into(),
        executor: executor.clone(),
        config: ManagerConfig {
            name: "bun".into(),
            binary: None,
            remove_binary: None,
            version_pin: Some(VersionPin::Inline("{name}@{version}".into())),
            install_args: vec!["add".into(), "-g".into()],
            remove_args: vec!["remove".into(), "-g".into()],
            purge_args: None,
            list_args: vec!["pm".into(), "ls".into(), "-g".into()],
            // `bun pm ls -g` lists the top-level global installs (dependencies only appear
            // under `--all`), so what it reports is what was asked for.
            manual: ManualListing::AllInstalled,
            essential_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec!["upgrade".into()],
            update_args: None,
            orphan_dry_run: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            repo_binary: None,
            repo_list_binary: None,
            repo_remove_binary: None,
            repo_list_shape: RepoListing::Columns,
            depends: None,
            // bun keeps its global module cache under `~/.bun/install/cache`.
            clean_cache: Some(CacheClean {
                binary: None,
                args: vec!["pm".into(), "cache".into(), "rm".into()],
            }),
            needs_root: false,
            is_exclusive: false,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: None,
            search_source: SearchSource::Command,
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::language::parse_installed("bun", o),
            search_fn: |_| vec![],
        }),
    });
    let core = with_manager_policy(core);
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

/// .NET global tools (`dotnet tool ...`). Cross-platform; gated by the `dotnet` binary.
/// This is the system-inventory surface of the .NET ecosystem — plain NuGet packages
/// are project-scoped and deliberately out of scope.
pub(super) fn register_dotnet(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "dotnet".into(),
        executor: executor.clone(),
        config: ManagerConfig {
            name: "dotnet".into(),
            binary: None,
            remove_binary: None,
            version_pin: Some(VersionPin::after(vec![
                "--version".into(),
                "{version}".into(),
            ])),
            install_args: vec!["tool".into(), "install".into(), "--global".into()],
            remove_args: vec!["tool".into(), "uninstall".into(), "--global".into()],
            purge_args: None,
            list_args: vec!["tool".into(), "list".into(), "--global".into()],
            // Global .NET tools are installed one by one, on request.
            manual: ManualListing::AllInstalled,
            essential_args: None,
            search_args: vec!["tool".into(), "search".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec![
                "tool".into(),
                "update".into(),
                "--global".into(),
                "--all".into(),
            ],
            update_args: None,
            orphan_dry_run: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            repo_binary: None,
            repo_list_binary: None,
            repo_remove_binary: None,
            repo_list_shape: RepoListing::Columns,
            depends: None,
            // The NuGet http-cache, global-packages and temp folders. `all` is every local
            // that `dotnet nuget locals --list` reports.
            clean_cache: Some(CacheClean {
                binary: None,
                args: vec![
                    "nuget".into(),
                    "locals".into(),
                    "all".into(),
                    "--clear".into(),
                ],
            }),
            needs_root: false,
            is_exclusive: false,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: Some(MachineListing {
                binary: None,
                // SDK 10 and later. Older SDKs reject the flag, which is what the
                // negotiation in `fetch_installed` is for.
                args: vec![
                    "tool".into(),
                    "list".into(),
                    "--global".into(),
                    "--format".into(),
                    "json".into(),
                ],
                parse: std::sync::Arc::new(crate::parsers::dotnet::parse_dotnet_list_json),
            }),
            outdated: None,
            search_source: SearchSource::Command,
        },
        parser: Arc::new(LambdaParser {
            installed_fn: crate::parsers::dotnet::parse_dotnet_list,
            search_fn: |o| crate::parsers::dotnet::parse_dotnet_search(o).unwrap_or_default(),
        }),
    });
    let core = with_manager_policy(core);
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
            .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

/// Python applications in their own venvs. Was 193 non-test lines, of which the only part
/// outside this table was asking pipx where a venv lives.
/// conda — **the backend that made the data path able to say "this argv depends on a setting".**
///
/// Every conda verb is environment-scoped: `-n <env>`, where the env is a user choice read from
/// `backend_settings.conda.env`. A `ManagerConfig` row is fixed at registration and had no way to
/// carry that, so the answer for 319 lines was a hand-written backend — one that, being bespoke,
/// overrode `essential()`, `purge`, `tracks_manual`, `Enumerable` and `RepoManager` exactly zero
/// times. **A bespoke backend is a data row someone wrote in Rust, minus eight capabilities.**
///
/// `{setting.env|base}` is the whole of what was missing. `ManagerConfig::resolve_settings`
/// substitutes it once, at registration, from this backend's `[backend_settings]` block, which is
/// where `conda.rs`'s own `resolve_env(cfg)` read it too.
///
/// **`search` deliberately carries no `-n`.** A search spans the configured channels rather than
/// one environment, and the hand-written backend said so in a comment; here it is said by the row
/// simply not naming the placeholder.
pub(super) fn register_conda(
    reg: &mut BackendRegistry,
    executor: &CommandExecutor,
    cfg_src: &Config,
) {
    const ENV: &str = "{setting.env|base}";
    let mut cfg = base_config("conda");
    // Conda pins with `name=version` (one `=`, unlike pip's two).
    cfg.version_pin = Some(VersionPin::Inline("{name}={version}".into()));
    cfg.install_args = vec!["install".into(), "-n".into(), ENV.into(), "-y".into()];
    cfg.remove_args = vec!["remove".into(), "-n".into(), ENV.into(), "-y".into()];
    cfg.list_args = vec!["list".into(), "-n".into(), ENV.into(), "--json".into()];
    cfg.search_args = vec!["search".into(), "--json".into()];
    cfg.upgrade_args = vec![
        "update".into(),
        "-n".into(),
        ENV.into(),
        "-y".into(),
        "--all".into(),
    ];
    // No index-refresh step: conda resolves against its channels live, which is what
    // `update_args: None` means for every generic backend.
    cfg.update_args = None;
    // `conda list` returns the environment's whole solved closure, so it cannot answer "what did
    // the user ask for?" — 88 packages against 4 on a stock `base`. `env export --from-history`
    // can, and its shape is a `dependencies` array of match-specs rather than the package objects
    // `list --json` returns: the same manager, two formats, which is what `ManualFormat::Read`
    // exists to say.
    cfg.manual = ManualListing::Command {
        binary: None,
        args: vec![
            "env".into(),
            "export".into(),
            "-n".into(),
            ENV.into(),
            "--from-history".into(),
            "--json".into(),
        ],
        format: ManualFormat::Read(Arc::new(crate::parsers::conda::parse_conda_history)),
    };
    // One conda process at a time: its environments share a package cache and a solver lock.
    cfg.is_exclusive = true;
    cfg.needs_root = false;
    // conda resolves dependencies internally at install time and exposes no cheap stable
    // per-package query, so it is not asked.
    cfg.depends = None;

    // The row is not finished until the machine's settings are in it. A failure here names the
    // key the user has to set; it cannot silently ship `-n {setting.env}` to conda.
    if let Err(e) = cfg.resolve_settings(cfg_src.backend_settings.get("conda")) {
        tracing::warn!("conda: {e}");
        return;
    }

    let core = Arc::new(GenericBackendCore {
        name: "conda".into(),
        executor: executor.clone(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: crate::parsers::conda::parse_conda_list,
            search_fn: crate::parsers::conda::parse_conda_search,
        }),
    });
    register_generic(reg, core, true, true, true);
}
