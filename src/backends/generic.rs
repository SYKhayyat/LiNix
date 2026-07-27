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

/// A dry run of the manager's own orphan verb, and how to read the names back out of it.
///
/// `apt-get autoremove --dry-run` prints `Remv libfoo1 [1.2-3]` per package; the prefix is
/// what separates those lines from the summary counts and the "0 upgraded" line.
#[derive(Debug, Clone)]
pub struct OrphanDryRun {
    /// Binary to run, when it is not the backend's own (apt's autoremove is `apt-get`).
    pub binary: Option<String>,
    pub args: Vec<String>,
    pub removes_line_prefix: String,
}

/// Configuration for the Generic Manager Strategy.
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    pub name: String,
    /// The program to run, when it is not the backend's own name (XIII.12). `None` — every
    /// built-in — means the name is the command.
    pub binary: Option<String>,
    pub install_args: Vec<String>,
    pub remove_args: Vec<String>,
    /// Optional: the program that runs the REMOVE commands, when a manager uninstalls with a
    /// *separate* binary from the one it installs with — OpenBSD installs with `pkg_add` and
    /// removes with `pkg_delete`. `None` = removal uses the same binary as everything else.
    pub remove_binary: Option<String>,
    /// Args that also destroy the package's configuration (Debian's `purge`). `None` means
    /// this manager draws no such distinction, and `--purge` on it is refused rather than
    /// quietly doing an ordinary removal.
    pub purge_args: Option<Vec<String>>,
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
    /// How to ask the manager what its own orphan verb *would* remove, without removing it.
    /// `None` means this manager cannot say, and a manager that cannot say does not remove.
    pub orphan_dry_run: Option<OrphanDryRun>,
    pub repo_add_args: Option<Vec<String>>,
    pub repo_remove_args: Option<Vec<String>>,
    pub repo_list_args: Option<Vec<String>>,
    /// Optional: the program that runs `repo_add_args`/`repo_remove_args`, when a manager
    /// edits its sources with a *separate* tool. apt's is `add-apt-repository` and apk's is a
    /// line appended to a file by `sh` — neither is `apt` or `apk`, and running them as
    /// subcommands of the manager is the same defect `list_binary` exists to prevent.
    pub repo_binary: Option<String>,
    /// Optional: the program that runs `repo_list_args`. Separate from `repo_binary` because a
    /// manager can write its sources one way and read them another (apk writes with `sh` and
    /// reads with `cat`). Falls back to `binary`, not to `repo_binary`.
    pub repo_list_binary: Option<String>,
    pub depends_args: Option<Vec<String>>,
    /// Native syntax for pinning an exact version at install (None = no version pinning).
    pub version_pin: Option<VersionPin>,
    /// Optional: the option key holding what `install_args` takes, when that is not the
    /// package's own name. `helm plugin install` takes a URL while `plugin list` and `plugin
    /// uninstall` speak the name from the plugin's `plugin.yaml` — so the name has to stay the
    /// identity (a declaration that names the URL installs once and can never be removed or
    /// recognised again), and the URL rides in an option. `None` = the name is the argument.
    pub install_source_option: Option<String>,
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

impl GenericBackendCore {
    /// The program this backend runs. `name` is the prefix a line is written with, and for
    /// every built-in the two are the same word — but a user-defined noun (`firewall:`) runs
    /// something else (`ufw`), so a command position must ask for this and never for `name`
    /// (XIII.12). `list_binary`/`search_binary`/`enumerate_binary` are narrower overrides and
    /// fall back to this, not to the name.
    pub fn binary(&self) -> &str {
        self.config.binary.as_deref().unwrap_or(&self.name)
    }

    /// The program that removes. Falls back to [`binary`](Self::binary), not to the name, so a
    /// user-defined noun with a separate remover still removes with the right tool.
    pub fn remove_binary(&self) -> &str {
        self.config
            .remove_binary
            .as_deref()
            .unwrap_or_else(|| self.binary())
    }

    /// The program that adds and removes repositories.
    pub fn repo_binary(&self) -> &str {
        self.config
            .repo_binary
            .as_deref()
            .unwrap_or_else(|| self.binary())
    }

