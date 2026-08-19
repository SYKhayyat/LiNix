//! `shall add` — vendoring someone else's modules into your repo (XIII.14, U14).
//!
//! `use` takes a name, never a URL (target-state): consuming a stranger's module is a **fetch
//! step** that puts files on disk, and then you `use` them by name like everything else. This
//! is that fetch step.
//!
//! **Vendoring, not importing.** The files land in your repo as a reviewable git diff — the one
//! thing between you and a stranger's code. It is a real defence and a weak one (nobody reads a
//! whole diff), so it is not the *only* defence: anything the vendored files can execute — an
//! `exec:` verb, an `adapters/*.toml` backend definition — arrives **unapproved**, and II.12's
//! ledger holds it until `shall lock`. Refuse-by-default, one deliberate act to permit, is the
//! pattern the guard's `--allow-mass-removal`, dotfiles' `--replace-existing` and `@allow_http`
//! all already follow. `--trust` vendors and locks in one step, for a source you already trust.
//!
//! Pure: classifying a source, keeping a fetched path inside the repo, and planning what lands
//! where (refusing a collision). The fetch and the copy are the caller's.

use std::path::{Path, PathBuf};

/// Where a module is being vendored from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// `github:owner/repo` — the shorthand, expanded to a clone URL.
    Github { owner: String, repo: String },
    /// Any git remote — `https://…`, `git@…`, `ssh://…`.
    Git(String),
    /// A single raw file over http(s).
    File(String),
    /// A module file or repo already on disk.
    Local(PathBuf),
}

impl Source {
    /// Classify what the user typed. The order is deliberate: `github:` is checked before the
    /// generic git test, and a `.git` URL or a git scheme before a plain file URL, so a git
    /// remote is never mistaken for a single file to download.
    pub fn classify(input: &str) -> Option<Source> {
        let s = input.trim();
        if s.is_empty() {
            return None;
        }

        if let Some(rest) = s.strip_prefix("github:") {
            let (owner, repo) = rest.split_once('/')?;
            let repo = repo.trim_end_matches(".git").trim_end_matches('/');
            if owner.is_empty() || repo.is_empty() || repo.contains('/') || repo.contains('\\') {
                return None;
            }
            return Some(Source::Github {
                owner: owner.to_string(),
                repo: repo.to_string(),
            });
        }

        let is_url = s.starts_with("http://") || s.starts_with("https://");
        if s.starts_with("git@") || s.starts_with("ssh://") || (is_url && s.ends_with(".git")) {
            return Some(Source::Git(s.to_string()));
        }
        if is_url {
            return Some(Source::File(s.to_string()));
        }

        // Anything else is a path on this machine. A relative path is left relative; the caller
        // resolves it against the working directory.
        Some(Source::Local(PathBuf::from(s)))
    }

    /// The git clone URL, for the two source kinds that clone.
    pub fn clone_url(&self) -> Option<String> {
        match self {
            Source::Github { owner, repo } => {
                Some(format!("https://github.com/{}/{}.git", owner, repo))
            }
            Source::Git(url) => Some(url.clone()),
            _ => None,
        }
    }

    /// A short label naming the source, for messages and for namespacing if it were ever wanted.
    pub fn label(&self) -> String {
        match self {
            Source::Github { owner, repo } => format!("github:{}/{}", owner, repo),
            Source::Git(url) => url.clone(),
            Source::File(url) => url.clone(),
            Source::Local(p) => p.display().to_string(),
        }
    }
}

/// The kinds of file `shall add` will copy out of a fetched source.
///
/// **A closed list, deliberately.** A shared repo is a config repo, and only some of it is
/// yours to take: the reusable lists (`modules/`) and the definitions they may lean on
/// (`adapters/`, and scripts an `exec:` points at). Not `profiles/`, `active` or `priority` —
/// those are the *other* machine's choices, and importing them would silently reconfigure
/// yours. Copying everything is how a vendor step turns into "run this repo".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendored {
    /// A `modules/*.txt` list.
    Module,
    /// An `adapters/*.toml` definition — code, so it lands unapproved.
    Adapter,
    /// A `scripts/*` file an `exec:` line may reference — also code, also unapproved.
    Script,
}

