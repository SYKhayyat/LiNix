use crate::core::{Error, Result};
use tracing::{info, warn};

/// Repositories holds only what it uses. It is built from an [`App`](crate::app::App) by
/// `App::repositories()` and can be built without one.
pub struct Repositories<'a> {
    pub(crate) config: &'a std::sync::Arc<crate::config::Config>,
    pub(crate) registry: &'a std::sync::Arc<crate::backends::BackendRegistry>,
}

impl Repositories<'_> {
    /// Apply every `repo:` line, then refresh the indexes of the backends touched.
    ///
    /// **First in the ordering (II.7): repos → refresh → packages → dependents.** A package
    /// from a PPA is uninstallable until the PPA is added and `apt update` has seen it, so
    /// this must complete before the package plan runs — it is not a step the planner can
    /// interleave. Each repo names its backend (V.47), so there is no guessing which tool
    /// adds it. Idempotent: adding a repo that already exists is a no-op every backend
    /// tolerates, which is what lets a repo live in a file that syncs on every run.
    pub async fn apply(&self, state: &crate::model::DesiredState) -> Result<()> {
        use crate::config::grammar::Statement;

        let mut touched: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for (stmt, origin) in &state.extras {
            let Statement::Repo { backend, spec } = stmt else {
                continue;
            };
            let Some(b) = self.registry.get(backend) else {
                warn!(
                    "{}: backend `{}` is not available here — skipping repo `{}`.",
                    origin, backend, spec
                );
                continue;
            };
            let Some(repos) = b.as_repo_manager() else {
                return Err(Error::Config(format!(
                    "{}: `{}` cannot manage repositories, so `repo:{}:{}` has nowhere to go.",
                    origin, backend, backend, spec
                )));
            };
            if self.config.dry_run {
                info!("[DRY-RUN] would add repo `{}` to {}", spec, backend);
            } else {
                info!("Repo: adding `{}` to {} ({})", spec, backend, origin);
                repos.add_repo(spec, spec, b.sudo_for_write()).await?;
            }
            touched.insert(backend.clone());
        }

        // Refresh once per backend, after all its repos are in — an index refresh is the
        // slow part, and doing it per-repo would pay that cost N times for one backend.
        for backend in touched {
            if self.config.dry_run {
                info!("[DRY-RUN] would refresh {} package index", backend);
                continue;
            }
            if let Some(b) = self.registry.get(&backend) {
                if let Some(up) = b.as_upgradable() {
                    info!("Repo: refreshing {} package index", backend);
                    if let Err(e) = up.update(b.sudo_for_write()).await {
                        warn!("Repo: {} index refresh failed: {} — a package from a new repo may not be found yet.", backend, e);
                    }
                }
            }
        }
        Ok(())
    }
}
