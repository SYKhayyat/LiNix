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
        for spec in specs {
            validate_symbol(&spec.name)?;
            info!("Emacs: Installing package '{}'...", spec.name);

            let lisp = format!(
                "(progn \
                    (require 'package) \
                    (package-initialize) \
                    (unless package-archive-contents (package-refresh-contents)) \
                    (package-install '{}) \
                )",
                spec.name
            );

            self.core
                .executor
                .run_exclusive("emacs", "emacs", &["--batch", "--eval", &lisp], false)
                .await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _: bool) -> Result<()> {
        for name in names {
            validate_symbol(name)?;
            info!("Emacs: Removing package '{}'...", name);

            let lisp = format!(
                "(progn \
                    (require 'package) \
                    (package-initialize) \
                    (let ((p (cadr (assoc '{} package-alist)))) \
                        (if p (package-delete p))) \
                )",
                name
            );

            self.core
                .executor
                .run_exclusive("emacs", "emacs", &["--batch", "--eval", &lisp], false)
                .await?;
        }
        Ok(())
    }
}

pub struct EmacsQueryable {
    pub core: Arc<EmacsBackendCore>,
}

#[async_trait]
impl Queryable for EmacsQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
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
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
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

    fn has_native_orphan_removal(&self) -> bool {
        true
    }

    async fn clean_orphans(&self, _: bool) -> Result<()> {
        info!("Emacs: Autoremoving unused packages...");
        let lisp = "(progn (require 'package) (package-initialize) \
            (when (fboundp 'package-autoremove) (package-autoremove)))";
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
