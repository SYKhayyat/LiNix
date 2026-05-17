use crate::core::{
    BackendCore, CommandExecutor, Installable, Package, PackageSpec, 
    Queryable, Result, Searchable, Upgradable, RepoManager, Error, HealthStatus, HealthReport
};
use crate::parsers::OutputParser;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, debug};

// ============================================================================
// FIX #9: Liskov Substitution Principle - Fix repository management
// Repository management is now properly conditional on configuration.
// ============================================================================

/// Configuration for the Generic Manager Strategy.
/// Defines how to translate trait calls into CLI commands and which lock key to use.
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    pub name: String,
    pub install_args: Vec<String>,
    pub remove_args: Vec<String>,
    pub list_args: Vec<String>,
    pub list_manual_args: Option<Vec<String>>,
    pub search_args: Vec<String>,
    pub upgrade_args: Vec<String>,
    pub update_args: Option<Vec<String>>,
    /// Roadmap Point 7: Commands for repository management.
    /// FIX #9: These must ALL be present for repo management to be available.
    pub repo_add_args: Option<Vec<String>>,
    pub repo_remove_args: Option<Vec<String>>,
    pub repo_list_args: Option<Vec<String>>,
    /// If true, uses the LockMap to ensure only one process uses this backend at a time.
    pub is_exclusive: bool,
    /// Maps PackageSpec options to CLI flags.
    pub flag_map: HashMap<String, String>,
}

impl ManagerConfig {
    /// FIX #9: Returns true if this backend fully supports repository management.
    /// All three repo operations must have arguments defined.
    pub fn supports_repo_management(&self) -> bool {
        self.repo_add_args.is_some() 
            && self.repo_remove_args.is_some() 
            && self.repo_list_args.is_some()
    }
    
    /// Returns true if this backend supports updating metadata.
    pub fn supports_update(&self) -> bool {
        self.update_args.is_some()
    }
    
    /// Returns true if this backend supports manual package tracking.
    pub fn supports_manual_listing(&self) -> bool {
        self.list_manual_args.is_some()
    }
}

/// Core backend implementation for generic CLI-based managers.
pub struct GenericBackendCore {
    pub name: String,
    pub executor: CommandExecutor,
    pub config: ManagerConfig,
    pub parser: Arc<dyn OutputParser>,
}

impl BackendCore for GenericBackendCore {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        self.executor.command_exists_sync(&self.name)
    }
    
    async fn check_health(&self) -> Result<HealthReport> {
        if !self.is_available() {
            return Ok(HealthReport {
                status: HealthStatus::Critical,
                message: Some(format!("Command '{}' not found in PATH", self.name)),
            });
        }
        
        // Check if we can run a basic command
        match self.executor.run_output(&self.name, &["--version"], false).await {
            Ok(_) => Ok(HealthReport {
                status: HealthStatus::Ok,
                message: None,
            }),
            Err(e) => Ok(HealthReport {
                status: HealthStatus::Degraded,
                message: Some(format!("Backend responded with error: {}", e)),
            }),
        }
    }
}

/// Installable capability for generic backends.
pub struct GenericInstallable {
    pub core: Arc<GenericBackendCore>,
}

#[async_trait]
impl Installable for GenericInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        if specs.is_empty() {
            return Ok(());
        }
        
        let mut final_args: Vec<String> = self.core.config.install_args.clone();

        for (opt_key, flag_val) in &self.core.config.flag_map {
            if specs.iter().all(|s| s.options.get(opt_key) == Some(&"true".to_string())) {
                final_args.push(flag_val.clone());
            }
        }

        for spec in specs {
            final_args.push(spec.name.clone());
        }

        let arg_refs: Vec<&str> = final_args.iter().map(|s| s.as_str()).collect();

        if self.core.config.is_exclusive {
            self.core.executor.run_exclusive(&self.core.name, &self.core.name, &arg_refs, sudo).await?;
        } else {
            self.core.executor.run(&self.core.name, &arg_refs, sudo).await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        if names.is_empty() {
            return Ok(());
        }
        
        let mut args: Vec<&str> = self.core.config.remove_args.iter().map(|s| s.as_str()).collect();
        args.extend(names.iter().map(|s| s.as_str()));

        if self.core.config.is_exclusive {
            self.core.executor.run_exclusive(&self.core.name, &self.core.name, &args, sudo).await?;
        } else {
            self.core.executor.run(&self.core.name, &args, sudo).await?;
        }
        Ok(())
    }
}

/// Queryable capability for generic backends.
pub struct GenericQueryable {
    pub core: Arc<GenericBackendCore>,
}

#[async_trait]
impl Queryable for GenericQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let args: Vec<&str> = self.core.config.list_args.iter().map(|s| s.as_str()).collect();
        let output = self.core.executor.run_output(&self.core.name, &args, false).await?;
        Ok(self.core.parser.parse_installed(&output))
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        if let Some(ref manual_args) = self.core.config.list_manual_args {
            let args: Vec<&str> = manual_args.iter().map(|s| s.as_str()).collect();
            let output = self.core.executor.run_output(&self.core.name, &args, false).await?;
            Ok(self.core.parser.parse_installed(&output))
        } else {
            // Fallback to full list if manual listing not supported
            self.list_installed().await
        }
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}

