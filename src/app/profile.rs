use crate::app::diagnostics::FailureDiagnosticEngine;
use crate::app::sync::{ChangePlanner, StateResolver, SyncEngine};
use crate::app::vocab::Vocab;
use crate::app::{LuaHooks, MetricsCollector};
use crate::backends::BackendRegistry;
use crate::config::grammar::Origin;
use crate::config::parser::HostFacts;
use crate::config::Config;
use crate::core::{CommandExecutor, Error, Journal, Result, SnapshotManager, StateRegistry};
use crate::model::profiles::{parse_active, ProfileLoader};
use crate::model::Layout;
use crate::utils::progress::ProgressReporter;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, instrument};

/// Turns profiles on and off (SPEC II.6).
///
/// **Only profiles can be activated**, and activating one edits exactly one file: `active`,
/// a plain list of profile names. Nothing is materialised, because a materialised copy is a
/// second place the same fact lives, and the day it disagrees with your files it wins
/// silently (P4). The resolver reads `active` on every run and composes from there.
pub struct ProfileManager {
    registry: Arc<BackendRegistry>,
    executor: CommandExecutor,
    metrics: MetricsCollector,
    progress: Arc<dyn ProgressReporter>,
    hooks: Arc<LuaHooks>,
    snapshot_manager: Arc<SnapshotManager>,
    journal: Arc<Mutex<Journal>>,
    state: Arc<Mutex<StateRegistry>>,
    config: Arc<Config>,
    diagnostics: Arc<FailureDiagnosticEngine>,
    layout: Layout,
}

