use crate::core::{
    BackendCore, CommandExecutor, Enumerable, Error, HealthReport, HealthStatus, Installable,
    MetadataProvider, Package, PackageSpec, Queryable, RepoManager, Result, Searchable, Upgradable,
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
            VersionPin::Inline(tmpl) => {
                vec![tmpl.replace("{name}", name).replace("{version}", version)]
            }
            VersionPin::Flag(flags) => {
                let mut out = vec![name.to_string()];
                out.extend(
                    flags
                        .iter()
                        .map(|f| f.replace("{name}", name).replace("{version}", version)),
                );
                out
            }
        }
    }
}

/// True when a version string represents a real pin (not "latest"/"*"/empty).
fn is_concrete_version(v: &str) -> bool {
    !v.is_empty() && v != "latest" && v != "*"
}

/// How a backend answers "which packages did the user actually ask for?" — the question
/// `adopt` must get right before it adopts anything into managed state.
///
/// This is stated per backend rather than inferred from the absence of config: "no manual
/// command configured" is ambiguous between *"listing everything is the correct answer"*
/// (winget has no dependencies) and *"we have no idea"* (pip does). Conflating the two is
/// how an entire dependency graph gets adopted and then purged.
#[derive(Debug, Clone)]
pub enum ManualListing {
    /// Every installed package was user-requested: the manager installs no dependencies
    /// of its own, so `list_installed` *is* the manual set (winget, choco, mas, dotnet).
    AllInstalled,
    /// The manager reports its explicit set via a command of its own.
    Command {
        /// Binary to run, when it is neither the backend nor `list_binary` (apt's manual
        /// set lives in `apt-mark`, a third binary distinct from its `dpkg-query` lister).
        /// `None` falls back to `list_binary`, then the backend name.
        binary: Option<String>,
        args: Vec<String>,
        format: ManualFormat,
    },
    /// The manager installs dependencies but exposes no way to tell them apart from what
    /// the user chose (pip, gem, zypper, pkgin). Adoption must skip the backend entirely.
    Unsupported,
}

/// The shape of a `ManualListing::Command`'s output.
#[derive(Debug, Clone, Copy)]
pub enum ManualFormat {
    /// Same shape as `list_args` output — reuse the backend's installed parser.
    SameAsInstalled,
    /// One bare package name per line, no versions (`apt-mark showmanual`).
    BareNames,
}

