use crate::core::{
    BackendCore, CommandExecutor, Error, Installable, MetadataProvider, Package, PackageSpec,
    Queryable, Result, Searchable, Upgradable,
};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;

/// Emacs package names become elisp symbols inside an `--eval` form. Reject anything
/// that isn't a plain package symbol so a crafted name (whitespace, parens, quotes,
/// backslash) cannot break out of the form and inject arbitrary Lisp.
fn validate_symbol(name: &str) -> Result<()> {
    if !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '+' | '.'))
    {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "Invalid emacs package name: '{}'",
            name
        )))
    }
}

/// Escape a free-text search term for safe embedding inside an elisp string literal.
fn escape_lisp_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub struct EmacsBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl EmacsBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "emacs".to_string(),
        }
    }

    async fn run_lisp(&self, lisp: &str) -> Result<String> {
        self.executor
            .run_output("emacs", &["--batch", "--eval", lisp], false)
            .await
    }

    /// The same evaluation, for a question whose empty answer means "no such package".
    /// An emacs that could not reach its archives prints nothing and must not be read as
    /// one that looked and found nothing.
    async fn search_lisp(&self, lisp: &str) -> Result<String> {
        self.executor
            .search_output("emacs", &["--batch", "--eval", lisp], false)
            .await
    }

    /// The same evaluation, for a *change*. `run_output` reports a failed batch run as
    /// empty output and exit-zero, so an install that never happened read as done.
    async fn change_lisp(&self, lisp: &str) -> Result<()> {
        self.executor
            .run("emacs", &["--batch", "--eval", lisp], false)
            .await
            .map(|_| ())
    }
}

#[async_trait]
impl BackendCore for EmacsBackendCore {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("emacs")
    }
    fn probes(&self) -> Vec<String> {
        vec!["emacs".into()]
    }

    fn needs_root(&self) -> bool {
        // Emacs packages are typically installed in user-owned ~/.emacs.d/elpa
        false
    }
}

#[async_trait]
impl MetadataProvider for EmacsBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        // package.el handles its own dependencies internally.
        Ok(vec![])
    }
}

pub struct EmacsInstallable {
    pub core: Arc<EmacsBackendCore>,
}

#[async_trait]
impl Installable for EmacsInstallable {
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        if specs.is_empty() {
            return Ok(());
        }
        // Every name is validated before ANY of them runs. They are interpolated into a Lisp
        // form, so this is what stands between a package name and evaluated code, and it has
        // to reject the batch rather than fail part-way through one.
        for spec in specs {
            validate_symbol(&spec.name)?;
        }
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        info!("Emacs: Installing {} package(s)...", names.len());
        // One Emacs for the lot (Q46). Each launch was paying for a whole startup AND a
        // `package-refresh-contents` — a network fetch of the archive — so ten packages meant
        // ten of each, for an archive that had not changed between them. Verified against GNU
        // Emacs 29.3 in a container: one `--batch` with a `dolist` installs them all.
        let lisp = format!(
            "(progn (require 'package) (package-initialize)              (unless package-archive-contents (package-refresh-contents))              (dolist (p '({})) (package-install p)))",
            names.join(" ")
        );
        self.core
            .executor
            .run_exclusive("emacs", "emacs", &["--batch", "--eval", &lisp], false)
            .await?;
        Ok(())
    }

    async fn remove(
        &self,
        names: &[String],
        _: bool,
        _reaped: crate::app::sync::guard::Reaped,
    ) -> Result<()> {
        if names.is_empty() {
            return Ok(());
        }
        for name in names {
            validate_symbol(name)?;
        }
        info!("Emacs: Removing {} package(s)...", names.len());
        let lisp = format!(
            "(progn (require 'package) (package-initialize)              (dolist (n '({}))                (let ((p (cadr (assoc n package-alist)))) (if p (package-delete p)))))",
            names.join(" ")
        );
        self.core
            .executor
            .run_exclusive("emacs", "emacs", &["--batch", "--eval", &lisp], false)
            .await?;
        Ok(())
    }
}

pub struct EmacsQueryable {
    pub core: Arc<EmacsBackendCore>,
}

#[async_trait]
impl Queryable for EmacsQueryable {
    fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
        (self.core.executor.installed_listings(), &self.core.name)
    }

    async fn fetch_installed(&self) -> Result<Vec<Package>> {
        let lisp = "(progn \
            (require 'package) \
            (package-initialize) \
            (mapc (lambda (p) \
                (princ (format \"%s %s\\n\" (car p) (package-version-join (package-desc-version (cadr p)))))) \
                package-alist) \
        )";

        let out = self.core.run_lisp(lisp).await?;
        Ok(out
            .lines()
            .filter_map(|l| {
                let (n, v) = l.split_once(' ')?;
                Some(Package::with_version(n.trim(), v.trim(), "emacs"))
            })
            .collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        let lisp = "(progn \
            (require 'package) \
            (package-initialize) \
            (mapc (lambda (p) (princ (format \"%s\\n\" p))) package-selected-packages) \
        )";

        let out = self.core.run_lisp(lisp).await?;
        Ok(out
            .lines()
            .map(|l| Package::new(l.trim(), "emacs"))
            .collect())
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.installed_listing().await?;
        Ok(all.iter().find(|p| p.name == name).cloned())
    }
}

pub struct EmacsSearchable {
    pub core: Arc<EmacsBackendCore>,
}

