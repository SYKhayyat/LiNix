use crate::core::{CommandExecutor, PackageManager};
use std::collections::HashMap;
use std::sync::Arc;

/// Registry of available package manager backends
pub struct BackendRegistry {
    managers: HashMap<String, Arc<dyn PackageManager>>,
}

impl BackendRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            managers: HashMap::new(),
        }
    }

    /// Register a package manager
    pub fn register<T: PackageManager + 'static>(&mut self, manager: T) {
        let name = manager.name().to_string();
        self.managers.insert(name, Arc::new(manager));
    }

    /// Get a package manager by name
    pub fn get(&self, name: &str) -> Option<Arc<dyn PackageManager>> {
        self.managers.get(name).cloned()
    }

    /// Get all registered managers
    pub fn all(&self) -> Vec<Arc<dyn PackageManager>> {
        self.managers.values().cloned().collect()
    }

    /// Get all available managers (those that are installed on the system)
    pub fn available(&self) -> Vec<Arc<dyn PackageManager>> {
        self.managers
            .values()
            .filter(|m| m.is_available())
            .cloned()
            .collect()
    }

    /// Get names of all registered managers
    pub fn names(&self) -> Vec<String> {
        self.managers.keys().cloned().collect()
    }

    /// Get names of all available managers
    pub fn available_names(&self) -> Vec<String> {
        self.managers
            .iter()
            .filter(|(_, m)| m.is_available())
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Check if a manager is registered
    pub fn has(&self, name: &str) -> bool {
        self.managers.contains_key(name)
    }

    /// Get managers filtered by a list of names
    pub fn get_filtered(&self, names: &[String]) -> Vec<Arc<dyn PackageManager>> {
        if names.is_empty() {
            return self.available();
        }

        names
            .iter()
            .filter_map(|name| self.get(name))
            .filter(|m| m.is_available())
            .collect()
    }
}

impl Default for BackendRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Create the default registry with all supported backends
pub async fn create_default_registry(executor: CommandExecutor) -> BackendRegistry {
    let mut registry = BackendRegistry::new();

    // Linux system package managers
    #[cfg(target_os = "linux")]
    {
        registry.register(super::apt::AptManager::new(executor.clone()));
        registry.register(super::pacman::PacmanManager::new(executor.clone()));
        registry.register(super::dnf::DnfManager::new(executor.clone()));
        registry.register(super::zypper::ZypperManager::new(executor.clone()));
        registry.register(super::apk::ApkManager::new(executor.clone()));
    }

    // Universal package managers
    registry.register(super::flatpak::FlatpakManager::new(executor.clone()));
    registry.register(super::snap::SnapManager::new(executor.clone()));
    registry.register(super::brew::BrewManager::new(executor.clone()));

    // Language-specific package managers
    registry.register(super::pip::PipManager::new(executor.clone()));
    registry.register(super::pipx::PipxManager::new(executor.clone()));
    registry.register(super::poetry::PoetryManager::new(executor.clone()));
    registry.register(super::npm::NpmManager::new(executor.clone()));
    registry.register(super::yarn::YarnManager::new(executor.clone()));
    registry.register(super::bun::BunManager::new(executor.clone()));
    registry.register(super::cargo::CargoManager::new(executor.clone()));
    registry.register(super::gem::GemManager::new(executor.clone()));
    registry.register(super::composer::ComposerManager::new(executor.clone()));
    registry.register(super::go::GoManager::new(executor.clone()));

    // GitHub releases manager
    registry.register(super::github::GithubManager::new(executor.clone()));

    // Windows backends
    #[cfg(target_os = "windows")]
    {
        registry.register(super::windows::winget::WingetManager::new(executor.clone()));
        registry.register(super::windows::choco::ChocoManager::new(executor.clone()));
        registry.register(super::windows::scoop::ScoopManager::new(executor.clone()));
    }

    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_basic() {
        let registry = BackendRegistry::new();
        assert!(registry.names().is_empty());
    }
}