/// Configuration for the Generic Manager Strategy.
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    pub name: String,
    pub install_args: Vec<String>,
    pub remove_args: Vec<String>,
    pub list_args: Vec<String>,
    pub manual: ManualListing,
    /// Optional: args (run with `list_binary`) that report the packages the OS treats as
    /// essential, for the removal guard. `None` = the manager has no such concept.
    pub essential_args: Option<Vec<String>>,
    pub search_args: Vec<String>,
    /// Optional: if specified, use this binary for search instead of the backend name.
    pub search_binary: Option<String>,
    /// Optional: args that print every installable package name, one per line, and nothing
    /// else — what II.15's `re:` expands against. `None` means this manager cannot list its
    /// catalogue, which is the honest answer for every language registry, and a `re:` line
    /// naming it is refused rather than expanded to nothing.
    pub enumerate_args: Option<Vec<String>>,
    /// Optional: binary for `enumerate_args`, when the catalogue lives in a separate program
    /// (apt's is `apt-cache`, not `apt`).
    pub enumerate_binary: Option<String>,
    /// Optional: binary to run the LIST commands (`list_args`/`essential_args`) with,
    /// instead of the backend name. Required when a manager's query tool is a *separate*
    /// program — e.g. apt lists installed packages via `dpkg-query`, not `apt dpkg-query`.
    pub list_binary: Option<String>,
    pub upgrade_args: Vec<String>,
    pub update_args: Option<Vec<String>>,
    /// Native args for orphan/unused-dependency removal (e.g. apt `autoremove -y`).
    /// `None` means the backend has no orphan concept → `clean_orphans` reports Unsupported.
    pub orphan_args: Option<Vec<String>>,
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
                message: Some(format!(
                    "Binary for generic manager '{}' not found in PATH",
                    self.name
                )),
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
        // Dependency resolution is a read-only query — never escalate with sudo.
        let output = self
            .executor
            .run_output(&self.name, &arg_refs, false)
            .await?;

        // Extract clean package names. apt/zypper print labelled lines
        // ("Depends: libc6", "Requires: foo"); strip the "Label: " prefix and take the
        // bare name (dropping any version constraint / alternative). Backends that print
        // bare names (e.g. apk) pass through unchanged.
        Ok(output
            .lines()
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
        if specs.is_empty() {
            return Ok(());
        }

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
            self.core
                .executor
                .run_exclusive(&self.core.name, &self.core.name, &arg_refs, sudo)
                .await?;
        } else {
            self.core
                .executor
                .run(&self.core.name, &arg_refs, sudo)
                .await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        if names.is_empty() {
            return Ok(());
        }

        // Some managers (e.g. Haskell's cabal/stack) genuinely have no uninstall verb.
        // An empty `remove_args` encodes that: report it honestly as Unsupported instead
        // of running the bare binary with just the package names, which would misbehave.
        if self.core.config.remove_args.is_empty() {
            return Err(crate::core::Error::Unsupported(self.core.name.clone()));
        }

        let mut args: Vec<String> = self.core.config.remove_args.clone();
        for name in names {
            args.push(name.clone());
        }

        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        if self.core.config.is_exclusive {
            self.core
                .executor
                .run_exclusive(&self.core.name, &self.core.name, &arg_refs, sudo)
                .await?;
        } else {
            self.core
                .executor
                .run(&self.core.name, &arg_refs, sudo)
                .await?;
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
        let args: Vec<&str> = self
            .core
            .config
            .list_args
            .iter()
            .map(|s| s.as_str())
            .collect();
        // Use the configured list binary if the query tool is a separate program (e.g.
        // apt -> dpkg-query); otherwise the backend's own binary.
        let bin = self
            .core
            .config
            .list_binary
            .as_deref()
            .unwrap_or(&self.core.name);
        let output = self.core.executor.run_output(bin, &args, false).await?;
        Ok(self.core.parser.parse_installed(&output))
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        match &self.core.config.manual {
            ManualListing::AllInstalled => self.list_installed().await,
            ManualListing::Command {
                binary,
                args,
                format,
            } => {
                let args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                let bin = binary
                    .as_deref()
                    .or(self.core.config.list_binary.as_deref())
                    .unwrap_or(&self.core.name);
                let output = self.core.executor.run_output(bin, &args, false).await?;
                Ok(match format {
                    ManualFormat::BareNames => {
                        crate::parsers::parse_bare_names(&output, &self.core.name)
                    }
                    ManualFormat::SameAsInstalled => self.core.parser.parse_installed(&output),
                })
            }
            // Deliberately empty, not `list_installed`. Callers gate on `tracks_manual`;
            // returning the installed set here would be a confident wrong answer.
            ManualListing::Unsupported => Ok(Vec::new()),
        }
    }

    fn tracks_manual(&self) -> bool {
        !matches!(self.core.config.manual, ManualListing::Unsupported)
    }

    fn manual_source(&self) -> String {
        match &self.core.config.manual {
            ManualListing::AllInstalled => format!(
                "everything {} installed ({0} installs no dependencies of its own)",
                self.core.name
            ),
            ManualListing::Command { binary, args, .. } => {
                let bin = binary
                    .as_deref()
                    .or(self.core.config.list_binary.as_deref())
                    .unwrap_or(&self.core.name);
                format!("{} {}", bin, args.join(" "))
            }
            ManualListing::Unsupported => {
                format!(
                    "{} cannot tell your choices from dependencies",
                    self.core.name
                )
            }
        }
    }

    async fn essential(&self) -> Result<Vec<String>> {
        let Some(ref essential_args) = self.core.config.essential_args else {
            return Ok(Vec::new());
        };
        let args: Vec<&str> = essential_args.iter().map(|s| s.as_str()).collect();
        let bin = self
            .core
            .config
            .list_binary
            .as_deref()
            .unwrap_or(&self.core.name);
        let output = self.core.executor.run_output(bin, &args, false).await?;
        Ok(self.core.parser.parse_essential(&output))
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        // Windows package managers use CASE-INSENSITIVE ids, but their list output frequently
        // returns a different casing than the install id: choco installs "wget" yet lists the
        // Title "Wget", so a case-sensitive `p.name == name` misses it and the remove is
        // silently skipped (package + manifest left behind). Match case-insensitively for
        // those. winget additionally records a vendor-qualified Id ("jqlang.jq") that is
        // commonly installed/removed by its bare moniker ("jq"), so also accept the trailing
        // dot-segment. Kept scoped to Windows managers to avoid mis-matching legitimately
        // case-distinct or dotted names elsewhere (e.g. npm "socket.io").
        let b = self.core.name.as_str();
        let ci = matches!(b, "choco" | "scoop" | "winget");
        let winget = b == "winget";
        Ok(all.into_iter().find(|p| {
            p.name == name
                || (ci && p.name.eq_ignore_ascii_case(name))
                || (winget
                    && p.name
                        .rsplit('.')
                        .next()
                        .is_some_and(|s| s.eq_ignore_ascii_case(name)))
        }))
    }
}

