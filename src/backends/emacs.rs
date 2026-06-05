use crate::core::{CommandExecutor, Package, Result, PackageSpec, BackendCore, Installable, Queryable, MetadataProvider};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;

/// Core backend implementation for Emacs packages via 'package.el'.
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

    /// Internal helper to execute arbitrary Emacs Lisp code in batch mode.
    async fn run_lisp(&self, lisp: &str) -> Result<String> {
        self.executor.run_output("emacs", &["--batch", "--eval", lisp], false).await
    }
}

#[async_trait]
impl BackendCore for EmacsBackendCore {
    fn name(&self) -> &str { &self.name }

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

            self.core.executor.run_exclusive("emacs", "emacs", &["--batch", "--eval", &lisp], false).await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _: bool) -> Result<()> {
        for name in names {
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

            self.core.executor.run_exclusive("emacs", "emacs", &["--batch", "--eval", &lisp], false).await?;
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
        Ok(out.lines().filter_map(|l| {
            let (n, v) = l.split_once(' ')?;
            Some(Package::with_version(n.trim(), v.trim(), "emacs"))
        }).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        let lisp = "(progn \
            (require 'package) \
            (package-initialize) \
            (mapc (lambda (p) (princ (format \"%s\\n\" p))) package-selected-packages) \
        )";
        
        let out = self.core.run_lisp(lisp).await?;
        Ok(out.lines()
            .map(|l| Package::new(l.trim(), "emacs"))
            .collect())
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}