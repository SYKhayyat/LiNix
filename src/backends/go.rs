// The Go toolchain as a LiNix backend. Go is a poor fit for the generic CLI-config model:
// `go install pkg@version` installs a binary, but there is no `go uninstall`, no command
// that lists globally-installed binaries with their module paths, and no CLI search
// (pkg.go.dev is web-only). So this is a dedicated backend:
//
//   * install — `go install <module>@<version|latest>`
//   * list    — enumerate the Go bin dir (GOBIN → `go env GOPATH`/bin → ~/go/bin) and read
//               each binary's originating module path via `go version -m`
//   * remove  — delete the installed binary (Go ships no uninstaller)
//   * upgrade — reinstall each module at @latest
//   * search  — unsupported (no Searchable capability is attached)

use crate::core::{
    BackendCore, CommandExecutor, Error, Installable, MetadataProvider, Package, PackageSpec,
    Queryable, Result, Upgradable,
};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Clone)]
pub struct GoBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl GoBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "go".to_string(),
        }
    }

    /// Resolve the directory Go installs binaries into: `$GOBIN`, else `$(go env
    /// GOPATH)/bin`, else `~/go/bin`.
    async fn bin_dir(&self) -> Result<PathBuf> {
        if let Ok(gobin) = std::env::var("GOBIN") {
            if !gobin.trim().is_empty() {
                return Ok(PathBuf::from(gobin));
            }
        }
        if let Ok(gopath) = self
            .executor
            .run_output("go", &["env", "GOPATH"], false)
            .await
        {
            if let Some(bin) = gopath_bin(&gopath) {
                return Ok(bin);
            }
        }
        let home = dirs::home_dir()
            .ok_or_else(|| Error::Other("Could not determine home directory for Go".into()))?;
        Ok(home.join("go").join("bin"))
    }

    /// The on-disk binary name for a module/spec: the last path segment, minus any
    /// `@version`, plus the platform executable extension.
    fn binary_name(spec: &str) -> String {
        let base = spec.split('@').next().unwrap_or(spec);
        let base = base.rsplit('/').next().unwrap_or(base);
        if cfg!(windows) {
            format!("{}.exe", base)
        } else {
            base.to_string()
        }
    }
}

/// The `bin` directory belonging to the first entry of a `go env GOPATH` reading.
///
/// GOPATH is a list in the platform's own separator, so it is split by the platform's own
/// rule: on Windows the separator is `;` and a colon is part of every drive letter.
fn gopath_bin(raw: &str) -> Option<PathBuf> {
    let line = raw.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        return None;
    }
    let first = std::env::split_paths(line).next()?;
    if first.as_os_str().is_empty() {
        return None;
    }
    Some(first.join("bin"))
}

/// Parse `go version -m <bin>` into the name `go install` would take, and its version. The
/// block looks like:
///   /path/goimports: go1.26.5
///   \tpath\tgolang.org/x/tools/cmd/goimports
///   \tmod\tgolang.org/x/tools\tv0.48.0\th1:...
///
/// `path` is the identity and `mod` only carries the version: a program in a subdirectory of
/// its module has two different names here, and the one `sync` compares against a declaration
/// — the one `go install` accepts — is `path`.
fn parse_go_version_m(output: &str) -> Option<(String, Option<String>)> {
    let mut package_path: Option<String> = None;
    let mut module: Option<(String, Option<String>)> = None;
    for line in output.lines() {
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some("path") => {
                if let Some(p) = fields.next() {
                    package_path.get_or_insert_with(|| p.to_string());
                }
            }
            Some("mod") if module.is_none() => {
                if let Some(m) = fields.next() {
                    module = Some((m.to_string(), fields.next().map(|s| s.to_string())));
                }
            }
            _ => {}
        }
    }
    let version = module.as_ref().and_then(|(_, v)| v.clone());
    package_path
        .or_else(|| module.map(|(m, _)| m))
        .map(|name| (name, version))
}

#[async_trait]
impl BackendCore for GoBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("go")
    }
    fn probes(&self) -> Vec<String> {
        vec!["go".into()]
    }
    fn needs_root(&self) -> bool {
        false
    }
}

#[async_trait]
impl MetadataProvider for GoBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub struct GoInstallable {
    pub core: Arc<GoBackendCore>,
}

fn install_argv(target: &str) -> Vec<String> {
    let mut args = vec!["install".to_string()];
    crate::core::argv::push_names(&mut args, "go", [target]);
    args
}

