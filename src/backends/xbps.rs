use crate::core::{
    BackendCore, CommandExecutor, Installable, MetadataProvider, Package, PackageSpec, Queryable,
    Result, Searchable, Upgradable,
};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;

/// Split an XBPS `pkgver` token (`<name>-<version>_<revision>`, e.g. `bash-5.2.15_2`)
/// into `(name, version)`. The version always begins with a digit after the final `-`,
/// which is what lets us separate it from names that themselves contain dashes
/// (`xbps-triggers-0.128_1` → `("xbps-triggers", "0.128_1")`).
fn split_pkgver(tok: &str) -> Option<(&str, &str)> {
    let (name, ver) = tok.rsplit_once('-')?;
    if ver.chars().next()?.is_ascii_digit() {
        Some((name, ver))
    } else {
        None
    }
}

/// XBPS spreads its operations across three binaries (`xbps-install`, `xbps-remove`,
/// `xbps-query`), so it is a specialized backend rather than a single-binary generic one.
pub struct XbpsBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl XbpsBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "xbps".to_string(),
        }
    }

    /// Parse `xbps-query -l` / `xbps-query -m` output. `-l` lines are prefixed with a
    /// two-character state flag (`ii <pkgver> <desc>`); `-m` lines are the bare pkgver.
    /// Either way the pkgver is the first token that parses, so this handles both.
    fn parse_query_list(output: &str) -> Vec<Package> {
        let mut packages = Vec::new();
        for line in output.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Some((name, ver)) = line.split_whitespace().find_map(split_pkgver) {
                packages.push(Package::with_version(name, ver, "xbps"));
            }
        }
        packages
    }

    /// Parse `xbps-query -Rs <query>` output. Lines look like
    /// `[-] bash-5.2.15_2   The GNU Bourne Again Shell` where `[*]` means installed.
    fn parse_search(output: &str) -> Vec<Package> {
        let mut packages = Vec::new();
        for line in output.lines() {
            let tokens: Vec<&str> = line.split_whitespace().collect();
            let Some(pos) = tokens.iter().position(|t| split_pkgver(t).is_some()) else {
                continue;
            };
            let (name, ver) = split_pkgver(tokens[pos]).unwrap();
            let mut pkg = Package::with_version(name, ver, "xbps");
            let desc = tokens[pos + 1..].join(" ");
            if !desc.is_empty() {
                pkg.properties.insert("description".into(), desc);
            }
            packages.push(pkg);
        }
        packages
    }
}

#[async_trait]
impl BackendCore for XbpsBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("xbps-install")
    }
    fn probes(&self) -> Vec<String> {
        vec!["xbps-install".into()]
    }
    fn needs_root(&self) -> bool {
        true
    }
}

#[async_trait]
impl MetadataProvider for XbpsBackendCore {
    async fn get_dependencies(&self, name: &str) -> Result<Vec<String>> {
        // `xbps-query -x <pkg>` lists run-time dependencies as version-constrained
        // patterns (e.g. `glibc>=2.36_1`); strip the constraint to the bare name.
        let mut args = vec!["-x".to_string()];
        crate::core::argv::push_names(&mut args, "xbps-query", [name]);
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let output = self
            .executor
            .run_output("xbps-query", &arg_refs, false)
            .await
            .unwrap_or_default();
        Ok(output
            .lines()
            .filter_map(|l| l.split(['>', '<', '=']).next())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect())
    }
}

pub struct XbpsInstallable {
    pub core: Arc<XbpsBackendCore>,
}

#[async_trait]
impl Installable for XbpsInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        if specs.is_empty() {
            return Ok(());
        }
        // XBPS is a rolling distribution: exact-version pinning is not part of its model
        // (mirrors the pacman/rolling stance), so `version` options are intentionally
        // ignored here rather than silently mis-encoded.
        let mut args = vec!["-Sy".to_string()];
        crate::core::argv::push_names(
            &mut args,
            "xbps-install",
            specs.iter().map(|s| s.name.as_str()),
        );
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        info!("XBPS: Installing {} package(s)...", specs.len());
        self.core
            .executor
            .run_exclusive("xbps", "xbps-install", &arg_refs, sudo)
            .await?;
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        if names.is_empty() {
            return Ok(());
        }
        let mut args = vec!["-y".to_string()];
        crate::core::argv::push_names(&mut args, "xbps-remove", names);
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        info!("XBPS: Removing {} package(s)...", names.len());
        self.core
            .executor
            .run_exclusive("xbps", "xbps-remove", &arg_refs, sudo)
            .await?;
        Ok(())
    }
}

pub struct XbpsQueryable {
    pub core: Arc<XbpsBackendCore>,
}

#[async_trait]
impl Queryable for XbpsQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .run_output("xbps-query", &["-l"], false)
            .await?;
        Ok(XbpsBackendCore::parse_query_list(&output))
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // `-m` lists only packages registered as explicitly (manually) installed.
        let output = self
            .core
            .executor
            .run_output("xbps-query", &["-m"], false)
            .await?;
        Ok(XbpsBackendCore::parse_query_list(&output))
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}

