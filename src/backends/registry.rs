// src/backends/registry.rs

use crate::core::{BackendCapabilities, CommandExecutor};
use crate::config::Config;
use crate::app::LuaHooks;
use crate::backends::generic::{
    GenericBackendCore, GenericInstallable, GenericQueryable, GenericSearchable,
    GenericUpgradable, GenericRepoManager, ManagerConfig, VersionPin,
};
use crate::backends::pip_search::PipSearchable;
use crate::parsers::{windows, LambdaParser};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::trace;

pub struct BackendRegistry {
    backends: HashMap<String, Arc<BackendCapabilities>>,
}

impl Default for BackendRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl BackendRegistry {
    pub fn new() -> Self {
        Self { backends: HashMap::new() }
    }

    pub fn register(&mut self, backend: Arc<BackendCapabilities>) {
        let name = backend.name().to_string();
        trace!("Registry: Cataloging backend '{}'", name);
        self.backends.insert(name, backend);
    }

    pub fn get(&self, name: &str) -> Option<Arc<BackendCapabilities>> {
        self.backends.get(name).cloned()
    }

    pub fn available(&self) -> Vec<Arc<BackendCapabilities>> {
        self.backends.values()
            .filter(|b| b.is_available())
            .cloned()
            .collect()
    }

    pub fn all(&self) -> Vec<Arc<BackendCapabilities>> {
        self.backends.values().cloned().collect()
    }

    pub fn get_filtered(&self, enabled: &[String]) -> Vec<Arc<BackendCapabilities>> {
        self.available().into_iter()
            .filter(|b| enabled.contains(&b.name().to_string()))
            .collect()
    }
}

/// Build the default backend registry.
///
/// This is a thin orchestrator: each specialized backend owns its own
/// `register(reg, exec, cfg)` in its module, and the generic (CLI-config-driven)
/// backends are registered by the small `register_*` helpers below. Adding a backend
/// is a localized change — write its `register` and add one call here.
pub async fn create_default_registry(
    executor: CommandExecutor,
    config: &Config,
    _hooks: Arc<LuaHooks>,
) -> BackendRegistry {
    let mut reg = BackendRegistry::new();

    // --- Linux native system managers ---
    #[cfg(target_os = "linux")]
    {
        register_apt(&mut reg, &executor);
        register_apk(&mut reg, &executor);
        register_zypper(&mut reg, &executor);
        crate::backends::pacman::register(&mut reg, &executor, config);
        crate::backends::dnf::register(&mut reg, &executor, config);
    }

    // --- Windows native system managers ---
    #[cfg(target_os = "windows")]
    {
        register_winget(&mut reg, &executor);
        register_scoop(&mut reg, &executor);
        register_choco(&mut reg, &executor);
    }

    // --- macOS native system managers ---
    #[cfg(target_os = "macos")]
    {
        register_mas(&mut reg, &executor);
    }

    // --- Cross-platform & specialized backends (each module owns its registration) ---
    crate::backends::brew::register(&mut reg, &executor, config);
    crate::backends::cargo::register(&mut reg, &executor, config);
    crate::backends::pipx::register(&mut reg, &executor, config);
    crate::backends::npm::register(&mut reg, &executor, config);
    crate::backends::pnpm::register(&mut reg, &executor, config);
    crate::backends::yarn::register(&mut reg, &executor, config);
    crate::backends::mise::register(&mut reg, &executor, config);
    crate::backends::github::register(&mut reg, &executor, config);
    crate::backends::web::register(&mut reg, &executor, config);
    crate::backends::btrfs::register(&mut reg, &executor, config);
    crate::backends::link::register(&mut reg, &executor, config);
    crate::backends::nix::register(&mut reg, &executor, config);
    crate::backends::vscode::register(&mut reg, &executor, config);
    crate::backends::emacs::register(&mut reg, &executor, config);
    crate::backends::service::register(&mut reg, &executor, config);
    crate::backends::appimage::register(&mut reg, &executor, config);
    crate::backends::snap::register(&mut reg, &executor, config);
    crate::backends::flatpak::register(&mut reg, &executor, config);

    // --- Language package managers (generic, config-driven) ---
    register_pip(&mut reg, &executor);
    register_gem(&mut reg, &executor);
    register_bun(&mut reg, &executor);

    reg
}

