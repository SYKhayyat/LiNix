//! Writing a line into your files, and taking one back out.
//!
//! P1: an imperative command is a shortcut for editing a file and syncing. Every one of these
//! is that shortcut's write half, which is why they share a type: the file the line lands in,
//! the vocabulary it is parsed against and the tense of the sentence reporting it are one
//! decision made four times, and `App` was where all four of them lived next to twelve fields
//! none of them touched.

use crate::app::sync::resolver::StateResolver;
use crate::backends::BackendRegistry;
use crate::config::grammar::Origin;
use crate::config::Config;
use crate::core::{Error, Result};
use std::sync::Arc;
use tracing::info;

/// Declarations holds only what it uses. It is built from an [`App`](crate::app::App) by
/// `App::declarations()` and can be built without one.
pub struct Declarations<'a> {
    pub(crate) config: &'a Arc<Config>,
    pub(crate) registry: &'a Arc<BackendRegistry>,
    /// The run's parsed pin file and its remote-lookup cap, borrowed from the `App`.
    pub(crate) locks: &'a crate::app::machinery::SharedLocks,
    pub(crate) remote_gate: &'a Arc<tokio::sync::Semaphore>,
}

impl Declarations<'_> {
    /// One resolver per edit, shared by the three questions each edit asks it (is the line
    /// usable, what is the vocabulary, what are this host's facts). `App` built a fresh one
    /// for each of the three.
    async fn resolver(&self) -> StateResolver<'_> {
        let locks = self
            .locks
            .get_or_init(|| async { StateResolver::read_locks(self.config, false).await })
            .await
            .clone();
        StateResolver::with_shared(
            self.config,
            self.registry.clone(),
            false,
            locks,
            self.remote_gate.clone(),
        )
    }

    /// "Added" when the file changed, "Would add" when a preview only says it would.
    fn edit_verb(&self, done: &'static str, planned: &'static str) -> &'static str {
        if self.config.dry_run {
            planned
        } else {
            done
        }
    }

    /// Write a declaration into your files (P1: an imperative command is a shortcut for
    /// editing a file and syncing), and say which file it touched (II.8).
    ///
    /// `into` is II.8's `--into`: a module (lowercase) or a profile (Capitalized). Without
    /// it, the line lands in the module named for how it arrived (V.40).
    pub async fn declare(
        &self,
        line: &str,
        into: Option<&str>,
        landing: crate::model::Landing,
    ) -> Result<crate::model::Edit> {
        let resolver = self.resolver().await;
        resolver.validate_line(line).await?;
        let vocab = resolver.vocabulary().await?;
        let layout = self.config.layout();
        let target = match into {
            Some(name) => crate::model::Target::parse(name, &Origin::argument())?,
            None => landing.target(),
        };
        let edit = crate::model::Editor::new(
            &layout,
            &vocab,
            resolver.facts_for_host().await?,
            crate::model::Writes::for_run(self.config.dry_run),
        )
        .add(&target, line)
        .map_err(Error::from)?;
        info!("{}", edit.describe(self.edit_verb("Added", "Would add")));
        Ok(edit)
    }

    /// Whether any active file declares this package.
    ///
    /// Asked through the resolver, so "declared" means the same thing here as it does to
    /// `sync` — a second definition of declared is a second answer.
    pub async fn declares(&self, target: &str) -> Result<bool> {
        let resolver = self.resolver().await;
        let vocab = resolver.vocabulary().await?;
        let layout = self.config.layout();
        let facts = resolver.facts_for_host().await?;
        let files = crate::model::active_module_files(&layout, &vocab, &facts);
        // Reads only; a `Writes` it never uses is still the honest one to hand it.
        let editor =
            crate::model::Editor::new(&layout, &vocab, facts, crate::model::Writes::Planned);
        Ok(editor.declares_in(&files, target))
    }

    /// Move a declared package to `new_backend` by rewriting its line in place (II.8's
    /// `teleport`), and say which files changed. Empty result = the package is declared in no
    /// active file, which the caller reports rather than silently doing nothing.
    pub async fn retarget(
        &self,
        target_pkg: &str,
        new_backend: &str,
    ) -> Result<Vec<crate::model::Edit>> {
        let resolver = self.resolver().await;
        // The same write-then-discover fault as `declare`: a move to a manager `priority`
        // does not list rewrites the line into one nothing can parse, and the package it
        // came from is already gone from the file.
        resolver
            .validate_line(&format!("{}:{}", new_backend, target_pkg))
            .await?;
        let vocab = resolver.vocabulary().await?;
        let layout = self.config.layout();
        let facts = resolver.facts_for_host().await?;
        let files = crate::model::active_module_files(&layout, &vocab, &facts);
        let edits = crate::model::Editor::new(
            &layout,
            &vocab,
            facts,
            crate::model::Writes::for_run(self.config.dry_run),
        )
        .retarget_backend(&files, target_pkg, new_backend)
        .map_err(Error::from)?;
        for e in &edits {
            info!("{}", e.describe(self.edit_verb("Moved", "Would move")));
        }
        Ok(edits)
    }

    /// Remove a package's declaration from every file the active profiles reach (II.8's
    /// `uninstall`), and say which files changed.
    pub async fn undeclare(&self, target_pkg: &str) -> Result<Vec<crate::model::Edit>> {
        let resolver = self.resolver().await;
        let vocab = resolver.vocabulary().await?;
        let layout = self.config.layout();
        let facts = resolver.facts_for_host().await?;
        let files = crate::model::active_module_files(&layout, &vocab, &facts);
        let edits = crate::model::Editor::new(
            &layout,
            &vocab,
            facts,
            crate::model::Writes::for_run(self.config.dry_run),
        )
        .remove_from(&files, target_pkg)
        .map_err(Error::from)?;
        for e in &edits {
            info!("{}", e.describe(self.edit_verb("Removed", "Would remove")));
        }
        Ok(edits)
    }
}
