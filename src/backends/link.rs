use crate::config::Config;
use crate::core::{
    BackendCore, CommandExecutor, Error, Installable, MetadataProvider, PackageSpec, Result,
};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tera::{Context, Tera};
use tracing::{debug, info, warn};

pub struct LinkBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
    pub config: Arc<Config>,
    /// Declared secret-decryption providers (U38), beyond the built-in `age`/`sops`.
    pub secret_providers: Vec<crate::model::secret::SecretProvider>,
}

/// Where a `@target=` value lands on disk. The install path and the pre-sync confirmation
/// must answer this the same way, or the run confirms one destination and writes another.
pub fn resolve_target(target: &str) -> Result<PathBuf> {
    let home = || dirs::home_dir().ok_or_else(|| Error::Other("Could not find home".into()));
    if let Some(rest) = target.strip_prefix("~/") {
        Ok(home()?.join(rest))
    } else if target == "~" {
        home()
    } else {
        Ok(PathBuf::from(target))
    }
}

/// Where a declared source file lives on disk — the `link:` name, or a `dotfiles:` tree.
///
/// **A relative source is relative to the config repo.** Not to the process's working
/// directory, and emphatically not to the directory holding the link: a symlink stores its
/// destination as a string and resolves it against its own location on every open, so
/// `link:./dotfiles/vimrc@target=/root/.vimrc` written verbatim produced `/root/.vimrc ->
/// ./dotfiles/vimrc`, which means `/root/dotfiles/vimrc`. That file does not exist. The dotfile
/// could not be read at all, by anything, and `check` filed the whole state under `ok` (B0b).
///
/// The config repo is the only reading that makes the declaration portable, and it is what
/// `dotfiles:` has always done with the identical string — which is why this is one function
/// and not two. **Every caller that touches a declared source must come through here**: the
/// installer, the readback that decides whether a resource is in effect, and the tree walk.
/// A source resolved one way when it is written and another way when it is read is how a
/// dangling link comes to look verified.
pub fn resolve_source(config: &Config, declared: &str) -> Result<PathBuf> {
    let expanded = resolve_target(declared)?;
    Ok(match expanded.is_absolute() {
        true => expanded,
        false => config.config_root().join(expanded),
    })
}

/// Whether a `link:` line reads its source file at all.
///
/// `@content=` declares the bytes inline and never opens the name; every other mode — a plain
/// symlink, `@template=true`, `@decrypt=` — needs the file to be there.
pub fn reads_its_source(opts: &crate::config::grammar::Options) -> bool {
    opts.one("content").is_none()
}

/// [`resolve_source`], refusing when the file is not on disk.
///
/// **A `link:` whose source does not exist cannot be placed, and placing it anyway is worse
/// than refusing.** `symlink` happily writes a pointer to nothing: the result exists, satisfies
/// an `-L` test, and reads back as *"Shall cannot read back"* — a sentence `check` then filed
/// under `ok`. So the failure mode of not asking this question is a green health check over a
/// dotfile that no program can open (B0b).
///
/// `dotfiles:` has refused the same way since it was written — *"is not a directory"* — on the
/// identical string. This is its twin answering the same question the same way, which is the
/// whole of what went wrong: one idea, two implementations, one of them right.
pub fn resolve_existing_source(config: &Config, declared: &str) -> Result<PathBuf> {
    let resolved = resolve_source(config, declared)?;
    if resolved.exists() {
        return Ok(resolved);
    }
    Err(Error::Validation(format!(
        "`link:{}` names a file that is not there ({}). A relative source is read from your \
         config repo at {} — check the path, or make it absolute. Shall will not place a link \
         to nothing: it would exist, and nothing could open it.",
        declared,
        resolved.display(),
        config.config_root().display()
    )))
}

/// Where the pre-existing file at `target` is kept while Shall owns that path. One function,
/// because the write path and the undo path must agree on the name or a restore looks for a
/// file nothing wrote.
pub fn backup_path(target: &Path) -> PathBuf {
    PathBuf::from(format!("{}.shall-backup", target.display()))
}

/// Whether a `link:` line wants its pre-existing target preserved (T6). Backing up is the
/// default; `@backup=no` opts a single line out, stated where the exception is. A machine-wide
/// key was deliberately not added — restore-on-removal already kills the pile-up one would have
/// been for, so one mechanism answers the whole question.
pub fn wants_backup(spec: &PackageSpec) -> bool {
    !matches!(spec.options.one("backup"), Some("no") | Some("false"))
}