pub struct GenericSearchable {
    pub core: Arc<GenericBackendCore>,
}

#[async_trait]
impl Searchable for GenericSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let mut args: Vec<&str> = self
            .core
            .config
            .search_args
            .iter()
            .map(|s| s.as_str())
            .collect();
        args.push(query);
        let bin = self
            .core
            .config
            .search_binary
            .as_deref()
            .unwrap_or(&self.core.name);
        let output = self.core.executor.search_output(bin, &args, false).await?;
        Ok(self.core.parser.parse_search(&output))
    }
}

pub struct GenericEnumerable {
    pub core: Arc<GenericBackendCore>,
}

#[async_trait]
impl Enumerable for GenericEnumerable {
    async fn available_names(&self) -> Result<Vec<String>> {
        let Some(args) = &self.core.config.enumerate_args else {
            return Err(Error::Other(format!(
                "`{}` cannot list every package it could install.",
                self.core.name
            )));
        };
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let bin = self
            .core
            .config
            .enumerate_binary
            .as_deref()
            .unwrap_or(&self.core.name);
        let output = self.core.executor.run_output(bin, &args, false).await?;
        Ok(output
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect())
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
        let args: Vec<&str> = self
            .core
            .config
            .upgrade_args
            .iter()
            .map(|s| s.as_str())
            .collect();
        if self.core.config.is_exclusive {
            self.core
                .executor
                .run_exclusive(&self.core.name, &self.core.name, &args, sudo)
                .await?;
        } else {
            self.core.executor.run(&self.core.name, &args, sudo).await?;
        }
        Ok(())
    }

    fn has_native_orphan_removal(&self) -> bool {
        self.core.config.orphan_args.is_some()
    }

    async fn clean_orphans(&self, sudo: bool) -> Result<()> {
        // If the backend declares native orphan-removal args, run them; otherwise be
        // honest that it has no orphan concept (LSP) rather than silently succeeding.
        match &self.core.config.orphan_args {
            Some(args) => {
                let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                if self.core.config.is_exclusive {
                    self.core
                        .executor
                        .run_exclusive(&self.core.name, &self.core.name, &arg_refs, sudo)
                        .await?;
                } else {
                    self.core
                        .executor
                        .run(&self.core.name, &arg_refs, sudo)
                        .await?;
                }
                Ok(())
            }
            None => Err(crate::core::Error::Unsupported(self.core.name.clone())),
        }
    }
}

