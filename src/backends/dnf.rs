use crate::core::{
    BackendCore, CommandExecutor, Error, Installable, MetadataProvider, Package, PackageSpec,
    Queryable, RepoManager, Result, Searchable, Upgradable,
};
use crate::parsers::dnf;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;

/// Reject repo names that could escape the `/etc/yum.repos.d/<name>.repo` path or be
/// used for command injection.
fn validate_repo_name(name: &str) -> Result<()> {
    if !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        Ok(())
    } else {
        Err(Error::Other(format!("Invalid repo name: '{}'", name)))
    }
}

pub struct DnfBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl DnfBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor: executor.with_exit_policy(crate::core::exit_policy::for_manager("dnf")),
            name: "dnf".to_string(),
        }
    }
}

#[async_trait]
impl BackendCore for DnfBackendCore {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("dnf")
    }
    fn probes(&self) -> Vec<String> {
        vec!["dnf".into()]
    }

    fn needs_root(&self) -> bool {
        true
    }
}

#[async_trait]
impl MetadataProvider for DnfBackendCore {
    async fn get_dependencies(&self, name: &str) -> Result<Vec<String>> {
        let output = self
            .executor
            .run_output(
                "dnf",
                &[
                    "repoquery",
                    "--requires",
                    "--resolve",
                    "--queryformat",
                    "%{name}",
                    name,
                ],
                false,
            )
            .await?;
        Ok(output
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect())
    }
}

pub struct DnfInstallable {
    pub core: Arc<DnfBackendCore>,
}

#[async_trait]
impl Installable for DnfInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        if specs.is_empty() {
            return Ok(());
        }
        let mut args = vec!["install", "-y"];
        // Reproducible installs: dnf pins with `name-version`.
        let names: Vec<String> = specs
            .iter()
            .map(|s| match s.options.get("version") {
                Some(v) if crate::backends::concrete_version(v) => format!("{}-{}", s.name, v),
                _ => s.name.clone(),
            })
            .collect();
        for name in &names {
            args.push(name);
        }

        info!("DNF: Installing {} package(s)...", specs.len());
        self.core
            .executor
            .run_exclusive("dnf", "dnf", &args, sudo)
            .await?;
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        if names.is_empty() {
            return Ok(());
        }
        let mut args = vec!["remove", "-y"];
        for name in names {
            args.push(name);
        }

        info!("DNF: Removing {} package(s)...", names.len());
        self.core
            .executor
            .run_exclusive("dnf", "dnf", &args, sudo)
            .await?;
        Ok(())
    }
}

pub struct DnfQueryable {
    pub core: Arc<DnfBackendCore>,
}

#[async_trait]
impl Queryable for DnfQueryable {
    fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
        (self.core.executor.installed_listings(), &self.core.name)
    }

    async fn fetch_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .run_output(
                "rpm",
                &["-qa", "--queryformat", "%{NAME}|%{VERSION}\n"],
                false,
            )
            .await?;
        Ok(dnf::parse_rpm_qa(&output, "dnf"))
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .run_output(
                "dnf",
                &["repoquery", "--userinstalled", "--qf", "%{name}|%{version}"],
                false,
            )
            .await?;
        Ok(dnf::parse_rpm_qa(&output, "dnf"))
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}

pub struct DnfSearchable {
    pub core: Arc<DnfBackendCore>,
}

#[async_trait]
impl Searchable for DnfSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .search_output("dnf", &["search", query], false)
            .await?;
        Ok(dnf::parse_dnf_search(&output))
    }
}

pub struct DnfRepoManager {
    pub core: Arc<DnfBackendCore>,
}

#[async_trait]
impl RepoManager for DnfRepoManager {
    /// Add a repo via `dnf config-manager --add-repo <url>` (requires dnf-plugins-core).
    async fn add_repo(&self, name: &str, url: &str, sudo: bool) -> Result<()> {
        if url.trim().is_empty() {
            return Err(Error::Other(
                "dnf add_repo requires a repository URL".into(),
            ));
        }
        info!("DNF: Adding repository '{}' ({})...", name, url);
        self.core
            .executor
            .run_exclusive("dnf", "dnf", &["config-manager", "--add-repo", url], sudo)
            .await?;
        Ok(())
    }

    /// Remove the drop-in `.repo` file from `/etc/yum.repos.d/`.
    async fn remove_repo(&self, name: &str, sudo: bool) -> Result<()> {
        validate_repo_name(name)?;
        let path = format!("/etc/yum.repos.d/{}.repo", name);
        info!("DNF: Removing repository file {}...", path);
        self.core.executor.run("rm", &["-f", &path], sudo).await?;
        Ok(())
    }

    /// List configured repositories via `dnf repolist` (id + display name).
    async fn list_repos(&self) -> Result<Vec<(String, String)>> {
        let output = self
            .core
            .executor
            .run_output("dnf", &["repolist", "--all"], false)
            .await?;
        let mut repos = Vec::new();
        for line in output.lines().skip(1) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut it = line.splitn(2, char::is_whitespace);
            if let Some(id) = it.next() {
                let name = it.next().unwrap_or("").trim().to_string();
                repos.push((id.to_string(), name));
            }
        }
        Ok(repos)
    }
}

pub struct DnfUpgradable {
    pub core: Arc<DnfBackendCore>,
}

#[async_trait]
impl Upgradable for DnfUpgradable {
    async fn update(&self, sudo: bool) -> Result<()> {
        self.core.executor.run("dnf", &["makecache"], sudo).await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("DNF: Upgrading system packages...");
        self.core
            .executor
            .run_exclusive("dnf", "dnf", &["upgrade", "-y"], sudo)
            .await?;
        Ok(())
    }

    async fn list_orphans(&self) -> Result<Vec<String>> {
        let out = self
            .core
            .executor
            .run_output(
                "dnf",
                &["repoquery", "--unneeded", "--queryformat", "%{name}"],
                false,
            )
            .await?;
        Ok(out
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect())
    }

    async fn clean_cache(&self, sudo: bool) -> Result<()> {
        info!("DNF: Clearing the package cache...");
        self.core
            .executor
            .run_exclusive("dnf", "dnf", &["clean", "all"], sudo)
            .await?;
        Ok(())
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let core = Arc::new(DnfBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(DnfInstallable { core: core.clone() }))
            .with_queryable(Arc::new(DnfQueryable { core: core.clone() }))
            .with_searchable(Arc::new(DnfSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(DnfUpgradable { core: core.clone() }))
            .with_repo_manager(Arc::new(DnfRepoManager { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}
