use crate::core::{
    BackendCore, CommandExecutor, Enumerable, Error, Installable, MetadataProvider, Package,
    PackageSpec, Queryable, RepoManager, Result, Searchable, Upgradable,
};
use crate::parsers::pacman;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;

/// Repo names are interpolated into shell commands and file paths, so allow only a
/// conservative character set.
fn validate_repo_name(name: &str) -> Result<()> {
    if !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "Invalid pacman repo name: '{}'",
            name
        )))
    }
}

/// Reject URLs containing shell metacharacters before they are embedded in a `sh -c`
/// command (we write the drop-in file as root via the shell).
fn validate_repo_url(url: &str) -> Result<()> {
    if url.trim().is_empty() {
        return Err(Error::Other(
            "pacman add_repo requires a repository URL".into(),
        ));
    }
    if url.chars().any(|c| {
        matches!(
            c,
            '\'' | '"' | '`' | '$' | ';' | '&' | '|' | '<' | '>' | '\n' | '\r' | '\\'
        )
    }) {
        return Err(Error::Other(format!(
            "Unsafe characters in repo URL: '{}'",
            url
        )));
    }
    Ok(())
}

pub struct PacmanBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl PacmanBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor: executor.with_exit_policy(crate::core::exit_policy::pacman()),
            name: "pacman".to_string(),
        }
    }
}

#[async_trait]
impl BackendCore for PacmanBackendCore {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("pacman")
    }
    fn probes(&self) -> Vec<String> {
        vec!["pacman".into()]
    }

    fn needs_root(&self) -> bool {
        true
    }
}

#[async_trait]
impl MetadataProvider for PacmanBackendCore {
    async fn get_dependencies(&self, name: &str) -> Result<Vec<String>> {
        let output = self
            .executor
            .run_output("pacman", &["-Si", name], false)
            .await?;
        let mut deps = Vec::new();
        for line in output.lines() {
            if let Some(dep_line) = line.strip_prefix("Depends On     :") {
                let parts: Vec<&str> = dep_line.split_whitespace().collect();
                for part in parts {
                    if part != "None" {
                        let clean_dep = part.split(['>', '<', '=']).next().unwrap_or(part);
                        deps.push(clean_dep.to_string());
                    }
                }
            }
        }
        Ok(deps)
    }
}

pub struct PacmanInstallable {
    pub core: Arc<PacmanBackendCore>,
}

#[async_trait]
impl Installable for PacmanInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        if specs.is_empty() {
            return Ok(());
        }

        let mut args = vec!["-S", "--noconfirm", "--needed"];
        let names: Vec<String> = specs.iter().map(|s| s.name.clone()).collect();
        for name in &names {
            args.push(name);
        }

        info!("Pacman: Installing {} package(s)...", specs.len());
        self.core
            .executor
            .run_exclusive("pacman", "pacman", &args, sudo)
            .await?;
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        if names.is_empty() {
            return Ok(());
        }

        let mut args = vec!["-Rs", "--noconfirm"];
        for name in names {
            args.push(name);
        }

        info!("Pacman: Removing {} package(s)...", names.len());
        self.core
            .executor
            .run_exclusive("pacman", "pacman", &args, sudo)
            .await?;
        Ok(())
    }
}

pub struct PacmanQueryable {
    pub core: Arc<PacmanBackendCore>,
}

#[async_trait]
impl Queryable for PacmanQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .run_output("pacman", &["-Q"], false)
            .await?;
        Ok(pacman::parse_list(&output))
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .run_output("pacman", &["-Qe"], false)
            .await?;
        Ok(pacman::parse_list(&output))
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}

pub struct PacmanSearchable {
    pub core: Arc<PacmanBackendCore>,
}

#[async_trait]
impl Searchable for PacmanSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .search_output("pacman", &["-Ss", query], false)
            .await?;
        Ok(pacman::parse_search(&output))
    }
}

pub struct PacmanEnumerable {
    pub core: Arc<PacmanBackendCore>,
}

#[async_trait]
impl Enumerable for PacmanEnumerable {
    /// `-Ssq` is the search form that prints bare names from the sync databases, with no
    /// query — the catalogue, which is what II.15's `re:` expands against. `pacman -Ss` (what
    /// [`PacmanSearchable`] runs) matches descriptions too and cannot answer a name pattern.
    async fn available_names(&self) -> Result<Vec<String>> {
        let output = self
            .core
            .executor
            .run_output("pacman", &["-Ssq"], false)
            .await?;
        Ok(output
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect())
    }
}

