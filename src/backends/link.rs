use crate::config::Config;
use crate::core::{
    BackendCore, CommandExecutor, Error, Installable, MetadataProvider, PackageSpec, Result,
};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tera::{Context, Tera};
#[allow(unused_imports)] // `warn` is used only under cfg(windows)
use tracing::{debug, info, warn};

/// Core backend implementation for filesystem links and configuration templating.
pub struct LinkBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
    pub config: Arc<Config>,
}

impl LinkBackendCore {
    pub fn new(executor: CommandExecutor, config: Arc<Config>) -> Self {
        Self {
            executor,
            name: "link".to_string(),
            config,
        }
    }

    /// Resolve the age identity file: explicit `@identity=`, else `$LINIX_AGE_IDENTITY`,
    /// else the conventional `~/.config/linix/age.key`.
    fn age_identity(&self, spec: &PackageSpec) -> Option<PathBuf> {
        if let Some(id) = spec.options.get("identity") {
            return Some(PathBuf::from(id));
        }
        if let Ok(id) = std::env::var("LINIX_AGE_IDENTITY") {
            return Some(PathBuf::from(id));
        }
        dirs::home_dir().map(|h| h.join(".config").join("linix").join("age.key"))
    }

    /// Decrypt an encrypted source file to plaintext by shelling out to the `age` or `sops`
    /// binary. LiNix stays true to its "manager of managers" model: it orchestrates the
    /// tool the user already trusts rather than embedding crypto. stdout is captured raw
    /// (never trimmed) so key material survives byte-for-byte.
    async fn decrypt_secret(&self, tool: &str, source: &Path, spec: &PackageSpec) -> Result<String> {
        use tokio::process::Command;
        let mut cmd;
        match tool {
            "age" => {
                let identity = self.age_identity(spec).ok_or_else(|| {
                    Error::Other(
                        "age decrypt needs an identity — set @identity=<path> or $LINIX_AGE_IDENTITY"
                            .into(),
                    )
                })?;
                cmd = Command::new("age");
                cmd.arg("--decrypt").arg("-i").arg(&identity).arg(source);
            }
            "sops" => {
                cmd = Command::new("sops");
                cmd.arg("--decrypt").arg(source);
            }
            other => {
                return Err(Error::Other(format!(
                    "unknown decrypt tool '{}' (use age or sops)",
                    other
                )))
            }
        }
        let output = cmd.output().await.map_err(|e| {
            Error::Other(format!(
                "could not launch '{}' to decrypt {:?}: {} — is it installed and on PATH?",
                tool, source, e
            ))
        })?;
        if !output.status.success() {
            return Err(Error::Other(format!(
                "{} failed to decrypt {:?}: {}",
                tool,
                source,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        String::from_utf8(output.stdout).map_err(|e| {
            Error::Other(format!("decrypted content of {:?} is not valid UTF-8: {}", source, e))
        })
    }

    /// Renders a configuration file using the Tera engine.
    async fn render_template(&self, source_path: &Path) -> Result<String> {
        let content = self.executor.read_file(source_path).await?;

        let mut tera = Tera::default();
        tera.add_raw_template("config", &content)
            .map_err(|e| Error::Other(format!("Tera Parse Error in {:?}: {}", source_path, e)))?;

        let mut context = Context::new();
        context.insert("OS", std::env::consts::OS);
        context.insert("ARCH", std::env::consts::ARCH);
        context.insert(
            "USER",
            &std::env::var("USER").unwrap_or_else(|_| "unknown".to_string()),
        );
        context.insert("HOSTNAME", &Config::get_hostname());

        context.insert("aliases", &self.config.aliases);
        context.insert("groups", &self.config.groups);

        tera.render("config", &context)
            .map_err(|e| Error::Other(format!("Tera Render Error in {:?}: {}", source_path, e)))
    }

    /// Write `desired` content to `target`, idempotently. If the target already holds
    /// exactly this content it is left untouched (no backup, no write); otherwise any
    /// pre-existing file is backed up (once) before the managed content is written.
    /// Shared by inline-content and rendered-template modes.
    async fn apply_managed_content(&self, target: &Path, desired: &str) -> Result<()> {
        if let Ok(existing) = self.executor.read_file(target).await {
            if existing == desired {
                debug!("Link: {:?} is already up-to-date.", target);
                return Ok(());
            }
        }
        self.backup_once(target).await?;
        info!("Link: Writing managed file {:?}", target);
        self.executor.write_atomic(target, desired).await?;
        Ok(())
    }

    /// Preserve a pre-existing, unmanaged file before LiNix overwrites or replaces it —
    /// exactly once, as `<target>.linix-backup`. So the user is never silently robbed of
    /// a config file they already had. Symlinks (mere pointers) and directories are
    /// skipped, and an existing backup is never clobbered, so the true original survives
    /// even across repeated syncs. Honors dry-run (previews instead of copying).
    async fn backup_once(&self, target: &Path) -> Result<()> {
        // A symlink is just a pointer; the real data lives elsewhere and is untouched.
        if target.is_symlink() {
            return Ok(());
        }
        if !tokio::fs::try_exists(target).await.unwrap_or(false) {
            return Ok(()); // nothing there to preserve
        }
        let backup = PathBuf::from(format!("{}.linix-backup", target.display()));
        if tokio::fs::try_exists(&backup).await.unwrap_or(false) {
            return Ok(()); // the original was already preserved on an earlier run
        }
        if self.executor.dry_run {
            info!(
                "[DRY-RUN] Link: would back up existing {:?} to {:?} before writing the managed version.",
                target, backup
            );
            return Ok(());
        }
        // Only regular files are byte-copied; a directory at the target is left alone
        // rather than silently folded into a single backup file.
        let meta = tokio::fs::symlink_metadata(target)
            .await
            .map_err(Error::from)?;
        if meta.is_dir() {
            warn!(
                "Link: {:?} is an existing directory; not auto-backing it up before replacement.",
                target
            );
            return Ok(());
        }
        tokio::fs::copy(target, &backup)
            .await
            .map_err(Error::from)?;
        info!(
            "Link: Existing {:?} was backed up to {:?} before applying the managed version.",
            target, backup
        );
        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn is_same_drive(a: &Path, b: &Path) -> bool {
        use std::path::Component;
        let drive_a = a.components().find(|c| matches!(c, Component::Prefix(_)));
        let drive_b = b.components().find(|c| matches!(c, Component::Prefix(_)));
        drive_a == drive_b
    }
}

#[async_trait]
impl BackendCore for LinkBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        true
    }
    fn needs_root(&self) -> bool {
        false
    }
}

/// Phase 1.1: MetadataProvider for Link (Filesystem objects have no native transitive deps).
#[async_trait]
impl MetadataProvider for LinkBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub struct LinkInstallable {
    pub core: Arc<LinkBackendCore>,
}

#[async_trait]
impl Installable for LinkInstallable {
    /// Aligns the target file with the source (Link or Rendered Template).
    /// Hardened for Phase 1.1 Correction: Uses the abstracted executor.symlink()
    /// to ensure dry-run VFS recording.
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        for spec in specs {
            let target_str = spec
                .options
                .get("target")
                .ok_or_else(|| Error::Other("Link requires @target".into()))?;

            let target_path = if target_str.starts_with('~') {
                dirs::home_dir()
                    .ok_or_else(|| Error::Other("Could not find home".into()))?
                    .join(&target_str[2..])
            } else {
                PathBuf::from(target_str)
            };

            // Mode A: Inline content declared directly (no separate source file).
            if let Some(content) = spec.options.get("content") {
                self.core
                    .apply_managed_content(&target_path, content)
                    .await?;
                continue;
            }

            let source = PathBuf::from(&spec.name);

            // Mode D: Secret — decrypt the source with age/sops and place the plaintext,
            // locked down to owner-only (0600) on Unix.
            if let Some(tool) = spec.options.get("decrypt") {
                if self.core.executor.dry_run {
                    info!(
                        "[DRY-RUN] Link: would decrypt {:?} with {} and write to {:?}",
                        source, tool, target_path
                    );
                    continue;
                }
                let plaintext = self.core.decrypt_secret(tool, &source, spec).await?;
                self.core
                    .apply_managed_content(&target_path, &plaintext)
                    .await?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = tokio::fs::set_permissions(
                        &target_path,
                        std::fs::Permissions::from_mode(0o600),
                    )
                    .await;
                }
                continue;
            }

            // Mode B: Rendered template read from a source file.
            if spec.options.get("template") == Some(&"true".to_string()) {
                let rendered = self.core.render_template(&source).await?;
                self.core
                    .apply_managed_content(&target_path, &rendered)
                    .await?;
                continue;
            }

            // Mode C: Standard Symlinking Path
            let exists = tokio::fs::try_exists(&target_path).await.unwrap_or(false);
            let is_symlink = target_path.is_symlink();

            if exists || is_symlink {
                if let Ok(existing_link) = tokio::fs::read_link(&target_path).await {
                    if existing_link == source {
                        debug!("Link: Correct symlink already exists at {:?}", target_path);
                        continue;
                    }
                }

                // Preserve a pre-existing real file before replacing it with our symlink.
                self.core.backup_once(&target_path).await?;

                if self.core.executor.dry_run {
                    info!(
                        "[DRY-RUN] Would remove existing file/link at {:?}",
                        target_path
                    );
                } else {
                    let metadata = tokio::fs::symlink_metadata(&target_path)
                        .await
                        .map_err(Error::from)?;
                    if metadata.is_dir() && !metadata.is_symlink() {
                        tokio::fs::remove_dir_all(&target_path)
                            .await
                            .map_err(Error::from)?;
                    } else {
                        tokio::fs::remove_file(&target_path)
                            .await
                            .map_err(Error::from)?;
                    }
                }
            }

            info!("Link: Creating link {:?} -> {:?}", source, target_path);

            // Fulfills Phase 1.1 Correction: Delegate to executor to allow VFS recording in tests.
            #[cfg(target_os = "windows")]
            {
                let source_abs = source.canonicalize().unwrap_or_else(|_| source.clone());
                if !self.core.executor.dry_run
                    && !LinkBackendCore::is_same_drive(&source_abs, &target_path)
                {
                    warn!("Link: Cross-drive fallback to COPY for {:?}", source);
                    tokio::fs::copy(&source, &target_path)
                        .await
                        .map_err(Error::from)?;
                } else {
                    // This now handles dry-run automatically via VFS
                    self.core.executor.symlink(&source, &target_path).await?;
                }
            }

            #[cfg(unix)]
            {
                self.core.executor.symlink(&source, &target_path).await?;
            }
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _: bool) -> Result<()> {
        for name in names {
            let path = Path::new(name);
            let exists = tokio::fs::try_exists(path).await.unwrap_or(false);
            let is_symlink = path.is_symlink();

            if exists || is_symlink {
                info!("Link: Removing link/rendered file {:?}", path);
                if !self.core.executor.dry_run {
                    let metadata = tokio::fs::symlink_metadata(path)
                        .await
                        .map_err(Error::from)?;
                    if metadata.is_dir() && !metadata.is_symlink() {
                        tokio::fs::remove_dir_all(path).await.map_err(Error::from)?;
                    } else {
                        tokio::fs::remove_file(path).await.map_err(Error::from)?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Build and register the symlink/template backend.
pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    cfg: &crate::config::Config,
) {
    let core = Arc::new(LinkBackendCore::new(
        exec.duplicate(),
        Arc::new(cfg.clone()),
    ));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(LinkInstallable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::CommandExecutor;
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn installer() -> LinkInstallable {
        // A real (non-dry-run) executor so backup copies and writes hit the tempdir.
        let exec = CommandExecutor::new(false, false);
        let core = Arc::new(LinkBackendCore::new(exec, Arc::new(Config::default())));
        LinkInstallable { core }
    }

    fn inline_spec(target: &Path, content: &str) -> PackageSpec {
        let mut options = HashMap::new();
        options.insert("target".into(), target.to_string_lossy().to_string());
        options.insert("content".into(), content.to_string());
        PackageSpec {
            name: target.to_string_lossy().to_string(),
            backend: "link".into(),
            options,
            requires: vec![],
        }
    }

    #[tokio::test]
    async fn backs_up_preexisting_file_then_writes_managed_content() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("gitconfig");
        tokio::fs::write(&target, "ORIGINAL USER CONTENT")
            .await
            .unwrap();

        let inst = installer();
        let spec = inline_spec(&target, "MANAGED CONTENT");
        inst.install(std::slice::from_ref(&spec), false)
            .await
            .unwrap();

        // The managed content is in place, and the user's original is preserved.
        assert_eq!(
            tokio::fs::read_to_string(&target).await.unwrap(),
            "MANAGED CONTENT"
        );
        let backup = PathBuf::from(format!("{}.linix-backup", target.display()));
        assert_eq!(
            tokio::fs::read_to_string(&backup).await.unwrap(),
            "ORIGINAL USER CONTENT"
        );

        // Idempotent: re-applying does not touch the single original backup.
        inst.install(&[spec], false).await.unwrap();
        assert_eq!(
            tokio::fs::read_to_string(&backup).await.unwrap(),
            "ORIGINAL USER CONTENT"
        );
    }

    fn decrypt_spec(source: &Path, target: &Path, tool: &str) -> PackageSpec {
        let mut options = HashMap::new();
        options.insert("target".into(), target.to_string_lossy().to_string());
        options.insert("decrypt".into(), tool.to_string());
        PackageSpec {
            name: source.to_string_lossy().to_string(),
            backend: "link".into(),
            options,
            requires: vec![],
        }
    }

    #[tokio::test]
    async fn decrypt_dry_run_writes_nothing() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("token.age");
        tokio::fs::write(&source, "ENCRYPTED").await.unwrap();
        let target = dir.path().join("token");

        // dry_run = true
        let exec = CommandExecutor::new(true, false);
        let core = Arc::new(LinkBackendCore::new(exec, Arc::new(Config::default())));
        let inst = LinkInstallable { core };
        inst.install(&[decrypt_spec(&source, &target, "age")], false)
            .await
            .unwrap();
        assert!(
            !tokio::fs::try_exists(&target).await.unwrap(),
            "dry-run must not decrypt or write the secret"
        );
    }

    #[tokio::test]
    async fn decrypt_unknown_tool_errors() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("s.enc");
        tokio::fs::write(&source, "x").await.unwrap();
        let target = dir.path().join("out");
        let inst = installer(); // real executor
        let r = inst
            .install(&[decrypt_spec(&source, &target, "rot13")], false)
            .await;
        assert!(r.is_err(), "an unknown decrypt tool must be rejected");
    }

    #[tokio::test]
    async fn no_backup_created_when_target_absent() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("fresh.conf");
        let inst = installer();
        inst.install(&[inline_spec(&target, "HELLO")], false)
            .await
            .unwrap();

        assert_eq!(tokio::fs::read_to_string(&target).await.unwrap(), "HELLO");
        let backup = PathBuf::from(format!("{}.linix-backup", target.display()));
        assert!(
            !tokio::fs::try_exists(&backup).await.unwrap(),
            "nothing pre-existed, so no backup should be written"
        );
    }

    #[tokio::test]
    async fn a_users_edit_after_adoption_does_not_clobber_the_original_backup() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("app.conf");
        tokio::fs::write(&target, "PRISTINE ORIGINAL")
            .await
            .unwrap();
        let inst = installer();

        inst.install(&[inline_spec(&target, "v1")], false)
            .await
            .unwrap();
        // Simulate the user hand-editing the managed file, then a later sync with new content.
        tokio::fs::write(&target, "user tweak").await.unwrap();
        inst.install(&[inline_spec(&target, "v2")], false)
            .await
            .unwrap();

        assert_eq!(tokio::fs::read_to_string(&target).await.unwrap(), "v2");
        // The backup still holds the true pre-LiNix original, not the interim edit.
        let backup = PathBuf::from(format!("{}.linix-backup", target.display()));
        assert_eq!(
            tokio::fs::read_to_string(&backup).await.unwrap(),
            "PRISTINE ORIGINAL"
        );
    }
}
