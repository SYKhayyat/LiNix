use crate::core::{
    BackendCapabilities, CommandExecutor
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
use tracing::{trace};

/// Central registry for all package management backends.
/// 
/// Functioning as the "Hardware Abstraction Layer" of LiNix, the Registry 
/// provides a capability-based discovery system. It decouples the core engine 
/// from the specific CLI flags and parsing logic of individual tools.
pub struct BackendRegistry {
    /// Internal map of backend identifiers (e.g., "apt") to their capabilities.
    backends: HashMap<String, Arc<BackendCapabilities>>,
}

impl BackendRegistry {
    /// Initializes an empty backend registry.
    pub fn new() -> Self { 
        Self { backends: HashMap::new() } 
    }
    
    /// Registers a new backend capability set.
    pub fn register(&mut self, backend: Arc<BackendCapabilities>) {
        let name = backend.name().to_string();
        trace!("Registry: Cataloging backend '{}'", name);
        self.backends.insert(name, backend);
    }

    /// Retrieves a specific backend by its unique identifier.
    pub fn get(&self, name: &str) -> Option<Arc<BackendCapabilities>> {
        self.backends.get(name).cloned()
    }

    /// Returns a list of backends that are currently present on the host system.
    /// Availability is determined by checking if the tool's binary exists in PATH.
    pub fn available(&self) -> Vec<Arc<BackendCapabilities>> {
        self.backends.values()
            .filter(|b| b.is_available())
            .cloned()
            .collect()
    }

    /// Returns every registered backend, regardless of current system availability.
    /// Essential for the 'linix doctor' diagnostic command.
    pub fn all(&self) -> Vec<Arc<BackendCapabilities>> {
        self.backends.values().cloned().collect()
    }

    /// Returns a filtered subset of available backends based on a whitelist.
    pub fn get_filtered(&self, enabled: &[String]) -> Vec<Arc<BackendCapabilities>> {
        self.available().into_iter()
            .filter(|b| enabled.contains(&b.name().to_string()))
            .collect()
    }
}

/// The Master Wiring Function for LiNix v3.6.0.
/// 
/// This is the most complex assembly point in the application. It instantiates
/// each backend core, attaches the appropriate parsers, and builds the
/// capability sets (Installable, Queryable, Upgradable, etc.) for each tool.
/// 
/// Hardened for v3.6.0: Full repo management for APK and auto-locking for AppImage.
pub async fn create_default_registry(
    executor: CommandExecutor, 
    config: &Config, 
    _hooks: Arc<LuaHooks>
) -> BackendRegistry {
    let mut reg = BackendRegistry::new();

    // ========================================================================
    // 1. LINUX NATIVE SYSTEM MANAGERS
    // ========================================================================
    #[cfg(target_os = "linux")]
    {
        // --- APT (Debian, Ubuntu, Linux Mint) ---
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

        // --- APK (Alpine Linux) ---
        // Bug Fix 9: Now fully supports repository management via /etc/apk/repositories
        let apk_core = Arc::new(GenericBackendCore {
            name: "apk".into(),
            executor: executor.duplicate(),
            config: ManagerConfig {
                name: "apk".into(),
                install_args: vec!["add".into()],
                remove_args: vec!["del".into()],
                list_args: vec!["info".into(), "-v".into()],
                list_manual_args: Some(vec!["world".into()]),
                search_args: vec!["search".into(), "-v".into()],
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
        reg.register(Arc::new(BackendCapabilities::builder(apk_core.clone())
            .with_installable(Arc::new(GenericInstallable { core: apk_core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: apk_core.clone() }))
            .with_upgradable(Arc::new(GenericUpgradable { core: apk_core.clone() }))
            .with_repo_manager(Arc::new(GenericRepoManager { core: apk_core.clone() }))
            .with_metadata_provider(apk_core.clone())
            .build()));

        // --- ZYPPER (OpenSUSE) ---
        let zypper_core = Arc::new(GenericBackendCore {
            name: "zypper".into(),
            executor: executor.duplicate(),
            config: ManagerConfig {
                name: "zypper".into(),
                install_args: vec!["install".into(), "-y".into()],
                remove_args: vec!["remove".into(), "-y".into()],
                list_args: vec!["search".into(), "--installed-only".into()],
                list_manual_args: None,
                search_args: vec!["search".into()],
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
        reg.register(Arc::new(BackendCapabilities::builder(zypper_core.clone())
            .with_installable(Arc::new(GenericInstallable { core: zypper_core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: zypper_core.clone() }))
            .with_upgradable(Arc::new(GenericUpgradable { core: zypper_core.clone() }))
            .with_repo_manager(Arc::new(GenericRepoManager { core: zypper_core.clone() }))
            .with_metadata_provider(zypper_core.clone())
            .build()));

        // --- PACMAN (Arch Linux) ---
        let pacman_core = Arc::new(crate::backends::pacman::PacmanBackendCore::new(executor.duplicate()));
        reg.register(Arc::new(BackendCapabilities::builder(pacman_core.clone())
            .with_installable(Arc::new(crate::backends::pacman::PacmanInstallable { core: pacman_core.clone() }))
            .with_queryable(Arc::new(crate::backends::pacman::PacmanQueryable { core: pacman_core.clone() }))
            .with_upgradable(Arc::new(crate::backends::pacman::PacmanUpgradable { core: pacman_core.clone() }))
            .with_metadata_provider(pacman_core.clone())
            .build()));

        // --- DNF (Fedora, RHEL, CentOS) ---
        let dnf_core = Arc::new(crate::backends::dnf::DnfBackendCore::new(executor.duplicate()));
        reg.register(Arc::new(BackendCapabilities::builder(dnf_core.clone())
            .with_installable(Arc::new(crate::backends::dnf::DnfInstallable { core: dnf_core.clone() }))
            .with_queryable(Arc::new(crate::backends::dnf::DnfQueryable { core: dnf_core.clone() }))
            .with_upgradable(Arc::new(crate::backends::dnf::DnfUpgradable { core: dnf_core.clone() }))
            .with_metadata_provider(dnf_core.clone())
            .build()));
    }

    // ========================================================================
    // 2. WINDOWS NATIVE SYSTEM MANAGERS
    // ========================================================================
    #[cfg(target_os = "windows")]
    {
        // --- WINGET (The Official Windows Package Manager) ---
        let winget_core = Arc::new(GenericBackendCore {
            name: "winget".into(),
            executor: executor.duplicate(),
            config: ManagerConfig {
                name: "winget".into(),
                install_args: vec!["install".into(), "--silent".into(), "--accept-source-agreements".into(), "--accept-package-agreements".into()],
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

        // --- SCOOP (Developer-friendly Windows tool) ---
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

        // --- CHOCOLATEY (Legacy Windows Manager) ---
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

    // ========================================================================
    // 3. MACOS NATIVE SYSTEM MANAGERS
    // ========================================================================
    #[cfg(target_os = "macos")]
    {
        // --- MAS (The Mac App Store CLI) ---
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

    // ========================================================================
    // 4. CROSS-PLATFORM & SPECIALIZED MANAGERS
    // ========================================================================

    // --- HOMEBREW (Universal Linux & macOS) ---
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

    // --- GITHUB RELEASES ---
    let github_core = Arc::new(crate::backends::github::GithubBackendCore::new(
        executor.duplicate(), 
        config.github_dir.clone(),
        config.github_token.clone()
    ));
    reg.register(Arc::new(BackendCapabilities::builder(github_core.clone())
        .with_installable(Arc::new(crate::backends::github::GithubInstallable { core: github_core.clone() }))
        .with_queryable(Arc::new(crate::backends::github::GithubQueryable { core: github_core.clone() }))
        .with_metadata_provider(github_core.clone())
        .build()));

    // --- WEB DOWNLOADS ---
    let web_core = Arc::new(crate::backends::web::WebBackendCore::new(
        executor.duplicate(),
        config.web_dir.clone()
    ));
    reg.register(Arc::new(BackendCapabilities::builder(web_core.clone())
        .with_installable(Arc::new(crate::backends::web::WebInstallable { core: web_core.clone() }))
        .with_queryable(Arc::new(crate::backends::web::WebQueryable { core: web_core.clone() }))
        .with_metadata_provider(web_core.clone())
        .build()));

    // --- BTRFS (Subvolume & Quota Management) ---
    let btrfs_core = Arc::new(crate::backends::btrfs::BtrfsBackendCore::new(executor.duplicate()));
    reg.register(Arc::new(BackendCapabilities::builder(btrfs_core.clone())
        .with_installable(Arc::new(crate::backends::btrfs::BtrfsInstallable { core: btrfs_core.clone() }))
        .with_queryable(Arc::new(crate::backends::btrfs::BtrfsQueryable { core: btrfs_core.clone() }))
        .with_metadata_provider(btrfs_core.clone())
        .build()));

    // --- LINK (Dotfiles & Templating Engine) ---
    let link_core = Arc::new(crate::backends::link::LinkBackendCore::new(executor.duplicate(), Arc::new(config.clone())));
    reg.register(Arc::new(BackendCapabilities::builder(link_core.clone())
        .with_installable(Arc::new(crate::backends::link::LinkInstallable { core: link_core.clone() }))
        .with_metadata_provider(link_core.clone())
        .build()));

    // --- NIX (User Profiles & Flakes) ---
    let nix_core = Arc::new(crate::backends::nix::NixBackendCore::new(executor.duplicate()));
    reg.register(Arc::new(BackendCapabilities::builder(nix_core.clone())
        .with_installable(Arc::new(crate::backends::nix::NixInstallable { core: nix_core.clone() }))
        .with_queryable(Arc::new(crate::backends::nix::NixQueryable { core: nix_core.clone() }))
        .with_upgradable(Arc::new(crate::backends::nix::NixUpgradable { core: nix_core.clone() }))
        .with_metadata_provider(nix_core.clone())
        .build()));

    // --- MISE (Dev Runtime Manager) ---
    let mise_core = Arc::new(crate::backends::mise::MiseBackendCore::new(executor.duplicate()));
    reg.register(Arc::new(BackendCapabilities::builder(mise_core.clone())
        .with_installable(Arc::new(crate::backends::mise::MiseInstallable { core: mise_core.clone() }))
        .with_queryable(Arc::new(crate::backends::mise::MiseQueryable { core: mise_core.clone() }))
        .with_upgradable(Arc::new(crate::backends::mise::MiseUpgradable { core: mise_core.clone() }))
        .with_metadata_provider(mise_core.clone())
        .build()));

    // --- VS CODE EXTENSIONS ---
    let vscode_core = Arc::new(crate::backends::vscode::VscodeBackendCore::new(executor.duplicate()));
    reg.register(Arc::new(BackendCapabilities::builder(vscode_core.clone())
        .with_installable(Arc::new(crate::backends::vscode::VscodeInstallable { core: vscode_core.clone() }))
        .with_queryable(Arc::new(crate::backends::vscode::VscodeQueryable { core: vscode_core.clone() }))
        .with_searchable(Arc::new(crate::backends::vscode::VscodeSearchable { core: vscode_core.clone() }))
        .with_metadata_provider(vscode_core.clone())
        .build()));

    // --- EMACS PACKAGES (package.el) ---
    let emacs_core = Arc::new(crate::backends::emacs::EmacsBackendCore::new(executor.duplicate()));
    reg.register(Arc::new(BackendCapabilities::builder(emacs_core.clone())
        .with_installable(Arc::new(crate::backends::emacs::EmacsInstallable { core: emacs_core.clone() }))
        .with_queryable(Arc::new(crate::backends::emacs::EmacsQueryable { core: emacs_core.clone() }))
        .with_metadata_provider(emacs_core.clone())
        .build()));

    // --- SYSTEM SERVICES (systemctl/sc/launchctl) ---
    let service_core = Arc::new(crate::backends::service::ServiceBackendCore::new(executor.duplicate()));
    reg.register(Arc::new(BackendCapabilities::builder(service_core.clone())
        .with_installable(Arc::new(crate::backends::service::ServiceInstallable { core: service_core.clone() }))
        .with_queryable(Arc::new(crate::backends::service::ServiceQueryable { core: service_core.clone() }))
        .with_metadata_provider(service_core.clone())
        .build()));

    // --- APPIMAGE (Bug Fix 7 Checksum Support) ---
    let appimage_core = Arc::new(crate::backends::appimage::AppImageBackendCore::new(
        executor.duplicate(),
        config.appimage_dir.clone()
    ));
    reg.register(Arc::new(BackendCapabilities::builder(appimage_core.clone())
        .with_installable(Arc::new(crate::backends::appimage::AppImageInstallable { core: appimage_core.clone() }))
        .with_queryable(Arc::new(crate::backends::appimage::AppImageQueryable { core: appimage_core.clone() }))
        .with_metadata_provider(appimage_core.clone())
        .build()));

    // --- SNAP (Ubuntu Snap Store) ---
    let snap_core = Arc::new(crate::backends::snap::SnapBackendCore::new(executor.duplicate()));
    reg.register(Arc::new(BackendCapabilities::builder(snap_core.clone())
        .with_installable(Arc::new(crate::backends::snap::SnapInstallable { core: snap_core.clone() }))
        .with_queryable(Arc::new(crate::backends::snap::SnapQueryable { core: snap_core.clone() }))
        .with_upgradable(Arc::new(crate::backends::snap::SnapUpgradable { core: snap_core.clone() }))
        .with_metadata_provider(snap_core.clone())
        .build()));

    // --- FLATPAK (Sandbox Containers) ---
    let flatpak_settings = config.backend_settings.get("flatpak").cloned().unwrap_or_default();
    let flatpak_core = Arc::new(crate::backends::flatpak::FlatpakBackendCore::new(executor.duplicate(), flatpak_settings));
    reg.register(Arc::new(BackendCapabilities::builder(flatpak_core.clone())
        .with_installable(Arc::new(crate::backends::flatpak::FlatpakInstallable { core: flatpak_core.clone() }))
        .with_queryable(Arc::new(crate::backends::flatpak::FlatpakQueryable { core: flatpak_core.clone() }))
        .with_upgradable(Arc::new(crate::backends::flatpak::FlatpakUpgradable { core: flatpak_core.clone() }))
        .with_metadata_provider(flatpak_core.clone())
        .build()));

    // ========================================================================
    // 5. LANGUAGE PACKAGE MANAGERS (Explicit Wiring)
    // ========================================================================
    
    // Explicit definitions for each language to ensure unique flags and parsers.
    
    // CARGO (Rust)
    let cargo_core = Arc::new(GenericBackendCore {
        name: "cargo".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "cargo".into(), install_args: vec!["install".into()], remove_args: vec!["uninstall".into()],
            list_args: vec!["install".into(), "--list".into()], list_manual_args: None,
            search_args: vec!["search".into()], upgrade_args: vec!["install".into()], update_args: None,
            repo_add_args: None, repo_remove_args: None, repo_list_args: None, depends_args: None,
            needs_root: false, is_exclusive: true, flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser { 
            installed_fn: |o| crate::parsers::language::parse_installed("cargo", o), 
            search_fn: |_| vec![] 
        }),
    });
    reg.register(Arc::new(BackendCapabilities::builder(cargo_core.clone())
        .with_installable(Arc::new(GenericInstallable { core: cargo_core.clone() }))
        .with_queryable(Arc::new(GenericQueryable { core: cargo_core.clone() }))
        .with_metadata_provider(cargo_core.clone()).build()));

    // NPM (Node.js)
    let npm_core = Arc::new(GenericBackendCore {
        name: "npm".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "npm".into(), install_args: vec!["add".into(), "--global".into()], remove_args: vec!["uninstall".into(), "--global".into()],
            list_args: vec!["list".into(), "--global".into(), "--depth=0".into()], list_manual_args: None,
            search_args: vec!["search".into()], upgrade_args: vec!["update".into(), "-g".into()], update_args: None,
            repo_add_args: None, repo_remove_args: None, repo_list_args: None, depends_args: None,
            needs_root: false, is_exclusive: true, flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser { 
            installed_fn: |o| crate::parsers::language::parse_installed("npm", o), 
            search_fn: |_| vec![] 
        }),
    });
    reg.register(Arc::new(BackendCapabilities::builder(npm_core.clone())
        .with_installable(Arc::new(GenericInstallable { core: npm_core.clone() }))
        .with_queryable(Arc::new(GenericQueryable { core: npm_core.clone() }))
        .with_metadata_provider(npm_core.clone()).build()));

    // PIP (Python)
    let pip_core = Arc::new(GenericBackendCore {
        name: "pip".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "pip".into(), install_args: vec!["install".into()], remove_args: vec!["uninstall".into(), "-y".into()],
            list_args: vec!["list".into(), "--format=json".into()], list_manual_args: None,
            search_args: vec!["search".into()], upgrade_args: vec!["install".into(), "--upgrade".into()], update_args: None,
            repo_add_args: None, repo_remove_args: None, repo_list_args: None, depends_args: None,
            needs_root: false, is_exclusive: false, flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser { 
            installed_fn: |o| crate::parsers::language::parse_installed("pip", o), 
            search_fn: |_| vec![] 
        }),
    });
    reg.register(Arc::new(BackendCapabilities::builder(pip_core.clone())
        .with_installable(Arc::new(GenericInstallable { core: pip_core.clone() }))
        .with_queryable(Arc::new(GenericQueryable { core: pip_core.clone() }))
        .with_metadata_provider(pip_core.clone()).build()));

    // PIPX (Isolated Python Apps)
    let pipx_core = Arc::new(GenericBackendCore {
        name: "pipx".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "pipx".into(), install_args: vec!["install".into()], remove_args: vec!["uninstall".into()],
            list_args: vec!["list".into(), "--json".into()], list_manual_args: None,
            search_args: vec!["search".into()], upgrade_args: vec!["upgrade-all".into()], update_args: None,
            repo_add_args: None, repo_remove_args: None, repo_list_args: None, depends_args: None,
            needs_root: false, is_exclusive: false, flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser { 
            installed_fn: |o| crate::parsers::language::parse_installed("pipx", o), 
            search_fn: |_| vec![] 
        }),
    });
    reg.register(Arc::new(BackendCapabilities::builder(pipx_core.clone())
        .with_installable(Arc::new(GenericInstallable { core: pipx_core.clone() }))
        .with_queryable(Arc::new(GenericQueryable { core: pipx_core.clone() }))
        .with_metadata_provider(pipx_core.clone()).build()));

    // YARN (Node.js Alternate)
    let yarn_core = Arc::new(GenericBackendCore {
        name: "yarn".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "yarn".into(), install_args: vec!["global".into(), "add".into()], remove_args: vec!["global".into(), "remove".into()],
            list_args: vec!["global".into(), "list".into()], list_manual_args: None,
            search_args: vec!["search".into()], upgrade_args: vec!["global".into(), "upgrade".into()], update_args: None,
            repo_add_args: None, repo_remove_args: None, repo_list_args: None, depends_args: None,
            needs_root: false, is_exclusive: false, flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser { 
            installed_fn: |o| crate::parsers::language::parse_installed("yarn", o), 
            search_fn: |_| vec![] 
        }),
    });
    reg.register(Arc::new(BackendCapabilities::builder(yarn_core.clone())
        .with_installable(Arc::new(GenericInstallable { core: yarn_core.clone() }))
        .with_queryable(Arc::new(GenericQueryable { core: yarn_core.clone() }))
        .with_metadata_provider(yarn_core.clone()).build()));

    // GEM (Ruby)
    let gem_core = Arc::new(GenericBackendCore {
        name: "gem".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "gem".into(), install_args: vec!["install".into()], remove_args: vec!["uninstall".into()],
            list_args: vec!["list".into(), "--local".into()], list_manual_args: None,
            search_args: vec!["search".into()], upgrade_args: vec!["update".into()], update_args: None,
            repo_add_args: Some(vec!["sources".into(), "-a".into(), "{url}".into()]),
            repo_remove_args: Some(vec!["sources".into(), "-r".into(), "{url}".into()]),
            repo_list_args: Some(vec!["sources".into()]), depends_args: None,
            needs_root: false, is_exclusive: false, flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser { 
            installed_fn: |o| crate::parsers::language::parse_installed("gem", o), 
            search_fn: |o| crate::parsers::language::parse_search("gem", o) 
        }),
    });
    reg.register(Arc::new(BackendCapabilities::builder(gem_core.clone())
        .with_installable(Arc::new(GenericInstallable { core: gem_core.clone() }))
        .with_queryable(Arc::new(GenericQueryable { core: gem_core.clone() }))
        .with_repo_manager(Arc::new(GenericRepoManager { core: gem_core.clone() }))
        .with_metadata_provider(gem_core.clone()).build()));

    // BUN (The High-Performance Runtime)
    let bun_core = Arc::new(GenericBackendCore {
        name: "bun".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "bun".into(), install_args: vec!["add".into(), "-g".into()], remove_args: vec!["remove".into(), "-g".into()],
            list_args: vec!["pm".into(), "ls".into(), "-g".into()], list_manual_args: None,
            search_args: vec!["search".into()], upgrade_args: vec!["upgrade".into()], update_args: None,
            repo_add_args: None, repo_remove_args: None, repo_list_args: None, depends_args: None,
            needs_root: false, is_exclusive: false, flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser { 
            installed_fn: |o| crate::parsers::language::parse_installed("bun", o), 
            search_fn: |_| vec![] 
        }),
    });
    reg.register(Arc::new(BackendCapabilities::builder(bun_core.clone())
        .with_installable(Arc::new(GenericInstallable { core: bun_core.clone() }))
        .with_queryable(Arc::new(GenericQueryable { core: bun_core.clone() }))
        .with_metadata_provider(bun_core.clone()).build()));

    // PNPM (Disk-efficient NPM)
    let pnpm_core = Arc::new(GenericBackendCore {
        name: "pnpm".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "pnpm".into(), install_args: vec!["add".into(), "-g".into()], remove_args: vec!["remove".into(), "-g".into()],
            list_args: vec!["list".into(), "-g".into()], list_manual_args: None,
            search_args: vec!["search".into()], upgrade_args: vec!["update".into(), "-g".into()], update_args: None,
            repo_add_args: None, repo_remove_args: None, repo_list_args: None, depends_args: None,
            needs_root: false, is_exclusive: false, flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser { 
            installed_fn: |o| crate::parsers::language::parse_installed("pnpm", o), 
            search_fn: |_| vec![] 
        }),
    });
    reg.register(Arc::new(BackendCapabilities::builder(pnpm_core.clone())
        .with_installable(Arc::new(GenericInstallable { core: pnpm_core.clone() }))
        .with_queryable(Arc::new(GenericQueryable { core: pnpm_core.clone() }))
        .with_metadata_provider(pnpm_core.clone()).build()));

    reg
}