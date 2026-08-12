//! **The managers an operating system ships or blesses: winget, scoop, choco, mas.**
//!
//! Grouped because they share the awkward half of this codebase's history — identifiers that
//! are not words (`ARP\Machine\X64\Mozilla Firefox`), listings that are fixed-width tables
//! rather than lines, and shims that are `.cmd` or `.ps1` files rather than executables. Nearly
//! every rule in the validator about spaces, backslashes and braces was written for a name one
//! of these four printed.

// src/backends/registry.rs

use crate::backends::generic::RepoListing;
use crate::backends::generic::{ExportFormat, MachineListing, OutdatedProbe};
use crate::backends::generic::{
    GenericBackendCore, GenericInstallable, GenericQueryable, GenericRepoManager,
    GenericSearchable, GenericUpgradable, ManagerConfig, ManualListing, SearchSource, VersionPin,
};
use crate::core::{BackendCapabilities, CommandExecutor};
use crate::parsers::windows;
use crate::parsers::LambdaParser;
use std::sync::Arc;

use super::{with_manager_policy, BackendRegistry};

pub(super) fn register_winget(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "winget".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "winget".into(),
            binary: None,
            remove_binary: None,
            version_pin: Some(VersionPin::after(vec![
                "--version".into(),
                "{version}".into(),
            ])),
            install_args: vec![
                "install".into(),
                "--silent".into(),
                "--accept-source-agreements".into(),
                "--accept-package-agreements".into(),
            ],
            remove_args: vec!["uninstall".into(), "--silent".into()],
            purge_args: None,
            list_args: vec!["list".into()],
            // `winget list` is the machine, not the manifest. 186 of the 280 rows it reports on
            // a stock Windows box are `ARP\…`/`MSIX\…` identifiers winget synthesises from the
            // registry: `winget uninstall` takes them, `winget install` answers `No package
            // found matching input criteria` for every one, and a third of them carry their own
            // version so the name changes when the package updates. `winget export` is winget's
            // own answer to what it could put back, and adoption may write nothing else.
            manual: ManualListing::ExportFile {
                binary: None,
                args: vec![
                    "export".into(),
                    "-o".into(),
                    "{file}".into(),
                    // No `--include-versions`: adoption declares a package, not a moment.
                    "--accept-source-agreements".into(),
                    "--disable-interactivity".into(),
                ],
                format: ExportFormat::WingetJson,
            },
            essential_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec!["upgrade".into(), "--all".into(), "--silent".into()],
            update_args: Some(vec!["source".into(), "update".into()]),
            orphan_dry_run: None,
            repo_add_args: Some(vec![
                "source".into(),
                "add".into(),
                "--name".into(),
                "{name}".into(),
                "--arg".into(),
                "{url}".into(),
            ]),
            repo_remove_args: Some(vec![
                "source".into(),
                "remove".into(),
                "--name".into(),
                "{name}".into(),
            ]),
            repo_list_args: Some(vec!["source".into(), "list".into()]),
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
                args: vec![
                    "upgrade".into(),
                    "--disable-interactivity".into(),
                    "--accept-source-agreements".into(),
                ],
                parse: std::sync::Arc::new(windows::parse_winget_outdated),
                silence_is_none: false,
            }),
            search_source: SearchSource::Command,
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| windows::parse_installed("winget", o),
            search_fn: |o| windows::parse_search("winget", o),
        }),
    });
    // Without this the manager runs on `ExitPolicy::default()` - no failure, permanent or
    // absent markers - so `scoop install <no-such-package>` is classified `unknown`, Q1 never
    // withdraws the line, and the dead declaration fails every later install.
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

