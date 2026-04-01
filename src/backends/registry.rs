use crate::core::PackageManager;
use crate::config::Config;
use crate::app::LuaHooks;
use std::collections::HashMap;
use std::sync::Arc;

/// The central registry holding every package manager LiNix can communicate with.
pub struct BackendRegistry {
    managers: HashMap<String, Arc<dyn PackageManager>>,
}

impl BackendRegistry {
    pub fn new() -> Self { 
        Self { managers: HashMap::new() } 
    }

    /// Registers a manager. Once registered, it can be accessed via CLI or Sync.
    pub fn register(&mut self, manager: Arc<dyn PackageManager>) {
        self.managers.insert(manager.name().to_string(), manager);
    }

    /// Fetches a specific manager by name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn PackageManager>> { 
        self.managers.get(name).cloned() 
    }

    /// Returns a list of managers whose binaries are actually present on this system.
    pub fn available(&self) -> Vec<Arc<dyn PackageManager>> {
        self.managers.values()
            .filter(|m| m.is_available())
            .cloned()
            .collect()
    }

    /// Returns every manager known to LiNix, regardless of installation status.
    pub fn all(&self) -> Vec<Arc<dyn PackageManager>> { 
        self.managers.values().cloned().collect() 
    }

    /// Filtered list based on the 'enabled_backends' list in config.toml.
    pub fn get_filtered(&self, names: &[String]) -> Vec<Arc<dyn PackageManager>> {
        names.iter()
            .filter_map(|name| self.get(name))
            .filter(|m| m.is_available())
            .collect()
    }

    /// Simple list of names for available backends.
    pub fn available_names(&self) -> Vec<String> {
        self.managers.iter()
            .filter(|(_, m)| m.is_available())
            .map(|(k, _)| k.clone())
            .collect()
    }
}

/// The master initialization function. 
/// It wires every manager to its config and the command executor.
pub async fn create_default_registry(
    executor: crate::core::CommandExecutor, 
    config: &Config, 
    hooks: Arc<LuaHooks>
) -> BackendRegistry {
    let mut registry = BackendRegistry::new();
    
    // Closure to pull manager-specific settings (e.g., custom registries) from config.toml
    let get_s = |name: &str| config.backend_settings.get(name).cloned();

    // 1. Linux System Package Managers
    #[cfg(target_os = "linux")] {
        registry.register(Arc::new(super::apt::AptManager::new(executor.clone(), get_s("apt"))));
        registry.register(Arc::new(super::pacman::PacmanManager::new(executor.clone(), get_s("pacman"))));
        registry.register(Arc::new(super::dnf::DnfManager::new(executor.clone(), get_s("dnf"))));
        registry.register(Arc::new(super::apk::ApkManager::new(executor.clone(), get_s("apk"))));
        registry.register(Arc::new(super::zypper::ZypperManager::new(executor.clone(), get_s("zypper"))));
        registry.register(Arc::new(super::snap::SnapManager::new(executor.clone(), get_s("snap"))));
        registry.register(Arc::new(super::service::ServiceManager::new(executor.clone(), get_s("service"))));
		registry.register(Arc::new(super::appimage::AppImageManager::new(executor.clone(), get_s("appimage"))));
    }

    // 2. Windows System Package Managers
    #[cfg(target_os = "windows")] {
        registry.register(Arc::new(super::windows::winget::WingetManager::new(executor.clone(), get_s("winget"))));
        registry.register(Arc::new(super::windows::choco::ChocoManager::new(executor.clone(), get_s("choco"))));
        registry.register(Arc::new(super::windows::scoop::ScoopManager::new(executor.clone(), get_s("scoop"))));
    }

    // 3. MacOS Specific (New)
    #[cfg(target_os = "macos")] {
        registry.register(Arc::new(super::mas::MasManager::new(executor.clone(), get_s("mas"))));
    }

    // 4. Cross-Platform App Managers
	registry.register(Arc::new(super::nix::NixManager::new(executor.clone(), get_s("nix"))));
    registry.register(Arc::new(super::brew::BrewManager::new(executor.clone(), get_s("brew"))));
    registry.register(Arc::new(super::flatpak::FlatpakManager::new(executor.clone(), get_s("flatpak"))));

    // 5. Global Language Managers
    registry.register(Arc::new(super::cargo::CargoManager::new(executor.clone(), get_s("cargo"))));
    registry.register(Arc::new(super::npm::NpmManager::new(executor.clone(), get_s("npm"))));
    registry.register(Arc::new(super::pnpm::PnpmManager::new(executor.clone(), get_s("pnpm"))));
    registry.register(Arc::new(super::yarn::YarnManager::new(executor.clone(), get_s("yarn"))));
    registry.register(Arc::new(super::bun::BunManager::new(executor.clone(), get_s("bun"))));
    registry.register(Arc::new(super::pip::PipManager::new(executor.clone(), get_s("pip"))));
    registry.register(Arc::new(super::pipx::PipxManager::new(executor.clone(), get_s("pipx"))));
    registry.register(Arc::new(super::poetry::PoetryManager::new(executor.clone(), get_s("poetry"))));
    registry.register(Arc::new(super::uv::UvManager::new(executor.clone(), get_s("uv"))));
    registry.register(Arc::new(super::go::GoManager::new(executor.clone(), get_s("go"))));
    registry.register(Arc::new(super::composer::ComposerManager::new(executor.clone(), get_s("composer"))));
    registry.register(Arc::new(super::gem::GemManager::new(executor.clone(), get_s("gem"))));

    // 6. Config/Dotfile Specialized Tools
    registry.register(Arc::new(super::vscode::VscodeManager::new(executor.clone(), get_s("vscode"))));
    registry.register(Arc::new(super::emacs::EmacsManager::new(executor.clone(), get_s("emacs"))));
    registry.register(Arc::new(super::mise::MiseManager::new(executor.clone(), get_s("mise"))));
    registry.register(Arc::new(super::github::GithubManager::new(executor.clone(), get_s("github"))));
    
    // 7. Core LiNix Provisioning Tools
    registry.register(Arc::new(super::web::WebManager::new(executor.clone(), get_s("web"))));
    registry.register(Arc::new(super::link::LinkManager::new(executor.clone(), hooks.clone())));

    registry
}