#[async_trait]
impl Installable for GoInstallable {
    async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        for spec in specs {
            // `go install` requires an @version suffix in module mode. Honor an explicit
            // pinned version; otherwise use @latest. A name that already carries @ is passed
            // through unchanged.
            let target = if spec.name.contains('@') {
                spec.name.clone()
            } else {
                let ver = spec
                    .options
                    .get("version")
                    .filter(|v| crate::backends::concrete_version(v))
                    .map(|s| s.as_str())
                    .unwrap_or("latest");
                format!("{}@{}", spec.name, ver)
            };
            info!("Go: Installing {}...", target);
            let args = install_argv(&target);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core
                .executor
                .run_exclusive("go", "go", &arg_refs, false)
                .await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        // Go has no uninstaller; removal deletes the installed binary. Convergent: a binary
        // that is already gone is treated as successfully removed.
        let dir = self.core.bin_dir().await?;
        for name in names {
            let bin = dir.join(GoBackendCore::binary_name(name));
            if self.core.executor.dry_run {
                info!("Go: [DRY-RUN] would delete {}", bin.display());
                continue;
            }
            match std::fs::remove_file(&bin) {
                Ok(_) => info!("Go: Removed {}", bin.display()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    warn!(
                        "Go: binary for '{}' not found at {}, nothing to remove",
                        name,
                        bin.display()
                    );
                }
                Err(e) => {
                    return Err(Error::Io(format!(
                        "failed to remove {}: {}",
                        bin.display(),
                        e
                    )))
                }
            }
        }
        Ok(())
    }
}

pub struct GoQueryable {
    pub core: Arc<GoBackendCore>,
}

impl GoQueryable {
    async fn scan(&self) -> Result<Vec<Package>> {
        let dir = self.core.bin_dir().await?;
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            // No bin dir yet ⇒ nothing installed.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => {
                return Err(Error::Io(format!(
                    "failed to read {}: {}",
                    dir.display(),
                    e
                )))
            }
        };
        let mut packages = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            let path_str = path.to_string_lossy().to_string();
            let mut ver_args = vec!["version".to_string(), "-m".to_string()];
            crate::core::argv::push_names(&mut ver_args, "go", [&path_str]);
            let ver_refs: Vec<&str> = ver_args.iter().map(String::as_str).collect();
            let (name, version) = match self
                .core
                .executor
                .run_output("go", &ver_refs, false)
                .await
                .ok()
                .and_then(|o| parse_go_version_m(&o))
            {
                Some((module, ver)) => (module, ver),
                // Not a Go-built binary (or older Go) — fall back to the file name.
                None => (file_name.trim_end_matches(".exe").to_string(), None),
            };
            let mut pkg = match version {
                Some(v) => Package::with_version(&name, &v, "go"),
                None => Package::new(name, "go"),
            };
            pkg.properties.insert("bin_path".into(), path_str);
            packages.push(pkg);
        }
        Ok(packages)
    }
}

#[async_trait]
impl Queryable for GoQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        self.scan().await
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.scan().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.scan().await?;
        // Match on the full module path or its trailing binary segment (`github.com/x/fzf`
        // vs `fzf`), so either form the user typed resolves.
        Ok(all
            .into_iter()
            .find(|p| p.name == name || p.name.rsplit('/').next() == Some(name)))
    }
}

pub struct GoUpgradable {
    pub core: Arc<GoBackendCore>,
}

#[async_trait]
impl Upgradable for GoUpgradable {
    async fn update(&self, _sudo: bool) -> Result<()> {
        Ok(())
    }

    async fn upgrade(&self, _sudo: bool) -> Result<()> {
        info!("Go: Upgrading all installed binaries to @latest...");
        let q = GoQueryable {
            core: self.core.clone(),
        };
        for pkg in q.scan().await? {
            // Only module paths can be reinstalled; skip bare-filename fallbacks.
            if !pkg.name.contains('/') {
                continue;
            }
            let args = install_argv(&format!("{}@latest", pkg.name));
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let _ = self
                .core
                .executor
                .run_exclusive("go", "go", &arg_refs, false)
                .await;
        }
        Ok(())
    }
}

