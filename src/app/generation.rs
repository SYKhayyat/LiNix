// src/app/generation.rs
//
// Generations: an append-only history of realized system states. After each change LiNix
// records a generation that captures BOTH what was actually installed (the resolved
// package set, with exact versions) AND a frozen copy of the manifest files that produced
// it. Rolling back to a generation restores that realized state and its manifests — the
// same idea as a Nix generation (a saved *result*), plus the small source that expressed
// it, which Nix does not keep.
//
// This store is self-contained: each generation is one JSON file holding its own manifest
// copies, so retention of generations is fully independent of the (separate) manifest
// archive and of filesystem snapshots.

use crate::app::sync::planner::SyncChanges;
use crate::core::retention::{RetentionItem, RetentionPolicy};
use crate::core::state::ManagedPackage;
use crate::core::{Error, GraphAction, PackageSpec, Result, StateRegistry};
use chrono::{DateTime, Utc};
use petgraph::stable_graph::StableDiGraph;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::warn;

/// Every `*.txt` manifest, keyed by *full path* -> body.
///
/// Keyed by full path, not filename, so a generation records where each file came from and
/// `rollback` can put it back there.
pub async fn read_manifests(groups_dir: &Path) -> Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    if tokio::fs::try_exists(groups_dir).await.unwrap_or(false) {
        let mut entries = tokio::fs::read_dir(groups_dir).await.map_err(Error::from)?;
        while let Some(entry) = entries.next_entry().await.map_err(Error::from)? {
            let path = entry.path();
            if !path.extension().map(|e| e == "txt").unwrap_or(false) {
                continue;
            }
            let fname = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if crate::config::parser::is_reserved_manifest(&fname) {
                continue;
            }
            if let Ok(body) = tokio::fs::read_to_string(&path).await {
                out.insert(path.to_string_lossy().into_owned(), body);
            }
        }
    }
    Ok(out)
}

/// Write manifest files back where they came from, backing up any pre-existing file that
/// would change to `<file>.linix-backup` (once) so a rollback never silently discards
/// uncommitted manifest edits.
///
/// Keys are full paths (see `read_manifests`).
pub async fn write_manifests_with_backup(
    manifests: &HashMap<String, String>,
    global_dir: &Path,
) -> Result<()> {
    tokio::fs::create_dir_all(global_dir)
        .await
        .map_err(Error::from)?;
    for (key, body) in manifests {
        let recorded = Path::new(key);
        if !recorded.is_absolute() {
            return Err(Error::Other(format!(
                "generation records the manifest {:?} without a full path, so there is no \
                 way to know where to restore it. Refusing to guess.",
                key
            )));
        }
        // The folder may be gone. Recreate it rather than dropping the file: a silent
        // partial restore is worse than an unexpected mkdir.
        if let Some(parent) = recorded.parent() {
            if !tokio::fs::try_exists(parent).await.unwrap_or(false) {
                warn!(
                    "Rollback: recreating {:?} — it held manifests when this generation \
                     was captured, and has since gone away.",
                    parent
                );
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(Error::from)?;
            }
        }
        let target = recorded.to_path_buf();
        if let Ok(existing) = tokio::fs::read_to_string(&target).await {
            if existing != *body {
                let backup = PathBuf::from(format!("{}.linix-backup", target.display()));
                if !tokio::fs::try_exists(&backup).await.unwrap_or(false) {
                    let _ = tokio::fs::copy(&target, &backup).await;
                }
            }
        }
        crate::utils::file::atomic_write(&target, body)?;
    }
    Ok(())
}

