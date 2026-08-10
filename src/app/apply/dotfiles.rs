use crate::config::grammar::{Options, Origin, Statement};
use crate::core::{Error, PackageSpec, Result};
use tracing::info;

/// Dotfiles holds only what it uses. It is built from an [`App`](crate::app::App) by
/// `App::dotfiles()` and can be built without one.
pub struct Dotfiles<'a> {
    pub(crate) config: &'a std::sync::Arc<crate::config::Config>,
    pub(crate) registry: &'a std::sync::Arc<crate::backends::BackendRegistry>,
}

/// One declared `dotfiles:` tree, resolved against this machine.
///
/// The origin travels with the plan because the `link:` lines the tree stands for are
/// synthesised, and a synthesised line that cannot name the file and line it came from is a
/// refusal nobody can act on.
pub struct Tree {
    pub name: String,
    pub origin: Origin,
    pub plan: crate::model::dotfiles::TreePlan,
}

impl Dotfiles<'_> {
    /// SEC3: the `link:` lines this run would place outside the home directory for the first
    /// time, as (line, destination) pairs.
    ///
    /// "First time" is asked of the destination, not of a ledger: `locks/extras.toml` keys a
    /// link by its *source*, so a line whose `@target` is edited to a system path is the same
    /// ledger entry it always was and would never be asked about. A destination that is not
    /// there yet is the run that creates it.
    ///
    /// Takes the statements rather than the state so that a `dotfiles:` tree can be expanded
    /// into them first. Reading `state.extras` directly, this asked about a hand-written
    /// `link:` into `/etc` and said nothing about a tree pointed at the same place — the
    /// question SEC3 exists to ask, skipped for the statement that asks it forty times at once.
    pub fn outside_home_links<'a>(
        statements: impl Iterator<Item = &'a (Statement, Origin)>,
        exists: &dyn Fn(&std::path::Path) -> bool,
    ) -> Vec<(String, std::path::PathBuf)> {
        statements
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
        let trees = self.links(state)?;
        let targets = Self::outside_home_links(state.extras.iter().chain(trees.iter()), &|p| {
            p.exists() || p.is_symlink()
        });
        if targets.is_empty() {
            return Ok(());
        }

        println!("\nThese lines place files outside your home directory:");
        for (line, dest) in &targets {
            println!("  {}  ->  {}", line, dest.display());
        }

        if self.config.dry_run {
            crate::would_print!("a real run would ask you to confirm these destinations.");
            return Ok(());
        }
        if self.config.yes {
            return Ok(());
        }

        let ok = crate::core::prompt::confirm(
            false,
            "Place these files?",
            crate::core::prompt::Unattended::Refuse(&format!(
                "refusing to place {} file(s) outside your home directory without \
                 confirmation in a non-interactive shell.\n\n\
                 What to do:\n  \
                 linix check         see every destination first\n  \
                 linix sync --yes    place them",
                targets.len()
            )),
        )?;
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
    pub fn plan(&self, state: &crate::model::DesiredState) -> Result<Vec<Tree>> {
        use crate::core::extras_lock::{ExtraKey, ExtrasLedger};
        use crate::core::LockFile;

        // What LiNix has recorded placing. Asked once for every tree, because it is the
        // *record* of ownership — `is_symlink` is only a guess at it, and the `link:` backend
        // already learned that guess is wrong in the one direction that matters: where the
        // deploy falls back to a copy (Windows, cross-drive), a file LiNix placed itself is
        // not a symlink. Under `is_symlink` alone the next sync called its own copy a
        // destination LiNix did not create and refused to touch the tree at all.
        let placed = ExtrasLedger::load(&ExtrasLedger::path_in(
            &self.config.config_root().join("locks"),
        ))?;
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
            // The union of the two proofs, never one: the ledger settles it for anything a
            // previous sync recorded, and a symlink still counts for a tree placed before the
            // row existed. Narrowing this to the ledger alone would turn every such
            // destination into a fresh U23 refusal on the sync after an upgrade.
            let owned = |p: &std::path::Path| {
                p.is_symlink() || placed.applied().contains(&ExtraKey::link(p).to_string())
            };
            let plan = crate::model::dotfiles::plan(
                &root,
                &target,
                &|p| p.exists() || p.is_symlink(),
                &owned,
            )
            .map_err(|e| Error::Io(format!("walking {}: {}", root.display(), e)))?;
            out.push(Tree {
                name: tree.to_string(),
                origin: origin.clone(),
                plan,
            });
        }

        // U25: several trees are fine; two claiming one destination is not, and the error
        // names both rather than letting whichever ran last win.
        let named: Vec<(String, crate::model::dotfiles::TreePlan)> = out
            .iter()
            .map(|t| (t.name.clone(), t.plan.clone()))
            .collect();
        let clashes = crate::model::dotfiles::conflicting_destinations(&named);
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
    /// The `link:` lines the declared trees stand for — one per file (U22).
    ///
    /// A `dotfiles:` line *"names a tree and stands for as many declarations as it holds"*, and
    /// this is that expansion. **One expansion, read by everything**: the placement installs
    /// these through the `link:` backend, and the extras ledger keys one row from each. Four
    /// documents — this module's own header, `core/extras_lock.rs`, `spec/history.md` and
    /// `spec/plan.md`'s 7n, marked DONE — said the ledger row existed. None did. The tree got
    /// its own placement loop instead, and so got none of what a hand-written `link:` has: no
    /// `<dest>.linix-backup` before it replaced a file the user wrote (T6), no ledger row, and
    /// therefore no teardown, no restore, and no removal guard. Deleting a file from a tree
    /// left a dangling symlink on the machine for ever, under a summary reading
    /// `already up to date`.
    ///
    /// Keyed per *file* and never per tree, because the question the ledger answers is "is this
    /// file still declared" — and a tree's contents change without its line changing.
    ///
    /// **A sync walks each tree four times** — the out-of-home confirmation, the resource plan,
    /// the placement, and the teardown's declared set — where it walked once before. Each walk
    /// is `read_dir` and no file contents, against a program that pays a subprocess per manager
    /// on the same path, so it is stated here rather than cached: an unstated cost is the thing
    /// worth avoiding, and a cache for an unmeasured one is worth less than the note.
    pub fn links(&self, state: &crate::model::DesiredState) -> Result<Vec<(Statement, Origin)>> {
        Ok(Self::links_of(&self.plan(state)?))
    }

    fn links_of(trees: &[Tree]) -> Vec<(Statement, Origin)> {
        trees
            .iter()
            .flat_map(|tree| {
                tree.plan.placements.iter().map(move |placement| {
                    let mut opts = Options::default();
                    opts.insert(
                        "target".to_string(),
                        placement.destination.display().to_string(),
                    );
                    (
                        Statement::Link(placement.source.display().to_string(), opts),
                        tree.origin.clone(),
                    )
                })
            })
            .collect()
    }

    /// The same expanded line in the shape `Installable::install` takes. Through
    /// `spec_from_extra`, so a tree's file and a hand-written `link:` reach the backend as the
    /// same value — a second converter here is how the two paths would drift apart again.
    fn spec_of(stmt: &Statement) -> Option<PackageSpec> {
        match stmt {
            Statement::Link(source, opts) => Some(crate::app::apply::dependents::spec_from_extra(
                "link", source, opts,
            )),
            _ => None,
        }
    }
    /// Mirror the declared trees into place (7n).
    ///
    /// **The collisions are shown before anything is written (U23).** A fresh machine's home
    /// directory is full of files a distribution put there, so the first sync of a new box asks
    /// this question forty times at once; answering it forty times individually is a refusal
    /// that teaches people to bypass refusals. So the whole list is printed, once, and the run
    /// stops — unless `--replace-existing` says they are all expected, which is the owner's
    /// bypass for the common case where every one of them is an untouched default.
    ///
    /// **The files go through the `link:` backend, not through a loop of this module's own.**
    /// `--replace-existing` waives the refusal to overwrite; it has never meant "and throw the
    /// original away", and a private `remove_file` here meant it did. The backend is where T6's
    /// `<dest>.linix-backup` lives, where the content-equality short-circuit lives (so a settled
    /// tree does no work and cannot end up backing up LiNix's own file), and where Windows'
    /// cross-drive copy fallback lives. The tree had none of the three.
    pub async fn apply(&self, state: &crate::model::DesiredState) -> Result<()> {
        let trees = self.plan(state)?;
        if trees.is_empty() {
            return Ok(());
        }

        let colliding: Vec<&std::path::PathBuf> = trees
            .iter()
            .flat_map(|t| t.plan.collisions.iter())
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

        for tree in &trees {
            if self.config.dry_run {
                // One line per tree, not one per file: the backend previews each placement it
                // would touch, and forty of those for a forty-file tree is the summary a user
                // came here to avoid writing forty `link:` lines for.
                crate::would!(
                    "would place {} file(s) from `dotfiles:{}`",
                    tree.plan.placements.len(),
                    tree.name
                );
                continue;
            }
            // A tree mirrors a directory structure, so its destinations' parents may not exist
            // yet. A hand-written `link:` names a path whose parent the user already has, which
            // is why this belongs to the tree and not to the backend.
            for placement in &tree.plan.placements {
                if let Some(parent) = placement.destination.parent() {
                    crate::utils::file::ensure_dir_async(parent).await?;
                }
            }
            // Both arms are the same refusal, because both mean the same thing to a user: the
            // tree cannot be placed. `register` always attaches a `LinkInstallable`, so the
            // second is unreachable today — and a silent `Ok` there would be a tree that
            // reported success having placed nothing, which is the failure mode this whole
            // change exists to remove.
            let cannot_place = || {
                Error::BackendNotFound(format!(
                    "the `link` backend cannot place files here, so `dotfiles:{}` cannot be \
                     applied — a tree is the `link:` lines it stands for, and there is nothing \
                     to place them with.",
                    tree.name
                ))
            };
            let backend = self.registry.get("link").ok_or_else(cannot_place)?;
            let inst = backend.as_installable().ok_or_else(cannot_place)?;
            let specs: Vec<PackageSpec> = Self::links_of(std::slice::from_ref(tree))
                .iter()
                .filter_map(|(stmt, _)| Self::spec_of(stmt))
                .collect();
            inst.install(&specs, backend.sudo_for_write()).await?;
            // "in place", not "placed": the backend skips a destination that already holds the
            // right bytes, so on a settled machine this line ran having written nothing. A
            // summary that reads back work it did not do is N-2, and the count of what was
            // actually written is not worth a content read per file to obtain.
            info!(
                "`dotfiles:{}` — {} file(s) in place.",
                tree.name,
                tree.plan.placements.len()
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(name: &str, target: &str) -> (Statement, Origin) {
        let mut opts = Options::default();
        opts.insert("target".to_string(), target);
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

        let asked = Dotfiles::outside_home_links(state.extras.iter(), &|_| false);
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].0, "link:cron/backup");
        assert_eq!(asked[0].1, std::path::PathBuf::from(system));

        // The destination is already there: it was agreed to on the run that placed it, and a
        // re-converge that asks again is a prompt on every sync.
        assert!(Dotfiles::outside_home_links(state.extras.iter(), &|_| true).is_empty());
    }

    /// SEC3 asks about a tree's files too, because a tree is the `link:` lines it stands for.
    ///
    /// It could not before: the question was asked of `state.extras`, and a `dotfiles:` tree is
    /// one statement there whose files are not. So `link:` into `/etc` asked and
    /// `dotfiles:./etc @target=/etc` — the same destinations, forty at a time — did not.
    #[test]
    fn a_trees_files_outside_home_are_asked_about_too() {
        #[cfg(windows)]
        let root = std::path::PathBuf::from(r"C:\ProgramData\linix");
        #[cfg(not(windows))]
        let root = std::path::PathBuf::from("/etc/linix");

        // What `links_of` produces for a tree: one synthesised `link:` per placed file.
        let expanded = Dotfiles::links_of(&[Tree {
            name: "./etc".to_string(),
            origin: Origin::new("modules/files.txt", 1),
            plan: crate::model::dotfiles::TreePlan {
                placements: vec![crate::model::dotfiles::Placement {
                    source: std::path::PathBuf::from("./etc/hosts"),
                    destination: root.join("hosts"),
                    relative: std::path::PathBuf::from("hosts"),
                }],
                collisions: Vec::new(),
            },
        }]);

        let asked = Dotfiles::outside_home_links(expanded.iter(), &|_| false);
        assert_eq!(
            asked.len(),
            1,
            "a tree file landing outside home was not asked about"
        );
        assert_eq!(asked[0].1, root.join("hosts"));

        // And the same rule as a hand-written line: an existing destination was agreed to on
        // the run that placed it, so it is not asked about again.
        assert!(Dotfiles::outside_home_links(expanded.iter(), &|_| true).is_empty());
    }
}
