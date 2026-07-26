// PowerShell modules from the PowerShell Gallery, via the modern PSResourceGet module
// (Install-PSResource / Get-InstalledPSResource / ...). Unlike the other backends the
// "binary" is PowerShell itself and the package manager is a set of cmdlets, so this
// can't ride the generic (argv) backend: every op is a `-Command` script string. Because
// the package name is interpolated into that script, names are validated against a strict
// allowlist to foreclose command injection.

use crate::config::Config;
use crate::core::{
    BackendCapabilities, BackendCore, CommandExecutor, Error, Installable, MetadataProvider,
    Package, PackageSpec, Queryable, Result, Searchable, Upgradable,
};
use crate::parsers::common::parse_simple_list;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;

#[derive(Clone)]
pub struct PsResourceCore {
    pub executor: CommandExecutor,
    pub name: String,
    /// The PowerShell host to invoke: "pwsh" (7+) when present, else "powershell" (5.1).
    pub shell: String,
}

impl PsResourceCore {
    pub fn new(executor: CommandExecutor) -> Self {
        // Prefer PowerShell 7+ (pwsh); fall back to Windows PowerShell 5.1, which ships
        // with every supported Windows and can load PSResourceGet.
        let shell = if executor.command_exists_sync("pwsh") {
            "pwsh".to_string()
        } else {
            "powershell".to_string()
        };
        Self {
            executor,
            name: "psresource".to_string(),
            shell,
        }
    }

    async fn run_ps(&self, script: &str) -> Result<String> {
        self.executor
            .run_output(&self.shell, &Self::argv(script), false)
            .await
    }

    /// The same shell, for a question whose empty answer means "no such resource". A
    /// repository that could not be reached prints nothing, and must not be read as one
    /// that looked and found nothing.
    async fn search_ps(&self, script: &str) -> Result<String> {
        self.executor
            .search_output(&self.shell, &Self::argv(script), false)
            .await
    }

    /// The same shell, for a *change*. `run_output` hands back a failed cmdlet's empty
    /// output as success, so an install that never happened read as done.
    async fn change_ps(&self, script: &str) -> Result<()> {
        self.executor
            .run(&self.shell, &Self::argv(script), false)
            .await
            .map(|_| ())
    }

    fn argv(script: &str) -> [&str; 4] {
        ["-NoProfile", "-NonInteractive", "-Command", script]
    }
}

/// Accepts PowerShell module names: letters, digits, `.`, `_`, `-`. Everything else
/// (quotes, `;`, `$`, whitespace, backtick, ...) is rejected so an interpolated name
/// cannot break out of the `-Command` script.
fn validate_name(name: &str) -> Result<()> {
    if !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "Invalid PowerShell resource name: '{}'",
            name
        )))
    }
}

/// Like [`validate_name`] but also permits the `*`/`?` wildcards a search query may use.
fn validate_query(query: &str) -> Result<()> {
    if !query.is_empty()
        && query
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '*' | '?'))
    {
        Ok(())
    } else {
        Err(Error::Other(format!("Invalid search query: '{}'", query)))
    }
}

/// Accepts version specifiers (digits, dots, and pre-release markers).
fn validate_version(version: &str) -> Result<()> {
    if !version.is_empty()
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
    {
        Ok(())
    } else {
        Err(Error::Other(format!("Invalid version: '{}'", version)))
    }
}

#[async_trait]
impl BackendCore for PsResourceCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        self.executor.command_exists_sync(&self.shell)
    }
    fn needs_root(&self) -> bool {
        false
    }
}

#[async_trait]
impl MetadataProvider for PsResourceCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub struct PsResourceInstallable {
    pub core: Arc<PsResourceCore>,
}

#[async_trait]
impl Installable for PsResourceInstallable {
    async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        for spec in specs {
            validate_name(&spec.name)?;
            let mut script = format!(
                "Install-PSResource -Name '{}' -TrustRepository -AcceptLicense -Scope CurrentUser -Reinstall",
                spec.name
            );
            if let Some(ver) = spec
                .options
                .get("version")
                .filter(|v| crate::backends::concrete_version(v))
            {
                validate_version(ver)?;
                script.push_str(&format!(" -Version '{}'", ver));
            }
            info!("PSResource: Installing {}...", spec.name);
            self.core.change_ps(&script).await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        for name in names {
            validate_name(name)?;
            let script = format!("Uninstall-PSResource -Name '{}'", name);
            info!("PSResource: Uninstalling {}...", name);
            self.core.change_ps(&script).await?;
        }
        Ok(())
    }
}

pub struct PsResourceQueryable {
    pub core: Arc<PsResourceCore>,
}

#[async_trait]
impl Queryable for PsResourceQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        // Emit "Name Version" lines so parsing is trivial and independent of the
        // NuGetVersion type's JSON shape.
        let script = r#"Get-InstalledPSResource | ForEach-Object { "$($_.Name) $($_.Version)" }"#;
        let output = self.core.run_ps(script).await?;
        Ok(parse_simple_list(&output, "psresource"))
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}

pub struct PsResourceSearchable {
    pub core: Arc<PsResourceCore>,
}

#[async_trait]
impl Searchable for PsResourceSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        validate_query(query)?;
        let script = format!(
            r#"Find-PSResource -Name '{}' | Select-Object -First 50 | ForEach-Object {{ "$($_.Name) $($_.Version)" }}"#,
            query
        );
        let output = self.core.search_ps(&script).await?;
        Ok(parse_simple_list(&output, "psresource"))
    }
}

pub struct PsResourceUpgradable {
    pub core: Arc<PsResourceCore>,
}

#[async_trait]
impl Upgradable for PsResourceUpgradable {
    async fn update(&self, _sudo: bool) -> Result<()> {
        Ok(())
    }

    async fn upgrade(&self, _sudo: bool) -> Result<()> {
        info!("PSResource: Updating all installed resources...");
        let script = r#"Get-InstalledPSResource | ForEach-Object { Update-PSResource -Name $_.Name -TrustRepository -AcceptLicense }"#;
        self.core.change_ps(script).await?;
        Ok(())
    }
}

pub fn register(reg: &mut crate::backends::BackendRegistry, exec: &CommandExecutor, _cfg: &Config) {
    let core = Arc::new(PsResourceCore::new(exec.duplicate()));
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(PsResourceInstallable { core: core.clone() }))
            .with_queryable(Arc::new(PsResourceQueryable { core: core.clone() }))
            .with_searchable(Arc::new(PsResourceSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(PsResourceUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_name_accepts_module_names() {
        assert!(validate_name("Pester").is_ok());
        assert!(validate_name("PSReadLine").is_ok());
        assert!(validate_name("Az.Accounts").is_ok());
    }

    #[test]
    fn validate_name_rejects_injection() {
        assert!(validate_name("Pester'; rm -rf /; '").is_err());
        assert!(validate_name("foo bar").is_err());
        assert!(validate_name("$(evil)").is_err());
        assert!(validate_name("").is_err());
    }

    #[test]
    fn validate_query_allows_wildcards() {
        assert!(validate_query("Az*").is_ok());
        assert!(validate_query("PS?eadLine").is_ok());
        assert!(validate_query("bad;name").is_err());
    }

    #[test]
    fn validate_version_accepts_semver_like() {
        assert!(validate_version("2.10.3").is_ok());
        assert!(validate_version("1.0.0-beta1").is_ok());
        assert!(validate_version("1.0'; evil").is_err());
    }
}
