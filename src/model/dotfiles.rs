//! `dotfiles:./tree` — a folder of files, linked where they belong (XIII.21, U22–U25).
//!
//! A dotfiles repo is already a tree that mirrors `$HOME`. Writing forty `link:` lines to say
//! so is forty chances to forget one. This statement says it once.
//!
//! **Per file, never per directory (U22).** One symlink at `~/.config/nvim` takes the whole
//! directory hostage: every cache, session file and plugin lockfile the application later
//! writes lands inside the git-tracked config repo, and `bundle` then hands it to whoever the
//! backup goes to. Linking each file leaves the directory the user's and puts nothing in the
//! repo that was not put there deliberately. The cost is walking the tree each sync and one
//! ledger row per file, and that is the cost worth paying.
//!
//! The rows are written by `Dotfiles::links`, which expands a tree into the `link:` lines it
//! stands for — that expansion is what makes the teardown, the `<dest>.linix-backup` and the
//! removal guard the tree's as much as a hand-written line's.
//!
//! **The tree never decrypts (U24).** A `.age` file in it is copied as the ciphertext it is.
//! Deciding by file extension is magic, and magic that silently writes plaintext; secrets stay
//! on explicit `link:` lines where `@decrypt=` is written down.
//!
//! This module is pure: it walks a tree and computes destinations and collisions. Nothing here
//! touches a destination — that is the caller's, after the plan has been shown.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// One file the tree would place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    /// The file inside the tree.
    pub source: PathBuf,
    /// Where it belongs, under the target root.
    pub destination: PathBuf,
    /// Its path relative to the tree root — what makes the mapping legible in a plan.
    pub relative: PathBuf,
}

/// What a tree would do, and what stands in its way.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreePlan {
    /// Every file, in a stable order so a plan reads the same twice.
    pub placements: Vec<Placement>,
    /// Destinations that already hold something LiNix did not put there (U23). The run is
    /// refused until these have been seen.
    pub collisions: Vec<PathBuf>,
}

impl TreePlan {
    pub fn is_empty(&self) -> bool {
        self.placements.is_empty()
    }
}

/// Which files a tree skips. Not configurable, and deliberately short: a dotfiles tree is a
/// tree of dotfiles, and the things below are never one of them.
///
/// `.git` is the big one — mirroring it into `$HOME` would put a second repository there.
fn is_skipped(name: &str) -> bool {
    matches!(name, ".git" | ".gitignore" | ".gitattributes" | ".DS_Store")
}

/// Walk `tree`, mapping every file to its place under `target_root`.
///
/// `owned` answers "did LiNix put this here?" for a destination that already exists — injected
/// rather than read here so this stays pure and the ownership rule lives in one place.
pub fn plan(
    tree: &Path,
    target_root: &Path,
    exists: &dyn Fn(&Path) -> bool,
    owned: &dyn Fn(&Path) -> bool,
) -> std::io::Result<TreePlan> {
    let mut out = TreePlan::default();
    let mut files = Vec::new();
    walk(tree, tree, &mut files)?;
    // Sorted so the plan — and therefore the confirmation a user reads — is stable.
    files.sort();

    for relative in files {
        let source = tree.join(&relative);
        let destination = target_root.join(&relative);
        if exists(&destination) && !owned(&destination) {
            out.collisions.push(destination.clone());
        }
        out.placements.push(Placement {
            source,
            destination,
            relative,
        });
    }
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if is_skipped(&name) {
            continue;
        }
        let path = entry.path();
        // `symlink_metadata`: a symlink inside the tree is a file to place, not a directory to
        // descend into — following it could walk out of the tree entirely.
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.is_dir() {
            walk(root, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_path_buf());
        }
    }
    Ok(())
}

/// Two trees that would place a file at the same destination (U25).
///
/// An error naming both, rather than a rule forbidding a second tree: the statement takes a
/// path, so several are natural, and the thing that actually goes wrong is two of them
/// claiming one destination. That is II.7 rule 5 reached by a new road, not a new rule.
pub fn conflicting_destinations(plans: &[(String, TreePlan)]) -> BTreeMap<PathBuf, Vec<String>> {
    let mut by_destination: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    for (tree, plan) in plans {
        for p in &plan.placements {
            by_destination
                .entry(p.destination.clone())
                .or_default()
                .push(tree.clone());
        }
    }
    by_destination.retain(|_, trees| {
        trees.sort();
        trees.dedup();
        trees.len() > 1
    });
    by_destination
}