impl ProfileManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        registry: Arc<BackendRegistry>,
        executor: CommandExecutor,
        metrics: MetricsCollector,
        progress: Arc<dyn ProgressReporter>,
        hooks: Arc<LuaHooks>,
        snapshot_manager: Arc<SnapshotManager>,
        journal: Arc<Mutex<Journal>>,
        state: Arc<Mutex<StateRegistry>>,
        config: Arc<Config>,
        diagnostics: Arc<FailureDiagnosticEngine>,
    ) -> Self {
        let layout = config.layout();
        Self {
            registry,
            executor,
            metrics,
            progress,
            hooks,
            snapshot_manager,
            journal,
            state,
            config,
            diagnostics,
            layout,
        }
    }

    async fn vocab(&self) -> Result<Vocab> {
        let priority = StateResolver::new(&self.config, self.registry.clone(), false)
            .await
            .priority_for_host()
            .await?;
        Ok(Vocab::new(&self.registry, &self.config, &priority))
    }

    /// Turn profiles on, then converge. Idempotent: activating an active profile changes
    /// nothing.
    #[instrument(skip(self))]
    pub async fn activate(&self, names: &[String]) -> Result<()> {
        let mut active = self.active_profiles().await?;
        for name in names {
            self.must_exist(name).await?;
            if !active.iter().any(|a| a == name) {
                active.push(name.clone());
            }
        }
        self.write_active(&active).await?;
        info!(
            "Active: {}. Converging...",
            if active.is_empty() {
                "nothing".to_string()
            } else {
                active.join(", ")
            }
        );
        self.sync_now().await
    }

    /// Turn profiles off, then converge — removing what no remaining active profile wants.
    #[instrument(skip(self))]
    pub async fn deactivate(&self, names: &[String]) -> Result<()> {
        let mut active = self.active_profiles().await?;
        let before = active.len();
        active.retain(|a| !names.iter().any(|n| n == a));
        if active.len() == before {
            info!("None of [{}] were active.", names.join(", "));
            return Ok(());
        }
        self.write_active(&active).await?;
        info!(
            "Deactivated [{}]. Still active: {}. Converging...",
            names.join(", "),
            if active.is_empty() {
                "nothing".to_string()
            } else {
                active.join(", ")
            }
        );
        self.sync_now().await
    }

    /// Swap the whole identity: this profile and nothing else.
    #[instrument(skip(self))]
    pub async fn switch(&self, name: &str) -> Result<()> {
        self.must_exist(name).await?;
        self.write_active(&[name.to_string()]).await?;
        info!("Switched to '{}'. Converging...", name);
        self.sync_now().await
    }

    pub async fn list_profiles(&self) -> Result<Vec<String>> {
        let vocab = self.vocab().await?;
        Ok(ProfileLoader::new(&self.layout, &vocab).available())
    }

    /// The currently-active profiles, in the order `active` lists them.
    pub async fn active_profiles(&self) -> Result<Vec<String>> {
        let file = self.layout.active_file();
        let body = tokio::fs::read_to_string(&file).await.unwrap_or_default();
        Ok(parse_active(&file, &body)?)
    }

    /// What a profile expands to, as `backend:name` lines.
    ///
    /// Resolved by the same code that resolves a sync, so what this prints and what a sync
    /// does cannot drift apart. It reports the profile as if it alone were active.
    pub async fn show(&self, name: &str) -> Result<Vec<String>> {
        self.must_exist(name).await?;

        let restore = self.active_profiles().await?;
        self.write_active(&[name.to_string()]).await?;
        let resolved = StateResolver::new(&self.config, self.registry.clone(), false)
            .await
            .resolve_desired_state()
            .await;
        // Whatever happened, put `active` back: a read-only command must not change what
        // this machine is set to.
        self.write_active(&restore).await?;

        let mut out: Vec<String> = resolved?
            .values()
            .flatten()
            .filter(|s| s.present)
            .map(|s| format!("{}:{}", s.backend, s.name))
            .collect();
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// Scaffold an empty profile.
    pub async fn create(&self, name: &str) -> Result<()> {
        self.check_name(name)?;
        let path = self.layout.profile_file(name);
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return Err(Error::Config(format!(
                "Profile '{}' already exists at {}. Delete it first, or edit it.",
                name,
                path.display()
            )));
        }
        tokio::fs::create_dir_all(self.layout.profiles_dir())
            .await
            .ok();
        tokio::fs::write(&path, PROFILE_TEMPLATE)
            .await
            .map_err(Error::from)?;
        info!("Created profile '{}' at {}.", name, path.display());
        Ok(())
    }

    /// Snapshot what this machine currently wants into a new profile.
    pub async fn save_current_as(&self, name: &str) -> Result<()> {
        self.check_name(name)?;
        let desired = StateResolver::new(&self.config, self.registry.clone(), false)
            .await
            .resolve_desired_state()
            .await?;

        let mut lines: Vec<String> = desired
            .values()
            .flatten()
            .filter(|s| s.present)
            .map(|s| format!("{}:{}", s.backend, s.name))
            .collect();
        lines.sort();
        lines.dedup();

        tokio::fs::create_dir_all(self.layout.profiles_dir())
            .await
            .ok();
        let path = self.layout.profile_file(name);
        let body = format!(
            "# Profile '{name}' — what this machine wanted when it was saved.\n\
             #\n\
             # These are package lines held directly by the profile, so no module can reach\n\
             # them. Move them into a module to share them between profiles.\n\n{}\n",
            lines.join("\n")
        );
        tokio::fs::write(&path, body).await.map_err(Error::from)?;
        info!(
            "Saved profile '{}' with {} package(s) to {}.",
            name,
            lines.len(),
            path.display()
        );
        Ok(())
    }

    /// II.5: profiles are Capitalized. A lowercase name would mint a module.
    fn check_name(&self, name: &str) -> Result<()> {
        if name.chars().next().is_some_and(char::is_uppercase) {
            return Ok(());
        }
        Err(Error::Config(format!(
            "`{}` is not a profile name — profiles are Capitalized, modules are lowercase.\n  \
             Did you mean `{}`? Only profiles can be activated.",
            name,
            capitalize(name)
        )))
    }

    async fn must_exist(&self, name: &str) -> Result<()> {
        self.check_name(name)?;
        let vocab = self.vocab().await?;
        let loader = ProfileLoader::new(&self.layout, &vocab);
        if loader.exists(name) {
            return Ok(());
        }
        // II.5's error teaches the rule rather than just saying no.
        let err = loader
            .resolve(name, &Origin::argument(), &HostFacts::current(), &mut Vec::new())
            .unwrap_err();
        Err(err.into())
    }

    /// `active` is a plain list of profile names and nothing else goes in it (II.6).
    async fn write_active(&self, active: &[String]) -> Result<()> {
        tokio::fs::create_dir_all(self.layout.config_root())
            .await
            .ok();
        let body = if active.is_empty() {
            String::new()
        } else {
            format!("{}\n", active.join("\n"))
        };
        tokio::fs::write(self.layout.active_file(), body)
            .await
            .map_err(Error::from)
    }

    /// Converge to whatever `active` now says.
    async fn sync_now(&self) -> Result<()> {
        let engine = SyncEngine::new(
            &self.config,
            self.registry.clone(),
            self.executor.duplicate(),
            self.metrics.clone(),
            self.progress.clone(),
            self.hooks.clone(),
            self.snapshot_manager.clone(),
            self.journal.clone(),
            self.state.clone(),
            self.diagnostics.clone(),
        )
        .await;

        let desired = StateResolver::new(&self.config, self.registry.clone(), false)
            .await
            .resolve_desired_state()
            .await?;

        let changes = {
            let state_guard = self.state.lock().await;
            let planner = ChangePlanner::new(self.registry.clone(), &state_guard, &self.config);
            planner.plan(&desired, None).await?
        };

        if changes.is_empty() {
            info!("This machine already matches the active profiles.");
            return Ok(());
        }

        engine
            .sync(changes, crate::app::sync::guard::GuardScope::Sync)
            .await?;
        Ok(())
    }
}

fn capitalize(name: &str) -> String {
    let mut c = name.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

const PROFILE_TEMPLATE: &str = "\
# A profile chooses; modules hold. Bring a module in with `use`:
#
#   use editors
#   use Work            (a profile — profiles are Capitalized, modules are lowercase)
#
# You can write packages here directly, but no module can reach them:
#
#   apt:curl
#   ripgrep             (no backend named — LiNix asks each one in `priority` order)
#
# Set math, if you need it:
#
#   exclude heavy       (drop that module's packages)
#   intersect approved  (keep only what is also in there)
#   -steam              (drop one package)
#   (Work | gaming) & security
#
# Gather, then narrow, then subtract — subtraction always wins.
";

#[cfg(test)]
mod tests {
    use super::capitalize;

    #[test]
    fn the_suggestion_capitalizes_the_name_you_typed() {
        assert_eq!(capitalize("work"), "Work");
        assert_eq!(capitalize(""), "");
    }
}
