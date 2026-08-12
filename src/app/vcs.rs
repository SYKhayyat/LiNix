//! Git over the Shall repo — the manifests, never whatever repo the user is standing in.

use crate::config::Config;
use std::sync::Arc;
use tracing::{info, warn};

/// Vcs holds only what it uses. It is built from an [`App`](crate::app::App) by `App::vcs()`
/// and can be built without one.
pub struct Vcs<'a> {
    pub(crate) config: &'a Arc<Config>,
}

impl Vcs<'_> {
    /// A [`GitManager`](crate::core::GitManager) scoped to the Shall repo root (II.1), which
    /// holds `modules/`, `profiles/`, `active`, `priority` and `locks/`.
    ///
    /// Safety: `config_root()` never resolves to the current working directory — an empty or
    /// relative stored root falls back to the platform config dir — so git never operates on
    /// whatever repo the user happens to be standing in.
    pub fn manager(&self) -> crate::core::GitManager {
        crate::core::GitManager::new(self.config.config_root())
    }

    /// Auto-commit manifest/config changes IF the config dir is already a git repo. This is
    /// opt-in: users enable manifest version control by running `shall git init` once; until
    /// then this is a silent no-op. Never fails a command — a git hiccup is logged, not fatal.
    pub fn autocommit(&self, message: &str) {
        if self.config.dry_run {
            return;
        }
        let git = self.manager();
        if !git.is_repo() {
            return;
        }
        match git.commit_all(message) {
            Ok(Some(hash)) => info!(
                "Git: committed manifest change {} ({})",
                &hash[..hash.len().min(8)],
                message
            ),
            Ok(None) => {} // nothing changed
            Err(e) => warn!("Git: auto-commit skipped: {}", e),
        }
    }
}
