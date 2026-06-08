use crate::core::{
    BackendCore, CommandExecutor, Installable, Package, PackageSpec, 
    Queryable, Result, Searchable, Upgradable, RepoManager, HealthStatus, 
    HealthReport, MetadataProvider
};
use crate::parsers::OutputParser;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

/// Configuration for the Generic Manager Strategy.
/// Fulfills Phase 5.3: Comprehensive documentation of configuration fields.
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    /// The binary name of the package manager (e.g., "apt", "apk").
    pub name: String,
    /// Static arguments required for installation (e.g., ["install", "-y"]).
    pub install_args: Vec<String>,
    /// Static arguments required for removal (e.g., ["remove", "-y"]).
    pub remove_args: Vec<String>,
    /// Static arguments to list all installed packages.
    pub list_args: Vec<String>,
    /// Optional arguments to list only user-installed (manual) packages.
    pub list_manual_args: Option<Vec<String>>,
    /// Static arguments for remote package searching.
    pub search_args: Vec<String>,
    /// Static arguments for full system upgrade.
    pub upgrade_args: Vec<String>,
    /// Optional arguments to refresh repository metadata (e.g., "apt update").
    pub update_args: Option<Vec<String>>,
    /// Optional template arguments for adding a repository. Supports {name} and {url} tokens.
    pub repo_add_args: Option<Vec<String>>,
    /// Optional template arguments for removing a repository. Supports {name} token.
    pub repo_remove_args: Option<Vec<String>>,
    /// Optional arguments to list configured repositories.
    pub repo_list_args: Option<Vec<String>>,
    /// Phase 1.1: Optional arguments to fetch native dependencies. Supports {name} token.
    pub depends_args: Option<Vec<String>>,
    /// Phase 2.2: Defines if this manager requires root/sudo privileges for modifications.
    pub needs_root: bool,
    /// If true, LiNix will ensure only one instance of this manager runs globally.
    pub is_exclusive: bool,
    /// Internal map for custom backend flags.
    pub flag_map: HashMap<String, String>,
}

/// Core backend implementation for generic CLI-based managers.
/// Implements the strategy pattern to allow LiNix to support any CLI tool via configuration.
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

    fn needs_root(&self) -> bool {
        self.config.needs_root
    }
    
    async fn check_health(&self) -> Result<HealthReport> {
        if !self.is_available() {
            return Ok(HealthReport {
                status: HealthStatus::Critical,
                message: Some(format!("Binary for generic manager '{}' not found in PATH", self.name)),
            });
        }
        
        Ok(HealthReport {
            status: HealthStatus::Ok,
            message: None,
        })
    }
}

#[async_trait]
impl MetadataProvider for GenericBackendCore {
    /// Fulfills Phase 1.1: Resolves native dependencies using the configured depends_args.
    async fn get_dependencies(&self, name: &str) -> Result<Vec<String>> {
        let base_args = match &self.config.depends_args {
            Some(args) => args,
            None => return Ok(vec![]),
        };

        let mut final_args = Vec::new();
        for arg in base_args {
            final_args.push(arg.replace("{name}", name));
        }

        let arg_refs: Vec<&str> = final_args.iter().map(|s| s.as_str()).collect();
        let output = self.executor.run_output(&self.name, &arg_refs, false).await?;
        
        Ok(output.lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect())
    }
}

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
        
        let mut args: Vec<String> = self.core.config.remove_args.clone();
        for name in names {
            args.push(name.clone());
        }

        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        if self.core.config.is_exclusive {
            self.core.executor.run_exclusive(&self.core.name, &self.core.name, &arg_refs, sudo).await?;
        } else {
            self.core.executor.run(&self.core.name, &arg_refs, sudo).await?;
        }
        Ok(())
    }
}

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
        Ok(())
    }
}

pub struct GenericRepoManager {
    pub core: Arc<GenericBackendCore>,
}

#[async_trait]
impl RepoManager for GenericRepoManager {
    async fn add_repo(&self, name: &str, url: &str, sudo: bool) -> Result<()> {
        let base_args = self.core.config.repo_add_args.as_ref()
            .ok_or_else(|| crate::core::Error::Other("Repository addition not supported for this backend".into()))?;
        
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
            .ok_or_else(|| crate::core::Error::Other("Repository removal not supported for this backend".into()))?;
            
        let final_args: Vec<String> = base_args.iter().map(|a| a.replace("{name}", name)).collect();
        let arg_refs: Vec<&str> = final_args.iter().map(|s| s.as_str()).collect();
        
        self.core.executor.run(&self.core.name, &arg_refs, sudo).await?;
        Ok(())
    }

    async fn list_repos(&self) -> Result<Vec<(String, String)>> {
        let base_args = self.core.config.repo_list_args.as_ref()
            .ok_or_else(|| crate::core::Error::Other("Repository listing not supported for this backend".into()))?;
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