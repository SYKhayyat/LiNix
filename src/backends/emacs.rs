use crate::core::{CommandExecutor, Package, Result, PackageSpec, Backend, Installable, Queryable};
use async_trait::async_trait;
use tracing::{debug, info};

/// Specialized manager for Emacs packages via the 'package.el' system.
/// Uses Emacs batch mode to execute Lisp commands for package lifecycle management.
/// Serializes operations using the "emacs" LockMap key to prevent profile corruption.
pub struct EmacsManager {
    executor: CommandExecutor,
}

impl EmacsManager {
    pub fn new(executor: CommandExecutor) -> Self { 
        Self { executor } 
    }

    /// Internal helper to execute arbitrary Emacs Lisp code in batch mode.
    async fn run_lisp(&self, lisp: &str) -> Result<String> {
        // Emacs batch mode: --batch (no UI), --eval (run code)
        self.executor.run_output("emacs", &["--batch", "--eval", lisp], false).await
    }
}

impl Backend for EmacsManager {
    fn name(&self) -> &str { "emacs" }

    fn is_available(&self) -> bool { 
        self.executor.command_exists_sync("emacs") 
    }

    fn as_installable(&self) -> Option<&dyn Installable> { Some(self) }
    fn as_queryable(&self) -> Option<&dyn Queryable> { Some(self) }
}

#[async_trait]
impl Installable for EmacsManager {
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        for spec in specs {
            info!("Emacs: Installing package '{}'...", spec.name);
            
            // Lisp Logic:
            // 1. Initialize the package system
            // 2. Refresh metadata if needed
            // 3. Install the package
            let lisp = format!(
                "(progn \
                    (require 'package) \
                    (package-initialize) \
                    (unless package-archive-contents (package-refresh-contents)) \
                    (package-install '{}) \
                )", 
                spec.name
            );

            // Serialize access to ~/.emacs.d/elpa
            self.executor.run_exclusive("emacs", "emacs", &["--batch", "--eval", &lisp], false).await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _: bool) -> Result<()> {
        for name in names {
            info!("Emacs: Removing package '{}'...", name);
            
            // Lisp Logic:
            // 1. Locate the package description in the alist
            // 2. Delete it if it exists
            let lisp = format!(
                "(progn \
                    (require 'package) \
                    (package-initialize) \
                    (let ((p (cadr (assoc '{} package-alist)))) \
                        (if p (package-delete p))) \
                )", 
                name
            );

            self.executor.run_exclusive("emacs", "emacs", &["--batch", "--eval", &lisp], false).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl Queryable for EmacsManager {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        // Lisp Logic: Map across package-alist and print "name version"
        let lisp = "(progn \
            (require 'package) \
            (package-initialize) \
            (mapc (lambda (p) \
                (princ (format \"%s %s\\n\" (car p) (package-version-join (package-desc-version (cadr p)))))) \
                package-alist) \
        )";
        
        let out = self.run_lisp(lisp).await?;
        Ok(out.lines().filter_map(|l| {
            let (n, v) = l.split_once(' ')?;
            Some(Package::with_version(n.trim(), v.trim(), "emacs"))
        }).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // Lisp Logic: Print packages specifically listed in package-selected-packages
        let lisp = "(progn \
            (require 'package) \
            (package-initialize) \
            (mapc (lambda (p) (princ (format \"%s\\n\" p))) package-selected-packages) \
        )";
        
        let out = self.run_lisp(lisp).await?;
        Ok(out.lines()
            .map(|l| Package::new(l.trim(), "emacs"))
            .collect())
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        // Detailed info is complex in batch mode; we return the basic object if it's in the list
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}