/// Compute the changes that make the CURRENT package set match a generation, optionally
/// scoped to a set of backends and/or a single package (`name` or `backend:name`). Install
/// nodes carry the generation's recorded version, so pin-capable backends downgrade/upgrade
/// to exactly that version; others reinstall (their honest limit). The result is a flat
/// graph handed to the normal transaction engine, so rollback keeps snapshot + WAL safety.
pub fn plan_rollback(
    generation: &Generation,
    current: &[ManagedPackage],
    backends: Option<&[String]>,
    package: Option<&str>,
) -> SyncChanges {
    let in_scope = |backend: &str, name: &str| -> bool {
        if let Some(bs) = backends {
            if !bs.iter().any(|b| b == backend) {
                return false;
            }
        }
        if let Some(p) = package {
            if p != name && p != format!("{}:{}", backend, name) {
                return false;
            }
        }
        true
    };
    let key = |b: &str, n: &str| format!("{}:{}", b, n);

    let target: HashMap<String, &ManagedPackage> = generation
        .packages
        .iter()
        .filter(|p| in_scope(&p.backend, &p.name))
        .map(|p| (key(&p.backend, &p.name), p))
        .collect();
    let curr: HashMap<String, &ManagedPackage> = current
        .iter()
        .filter(|p| in_scope(&p.backend, &p.name))
        .map(|p| (key(&p.backend, &p.name), p))
        .collect();

    let mut graph: StableDiGraph<GraphAction, ()> = StableDiGraph::new();

    // Present in the generation but missing or at a different version now → (re)install.
    for (k, tp) in &target {
        let needs = curr
            .get(k)
            .map(|cp| cp.version != tp.version)
            .unwrap_or(true);
        if needs {
            let mut options = HashMap::new();
            if let Some(v) = &tp.version {
                options.insert("version".to_string(), v.clone());
            }
            graph.add_node(GraphAction::Install(PackageSpec {
                name: tp.name.clone(),
                backend: tp.backend.clone(),
                options,
                requires: vec![],
                present: true,
            }));
        }
    }
    // Present now but absent in the generation → remove.
    for (k, cp) in &curr {
        if !target.contains_key(k) {
            graph.add_node(GraphAction::Remove {
                name: cp.name.clone(),
                backend: cp.backend.clone(),
            });
        }
    }

    SyncChanges {
        graph,
        ..Default::default()
    }
}

/// The package-level delta between two generations: what was added, removed, or changed
/// version. Pure data so it can be rendered as text or JSON and unit-tested without I/O.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GenerationDelta {
    /// Packages present in the newer set but not the older one ("backend:name version").
    pub added: Vec<String>,
    /// Packages present in the older set but not the newer one.
    pub removed: Vec<String>,
    /// Packages in both sets whose version changed: (backend:name, from, to).
    pub changed: Vec<(String, String, String)>,
}

impl GenerationDelta {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

/// Render a version for display, mapping an unknown/unpinned version to `-`.
fn ver(v: &Option<String>) -> String {
    v.clone().unwrap_or_else(|| "-".to_string())
}

/// Diff two realized package sets (`from` = older/baseline, `to` = newer). Keyed by
/// `backend:name`, so the same package moving version shows up as a single `changed` entry
/// rather than an add + a remove. Ordered for stable output (added/removed sorted by key).
pub fn diff_package_sets(from: &[ManagedPackage], to: &[ManagedPackage]) -> GenerationDelta {
    let key = |p: &ManagedPackage| format!("{}:{}", p.backend, p.name);
    let from_map: HashMap<String, &ManagedPackage> = from.iter().map(|p| (key(p), p)).collect();
    let to_map: HashMap<String, &ManagedPackage> = to.iter().map(|p| (key(p), p)).collect();

    let mut delta = GenerationDelta::default();
    for (k, tp) in &to_map {
        match from_map.get(k) {
            None => delta.added.push(format!("{} {}", k, ver(&tp.version))),
            Some(fp) if fp.version != tp.version => {
                delta
                    .changed
                    .push((k.clone(), ver(&fp.version), ver(&tp.version)));
            }
            Some(_) => {}
        }
    }
    for (k, fp) in &from_map {
        if !to_map.contains_key(k) {
            delta.removed.push(format!("{} {}", k, ver(&fp.version)));
        }
    }
    delta.added.sort();
    delta.removed.sort();
    delta.changed.sort();
    delta
}

/// Diff two whole generations (convenience wrapper over [`diff_package_sets`]).
pub fn diff_generations(from: &Generation, to: &Generation) -> GenerationDelta {
    diff_package_sets(&from.packages, &to.packages)
}

/// A single frozen generation: the realized state plus the manifests that produced it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Generation {
    pub id: String,
    /// RFC3339 timestamp.
    pub timestamp: String,
    /// Optional human label (for `keep = [...]` pins and readability).
    #[serde(default)]
    pub label: String,
    /// Protects this generation from retention GC when set imperatively.
    #[serde(default)]
    pub pinned: bool,
    /// The exact package set that was managed at capture time.
    pub packages: Vec<ManagedPackage>,
    /// A frozen copy of the manifest files (filename -> body) that produced this state.
    #[serde(default)]
    pub manifests: HashMap<String, String>,
    /// The git commit of the manifest repo at capture time, if the config dir is a git repo.
    /// This ties a realized generation to the exact *intent* commit that produced it, so a
    /// system rollback can optionally also restore the matching manifests via git (and vice
    /// versa). `None` when git isn't in use. `#[serde(default)]` keeps old generations valid.
    #[serde(default)]
    pub git_commit: Option<String>,
}