// ============================================================================
// Generic (CLI-config-driven) backend registrations
// ============================================================================

#[cfg(target_os = "linux")]
fn register_apt(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "apt".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "apt".into(),
            version_pin: Some(VersionPin::Inline("{name}={version}".into())),
            install_args: vec!["install".into(), "-y".into()],
            remove_args: vec!["purge".into(), "-y".into()],
            list_args: vec!["dpkg-query".into(), "-W".into(), "-f=${Package} ${Version}\\n".into()],
            list_manual_args: Some(vec!["apt-mark".into(), "showmanual".into()]),
            search_args: vec!["search".into()],
            search_binary: Some("apt-cache".into()),
            upgrade_args: vec!["dist-upgrade".into(), "-y".into()],
            update_args: Some(vec!["update".into()]),
            repo_add_args: Some(vec!["add-apt-repository".into(), "-y".into(), "{url}".into()]),
            repo_remove_args: Some(vec!["add-apt-repository".into(), "--remove".into(), "-y".into(), "{name}".into()]),
            repo_list_args: None,
            depends_args: Some(vec!["depends".into(), "--no-recommends".into(), "--no-suggests".into(), "{name}".into()]),
            needs_root: true,
            is_exclusive: true,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: crate::parsers::apt::parse_list,
            search_fn: crate::parsers::apt::parse_search,
        }),
    });
    reg.register(Arc::new(BackendCapabilities::builder(core.clone())
        .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
        .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
        .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
        .with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }))
        .with_repo_manager(Arc::new(GenericRepoManager { core: core.clone() }))
        .with_metadata_provider(core.clone())
        .build()));
}

#[cfg(target_os = "linux")]
fn register_apk(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "apk".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "apk".into(),
            version_pin: Some(VersionPin::Inline("{name}={version}".into())),
            install_args: vec!["add".into()],
            remove_args: vec!["del".into()],
            list_args: vec!["info".into(), "-v".into()],
            list_manual_args: Some(vec!["world".into()]),
            search_args: vec!["search".into(), "-v".into()],
            search_binary: None,
            upgrade_args: vec!["upgrade".into()],
            update_args: Some(vec!["update".into()]),
            repo_add_args: Some(vec!["sh".into(), "-c".into(), "echo '{url}' >> /etc/apk/repositories".into()]),
            repo_remove_args: Some(vec!["sh".into(), "-c".into(), "sed -i '\\|{url}|d' /etc/apk/repositories".into()]),
            repo_list_args: Some(vec!["cat".into(), "/etc/apk/repositories".into()]),
            depends_args: Some(vec!["info".into(), "-R".into(), "{name}".into()]),
            needs_root: true,
            is_exclusive: true,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::common::parse_simple_list(o, "apk"),
            search_fn: |o| crate::parsers::common::parse_simple_list(o, "apk"),
        }),
    });
    reg.register(Arc::new(BackendCapabilities::builder(core.clone())
        .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
        .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
        .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
        .with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }))
        .with_repo_manager(Arc::new(GenericRepoManager { core: core.clone() }))
        .with_metadata_provider(core.clone())
        .build()));
}

#[cfg(target_os = "linux")]
fn register_zypper(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "zypper".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "zypper".into(),
            version_pin: Some(VersionPin::Inline("{name}={version}".into())),
            install_args: vec!["install".into(), "-y".into()],
            remove_args: vec!["remove".into(), "-y".into()],
            list_args: vec!["search".into(), "--installed-only".into()],
            list_manual_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            upgrade_args: vec!["update".into(), "-y".into()],
            update_args: Some(vec!["refresh".into()]),
            repo_add_args: Some(vec!["addrepo".into(), "{url}".into(), "{name}".into()]),
            repo_remove_args: Some(vec!["removerepo".into(), "{name}".into()]),
            repo_list_args: Some(vec!["repos".into()]),
            depends_args: Some(vec!["info".into(), "--requires".into(), "{name}".into()]),
            needs_root: true,
            is_exclusive: true,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: crate::parsers::dnf::parse_zypper_search,
            search_fn: crate::parsers::dnf::parse_zypper_search,
        }),
    });
    reg.register(Arc::new(BackendCapabilities::builder(core.clone())
        .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
        .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
        .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
        .with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }))
        .with_repo_manager(Arc::new(GenericRepoManager { core: core.clone() }))
        .with_metadata_provider(core.clone())
        .build()));
}

