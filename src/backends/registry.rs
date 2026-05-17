use crate::core::{
    BackendCore, Installable, Queryable, Searchable, Upgradable, RepoManager,
    BackendCapabilities, CommandExecutor, Package, PackageSpec, Result
};
use crate::config::Config;
use crate::app::LuaHooks;
use crate::backends::generic::{
    GenericBackendCore, GenericInstallable, GenericSearchable, GenericQueryable,
    GenericUpgradable, GenericRepoManager, ManagerConfig
};
use crate::parsers::{apt, pacman, dnf, brew, language, windows, macos, LambdaParser};
use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;

/// Central registry for all package management backends.
/// Updated for LiNix v3.5.0 to use the new ISP-compliant capability system.
pub struct BackendRegistry {
    backends: HashMap<String, Arc<BackendCapabilities>>,
}

impl BackendRegistry {
    pub fn new() -> Self { 
        Self { backends: HashMap::new() } 
    }
    
    pub fn register(&mut self, backend: Arc<BackendCapabilities>) {
        self.backends.insert(backend.core().name().to_string(), backend);
    }

    pub fn get(&self, name: &str) -> Option<Arc<BackendCapabilities>> {
        self.backends.get(name).cloned()
    }

    pub fn available(&self) -> Vec<Arc<BackendCapabilities>> {
        self.backends.values()
            .filter(|b| b.core().is_available())
            .cloned()
            .collect()
    }

    pub fn all(&self) -> Vec<Arc<BackendCapabilities>> {
        self.backends.values().cloned().collect()
    }

    pub fn get_filtered(&self, enabled: &[String]) -> Vec<Arc<BackendCapabilities>> {
        self.available().into_iter()
            .filter(|b| enabled.contains(&b.core().name().to_string()))
            .collect()
    }
    
    /// Returns all backends that support a specific capability.
    pub fn get_installable(&self) -> Vec<Arc<BackendCapabilities>> {
        self.available().into_iter()
            .filter(|b| b.is_installable())
            .collect()
    }
    
    /// Returns all backends that support searching.
    pub fn get_searchable(&self) -> Vec<Arc<BackendCapabilities>> {
        self.available().into_iter()
            .filter(|b| b.is_searchable())
            .collect()
    }
    
    /// Returns all backends that support querying.
    pub fn get_queryable(&self) -> Vec<Arc<BackendCapabilities>> {
        self.available().into_iter()
            .filter(|b| b.is_queryable())
            .collect()
    }
    
    /// Returns all backends that support upgrades.
    pub fn get_upgradable(&self) -> Vec<Arc<BackendCapabilities>> {
        self.available().into_iter()
            .filter(|b| b.is_upgradable())
            .collect()
    }
    
    /// Returns all backends that support repository management.
    pub fn get_repo_managers(&self) -> Vec<Arc<BackendCapabilities>> {
        self.available().into_iter()
            .filter(|b| b.is_repo_manager())
            .collect()
    }
}

/// Helper trait to convert legacy backends to new capability system.
pub trait IntoBackendCapabilities {
    fn into_capabilities(self) -> BackendCapabilities;
}

