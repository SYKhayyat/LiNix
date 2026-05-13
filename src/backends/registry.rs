use crate::core::{Backend, CommandExecutor};
use crate::config::Config;
use crate::app::LuaHooks;
use crate::backends::generic::{GenericManager, ManagerConfig};
use crate::parsers::{apt, pacman, dnf, brew, language, windows, macos, LambdaParser};
use std::collections::HashMap;
use std::sync::Arc;

/// Central registry for all package management backends.
/// Updated for LiNix v3.3.0 to support Resource Backends (BTRFS) 
/// and Generic Repo Management (Point 1 & 7).
pub struct BackendRegistry {
    backends: HashMap<String, Arc<dyn Backend>>,
}

impl BackendRegistry {
    pub fn new() -> Self { 
        Self { backends: HashMap::new() } 
    }
    
    pub fn register(&mut self, backend: Arc<dyn Backend>) {
        self.backends.insert(backend.name().to_string(), backend);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Backend>> {
        self.backends.get(name).cloned()
    }

    pub fn available(&self) -> Vec<Arc<dyn Backend>> {
        self.backends.values()
            .filter(|b| b.is_available())
            .cloned()
            .collect()
    }

    pub fn all(&self) -> Vec<Arc<dyn Backend>> {
        self.backends.values().cloned().collect()
    }

    pub fn get_filtered(&self, enabled: &[String]) -> Vec<Arc<dyn Backend>> {
        self.available().into_iter()
            .filter(|b| enabled.contains(&b.name().to_string()))
            .collect()
    }
}

/// The master wiring function for LiNix.
pub async fn create_default_registry(executor: CommandExecutor, _config: &Config, hooks: Arc<LuaHooks>) -> BackendRegistry {
    let mut reg = BackendRegistry::new();

    // --- 1. SYSTEM PACKAGE MANAGERS (Linux) ---
    #[cfg(target_os = "linux")] {
        // APT (Debian/Ubuntu) - Includes Point 7 Repo Management
        reg.register(Arc::new(GenericManager {
            executor: executor.clone(),
            parser: Arc::new(LambdaParser { installed_fn: apt::parse_list, search_fn: apt::parse_search }),
            config: ManagerConfig {
                name: "apt".into(),
                install_args: vec!["install".into(), "-y".into()],
                remove_args: vec!["purge".into(), "-y".into()],
                list_args: vec!["dpkg-query".into(), "-W".into(), "-f=${Package} ${Version}\\n".into()],
                list_manual_args: Some(vec!["apt-mark".into(), "showmanual".into()]),
                search_args: vec!["apt-cache".into(), "search".into()],
                upgrade_args: vec!["dist-upgrade".into(), "-y".into()],
                update_args: Some(vec!["update".into()]),
                repo_add_args: Some(vec!["add-壓-repository".into(), "-y".into(), "{url}".into()]),
                repo_remove_args: Some(vec!["add-apt-repository".into(), "--remove".into(), "-y".into(), "{name}".into()]),
                repo_list_args: None, 
                is_exclusive: true,
                flag_map: HashMap::new(),
            }
        }));

        // Pacman (Arch)
        reg.register(Arc::new(GenericManager {
            executor: executor.clone(),
            parser: Arc::new(LambdaParser { installed_fn: pacman::parse_list, search_fn: pacman::parse_search }),
            config: ManagerConfig {
                name: "pacman".into(),
                install_args: vec!["-S".into(), "--noconfirm".into(), "--needed".into()],
                remove_args: vec!["-Rs".into(), "--noconfirm".into()],
                list_args: vec!["-Q".into()],
                list_manual_args: Some(vec!["-Qe".into()]),
                search_args: vec!["-Ss".into()],
                upgrade_args: vec!["-Syu".into(), "--noconfirm".into()],
                update_args: Some(vec!["-Sy".into()]),
                repo_add_args: None, // Pacman repos are managed via mirrorlist files
                repo_remove_args: None,
                repo_list_args: None,
                is_exclusive: true,
                flag_map: HashMap::new(),
            }
        }));
    }

    // --- 2. WINDOWS PACKAGE MANAGERS ---
    #[cfg(target_os = "windows")] {
        // Scoop (Includes Point 7 Buckets)
        reg.register(Arc::new(GenericManager {
            executor: executor.clone(),
            parser: Arc::new(LambdaParser { installed_fn: |o| windows::parse_installed("scoop", o), search_fn: |o| windows::parse_search("scoop", o) }),
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
                is_exclusive: false,
                flag_map: HashMap::new(),
            }
        }));
    }

    // --- 3. CROSS-PLATFORM MANAGERS ---
    // Homebrew (Includes Point 7 Taps)
    reg.register(Arc::new(GenericManager {
        executor: executor.clone(),
        parser: Arc::new(LambdaParser { installed_fn: brew::parse_list, search_fn: brew::parse_search }),
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
            is_exclusive: false,
            flag_map: HashMap::new(),
        }
    }));

    // Language Managers (Cargo, NPM, Pip, etc.)
    let languages = vec!["cargo", "npm", "pip", "pipx", "yarn", "gem", "go", "bun", "pnpm"];
    for name in languages {
        reg.register(Arc::new(GenericManager {
            executor: executor.clone(),
            parser: Arc::new(LambdaParser { 
                installed_fn: move |o| language::parse_installed(name, o), 
                search_fn: move |o| language::parse_search(name, o) 
            }),
            config: ManagerConfig {
                name: name.into(),
                install_args: match name { "npm" => vec!["install".into(), "-g".into()], "cargo" => vec!["install".into()], _ => vec!["install".into()] },
                remove_args: match name { "npm" => vec!["uninstall".into(), "-g".into()], _ => vec!["uninstall".into()] },
                list_args: match name { "cargo" => vec!["install".into(), "--list".into()], "npm" => vec!["list".into(), "-g".into(), "--depth=0".into(), "--json".into()], "pip" => vec!["list".into(), "--format=json".into()], _ => vec!["list".into()] },
                list_manual_args: None,
                search_args: vec!["search".into()],
                upgrade_args: match name { "cargo" => vec!["install-update".into(), "-a".into()], _ => vec!["upgrade".into()] },
                update_args: None,
                repo_add_args: None,
                repo_remove_args: None,
                repo_list_args: None,
                is_exclusive: matches!(name, "cargo" | "npm"),
                flag_map: HashMap::new(),
            }
        }));
    }

    // --- 4. RESOURCE & SPECIALIZED MANAGERS ---
    reg.register(Arc::new(super::btrfs::BtrfsManager::new(executor.clone()))); // Point 1
    reg.register(Arc::new(super::github::GithubManager::new(executor.clone())));
    reg.register(Arc::new(super::web::WebManager::new(executor.clone())));
    reg.register(Arc::new(super::link::LinkManager::new(executor.clone(), hooks.clone())));
    reg.register(Arc::new(super::nix::NixManager::new(executor.clone())));
    reg.register(Arc::new(super::vscode::VscodeManager::new(executor.clone())));
    reg.register(Arc::new(super::mise::MiseManager::new(executor.clone())));
    reg.register(Arc::new(super::service::ServiceManager::new(executor.clone())));
    reg.register(Arc::new(super::appimage::AppImageManager::new(executor.clone())));
    reg.register(Arc::new(super::snap::SnapManager::new(executor.clone())));
    reg.register(Arc::new(super::flatpak::FlatpakManager::new(executor.clone(), HashMap::new())));

    reg
}