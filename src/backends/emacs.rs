use crate::core::{CommandExecutor, Package, PackageManager, Result, PackageSpec, Error};
use async_trait::async_trait;
use std::collections::HashMap;

pub struct EmacsManager {
    executor: CommandExecutor,
}

impl EmacsManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor }
    }

    /// Helper to run Elisp code in batch mode and return the output
    async fn run_elisp(&self, lisp: &str) -> Result<String> {
        self.executor.run_output("emacs", &["--batch", "--eval", lisp], false).await
    }
}

#[async_trait]
impl PackageManager for EmacsManager {
    fn name(&self) -> &str { "emacs" }

    fn is_available(&self) -> bool {
        // Checks if emacs is in the PATH
        std::process::Command::new("emacs").arg("--version").output().is_ok()
    }

    async fn install(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for pkg in p {
            // REAL LOGIC: Initialize, refresh archives if needed, and install
            let lisp = format!(
                "(progn \
                    (require 'package) \
                    (package-initialize) \
                    (unless package-archive-contents (package-refresh-contents)) \
                    (package-install '{}) \
                )", pkg);
            self.executor.run("emacs", &["--batch", "--eval", &lisp], false).await?;
        }
        Ok(())
    }

    async fn remove(&self, p: &[String], _: bool) -> Result<()> {
        if p.is_empty() { return Ok(()); }
        for pkg in p {
            // REAL LOGIC: Use package-delete to cleanly remove the package and its files
            let lisp = format!(
                "(progn \
                    (require 'package) \
                    (package-initialize) \
                    (let ((pkg-desc (cadr (assoc '{} package-alist)))) \
                        (if pkg-desc (package-delete pkg-desc) \
                        (error \"Package not found\"))) \
                )", pkg);
            self.executor.run("emacs", &["--batch", "--eval", &lisp], false).await?;
        }
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Queries package-alist for names and joined version strings
        let lisp = "(progn \
            (require 'package) \
            (package-initialize) \
            (mapc (lambda (pkg) \
                (princ (format \"%s %s\\n\" (car pkg) \
                (package-version-join (package-desc-version (cadr pkg)))))) \
            package-alist))";
        
        let out = self.run_elisp(lisp).await?;
        Ok(out.lines().filter_map(|l| {
            let (name, ver) = l.split_once(' ')?;
            Some(Package {
                name: name.to_string(),
                version: Some(ver.to_string()),
                backend: "emacs".into(),
                ..Package::new("", "")
            })
        }).collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // NUCLEAR DELETE FIX: Only returns packages in 'package-selected-packages'
        // This ensures LiNix doesn't try to delete built-in Emacs features or auto-installed deps.
        let lisp = "(progn \
            (require 'package) \
            (package-initialize) \
            (mapc (lambda (pkg) (princ (format \"%s\\n\" pkg))) \
            package-selected-packages))";
        
        let out = self.run_elisp(lisp).await?;
        Ok(out.lines()
            .filter(|l| !l.is_empty())
            .map(|l| Package::new(l.trim(), "emacs"))
            .collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        // REAL LOGIC: Searches through the available archives (MELPA/ELPA)
        let lisp = format!(
            "(progn \
                (require 'package) \
                (package-initialize) \
                (unless package-archive-contents (package-refresh-contents)) \
                (mapc (lambda (pkg) \
                    (let ((name (symbol-name (car pkg)))) \
                        (if (string-match-p \"{}\" name) (princ (format \"%s\\n\" name))))) \
                package-archive-contents))", query);
        
        let out = self.run_elisp(&lisp).await?;
        Ok(out.lines()
            .filter(|l| !l.is_empty())
            .map(|l| Package::new(l.trim(), "emacs"))
            .collect())
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Extracts summary and version from the package descriptor
        let lisp = format!(
            "(progn \
                (require 'package) \
                (package-initialize) \
                (unless package-archive-contents (package-refresh-contents)) \
                (let ((desc (cadr (assoc '{} package-archive-contents)))) \
                    (if desc \
                        (princ (format \"Summary: %s\\nVersion: %s\" \
                            (package-desc-summary desc) \
                            (package-version-join (package-desc-version desc)))))))", package);
        
        let out = self.run_elisp(&lisp).await?;
        if out.is_empty() { return Ok(None); }

        let mut pkg = Package::new(package, "emacs");
        for line in out.lines() {
            if let Some(s) = line.strip_prefix("Summary: ") { pkg.description = Some(s.to_string()); }
            if let Some(v) = line.strip_prefix("Version: ") { pkg.version = Some(v.to_string()); }
        }
        Ok(Some(pkg))
    }

    async fn update(&self, _: bool) -> Result<()> {
        // Syncs the local package index with remote (MELPA/ELPA)
        self.run_elisp("(progn (require 'package) (package-refresh-contents))").await?;
        Ok(())
    }

    async fn upgrade(&self, _: bool) -> Result<()> {
        // REAL LOGIC: Iterates through installed packages and upgrades those with newer versions
        let lisp = "(progn \
            (require 'package) \
            (package-initialize) \
            (package-refresh-contents) \
            (mapc (lambda (p) \
                (let ((name (car p))) \
                    (when (package-installed-p name) \
                        (package-install name)))) \
            package-selected-packages))";
        self.executor.run("emacs", &["--batch", "--eval", lisp], false).await?;
        Ok(())
    }
}