impl Vendored {
    /// Which category a source-relative path falls into, or `None` for a file `add` leaves
    /// behind (a README, a `profiles/` file, the other machine's `active`).
    pub fn of(rel: &Path) -> Option<Vendored> {
        let mut comps = rel.components();
        let top = comps.next()?.as_os_str().to_str()?;
        let ext = rel.extension().and_then(|e| e.to_str());
        match (top, ext) {
            ("modules", Some("txt")) => Some(Vendored::Module),
            ("adapters", Some("toml")) => Some(Vendored::Adapter),
            ("scripts", _) => Some(Vendored::Script),
            _ => None,
        }
    }

    /// Whether files of this kind arrive as executable code, held by II.12 until `shall lock`.
    pub fn is_code(self) -> bool {
        matches!(self, Vendored::Adapter | Vendored::Script)
    }
}

/// One file the vendor step would place, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub kind: Vendored,
    /// Path inside the fetched source.
    pub from: PathBuf,
    /// Path inside the user's repo.
    pub to: PathBuf,
}

/// The result of planning a vendor: what would land, and what already exists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VendorPlan {
    pub placements: Vec<Placement>,
    /// Destinations that already exist — the collision the user is refused on (U14: refuse and
    /// name it, `--force` to overwrite).
    pub collisions: Vec<PathBuf>,
}

/// Keep a source-relative path inside the repo it is being copied into.
///
/// A fetched tree is a stranger's, and a file named `../../.bashrc` or `/etc/passwd` in it
/// would, joined naively, write outside the config repo entirely — the S-class path-traversal
/// this rejects. A path with a `..` component, an absolute path, or a Windows prefix is
/// refused; everything else is returned as the safe relative path.
pub fn safe_relative(rel: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut clean = PathBuf::new();
    for comp in rel.components() {
        match comp {
            // A component the *running* platform calls ordinary may still be a separator on the
            // other one: Unix reads `..\..\x` as a single filename, Windows as a climb out. A
            // vendored tree is a stranger's, so the answer must not depend on which machine
            // fetched it — see `not_a_bare_file_name`, where the same asymmetry was live.
            Component::Normal(c) => match c.to_str() {
                Some(s) if s.contains('/') || s.contains('\\') => return None,
                _ => clean.push(c),
            },
            // A leading `./` is harmless noise; drop it.
            Component::CurDir => {}
            // Everything else — `..`, a root, a drive prefix — is an escape attempt.
            _ => return None,
        }
    }
    if clean.as_os_str().is_empty() {
        return None;
    }
    Some(clean)
}

