// src/backends/generic.rs

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

/// How a backend expresses an exact version at install time, for reproducible
/// (locked) installs. `{name}` / `{version}` are substituted.
#[derive(Debug, Clone)]
pub enum VersionPin {
    /// A single token, e.g. apt `name=version`, pip `name==version`, bun `name@version`.
    Inline(String),
    /// The bare name followed by flag args, e.g. winget/choco `--version {version}`,
    /// gem `-v {version}`.
    Flag(Vec<String>),
}

impl VersionPin {
    /// Produce the install argument(s) for `name` pinned to `version`.
    fn apply(&self, name: &str, version: &str) -> Vec<String> {
        match self {
            VersionPin::Inline(tmpl) => vec![tmpl.replace("{name}", name).replace("{version}", version)],
            VersionPin::Flag(flags) => {
                let mut out = vec![name.to_string()];
                out.extend(flags.iter().map(|f| f.replace("{name}", name).replace("{version}", version)));
                out
            }
        }
    }
}

/// True when a version string represents a real pin (not "latest"/"*"/empty).
fn is_concrete_version(v: &str) -> bool {
    !v.is_empty() && v != "latest" && v != "*"
}

/// Configuration for the Generic Manager Strategy.
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    pub name: String,
    pub install_args: Vec<String>,
    pub remove_args: Vec<String>,
    pub list_args: Vec<String>,
    pub list_manual_args: Option<Vec<String>>,
    pub search_args: Vec<String>,
    /// Optional: if specified, use this binary for search instead of the backend name.
    pub search_binary: Option<String>,
    pub upgrade_args: Vec<String>,
    pub update_args: Option<Vec<String>>,
    pub repo_add_args: Option<Vec<String>>,
    pub repo_remove_args: Option<Vec<String>>,
    pub repo_list_args: Option<Vec<String>>,
    pub depends_args: Option<Vec<String>>,
    /// Native syntax for pinning an exact version at install (None = no version pinning).
    pub version_pin: Option<VersionPin>,
    pub needs_root: bool,
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
        let sudo = self.needs_root();
        let output = self.executor.run_output(&self.name, &arg_refs, sudo).await?;

        // Extract clean package names. apt/zypper print labelled lines
        // ("Depends: libc6", "Requires: foo"); strip the "Label: " prefix and take the
        // bare name (dropping any version constraint / alternative). Backends that print
        // bare names (e.g. apk) pass through unchanged.
        Ok(output.lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .filter_map(|l| {
                let after_label = l.rsplit(": ").next().unwrap_or(l);
                after_label.split_whitespace().next().map(|s| s.to_string())
            })
            .filter(|s| !s.is_empty())
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
            // Honor an exact version pin (reproducible/locked installs) using the
            // backend's native syntax, when both a pin syntax and a concrete version exist.
            match (spec.options.get("version"), &self.core.config.version_pin) {
                (Some(ver), Some(pin)) if is_concrete_version(ver) => {
                    final_args.extend(pin.apply(&spec.name, ver));
                }
                _ => final_args.push(spec.name.clone()),
            }
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
        // Use search_binary if specified, otherwise fallback to the backend name
        let bin = self.core.config.search_binary.as_deref().unwrap_or(&self.core.name);
        let output = self.core.executor.run_output(bin, &args, false).await?;
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

/// Some backends interpolate repo `{name}`/`{url}` into `sh -c` strings
/// (e.g. apk/apt). Reject shell metacharacters so a crafted argument cannot break out
/// of the intended command.
fn reject_shell_meta(field: &str, value: &str) -> Result<()> {
    if value.chars().any(|c| matches!(c, '\'' | '"' | '`' | '$' | ';' | '&' | '|' | '<' | '>' | '\n' | '\r' | '\\')) {
        return Err(crate::core::Error::Other(format!("Unsafe characters in repo {}: '{}'", field, value)));
    }
    Ok(())
}

#[async_trait]
impl RepoManager for GenericRepoManager {
    async fn add_repo(&self, name: &str, url: &str, sudo: bool) -> Result<()> {
        reject_shell_meta("name", name)?;
        reject_shell_meta("url", url)?;
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
        reject_shell_meta("name", name)?;
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
            let trimmed = line.trim();
            if trimmed.is_empty() { continue; }
            // Skip dashed separator rows ("--------").
            if trimmed.chars().all(|c| c == '-' || c == '=') { continue; }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 { continue; }
            // Skip obvious table headers (e.g. winget "Name Argument Explicit",
            // scoop "Name Source Updated") so they don't show up as repositories.
            let is_header = matches!(parts[0], "Name" | "NAME" | "Repository" | "Repo" | "Bucket" | "Source")
                && matches!(parts[1], "Argument" | "URL" | "Url" | "Source" | "Updated" | "Explicit" | "Enabled");
            if is_header { continue; }
            repos.push((parts[0].to_string(), parts[1].to_string()));
        }
        Ok(repos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_pin_renders_native_syntax() {
        // inline forms (apt/pip/bun)
        assert_eq!(VersionPin::Inline("{name}={version}".into()).apply("curl", "7.81.0"), vec!["curl=7.81.0"]);
        assert_eq!(VersionPin::Inline("{name}=={version}".into()).apply("requests", "2.31.0"), vec!["requests==2.31.0"]);
        // flag forms (winget/choco/gem)
        assert_eq!(
            VersionPin::Flag(vec!["--version".into(), "{version}".into()]).apply("Git.Git", "2.54.0"),
            vec!["Git.Git", "--version", "2.54.0"]
        );
    }

    #[test]
    fn concrete_version_rejects_floating() {
        assert!(is_concrete_version("1.2.3"));
        assert!(!is_concrete_version("latest"));
        assert!(!is_concrete_version("*"));
        assert!(!is_concrete_version(""));
    }
}