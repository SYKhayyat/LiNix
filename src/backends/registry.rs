use crate::core::{
    BackendCapabilities, CommandExecutor, Package
};
use crate::config::Config;
use crate::app::LuaHooks;
use crate::backends::generic::{
    GenericBackendCore, GenericInstallable, GenericQueryable,
    GenericUpgradable, GenericRepoManager, ManagerConfig
};
use crate::parsers::{brew, windows, LambdaParser};
use std::collections::HashMap;
use std::sync::Arc;

/// Central registry for all package management backends.
/// Coordinates the capability-based discovery for the parallel engine.
pub struct BackendRegistry {
    backends: HashMap<String, Arc<BackendCapabilities>>,
}

impl BackendRegistry {
    pub fn new() -> Self { 
        Self { backends: HashMap::new() } 
    }
    
    pub fn register(&mut self, backend: Arc<BackendCapabilities>) {
        self.backends.insert(backend.name().to_string(), backend);
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

/// The master wiring function for LiNix.
/// 
/// Hardened for Phase 6.1: Full OS coverage for Linux, macOS, and Windows.
pub async fn create_default_registry(executor: CommandExecutor, config: &Config, _hooks: Arc<LuaHooks>) -> BackendRegistry {
    let mut reg = BackendRegistry::new();

    // --- 1. SYSTEM PACKAGE MANAGERS (Linux) ---
    #[cfg(target_os = "linux")]
    {
        // APT
        let apt_core = Arc::new(GenericBackendCore {
            name: "apt".into(),
            executor: executor.duplicate(),
            config: ManagerConfig {
                name: "apt".into(),
                install_args: vec!["install".into(), "-y".into()],
                remove_args: vec!["purge".into(), "-y".into()],
                list_args: vec!["dpkg-query".into(), "-W".into(), "-f=${Package} ${Version}\\n".into()],
                list_manual_args: Some(vec!["apt-mark".into(), "showmanual".into()]),
                search_args: vec!["apt-cache".into(), "search".into()],
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
                search_fn: crate::parsers::apt::parse_search 
            }),
        });
        
        reg.register(Arc::new(BackendCapabilities::builder(apt_core.clone())
            .with_installable(Arc::new(GenericInstallable { core: apt_core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: apt_core.clone() }))
            .with_upgradable(Arc::new(GenericUpgradable { core: apt_core.clone() }))
            .with_repo_manager(Arc::new(GenericRepoManager { core: apt_core.clone() }))
            .with_metadata_provider(apt_core.clone())
            .build()));

        // Pacman
        let pacman_core = Arc::new(crate::backends::pacman::PacmanBackendCore::new(executor.duplicate()));
        reg.register(Arc::new(BackendCapabilities::builder(pacman_core.clone())
            .with_installable(Arc::new(crate::backends::pacman::PacmanInstallable { core: pacman_core.clone() }))
            .with_queryable(Arc::new(crate::backends::pacman::PacmanQueryable { core: pacman_core.clone() }))
            .with_upgradable(Arc::new(crate::backends::pacman::PacmanUpgradable { core: pacman_core.clone() }))
            .with_metadata_provider(pacman_core.clone())
            .build()));

        // DNF
        let dnf_core = Arc::new(crate::backends::dnf::DnfBackendCore::new(executor.duplicate()));
        reg.register(Arc::new(BackendCapabilities::builder(dnf_core.clone())
            .with_installable(Arc::new(crate::backends::dnf::DnfInstallable { core: dnf_core.clone() }))
            .with_queryable(Arc::new(crate::backends::dnf::DnfQueryable { core: dnf_core.clone() }))
            .with_upgradable(Arc::new(crate::backends::dnf::DnfUpgradable { core: dnf_core.clone() }))
            .with_metadata_provider(dnf_core.clone())
            .build()));
    }

    // --- 2. WINDOWS PACKAGE MANAGERS ---
    #[cfg(target_os = "windows")]
    {
        // Winget
        let winget_core = Arc::new(GenericBackendCore {
            name: "winget".into(),
            executor: executor.duplicate(),
            config: ManagerConfig {
                name: "winget".into(),
                install_args: vec!["install".into(), "--silent".into()],
                remove_args: vec!["uninstall".into(), "--silent".into()],
                list_args: vec!["list".into()],
                list_manual_args: None,
                search_args: vec!["search".into()],
                upgrade_args: vec!["upgrade".into(), "--all".into(), "--silent".into()],
                update_args: Some(vec!["source".into(), "update".into()]),
                repo_add_args: None, repo_remove_args: None, repo_list_args: None,
                depends_args: None,
                needs_root: false,
                is_exclusive: false,
                flag_map: HashMap::new(),
            },
            parser: Arc::new(LambdaParser { 
                installed_fn: |o| windows::parse_installed("winget", o), 
                search_fn: |o| windows::parse_search("winget", o) 
            }),
        });
        
        reg.register(Arc::new(BackendCapabilities::builder(winget_core.clone())
            .with_installable(Arc::new(GenericInstallable { core: winget_core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: winget_core.clone() }))
            .with_metadata_provider(winget_core.clone())
            .build()));

        // Scoop
        let scoop_core = Arc::new(GenericBackendCore {
            name: "scoop".into(),
            executor: executor.duplicate(),
            config: ManagerConfig {
                name: "scoop".into(),
                install_args: vec!["install".into()],
                remove_args: vec!["uninstall".into()],
                list_args: vec!["list".into()],
                list_manual_args: None,
                search_args: vec!["search".into()],
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
                search_fn: |o| windows::parse_search("scoop", o) 
            }),
        });
        
        reg.register(Arc::new(BackendCapabilities::builder(scoop_core.clone())
            .with_installable(Arc::new(GenericInstallable { core: scoop_core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: scoop_core.clone() }))
            .with_repo_manager(Arc::new(GenericRepoManager { core: scoop_core.clone() }))
            .with_metadata_provider(scoop_core.clone())
            .build()));

        // Chocolatey
        let choco_core = Arc::new(GenericBackendCore {
            name: "choco".into(),
            executor: executor.duplicate(),
            config: ManagerConfig {
                name: "choco".into(),
                install_args: vec!["install".into(), "-y".into()],
                remove_args: vec!["uninstall".into(), "-y".into()],
                list_args: vec!["list".into(), "-lo".into(), "-r".into()],
                list_manual_args: None,
                search_args: vec!["search".into()],
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
                search_fn: |o| windows::parse_search("choco", o) 
            }),
        });

