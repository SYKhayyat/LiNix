// src/app/shell/mod.rs

use crate::app::sandbox::{Sandbox, SandboxConfig};
use crate::app::sync::{ChangePlanner, PlanScope, StateResolver, SyncEngine};
use crate::app::Machinery;
use crate::core::{Error, PackageSpec, Result};

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use tracing::{debug, info, instrument, warn};
use uuid::Uuid;

/// The package lines in a project-local `shall.txt`.
///
/// The comment rule is the grammar's, not a fourth hand-rolled one: a `#` opens a comment at
/// the start of a line or after whitespace, so `brew:jq  # my favourite` declares `brew:jq`
/// and `web:http://x/a#b` keeps its fragment. Filtering on `starts_with('#')` handled only the
/// whole-line case and handed the rest of the line to the parser as part of the name.
pub fn manifest_lines(content: &str) -> Vec<String> {
    content
        .lines()
        .map(|l| crate::config::grammar::strip_comment(l).trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

pub struct EphemeralShell {
    /// What it takes to install a session and tear it down again — including the command's
    /// removal budget, because a teardown removes real packages.
    m: Machinery,
}

impl EphemeralShell {
    pub fn new(m: Machinery) -> Self {
        Self { m }
    }

    #[instrument(skip(self, packages))]
    pub async fn enter(&self, packages: &[String]) -> Result<()> {
        let session_id = format!("shell-{}", Uuid::new_v4().simple());
        info!("starting ephemeral shell '{}'", session_id);

        // **The lock covers the provisioning, not the session** (`LockScope::Deferred`).
        // `shell` installs packages, hands the user an interactive `$SHELL`, and removes them
        // again when that shell exits. Held for the run, the 120-second exclusive lock was held
        // for as long as somebody sat at a prompt — the `edit`-blocks-on-`$EDITOR` failure AU6
        // records, with a subshell in place of vim. The scope below ends before the shell is
        // launched; `cleanup_transient_env` takes it again for the teardown.
        {
            let _data_lock = crate::core::datalock::DataLock::for_one_step("shell").await?;
            {
                let mut state_guard = self.m.state.lock().await;
                state_guard.active_session_id = Some(session_id.clone());
            }

            info!("installing session packages");
            self.provision_transient_env(packages, &session_id).await?;
        }

        let mut store_paths = Vec::new();
        for pkg_req in packages {
            let resolver = StateResolver::new(&self.m.config, self.m.registry.clone(), false).await;
            if let Ok(spec) = resolver.parse_and_probe_spec(pkg_req).await {
                if let Some(path) = self.locate_package_root(&spec).await? {
                    debug!("Mapping root for {}: {:?}", spec.name, path);
                    store_paths.push((path.to_string_lossy().to_string(), spec.name.clone()));
                }
            }
        }

        let shell_bin = env::var("SHELL").unwrap_or_else(|_| {
            if cfg!(windows) {
                "cmd.exe".into()
            } else {
                "/bin/bash".into()
            }
        });

        let can_sandbox = Sandbox::is_available(&self.m.config.sandbox).await;

        if can_sandbox {
            debug!("using sandbox isolation");
            self.launch_sandboxed_shell(&shell_bin, &session_id, &store_paths)
                .await?;
        } else if self.m.config.sandbox.fallback_allowed {
            warn!("sandbox unavailable — falling back to PATH-only isolation");
            self.spawn_fallback_shell(&shell_bin, &session_id, &store_paths)
                .await?;
        } else {
            return Err(Error::UnsupportedPlatform(
                "Sandbox policy violation: Isolation requested but unavailable.".into(),
            ));
        }

        info!("session ended — removing session packages");
        // The lock again, for the other half of the write. Taken *after* the session, because
        // the whole point of the split is that nobody waits on a person's shell — and released
        // by the end of this scope rather than by the end of `enter`.
        let _data_lock = crate::core::datalock::DataLock::for_one_step("shell teardown").await?;
        self.cleanup_transient_env(&session_id).await?;

        // Restore anything the user temporarily uninstalled for the duration of this
        // session (`remove --temp` with no duration inside a ephemeral shell).
        if let Err(e) = self.restore_session_suspensions(&session_id).await {
            warn!("session suspension restore failed: {}", e);
        }

        {
            let mut state_guard = self.m.state.lock().await;
            state_guard.active_session_id = None;
            // Don't drop this write silently (H2): if it fails, `active_session_id` stays
            // set on disk and the next run believes an ephemeral session is still live.
            if let Err(e) = state_guard.save() {
                warn!(
                    "could not persist session teardown ({}); the on-disk state \
                     still marks a session active, which the next `shall shell` will clear \
                     when it starts a new one.",
                    e
                );
            }
        }

        debug!("session packages removed");
        Ok(())
    }

    async fn launch_sandboxed_shell(
        &self,
        shell: &str,
        session_id: &str,
        store_paths: &[(String, String)],
    ) -> Result<()> {
        let mut mounts = Vec::new();
        let mut internal_path = String::from("/usr/local/bin:/usr/bin:/bin");

        for (path, name) in store_paths {
            let target = format!("/opt/shall/packages/{}", name);
            mounts.push((path.clone(), target.clone()));
            internal_path = format!("{}:{}/bin", internal_path, target);
        }

        let sandbox_cfg = SandboxConfig {
            allow_network: true,
            allow_home: true,
            allow_write: true,
            custom_mounts: mounts,
            ..Default::default()
        };

        let shell_owned = shell.to_string();
        let session_owned = session_id.to_string();
        let settings_clone = self.m.config.sandbox.clone();

        tokio::task::spawn_blocking(move || {
            let mut bwrap = Sandbox::wrap(&shell_owned, &[], &sandbox_cfg, &settings_clone)?;
            bwrap
                .env("PATH", internal_path)
                .env("SHALL_EPHEMERAL_SHELL", "1")
                .env("SHALL_SESSION_ID", session_owned);
            let mut handle = bwrap
                .spawn()
                .map_err(|e| Error::command_failed(format!("Sandbox error: {}", e)))?;
            let _ = handle
                .wait()
                .map_err(|e| Error::command_failed(e.to_string()))?;
            Ok::<(), Error>(())
        })
        .await
        .map_err(|e| Error::Other(format!("Task Join Panic: {}", e)))??;

        Ok(())
    }

    async fn spawn_fallback_shell(
        &self,
        shell: &str,
        session_id: &str,
        store_paths: &[(String, String)],
    ) -> Result<()> {
        let mut new_path_parts = Vec::new();

        for (path, _) in store_paths {
            new_path_parts.push(PathBuf::from(path.clone()));
            let bin_sub = Path::new(path).join("bin");
            if tokio::fs::try_exists(&bin_sub).await.unwrap_or(false) {
                new_path_parts.push(bin_sub);
            }
        }

        if let Ok(current) = env::var("PATH") {
            for p in env::split_paths(&current) {
                new_path_parts.push(p);
            }
        }

        let new_path_env = env::join_paths(new_path_parts)
            .map_err(|e| Error::Other(format!("PATH building failed: {}", e)))?;

        let mut child = tokio::process::Command::new(shell);
        child
            .env("PATH", new_path_env)
            .env("SHALL_EPHEMERAL_SHELL", "1")
            .env("SHALL_SESSION_ID", session_id)
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());

        // The terminal-handoff door: the person is *in* this shell, so it is inherited and
        // unbounded — but owned, because a shell left holding the terminal after Shall has gone
        // is a session nobody can account for.
        let _ = crate::core::supervise::supervised_status(child, "the ephemeral shell").await?;
        Ok(())
    }

    pub async fn locate_package_root(&self, spec: &PackageSpec) -> Result<Option<PathBuf>> {
        let backend = self
            .m
            .registry
            .get(&spec.backend)
            .ok_or_else(|| Error::BackendNotFound(spec.backend.clone()))?;

        if let Some(queryable) = backend.as_queryable() {
            if let Ok(Some(pkg)) = queryable.info(&spec.name).await {
                for key in ["local_path", "install_path", "path", "store_path"] {
                    if let Some(val) = pkg.properties.get(key) {
                        let p = PathBuf::from(val);
                        if tokio::fs::try_exists(&p).await.unwrap_or(false) {
                            return Ok(Some(p));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    /// Logic for provisioning the ephemeral state.
    /// FIXED: Release state lock before calling sync() to prevent deadlock.
    pub async fn provision_transient_env(
        &self,
        requests: &[String],
        _session_id: &str,
    ) -> Result<()> {
        let resolver = StateResolver::new(&self.m.config, self.m.registry.clone(), false).await;

        let mut transient_desired = HashMap::new();
        for req in requests {
            if let Ok(spec) = resolver.parse_and_probe_spec(req).await {
                transient_desired
                    .entry(spec.backend.clone())
                    .or_insert_with(Vec::new)
                    .push(spec);
            }
        }

        // Plan the changes while holding the state lock.
        //
        // `JustThese`, because `transient_desired` holds the shell's requests and nothing else.
        // Planned as a whole-machine converge it made every other managed package on the box a
        // removal — `shall shell ripgrep` proposing to uninstall the machine — with `max_removals`
        // the only thing in the way, and a ceiling is not a rule.
        let changes = {
            let state_guard = self.m.state.lock().await;
            let planner = ChangePlanner::new(self.m.registry.clone(), &state_guard, &self.m.config);
            planner
                .plan(&transient_desired, PlanScope::JustThese)
                .await?
        }; // <-- state_guard dropped here

        if !changes.is_empty() {
            let engine = self.create_sync_engine();
            engine
                .sync(changes, crate::app::sync::guard::GuardScope::Sync)
                .await?;
        }

        Ok(())
    }

    pub async fn cleanup_transient_env(&self, session_id: &str) -> Result<()> {
        let to_remove = {
            let state = self.m.state.lock().await;
            state.get_transient_packages(session_id)
        };

        if to_remove.is_empty() {
            return Ok(());
        }

        let mut graph = petgraph::stable_graph::StableDiGraph::new();
        for (backend, name) in to_remove {
            graph.add_node(crate::core::GraphAction::Remove { name, backend });
        }

        let changes = crate::app::sync::SyncChanges {
            graph,
            ..Default::default()
        };
        let engine = self.create_sync_engine();
        engine
            .sync(changes, crate::app::sync::guard::GuardScope::ShellExit)
            .await?;

        Ok(())
    }

    /// Reinstall packages the user temporarily uninstalled for the lifetime of this
    /// ephemeral shell session (`remove --temp` with no duration). Best-effort: a package the
    /// backend can no longer install is warned about and its suspension dropped, matching
    /// the timed-restore contract in `Leases::sweep_due_suspensions`.
    pub async fn restore_session_suspensions(&self, session_id: &str) -> Result<()> {
        let owned = {
            let state = self.m.state.lock().await;
            state.get_session_suspensions(session_id)
        };
        for s in owned {
            let restored = match self.m.registry.get(&s.backend) {
                Some(b) => match b.as_installable() {
                    Some(inst) => {
                        let spec = PackageSpec {
                            name: s.name.clone(),
                            backend: s.backend.clone(),
                            options: Default::default(),
                            requires: Vec::new(),
                            present: true,
                        };
                        // The other half of `packages.rs`'s suspend, and journalled for the
                        // same reason: a shell that exits into a killed reinstall leaves the
                        // package neither suspended nor back.
                        crate::core::journalled(
                            &self.m.journal,
                            vec![crate::core::JournalAction::Install(spec.clone())],
                            inst.install(std::slice::from_ref(&spec), b.sudo_for_write()),
                        )
                        .await
                    }
                    None => Err(Error::Other(format!(
                        "Backend '{}' cannot install",
                        s.backend
                    ))),
                },
                None => Err(Error::BackendNotFound(s.backend.clone())),
            };

            let mut state = self.m.state.lock().await;
            match restored {
                Ok(()) => {
                    info!("restored session-suspended {}:{}", s.backend, s.name);
                    state.add(
                        &s.backend,
                        &s.name,
                        s.version.clone(),
                        Default::default(),
                        "imperative",
                        false,
                    );
                }
                Err(e) => warn!(
                    "could not restore {}:{} ({}); dropping suspension.",
                    s.backend, s.name, e
                ),
            }
            state.clear_suspension(&s.backend, &s.name);
        }
        Ok(())
    }

    pub async fn auto_shell(&self) -> Result<()> {
        let local_config = Path::new("shall.txt");
        if tokio::fs::try_exists(local_config).await.unwrap_or(false) {
            debug!("using project-local shall.txt");
            let content = tokio::fs::read_to_string(local_config).await?;
            let pkgs = manifest_lines(&content);
            if !pkgs.is_empty() {
                self.enter(&pkgs).await?;
            }
        }
        Ok(())
    }

    fn create_sync_engine(&self) -> SyncEngine {
        SyncEngine::new(self.m.clone())
    }
}