/// The master wiring function for LiNix.
pub async fn create_default_registry(executor: CommandExecutor, config: &Config, hooks: Arc<LuaHooks>) -> BackendRegistry {
    let mut reg = BackendRegistry::new();

    // --- 1. SYSTEM PACKAGE MANAGERS (Linux) ---
    #[cfg(target_os = "linux")] {
        // APT (Debian/Ubuntu) - Includes Point 7 Repo Management
        let apt_config = ManagerConfig {
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
            is_exclusive: true,
            flag_map: HashMap::new(),
        };
        
        let apt_parser = Arc::new(LambdaParser { installed_fn: apt::parse_list, search_fn: apt::parse_search });
        
        let apt_core = Arc::new(GenericBackendCore {
            name: "apt".to_string(),
            executor: executor.clone(),
            config: apt_config,
            parser: apt_parser,
        });
        
        let apt_installable = Arc::new(GenericInstallable { core: apt_core.clone() });
        let apt_searchable = Arc::new(GenericSearchable { core: apt_core.clone() });
        let apt_queryable = Arc::new(GenericQueryable { core: apt_core.clone() });
        let apt_upgradable = Arc::new(GenericUpgradable { core: apt_core.clone() });
        let apt_repo = Arc::new(GenericRepoManager { core: apt_core.clone() });
        
        let apt_caps = BackendCapabilities::builder(apt_core)
            .with_installable(apt_installable)
            .with_searchable(apt_searchable)
            .with_queryable(apt_queryable)
            .with_upgradable(apt_upgradable)
            .with_repo_manager(apt_repo)
            .build();
        
        reg.register(Arc::new(apt_caps));
        
        // Pacman (Arch)
        let pacman_config = ManagerConfig {
            name: "pacman".into(),
            install_args: vec!["-S".into(), "--noconfirm".into(), "--needed".into()],
            remove_args: vec!["-Rs".into(), "--noconfirm".into()],
            list_args: vec!["-Q".into()],
            list_manual_args: Some(vec!["-Qe".into()]),
            search_args: vec!["-Ss".into()],
            upgrade_args: vec!["-Syu".into(), "--noconfirm".into()],
            update_args: Some(vec!["-Sy".into()]),
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            is_exclusive: true,
            flag_map: HashMap::new(),
        };
        
        let pacman_parser = Arc::new(LambdaParser { installed_fn: pacman::parse_list, search_fn: pacman::parse_search });
        
        let pacman_core = Arc::new(GenericBackendCore {
            name: "pacman".to_string(),
            executor: executor.clone(),
            config: pacman_config,
            parser: pacman_parser,
        });
        
        let pacman_installable = Arc::new(GenericInstallable { core: pacman_core.clone() });
        let pacman_searchable = Arc::new(GenericSearchable { core: pacman_core.clone() });
        let pacman_queryable = Arc::new(GenericQueryable { core: pacman_core.clone() });
        let pacman_upgradable = Arc::new(GenericUpgradable { core: pacman_core.clone() });
        
        let pacman_caps = BackendCapabilities::builder(pacman_core)
            .with_installable(pacman_installable)
            .with_searchable(pacman_searchable)
            .with_queryable(pacman_queryable)
            .with_upgradable(pacman_upgradable)
            .build();
        
        reg.register(Arc::new(pacman_caps));
        
        // DNF (Fedora/RHEL)
        let dnf_config = ManagerConfig {
            name: "dnf".into(),
            install_args: vec!["install".into(), "-y".into()],
            remove_args: vec!["remove".into(), "-y".into()],
            list_args: vec!["list".into(), "installed".into()],
            list_manual_args: Some(vec!["list".into(), "installed".into()]),
            search_args: vec!["search".into()],
            upgrade_args: vec!["upgrade".into(), "-y".into()],
            update_args: Some(vec!["makecache".into()]),
            repo_add_args: Some(vec!["config-manager".into(), "--add-repo".into(), "{url}".into()]),
            repo_remove_args: Some(vec!["config-manager".into(), "--remove-repo".into(), "{name}".into()]),
            repo_list_args: Some(vec!["repolist".into()]),
            is_exclusive: true,
            flag_map: HashMap::new(),
        };
        
        let dnf_parser = Arc::new(LambdaParser { installed_fn: dnf::parse_rpm_qa, search_fn: dnf::parse_dnf_search });
        
        let dnf_core = Arc::new(GenericBackendCore {
            name: "dnf".to_string(),
            executor: executor.clone(),
            config: dnf_config,
            parser: dnf_parser,
        });
        
        let dnf_installable = Arc::new(GenericInstallable { core: dnf_core.clone() });
        let dnf_searchable = Arc::new(GenericSearchable { core: dnf_core.clone() });
        let dnf_queryable = Arc::new(GenericQueryable { core: dnf_core.clone() });
        let dnf_upgradable = Arc::new(GenericUpgradable { core: dnf_core.clone() });
        let dnf_repo = Arc::new(GenericRepoManager { core: dnf_core.clone() });
        
        let dnf_caps = BackendCapabilities::builder(dnf_core)
            .with_installable(dnf_installable)
            .with_searchable(dnf_searchable)
            .with_queryable(dnf_queryable)
            .with_upgradable(dnf_upgradable)
            .with_repo_manager(dnf_repo)
            .build();
        
        reg.register(Arc::new(dnf_caps));
    }

    // --- 2. WINDOWS PACKAGE MANAGERS ---
    #[cfg(target_os = "windows")] {
        // Winget
        let winget_config = ManagerConfig {
            name: "winget".into(),
            install_args: vec!["install".into(), "--silent".into()],
            remove_args: vec!["uninstall".into(), "--silent".into()],
            list_args: vec!["list".into()],
            list_manual_args: None,
            search_args: vec!["search".into()],
            upgrade_args: vec!["upgrade".into(), "--all".into(), "--silent".into()],
            update_args: Some(vec!["source".into(), "update".into()]),
            repo_add_args: Some(vec!["source".into(), "add".into(), "--name".into(), "{name}".into(), "--arg".into(), "{url}".into()]),
            repo_remove_args: Some(vec!["source".into(), "remove".into(), "--name".into(), "{name}".into()]),
            repo_list_args: Some(vec!["source".into(), "list".into()]),
            is_exclusive: false,
            flag_map: HashMap::new(),
        };
        
        let winget_parser = Arc::new(LambdaParser { 
            installed_fn: |o| windows::parse_installed("winget", o), 
            search_fn: |o| windows::parse_search("winget", o) 
        });
        
        let winget_core = Arc::new(GenericBackendCore {
            name: "winget".to_string(),
            executor: executor.clone(),
            config: winget_config,
            parser: winget_parser,
        });
        
        let winget_installable = Arc::new(GenericInstallable { core: winget_core.clone() });
        let winget_searchable = Arc::new(GenericSearchable { core: winget_core.clone() });
        let winget_queryable = Arc::new(GenericQueryable { core: winget_core.clone() });
        let winget_upgradable = Arc::new(GenericUpgradable { core: winget_core.clone() });
        let winget_repo = Arc::new(GenericRepoManager { core: winget_core.clone() });
        
        let winget_caps = BackendCapabilities::builder(winget_core)
            .with_installable(winget_installable)
            .with_searchable(winget_searchable)
            .with_queryable(winget_queryable)
            .with_upgradable(winget_upgradable)
            .with_repo_manager(winget_repo)
            .build();
        
        reg.register(Arc::new(winget_caps));
        
        // Scoop
        let scoop_config = ManagerConfig {
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
            is_exclusive: false,
            flag_map: HashMap::new(),
        };
        
        let scoop_parser = Arc::new(LambdaParser { 
            installed_fn: |o| windows::parse_installed("scoop", o), 
            search_fn: |o| windows::parse_search("scoop", o) 
        });
        
        let scoop_core = Arc::new(GenericBackendCore {
            name: "scoop".to_string(),
            executor: executor.clone(),
            config: scoop_config,
            parser: scoop_parser,
        });
        
        let scoop_installable = Arc::new(GenericInstallable { core: scoop_core.clone() });
        let scoop_searchable = Arc::new(GenericSearchable { core: scoop_core.clone() });
        let scoop_queryable = Arc::new(GenericQueryable { core: scoop_core.clone() });
        let scoop_upgradable = Arc::new(GenericUpgradable { core: scoop_core.clone() });
        let scoop_repo = Arc::new(GenericRepoManager { core: scoop_core.clone() });
        
        let scoop_caps = BackendCapabilities::builder(scoop_core)
            .with_installable(scoop_installable)
            .with_searchable(scoop_searchable)
            .with_queryable(scoop_queryable)
            .with_upgradable(scoop_upgradable)
            .with_repo_manager(scoop_repo)
            .build();
        
        reg.register(Arc::new(scoop_caps));
    }

    // --- 3. CROSS-PLATFORM MANAGERS ---
    // Homebrew (Includes Point 7 Taps)
    let brew_config = ManagerConfig {
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
        is_exclusive: false,
        flag_map: HashMap::new(),
    };
    
    let brew_parser = Arc::new(LambdaParser { installed_fn: brew::parse_list, search_fn: brew::parse_search });
    
    let brew_core = Arc::new(GenericBackendCore {
        name: "brew".to_string(),
        executor: executor.clone(),
        config: brew_config,
        parser: brew_parser,
    });
    
    let brew_installable = Arc::new(GenericInstallable { core: brew_core.clone() });
    let brew_searchable = Arc::new(GenericSearchable { core: brew_core.clone() });
    let brew_queryable = Arc::new(GenericQueryable { core: brew_core.clone() });
    let brew_upgradable = Arc::new(GenericUpgradable { core: brew_core.clone() });
    let brew_repo = Arc::new(GenericRepoManager { core: brew_core.clone() });
    
    let brew_caps = BackendCapabilities::builder(brew_core)
        .with_installable(brew_installable)
        .with_searchable(brew_searchable)
        .with_queryable(brew_queryable)
        .with_upgradable(brew_upgradable)
        .with_repo_manager(brew_repo)
        .build();
    
    reg.register(Arc::new(brew_caps));

    // Language Managers (Cargo, NPM, Pip, Pipx, Yarn, Gem, Go, Bun, Pnpm)
    let languages = vec![
        ("cargo", vec!["install".into()], vec!["uninstall".into()], vec!["install".into(), "--list".into()], None, vec!["search".into()], vec!["install-update".into(), "-a".into()], true),
        ("npm", vec!["install".into(), "-g".into()], vec!["uninstall".into(), "-g".into()], vec!["list".into(), "-g".into(), "--depth=0".into(), "--json".into()], None, vec!["search".into()], vec!["upgrade".into()], true),
        ("pip", vec!["install".into()], vec!["uninstall".into()], vec!["list".into(), "--format=json".into()], None, vec!["search".into()], vec!["upgrade".into()], false),
        ("pipx", vec!["install".into()], vec!["uninstall".into()], vec!["list".into(), "--json".into()], None, vec!["search".into()], vec!["upgrade".into()], false),
        ("yarn", vec!["global".into(), "add".into()], vec!["global".into(), "remove".into()], vec!["global".into(), "list".into()], None, vec!["search".into()], vec!["upgrade".into()], false),
        ("gem", vec!["install".into()], vec!["uninstall".into()], vec!["list".into(), "--local".into()], None, vec!["search".into()], vec!["update".into()], false),
        ("go", vec!["install".into()], vec!["clean".into(), "-i".into()], vec!["list".into()], None, vec!["search".into()], vec!["upgrade".into()], false),
        ("bun", vec!["add".into(), "-g".into()], vec!["remove".into(), "-g".into()], vec!["list".into(), "-g".into()], None, vec!["search".into()], vec!["upgrade".into()], false),
        ("pnpm", vec!["add".into(), "-g".into()], vec!["remove".into(), "-g".into()], vec!["list".into(), "-g".into(), "--json".into()], None, vec!["search".into()], vec!["upgrade".into()], true),
    ];
    
    for (name, install_args, remove_args, list_args, list_manual_args, search_args, upgrade_args, is_exclusive) in languages {
        let lang_config = ManagerConfig {
            name: name.into(),
            install_args,
            remove_args,
            list_args,
            list_manual_args,
            search_args,
            upgrade_args,
            update_args: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            is_exclusive,
            flag_map: HashMap::new(),
        };
        
        let lang_parser = Arc::new(LambdaParser { 
            installed_fn: move |o| language::parse_installed(name, o), 
            search_fn: move |o| language::parse_search(name, o) 
        });
        
        let lang_core = Arc::new(GenericBackendCore {
            name: name.to_string(),
            executor: executor.clone(),
            config: lang_config,
            parser: lang_parser,
        });
        
        let lang_installable = Arc::new(GenericInstallable { core: lang_core.clone() });
        let lang_searchable = Arc::new(GenericSearchable { core: lang_core.clone() });
        let lang_queryable = Arc::new(GenericQueryable { core: lang_core.clone() });
        let lang_upgradable = Arc::new(GenericUpgradable { core: lang_core.clone() });
        
        let lang_caps = BackendCapabilities::builder(lang_core)
            .with_installable(lang_installable)
            .with_searchable(lang_searchable)
            .with_queryable(lang_queryable)
            .with_upgradable(lang_upgradable)
            .build();
        
        reg.register(Arc::new(lang_caps));
    }

    // --- 4. RESOURCE & SPECIALIZED MANAGERS ---
    // Btrfs
    let btrfs_core = Arc::new(crate::backends::btrfs::BtrfsBackendCore {
        executor: executor.clone(),
        name: "btrfs".to_string(),
    });
    let btrfs_installable = Arc::new(crate::backends::btrfs::BtrfsInstallable { core: btrfs_core.clone() });
    let btrfs_queryable = Arc::new(crate::backends::btrfs::BtrfsQueryable { core: btrfs_core.clone() });
    let btrfs_caps = BackendCapabilities::builder(btrfs_core)
        .with_installable(btrfs_installable)
        .with_queryable(btrfs_queryable)
        .build();
    reg.register(Arc::new(btrfs_caps));
    
    // GitHub
    let github_core = Arc::new(crate::backends::github::GithubBackendCore {
        executor: executor.clone(),
        name: "github".to_string(),
    });
    let github_installable = Arc::new(crate::backends::github::GithubInstallable { core: github_core.clone() });
    let github_queryable = Arc::new(crate::backends::github::GithubQueryable { core: github_core.clone() });
    let github_caps = BackendCapabilities::builder(github_core)
        .with_installable(github_installable)
        .with_queryable(github_queryable)
        .build();
    reg.register(Arc::new(github_caps));
    
    // Web
    let web_core = Arc::new(crate::backends::web::WebBackendCore {
        executor: executor.clone(),
        name: "web".to_string(),
    });
    let web_installable = Arc::new(crate::backends::web::WebInstallable { core: web_core.clone() });
    let web_queryable = Arc::new(crate::backends::web::WebQueryable { core: web_core.clone() });
    let web_caps = BackendCapabilities::builder(web_core)
        .with_installable(web_installable)
        .with_queryable(web_queryable)
        .build();
    reg.register(Arc::new(web_caps));
    
    // Link
    let link_core = Arc::new(crate::backends::link::LinkBackendCore {
        executor: executor.clone(),
        name: "link".to_string(),
        config: Arc::new(config.clone()),
        hooks: hooks.clone(),
    });
    let link_installable = Arc::new(crate::backends::link::LinkInstallable { core: link_core.clone() });
    let link_caps = BackendCapabilities::builder(link_core)
        .with_installable(link_installable)
        .build();
    reg.register(Arc::new(link_caps));
    
    // Nix
    let nix_core = Arc::new(crate::backends::nix::NixBackendCore {
        executor: executor.clone(),
        name: "nix".to_string(),
    });
    let nix_installable = Arc::new(crate::backends::nix::NixInstallable { core: nix_core.clone() });
    let nix_queryable = Arc::new(crate::backends::nix::NixQueryable { core: nix_core.clone() });
    let nix_upgradable = Arc::new(crate::backends::nix::NixUpgradable { core: nix_core.clone() });
    let nix_caps = BackendCapabilities::builder(nix_core)
        .with_installable(nix_installable)
        .with_queryable(nix_queryable)
        .with_upgradable(nix_upgradable)
        .build();
    reg.register(Arc::new(nix_caps));
    
    // VSCode
    let vscode_core = Arc::new(crate::backends::vscode::VscodeBackendCore {
        executor: executor.clone(),
        name: "vscode".to_string(),
    });
    let vscode_installable = Arc::new(crate::backends::vscode::VscodeInstallable { core: vscode_core.clone() });
    let vscode_queryable = Arc::new(crate::backends::vscode::VscodeQueryable { core: vscode_core.clone() });
    let vscode_searchable = Arc::new(crate::backends::vscode::VscodeSearchable { core: vscode_core.clone() });
    let vscode_caps = BackendCapabilities::builder(vscode_core)
        .with_installable(vscode_installable)
        .with_queryable(vscode_queryable)
        .with_searchable(vscode_searchable)
        .build();
    reg.register(Arc::new(vscode_caps));
    
    // Mise
    let mise_core = Arc::new(crate::backends::mise::MiseBackendCore {
        executor: executor.clone(),
        name: "mise".to_string(),
    });
    let mise_installable = Arc::new(crate::backends::mise::MiseInstallable { core: mise_core.clone() });
    let mise_queryable = Arc::new(crate::backends::mise::MiseQueryable { core: mise_core.clone() });
    let mise_upgradable = Arc::new(crate::backends::mise::MiseUpgradable { core: mise_core.clone() });
    let mise_caps = BackendCapabilities::builder(mise_core)
        .with_installable(mise_installable)
        .with_queryable(mise_queryable)
        .with_upgradable(mise_upgradable)
        .build();
    reg.register(Arc::new(mise_caps));
    
    // Emacs
    let emacs_core = Arc::new(crate::backends::emacs::EmacsBackendCore {
        executor: executor.clone(),
        name: "emacs".to_string(),
    });
    let emacs_installable = Arc::new(crate::backends::emacs::EmacsInstallable { core: emacs_core.clone() });
    let emacs_queryable = Arc::new(crate::backends::emacs::EmacsQueryable { core: emacs_core.clone() });
    let emacs_caps = BackendCapabilities::builder(emacs_core)
        .with_installable(emacs_installable)
        .with_queryable(emacs_queryable)
        .build();
    reg.register(Arc::new(emacs_caps));
    
    // Service
    let service_core = Arc::new(crate::backends::service::ServiceBackendCore {
        executor: executor.clone(),
        name: "service".to_string(),
    });
    let service_installable = Arc::new(crate::backends::service::ServiceInstallable { core: service_core.clone() });
    let service_queryable = Arc::new(crate::backends::service::ServiceQueryable { core: service_core.clone() });
    let service_caps = BackendCapabilities::builder(service_core)
        .with_installable(service_installable)
        .with_queryable(service_queryable)
        .build();
    reg.register(Arc::new(service_caps));
    
    // AppImage
    let appimage_core = Arc::new(crate::backends::appimage::AppImageBackendCore {
        executor: executor.clone(),
        name: "appimage".to_string(),
    });
    let appimage_installable = Arc::new(crate::backends::appimage::AppImageInstallable { core: appimage_core.clone() });
    let appimage_queryable = Arc::new(crate::backends::appimage::AppImageQueryable { core: appimage_core.clone() });
    let appimage_caps = BackendCapabilities::builder(appimage_core)
        .with_installable(appimage_installable)
        .with_queryable(appimage_queryable)
        .build();
    reg.register(Arc::new(appimage_caps));
    
    // Snap
    let snap_core = Arc::new(crate::backends::snap::SnapBackendCore {
        executor: executor.clone(),
        name: "snap".to_string(),
    });
    let snap_installable = Arc::new(crate::backends::snap::SnapInstallable { core: snap_core.clone() });
    let snap_queryable = Arc::new(crate::backends::snap::SnapQueryable { core: snap_core.clone() });
    let snap_upgradable = Arc::new(crate::backends::snap::SnapUpgradable { core: snap_core.clone() });
    let snap_caps = BackendCapabilities::builder(snap_core)
        .with_installable(snap_installable)
        .with_queryable(snap_queryable)
        .with_upgradable(snap_upgradable)
        .build();
    reg.register(Arc::new(snap_caps));
    
    // Flatpak
    let flatpak_config = config.backend_settings.get("flatpak").cloned().unwrap_or_default();
    let flatpak_core = Arc::new(crate::backends::flatpak::FlatpakBackendCore {
        executor: executor.clone(),
        name: "flatpak".to_string(),
        settings: flatpak_config,
    });
    let flatpak_installable = Arc::new(crate::backends::flatpak::FlatpakInstallable { core: flatpak_core.clone() });
    let flatpak_queryable = Arc::new(crate::backends::flatpak::FlatpakQueryable { core: flatpak_core.clone() });
    let flatpak_upgradable = Arc::new(crate::backends::flatpak::FlatpakUpgradable { core: flatpak_core.clone() });
    let flatpak_caps = BackendCapabilities::builder(flatpak_core)
        .with_installable(flatpak_installable)
        .with_queryable(flatpak_queryable)
        .with_upgradable(flatpak_upgradable)
        .build();
    reg.register(Arc::new(flatpak_caps));

    reg
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_registry_creation() {
        let config = Config::default();
        let executor = CommandExecutor::new(false, false);
        let hooks = Arc::new(LuaHooks::new(&config).unwrap());
        
        let rt = tokio::runtime::Runtime::new().unwrap();
        let registry = rt.block_on(create_default_registry(executor, &config, hooks));
        
        assert!(!registry.all().is_empty());
        
        // Test getting specific backends
        if let Some(apt) = registry.get("apt") {
            assert_eq!(apt.core().name(), "apt");
        }
        
        // Test capability filtering
        let installable = registry.get_installable();
        assert!(!installable.is_empty());
        
        let searchable = registry.get_searchable();
        // At minimum, apt should be searchable
        assert!(searchable.iter().any(|b| b.core().name() == "apt"));
    }
}