// src/backends/registry.rs
use crate::core::PackageManager;
use crate::config::Config;
use crate::app::LuaHooks;
use std::collections::HashMap;
use std::sync::Arc;

pub struct BackendRegistry {
    managers: HashMap<String, Arc<dyn PackageManager>>,
}

impl BackendRegistry {
    pub fn new() -> Self { Self { managers: HashMap::new() } }
    pub fn register(&mut self, manager: Arc<dyn PackageManager>) {
        self.managers.insert(manager.name().to_string(), manager);
    }
    pub fn get(&self, name: &str) -> Option<Arc<dyn PackageManager>> { self.managers.get(name).cloned() }
    pub fn available(&self) -> Vec<Arc<dyn PackageManager>> {
        self.managers.values().filter(|m| m.is_available()).cloned().collect()
    }
    pub fn all(&self) -> Vec<Arc<dyn PackageManager>> { self.managers.values().cloned().collect() }
    pub fn get_filtered(&self, names: &[String]) -> Vec<Arc<dyn PackageManager>> {
        names.iter().filter_map(|name| self.get(name)).filter(|m| m.is_available()).collect()
    }
    pub fn available_names(&self) -> Vec<String> {
        self.managers.iter().filter(|(_, m)| m.is_available()).map(|(k, _)| k.clone()).collect()
    }
}

pub async fn create_default_registry(executor: crate::core::CommandExecutor, config: &Config, hooks: Arc<LuaHooks>) -> BackendRegistry {
    let mut registry = BackendRegistry::new();
    let get_s = |name: &str| config.backend_settings.get(name).cloned();

    #[cfg(target_os = "linux")] {
        registry.register(Arc::new(super::apt::AptManager::new(executor.clone(), get_s("apt"))));
        registry.register(Arc::new(super::pacman::PacmanManager::new(executor.clone(), get_s("pacman"))));
        registry.register(Arc::new(super::dnf::DnfManager::new(executor.clone(), get_s("dnf"))));
        registry.register(Arc::new(super::apk::ApkManager::new(executor.clone(), get_s("apk"))));
    }
    #[cfg(target_os = "windows")] {
        registry.register(Arc::new(super::windows::winget::WingetManager::new(executor.clone(), get_s("winget"))));
        registry.register(Arc::new(super::windows::choco::ChocoManager::new(executor.clone(), get_s("choco"))));
        registry.register(Arc::new(super::windows::scoop::ScoopManager::new(executor.clone(), get_s("scoop"))));
    }

    registry.register(Arc::new(super::flatpak::FlatpakManager::new(executor.clone(), get_s("flatpak"))));
    registry.register(Arc::new(super::brew::BrewManager::new(executor.clone(), get_s("brew"))));
    registry.register(Arc::new(super::cargo::CargoManager::new(executor.clone(), get_s("cargo"))));
    registry.register(Arc::new(super::npm::NpmManager::new(executor.clone(), get_s("npm"))));
    registry.register(Arc::new(super::pip::PipManager::new(executor.clone(), get_s("pip"))));
    registry.register(Arc::new(super::web::WebManager::new(executor.clone(), get_s("web"))));
    registry.register(Arc::new(super::link::LinkManager::new(executor.clone(), hooks.clone())));
    registry.register(Arc::new(super::emacs::EmacsManager::new(executor.clone(), None)));
    registry.register(Arc::new(super::github::GithubManager::new(executor.clone(), get_s("github"))));
    
    registry
}