pub struct GenericRepoManager {
    pub core: Arc<GenericBackendCore>,
}

/// Some backends interpolate repo `{name}`/`{url}` into `sh -c` strings
/// (e.g. apk/apt). Reject shell metacharacters so a crafted argument cannot break out
/// of the intended command.
fn reject_shell_meta(field: &str, value: &str) -> Result<()> {
    if value.chars().any(|c| {
        matches!(
            c,
            '\'' | '"' | '`' | '$' | ';' | '&' | '|' | '<' | '>' | '\n' | '\r' | '\\'
        )
    }) {
        return Err(crate::core::Error::Other(format!(
            "Unsafe characters in repo {}: '{}'",
            field, value
        )));
    }
    Ok(())
}

#[async_trait]
impl RepoManager for GenericRepoManager {
    async fn add_repo(&self, name: &str, url: &str, sudo: bool) -> Result<()> {
        reject_shell_meta("name", name)?;
        reject_shell_meta("url", url)?;
        let base_args = self.core.config.repo_add_args.as_ref().ok_or_else(|| {
            crate::core::Error::Other("Repository addition not supported for this backend".into())
        })?;

        let mut final_args = Vec::new();
        for arg in base_args {
            final_args.push(arg.replace("{name}", name).replace("{url}", url));
        }

        let arg_refs: Vec<&str> = final_args.iter().map(|s| s.as_str()).collect();
        info!("Repo: Adding {} to {}...", name, self.core.name);
        self.core
            .executor
            .run(&self.core.name, &arg_refs, sudo)
            .await?;
        Ok(())
    }

    async fn remove_repo(&self, name: &str, sudo: bool) -> Result<()> {
        reject_shell_meta("name", name)?;
        let base_args = self.core.config.repo_remove_args.as_ref().ok_or_else(|| {
            crate::core::Error::Other("Repository removal not supported for this backend".into())
        })?;

        let final_args: Vec<String> = base_args
            .iter()
            .map(|a| a.replace("{name}", name))
            .collect();
        let arg_refs: Vec<&str> = final_args.iter().map(|s| s.as_str()).collect();

        self.core
            .executor
            .run(&self.core.name, &arg_refs, sudo)
            .await?;
        Ok(())
    }