    /// The program that lists repositories.
    pub fn repo_list_binary(&self) -> &str {
        self.config
            .repo_list_binary
            .as_deref()
            .unwrap_or_else(|| self.binary())
    }
}

#[async_trait]
impl BackendCore for GenericBackendCore {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        self.executor.command_exists_sync(self.binary())
    }

    fn needs_root(&self) -> bool {
        self.config.needs_root
    }

    async fn check_health(&self) -> Result<HealthReport> {
        if !self.is_available() {
            // "not found" rather than "not on PATH": a custom backend's binary may be an
            // absolute path (U16), and telling someone their `/opt/vendor/thing` is "not on
            // PATH" points them at the wrong thing to fix.
            let b = self.binary();
            let where_ = if b.contains(['/', '\\']) {
                format!("`{}` does not exist or is not executable", b)
            } else {
                format!("`{}` is not on PATH", b)
            };
            return Ok(HealthReport {
                status: HealthStatus::Critical,
                message: Some(format!(
                    "{}, so the `{}` backend cannot run",
                    where_, self.name
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
            .run_output(self.binary(), &arg_refs, false)
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

/// The argument `install_args` takes for a backend whose install speaks a different vocabulary
/// than its list and remove (`install_source_option`).
///
/// Refusing beats guessing: deriving `diff` from `.../helm-diff` is right often enough to be
/// trusted and wrong often enough to install a plugin under a name nothing can remove, and the
/// name lives in the plugin's own `plugin.yaml`, which cannot be read before it is fetched.
fn install_source(backend: &str, spec: &PackageSpec, key: &str) -> Result<String> {
    spec.options
        .get(key)
        .filter(|v| !v.trim().is_empty())
        .cloned()
        .ok_or_else(|| {
            crate::core::Error::Validation(format!(
            "{backend}:{name} needs `@{key}=…`. {backend} installs from that value but lists and \
             removes by name, so the declaration has to carry both: \
             `{backend}:{name}@{key}=<source>`.",
            backend = backend,
            name = spec.name,
            key = key,
        ))
        })
}

#[async_trait]
impl Installable for GenericInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        if specs.is_empty() {
            return Ok(());
        }

        let mut final_args: Vec<String> = self.core.config.install_args.clone();
        let mut names: Vec<String> = Vec::with_capacity(specs.len());
        // A `Flag` pin puts an option *after* the name it pins (`gem install jq -v 1.6`), so
        // the terminator cannot precede it — behind `--` that `-v` is a package.
        let mut trailing_flags = false;
        for spec in specs {
            if let Some(key) = &self.core.config.install_source_option {
                names.push(install_source(&self.core.name, spec, key)?);
                continue;
            }
            // Honor an exact version pin (reproducible/locked installs) using the
            // backend's native syntax, when both a pin syntax and a concrete version exist.
            match (spec.options.get("version"), &self.core.config.version_pin) {
                (Some(ver), Some(pin)) if is_concrete_version(ver) => {
                    trailing_flags |= matches!(pin, VersionPin::Flag(_));
                    names.extend(pin.apply(&spec.name, ver));
                }
                _ => names.push(spec.name.clone()),
            }
        }
        if trailing_flags {
            final_args.extend(names);
        } else {
            crate::core::argv::push_names(&mut final_args, self.core.binary(), names);
        }

        let arg_refs: Vec<&str> = final_args.iter().map(|s| s.as_str()).collect();

        if self.core.config.is_exclusive {
            self.core
                .executor
                .run_exclusive(self.core.binary(), self.core.binary(), &arg_refs, sudo)
                .await?;
        } else {
            self.core
                .executor
                .run(self.core.binary(), &arg_refs, sudo)
                .await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        // Some managers (e.g. Haskell's cabal/stack) genuinely have no uninstall verb.
        // An empty `remove_args` encodes that — UNLESS the manager removes with a separate
        // binary that is itself the verb (OpenBSD's `pkg_delete <name>`, no subcommand). A
        // separate remove binary means removal is supported however few args it takes.
        if self.core.config.remove_args.is_empty() && self.core.config.remove_binary.is_none() {
            return Err(crate::core::Error::Unsupported(self.core.name.clone()));
        }
        self.run_removal(self.core.config.remove_args.clone(), names, sudo)
            .await
    }

    fn supports_purge(&self) -> bool {
        self.core.config.purge_args.is_some()
    }

    async fn purge(&self, names: &[String], sudo: bool) -> Result<()> {
        let Some(args) = self.core.config.purge_args.clone() else {
            return Err(crate::core::Error::Unsupported(format!(
                "{} has no purge — it does not keep a package's configuration apart from the \
                 package",
                self.core.name
            )));
        };
        self.run_removal(args, names, sudo).await
    }
}

impl GenericInstallable {
    async fn run_removal(&self, mut args: Vec<String>, names: &[String], sudo: bool) -> Result<()> {
        if names.is_empty() {
            return Ok(());
        }
        let bin = self.core.remove_binary();
        crate::core::argv::push_names(&mut args, bin, names);
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        if self.core.config.is_exclusive {
            self.core
                .executor
                .run_exclusive(bin, bin, &arg_refs, sudo)
                .await?;
        } else {
            self.core.executor.run(bin, &arg_refs, sudo).await?;
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
            .unwrap_or(self.core.binary());
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
                    .unwrap_or(self.core.binary());
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
                    .unwrap_or(self.core.binary());
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
            .unwrap_or(self.core.binary());
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
        let bin = self
            .core
            .config
            .search_binary
            .as_deref()
            .unwrap_or(self.core.binary());
        let mut owned: Vec<String> = self.core.config.search_args.clone();
        crate::core::argv::push_names(&mut owned, bin, [query]);
        let args: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
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
            .unwrap_or(self.core.binary());
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
            self.core
                .executor
                .run(self.core.binary(), &args, sudo)
                .await?;
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
                .run_exclusive(self.core.binary(), self.core.binary(), &args, sudo)
                .await?;
        } else {
            self.core
                .executor
                .run(self.core.binary(), &args, sudo)
                .await?;
        }
        Ok(())
    }

    async fn list_orphans(&self) -> Result<Vec<String>> {
        let Some(dry) = &self.core.config.orphan_dry_run else {
            return Err(crate::core::Error::Unsupported(self.core.name.clone()));
        };
        let args: Vec<&str> = dry.args.iter().map(String::as_str).collect();
        let binary = dry.binary.as_deref().unwrap_or(self.core.binary());
        let out = self.core.executor.run_output(binary, &args, false).await?;
        Ok(out
            .lines()
            .filter_map(|l| l.trim().strip_prefix(&dry.removes_line_prefix))
            .filter_map(|rest| rest.split_whitespace().next())
            .map(|n| n.to_string())
            .collect())
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

/// The first `{placeholder}` still standing in a template argument, if any.
fn find_placeholder(s: &str) -> Option<String> {
    let mut rest = s;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        if let Some(close) = after.find('}') {
            let inner = &after[..close];
            if !inner.is_empty() && inner.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                return Some(format!("{{{}}}", inner));
            }
            rest = &after[close..];
        } else {
            return None;
        }
    }
    None
}

/// Refuse an argv that still carries a template placeholder.
///
/// apk's removal row is `sed -i '\|{url}|d' /etc/apk/repositories`. With `{url}` never filled,
/// sed searched for the literal text `{url}`, matched nothing, and **exited 0** — so `run()`
/// saw success and LiNix reported a repository removed that was still there. An unfilled
/// placeholder has to be a loud failure, or the next row with a new placeholder repeats that
/// silently.
fn reject_unsubstituted(backend: &str, args: &[String]) -> Result<()> {
    for a in args {
        if let Some(ph) = find_placeholder(a) {
            return Err(crate::core::Error::Other(format!(
                "the `{}` backend's repository command still contains `{}` after substitution, \
                 so it would run against the literal text. Refusing. This is a defect in the \
                 backend definition, not in what you asked for.",
                backend, ph
            )));
        }
    }
    Ok(())
}

impl GenericRepoManager {
    /// The URL of the repository the user named.
    ///
    /// A few managers know a repository only by its URL — `gem sources -r <url>`, apk's line in
    /// `/etc/apk/repositories` — so their removal rows carry `{url}` while the caller has one
    /// identifier. Ask the manager's own listing first; a manager whose listing is the URL
    /// itself (apk prints one field per line, which `list_repos` cannot read as a pair) leaves
    /// the identifier as the only thing that can be it.
    async fn url_for(&self, ident: &str) -> Result<String> {
        if let Ok(repos) = self.list_repos().await {
            if let Some((_, url)) = repos.iter().find(|(n, _)| n == ident) {
                return Ok(url.clone());
            }
            if repos.iter().any(|(_, url)| url == ident) {
                return Ok(ident.to_string());
            }
        }
        if ident.contains("://") || ident.starts_with('/') {
            return Ok(ident.to_string());
        }
        Err(crate::core::Error::Other(format!(
            "`{backend}` identifies a repository by its URL, and `{ident}` is neither a URL nor \
             a name `{backend}` reports.\n  \
             Run `linix repo list -b {backend}` and pass the source exactly as it appears there.",
            backend = self.core.name,
            ident = ident
        )))
    }
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
        reject_unsubstituted(&self.core.name, &final_args)?;

        let arg_refs: Vec<&str> = final_args.iter().map(|s| s.as_str()).collect();
        info!("Repo: Adding {} to {}...", name, self.core.name);
        self.core
            .executor
            .run(self.core.repo_binary(), &arg_refs, sudo)
            .await?;
        Ok(())
    }

    async fn remove_repo(&self, name: &str, sudo: bool) -> Result<()> {
        reject_shell_meta("name", name)?;
        let base_args = self.core.config.repo_remove_args.as_ref().ok_or_else(|| {
            crate::core::Error::Other("Repository removal not supported for this backend".into())
        })?;

        // Resolved before anything runs: the URL comes from the manager's own listing or from
        // the identifier, and either way it can land inside an `sh -c` string.
        let url = if base_args.iter().any(|a| a.contains("{url}")) {
            let resolved = self.url_for(name).await?;
            reject_shell_meta("url", &resolved)?;
            Some(resolved)
        } else {
            None
        };

        let final_args: Vec<String> = base_args
            .iter()
            .map(|a| {
                let filled = a.replace("{name}", name);
                match &url {
                    Some(u) => filled.replace("{url}", u),
                    None => filled,
                }
            })
            .collect();
        reject_unsubstituted(&self.core.name, &final_args)?;
        let arg_refs: Vec<&str> = final_args.iter().map(|s| s.as_str()).collect();

        self.core
            .executor
            .run(self.core.repo_binary(), &arg_refs, sudo)
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
            .run_output(self.core.repo_list_binary(), &arg_refs, false)
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
                binary: None,
                remove_binary: None,
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
                purge_args: None,
                orphan_dry_run: None,
                repo_add_args: None,
                repo_remove_args: None,
                repo_list_args: None,
                repo_binary: None,
                repo_list_binary: None,
                depends_args: Some(vec![
                    "depends".into(),
                    "--no-recommends".into(),
                    "--no-suggests".into(),
                    "{name}".into(),
                ]),
                version_pin: None,
                needs_root: true, // apt needs root for writes — but reads must NOT escalate
                is_exclusive: true,
                install_source_option: None,
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
    async fn list_orphans_reports_unsupported_without_a_dry_run() {
        // A generic backend with no `orphan_dry_run` cannot say what its orphan verb would
        // delete, so it reports Unsupported and never removes blind.
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let core = Arc::new(apt_like_core(mock, vfs)); // apt_like_core sets orphan_dry_run: None
        let up = GenericUpgradable { core };
        match up.list_orphans().await {
            Err(crate::core::Error::Unsupported(name)) => assert_eq!(name, "apt"),
            other => panic!("expected Unsupported, got {:?}", other),
        }
    }

    /// A `helm`-shaped core: installs from an option, lists and removes by name.
    fn source_option_core(
        mock: Arc<MockExecutor>,
        vfs: Arc<DashMap<std::path::PathBuf, String>>,
    ) -> GenericBackendCore {
        let mut core = apt_like_core(mock, vfs);
        core.name = "helm".into();
        core.config.name = "helm".into();
        core.config.install_source_option = Some("url".into());
        core.config.install_args = vec!["plugin".into(), "install".into()];
        core.config.remove_args = vec!["plugin".into(), "uninstall".into()];
        core.config.needs_root = false;
        core
    }

    fn spec_with(name: &str, opts: &[(&str, &str)]) -> PackageSpec {
        PackageSpec {
            name: name.into(),
            backend: "helm".into(),
            options: opts
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn install_from_a_source_option_sends_the_source_and_removes_by_name() {
        // U39. The whole bug in one test: what goes out at install is the URL, what goes out
        // at remove is the name, and they come from the same one-line declaration.
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let core = Arc::new(source_option_core(mock.clone(), vfs));
        let inst = GenericInstallable { core };

        let url = "https://github.com/databus23/helm-diff";
        inst.install(&[spec_with("diff", &[("url", url)])], false)
            .await
            .unwrap();
        inst.remove(&["diff".to_string()], false).await.unwrap();

        let calls = mock.get_calls().await;
        assert!(
            calls
                .iter()
                .any(|c| c.contains("plugin install") && c.contains(url)),
            "install must send the url: {:?}",
            calls
        );
        assert!(
            !calls
                .iter()
                .any(|c| c.contains("plugin install") && c.contains(" diff")),
            "install must not send the name: {:?}",
            calls
        );
        assert!(
            calls
                .iter()
                .any(|c| c.contains("plugin uninstall") && c.contains("diff")),
            "remove must send the name: {:?}",
            calls
        );
    }

    #[tokio::test]
    async fn install_without_the_source_option_refuses_and_names_the_fix() {
        // Refusing beats guessing a URL→name mapping: the old behaviour installed happily and
        // then failed every later sync, because nothing could remove what it had installed.
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let core = Arc::new(source_option_core(mock.clone(), vfs));
        let inst = GenericInstallable { core };

        let err = inst
            .install(&[spec_with("diff", &[])], false)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("helm:diff@url="), "{}", msg);
        assert!(
            mock.get_calls().await.is_empty(),
            "nothing may reach the machine when the declaration is incomplete"
        );
    }

    #[tokio::test]
    async fn an_empty_source_option_is_as_missing_as_no_option() {
        // `@url=` with nothing after it would otherwise run `helm plugin install ''`.
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let core = Arc::new(source_option_core(mock.clone(), vfs));
        let inst = GenericInstallable { core };
        assert!(inst
            .install(&[spec_with("diff", &[("url", "  ")])], false)
            .await
            .is_err());
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

    fn repo_mgr(
        name: &str,
        mock: Arc<MockExecutor>,
        vfs: Arc<DashMap<std::path::PathBuf, String>>,
        edit: impl FnOnce(&mut ManagerConfig),
    ) -> GenericRepoManager {
        let mut core = apt_like_core(mock, vfs);
        core.name = name.to_string();
        core.config.name = name.to_string();
        edit(&mut core.config);
        GenericRepoManager {
            core: Arc::new(core),
        }
    }

    fn apk_repo(
        mock: Arc<MockExecutor>,
        vfs: Arc<DashMap<std::path::PathBuf, String>>,
    ) -> GenericRepoManager {
        repo_mgr("apk", mock, vfs, |c| {
            c.repo_add_args = Some(vec![
                "-c".into(),
                "echo '{url}' >> /etc/apk/repositories".into(),
            ]);
            c.repo_remove_args = Some(vec![
                "-c".into(),
                "sed -i '\\|{url}|d' /etc/apk/repositories".into(),
            ]);
            c.repo_list_args = Some(vec!["/etc/apk/repositories".into()]);
            c.repo_binary = Some("sh".into());
            c.repo_list_binary = Some("cat".into());
        })
    }

    /// The finding: `{url}` was never substituted on the removal path, so `sed` searched for
    /// the literal text `{url}`, matched nothing, and **exited 0** — LiNix reported a
    /// repository removed that was still in the file.
    #[tokio::test]
    async fn apk_repo_removal_carries_the_real_url_and_no_placeholder() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let mgr = apk_repo(mock.clone(), vfs);
        mgr.remove_repo("https://dl-cdn.alpinelinux.org/alpine/edge/testing", false)
            .await
            .expect("a URL is a repository apk can be told to forget");
        let calls = mock.get_calls().await;
        assert!(
            calls.iter().any(|c| c.contains("dl-cdn.alpinelinux.org")),
            "{:?}",
            calls
        );
        assert!(
            !calls.iter().any(|c| c.contains("{url}")),
            "the placeholder reached the machine: {:?}",
            calls
        );
    }

    /// A removal LiNix cannot address must refuse, not run a command that matches nothing.
    #[tokio::test]
    async fn a_repo_named_by_something_that_is_not_a_url_is_refused() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let mgr = apk_repo(mock.clone(), vfs);
        let err = mgr
            .remove_repo("testing", false)
            .await
            .expect_err("apk knows no repository called `testing`")
            .to_string();
        assert!(err.contains("testing"), "{}", err);
        assert!(err.contains("repo list"), "{}", err);
        assert!(
            !mock
                .get_calls()
                .await
                .iter()
                .any(|c| c.contains("sed") || c.contains("echo")),
            "nothing may run when the repository cannot be identified"
        );
    }

    /// The other `{url}` template: gem removes by source URL, and a name the listing knows
    /// resolves to one.
    #[tokio::test]
    async fn a_listed_name_resolves_to_its_url() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let mut listed: std::process::Output = DryRunOutput::new().into();
        listed.stdout = b"internal https://gems.example.invalid/\n".to_vec();
        mock.set_response("gem sources", Ok(listed));
        let mgr = repo_mgr("gem", mock.clone(), vfs, |c| {
            c.repo_remove_args = Some(vec!["sources".into(), "-r".into(), "{url}".into()]);
            c.repo_list_args = Some(vec!["sources".into()]);
        });
        mgr.remove_repo("internal", false).await.unwrap();
        let calls = mock.get_calls().await;
        assert!(
            calls
                .iter()
                .any(|c| c == "gem sources -r https://gems.example.invalid/"),
            "{:?}",
            calls
        );
    }

    /// The part that makes this a fixed *class* rather than a fixed instance: a template
    /// carrying a placeholder nothing fills is refused before it runs, whatever the
    /// placeholder is.
    #[tokio::test]
    async fn a_template_with_an_unfillable_placeholder_is_refused() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let mgr = repo_mgr("weird", mock.clone(), vfs, |c| {
            c.repo_remove_args = Some(vec!["drop".into(), "{name}".into(), "{channel}".into()]);
        });
        let err = mgr
            .remove_repo("internal", false)
            .await
            .expect_err("an unfilled placeholder is not a repository name")
            .to_string();
        assert!(err.contains("{channel}"), "{}", err);
        assert!(
            mock.get_calls().await.is_empty(),
            "the template ran with a placeholder in it"
        );
    }

    /// `add_repo` substitutes both keys and always did — asserted so the guard cannot break
    /// the path that was working.
    #[tokio::test]
    async fn add_repo_still_substitutes_name_and_url() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let mgr = repo_mgr("winget", mock.clone(), vfs, |c| {
            c.repo_add_args = Some(vec![
                "source".into(),
                "add".into(),
                "--name".into(),
                "{name}".into(),
                "--arg".into(),
                "{url}".into(),
            ]);
        });
        mgr.add_repo("internal", "https://feed.example.invalid/", false)
            .await
            .unwrap();
        assert_eq!(
            mock.get_calls().await,
            vec!["winget source add --name internal --arg https://feed.example.invalid/"]
        );
    }

    #[test]
    fn a_placeholder_is_recognised_wherever_it_sits_in_the_argument() {
        assert_eq!(find_placeholder("{url}").as_deref(), Some("{url}"));
        assert_eq!(
            find_placeholder("sed -i '\\|{url}|d' /etc/apk/repositories").as_deref(),
            Some("{url}")
        );
        assert_eq!(find_placeholder("--name").as_deref(), None);
        // A brace that is not a placeholder must not become a refusal: shell brace expansion
        // and printf formats both use them.
        assert_eq!(find_placeholder("printf '%s\\n'").as_deref(), None);
        assert_eq!(find_placeholder("{NAME}").as_deref(), None);
        assert_eq!(find_placeholder("{}").as_deref(), None);
        assert_eq!(find_placeholder("a{b").as_deref(), None);
    }

    #[test]
    fn concrete_version_rejects_floating() {
        assert!(is_concrete_version("1.2.3"));
        assert!(!is_concrete_version("latest"));
        assert!(!is_concrete_version("*"));
        assert!(!is_concrete_version(""));
    }
}
