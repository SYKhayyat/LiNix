use crate::core::{Error, Result};
use tracing::info;

/// Dotfiles holds only what it uses. It is built from an [`App`](crate::app::App) by
/// `App::dotfiles()` and can be built without one.
pub struct Dotfiles<'a> {
    pub(crate) config: &'a std::sync::Arc<crate::config::Config>,
    pub(crate) executor: &'a crate::core::CommandExecutor,
}

impl Dotfiles<'_> {
    /// SEC3: the `link:` lines this run would place outside the home directory for the first
    /// time, as (line, destination) pairs.
    ///
    /// "First time" is asked of the destination, not of a ledger: `locks/extras.toml` keys a
    /// link by its *source*, so a line whose `@target` is edited to a system path is the same
    /// ledger entry it always was and would never be asked about. A destination that is not
    /// there yet is the run that creates it.
    pub fn outside_home_links(
        state: &crate::model::DesiredState,
        exists: &dyn Fn(&std::path::Path) -> bool,
    ) -> Vec<(String, std::path::PathBuf)> {
        use crate::config::grammar::Statement;

        state
            .extras
            .iter()
            .filter_map(|(stmt, _)| {
                let Statement::Link(name, opts) = stmt else {
                    return None;
                };
                // An unresolvable target is the install path's error to report, with its own
                // message; swallowing it here would turn it into a silent skip.
                let resolved = crate::backends::link::resolve_target(opts.one("target")?).ok()?;
                (crate::backends::link::is_outside_home(&resolved) && !exists(&resolved))
                    .then_some((format!("link:{}", name), resolved))
            })
            .collect()
    }
    /// Ask about those destinations before anything is applied.
    ///
    /// SEC3: `@target` is deliberately unconfined — an arbitrary destination is the link
    /// backend's purpose — so this asks rather than refuses, and no config key turns it off.
    /// What it buys is a beat between a pasted spec line and a system path.
    pub fn confirm_outside_home(&self, state: &crate::model::DesiredState) -> Result<()> {
        let targets = Self::outside_home_links(state, &|p| p.exists() || p.is_symlink());
        if targets.is_empty() {
            return Ok(());
        }

        println!("\nThese lines place files outside your home directory:");
        for (line, dest) in &targets {
            println!("  {}  ->  {}", line, dest.display());
        }

        if self.config.dry_run {
            println!("[DRY-RUN] a real run would ask you to confirm these destinations.");
            return Ok(());
        }
        if self.config.yes {
            return Ok(());
        }

        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            return Err(Error::Refused(format!(
                "refusing to place {} file(s) outside your home directory without \
                 confirmation in a non-interactive shell.\n\n\
                 What to do:\n  \
                 linix status        see every destination first\n  \
                 linix sync --yes    place them",
                targets.len()
            )));
        }

        let ok = dialoguer::Confirm::new()
            .with_prompt("Place these files?")
            .default(false)
            .interact()
            .map_err(|e| Error::Other(format!("could not ask for confirmation: {}", e)))?;
        if ok {
            Ok(())
        } else {
            Err(Error::Other("cancelled — nothing was changed.".to_string()))
        }
    }
    /// Plan every declared `dotfiles:` tree, resolved against this machine (7n).
    ///
    /// Returns each tree with what it would place, so one walk answers the preview, the
    /// collision check and the apply. Walking three times would be three chances to disagree.
    pub fn plan(
        &self,
        state: &crate::model::DesiredState,
    ) -> Result<Vec<(String, crate::model::dotfiles::TreePlan)>> {
        let mut out = Vec::new();
        for (tree, opts, origin) in state.dotfile_trees() {
            let declared = std::path::Path::new(tree);
            let root = if declared.is_absolute() {
                declared.to_path_buf()
            } else {
                self.config.config_root().join(declared)
            };
            if !root.is_dir() {
                return Err(Error::Validation(format!(
                    "{}: `dotfiles:{}` is not a directory ({}). It names a folder to mirror, \
                     not a file — a single file is a `link:`.",
                    origin,
                    tree,
                    root.display()
                )));
            }
            let target = match opts.one("target") {
                Some(t) => crate::backends::link::resolve_target(t)?,
                None => dirs::home_dir()
                    .ok_or_else(|| Error::Other("could not find the home directory".into()))?,
            };
            let owned = |p: &std::path::Path| p.is_symlink();
            let plan = crate::model::dotfiles::plan(
                &root,
                &target,
                &|p| p.exists() || p.is_symlink(),
                &owned,
            )
            .map_err(|e| Error::Io(format!("walking {}: {}", root.display(), e)))?;
            out.push((tree.to_string(), plan));
        }

        // U25: several trees are fine; two claiming one destination is not, and the error
        // names both rather than letting whichever ran last win.
        let clashes = crate::model::dotfiles::conflicting_destinations(&out);
        if let Some((dest, trees)) = clashes.iter().next() {
            return Err(Error::Validation(format!(
                "two dotfiles trees both place {}: {}. One destination has one source — \
                 remove it from one tree, or narrow one with a `when`.",
                dest.display(),
                trees.join(" and ")
            )));
        }
        Ok(out)
    }
    /// Mirror the declared trees into place (7n).
    ///
    /// **The collisions are shown before anything is written (U23).** A fresh machine's home
    /// directory is full of files a distribution put there, so the first sync of a new box asks
    /// this question forty times at once; answering it forty times individually is a refusal
    /// that teaches people to bypass refusals. So the whole list is printed, once, and the run
    /// stops — unless `--replace-existing` says they are all expected, which is the owner's
    /// bypass for the common case where every one of them is an untouched default.
    pub async fn apply(&self, state: &crate::model::DesiredState) -> Result<()> {
        let plans = self.plan(state)?;
        if plans.is_empty() {
            return Ok(());
        }

        let colliding: Vec<&std::path::PathBuf> = plans
            .iter()
            .flat_map(|(_, p)| p.collisions.iter())
            .collect();
        if !colliding.is_empty() && !self.config.replace_existing {
            let mut msg = format!(
                "{} destination(s) already hold a file LiNix did not put there:\n",
                colliding.len()
            );
            for dest in &colliding {
                msg.push_str(&format!("    {}\n", dest.display()));
            }
            msg.push_str(
                "\nNothing has been written. Move or delete them, or re-run with \
                 `--replace-existing` if they are defaults you do not need.",
            );
            return Err(Error::Refused(msg));
        }

        for (tree, plan) in &plans {
            if self.config.dry_run {
                info!(
                    "[DRY-RUN] would place {} file(s) from `dotfiles:{}`",
                    plan.placements.len(),
                    tree
                );
                continue;
            }
            for placement in &plan.placements {
                if let Some(parent) = placement.destination.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .map_err(Error::from)?;
                }
                // An existing destination is replaced only once the collision check above has
                // passed or been waived, so this cannot silently overwrite anything.
                if placement.destination.exists() || placement.destination.is_symlink() {
                    tokio::fs::remove_file(&placement.destination)
                        .await
                        .map_err(Error::from)?;
                }
                self.executor
                    .symlink(&placement.source, &placement.destination)
                    .await?;
            }
            info!(
                "`dotfiles:{}` placed {} file(s).",
                tree,
                plan.placements.len()
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::grammar::Origin;
    use crate::config::grammar::{Options, Statement};

    fn link(name: &str, target: &str) -> (Statement, Origin) {
        let mut opts = Options::default();
        opts.insert("target", target);
        (
            Statement::Link(name.to_string(), opts),
            Origin::new("modules/files.txt", 1),
        )
    }

    fn state_with(extras: Vec<(Statement, Origin)>) -> crate::model::DesiredState {
        crate::model::DesiredState {
            extras,
            ..Default::default()
        }
    }

    #[test]
    fn only_a_new_link_outside_home_is_asked_about() {
        #[cfg(windows)]
        let system = r"C:\ProgramData\linix\hosts";
        #[cfg(not(windows))]
        let system = "/etc/cron.d/backup";

        let state = state_with(vec![
            link("dotfiles/gitconfig", "~/.gitconfig"),
            link("cron/backup", system),
        ]);

        let asked = Dotfiles::outside_home_links(&state, &|_| false);
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].0, "link:cron/backup");
        assert_eq!(asked[0].1, std::path::PathBuf::from(system));

        // The destination is already there: it was agreed to on the run that placed it, and a
        // re-converge that asks again is a prompt on every sync.
        assert!(Dotfiles::outside_home_links(&state, &|_| true).is_empty());
    }
}