/// Search is intentionally omitted: Go has no CLI package search (pkg.go.dev is web-only).
pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let core = Arc::new(GoBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GoInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GoQueryable { core: core.clone() }))
            .with_upgradable(Arc::new(GoUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_module_and_version_from_mod_line() {
        let out = "/root/go/bin/fzf: go1.21.0\n\tpath\tgithub.com/junegunn/fzf\n\tmod\tgithub.com/junegunn/fzf\tv0.42.0\th1:abcd\n";
        let (module, version) = parse_go_version_m(out).unwrap();
        assert_eq!(module, "github.com/junegunn/fzf");
        assert_eq!(version.as_deref(), Some("v0.42.0"));
    }

    #[test]
    fn falls_back_to_path_when_no_mod() {
        let out = "/root/go/bin/tool: go1.21.0\n\tpath\texample.com/tool\n";
        let (module, version) = parse_go_version_m(out).unwrap();
        assert_eq!(module, "example.com/tool");
        assert_eq!(version, None);
    }

    #[test]
    fn binary_name_strips_path_and_version() {
        let expected = if cfg!(windows) { "fzf.exe" } else { "fzf" };
        assert_eq!(
            GoBackendCore::binary_name("github.com/junegunn/fzf@latest"),
            expected
        );
        assert_eq!(GoBackendCore::binary_name("fzf"), expected);
    }

    #[test]
    fn the_module_path_goes_behind_the_terminator() {
        assert_eq!(
            install_argv("github.com/junegunn/fzf@latest"),
            ["install", "--", "github.com/junegunn/fzf@latest"]
        );
    }

    #[tokio::test]
    async fn install_and_upgrade_end_their_options_before_the_module() {
        let vfs = Arc::new(dashmap::DashMap::new());
        let mock = Arc::new(crate::core::executor::MockExecutor::new(vfs.clone()));
        let exec = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(dashmap::DashMap::new()),
        );
        let core = Arc::new(GoBackendCore::new(exec));

        let spec = PackageSpec {
            name: "github.com/junegunn/fzf".into(),
            backend: "go".into(),
            ..Default::default()
        };
        GoInstallable { core: core.clone() }
            .install(&[spec], false)
            .await
            .unwrap();

        assert_eq!(
            mock.get_calls().await,
            vec!["go install -- github.com/junegunn/fzf@latest"]
        );
    }

    #[test]
    fn non_go_binary_yields_none() {
        assert!(parse_go_version_m("some random text\nno module info here\n").is_none());
    }

    /// Captured from `go version -m` on a real install of
    /// `golang.org/x/tools/cmd/goimports@latest` (go1.26.5).
    const SUBPACKAGE: &str = include_str!("../../tests/fixtures/go/version-m-subpackage.txt");
    /// Captured from a real install of `golang.org/x/example/hello@latest`, where the
    /// package sits at the module root so `path` and `mod` agree.
    const MODULE_ROOT: &str = include_str!("../../tests/fixtures/go/version-m-module-root.txt");
    /// Captured from `go env GOPATH` on Windows — a drive letter, so a colon.
    const GOPATH_WINDOWS: &str = include_str!("../../tests/fixtures/go/env-gopath-windows.txt");

    /// `sync` compares what was declared against what `list` reports, so the name `list`
    /// reports must be the name `go install` takes — the package path. Preferring the `mod`
    /// line reported `golang.org/x/tools` for a binary declared as
    /// `golang.org/x/tools/cmd/goimports`: never equal, so drift that never converges, and an
    /// `upgrade` that reinstalls a module with no program in it.
    #[test]
    fn the_identity_is_the_package_path_not_the_module() {
        let (name, version) = parse_go_version_m(SUBPACKAGE).unwrap();
        assert_eq!(name, "golang.org/x/tools/cmd/goimports");
        assert_eq!(version.as_deref(), Some("v0.48.0"));
    }

    #[test]
    fn a_package_at_the_module_root_still_carries_its_version() {
        let (name, version) = parse_go_version_m(MODULE_ROOT).unwrap();
        assert_eq!(name, "golang.org/x/example/hello");
        assert_eq!(
            version.as_deref(),
            Some("v0.0.0-20250915201037-7f05d217867b")
        );
    }

    /// GOPATH is a list in the platform's own separator. Splitting on `:` as well as `;`
    /// decapitated every Windows path at the drive letter, so the bin dir resolved to `C\bin`,
    /// which does not exist — and a missing bin dir is reported as "nothing installed".
    #[test]
    fn a_windows_gopath_keeps_its_drive_letter() {
        let bin = gopath_bin(GOPATH_WINDOWS).unwrap();
        assert_eq!(bin, PathBuf::from(r"C:\Users\Administrator\go").join("bin"));
    }

    #[test]
    fn the_first_entry_of_a_gopath_list_owns_bin() {
        let (list, first) = if cfg!(windows) {
            (r"C:\one;D:\two", r"C:\one")
        } else {
            ("/one:/two", "/one")
        };
        assert_eq!(gopath_bin(list).unwrap(), PathBuf::from(first).join("bin"));
    }

    #[test]
    fn an_empty_gopath_resolves_to_nothing_rather_than_to_bin() {
        assert!(gopath_bin("").is_none());
        assert!(gopath_bin("   \n").is_none());
    }
}