#[cfg(target_os = "windows")]
fn register_winget(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "winget".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "winget".into(),
            version_pin: Some(VersionPin::Flag(vec!["--version".into(), "{version}".into()])),
            install_args: vec!["install".into(), "--silent".into(), "--accept-source-agreements".into(), "--accept-package-agreements".into()],
            remove_args: vec!["uninstall".into(), "--silent".into()],
            list_args: vec!["list".into()],
            list_manual_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            upgrade_args: vec!["upgrade".into(), "--all".into(), "--silent".into()],
            update_args: Some(vec!["source".into(), "update".into()]),
            repo_add_args: Some(vec!["source".into(), "add".into(), "--name".into(), "{name}".into(), "--arg".into(), "{url}".into()]),
            repo_remove_args: Some(vec!["source".into(), "remove".into(), "--name".into(), "{name}".into()]),
            repo_list_args: Some(vec!["source".into(), "list".into()]),
            depends_args: None,
            needs_root: false,
            is_exclusive: false,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| windows::parse_installed("winget", o),
            search_fn: |o| windows::parse_search("winget", o),
        }),
    });
    reg.register(Arc::new(BackendCapabilities::builder(core.clone())
        .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
        .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
        .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
        .with_repo_manager(Arc::new(GenericRepoManager { core: core.clone() }))
        .with_metadata_provider(core.clone())
        .build()));
}

#[cfg(target_os = "windows")]
fn register_scoop(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "scoop".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "scoop".into(),
            version_pin: None, // scoop pins via versioned manifests; not a simple flag

            install_args: vec!["install".into()],
            remove_args: vec!["uninstall".into()],
            list_args: vec!["list".into()],
            list_manual_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            upgrade_args: vec!["update".into(), "*".into()],
            update_args: Some(vec!["update".into()]),
            repo_add_args: Some(vec!["bucket".into(), "add".into(), "{name}".into(), "{url}".into()]),
            repo_remove_args: Some(vec!["bucket".into(), "rm".into(), "{name}".into()]),
            repo_list_args: Some(vec!["bucket".into(), "list".into()]),
            depends_args: None,
            needs_root: false,
            is_exclusive: false,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| windows::parse_installed("scoop", o),
            search_fn: |o| windows::parse_search("scoop", o),
        }),
    });
    reg.register(Arc::new(BackendCapabilities::builder(core.clone())
        .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
        .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
        .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
        .with_repo_manager(Arc::new(GenericRepoManager { core: core.clone() }))
        .with_metadata_provider(core.clone())
        .build()));
}

#[cfg(target_os = "windows")]
fn register_choco(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "choco".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "choco".into(),
            version_pin: Some(VersionPin::Flag(vec!["--version".into(), "{version}".into()])),
            install_args: vec!["install".into(), "-y".into()],
            remove_args: vec!["uninstall".into(), "-y".into()],
            list_args: vec!["list".into(), "-lo".into(), "-r".into()],
            list_manual_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            upgrade_args: vec!["upgrade".into(), "all".into(), "-y".into()],
            update_args: None,
            repo_add_args: Some(vec!["source".into(), "add".into(), "-n".into(), "{name}".into(), "-s".into(), "{url}".into()]),
            repo_remove_args: Some(vec!["source".into(), "remove".into(), "-n".into(), "{name}".into()]),
            repo_list_args: Some(vec!["source".into(), "list".into()]),
            depends_args: None,
            needs_root: true,
            is_exclusive: true,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| windows::parse_installed("choco", o),
            search_fn: |o| windows::parse_search("choco", o),
        }),
    });
    reg.register(Arc::new(BackendCapabilities::builder(core.clone())
        .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
        .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
        .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
        .with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }))
        .with_repo_manager(Arc::new(GenericRepoManager { core: core.clone() }))
        .with_metadata_provider(core.clone())
        .build()));
}