#[async_trait]
impl Searchable for EmacsSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let needle = escape_lisp_string(query);
        let lisp = format!(
            "(progn \
                (require 'package) \
                (package-initialize) \
                (unless package-archive-contents (package-refresh-contents)) \
                (dolist (p package-archive-contents) \
                    (let ((name (symbol-name (car p)))) \
                        (when (string-match-p \"{}\" name) \
                            (princ (format \"%s %s\\n\" name \
                                (package-version-join (package-desc-version (cadr p))))))))) ",
            needle
        );
        let out = self.core.search_lisp(&lisp).await?;
        Ok(out
            .lines()
            .filter_map(|l| {
                let (n, v) = l.split_once(' ')?;
                Some(Package::with_version(n.trim(), v.trim(), "emacs"))
            })
            .collect())
    }
}

pub struct EmacsUpgradable {
    pub core: Arc<EmacsBackendCore>,
}

#[async_trait]
impl Upgradable for EmacsUpgradable {
    async fn update(&self, _: bool) -> Result<()> {
        info!("Emacs: Refreshing package archives...");
        let lisp = "(progn (require 'package) (package-initialize) (package-refresh-contents))";
        self.core.change_lisp(lisp).await?;
        Ok(())
    }

    async fn upgrade(&self, _: bool) -> Result<()> {
        info!("Emacs: Upgrading all packages...");
        // `package-upgrade-all` exists on Emacs 29+. On older versions fall back to
        // per-package `package-upgrade` where available; a no-op otherwise.
        let lisp = "(progn \
            (require 'package) \
            (package-initialize) \
            (package-refresh-contents) \
            (if (fboundp 'package-upgrade-all) \
                (package-upgrade-all) \
                (when (fboundp 'package-upgrade) \
                    (dolist (pkg (mapcar #'car package-alist)) \
                        (ignore-errors (package-upgrade pkg)))))) ";
        self.core
            .executor
            .run_exclusive("emacs", "emacs", &["--batch", "--eval", lisp], false)
            .await?;
        Ok(())
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let core = Arc::new(EmacsBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(EmacsInstallable { core: core.clone() }))
            .with_queryable(Arc::new(EmacsQueryable { core: core.clone() }))
            .with_searchable(Arc::new(EmacsSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(EmacsUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

#[cfg(test)]
mod batch_tests {
    use super::*;
    use crate::core::executor::MockExecutor;
    use dashmap::DashMap;

    fn wired() -> (Arc<EmacsBackendCore>, Arc<MockExecutor>) {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let exec =
            CommandExecutor::with_layer(false, false, mock.clone(), vfs, Arc::new(DashMap::new()));
        (Arc::new(EmacsBackendCore::new(exec)), mock)
    }

    /// `Q46`. Each install spawned a whole Emacs **and** a `package-refresh-contents`, which is
    /// a network fetch of the archive. Ten packages meant ten startups and ten refreshes of an
    /// archive that had not changed. One `--batch` with a `dolist` does all of it; verified
    /// against GNU Emacs 29.3 in a container before this was written.
    #[tokio::test]
    async fn a_batch_of_packages_is_one_emacs_and_one_archive_refresh() {
        let (core, mock) = wired();
        let specs = vec![
            crate::core::PackageSpec {
                name: "csv-mode".into(),
                backend: "emacs".into(),
                ..Default::default()
            },
            crate::core::PackageSpec {
                name: "rainbow-mode".into(),
                backend: "emacs".into(),
                ..Default::default()
            },
        ];
        EmacsInstallable { core: core.clone() }
            .install(&specs, false)
            .await
            .unwrap();

        let calls = mock.get_calls().await;
        assert_eq!(calls.len(), 1, "one Emacs for the batch, got {:?}", calls);
        assert_eq!(
            calls[0].matches("package-refresh-contents").count(),
            1,
            "the archive is fetched once, not once per package: {:?}",
            calls
        );
        assert!(
            calls[0].contains("csv-mode") && calls[0].contains("rainbow-mode"),
            "{:?}",
            calls
        );
    }

    /// Removal batches the same way.
    #[tokio::test]
    async fn a_batch_of_removals_is_one_emacs() {
        let (core, mock) = wired();
        EmacsInstallable { core }
            .remove(
                &["csv-mode".to_string(), "rainbow-mode".to_string()],
                false,
                crate::app::sync::guard::Reaped::for_reason(
                    crate::app::sync::guard::GuardScope::Remove,
                    "a unit test of the effector itself",
                ),
            )
            .await
            .unwrap();
        let calls = mock.get_calls().await;
        assert_eq!(calls.len(), 1, "{:?}", calls);
        assert!(calls[0].contains("package-delete"), "{:?}", calls);
    }

    /// **The names are interpolated into evaluated Lisp, so one bad name must stop the whole
    /// batch** — not fail after the good ones have already been written into the form. This is
    /// the property batching could most easily have lost.
    #[tokio::test]
    async fn one_illegal_name_refuses_the_whole_batch_before_anything_runs() {
        let (core, mock) = wired();
        let specs = vec![
            crate::core::PackageSpec {
                name: "csv-mode".into(),
                backend: "emacs".into(),
                ..Default::default()
            },
            crate::core::PackageSpec {
                name: "evil (shell-command \"rm -rf /\")".into(),
                backend: "emacs".into(),
                ..Default::default()
            },
        ];
        let err = EmacsInstallable { core }.install(&specs, false).await;
        assert!(err.is_err(), "an illegal symbol must refuse the batch");
        assert!(
            mock.get_calls().await.is_empty(),
            "nothing may run when any name in the batch is refused"
        );
    }
}
