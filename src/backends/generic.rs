use crate::core::{
    Backend, CommandExecutor, Installable, Package, PackageSpec, 
    Queryable, Result, Searchable, Upgradable, RepoManager, Error
};
use crate::parsers::OutputParser;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, debug};

/// Configuration for the Generic Manager Strategy.
/// Defines how to translate trait calls into CLI commands and which lock key to use.
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
    pub repo_add_args: Option<Vec<String>>,
    pub repo_remove_args: Option<Vec<String>>,
    pub repo_list_args: Option<Vec<String>>,
    /// If true, uses the LockMap to ensure only one process uses this backend at a time.
    pub is_exclusive: bool,
    /// Maps PackageSpec options to CLI flags.
    pub flag_map: HashMap<String, String>,
}

/// A Strategy-pattern based manager that handles the majority of CLI-based backends.
/// Delegates parsing to an injected OutputParser and execution to CommandExecutor.
/// Utilizes the LockMap for parallel-safe execution.
pub struct GenericManager {
    pub config: ManagerConfig,
    pub executor: CommandExecutor,
    pub parser: Arc<dyn OutputParser>,
}

impl Backend for GenericManager {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn is_available(&self) -> bool {
        self.executor.command_exists_sync(&self.config.name)
    }

    fn as_installable(&self) -> Option<&dyn Installable> { Some(self) }
    fn as_searchable(&self) -> Option<&dyn Searchable> { Some(self) }
    fn as_queryable(&self) -> Option<&dyn Queryable> { Some(self) }
    fn as_upgradable(&self) -> Option<&dyn Upgradable> { Some(self) }
    
    /// Point 7: Expose RepoManager capability if arguments are defined in config.
    fn as_repo_manager(&self) -> Option<&dyn RepoManager> {
        if self.config.repo_add_args.is_some() {
            Some(self)
        } else {
            None
        }
    }
}

#[async_trait]
impl Installable for GenericManager {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        if specs.is_empty() { return Ok(()); }
        let mut final_args: Vec<String> = self.config.install_args.clone();

        for (opt_key, flag_val) in &self.config.flag_map {
            if specs.iter().all(|s| s.options.get(opt_key) == Some(&"true".to_string())) {
                final_args.push(flag_val.clone());
            }
        }

        for spec in specs {
            final_args.push(spec.name.clone());
        }

        let arg_refs: Vec<&str> = final_args.iter().map(|s| s.as_str()).collect();

        if self.config.is_exclusive {
            self.executor.run_exclusive(&self.config.name, &self.config.name, &arg_refs, sudo).await?;
        } else {
            self.executor.run(&self.config.name, &arg_refs, sudo).await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        if names.is_empty() { return Ok(()); }
        let mut args: Vec<&str> = self.config.remove_args.iter().map(|s| s.as_str()).collect();
        args.extend(names.iter().map(|s| s.as_str()));

        if self.config.is_exclusive {
            self.executor.run_exclusive(&self.config.name, &self.config.name, &args, sudo).await?;
        } else {
            self.executor.run(&self.config.name, &args, sudo).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl Queryable for GenericManager {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let args: Vec<&str> = self.config.list_args.iter().map(|s| s.as_str()).collect();
        let output = self.executor.run_output(&self.config.name, &args, false).await?;
        Ok(self.parser.parse_installed(&output))
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        if let Some(ref manual_args) = self.config.list_manual_args {
            let args: Vec<&str> = manual_args.iter().map(|s| s.as_str()).collect();
            let output = self.executor.run_output(&self.config.name, &args, false).await?;
            Ok(self.parser.parse_installed(&output))
        } else {
            self.list_installed().await
        }
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}

#[async_trait]
impl Searchable for GenericManager {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let mut args: Vec<&str> = self.config.search_args.iter().map(|s| s.as_str()).collect();
        args.push(query);
        let output = self.executor.run_output(&self.config.name, &args, false).await?;
        Ok(self.parser.parse_search(&output))
    }
}

#[async_trait]
impl Upgradable for GenericManager {
    async fn update(&self, sudo: bool) -> Result<()> {
        if let Some(ref update_args) = self.config.update_args {
            let args: Vec<&str> = update_args.iter().map(|s| s.as_str()).collect();
            if self.config.is_exclusive {
                self.executor.run_exclusive(&self.config.name, &self.config.name, &args, sudo).await?;
            } else {
                self.executor.run(&self.config.name, &args, sudo).await?;
            }
        }
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        let args: Vec<&str> = self.config.upgrade_args.iter().map(|s| s.as_str()).collect();
        if self.config.is_exclusive {
            self.executor.run_exclusive(&self.config.name, &self.config.name, &args, sudo).await?;
        } else {
            self.executor.run(&self.config.name, &args, sudo).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl RepoManager for GenericManager {
    async fn add_repo(&self, name: &str, url: &str, sudo: bool) -> Result<()> {
        let base_args = self.config.repo_add_args.as_ref().ok_or_else(|| {
            Error::Other(format!("Backend {} does not support adding repositories", self.config.name))
        })?;

        let mut final_args = Vec::new();
        for arg in base_args {
            // Support placeholders in the config strings
            let processed = arg.replace("{name}", name).replace("{url}", url);
            final_args.push(processed);
        }
        
        let arg_refs: Vec<&str> = final_args.iter().map(|s| s.as_str()).collect();
        info!("Repo: Adding {} ({}) to {}...", name, url, self.config.name);
        
        self.executor.run_exclusive(&self.config.name, &self.config.name, &arg_refs, sudo).await?;
        Ok(())
    }

    async fn remove_repo(&self, name: &str, sudo: bool) -> Result<()> {
        let base_args = self.config.repo_remove_args.as_ref().ok_or_else(|| {
            Error::Other(format!("Backend {} does not support removing repositories", self.config.name))
        })?;

        let final_args: Vec<String> = base_args.iter().map(|a| a.replace("{name}", name)).collect();
        let arg_refs: Vec<&str> = final_args.iter().map(|s| s.as_str()).collect();
        
        info!("Repo: Removing {} from {}...", name, self.config.name);
        self.executor.run_exclusive(&self.config.name, &self.config.name, &arg_refs, sudo).await?;
        Ok(())
    }

    async fn list_repos(&self) -> Result<Vec<(String, String)>> {
        let base_args = self.config.repo_list_args.as_ref().ok_or_else(|| {
            Error::Other(format!("Backend {} does not support listing repositories", self.config.name))
        })?;

        let arg_refs: Vec<&str> = base_args.iter().map(|s| s.as_str()).collect();
        let output = self.executor.run_output(&self.config.name, &arg_refs, false).await?;
        
        // Use a simple default parsing logic for repos (name on left, url on right)
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
}