        reg.register(Arc::new(BackendCapabilities::builder(choco_core.clone())
            .with_installable(Arc::new(GenericInstallable { core: choco_core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: choco_core.clone() }))
            .with_upgradable(Arc::new(GenericUpgradable { core: choco_core.clone() }))
            .with_repo_manager(Arc::new(GenericRepoManager { core: choco_core.clone() }))
            .with_metadata_provider(choco_core.clone())
            .build()));
    }

    // --- 3. MACOS PACKAGE MANAGERS ---
    #[cfg(target_os = "macos")]
    {
        // Mas (App Store)
        let mas_core = Arc::new(GenericBackendCore {
            name: "mas".into(),
            executor: executor.duplicate(),
            config: ManagerConfig {
                name: "mas".into(),
                install_args: vec!["install".into()],
                remove_args: vec!["uninstall".into()],
                list_args: vec!["list".into()],
                list_manual_args: None,
                search_args: vec!["search".into()],
                upgrade_args: vec!["upgrade".into()],
                update_args: None,
                repo_add_args: None, repo_remove_args: None, repo_list_args: None,
                depends_args: None,
                needs_root: false,
                is_exclusive: false,
                flag_map: HashMap::new(),
            },
            parser: Arc::new(LambdaParser { 
                installed_fn: crate::parsers::macos::parse_mas_list, 
                search_fn: crate::parsers::macos::parse_mas_search 
            }),
        });

        reg.register(Arc::new(BackendCapabilities::builder(mas_core.clone())
            .with_installable(Arc::new(GenericInstallable { core: mas_core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: mas_core.clone() }))
            .with_upgradable(Arc::new(GenericUpgradable { core: mas_core.clone() }))
            .with_metadata_provider(mas_core.clone())
            .build()));
    }

    // --- 4. CROSS-PLATFORM MANAGERS ---
    // Homebrew
    let brew_core = Arc::new(GenericBackendCore {
        name: "brew".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "brew".into(),
            install_args: vec!["install".into()],
            remove_args: vec!["uninstall".into()],
            list_args: vec!["list".into(), "--versions".into()],
            list_manual_args: Some(vec!["leaves".into()]),
            search_args: vec!["search".into()],
            upgrade_args: vec!["upgrade".into()],
            update_args: Some(vec!["update".into()]),
            repo_add_args: Some(vec!["tap".into(), "{name}".into(), "{url}".into()]),
            repo_remove_args: Some(vec!["untap".into(), "{name}".into()]),
            repo_list_args: Some(vec!["tap".into()]),
            depends_args: Some(vec!["deps".into(), "{name}".into()]),
            needs_root: false,
            is_exclusive: false,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser { installed_fn: brew::parse_list, search_fn: brew::parse_search }),
    });
    
    reg.register(Arc::new(BackendCapabilities::builder(brew_core.clone())
        .with_installable(Arc::new(GenericInstallable { core: brew_core.clone() }))
        .with_queryable(Arc::new(GenericQueryable { core: brew_core.clone() }))
        .with_upgradable(Arc::new(GenericUpgradable { core: brew_core.clone() }))
        .with_repo_manager(Arc::new(GenericRepoManager { core: brew_core.clone() }))
        .with_metadata_provider(brew_core.clone())
        .build()));

    // Specialized Modules
    let github_core = Arc::new(crate::backends::github::GithubBackendCore::new(executor.duplicate(), config.github_token.clone()));
    reg.register(Arc::new(BackendCapabilities::builder(github_core.clone())
        .with_installable(Arc::new(crate::backends::github::GithubInstallable { core: github_core.clone() }))
        .with_queryable(Arc::new(crate::backends::github::GithubQueryable { core: github_core.clone() }))
        .with_metadata_provider(github_core.clone())
        .build()));

    let web_core = Arc::new(crate::backends::web::WebBackendCore::new(executor.duplicate()));
    reg.register(Arc::new(BackendCapabilities::builder(web_core.clone())
        .with_installable(Arc::new(crate::backends::web::WebInstallable { core: web_core.clone() }))
        .with_queryable(Arc::new(crate::backends::web::WebQueryable { core: web_core.clone() }))
        .with_metadata_provider(web_core.clone())
        .build()));

    let btrfs_core = Arc::new(crate::backends::btrfs::BtrfsBackendCore::new(executor.duplicate()));
    reg.register(Arc::new(BackendCapabilities::builder(btrfs_core.clone())
        .with_installable(Arc::new(crate::backends::btrfs::BtrfsInstallable { core: btrfs_core.clone() }))
        .with_queryable(Arc::new(crate::backends::btrfs::BtrfsQueryable { core: btrfs_core.clone() }))
        .with_metadata_provider(btrfs_core.clone())
        .build()));

    let link_core = Arc::new(crate::backends::link::LinkBackendCore::new(executor.duplicate(), Arc::new(config.clone())));
    reg.register(Arc::new(BackendCapabilities::builder(link_core.clone())
        .with_installable(Arc::new(crate::backends::link::LinkInstallable { core: link_core.clone() }))
        .with_metadata_provider(link_core.clone())
        .build()));

    let nix_core = Arc::new(crate::backends::nix::NixBackendCore::new(executor.duplicate()));
    reg.register(Arc::new(BackendCapabilities::builder(nix_core.clone())
        .with_installable(Arc::new(crate::backends::nix::NixInstallable { core: nix_core.clone() }))
        .with_queryable(Arc::new(crate::backends::nix::NixQueryable { core: nix_core.clone() }))
        .with_upgradable(Arc::new(crate::backends::nix::NixUpgradable { core: nix_core.clone() }))
        .with_metadata_provider(nix_core.clone())
        .build()));

    let mise_core = Arc::new(crate::backends::mise::MiseBackendCore::new(executor.duplicate()));
    reg.register(Arc::new(BackendCapabilities::builder(mise_core.clone())
        .with_installable(Arc::new(crate::backends::mise::MiseInstallable { core: mise_core.clone() }))
        .with_queryable(Arc::new(crate::backends::mise::MiseQueryable { core: mise_core.clone() }))
        .with_upgradable(Arc::new(crate::backends::mise::MiseUpgradable { core: mise_core.clone() }))
        .with_metadata_provider(mise_core.clone())
        .build()));

    let vscode_core = Arc::new(crate::backends::vscode::VscodeBackendCore::new(executor.duplicate()));
    reg.register(Arc::new(BackendCapabilities::builder(vscode_core.clone())
        .with_installable(Arc::new(crate::backends::vscode::VscodeInstallable { core: vscode_core.clone() }))
        .with_queryable(Arc::new(crate::backends::vscode::VscodeQueryable { core: vscode_core.clone() }))
        .with_searchable(Arc::new(crate::backends::vscode::VscodeSearchable { core: vscode_core.clone() }))
        .with_metadata_provider(vscode_core.clone())
        .build()));

    let emacs_core = Arc::new(crate::backends::emacs::EmacsBackendCore::new(executor.duplicate()));
    reg.register(Arc::new(BackendCapabilities::builder(emacs_core.clone())
        .with_installable(Arc::new(crate::backends::emacs::EmacsInstallable { core: emacs_core.clone() }))
        .with_queryable(Arc::new(crate::backends::emacs::EmacsQueryable { core: emacs_core.clone() }))
        .with_metadata_provider(emacs_core.clone())
        .build()));

    let service_core = Arc::new(crate::backends::service::ServiceBackendCore::new(executor.duplicate()));
    reg.register(Arc::new(BackendCapabilities::builder(service_core.clone())
        .with_installable(Arc::new(crate::backends::service::ServiceInstallable { core: service_core.clone() }))
        .with_queryable(Arc::new(crate::backends::service::ServiceQueryable { core: service_core.clone() }))
        .with_metadata_provider(service_core.clone())
        .build()));

    let appimage_core = Arc::new(crate::backends::appimage::AppImageBackendCore::new(executor.duplicate()));
    reg.register(Arc::new(BackendCapabilities::builder(appimage_core.clone())
        .with_installable(Arc::new(crate::backends::appimage::AppImageInstallable { core: appimage_core.clone() }))
        .with_queryable(Arc::new(crate::backends::appimage::AppImageQueryable { core: appimage_core.clone() }))
        .with_metadata_provider(appimage_core.clone())
        .build()));

    let snap_core = Arc::new(crate::backends::snap::SnapBackendCore::new(executor.duplicate()));
    reg.register(Arc::new(BackendCapabilities::builder(snap_core.clone())
        .with_installable(Arc::new(crate::backends::snap::SnapInstallable { core: snap_core.clone() }))
        .with_queryable(Arc::new(crate::backends::snap::SnapQueryable { core: snap_core.clone() }))
        .with_upgradable(Arc::new(crate::backends::snap::SnapUpgradable { core: snap_core.clone() }))
        .with_metadata_provider(snap_core.clone())
        .build()));

    let flatpak_settings = config.backend_settings.get("flatpak").cloned().unwrap_or_default();
    let flatpak_core = Arc::new(crate::backends::flatpak::FlatpakBackendCore::new(executor.duplicate(), flatpak_settings));
    reg.register(Arc::new(BackendCapabilities::builder(flatpak_core.clone())
        .with_installable(Arc::new(crate::backends::flatpak::FlatpakInstallable { core: flatpak_core.clone() }))
        .with_queryable(Arc::new(crate::backends::flatpak::FlatpakQueryable { core: flatpak_core.clone() }))
        .with_upgradable(Arc::new(crate::backends::flatpak::FlatpakUpgradable { core: flatpak_core.clone() }))
        .with_metadata_provider(flatpak_core.clone())
        .build()));

    // --- 5. LANGUAGE MANAGERS ---
    let langs = ["cargo", "npm", "pip", "pipx", "yarn", "gem", "bun", "pnpm"];
    for name in langs {
        let n_str = name.to_string();
        let parser_fn: fn(&str) -> Vec<Package> = match name {
            "cargo" => |o| crate::parsers::language::parse_installed("cargo", o),
            "npm" => |o| crate::parsers::language::parse_installed("npm", o),
            "pip" => |o| crate::parsers::language::parse_installed("pip", o),
            "pipx" => |o| crate::parsers::language::parse_installed("pipx", o),
            "yarn" => |o| crate::parsers::language::parse_installed("yarn", o),
            "gem" => |o| crate::parsers::language::parse_installed("gem", o),
            "bun" => |o| crate::parsers::language::parse_installed("bun", o),
            "pnpm" => |o| crate::parsers::language::parse_installed("pnpm", o),
            _ => |_| vec![],
        };

        let lang_core = Arc::new(GenericBackendCore {
            name: n_str.clone(),
            executor: executor.duplicate(),
            config: ManagerConfig {
                name: n_str.clone(),
                install_args: {
                    let mut args = vec![match n_str.as_str() {
                        "npm" | "pnpm" | "bun" => "add".to_string(),
                        _ => "install".to_string()
                    }];
                    if n_str == "npm" { args.push("--global".to_string()); }
                    args
                },
                remove_args: {
                    let mut args = vec!["uninstall".to_string()];
                    if n_str == "npm" { args.push("--global".to_string()); }
                    args
                },
                list_args: {
                    let mut args = vec!["list".to_string()];
                    if n_str == "npm" { args.push("--global".to_string()); }
                    args
                },
                list_manual_args: None,
                search_args: vec!["search".into()],
                upgrade_args: vec!["upgrade".into()],
                update_args: None,
                repo_add_args: None, repo_remove_args: None, repo_list_args: None,
                depends_args: None,
                needs_root: false,
                is_exclusive: (n_str == "cargo" || n_str == "npm"),
                flag_map: HashMap::new(),
            },
            parser: Arc::new(LambdaParser { installed_fn: parser_fn, search_fn: |_| vec![] }),
        });
        
        reg.register(Arc::new(BackendCapabilities::builder(lang_core.clone())
            .with_installable(Arc::new(GenericInstallable { core: lang_core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: lang_core.clone() }))
            .with_metadata_provider(lang_core.clone())
            .build()));
    }

    reg
}