/// Refuse a decrypted secret whose destination is inside the config repo (T2).
///
/// The repo is git-tracked and `sync` commits it, so a plaintext written there is a plaintext
/// in history — and a secret in git history is a rotated secret, which is unrecoverable rather
/// than merely bad. The refusal names both paths, because "somewhere inside your repo" is not
/// something a reader can act on.
pub fn refuse_target_in_repo(config: &Config, resolved: &Path) -> Result<()> {
    let root = config.config_root();
    let inside = match (resolved.canonicalize(), root.canonicalize()) {
        // Canonicalising the target fails when it does not exist yet, which is the ordinary
        // case for a first install — so compare the paths as written when it does.
        (Ok(t), Ok(r)) => t.starts_with(r),
        _ => resolved.starts_with(&root),
    };
    if !inside {
        return Ok(());
    }
    Err(Error::Refused(format!(
        "refusing to decrypt into {} — it is inside the config repo at {}, which git tracks \
         and `sync` commits. A secret that reaches git history has to be rotated, not deleted. \
         Point @target= outside the repo.",
        resolved.display(),
        root.display()
    )))
}

/// Whether a resolved `@target` lands outside the user's home directory. An unknown home
/// counts as outside: the point of the question is that the destination is not one of the
/// dotfiles the link backend exists for, and a machine that cannot say where home is
/// cannot say the path is under it.
pub fn is_outside_home(resolved: &Path) -> bool {
    match dirs::home_dir() {
        Some(home) => !resolved.starts_with(home),
        None => true,
    }
}

/// The argument list for a decrypt tool. `-i` takes the identity as its value, so it stays
/// in front of the terminator; only the source path goes behind it.
fn decrypt_argv(tool: &str, source: &Path, identity: Option<&Path>) -> Result<Vec<String>> {
    let mut args = vec!["--decrypt".to_string()];
    match tool {
        "age" => {
            let identity = identity.ok_or_else(|| {
                Error::Other(
                    "age decrypt needs an identity — set @identity=<path> or $SHALL_AGE_IDENTITY"
                        .into(),
                )
            })?;
            args.push("-i".to_string());
            args.push(identity.to_string_lossy().to_string());
        }
        "sops" => {}
        other => {
            return Err(Error::Other(format!(
                "unknown decrypt tool '{}' (use age or sops)",
                other
            )))
        }
    }
    crate::core::argv::push_names(&mut args, tool, [source.to_string_lossy().to_string()]);
    Ok(args)
}

impl LinkBackendCore {
    pub fn new(executor: CommandExecutor, config: Arc<Config>) -> Self {
        Self {
            executor,
            name: "link".to_string(),
            config,
            secret_providers: Vec::new(),
        }
    }

    /// Attach declared secret providers (U38), loaded from `adapters/secret.toml`.
    pub fn with_secret_providers(
        mut self,
        providers: Vec<crate::model::secret::SecretProvider>,
    ) -> Self {
        self.secret_providers = providers;
        self
    }

    /// Resolve the age identity file: explicit `@identity=`, else `$SHALL_AGE_IDENTITY`,
    /// else the conventional `~/.config/shall/age.key`.
    fn age_identity(&self, spec: &PackageSpec) -> Option<PathBuf> {
        if let Some(id) = spec.options.one("identity") {
            return Some(PathBuf::from(id));
        }
        if let Ok(id) = std::env::var("SHALL_AGE_IDENTITY") {
            return Some(PathBuf::from(id));
        }
        dirs::home_dir().map(|h| h.join(".config").join("shall").join("age.key"))
    }

