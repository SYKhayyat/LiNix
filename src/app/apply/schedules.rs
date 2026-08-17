use crate::app::scheduler::Standing;
use crate::core::Result;
use tracing::info;

/// Schedules holds only what it uses. It is built from an [`App`](crate::app::App) by
/// `App::schedules()` and can be built without one.
pub struct Schedules<'a> {
    pub(crate) config: &'a std::sync::Arc<crate::config::Config>,
    pub(crate) executor: &'a crate::core::CommandExecutor,
    pub(crate) scheduler: &'a std::sync::Arc<crate::app::scheduler::SchedulerManager>,
}

impl Schedules<'_> {
    /// Provision the declared `schedule:` lines onto the OS scheduler (S21) — II.7's schedule
    /// phase, after packages and dependents. Each line is mapped to a `ScheduleConfig` (which
    /// validates it carries `cron` and `run`) and handed to the `SchedulerManager`. Declarative
    /// and idempotent: re-registering the same task each sync is how the system state is kept
    /// equal to what the `schedules` file says.
    pub async fn apply(&self, state: &crate::model::DesiredState) -> Result<()> {
        for (name, opts, origin) in state.schedules() {
            let cfg = crate::model::schedule::schedule_config(
                name,
                opts,
                origin,
                &self.config.guard.never_unattended,
            )?;
            // Asked before the preview and before the work, so `plan` and `sync` describe the
            // same machine. A schedule that already says what the line says is left alone: it
            // was always re-registered on every run, which converged and reported nothing, and
            // reporting nothing about a schedule being rewritten is the defect this closes.
            let because = match self.scheduler.standing(self.executor, &cfg).await {
                Standing::Matches => continue,
                Standing::Missing => "it is not registered",
                Standing::Differs(what) => what,
                // Named rather than silently re-provisioned as if it were missing: a scheduler
                // that could not be asked is a schedule nobody can claim is in force.
                Standing::Unknown => "Shall could not read it back from this scheduler",
            };
            if self.config.dry_run {
                crate::would!(
                    "would schedule `{}`: `{}` on `{}` — {}",
                    name,
                    cfg.command,
                    cfg.cron,
                    because
                );
                continue;
            }
            info!(
                "Schedule: provisioning `{}` ({}) — `{}` on `{}`, because {}",
                name, origin, cfg.command, cfg.cron, because
            );
            self.scheduler.provision(self.executor, &cfg).await?;
        }
        Ok(())
    }
}