    async fn list_repos(&self) -> Result<Vec<(String, String)>> {
        let base_args = self.core.config.repo_list_args.as_ref().ok_or_else(|| {
            crate::core::Error::Other("Repository listing not supported for this backend".into())
        })?;
        let arg_refs: Vec<&str> = base_args.iter().map(|s| s.as_str()).collect();
        let output = self
            .core
            .executor
            .run_output(&self.core.name, &arg_refs, false)
            .await?;

        let mut repos = Vec::new();
        for line in output.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            // Skip dashed separator rows ("--------").
            if trimmed.chars().all(|c| c == '-' || c == '=') {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                continue;
            }
            // Skip obvious table headers (e.g. winget "Name Argument Explicit",
            // scoop "Name Source Updated") so they don't show up as repositories.
            let is_header = matches!(
                parts[0],
                "Name" | "NAME" | "Repository" | "Repo" | "Bucket" | "Source"
            ) && matches!(
                parts[1],
                "Argument" | "URL" | "Url" | "Source" | "Updated" | "Explicit" | "Enabled"
            );
            if is_header {
                continue;
            }
            repos.push((parts[0].to_string(), parts[1].to_string()));
        }
        Ok(repos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::core::executor::{CommandExecutor, DryRunOutput, MockExecutor};
    use crate::parsers::LambdaParser;
    use dashmap::DashMap;

    fn apt_like_core(
        mock: Arc<MockExecutor>,
        vfs: Arc<DashMap<std::path::PathBuf, String>>,
    ) -> GenericBackendCore {
        let exec = CommandExecutor::with_layer(true, false, mock, vfs, Arc::new(DashMap::new()));
        GenericBackendCore {
            name: "apt".into(),
            executor: exec,
            config: ManagerConfig {
                name: "apt".into(),
                install_args: vec![],
                remove_args: vec![],
                list_args: vec![],
                manual: ManualListing::AllInstalled,
                essential_args: None,
                search_args: vec![],
                search_binary: None,
                enumerate_args: None,
                enumerate_binary: None,
                list_binary: None,
                upgrade_args: vec![],
                update_args: None,
                orphan_args: None,
                repo_add_args: None,
                repo_remove_args: None,
                repo_list_args: None,
                depends_args: Some(vec![
                    "depends".into(),
                    "--no-recommends".into(),
                    "--no-suggests".into(),
                    "{name}".into(),
                ]),
                version_pin: None,
                needs_root: true, // apt needs root for writes — but reads must NOT escalate
                is_exclusive: true,
                flag_map: HashMap::new(),
            },
            parser: Arc::new(LambdaParser {
                installed_fn: |_| vec![],
                search_fn: |_| vec![],
            }),
        }
    }

    #[tokio::test]
    async fn get_dependencies_parses_names_without_sudo() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        // Respond to the NON-sudo command; if get_dependencies escalated, this wouldn't
        // match and the result would be empty.
        mock.set_response(
            "apt depends --no-recommends --no-suggests curl",
            Ok(DryRunOutput {
                stdout: b"Depends: libc6\nDepends: bash\n".to_vec(),
                stderr: vec![],
            }
            .into()),
        );
        let core = apt_like_core(mock, vfs);
        let deps = core.get_dependencies("curl").await.unwrap();
        // "Depends: libc6" -> "libc6" (label + constraints stripped)
        assert_eq!(deps, vec!["libc6".to_string(), "bash".to_string()]);
    }

    fn queryable_with(
        manual: ManualListing,
        mock: Arc<MockExecutor>,
        vfs: Arc<DashMap<std::path::PathBuf, String>>,
    ) -> GenericQueryable {
        let mut core = apt_like_core(mock, vfs);
        core.config.list_binary = Some("dpkg-query".into());
        core.config.list_args = vec!["-W".into(), "-f=${Package} ${Version}\\n".into()];
        core.config.manual = manual;
        core.parser = Arc::new(crate::parsers::apt::AptParser);
        GenericQueryable {
            core: Arc::new(core),
        }
    }

    #[tokio::test]
    async fn apt_manual_list_asks_apt_mark_not_dpkg_query() {
        // The bug: apt had no manual command, so `list_manual` fell back to `dpkg-query
        // -W` — every installed package, dependencies included (579 vs 103 on the real
        // ubuntu image). It must ask `apt-mark showmanual` instead.
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        mock.set_response(
            "apt-mark showmanual",
            Ok(DryRunOutput {
                stdout: b"apt\nbase-files\njq\n".to_vec(),
                stderr: vec![],
            }
            .into()),
        );
        // If it wrongly fell back, it would hit this instead — and adopt a dependency.
        mock.set_response(
            "dpkg-query -W -f=${Package} ${Version}\\n",
            Ok(DryRunOutput {
                stdout: b"apt 2.7.14\njq 1.7.1\nlibperl5.38t64 5.38.2\n".to_vec(),
                stderr: vec![],
            }
            .into()),
        );

        let q = queryable_with(
            ManualListing::Command {
                binary: Some("apt-mark".into()),
                args: vec!["showmanual".into()],
                format: ManualFormat::BareNames,
            },
            mock.clone(),
            vfs,
        );

        let names: Vec<String> = q
            .list_manual()
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, vec!["apt", "base-files", "jq"]);
        assert!(
            !names.contains(&"libperl5.38t64".to_string()),
            "a pure dependency must never be reported as user-chosen"
        );
        assert!(q.tracks_manual());