    /// Decrypt an encrypted source file to plaintext by shelling out to the `age` or `sops`
    /// binary. Shall stays true to its "manager of managers" model: it orchestrates the
    /// tool the user already trusts rather than embedding crypto. stdout is captured raw
    /// (never trimmed) so key material survives byte-for-byte.
    /// Decrypt `source`, or return `Ok(None)` when an unattended run skips a touch-required
    /// line (T4).
    ///
    /// `Ok(None)` is a deliberate skip, not a failure: T4 says a `watch` tick does not block the
    /// whole reconcile waiting for a physical touch nobody will give. `Ok(Some)` is the
    /// plaintext; an `Err` is a real failure — including a decrypt that hung past the timeout
    /// (T3), which is the touch-required case reached at a terminal rather than under `watch`.
    async fn decrypt_secret(
        &self,
        tool: &str,
        source: &Path,
        spec: &PackageSpec,
    ) -> Result<Option<String>> {
        use tokio::process::Command;
        let identity = self.age_identity(spec);

        // Is this a hardware/interactive identity? Read the identity file and look for an age
        // plugin marker. A file we cannot read is treated as not-a-plugin: the decrypt below
        // will fail with age's own error if the identity is genuinely bad, which is the honest
        // report.
        let plugin = identity
            .as_deref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|c| crate::model::secret::plugin_of(&c));

        // T4: an unattended `watch` tick does not attempt a touch-required line.
        if self.config.unattended {
            if let Some(plugin) = &plugin {
                warn!(
                    "{}",
                    crate::model::secret::watch_skip_message(source, plugin)
                );
                return Ok(None);
            }
        }