/// Searchable capability for generic backends.
pub struct GenericSearchable {
    pub core: Arc<GenericBackendCore>,
}

#[async_trait]
impl Searchable for GenericSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let mut args: Vec<&str> = self.core.config.search_args.iter().map(|s| s.as_str()).collect();
        args.push(query);
        let output = self.core.executor.run_output(&self.core.name, &args, false).await?;
        Ok(self.core.parser.parse_search(&output))
    }
}

/// Upgradable capability for generic backends.
pub struct GenericUpgradable {
    pub core: Arc<GenericBackendCore>,
}

#[async_trait]
impl Upgradable for GenericUpgradable {
    async fn update(&self, sudo: bool) -> Result<()> {
        if let Some(ref update_args) = self.core.config.update_args {
            let args: Vec<&str> = update_args.iter().map(|s| s.as_str()).collect();
            if self.core.config.is_exclusive {
                self.core.executor.run_exclusive(&self.core.name, &self.core.name, &args, sudo).await?;
            } else {
                self.core.executor.run(&self.core.name, &args, sudo).await?;
            }
        }
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        let args: Vec<&str> = self.core.config.upgrade_args.iter().map(|s| s.as_str()).collect();
        if self.core.config.is_exclusive {
            self.core.executor.run_exclusive(&self.core.name, &self.core.name, &args, sudo).await?;
        } else {
            self.core.executor.run(&self.core.name, &args, sudo).await?;
        }
        Ok(())
    }
}

/// FIX #9: RepoManager capability for generic backends.
/// This struct is only created and returned if the backend fully supports
/// all three repository operations (add, remove, list).
pub struct GenericRepoManager {
    pub core: Arc<GenericBackendCore>,
}

impl GenericRepoManager {
    /// FIX #9: Returns true only if all repo operations are supported.
    /// This prevents Liskov violations where partial implementation would panic.
    pub fn is_fully_supported(&self) -> bool {
        self.core.config.supports_repo_management()
    }
}

#[async_trait]
impl RepoManager for GenericRepoManager {
    async fn add_repo(&self, name: &str, url: &str, sudo: bool) -> Result<()> {
        // FIX #9: Explicit check before attempting operation
        if !self.is_fully_supported() {
            return Err(Error::Other(format!(
                "Backend '{}' does not support repository management. Missing configuration for repo operations.",
                self.core.name
            )));
        }
        
        let base_args = self.core.config.repo_add_args.as_ref().unwrap();
        let mut final_args = Vec::new();
        for arg in base_args {
            let processed = arg.replace("{name}", name).replace("{url}", url);
            final_args.push(processed);
        }
        
        let arg_refs: Vec<&str> = final_args.iter().map(|s| s.as_str()).collect();
        info!("Repo: Adding {} ({}) to {}...", name, url, self.core.name);
        
        if self.core.config.is_exclusive {
            self.core.executor.run_exclusive(&self.core.name, &self.core.name, &arg_refs, sudo).await?;
        } else {
            self.core.executor.run(&self.core.name, &arg_refs, sudo).await?;
        }
        Ok(())
    }

    async fn remove_repo(&self, name: &str, sudo: bool) -> Result<()> {
        // FIX #9: Explicit check before attempting operation
        if !self.is_fully_supported() {
            return Err(Error::Other(format!(
                "Backend '{}' does not support repository removal.",
                self.core.name
            )));
        }
        
        let base_args = self.core.config.repo_remove_args.as_ref().unwrap();
        let final_args: Vec<String> = base_args.iter().map(|a| a.replace("{name}", name)).collect();
        let arg_refs: Vec<&str> = final_args.iter().map(|s| s.as_str()).collect();
        
        info!("Repo: Removing {} from {}...", name, self.core.name);
        if self.core.config.is_exclusive {
            self.core.executor.run_exclusive(&self.core.name, &self.core.name, &arg_refs, sudo).await?;
        } else {
            self.core.executor.run(&self.core.name, &arg_refs, sudo).await?;
        }
        Ok(())
    }

    async fn list_repos(&self) -> Result<Vec<(String, String)>> {
        // FIX #9: Explicit check before attempting operation
        if !self.is_fully_supported() {
            return Err(Error::Other(format!(
                "Backend '{}' does not support repository listing.",
                self.core.name
            )));
        }
        
        let base_args = self.core.config.repo_list_args.as_ref().unwrap();
        let arg_refs: Vec<&str> = base_args.iter().map(|s| s.as_str()).collect();
        let output = self.core.executor.run_output(&self.core.name, &arg_refs, false).await?;
        
        // Parse repos - simple heuristic: name on left, url on right
        let mut repos = Vec::new();
        for line in output.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                repos.push((parts[0].to_string(), parts[1].to_string()));
            } else if !line.trim().is_empty() {
                repos.push((line.trim().to_string(), "unknown".to_string()));
            }
        }
        Ok(repos)
    }
    
    fn can_manage_repos(&self) -> bool {
        self.is_fully_supported()
    }
}

