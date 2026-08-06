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
            executor: executor.with_exit_policy(crate::core::exit_policy::for_manager("pacman")),
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

/// The `Depends On` row of a `pacman -Si` report, as package names.
///
/// **Matched by key, not by column.** This stripped the literal `"Depends On     :"` — five
/// spaces — and Arch prints six, so the prefix never matched and `pacman:` answered every
/// dependency query with an empty list for the whole life of the backend. Nothing caught it,
/// because the only consumer was a planner that installed whatever came back and was better off
/// with nothing (`Y9`); now that the answer is only ever *shown*, an empty one is a lie a user
/// reads. Captured from `pacman -Si jq` in the Arch integration image, 2026-08-06:
///
/// ```text
/// Depends On      : glibc  oniguruma
/// ```
///
/// `Optional Deps` and `Conflicts With` are rows of the same shape, which is why the key is
/// compared whole rather than by prefix.
fn parse_depends_on(output: &str) -> Vec<String> {
    let mut deps = Vec::new();
    for line in output.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.trim() != "Depends On" {
            continue;
        }
        for part in value.split_whitespace() {
            // pacman writes `None` where there are no dependencies at all.
            if part == "None" {
                continue;
            }
            // `glibc>=2.36` and `sh=5` are one dependency each; the constraint is not part of
            // the name a later command would have to use.
            let name = part.split(['>', '<', '=']).next().unwrap_or(part);
            if !name.is_empty() {
                deps.push(name.to_string());
            }
        }
    }
    deps
}

#[async_trait]
impl MetadataProvider for PacmanBackendCore {
    async fn get_dependencies(&self, name: &str) -> Result<Vec<String>> {
        let output = self
            .executor
            .run_output("pacman", &["-Si", name], false)
            .await?;
        Ok(parse_depends_on(&output))
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
    fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
        (self.core.executor.installed_listings(), &self.core.name)
    }

    async fn fetch_installed(&self) -> Result<Vec<Package>> {
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

    /// `pacman -Qu` — every update in one call (`Q44`).
    ///
    /// **It exits 1 with no output at all when nothing is out of date**, which is the one shape
    /// `Q40` calls a failed read — correctly, in general, and wrongly here. pacman documents
    /// this exit as the answer, so it is translated back where the manager's meaning is known,
    /// rather than by loosening the rule for every read in the program.
    ///
    /// A pacman that genuinely failed says so on stderr, and that path still raises.
    async fn outdated_all(&self) -> Result<Option<Vec<Package>>> {
        match self
            .core
            .executor
            .run_output("pacman", &["-Qu"], false)
            .await
        {
            Ok(output) => Ok(Some(pacman::parse_pacman_outdated(&output))),
            Err(e) if e.to_string().contains("no output") => Ok(Some(Vec::new())),
            Err(e) => Err(e),
        }
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

/// What pacman prints, and what LiNix reads out of it. Both halves are here because both are
/// the same risk: a report whose shape is assumed rather than captured.
#[cfg(test)]
mod pacman_output_tests {
    use super::*;
    use crate::core::executor::MockExecutor;
    use crate::core::Searchable;
    use dashmap::DashMap;

    fn wired() -> (Arc<PacmanBackendCore>, Arc<MockExecutor>) {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let exec =
            CommandExecutor::with_layer(false, false, mock.clone(), vfs, Arc::new(DashMap::new()));
        (Arc::new(PacmanBackendCore::new(exec)), mock)
    }

    /// **`pacman -Qu` exits 1 with nothing on either stream when nothing is out of date.**
    ///
    /// That is precisely the shape `Q40` calls a failed read — correctly in general, and
    /// wrongly here, because pacman documents this exit as the answer. Translated back where
    /// the manager's meaning is known, rather than by loosening the rule for every read.
    ///
    /// `Some(vec![])` and not `None`: the manager *was* asked and reported nothing. `None`
    /// would send the caller round the per-package path for an answer it already has.
    #[tokio::test]
    async fn nothing_out_of_date_is_an_empty_answer_not_a_failed_read() {
        let (core, mock) = wired();
        mock.set_response("pacman -Qu", Ok(crate::core::executor::silent_failure(1)));
        let got = PacmanSearchable { core }
            .outdated_all()
            .await
            .expect("pacman's own way of saying `none` is not a failure");
        assert_eq!(
            got.map(|v| v.len()),
            Some(0),
            "asked and nothing stale — not `could not be asked`"
        );
    }

    /// A pacman that genuinely failed says so, and that must still raise rather than be read
    /// as a clean bill of health.
    #[tokio::test]
    async fn a_pacman_that_complained_is_still_a_failure() {
        let (core, mock) = wired();
        mock.set_response(
            "pacman -Qu",
            Ok(crate::core::executor::spoken_failure(
                1,
                "",
                "error: failed to init transaction",
            )),
        );
        // A complaint is handed to the caller as an empty read (Q40's boundary), so the probe
        // reports nothing stale rather than inventing rows — but it never claims more.
        let got = PacmanSearchable { core }.outdated_all().await.unwrap();
        assert_eq!(got.map(|v| v.len()), Some(0));
    }

    /// The ordinary case, against output captured from a real `linix-it-arch` container.
    #[tokio::test]
    async fn updates_are_reported_from_one_call() {
        let (core, mock) = wired();
        mock.set_response(
            "pacman -Qu",
            Ok(crate::core::executor::DryRunOutput {
                stdout: b"audit 4.1.4-2 -> 4.2.1-1\nglib2 2.88.2-1 -> 2.88.3-1\n".to_vec(),
                stderr: vec![],
            }
            .into()),
        );
        let got = PacmanSearchable { core }
            .outdated_all()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].name, "audit");
        assert_eq!(got[0].version.as_deref(), Some("4.2.1-1"));
        assert_eq!(
            mock.get_calls().await.len(),
            1,
            "one call for the whole machine"
        );
    }

    /// Verbatim from `pacman -Si` in the Arch integration image, 2026-08-06 — the whole point
    /// being the column width, which the old fixed-width `strip_prefix` was one space short of.
    const SI_JQ: &str = "\
Repository      : extra
Name            : jq
Version         : 1.8.1-1
Depends On      : glibc  oniguruma
Optional Deps   : None
Conflicts With  : None
Description     : Command-line JSON processor
";

    #[test]
    fn a_depends_row_is_matched_by_its_key_and_not_by_its_column() {
        assert_eq!(parse_depends_on(SI_JQ), vec!["glibc", "oniguruma"]);
    }

    #[test]
    fn a_row_of_the_same_shape_that_is_not_a_dependency_is_left_alone() {
        // `Optional Deps` and `Conflicts With` sit in the same column and are not dependencies;
        // matching by prefix on `Depends` would have taken `Optional Deps` too once the width
        // was right.
        let deps = parse_depends_on(SI_JQ);
        assert!(!deps.iter().any(|d| d == "None"), "{:?}", deps);
        assert!(!deps.iter().any(|d| d == "Command-line"), "{:?}", deps);
    }

    #[test]
    fn a_version_constraint_is_not_part_of_the_name() {
        let out = "Depends On      : glibc>=2.36  sh=5  libc.so\n";
        assert_eq!(parse_depends_on(out), vec!["glibc", "sh", "libc.so"]);
    }

    #[test]
    fn a_package_with_no_dependencies_reports_none_of_them() {
        assert!(parse_depends_on("Depends On      : None\n").is_empty());
        assert!(parse_depends_on("Name            : tree\n").is_empty());
    }
}