/// The destinations a set of plans would own, for the teardown ledger.
pub fn destinations(plan: &TreePlan) -> BTreeSet<PathBuf> {
    plan.placements
        .iter()
        .map(|p| p.destination.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree_with(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for f in files {
            let path = dir.path().join(f);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "x").unwrap();
        }
        dir
    }

    fn nothing_exists(_: &Path) -> bool {
        false
    }
    fn nothing_owned(_: &Path) -> bool {
        false
    }

    /// U22: files, not directories. A tree containing `.config/nvim/init.lua` places that one
    /// file — it does not place `.config/nvim`, which would take the directory hostage.
    #[test]
    fn the_tree_places_files_not_directories() {
        let tree = tree_with(&[".config/nvim/init.lua", ".bashrc"]);
        let home = Path::new("/home/me");
        let p = plan(tree.path(), home, &nothing_exists, &nothing_owned).unwrap();

        let rels: Vec<String> = p
            .placements
            .iter()
            .map(|x| x.relative.to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(rels, vec![".bashrc", ".config/nvim/init.lua"]);
        // The directory itself is never a placement.
        assert!(!rels.iter().any(|r| r == ".config/nvim"));
    }

    #[test]
    fn destinations_mirror_the_tree_under_the_target_root() {
        let tree = tree_with(&[".config/nvim/init.lua"]);
        let home = Path::new("/home/me");
        let p = plan(tree.path(), home, &nothing_exists, &nothing_owned).unwrap();
        assert_eq!(
            p.placements[0].destination,
            home.join(".config").join("nvim").join("init.lua")
        );
    }

    /// The repository's own metadata is not a dotfile. Mirroring `.git` into `$HOME` would put
    /// a second repository there.
    #[test]
    fn the_trees_own_git_metadata_is_never_placed() {
        let tree = tree_with(&[".git/config", ".gitignore", ".vimrc"]);
        let p = plan(
            tree.path(),
            Path::new("/home/me"),
            &nothing_exists,
            &nothing_owned,
        )
        .unwrap();
        let rels: Vec<String> = p
            .placements
            .iter()
            .map(|x| x.relative.to_string_lossy().into())
            .collect();
        assert_eq!(rels, vec![".vimrc"]);
    }

    /// U23: a destination holding the user's own file is a collision, reported before anything
    /// is written. One LiNix already owns is not.
    #[test]
    fn a_destination_the_user_owns_is_a_collision_and_ours_is_not() {
        let tree = tree_with(&["a.conf", "b.conf"]);
        let home = Path::new("/home/me");
        let exists = |_: &Path| true;
        let owned = |p: &Path| p.ends_with("b.conf");

        let plan = plan(tree.path(), home, &exists, &owned).unwrap();
        assert_eq!(plan.collisions.len(), 1);
        assert!(plan.collisions[0].ends_with("a.conf"));
        // Both are still placed — a collision is a thing to confirm, not a file to drop.
        assert_eq!(plan.placements.len(), 2);
    }

    #[test]
    fn a_fresh_machine_has_no_collisions() {
        let tree = tree_with(&["a.conf"]);
        let p = plan(
            tree.path(),
            Path::new("/home/me"),
            &nothing_exists,
            &nothing_owned,
        )
        .unwrap();
        assert!(p.collisions.is_empty());
    }

    /// U25: several trees are fine; two claiming one destination is an error naming both.
    #[test]
    fn two_trees_claiming_one_destination_are_named_together() {
        let work = tree_with(&[".gitconfig", "work-only"]);
        let home_tree = tree_with(&[".gitconfig", "home-only"]);
        let home = Path::new("/home/me");
        let a = plan(work.path(), home, &nothing_exists, &nothing_owned).unwrap();
        let b = plan(home_tree.path(), home, &nothing_exists, &nothing_owned).unwrap();

        let clashes = conflicting_destinations(&[("./work".into(), a), ("./home".into(), b)]);
        assert_eq!(clashes.len(), 1, "{:?}", clashes);
        let (dest, trees) = clashes.iter().next().unwrap();
        assert!(dest.ends_with(".gitconfig"));
        assert_eq!(trees, &vec!["./home".to_string(), "./work".to_string()]);
    }

    #[test]
    fn trees_that_do_not_overlap_are_not_a_conflict() {
        let a = tree_with(&["one"]);
        let b = tree_with(&["two"]);
        let home = Path::new("/home/me");
        let pa = plan(a.path(), home, &nothing_exists, &nothing_owned).unwrap();
        let pb = plan(b.path(), home, &nothing_exists, &nothing_owned).unwrap();
        assert!(conflicting_destinations(&[("a".into(), pa), ("b".into(), pb)]).is_empty());
    }

    /// U24: an encrypted file is placed as the ciphertext it is. The tree has no per-file
    /// options by construction, so deciding to decrypt could only be done by extension — magic
    /// that silently writes plaintext.
    #[test]
    fn an_encrypted_file_is_placed_as_ciphertext_like_any_other() {
        let tree = tree_with(&["secrets/token.age"]);
        let p = plan(
            tree.path(),
            Path::new("/home/me"),
            &nothing_exists,
            &nothing_owned,
        )
        .unwrap();
        assert_eq!(p.placements.len(), 1);
        assert!(p.placements[0].destination.ends_with("token.age"));
    }

    /// The plan is read by a human before anything is written, so it must not reorder itself
    /// between runs.
    #[test]
    fn the_plan_is_in_a_stable_order() {
        let tree = tree_with(&["z", "a", "m/b", "m/a"]);
        let home = Path::new("/home/me");
        let first = plan(tree.path(), home, &nothing_exists, &nothing_owned).unwrap();
        let again = plan(tree.path(), home, &nothing_exists, &nothing_owned).unwrap();
        assert_eq!(first, again);
        let rels: Vec<String> = first
            .placements
            .iter()
            .map(|x| x.relative.to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(rels, vec!["a", "m/a", "m/b", "z"]);
    }
}