        // `age`/`sops` are built in (age carries the hardware handling above); anything else is a
        // declared provider (U38), which plugs into this same plaintext-handling path — captured
        // from stdout, bounded by the timeout, restricted before it is written. A provider is
        // trusted only because it promised stdout-only and the ledger approved its file.
        let (program, args) = if tool == "age" || tool == "sops" {
            (
                tool.to_string(),
                decrypt_argv(tool, source, identity.as_deref())?,
            )
        } else {
            let provider = self
                .secret_providers
                .iter()
                .find(|p| p.name == tool)
                .ok_or_else(|| {
                    let mut known = vec!["age".to_string(), "sops".to_string()];
                    known.extend(self.secret_providers.iter().map(|p| p.name.clone()));
                    Error::Other(format!(
                        "unknown decrypt tool '{}' — known: {}. Add a `[[secret]]` row to \
                         `adapters/secret.toml` for another provider.",
                        tool,
                        known.join(", ")
                    ))
                })?;
            let id = identity.as_deref().map(|p| p.to_string_lossy().to_string());
            provider
                .command(&source.to_string_lossy(), id.as_deref())
                .ok_or_else(|| {
                    Error::Other(format!("the `{}` secret provider has no command", tool))
                })?
        };
        let mut cmd = Command::new(&program);
        cmd.args(&args);
        // T3: a decrypt that does not complete is waiting on a prompt nobody will answer. Bound
        // it, and on timeout name the token and the identity rather than leaving the process
        // (and this sync) hung forever.
        //
        // **And the process really does stop now.** Dropping the future that awaits a child does
        // not kill it, so this bound used to free the sync and leave `gpg` running against a
        // prompt for as long as the machine was up — the one thing the comment above promised it
        // would not do. `supervised_output` owns the child, so the timeout below reaches it.
        let output = match tokio::time::timeout(
            crate::model::secret::DECRYPT_TIMEOUT,
            crate::core::supervise::supervised_output(cmd, &program, false),
        )
        .await
        {
            Ok(result) => result.map_err(|e| {
                Error::Other(format!(
                    "could not launch '{}' to decrypt {:?}: {} — is it installed and on PATH?",
                    tool, source, e
                ))
            })?,
            Err(_) => {
                return Err(Error::Other(crate::model::secret::token_timeout_message(
                    source,
                    identity.as_deref().unwrap_or(Path::new("(none)")),
                    plugin.as_deref(),
                )));
            }
        };
        if !output.status.success() {
            return Err(Error::Other(format!(
                "{} failed to decrypt {:?}: {}",
                tool,
                source,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        String::from_utf8(output.stdout).map(Some).map_err(|e| {
            Error::Other(format!(
                "decrypted content of {:?} is not valid UTF-8: {}",
                source, e
            ))
        })
    }

    async fn render_template(&self, source_path: &Path) -> Result<String> {
        let content = self.executor.read_file(source_path).await?;

        let mut tera = Tera::default();
        tera.add_raw_template("config", &content)
            .map_err(|e| Error::Other(format!("Tera Parse Error in {:?}: {}", source_path, e)))?;

        let mut context = Context::new();
        context.insert("OS".to_string(), std::env::consts::OS);
        context.insert("ARCH".to_string(), std::env::consts::ARCH);
        context.insert(
            "USER",
            &std::env::var("USER").unwrap_or_else(|_| "unknown".to_string()),
        );
        context.insert("HOSTNAME".to_string(), &Config::get_hostname());

        context.insert("aliases".to_string(), &self.config.aliases);

        tera.render("config", &context)
            .map_err(|e| Error::Other(format!("Tera Render Error in {:?}: {}", source_path, e)))
    }

    /// Write `desired` content to `target`, idempotently. If the target already holds
    /// exactly this content it is left untouched (no backup, no write); otherwise any
    /// pre-existing file is backed up (once) before the managed content is written.
    /// Shared by inline-content and rendered-template modes.
    async fn apply_managed_content(
        &self,
        target: &Path,
        desired: &str,
        backup: bool,
    ) -> Result<()> {
        if let Ok(existing) = self.executor.read_file(target).await {
            if existing == desired {
                debug!("Link: {:?} is already up-to-date.", target);
                return Ok(());
            }
        }
        if backup {
            self.backup_once(target).await?;
        }
        info!("Link: Writing managed file {:?}", target);
        self.executor.write_atomic(target, desired).await?;
        Ok(())
    }

    /// Preserve a pre-existing, unmanaged file before Shall overwrites or replaces it —
    /// exactly once, as `<target>.shall-backup`. So the user is never silently robbed of
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
        let backup = backup_path(target);
        if tokio::fs::try_exists(&backup).await.unwrap_or(false) {
            return Ok(()); // the original was already preserved on an earlier run
        }
        if self.executor.dry_run {
            crate::would!(
                "Link: would back up existing {:?} to {:?} before writing the managed version.",
                target,
                backup
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
}

/// Whether Windows refused a symlink because the process may not create one.
///
/// `CreateSymbolicLinkW` fails with `ERROR_PRIVILEGE_NOT_HELD` (1314) unless the process is
/// elevated or Developer Mode is on. The executor renders io errors to strings before this
/// layer sees them, so the code is matched in the one rendering Rust guarantees for an OS
/// error: a trailing `(os error N)`.
#[cfg(target_os = "windows")]
fn is_missing_symlink_privilege(e: &Error) -> bool {
    matches!(e, Error::Io(msg) if msg.contains("os error 1314"))
}

/// Said when a `link:` becomes a copy, because the two differ in a way the user will meet
/// later: edits to the source stop appearing at the destination until the next sync. Names
/// the privilege and how to get it, since "fell back to a copy" is not something a reader
/// can act on.
#[cfg(target_os = "windows")]
fn copy_fallback_message(source: &Path, target: &Path) -> String {
    format!(
        "Link: placed {:?} as a COPY of {:?}, not a link — creating symlinks needs Developer \
         Mode or an elevated shell. Edits to the source will not appear at the destination \
         until the next sync. Settings > For developers > Developer Mode turns this into a \
         real link.",
        target, source
    )
}

#[async_trait]
impl BackendCore for LinkBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        true
    }
    fn probes(&self) -> Vec<String> {
        Vec::new()
    }
    fn needs_root(&self) -> bool {
        false
    }
}

/// Filesystem objects have no native transitive deps.
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
    /// Must go through `executor.symlink()` rather than `tokio::fs` directly: only the
    /// executor records into the dry-run VFS, so a direct call would touch the real
    /// filesystem during a dry run.
    async fn install(&self, specs: &[PackageSpec], _: bool) -> Result<()> {
        for spec in specs {
            let target_str = spec
                .options
                .one("target")
                .ok_or_else(|| Error::Other("Link requires @target".into()))?;

            let target_path = resolve_target(target_str)?;
            let backup = wants_backup(spec);

            // Mode A: Inline content declared directly (no separate source file).
            if let Some(content) = spec.options.one("content") {
                self.core
                    .apply_managed_content(&target_path, content, backup)
                    .await?;
                continue;
            }

            // Resolved, not stored verbatim. The symlink written below keeps whatever string it
            // is given and resolves it against the *link's* directory, so a relative source is
            // a dangling link at a path nobody in the config repo named (B0b). Refused rather
            // than resolved when the file is not there, because the three modes below all read
            // it and a link to nothing is the failure this whole path exists to prevent.
            let source = resolve_existing_source(&self.core.config, &spec.name)?;

            // Mode D: Secret — decrypt the source with age/sops and place the plaintext.
            if let Some(tool) = spec.options.one("decrypt") {
                // T2, before anything is decrypted: the config root is a git repo, and a
                // plaintext written inside it is committed by the next sync. A secret in git
                // history is a rotated secret, so this is a refusal rather than a warning.
                refuse_target_in_repo(&self.core.config, &target_path)?;
                if self.core.executor.dry_run {
                    crate::would!(
                        "Link: would decrypt {:?} with {} and write to {:?}",
                        source,
                        tool,
                        target_path
                    );
                    continue;
                }
                // `None` is T4's deliberate skip (an unattended tick met a touch-required key);
                // decrypt_secret already said so. Move to the next line rather than failing.
                let Some(plaintext) = self.core.decrypt_secret(tool, &source, spec).await? else {
                    continue;
                };
                // T1: no backup. `backup_once` exists so a user is not silently robbed of a
                // config file they hand-wrote; a secret Shall decrypted a moment ago is not
                // that, and the copy would sit beside the target under the ordinary umask,
                // outlasting the declaration that made it.
                if let Ok(existing) = self.core.executor.read_file(&target_path).await {
                    if existing == plaintext {
                        debug!("Link: {:?} is already up-to-date.", target_path);
                        continue;
                    }
                }
                // T5: restricted before it lands, not chmod'd after.
                self.core
                    .executor
                    .write_secret(&target_path, &plaintext)
                    .await?;
                info!("Link: Writing managed secret {:?}", target_path);
                continue;
            }

            // Mode B: Rendered template read from a source file.
            if spec.options.one("template") == Some("true") {
                let rendered = self.core.render_template(&source).await?;
                self.core
                    .apply_managed_content(&target_path, &rendered, backup)
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

                // A copy Shall made is as much "already in effect" as a symlink it made.
                // Windows without the symlink privilege gets a copy, and asking only
                // `read_link` meant every later sync backed up its own copy as
                // `<target>.shall-backup` and wrote the file again, under a summary
                // reading `already up to date`.
                if exists && !is_symlink {
                    if let (Ok(from), Ok(to)) = (
                        tokio::fs::read(&source).await,
                        tokio::fs::read(&target_path).await,
                    ) {
                        if from == to {
                            debug!("Link: {:?} already matches {:?}", target_path, source);
                            continue;
                        }
                    }
                }

                // Preserve a pre-existing real file before replacing it with our symlink.
                if backup {
                    self.core.backup_once(&target_path).await?;
                }

                if self.core.executor.dry_run {
                    crate::would!("Would remove existing file/link at {:?}", target_path);
                } else {
                    crate::utils::file::remove_deployed_path(&target_path)
                        .await
                        .map_err(Error::Io)?;
                }
            }

            info!("Link: Creating link {:?} -> {:?}", source, target_path);

            // Delegate to the executor so the dry-run VFS records this instead of the
            // real filesystem being touched.
            // A symlink is attempted for every target, including one on another drive: a
            // Windows symlink stores the destination as a string and resolves it on open, so
            // it spans volumes. Only the privilege varies, so only the privilege is handled.
            #[cfg(target_os = "windows")]
            {
                match self.core.executor.symlink(&source, &target_path).await {
                    Err(e) if is_missing_symlink_privilege(&e) => {
                        warn!("{}", copy_fallback_message(&source, &target_path));
                        tokio::fs::copy(&source, &target_path)
                            .await
                            .map_err(Error::from)?;
                    }
                    other => other?,
                }
            }

            #[cfg(unix)]
            {
                self.core.executor.symlink(&source, &target_path).await?;
            }
        }
        Ok(())
    }

    /// Undo a `link:` declaration. Each name is the DESTINATION Shall wrote — never the source
    /// in your repo, which Shall does not own and must not delete.
    ///
    /// **A declaration undoes what it did (T6).** If a `<target>.shall-backup` is sitting there,
    /// the target was somebody's file before Shall took it over: the backup is put back and the
    /// backup file removed, so a `link:` line that comes and goes leaves the machine as it found
    /// it and nothing accumulates. With no backup there was nothing there before, so the target
    /// is removed.
    async fn remove(
        &self,
        names: &[String],
        _: bool,
        _reaped: crate::app::sync::guard::Reaped,
    ) -> Result<()> {
        for name in names {
            let path = Path::new(name);
            let exists = tokio::fs::try_exists(path).await.unwrap_or(false);
            let is_symlink = path.is_symlink();
            let backup = backup_path(path);
            let has_backup = tokio::fs::try_exists(&backup).await.unwrap_or(false);

            if !exists && !is_symlink && !has_backup {
                continue;
            }

            if self.core.executor.dry_run {
                if has_backup {
                    crate::would!("Link: would restore {:?} from {:?}", path, backup);
                } else {
                    crate::would!("Link: would remove {:?}", path);
                }
                continue;
            }

            if exists || is_symlink {
                crate::utils::file::remove_deployed_path(path)
                    .await
                    .map_err(Error::Io)?;
            }

            if has_backup {
                // A failed restore leaves the user with neither their file nor an error, so
                // it propagates rather than being logged past.
                tokio::fs::rename(&backup, path)
                    .await
                    .map_err(Error::from)?;
                info!(
                    "Link: {:?} restored from the backup taken when it was declared.",
                    path
                );
            } else {
                info!("Link: removed {:?}", path);
            }
        }
        Ok(())
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    cfg: &crate::config::Config,
) {
    // Declared secret providers (U38), through the same approved loader every adapter file uses.
    let layout = cfg.layout();
    let secret_providers = crate::backends::onboarder::read_approved_definitions(
        &layout.adapter_secret_file(),
        &layout.locks_dir(),
    )
    .and_then(
        |body| match toml::from_str::<crate::model::secret::SecretProviderFile>(&body) {
            Ok(f) => Some(crate::model::secret::providers(f.secret)),
            Err(e) => {
                tracing::warn!(
                    "{}",
                    crate::app::adapters::cannot_use(
                        crate::app::adapters::surface("secret").expect("a declared surface"),
                        e,
                    )
                );
                None
            }
        },
    )
    .unwrap_or_default();

    let core = Arc::new(
        LinkBackendCore::new(exec.duplicate(), Arc::new(cfg.clone()))
            .with_secret_providers(secret_providers),
    );
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

    use tempfile::tempdir;

    fn installer() -> LinkInstallable {
        // A real (non-dry-run) executor so backup copies and writes hit the tempdir.
        let exec = CommandExecutor::new(false, false);
        let core = Arc::new(LinkBackendCore::new(exec, Arc::new(Config::default())));
        LinkInstallable { core }
    }

    fn installer_rooted_at(cfg: &Config) -> LinkInstallable {
        let exec = CommandExecutor::new(false, false);
        let core = Arc::new(LinkBackendCore::new(exec, Arc::new(cfg.clone())));
        LinkInstallable { core }
    }

    /// **Placed is not the same as working, and only one of them is the promise.**
    ///
    /// The check that passed over a dangling dotfile was "does the target exist" — `-L` is true
    /// of a symlink whose destination is not there. So this asserts the target can be *read*,
    /// through the link, byte for byte. That is the property a user has when they declare a
    /// dotfile, and it is the one that was false (B0b).
    #[tokio::test]
    async fn a_link_declared_the_way_the_readme_writes_it_can_actually_be_read() {
        let dir = tempdir().unwrap();
        let cfg = Config::sandboxed(dir.path());
        let src = cfg.config_root().join("dotfiles");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("vimrc"), b"set nocompatible\n").unwrap();

        // Somewhere that is not the config repo and not the process's working directory —
        // which is exactly the pair the verbatim source accidentally worked under.
        let target = dir.path().join("home").join(".vimrc");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();

        let mut options = crate::config::grammar::Options::default();
        options.set("target", target.to_string_lossy().to_string());
        let spec = PackageSpec {
            name: "./dotfiles/vimrc".into(),
            backend: "link".into(),
            options,
            requires: vec![],
            present: true,
        };

        installer_rooted_at(&cfg)
            .install(std::slice::from_ref(&spec), false)
            .await
            .expect("a link whose source is in the config repo must place");

        assert_eq!(
            std::fs::read(&target).expect(
                "the dotfile could not be opened — the link points at a path nothing wrote, \
                 which is the whole of B0b"
            ),
            b"set nocompatible\n"
        );

        // Idempotent for the same reason it is readable: the second run compares the link
        // against the resolved source, so it must recognise its own work.
        installer_rooted_at(&cfg)
            .install(std::slice::from_ref(&spec), false)
            .await
            .expect("re-placing an already-correct link is a no-op, not an error");
        assert_eq!(std::fs::read(&target).unwrap(), b"set nocompatible\n");
    }

    fn inline_spec(target: &Path, content: &str) -> PackageSpec {
        let mut options = crate::config::grammar::Options::default();
        options.set("target", target.to_string_lossy().to_string());
        options.set("content", content.to_string());
        PackageSpec {
            name: target.to_string_lossy().to_string(),
            backend: "link".into(),
            options,
            requires: vec![],
            present: true,
        }
    }

    #[test]
    fn a_bare_tilde_is_the_home_directory_and_is_inside_it() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(resolve_target("~").unwrap(), home);
        assert_eq!(
            resolve_target("~/.gitconfig").unwrap(),
            home.join(".gitconfig")
        );
        assert!(!is_outside_home(&resolve_target("~/.config/nvim").unwrap()));
    }

    /// **The finding, in the two lines that differed.** `link:` stored its source verbatim and
    /// `dotfiles:` resolved the identical string against the config repo, so one worked and one
    /// produced a symlink to a path nobody had named — `/root/.vimrc -> ./dotfiles/vimrc`,
    /// which a symlink resolves against `/root`, not against the repo (B0b).
    ///
    /// Asserted as *the same answer for the same string*, because the bug was never in either
    /// rule on its own. It was in there being two of them.
    #[test]
    fn a_relative_source_is_read_from_the_config_repo_whichever_declaration_names_it() {
        let dir = tempdir().unwrap();
        let cfg = Config::sandboxed(dir.path());
        let root = cfg.config_root();

        for declared in ["./dotfiles/vimrc", "dotfiles/vimrc"] {
            assert_eq!(
                resolve_source(&cfg, declared).unwrap(),
                root.join(declared),
                "`{declared}` must be read from the config repo, not from the process's \
                 working directory and not from the directory holding the link"
            );
        }

        // An absolute source is already an answer and must not be re-rooted.
        #[cfg(windows)]
        let absolute = r"C:\cfg\dotfiles\vimrc";
        #[cfg(not(windows))]
        let absolute = "/cfg/dotfiles/vimrc";
        assert_eq!(
            resolve_source(&cfg, absolute).unwrap(),
            PathBuf::from(absolute)
        );

        // And `~` means the same thing on a source as it does on a target — one expansion,
        // not two spellings of home.
        assert_eq!(
            resolve_source(&cfg, "~/.vimrc").unwrap(),
            dirs::home_dir().unwrap().join(".vimrc")
        );
    }

    /// A source that is not there is refused, rather than placed as a link to nothing.
    ///
    /// This is the half that let B0b ship. A dangling symlink *exists*: it satisfies an `-L`
    /// test, it survives a teardown check, and reading it back fails in a way that reported as
    /// "Shall cannot read back" — which `check` then filed under `ok`. `dotfiles:` has always
    /// refused its missing tree in the same position and with the same shape of message.
    #[test]
    fn a_link_to_a_file_that_is_not_there_is_refused_and_the_message_says_where_it_looked() {
        let dir = tempdir().unwrap();
        let cfg = Config::sandboxed(dir.path());

        let e = resolve_existing_source(&cfg, "./dotfiles/vimrc")
            .expect_err("a link to a file that does not exist must not be placed");
        let msg = e.to_string();
        assert!(msg.contains("dotfiles/vimrc"), "{msg}");
        assert!(
            msg.contains(&cfg.config_root().display().to_string()),
            "the refusal must name where a relative source is read from, or it is unactionable: \
             {msg}"
        );

        // The control: once the file is there, the same declaration resolves.
        let src = cfg.config_root().join("dotfiles");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("vimrc"), b"set nocompatible\n").unwrap();
        assert_eq!(
            resolve_existing_source(&cfg, "./dotfiles/vimrc").unwrap(),
            cfg.config_root().join("./dotfiles/vimrc")
        );
    }