/// Helper function to determine if a backend capability should be provided.
pub fn should_provide_repo_manager(config: &ManagerConfig) -> bool {
    config.supports_repo_management()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsers::LambdaParser;

    fn create_test_config() -> ManagerConfig {
        ManagerConfig {
            name: "test".to_string(),
            install_args: vec!["install".to_string()],
            remove_args: vec!["remove".to_string()],
            list_args: vec!["list".to_string()],
            list_manual_args: None,
            search_args: vec!["search".to_string()],
            upgrade_args: vec!["upgrade".to_string()],
            update_args: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            is_exclusive: false,
            flag_map: HashMap::new(),
        }
    }
    
    fn create_full_repo_config() -> ManagerConfig {
        ManagerConfig {
            name: "test".to_string(),
            install_args: vec!["install".to_string()],
            remove_args: vec!["remove".to_string()],
            list_args: vec!["list".to_string()],
            list_manual_args: None,
            search_args: vec!["search".to_string()],
            upgrade_args: vec!["upgrade".to_string()],
            update_args: None,
            repo_add_args: Some(vec!["add".to_string(), "{url}".to_string()]),
            repo_remove_args: Some(vec!["remove".to_string(), "{name}".to_string()]),
            repo_list_args: Some(vec!["list".to_string()]),
            is_exclusive: false,
            flag_map: HashMap::new(),
        }
    }

    #[test]
    fn test_supports_repo_management() {
        let config = create_test_config();
        assert!(!config.supports_repo_management());
        
        let config2 = create_full_repo_config();
        assert!(config2.supports_repo_management());
    }
    
    #[test]
    fn test_partial_repo_config_not_supported() {
        let mut config = create_test_config();
        config.repo_add_args = Some(vec!["add".to_string()]);
        // Missing remove and list
        assert!(!config.supports_repo_management());
        
        config.repo_remove_args = Some(vec!["remove".to_string()]);
        // Still missing list
        assert!(!config.supports_repo_management());
    }

    #[test]
    fn test_should_provide_repo_manager() {
        let config = create_test_config();
        assert!(!should_provide_repo_manager(&config));
        
        let config2 = create_full_repo_config();
        assert!(should_provide_repo_manager(&config2));
    }
    
    #[test]
    fn test_generic_repo_manager_is_fully_supported() {
        let executor = CommandExecutor::new(true, false);
        let parser: Arc<dyn OutputParser> = Arc::new(LambdaParser {
            installed_fn: |_| vec![],
            search_fn: |_| vec![],
        });
        
        let core = Arc::new(GenericBackendCore {
            name: "test".to_string(),
            executor,
            config: create_full_repo_config(),
            parser,
        });
        
        let repo_manager = GenericRepoManager { core: core.clone() };
        assert!(repo_manager.is_fully_supported());
        assert!(repo_manager.can_manage_repos());
    }
    
    #[test]
    fn test_generic_repo_manager_not_supported() {
        let executor = CommandExecutor::new(true, false);
        let parser: Arc<dyn OutputParser> = Arc::new(LambdaParser {
            installed_fn: |_| vec![],
            search_fn: |_| vec![],
        });
        
        let core = Arc::new(GenericBackendCore {
            name: "test".to_string(),
            executor,
            config: create_test_config(),
            parser,
        });
        
        let repo_manager = GenericRepoManager { core: core.clone() };
        assert!(!repo_manager.is_fully_supported());
        assert!(!repo_manager.can_manage_repos());
    }
    
    #[tokio::test]
    async fn test_add_repo_errors_when_not_supported() {
        let executor = CommandExecutor::new(true, false);
        let parser: Arc<dyn OutputParser> = Arc::new(LambdaParser {
            installed_fn: |_| vec![],
            search_fn: |_| vec![],
        });
        
        let core = Arc::new(GenericBackendCore {
            name: "test".to_string(),
            executor,
            config: create_test_config(),
            parser,
        });
        
        let repo_manager = GenericRepoManager { core };
        let result = repo_manager.add_repo("test-repo", "https://test.com", false).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("does not support repository management"));
    }
    
    #[tokio::test]
    async fn test_add_repo_succeeds_when_supported() {
        let executor = CommandExecutor::new(true, false);
        let parser: Arc<dyn OutputParser> = Arc::new(LambdaParser {
            installed_fn: |_| vec![],
            search_fn: |_| vec![],
        });
        
        let core = Arc::new(GenericBackendCore {
            name: "test".to_string(),
            executor,
            config: create_full_repo_config(),
            parser,
        });
        
        let repo_manager = GenericRepoManager { core };
        // In dry-run mode, this should succeed
        let result = repo_manager.add_repo("test-repo", "https://test.com", false).await;
        assert!(result.is_ok());
    }
}