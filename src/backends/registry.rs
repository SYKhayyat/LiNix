// C:\Users\Administrator\Videos\Nexus\linix\src\backends\registry.rs
use crate::core::{CommandExecutor, PackageManager};
use crate::config::Config;
use std::collections::HashMap;
use std::sync::Arc;

pub struct BackendRegistry {
    managers: HashMap<String, Arc<dyn PackageManager>>,
}

impl BackendRegistry {
    pub fn new() -> Self {
        Self { managers: HashMap::new() }
    }

    pub fn register<T: PackageManager + 'static>(&mut self, manager: T) {
        let name = manager.name().to_string();
        self.managers.insert(name, Arc::new(manager));
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn PackageManager>> {
        self.managers.get(name).cloned()
    }

    pub fn available(&self) -> Vec<Arc<dyn PackageManager>> {
        self.managers.values().filter(|m| m.is_available()).cloned().collect()
    }

    pub fn all(&self) -> Vec<Arc<dyn PackageManager>> {
        self.managers.values().cloned().collect()
    }
	pub fn get_filtered(&self, names: &[String]) -> Vec<std::sync::Arc<dyn crate::core::PackageManager>> {
        if names.is_empty() {
            return self.available();
        }
        names
            .iter()
            .filter_map(|name| self.get(name))
            .filter(|m| m.is_available())
            .collect()
    }
	pub fn available_names(&self) -> Vec<String> {
        self.managers
            .iter()
            .filter(|(_, m)| m.is_available())
            .map(|(name, _)| name.clone())
            .collect()
    }
}


pub async fn create_default_registry(executor: CommandExecutor, config: &Config) -> BackendRegistry {
    let mut registry = BackendRegistry::new();
    let get_settings = |name: &str| config.backend_settings.get(name).cloned();

    // Linux-specific
    #[cfg(target_os = "linux")]
    {
        registry.register(super::apt::AptManager::new(executor.clone(), get_settings("apt")));
        registry.register(super::dnf::DnfManager::new(executor.clone(), get_settings("dnf")));
        registry.register(super::pacman::PacmanManager::new(executor.clone(), get_settings("pacman")));
        registry.register(super::apk::ApkManager::new(executor.clone(), get_settings("apk")));
    }

    // Windows-specific
    #[cfg(target_os = "windows")]
    {
        registry.register(super::windows::winget::WingetManager::new(executor.clone(), get_settings("winget")));
        registry.register(super::windows::choco::ChocoManager::new(executor.clone(), get_settings("choco")));
        registry.register(super::windows::scoop::ScoopManager::new(executor.clone(), get_settings("scoop")));
    }

    // Universal & Modern
    registry.register(super::flatpak::FlatpakManager::new(executor.clone(), get_settings("flatpak")));
    registry.register(super::brew::BrewManager::new(executor.clone(), get_settings("brew")));
    registry.register(super::github::GithubManager::new(executor.clone(), get_settings("github")));
    registry.register(super::uv::UvManager::new(executor.clone(), get_settings("uv")));
    registry.register(super::pnpm::PnpmManager::new(executor.clone(), get_settings("pnpm")));
    registry.register(super::vscode::VscodeManager::new(executor.clone(), get_settings("vscode")));
    registry.register(super::mise::MiseManager::new(executor.clone(), get_settings("mise")));

    // Languages
    registry.register(super::cargo::CargoManager::new(executor.clone(), get_settings("cargo")));
    registry.register(super::npm::NpmManager::new(executor.clone(), get_settings("npm")));
    registry.register(super::pip::PipManager::new(executor.clone(), get_settings("pip")));

    registry
}