    /// Only the modes that open the file are subject to that refusal. `@content=` carries the
    /// bytes in the line and never reads the name, so requiring the name to exist would refuse
    /// a declaration that is complete.
    #[test]
    fn a_content_link_names_no_source_and_is_not_asked_for_one() {
        let mut opts = crate::config::grammar::Options::default();
        assert!(reads_its_source(&opts));
        opts.set("content", "hello\n".to_string());
        assert!(!reads_its_source(&opts));

        // A template and a secret both read their source, so both keep the check.
        let mut template = crate::config::grammar::Options::default();
        template.set("template", "true".to_string());
        assert!(reads_its_source(&template));
        let mut secret = crate::config::grammar::Options::default();
        secret.set("decrypt", "age".to_string());
        assert!(reads_its_source(&secret));
    }

    #[test]
    fn a_system_path_is_outside_home() {
        #[cfg(windows)]
        let system = r"C:\ProgramData\shall\x";
        #[cfg(not(windows))]
        let system = "/etc/cron.d/x";
        assert!(is_outside_home(&resolve_target(system).unwrap()));
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
        let backup = backup_path(&target);
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
        let mut options = crate::config::grammar::Options::default();
        options.set("target", target.to_string_lossy().to_string());
        options.set("decrypt", tool.to_string());
        PackageSpec {
            name: source.to_string_lossy().to_string(),
            backend: "link".into(),
            options,
            requires: vec![],
            present: true,
        }
    }

