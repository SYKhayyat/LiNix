use crate::core::{
    BackendCore, CommandExecutor, Installable, Package, PackageSpec, 
    Queryable, Result, Searchable, Upgradable, RepoManager, Error, HealthStatus, HealthReport
};
use crate::parsers::OutputParser;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

/// Configuration for the Generic Manager Strategy.
/// Defines how to translate trait calls into CLI commands.
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
    pub repo_add_args: Option<Vec<String>>,
    pub repo_remove_args: Option<Vec<String>>,
    pub repo_list_args: Option<Vec<String>>,
    pub is_exclusive: bool,
    pub flag_map: HashMap<String, String>,
}

/// Core backend implementation for generic CLI-based managers.
pub struct GenericBackendCore {
    pub name: String,
    pub executor: CommandExecutor,
    pub config: ManagerConfig,
    pub parser: Arc<dyn OutputParser>,
}

#[async_trait]
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
        
        Ok(HealthReport {
            status: HealthStatus::Ok,
            message: None,
        })
    }
}

/// Installable capability for generic backends.
pub struct GenericInstallable {
    pub core: Arc<GenericBackendCore>,
}

#[async_trait]
impl Installable for GenericInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        if specs.is_empty() { return Ok(()); }
        
        let mut final_args: Vec<String> = self.core.config.install_args.clone();

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
        if names.is_empty() { return Ok(()); }
        
        let mut args: Vec<&str> = self.core.config.remove_args.iter().map(|s| s.as_str()).collect();
        for name in names {
            args.push(name);
        }

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
            self.core.executor.run(&self.core.name, &args, sudo).await?;
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

    async fn clean_orphans(&self, _sudo: bool) -> Result<()> {
        // Default implementation for generic managers.
        Ok(())
    }
}

/// RepoManager capability for generic backends.
pub struct GenericRepoManager {
    pub core: Arc<GenericBackendCore>,
}

#[async_trait]
impl RepoManager for GenericRepoManager {
    async fn add_repo(&self, name: &str, url: &str, sudo: bool) -> Result<()> {
        let base_args = self.core.config.repo_add_args.as_ref()
            .ok_or_else(|| Error::Other("Repo adding not supported".into()))?;
        
        let mut final_args = Vec::new();
        for arg in base_args {
            final_args.push(arg.replace("{name}", name).replace("{url}", url));
        }
        
        let arg_refs: Vec<&str> = final_args.iter().map(|s| s.as_str()).collect();
        info!("Repo: Adding {} to {}...", name, self.core.name);
        self.core.executor.run(&self.core.name, &arg_refs, sudo).await?;
        Ok(())
    }

    async fn remove_repo(&self, name: &str, sudo: bool) -> Result<()> {
        let base_args = self.core.config.repo_remove_args.as_ref()
            .ok_or_else(|| Error::Other("Repo removal not supported".into()))?;
            
        let final_args: Vec<String> = base_args.iter().map(|a| a.replace("{name}", name)).collect();
        let arg_refs: Vec<&str> = final_args.iter().map(|s| s.as_str()).collect();
        
        self.core.executor.run(&self.core.name, &arg_refs, sudo).await?;
        Ok(())
    }

    async fn list_repos(&self) -> Result<Vec<(String, String)>> {
        let base_args = self.core.config.repo_list_args.as_ref()
            .ok_or_else(|| Error::Other("Repo listing not supported".into()))?;
        let arg_refs: Vec<&str> = base_args.iter().map(|s| s.as_str()).collect();
        let output = self.core.executor.run_output(&self.core.name, &arg_refs, false).await?;
        
        let mut repos = Vec::new();
        for line in output.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                repos.push((parts[0].to_string(), parts[1].to_string()));
            }
        }
        Ok(repos)
    }
}