#[cfg(target_os = "macos")]
fn register_mas(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "mas".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "mas".into(),
            version_pin: None, // Mac App Store installs the current published version only

            install_args: vec!["install".into()],
            remove_args: vec!["uninstall".into()],
            list_args: vec!["list".into()],
            list_manual_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            upgrade_args: vec!["upgrade".into()],
            update_args: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            depends_args: None,
            needs_root: false,
            is_exclusive: false,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: crate::parsers::macos::parse_mas_list,
            search_fn: crate::parsers::macos::parse_mas_search,
        }),
    });
    reg.register(Arc::new(BackendCapabilities::builder(core.clone())
        .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
        .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
        .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
        .with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }))
        .with_metadata_provider(core.clone())
        .build()));
}

fn register_pip(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    // Generic for install/list; Searchable is a bespoke PyPI JSON lookup
    // (pip's own `search` was disabled upstream).
    let core = Arc::new(GenericBackendCore {
        name: "pip".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "pip".into(),
            version_pin: Some(VersionPin::Inline("{name}=={version}".into())),
            install_args: vec!["install".into()],
            remove_args: vec!["uninstall".into(), "-y".into()],
            list_args: vec!["list".into(), "--format=json".into()],
            list_manual_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            upgrade_args: vec!["install".into(), "--upgrade".into()],
            update_args: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            depends_args: None,
            needs_root: false,
            is_exclusive: false,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::language::parse_installed("pip", o),
            search_fn: |_| vec![],
        }),
    });
    reg.register(Arc::new(BackendCapabilities::builder(core.clone())
        .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
        .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
        .with_searchable(Arc::new(PipSearchable))
        .with_metadata_provider(core.clone())
        .build()));
}

fn register_gem(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "gem".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "gem".into(),
            version_pin: Some(VersionPin::Flag(vec!["-v".into(), "{version}".into()])),
            install_args: vec!["install".into()],
            remove_args: vec!["uninstall".into()],
            list_args: vec!["list".into(), "--local".into()],
            list_manual_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            upgrade_args: vec!["update".into()],
            update_args: None,
            repo_add_args: Some(vec!["sources".into(), "-a".into(), "{url}".into()]),
            repo_remove_args: Some(vec!["sources".into(), "-r".into(), "{url}".into()]),
            repo_list_args: Some(vec!["sources".into()]),
            depends_args: None,
            needs_root: false,
            is_exclusive: false,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::language::parse_installed("gem", o),
            search_fn: |o| crate::parsers::language::parse_search("gem", o),
        }),
    });
    reg.register(Arc::new(BackendCapabilities::builder(core.clone())
        .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
        .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
        .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
        .with_repo_manager(Arc::new(GenericRepoManager { core: core.clone() }))
        .with_metadata_provider(core.clone())
        .build()));
}