impl Generation {
    fn parsed_time(&self) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(&self.timestamp)
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now())
    }

    fn retention_item(&self) -> RetentionItem {
        RetentionItem {
            id: self.id.clone(),
            label: self.label.clone(),
            timestamp: self.parsed_time(),
            pinned: self.pinned,
        }
    }
}

/// On-disk store of generations under `<data_dir>/generations/`.
pub struct GenerationStore {
    dir: PathBuf,
}

impl GenerationStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Default store location under the LiNix data directory.
    pub fn default_store() -> Self {
        Self::new(crate::utils::safe_data_dir().join("generations"))
    }

    fn path_for(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.json", id))
    }

    /// Capture a new generation from the current realized state + manifests and persist it.
    /// `id` is caller-supplied so it can be tied to a snapshot / timestamp deterministically.
    pub async fn capture(
        &self,
        id: &str,
        timestamp: &str,
        label: &str,
        state: &StateRegistry,
        groups_dir: &Path,
    ) -> Result<Generation> {
        // The manifest repo's root is the config dir, i.e. the parent of the groups dir.
        let git_commit = groups_dir
            .parent()
            .map(crate::core::GitManager::new)
            .filter(|g| g.is_repo())
            .and_then(|g| g.head().ok().flatten());

        let generation = Generation {
            id: id.to_string(),
            timestamp: timestamp.to_string(),
            label: label.to_string(),
            pinned: false,
            packages: state.packages.clone(),
            manifests: read_manifests(groups_dir).await?,
            git_commit,
        };
        tokio::fs::create_dir_all(&self.dir)
            .await
            .map_err(Error::from)?;
        let json = serde_json::to_string_pretty(&generation)
            .map_err(|e| Error::Other(format!("generation serialize: {}", e)))?;
        crate::utils::file::atomic_write(&self.path_for(id), &json)?;
        Ok(generation)
    }

    /// List all stored generations, newest first.
    pub async fn list(&self) -> Result<Vec<Generation>> {
        let mut generations = Vec::new();
        if !tokio::fs::try_exists(&self.dir).await.unwrap_or(false) {
            return Ok(generations);
        }
        let mut entries = tokio::fs::read_dir(&self.dir).await.map_err(Error::from)?;
        while let Some(entry) = entries.next_entry().await.map_err(Error::from)? {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(body) = tokio::fs::read_to_string(&path).await {
                    if let Ok(g) = serde_json::from_str::<Generation>(&body) {
                        generations.push(g);
                    }
                }
            }
        }
        generations.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        Ok(generations)
    }

    pub async fn load(&self, id: &str) -> Result<Generation> {
        let body = tokio::fs::read_to_string(self.path_for(id))
            .await
            .map_err(|_| Error::Other(format!("generation '{}' not found", id)))?;
        serde_json::from_str(&body).map_err(|e| Error::Other(format!("generation parse: {}", e)))
    }

    /// Set or clear a generation's pin (a pinned generation survives retention GC).
    pub async fn set_pinned(&self, id: &str, pinned: bool) -> Result<()> {
        let mut g = self.load(id).await?;
        g.pinned = pinned;
        let json = serde_json::to_string_pretty(&g)
            .map_err(|e| Error::Other(format!("generation serialize: {}", e)))?;
        crate::utils::file::atomic_write(&self.path_for(id), &json)?;
        Ok(())
    }

    /// The generation whose timestamp is at or before `when`, closest to it — i.e. the
    /// state that was current at that moment. Used to pair a snapshot rollback with its
    /// generation.
    pub async fn nearest_at_or_before(&self, when: DateTime<Utc>) -> Result<Option<Generation>> {
        let mut best: Option<Generation> = None;
        for g in self.list().await? {
            if g.parsed_time() <= when {
                // list() is newest-first, so the first match is already the closest.
                best = Some(g);
                break;
            }
        }
        Ok(best)
    }

    /// Apply a retention policy, deleting the generations it does not keep. Returns the
    /// ids that were removed.
    pub async fn prune(&self, policy: &RetentionPolicy, now: DateTime<Utc>) -> Result<Vec<String>> {
        let generations = self.list().await?;
        let items: Vec<RetentionItem> =
            generations.iter().map(Generation::retention_item).collect();
        let doomed = policy.select_deletions(&items, now);
        for id in &doomed {
            let _ = tokio::fs::remove_file(self.path_for(id)).await;
        }
        Ok(doomed)
    }

    /// Roll back to a generation: restore its realized package set into `state` and write
    /// its frozen manifests back into `groups_dir`. Existing manifest files that the
    /// generation would overwrite are backed up once to `<file>.linix-backup` first, so a
    /// rollback never silently discards uncommitted manifest edits.
    pub async fn restore(
        &self,
        id: &str,
        state: &mut StateRegistry,
        global_dir: &Path,
    ) -> Result<()> {
        let generation = self.load(id).await?;
        // Each manifest goes back to the folder it was captured from. `global_dir` is not a
        // fallback for generations recorded without full paths — those are refused outright,
        // since guessing a destination would write manifests to the wrong folder.
        write_manifests_with_backup(&generation.manifests, global_dir).await?;
        state.packages = generation.packages.clone();
        state.save()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap as Map;
    use tempfile::tempdir;

    fn pkg(name: &str, backend: &str, ver: &str) -> ManagedPackage {
        ManagedPackage {
            name: name.into(),
            backend: backend.into(),
            version: Some(ver.into()),
            installed_at: 0,
            expires_at: None,
            options: Map::new(),
            source: None,
            is_transient: false,
            session_id: None,
        }
    }

    async fn write(dir: &Path, name: &str, body: &str) {
        tokio::fs::create_dir_all(dir).await.unwrap();
        tokio::fs::write(dir.join(name), body).await.unwrap();
    }

    #[tokio::test]
    async fn capture_then_restore_round_trips_state_and_manifests() {
        let tmp = tempdir().unwrap();
        let groups = tmp.path().join("groups");
        write(&groups, "base.txt", "apt:curl\ncargo:ripgrep\n").await;

        let store = GenerationStore::new(tmp.path().join("gens"));
        let mut state = StateRegistry::new(tmp.path().join("registry.json"));
        state.packages = vec![pkg("curl", "apt", "8.4.0"), pkg("ripgrep", "cargo", "14.1")];

        let gen = store
            .capture(
                "gen1",
                "2026-07-03T00:00:00Z",
                "",
                &state,
                &groups,
            )
            .await
            .unwrap();
        assert_eq!(gen.packages.len(), 2);
        // Keyed by full path, not bare filename: with -g there can be several folders in
        // play, each with its own base.txt, and a name-keyed map silently keeps one.
        assert_eq!(
            gen.manifests
                .get(groups.join("base.txt").to_string_lossy().as_ref())
                .unwrap(),
            "apt:curl\ncargo:ripgrep\n"
        );

        // The world moves on: manifest edited, state changed.
        write(&groups, "base.txt", "apt:curl\n").await;
        state.packages = vec![pkg("curl", "apt", "9.0.0")];

        // Roll back.
        store.restore("gen1", &mut state, &groups).await.unwrap();

        // Manifest restored to the captured version (and the newer one backed up).
        assert_eq!(
            tokio::fs::read_to_string(groups.join("base.txt"))
                .await
                .unwrap(),
            "apt:curl\ncargo:ripgrep\n"
        );
        assert!(tokio::fs::try_exists(groups.join("base.txt.linix-backup"))
            .await
            .unwrap());
        // Realized state restored (curl back to 8.4.0, ripgrep back).
        assert_eq!(state.packages.len(), 2);
        assert!(state.packages.iter().any(|p| p.name == "ripgrep"));
        assert!(state
            .packages
            .iter()
            .any(|p| p.name == "curl" && p.version.as_deref() == Some("8.4.0")));
    }

    fn gen_with(packages: Vec<ManagedPackage>) -> Generation {
        Generation {
            id: "g1".into(),
            timestamp: "2026-07-01T00:00:00Z".into(),
            label: String::new(),
            pinned: false,
            packages,
            manifests: Map::new(),
            git_commit: None,
        }
    }

    #[test]
    fn plan_rollback_diffs_installs_downgrades_and_removes() {
        let generation = gen_with(vec![
            pkg("curl", "apt", "8.4.0"),     // same as current -> no-op
            pkg("ripgrep", "cargo", "14.1"), // current has 14.0 -> reinstall to 14.1
            pkg("bat", "cargo", "0.24"),     // missing now -> install
        ]);
        let current = vec![
            pkg("curl", "apt", "8.4.0"),
            pkg("ripgrep", "cargo", "14.0"),
            pkg("fd", "cargo", "9.0"), // extra -> remove
        ];

        let changes = plan_rollback(&generation, &current, None, None);
        let installs: Vec<&PackageSpec> = changes
            .graph
            .node_weights()
            .filter_map(|w| match w {
                GraphAction::Install(s) => Some(s),
                _ => None,
            })
            .collect();
        let removes: Vec<&str> = changes
            .graph
            .node_weights()
            .filter_map(|w| match w {
                GraphAction::Remove { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();

        assert_eq!(installs.len(), 2, "ripgrep + bat");
        assert!(installs
            .iter()
            .any(|s| s.name == "ripgrep"
                && s.options.get("version").map(String::as_str) == Some("14.1")));
        assert!(installs.iter().any(|s| s.name == "bat"));
        assert_eq!(removes, vec!["fd"]);
    }

    #[test]
    fn plan_rollback_respects_package_and_backend_scope() {
        let generation = gen_with(vec![
            pkg("curl", "apt", "8.4.0"),
            pkg("ripgrep", "cargo", "14.1"),
        ]);
        let current = vec![
            pkg("curl", "apt", "8.4.0"),
            pkg("ripgrep", "cargo", "14.0"),
            pkg("fd", "cargo", "9.0"),
        ];

        // Only ripgrep: one reinstall, and fd is out of scope so NOT removed.
        let scoped = plan_rollback(&generation, &current, None, Some("ripgrep"));
        assert_eq!(scoped.total_install(), 1);
        assert_eq!(scoped.total_remove(), 0);

        // Only the apt backend: curl is unchanged, so nothing to do (cargo untouched).
        let apt_only = plan_rollback(&generation, &current, Some(&["apt".to_string()]), None);
        assert!(apt_only.is_empty());
    }

    #[test]
    fn legacy_generation_without_git_commit_field_loads() {
        // A generation written before git stamping existed must still deserialize.
        let json = r#"{"id":"g1","timestamp":"2026-07-01T00:00:00Z","packages":[]}"#;
        let g: Generation = serde_json::from_str(json).expect("legacy generation loads");
        assert!(g.git_commit.is_none());
        assert!(g.manifests.is_empty());
    }

    #[tokio::test]
    async fn capture_stamps_git_commit_when_config_dir_is_a_repo() {
        if !crate::core::GitManager::git_available() {
            eprintln!("skipping: git not installed");
            return;
        }
        let tmp = tempdir().unwrap();
        let root = tmp.path(); // config root = parent of groups
        let groups = root.join("groups");
        write(&groups, "base.txt", "apt:curl\n").await;

        // Make the config root a git repo with one commit.
        let git = crate::core::GitManager::new(root);
        git.init().unwrap();
        git.commit_all("initial").unwrap();
        let head = git.head().unwrap().unwrap();

        let store = GenerationStore::new(root.join("gens"));
        let state = StateRegistry::new(root.join("registry.json"));
        let gen = store
            .capture(
                "g1",
                "2026-07-03T00:00:00Z",
                "",
                &state,
                &groups,
            )
            .await
            .unwrap();

        assert_eq!(gen.git_commit.as_deref(), Some(head.as_str()));
    }

    #[tokio::test]
    async fn pin_and_nearest_at_or_before() {
        let tmp = tempdir().unwrap();
        let groups = tmp.path().join("groups");
        write(&groups, "base.txt", "apt:curl\n").await;
        let store = GenerationStore::new(tmp.path().join("gens"));
        let state = StateRegistry::new(tmp.path().join("registry.json"));
        store
            .capture(
                "g1",
                "2026-07-01T00:00:00Z",
                "",
                &state,
                &groups,
            )
            .await
            .unwrap();
        store
            .capture(
                "g2",
                "2026-07-05T00:00:00Z",
                "",
                &state,
                &groups,
            )
            .await
            .unwrap();

        store.set_pinned("g1", true).await.unwrap();
        assert!(store.load("g1").await.unwrap().pinned);

        // Nearest at-or-before 2026-07-03 is g1 (g2 is later).
        let when = DateTime::parse_from_rfc3339("2026-07-03T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            store.nearest_at_or_before(when).await.unwrap().unwrap().id,
            "g1"
        );
    }

    #[tokio::test]
    async fn prune_keeps_last_one_and_deletes_older() {
        let tmp = tempdir().unwrap();
        let groups = tmp.path().join("groups");
        write(&groups, "base.txt", "apt:curl\n").await;
        let store = GenerationStore::new(tmp.path().join("gens"));
        let state = StateRegistry::new(tmp.path().join("registry.json"));

        store
            .capture(
                "g1",
                "2026-07-01T00:00:00Z",
                "",
                &state,
                &groups,
            )
            .await
            .unwrap();
        store
            .capture(
                "g2",
                "2026-07-02T00:00:00Z",
                "",
                &state,
                &groups,
            )
            .await
            .unwrap();
        store
            .capture(
                "g3",
                "2026-07-03T00:00:00Z",
                "",
                &state,
                &groups,
            )
            .await
            .unwrap();

        let policy = RetentionPolicy {
            keep_last: 1,
            ..Default::default()
        };
        let now = DateTime::parse_from_rfc3339("2026-07-03T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut deleted = store.prune(&policy, now).await.unwrap();
        deleted.sort();
        assert_eq!(deleted, vec!["g1", "g2"]);
        let remaining = store.list().await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "g3");
    }

    #[test]
    fn diff_classifies_added_removed_and_version_changed() {
        let from = vec![
            pkg("curl", "apt", "8.4.0"),     // stays same
            pkg("ripgrep", "cargo", "14.0"), // version changes
            pkg("fd", "cargo", "9.0"),       // removed
        ];
        let to = vec![
            pkg("curl", "apt", "8.4.0"),
            pkg("ripgrep", "cargo", "14.1"),
            pkg("bat", "cargo", "0.24"), // added
        ];

        let d = diff_package_sets(&from, &to);
        assert_eq!(d.added, vec!["cargo:bat 0.24".to_string()]);
        assert_eq!(d.removed, vec!["cargo:fd 9.0".to_string()]);
        assert_eq!(
            d.changed,
            vec![(
                "cargo:ripgrep".to_string(),
                "14.0".to_string(),
                "14.1".to_string()
            )]
        );
        assert!(!d.is_empty());
    }

    #[test]
    fn diff_of_identical_sets_is_empty() {
        let set = vec![pkg("curl", "apt", "8.4.0"), pkg("bat", "cargo", "0.24")];
        assert!(diff_package_sets(&set, &set).is_empty());
    }

    #[test]
    fn diff_renders_unknown_version_as_dash() {
        let mut p = pkg("mystery", "web", "0");
        p.version = None;
        let d = diff_package_sets(&[], std::slice::from_ref(&p));
        assert_eq!(d.added, vec!["web:mystery -".to_string()]);
    }
}