    #[tokio::test]
    async fn decrypt_dry_run_writes_nothing() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("token.age");
        tokio::fs::write(&source, "ENCRYPTED").await.unwrap();
        let target = dir.path().join("token");

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

    #[test]
    fn the_source_path_goes_behind_the_terminator_and_the_identity_stays_in_front() {
        let identity = PathBuf::from("/home/u/.config/shall/age.key");
        assert_eq!(
            decrypt_argv("age", Path::new("/cfg/token.age"), Some(&identity)).unwrap(),
            [
                "--decrypt",
                "-i",
                "/home/u/.config/shall/age.key",
                "--",
                "/cfg/token.age"
            ]
        );
        assert_eq!(
            decrypt_argv("sops", Path::new("/cfg/token.enc"), None).unwrap(),
            ["--decrypt", "--", "/cfg/token.enc"]
        );
        assert!(decrypt_argv("age", Path::new("/cfg/token.age"), None).is_err());
        assert!(decrypt_argv("rot13", Path::new("/cfg/x"), None).is_err());
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
        let backup = backup_path(&target);
        assert!(
            !tokio::fs::try_exists(&backup).await.unwrap(),
            "nothing pre-existed, so no backup should be written"
        );
    }

    #[tokio::test]
    async fn backup_no_opts_a_single_line_out_of_the_backup() {
        // T6: @backup=no writes the managed content and leaves NO .shall-backup, so a user who
        // explicitly does not want the original kept does not get a stray copy beside it.
        let dir = tempdir().unwrap();
        let target = dir.path().join("gitconfig");
        tokio::fs::write(&target, "ORIGINAL").await.unwrap();

        let inst = installer();
        let mut spec = inline_spec(&target, "MANAGED");
        spec.options.set("backup", "no");
        inst.install(&[spec], false).await.unwrap();

        assert_eq!(tokio::fs::read_to_string(&target).await.unwrap(), "MANAGED");
        assert!(
            !tokio::fs::try_exists(&backup_path(&target)).await.unwrap(),
            "@backup=no must not leave a backup file"
        );
    }

    #[test]
    fn backup_defaults_on_and_only_no_or_false_opts_out() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("x");
        let mut spec = inline_spec(&target, "c");
        assert!(wants_backup(&spec), "absent @backup backs up by default");
        spec.options.set("backup", "no");
        assert!(!wants_backup(&spec));
        spec.options.set("backup", "false");
        assert!(!wants_backup(&spec));
        spec.options.set("backup", "yes");
        assert!(
            wants_backup(&spec),
            "any value but no/false keeps the backup"
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
        // The backup still holds the true pre-Shall original, not the interim edit.
        let backup = backup_path(&target);
        assert_eq!(
            tokio::fs::read_to_string(&backup).await.unwrap(),
            "PRISTINE ORIGINAL"
        );
    }
}