fn register_bun(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "bun".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "bun".into(),
            version_pin: Some(VersionPin::Inline("{name}@{version}".into())),
            install_args: vec!["add".into(), "-g".into()],
            remove_args: vec!["remove".into(), "-g".into()],
            list_args: vec!["pm".into(), "ls".into(), "-g".into()],
            list_manual_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            upgrade_args: vec!["upgrade".into()],
            update_args: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            depends_args: None,
            needs_root: false,
            is_exclusive: false,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::language::parse_installed("bun", o),
            search_fn: |_| vec![],
        }),
    });
    reg.register(Arc::new(BackendCapabilities::builder(core.clone())
        .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
        .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
        .with_metadata_provider(core.clone())
        .build()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LuaHooks;

    async fn build_registry() -> BackendRegistry {
        let exec = CommandExecutor::new(true, false);
        let config = Config::default();
        let hooks = Arc::new(LuaHooks::new(&config).expect("hooks init"));
        create_default_registry(exec, &config, hooks).await
    }

    /// The set of capability labels a backend currently advertises.
    fn caps(b: &BackendCapabilities) -> Vec<&'static str> {
        let mut v = Vec::new();
        if b.as_installable().is_some() { v.push("installable"); }
        if b.as_queryable().is_some() { v.push("queryable"); }
        if b.as_searchable().is_some() { v.push("searchable"); }
        if b.as_upgradable().is_some() { v.push("upgradable"); }
        if b.as_repo_manager().is_some() { v.push("repo_manager"); }
        if b.as_metadata_provider().is_some() { v.push("metadata_provider"); }
        v
    }

    /// Assert a backend is registered with EXACTLY the expected capability set.
    /// Exact-match catches both a dropped `.with_*` (e.g. after a refactor) and an
    /// accidental extra capability.
    fn assert_caps(reg: &BackendRegistry, name: &str, expected: &[&str]) {
        let b = reg.get(name).unwrap_or_else(|| panic!("backend '{}' is not registered", name));
        let got = caps(&b);
        for cap in expected {
            assert!(got.contains(cap), "backend '{}' is missing capability '{}' (has {:?})", name, cap, got);
        }
        assert_eq!(
            got.len(), expected.len(),
            "backend '{}' capability set mismatch: got {:?}, expected {:?}", name, got, expected
        );
    }

    // Regression guard for the per-backend register() refactor: every backend must
    // register with its intended capability set. Cross-platform backends are asserted
    // everywhere; OS-native ones under their cfg (so Linux apt/dnf/pacman are checked
    // when this runs on Linux).
    #[tokio::test]
    async fn registry_capability_matrix() {
        let reg = build_registry().await;

        const FULL: &[&str] = &["installable", "queryable", "searchable", "upgradable", "metadata_provider"];

        // Cross-platform specialized backends
        assert_caps(&reg, "brew", FULL);
        assert_caps(&reg, "cargo", FULL);
        assert_caps(&reg, "pipx", &["installable", "queryable", "upgradable", "metadata_provider"]);
        assert_caps(&reg, "npm", FULL);
        assert_caps(&reg, "pnpm", FULL);
        assert_caps(&reg, "yarn", FULL);
        assert_caps(&reg, "mise", FULL);
        assert_caps(&reg, "github", &["installable", "queryable", "metadata_provider"]);
        assert_caps(&reg, "web", &["installable", "queryable", "metadata_provider"]);
        assert_caps(&reg, "btrfs", &["installable", "queryable", "metadata_provider"]);
        assert_caps(&reg, "link", &["installable", "metadata_provider"]);
        assert_caps(&reg, "nix", FULL);
        assert_caps(&reg, "vscode", FULL);
        assert_caps(&reg, "emacs", FULL);
        assert_caps(&reg, "service", &["installable", "queryable", "metadata_provider"]);
        assert_caps(&reg, "appimage", &["installable", "queryable", "metadata_provider"]);
        assert_caps(&reg, "snap", FULL);
        assert_caps(&reg, "flatpak", FULL);

        // Language managers (generic)
        assert_caps(&reg, "pip", &["installable", "queryable", "searchable", "metadata_provider"]);
        assert_caps(&reg, "gem", &["installable", "queryable", "searchable", "repo_manager", "metadata_provider"]);
        assert_caps(&reg, "bun", &["installable", "queryable", "metadata_provider"]);

        #[cfg(target_os = "linux")]
        {
            const SYS: &[&str] = &["installable", "queryable", "searchable", "upgradable", "repo_manager", "metadata_provider"];
            assert_caps(&reg, "apt", SYS);
            assert_caps(&reg, "apk", SYS);
            assert_caps(&reg, "zypper", SYS);
            assert_caps(&reg, "pacman", SYS);
            assert_caps(&reg, "dnf", SYS);
        }
        #[cfg(target_os = "windows")]
        {
            assert_caps(&reg, "winget", &["installable", "queryable", "searchable", "repo_manager", "metadata_provider"]);
            assert_caps(&reg, "scoop", &["installable", "queryable", "searchable", "repo_manager", "metadata_provider"]);
            assert_caps(&reg, "choco", &["installable", "queryable", "searchable", "upgradable", "repo_manager", "metadata_provider"]);
        }
        #[cfg(target_os = "macos")]
        {
            assert_caps(&reg, "mas", &["installable", "queryable", "searchable", "upgradable", "metadata_provider"]);
        }
    }
}