/// Plan the vendor: for every file in `source_files` (paths relative to the fetched root),
/// decide what lands where, and flag the ones whose destination already exists.
///
/// `exists` answers whether a repo-relative destination is already present — injected so the
/// planner stays pure and testable without a repo on disk.
pub fn plan(source_files: &[PathBuf], exists: &dyn Fn(&Path) -> bool) -> VendorPlan {
    let mut plan = VendorPlan::default();
    for rel in source_files {
        let Some(kind) = Vendored::of(rel) else {
            continue;
        };
        let Some(safe) = safe_relative(rel) else {
            continue;
        };
        // The destination mirrors the source layout: a `modules/x.txt` lands at `modules/x.txt`.
        let to = safe.clone();
        if exists(&to) {
            plan.collisions.push(to.clone());
        }
        plan.placements.push(Placement {
            kind,
            from: rel.clone(),
            to,
        });
    }
    plan.placements.sort_by(|a, b| a.to.cmp(&b.to));
    plan.collisions.sort();
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_shorthand_classifies_and_expands() {
        let s = Source::classify("github:BurntSushi/ripgrep").unwrap();
        assert_eq!(
            s,
            Source::Github {
                owner: "BurntSushi".into(),
                repo: "ripgrep".into()
            }
        );
        assert_eq!(
            s.clone_url().unwrap(),
            "https://github.com/BurntSushi/ripgrep.git"
        );
        // A trailing `.git` or slash is tolerated.
        assert_eq!(
            Source::classify("github:a/b.git"),
            Source::classify("github:a/b")
        );
    }

    #[test]
    fn a_malformed_github_ref_is_not_classified() {
        assert_eq!(Source::classify("github:noshslash"), None);
        assert_eq!(Source::classify("github:a/b/c"), None);
        assert_eq!(Source::classify("github:/b"), None);
    }

    /// A git remote must not be read as a single file to download — it clones.
    #[test]
    fn git_urls_clone_and_file_urls_do_not() {
        assert!(matches!(
            Source::classify("https://gitlab.com/x/y.git"),
            Some(Source::Git(_))
        ));
        assert!(matches!(
            Source::classify("git@github.com:x/y.git"),
            Some(Source::Git(_))
        ));
        assert!(matches!(
            Source::classify("https://example.com/mod.txt"),
            Some(Source::File(_))
        ));
    }

    #[test]
    fn a_bare_path_is_local() {
        assert!(matches!(
            Source::classify("./shared/modules"),
            Some(Source::Local(_))
        ));
        assert!(matches!(
            Source::classify("/home/a/repo"),
            Some(Source::Local(_))
        ));
        assert_eq!(Source::classify("   "), None);
    }

    /// Only some of a shared repo is yours to take. `profiles/`, `active` and `priority` are
    /// the other machine's choices — vendoring them would silently reconfigure yours.
    #[test]
    fn only_shareable_files_are_vendored() {
        assert_eq!(
            Vendored::of(Path::new("modules/tools.txt")),
            Some(Vendored::Module)
        );
        assert_eq!(
            Vendored::of(Path::new("adapters/backends.toml")),
            Some(Vendored::Adapter)
        );
        assert_eq!(
            Vendored::of(Path::new("scripts/setup.sh")),
            Some(Vendored::Script)
        );
        for skip in [
            "profiles/Work",
            "active",
            "priority",
            "README.md",
            "modules/notes.md",
        ] {
            assert_eq!(
                Vendored::of(Path::new(skip)),
                None,
                "{} should not vendor",
                skip
            );
        }
    }

    #[test]
    fn adapters_and_scripts_are_code_modules_are_not() {
        assert!(Vendored::Adapter.is_code());
        assert!(Vendored::Script.is_code());
        assert!(!Vendored::Module.is_code());
    }

    /// The path-traversal defence: a stranger's file named to escape the repo is refused.
    #[test]
    fn a_path_that_escapes_the_repo_is_refused() {
        assert_eq!(safe_relative(Path::new("../../.bashrc")), None);
        assert_eq!(
            safe_relative(Path::new("modules/../../../etc/passwd")),
            None
        );
        assert_eq!(safe_relative(Path::new("/etc/passwd")), None);
        // **The same answer on both platforms.** `components()` reads a backslash as a
        // separator on Windows and as an ordinary filename character on Unix, so this pair
        // was refused on one and accepted on the other — from a tree a stranger supplied. It
        // is the same asymmetry that let `..\\..\\x` through `@bin=` on Linux and macOS
        // while the Windows gate passed, which is how it reached CI unseen.
        assert_eq!(safe_relative(Path::new(r"..\..\.bashrc")), None);
        assert_eq!(safe_relative(Path::new(r"modules\..\x")), None);
        // A clean relative path survives, with a leading `./` stripped.
        assert_eq!(
            safe_relative(Path::new("./modules/x.txt")),
            Some(PathBuf::from("modules/x.txt"))
        );
    }

    #[test]
    fn the_plan_places_shareable_files_and_skips_the_rest() {
        let files = vec![
            PathBuf::from("modules/tools.txt"),
            PathBuf::from("adapters/backends.toml"),
            PathBuf::from("profiles/Work"),
            PathBuf::from("README.md"),
        ];
        let plan = plan(&files, &|_| false);
        assert_eq!(plan.placements.len(), 2, "{:?}", plan.placements);
        assert!(plan.collisions.is_empty());
        assert_eq!(
            plan.placements[0].to,
            PathBuf::from("adapters/backends.toml")
        );
        assert_eq!(plan.placements[1].to, PathBuf::from("modules/tools.txt"));
    }

    /// U14: a destination that already exists is a collision — named, not silently overwritten.
    #[test]
    fn an_existing_destination_is_flagged_as_a_collision() {
        let files = vec![PathBuf::from("modules/tools.txt")];
        let mine = |p: &Path| p == Path::new("modules/tools.txt");
        let plan = plan(&files, &mine);
        assert_eq!(plan.collisions, vec![PathBuf::from("modules/tools.txt")]);
        // The placement is still listed — `--force` acts on exactly these.
        assert_eq!(plan.placements.len(), 1);
    }

    /// A malicious traversal path never even reaches the placement list.
    #[test]
    fn a_traversal_path_is_dropped_from_the_plan() {
        let files = vec![PathBuf::from("modules/../../../etc/cron.d/evil")];
        assert!(plan(&files, &|_| false).placements.is_empty());
    }
}
