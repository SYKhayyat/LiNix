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
    /// Whether the cmdlets this backend is made of actually resolve. Asked once: it costs a
    /// PowerShell start-up, and `is_available` is called on every registry pass.
    cmdlets: Arc<std::sync::OnceLock<bool>>,
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
            cmdlets: Arc::new(std::sync::OnceLock::new()),
        }
    }

    /// Does `Get-InstalledPSResource` actually resolve in this shell?
    ///
    /// The question that was being asked instead was "does PowerShell exist", which on Windows
    /// is always yes — so `check health` printed `[READY] psresource` and then every operation
    /// died with *The term 'Get-InstalledPSResource' is not recognized*. PSResourceGet supplies
    /// those cmdlets and does not ship with Windows PowerShell 5.1.
    ///
    /// `krew` had the right shape all along: it probes `kubectl` **and** `kubectl-krew`. A
    /// backend must probe the thing that has to work, not the thing that hosts it.
    fn cmdlets_present(&self) -> bool {
        *self.cmdlets.get_or_init(|| {
            if !self.executor.command_exists_sync(&self.shell) {
                return false;
            }
            // Timed like any other child: this is the one availability probe that starts a
            // process rather than reading PATH, so a `--timings` run that left it out would
            // show a probe pass costing more than every command in it.
            let timing = crate::core::timing::begin();
            let mut probe_cmd = std::process::Command::new(&self.shell);
            probe_cmd
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "if (Get-Command Get-InstalledPSResource -ErrorAction SilentlyContinue) \
                     { exit 0 } else { exit 1 }",
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            let present = crate::core::blocking::command_status(&mut probe_cmd)
                .map(|s| s.success())
                .unwrap_or(false);
            crate::core::timing::end(
                timing,
                &self.shell,
                &["-Command Get-InstalledPSResource".to_string()],
            );
            present
        })
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
        self.cmdlets_present()
    }
    /// The module, not the host. Naming `powershell` here would send a Windows user looking
    /// for a program they already have.
    fn probes(&self) -> Vec<String> {
        vec!["Microsoft.PowerShell.PSResourceGet".into()]
    }
    fn needs_root(&self) -> bool {
        false
    }

    /// The one backend that overrides the shared message, because the thing it is missing is
    /// not a program: a PowerShell module is not "on PATH", and telling a user to put it there
    /// is the same species of wrong answer as telling them to install `lvm`. The exception is
    /// here and stated rather than spread through the renderer as a guess about which names
    /// look like modules.
    async fn check_health(&self) -> Result<crate::core::HealthReport> {
        use crate::core::{HealthReport, HealthStatus};
        if self.is_available() {
            return Ok(HealthReport {
                status: HealthStatus::Ok,
                message: None,
            });
        }
        Ok(HealthReport {
            status: HealthStatus::Absent,
            message: Some(format!(
                "`{}` has no PSResourceGet cmdlets, so the `psresource` backend cannot run. \
                 Install it with: {} -Command \"Install-Module Microsoft.PowerShell.PSResourceGet \
                 -Scope CurrentUser\"",
                self.shell, self.shell
            )),
        })
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
                .one("version")
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

    async fn remove(
        &self,
        names: &[String],
        _sudo: bool,
        _reaped: crate::app::sync::guard::Reaped,
    ) -> Result<()> {
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
    fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
        (self.core.executor.installed_listings(), &self.core.name)
    }

    async fn fetch_installed(&self) -> Result<Vec<Package>> {
        // Emit "Name Version" lines so parsing is trivial and independent of the
        // NuGetVersion type's JSON shape.
        let script = r#"Get-InstalledPSResource | ForEach-Object { "$($_.Name) $($_.Version)" }"#;
        let output = self.core.run_ps(script).await?;
        Ok(parse_simple_list(&output, "psresource")?)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.installed_listing().await?;
        Ok(all.iter().find(|p| p.name == name).cloned())
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
        // A search that reads nothing is a search with no results, which is an answer the user
        // asked for and can see. Only the installed listing above may not guess.
        Ok(parse_simple_list(&output, "psresource").unwrap_or_default())
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