        let calls = mock.get_calls().await;
        assert!(
            calls.iter().any(|c| c == "apt-mark showmanual"),
            "{:?}",
            calls
        );
    }

    #[tokio::test]
    async fn unsupported_backend_reports_nothing_rather_than_everything() {
        // The safety backstop: a manager with dependencies and no way to name the user's
        // choices must return an empty list AND admit it via tracks_manual, so adoption
        // skips it. Returning list_installed here is a confident wrong answer.
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        mock.set_response(
            "dpkg-query -W -f=${Package} ${Version}\\n",
            Ok(DryRunOutput {
                stdout: b"apt 2.7.14\nlibperl5.38t64 5.38.2\n".to_vec(),
                stderr: vec![],
            }
            .into()),
        );

        let q = queryable_with(ManualListing::Unsupported, mock, vfs);
        assert!(!q.tracks_manual());
        assert!(
            q.list_manual().await.unwrap().is_empty(),
            "adopting nothing is safe; adopting everything is catastrophic"
        );
        // list_installed still works — only the *intent* question is unanswerable.
        assert_eq!(q.list_installed().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn all_installed_backends_still_report_their_installed_set() {
        // winget/choco/mas install no dependencies, so everything listed was asked for.
        // The Unsupported default must not silently swallow these.
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        mock.set_response(
            "dpkg-query -W -f=${Package} ${Version}\\n",
            Ok(DryRunOutput {
                stdout: b"jq 1.7.1\n".to_vec(),
                stderr: vec![],
            }
            .into()),
        );
        let q = queryable_with(ManualListing::AllInstalled, mock, vfs);
        assert!(q.tracks_manual());
        assert_eq!(q.list_manual().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn essential_query_is_absent_unless_a_backend_declares_it() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let q = queryable_with(ManualListing::AllInstalled, mock, vfs);
        // apt_like_core sets essential_args: None → no OS essential list, and no crash.
        assert!(q.essential().await.unwrap().is_empty());
    }

    #[test]
    fn version_pin_renders_native_syntax() {
        // inline forms (apt/pip/bun)
        assert_eq!(
            VersionPin::Inline("{name}={version}".into()).apply("curl", "7.81.0"),
            vec!["curl=7.81.0"]
        );
        assert_eq!(
            VersionPin::Inline("{name}=={version}".into()).apply("requests", "2.31.0"),
            vec!["requests==2.31.0"]
        );
        // flag forms (winget/choco/gem)
        assert_eq!(
            VersionPin::Flag(vec!["--version".into(), "{version}".into()])
                .apply("Git.Git", "2.54.0"),
            vec!["Git.Git", "--version", "2.54.0"]
        );
    }

    #[tokio::test]
    async fn clean_orphans_reports_unsupported_without_orphan_args() {
        // A generic backend with no `orphan_args` must report Unsupported (an honest,
        // benign skip) instead of silently returning Ok — the LSP fix.
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let core = Arc::new(apt_like_core(mock, vfs)); // apt_like_core sets orphan_args: None
        let up = GenericUpgradable { core };
        match up.clean_orphans(true).await {
            Err(crate::core::Error::Unsupported(name)) => assert_eq!(name, "apt"),
            other => panic!("expected Unsupported, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn remove_reports_unsupported_with_empty_remove_args() {
        // A manager with no uninstall verb encodes it as empty remove_args → Unsupported,
        // rather than running the bare binary against the package names.
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let core = Arc::new(apt_like_core(mock, vfs)); // apt_like_core sets remove_args: vec![]
        let inst = GenericInstallable { core };
        match inst.remove(&["ghc".to_string()], false).await {
            Err(crate::core::Error::Unsupported(name)) => assert_eq!(name, "apt"),
            other => panic!("expected Unsupported, got {:?}", other),
        }
    }

    #[test]
    fn concrete_version_rejects_floating() {
        assert!(is_concrete_version("1.2.3"));
        assert!(!is_concrete_version("latest"));
        assert!(!is_concrete_version("*"));
        assert!(!is_concrete_version(""));
    }
}