pub struct XbpsSearchable {
    pub core: Arc<XbpsBackendCore>,
}

#[async_trait]
impl Searchable for XbpsSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let mut args = vec!["-Rs".to_string()];
        crate::core::argv::push_names(&mut args, "xbps-query", [query]);
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let output = self
            .core
            .executor
            .search_output("xbps-query", &arg_refs, false)
            .await?;
        Ok(XbpsBackendCore::parse_search(&output))
    }
}

pub struct XbpsUpgradable {
    pub core: Arc<XbpsBackendCore>,
}

#[async_trait]
impl Upgradable for XbpsUpgradable {
    async fn update(&self, sudo: bool) -> Result<()> {
        info!("XBPS: Synchronizing repository index...");
        self.core
            .executor
            .run("xbps-install", &["-S"], sudo)
            .await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        info!("XBPS: Upgrading system packages...");
        self.core
            .executor
            .run_exclusive("xbps", "xbps-install", &["-Suy"], sudo)
            .await?;
        Ok(())
    }

    async fn list_orphans(&self) -> Result<Vec<String>> {
        let out = self
            .core
            .executor
            .run_output("xbps-query", &["-O"], false)
            .await?;
        Ok(out
            .lines()
            .filter_map(|l| l.split_whitespace().next())
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect())
    }

    async fn clean_cache(&self, sudo: bool) -> Result<()> {
        info!("XBPS: Clearing the package cache...");
        self.core
            .executor
            .run_exclusive("xbps", "xbps-remove", &["-Oy"], sudo)
            .await?;
        Ok(())
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let core = Arc::new(XbpsBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(XbpsInstallable { core: core.clone() }))
            .with_queryable(Arc::new(XbpsQueryable { core: core.clone() }))
            .with_searchable(Arc::new(XbpsSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(XbpsUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_query_list_with_state_flags() {
        let out = "\
ii bash-5.2.15_2            The GNU Bourne Again Shell
ii xbps-triggers-0.128_1    XBPS triggers for Void Linux
";
        let pkgs = XbpsBackendCore::parse_query_list(out);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "bash");
        assert_eq!(pkgs[0].version.as_deref(), Some("5.2.15_2"));
        assert_eq!(pkgs[0].backend, "xbps");
        // Name with an internal dash must not be split at the wrong hyphen.
        assert_eq!(pkgs[1].name, "xbps-triggers");
        assert_eq!(pkgs[1].version.as_deref(), Some("0.128_1"));
    }

    #[test]
    fn parses_bare_pkgver_list() {
        let pkgs = XbpsBackendCore::parse_query_list("curl-8.4.0_1\ngit-2.42.0_1\n");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "curl");
        assert_eq!(pkgs[1].name, "git");
    }

    #[tokio::test]
    async fn every_xbps_binary_ends_its_options_before_the_names() {
        let vfs = Arc::new(dashmap::DashMap::new());
        let mock = Arc::new(crate::core::executor::MockExecutor::new(vfs.clone()));
        let exec = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(dashmap::DashMap::new()),
        );
        let core = Arc::new(XbpsBackendCore::new(exec));

        let spec = PackageSpec {
            name: "ripgrep".into(),
            backend: "xbps".into(),
            ..Default::default()
        };
        XbpsInstallable { core: core.clone() }
            .install(&[spec], false)
            .await
            .unwrap();
        XbpsInstallable { core: core.clone() }
            .remove(&["ripgrep".to_string()], false)
            .await
            .unwrap();
        XbpsSearchable { core: core.clone() }
            .search("ripgrep")
            .await
            .ok();
        core.get_dependencies("ripgrep").await.unwrap();

        assert_eq!(
            mock.get_calls().await,
            vec![
                "xbps-install -Sy -- ripgrep",
                "xbps-remove -y -- ripgrep",
                "xbps-query -Rs -- ripgrep",
                "xbps-query -x -- ripgrep",
            ]
        );
    }

    #[tokio::test]
    async fn an_empty_removal_emits_no_terminator() {
        let vfs = Arc::new(dashmap::DashMap::new());
        let mock = Arc::new(crate::core::executor::MockExecutor::new(vfs.clone()));
        let exec = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(dashmap::DashMap::new()),
        );
        let core = Arc::new(XbpsBackendCore::new(exec));
        XbpsInstallable { core: core.clone() }
            .remove(&[], false)
            .await
            .unwrap();
        assert!(mock.get_calls().await.is_empty());
    }

    #[test]
    fn parses_search_with_install_markers() {
        let out = "[*] bash-5.2.15_2   The GNU Bourne Again Shell\n[-] zsh-5.9_2   The Z shell\n";
        let pkgs = XbpsBackendCore::parse_search(out);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "bash");
        assert_eq!(
            pkgs[0].properties.get("description").map(String::as_str),
            Some("The GNU Bourne Again Shell")
        );
        assert_eq!(pkgs[1].name, "zsh");
        assert_eq!(pkgs[1].version.as_deref(), Some("5.9_2"));
    }
}