pub(super) fn register_scoop(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "scoop".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "scoop".into(),
            binary: None,
            remove_binary: None,
            version_pin: None, // scoop pins via versioned manifests; not a simple flag

            install_args: vec!["install".into()],
            remove_args: vec!["uninstall".into()],
            purge_args: None,
            list_args: vec!["list".into()],
            // scoop apps are each installed on request; it tracks no dependency graph.
            manual: ManualListing::AllInstalled,
            essential_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec!["update".into(), "*".into()],
            update_args: Some(vec!["update".into()]),
            orphan_dry_run: None,
            repo_add_args: Some(vec![
                "bucket".into(),
                "add".into(),
                "{name}".into(),
                "{url}".into(),
            ]),
            repo_remove_args: Some(vec!["bucket".into(), "rm".into(), "{name}".into()]),
            repo_list_args: Some(vec!["bucket".into(), "list".into()]),
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
            machine_list: Some(MachineListing {
                binary: None,
                // `scoop export` is JSON in current scoop and plain lines in older ones; the
                // negotiation below is what makes asking safe.
                args: vec!["export".into()],
                parse: std::sync::Arc::new(windows::parse_scoop_export),
            }),
            outdated: Some(OutdatedProbe {
                binary: None,
                args: vec!["status".into()],
                parse: std::sync::Arc::new(windows::parse_scoop_outdated),
                silence_is_none: false,
            }),
            search_source: SearchSource::Command,
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| windows::parse_installed("scoop", o),
            search_fn: |o| windows::parse_search("scoop", o),
        }),
    });
    // Without this the manager runs on `ExitPolicy::default()` - no failure, permanent or
    // absent markers - so `scoop install <no-such-package>` is classified `unknown`, Q1 never
    // withdraws the line, and the dead declaration fails every later install.
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

pub(super) fn register_choco(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "choco".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "choco".into(),
            binary: None,
            remove_binary: None,
            version_pin: Some(VersionPin::after(vec![
                "--version".into(),
                "{version}".into(),
            ])),
            install_args: vec!["install".into(), "-y".into()],
            remove_args: vec!["uninstall".into(), "-y".into()],
            purge_args: None,
            // Chocolatey 2.x removed `-lo`: `list` is local-only now and the flag is an
            // error, so the command failed, the output was empty, and Shall took that as
            // "nothing is installed" — the input to a mass removal, not a bad listing.
            list_args: vec!["list".into(), "-r".into()],
            // `choco list` reports locally-installed packages, all user-requested.
            manual: ManualListing::AllInstalled,
            essential_args: None,
            // `-r` (--limit-output) makes search machine-readable `name|version` rows. Without
            // it choco prints its own `Chocolatey v2.7.3` banner and an `N packages found.`
            // summary, and both became packages in the results. `list` was given `-r` long
            // ago, for a related reason; its twin was left alone.
            search_args: vec!["search".into(), "-r".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec!["upgrade".into(), "all".into(), "-y".into()],
            update_args: None,
            orphan_dry_run: None,
            repo_add_args: Some(vec![
                "source".into(),
                "add".into(),
                "-n".into(),
                "{name}".into(),
                "-s".into(),
                "{url}".into(),
            ]),
            repo_remove_args: Some(vec![
                "source".into(),
                "remove".into(),
                "-n".into(),
                "{name}".into(),
            ]),
            repo_list_args: Some(vec!["source".into(), "list".into()]),
            repo_binary: None,
            repo_list_binary: None,
            repo_remove_binary: None,
            repo_list_shape: RepoListing::Columns,
            depends: None,
            clean_cache: None,
            needs_root: true,
            is_exclusive: true,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: Some(OutdatedProbe {
                binary: None,
                args: vec!["outdated".into(), "-r".into()],
                parse: std::sync::Arc::new(windows::parse_choco_outdated),
                silence_is_none: false,
            }),
            search_source: SearchSource::Command,
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| windows::parse_installed("choco", o),
            search_fn: |o| windows::parse_search("choco", o),
        }),
    });
    // Without this the manager runs on `ExitPolicy::default()` - no failure, permanent or
    // absent markers - so `scoop install <no-such-package>` is classified `unknown`, Q1 never
    // withdraws the line, and the dead declaration fails every later install.
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

pub(super) fn register_mas(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "mas".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "mas".into(),
            binary: None,
            remove_binary: None,
            version_pin: None, // Mac App Store installs the current published version only

            install_args: vec!["install".into()],
            remove_args: vec!["uninstall".into()],
            purge_args: None,
            list_args: vec!["list".into()],
            // Every App Store app was installed by a person clicking Get.
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
            clean_cache: None,
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
            installed_fn: crate::parsers::macos::parse_mas_list,
            search_fn: crate::parsers::macos::parse_mas_search,
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