pub struct PacmanRepoManager {
    pub core: Arc<PacmanBackendCore>,
}

#[async_trait]
impl RepoManager for PacmanRepoManager {
    /// Drop-in policy: write `/etc/pacman.d/linix-<name>.conf` and add a single
    /// `Include = ...` line to `/etc/pacman.conf` (never rewriting its body). The whole
    /// write runs as root via `sh -c`; name/url are validated to be shell-safe first.
    async fn add_repo(&self, name: &str, url: &str, sudo: bool) -> Result<()> {
        validate_repo_name(name)?;
        validate_repo_url(url)?;
        let file = format!("/etc/pacman.d/linix-{}.conf", name);
        let include = format!("Include = {}", file);
        let script = format!(
            "set -e; \
             printf '[%s]\\nServer = %s\\n' '{name}' '{url}' > '{file}'; \
             grep -qxF '{include}' /etc/pacman.conf || printf '\\n%s\\n' '{include}' >> /etc/pacman.conf",
            name = name, url = url, file = file, include = include
        );
        info!("Pacman: Adding repository '{}' (drop-in {})...", name, file);
        self.core.executor.run("sh", &["-c", &script], sudo).await?;
        Ok(())
    }

    /// Delete the drop-in file and strip its `Include` line from `/etc/pacman.conf`.
    async fn remove_repo(&self, name: &str, sudo: bool) -> Result<()> {
        validate_repo_name(name)?;
        let file = format!("/etc/pacman.d/linix-{}.conf", name);
        // Custom sed delimiter '#' avoids escaping the slashes in the path.
        let script = format!(
            "rm -f '{file}'; sed -i '\\#Include = {file}#d' /etc/pacman.conf",
            file = file
        );
        info!("Pacman: Removing repository '{}'...", name);
        self.core.executor.run("sh", &["-c", &script], sudo).await?;
        Ok(())
    }

    /// List configured repositories via `pacman-conf`, resolving each repo's Server.
    async fn list_repos(&self) -> Result<Vec<(String, String)>> {
        let names = self
            .core
            .executor
            .run_output("pacman-conf", &["--repo-list"], false)
            .await?;
        let mut repos = Vec::new();
        for name in names.lines().map(|l| l.trim()).filter(|l| !l.is_empty()) {
            let server = self
                .core
                .executor
                .run_output("pacman-conf", &["-r", name, "Server"], false)
                .await
                .ok()
                .and_then(|s| s.lines().next().map(|l| l.trim().to_string()))
                .unwrap_or_default();
            repos.push((name.to_string(), server));
        }
        Ok(repos)
    }
}

pub struct PacmanUpgradable {
    pub core: Arc<PacmanBackendCore>,
}

#[async_trait]
impl Upgradable for PacmanUpgradable {
    async fn update(&self, sudo: bool) -> Result<()> {
        info!("Pacman: Refreshing package databases...");
        self.core.executor.run("pacman", &["-Sy"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("Pacman: Upgrading system packages...");
        self.core
            .executor
            .run_exclusive("pacman", "pacman", &["-Syu", "--noconfirm"], sudo)
            .await?;
        Ok(())
    }

    async fn list_orphans(&self) -> Result<Vec<String>> {
        let orphans = self
            .core
            .executor
            .run_output("pacman", &["-Qdtq"], false)
            .await?;
        Ok(orphans
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect())
    }

    async fn clean_cache(&self, sudo: bool) -> Result<()> {
        info!("Pacman: Clearing the package cache...");
        let args = vec!["-Sc", "--noconfirm"];
        self.core
            .executor
            .run_exclusive("pacman", "pacman", &args, sudo)
            .await?;
        Ok(())
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let core = Arc::new(PacmanBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(PacmanInstallable { core: core.clone() }))
            .with_queryable(Arc::new(PacmanQueryable { core: core.clone() }))
            .with_searchable(Arc::new(PacmanSearchable { core: core.clone() }))
            .with_enumerable(Arc::new(PacmanEnumerable { core: core.clone() }))
            .with_upgradable(Arc::new(PacmanUpgradable { core: core.clone() }))
            .with_repo_manager(Arc::new(PacmanRepoManager { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}
