// src/app/profile.rs

use crate::app::diagnostics::FailureDiagnosticEngine;
use crate::app::sync::{ChangePlanner, ScopedFilter, StateResolver, SyncEngine};
use crate::app::{LuaHooks, MetricsCollector};
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::core::{CommandExecutor, Error, Journal, Result, SnapshotManager, StateRegistry};
use crate::utils::progress::ProgressReporter;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, instrument};

/// The reserved manifest LiNix writes the union of all active profiles into. It lives in
/// the groups dir alongside `local.txt`, so the ordinary declarative resolver picks it up
/// and `sync`/`status`/`prune` converge the system to the active profile set. It is
/// machine-owned: `activate`/`deactivate` overwrite it wholesale.
const RESERVED_MANIFEST: &str = "_active_profiles.txt";

/// The file (in the profiles dir) that records which profiles are currently active, one
/// name per line. Multiple profiles can be active at once; their package sets are unioned.
const ACTIVE_FILE: &str = "active";

/// Manages system "Identities" or Profiles.
///
/// A profile is a named set of software defined in `<profiles_dir>/<name>.profile` (a
/// manifest with extra directives), or, for backward compatibility, a legacy directory
/// `<profiles_dir>/<name>/` of `.txt` manifests. Profiles compose:
///
/// - `include <other>` — union in another profile's resolved packages (the "plus").
/// - `exclude <other>` — subtract another profile's resolved packages.
/// - `-<pkg>`          — subtract a single package (the "minus").
/// - any other line    — a package spec (`ripgrep`, `cargo:exa`, `@module:dev`, …).
///
/// Several profiles can be *active* simultaneously; the effective desired state is the
/// union of every active profile. Activation and deactivation are live — they re-render
/// the reserved manifest and converge the running system with no reboot.
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
    profiles_dir: PathBuf,
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
        let profiles_dir = config
            .groups_dir
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("profiles");

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
            profiles_dir,
        }
    }

    // ---------------------------------------------------------------- paths

    fn profile_file(&self, name: &str) -> PathBuf {
        self.profiles_dir.join(format!("{name}.profile"))
    }
    fn legacy_dir(&self, name: &str) -> PathBuf {
        self.profiles_dir.join(name)
    }
    fn active_file(&self) -> PathBuf {
        self.profiles_dir.join(ACTIVE_FILE)
    }

    async fn profile_exists(&self, name: &str) -> bool {
        tokio::fs::try_exists(self.profile_file(name))
            .await
            .unwrap_or(false)
            || tokio::fs::try_exists(self.legacy_dir(name))
                .await
                .unwrap_or(false)
    }

    // ----------------------------------------------------- public lifecycle

    /// Activate one or more profiles: add each to the active set, then converge the system
    /// to the union of all active profiles. Idempotent — activating an already-active
    /// profile is a no-op for the set.
    #[instrument(skip(self))]
    pub async fn activate(&self, names: &[String]) -> Result<()> {
        let mut active = self.load_active().await?;
        for name in names {
            if !self.profile_exists(name).await {
                return Err(Error::Config(format!(
                    "Profile '{}' not found in {:?}. Create it with `linix profile create {}`.",
                    name, self.profiles_dir, name
                )));
            }
            if !active.iter().any(|a| a == name) {
                active.push(name.clone());
            }
        }
        self.save_active(&active).await?;
        let union = self.materialize(&active).await?;
        info!(
            "Profiles: {} active [{}] → {} package(s) desired. Converging...",
            active.len(),
            active.join(", "),
            union.len()
        );
        self.sync_now().await
    }

    /// Deactivate one or more profiles: drop each from the active set, then converge —
    /// removing packages no longer required by any *remaining* active profile (packages
    /// still provided by another active profile survive; imperative installs are always
    /// spared per `protect_imperative`).
    #[instrument(skip(self))]
    pub async fn deactivate(&self, names: &[String]) -> Result<()> {
        let mut active = self.load_active().await?;
        let before = active.len();
        active.retain(|a| !names.iter().any(|n| n == a));
        if active.len() == before {
            info!("Profiles: none of [{}] were active.", names.join(", "));
        }
        self.save_active(&active).await?;
        let _ = self.materialize(&active).await?;
        info!(
            "Profiles: deactivated [{}]; {} still active [{}]. Converging...",
            names.join(", "),
            active.len(),
            active.join(", ")
        );
        self.sync_now().await
    }

    /// Exclusively switch to a single profile: deactivate everything else, activate this
    /// one, and converge. This is the "swap my whole identity" operation.
    #[instrument(skip(self))]
    pub async fn switch(&self, name: &str) -> Result<()> {
        if !self.profile_exists(name).await {
            return Err(Error::Config(format!(
                "Profile '{}' not found in {:?}",
                name, self.profiles_dir
            )));
        }
        let active = vec![name.to_string()];
        self.save_active(&active).await?;
        self.materialize(&active).await?;
        info!(
            "Profiles: switched to exclusive identity '{}'. Converging...",
            name
        );
        self.sync_now().await
    }

    // --------------------------------------------------------- introspection

    /// Every defined profile (new `.profile` files and legacy directories), sorted.
    pub async fn list_profiles(&self) -> Result<Vec<String>> {
        let mut profiles: HashSet<String> = HashSet::new();
        if tokio::fs::try_exists(&self.profiles_dir)
            .await
            .unwrap_or(false)
        {
            let mut entries = tokio::fs::read_dir(&self.profiles_dir)
                .await
                .map_err(Error::from)?;
            while let Some(entry) = entries.next_entry().await.map_err(Error::from)? {
                let path = entry.path();
                let ft = entry.file_type().await.map_err(Error::from)?;
                if ft.is_dir() {
                    profiles.insert(entry.file_name().to_string_lossy().into_owned());
                } else if path.extension().is_some_and(|e| e == "profile") {
                    if let Some(stem) = path.file_stem() {
                        profiles.insert(stem.to_string_lossy().into_owned());
                    }
                }
            }
        }
        let mut out: Vec<String> = profiles.into_iter().collect();
        out.sort();
        Ok(out)
    }

    /// The currently-active profiles, in activation order.
    pub async fn active_profiles(&self) -> Result<Vec<String>> {
        self.load_active().await
    }

    /// The resolved (composed) package set a profile expands to.
    pub async fn show(&self, name: &str) -> Result<Vec<String>> {
        if !self.profile_exists(name).await {
            return Err(Error::Config(format!(
                "Profile '{}' not found in {:?}",
                name, self.profiles_dir
            )));
        }
        self.resolve(name).await
    }

    /// Scaffold a new, empty profile definition file.
    pub async fn create(&self, name: &str) -> Result<()> {
        tokio::fs::create_dir_all(&self.profiles_dir).await.ok();
        let path = self.profile_file(name);
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return Err(Error::Config(format!(
                "Profile '{}' already exists at {:?}",
                name, path
            )));
        }
        let tmpl = format!(
            "# LiNix profile: {name}\n\
             # One package per line, e.g.:\n\
             #   ripgrep\n\
             #   cargo:exa\n\
             #   npm:cowsay\n\
             # Compose from other profiles:\n\
             #   include base      (union another profile — the \"plus\")\n\
             #   intersect secure  (keep only packages also in another profile)\n\
             #   exclude heavy     (subtract another profile's packages)\n\
             #   -vim              (subtract a single package — the \"minus\")\n\
             # Or a full set expression with grouping (parentheses nest infinitely):\n\
             #   (work | gaming) & security          (union, then intersect)\n\
             #   intersect(union(work, gaming), security)   (same, function form)\n\
             #   base \\\\ heavy                        (difference)\n"
        );
        tokio::fs::write(&path, tmpl).await.map_err(Error::from)?;
        Ok(())
    }

    /// Snapshot the current desired state (everything the manifests resolve to) into a new
    /// standalone profile, so "what I have now" becomes a reusable identity.
    pub async fn save_current_as(&self, name: &str) -> Result<()> {
        let resolver = StateResolver::new(&self.config, self.registry.clone(), false).await;
        let desired = resolver.resolve_desired_state().await?;

        let mut lines: Vec<String> = Vec::new();
        for specs in desired.values() {
            for s in specs {
                lines.push(format!("{}:{}", s.backend, s.name));
            }
        }
        lines.sort();
        lines.dedup();

        tokio::fs::create_dir_all(&self.profiles_dir).await.ok();
        let body = format!(
            "# LiNix profile '{name}' — snapshot of the current desired state\n{}\n",
            lines.join("\n")
        );
        tokio::fs::write(self.profile_file(name), body)
            .await
            .map_err(Error::from)?;
        info!("Profile '{}' saved with {} package(s).", name, lines.len());
        Ok(())
    }

    // ------------------------------------------------------------- internals

    async fn load_active(&self) -> Result<Vec<String>> {
        let path = self.active_file();
        if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return Ok(vec![]);
        }
        let body = tokio::fs::read_to_string(&path)
            .await
            .map_err(Error::from)?;
        Ok(body
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.to_string())
            .collect())
    }

    async fn save_active(&self, active: &[String]) -> Result<()> {
        tokio::fs::create_dir_all(&self.profiles_dir).await.ok();
        let body = format!(
            "# LiNix active profiles (managed by `linix activate`/`deactivate`)\n{}\n",
            active.join("\n")
        );
        tokio::fs::write(self.active_file(), body)
            .await
            .map_err(Error::from)?;
        Ok(())
    }

    /// Read every profile definition under `profiles_dir` into a name → raw-lines map:
    /// `<name>.profile` files, and (legacy) `<name>/` directories of concatenated `.txt`
    /// manifests. Reading the whole set once keeps the composition algorithm pure/sync and
    /// unit-testable (see [`compose`]).
    async fn load_all_definitions(&self) -> Result<HashMap<String, Vec<String>>> {
        let mut defs: HashMap<String, Vec<String>> = HashMap::new();
        if !tokio::fs::try_exists(&self.profiles_dir)
            .await
            .unwrap_or(false)
        {
            return Ok(defs);
        }
        let mut entries = tokio::fs::read_dir(&self.profiles_dir)
            .await
            .map_err(Error::from)?;
        while let Some(entry) = entries.next_entry().await.map_err(Error::from)? {
            let path = entry.path();
            let ft = entry.file_type().await.map_err(Error::from)?;
            if ft.is_dir() {
                // Legacy: a directory of .txt manifests.
                let name = entry.file_name().to_string_lossy().into_owned();
                let mut lines = Vec::new();
                let mut inner = tokio::fs::read_dir(&path).await.map_err(Error::from)?;
                while let Some(e) = inner.next_entry().await.map_err(Error::from)? {
                    let p = e.path();
                    if p.extension().is_some_and(|x| x == "txt") {
                        let body = tokio::fs::read_to_string(&p).await.map_err(Error::from)?;
                        lines.extend(body.lines().map(|l| l.to_string()));
                    }
                }
                defs.entry(name).or_default().extend(lines);
            } else if path.extension().is_some_and(|e| e == "profile") {
                if let Some(stem) = path.file_stem() {
                    let body = tokio::fs::read_to_string(&path)
                        .await
                        .map_err(Error::from)?;
                    defs.insert(
                        stem.to_string_lossy().into_owned(),
                        body.lines().map(|l| l.to_string()).collect(),
                    );
                }
            }
        }
        Ok(defs)
    }

    /// Resolve a profile to its concrete, de-duplicated, ordered package set, applying
    /// `include` (union), `exclude` (subtract a profile), and `-pkg` (subtract a package).
    async fn resolve(&self, name: &str) -> Result<Vec<String>> {
        let defs = self.load_all_definitions().await?;
        Ok(compose(name, &defs, &mut HashSet::new()))
    }

    /// Render the union of the active profiles into the reserved manifest so the ordinary
    /// declarative pipeline (resolve → plan → sync) sees it. Returns the union for logging.
    async fn materialize(&self, active: &[String]) -> Result<Vec<String>> {
        let mut union: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for name in active {
            for tok in self.resolve(name).await? {
                if seen.insert(tok.clone()) {
                    union.push(tok);
                }
            }
        }

        tokio::fs::create_dir_all(&self.config.groups_dir)
            .await
            .ok();
        let path = self.config.groups_dir.join(RESERVED_MANIFEST);
        let mut body = String::from(
            "# AUTO-GENERATED by `linix activate`/`deactivate` — do not edit by hand.\n",
        );
        body.push_str(&format!("# Active profiles: {}\n", active.join(", ")));
        for tok in &union {
            body.push_str(tok);
            body.push('\n');
        }
        tokio::fs::write(&path, body).await.map_err(Error::from)?;
        Ok(union)
    }

    /// Converge the running system to the current manifest set (including the reserved
    /// active-profiles manifest). Uses the default planner, which prunes managed drift —
    /// so deactivation actually removes the packages that dropped out.
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

        let resolver = StateResolver::new(&self.config, self.registry.clone(), false).await;
        let desired = resolver.resolve_desired_state().await?;

        let changes = {
            let state_guard = self.state.lock().await;
            let planner = ChangePlanner::new(self.registry.clone(), &state_guard, &self.config);
            planner.plan(&desired, ScopedFilter::None).await?
        };

        if changes.is_empty() {
            info!("Profiles: system already matches the active profile set.");
            return Ok(());
        }

        engine.sync(changes).await?;
        Ok(())
    }
}

/// Pure composition of a profile into its concrete, de-duplicated, ordered package set.
///
/// Directives (per line; `#` comments and blanks ignored):
/// - `include <name>` / `use <name>` — union another profile's resolved set (the "plus").
/// - `intersect <name>`              — keep only packages ALSO in another profile.
/// - `exclude <name>`                — subtract another profile's resolved set.
/// - `-<pkg>`                        — subtract a single package (the "minus").
/// - a set *expression*             — e.g. `intersect(union(work, gaming), security)` or
///   `(work | gaming) & security` (see [`crate::app::profile_expr`]); its result is unioned in.
/// - anything else                   — a package spec.
///
/// Evaluation order is fixed and predictable: **union first, then intersect, then subtract**.
/// So additions (plain packages, `include`, and expression results) are collected; the set is
/// then narrowed to the intersection with every `intersect`ed profile; and finally all
/// subtractions (`-pkg` and every `exclude`d profile) are removed. Subtraction always wins.
/// `visiting` guards cyclic `include`/`intersect`/`exclude` references — a profile already on
/// the resolution stack contributes nothing further. Unknown profile names resolve to the
/// empty set (missing files are validated at the `activate`/`switch` entry points).
fn compose(
    name: &str,
    defs: &HashMap<String, Vec<String>>,
    visiting: &mut HashSet<String>,
) -> Vec<String> {
    if !visiting.insert(name.to_string()) {
        return vec![]; // cycle
    }
    let mut adds: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut sub_pkgs: HashSet<String> = HashSet::new();
    let mut sub_profiles: Vec<String> = Vec::new();
    let mut intersect_profiles: Vec<String> = Vec::new();

    for raw in defs.get(name).map(|v| v.as_slice()).unwrap_or(&[]) {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line
            .strip_prefix("include ")
            .or_else(|| line.strip_prefix("use "))
        {
            let child = rest.trim();
            if !child.is_empty() {
                for tok in compose(child, defs, visiting) {
                    if seen.insert(tok.clone()) {
                        adds.push(tok);
                    }
                }
            }
        } else if let Some(rest) = line.strip_prefix("intersect ") {
            let child = rest.trim();
            if !child.is_empty() {
                intersect_profiles.push(child.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("exclude ") {
            let child = rest.trim();
            if !child.is_empty() {
                sub_profiles.push(child.to_string());
            }
        } else if let Some(rest) = line.strip_prefix('-') {
            let tok = rest.trim();
            if !tok.is_empty() {
                sub_pkgs.insert(tok.to_string());
            }
        } else if crate::app::profile_expr::looks_like_expression(line) {
            // A set expression: resolve each atom as a profile (recursively) if one exists
            // by that name, otherwise treat it as a literal package token. Then union the
            // result into this profile's additions.
            let mut resolve_atom = |atom: &str| {
                if defs.contains_key(atom) {
                    compose(atom, defs, visiting)
                } else {
                    vec![atom.to_string()]
                }
            };
            match crate::app::profile_expr::evaluate(line, &mut resolve_atom) {
                Ok(tokens) => {
                    for tok in tokens {
                        if seen.insert(tok.clone()) {
                            adds.push(tok);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Profile '{}': ignoring malformed set expression `{}`: {}",
                        name,
                        line,
                        e
                    );
                }
            }
        } else if seen.insert(line.to_string()) {
            adds.push(line.to_string());
        }
    }

    // Narrow to the intersection with each `intersect`ed profile (after all unions).
    for p in &intersect_profiles {
        let keep: HashSet<String> = compose(p, defs, visiting).into_iter().collect();
        adds.retain(|t| keep.contains(t));
    }

    // Collect profile-level subtractions, then apply all subtractions last.
    for p in &sub_profiles {
        for tok in compose(p, defs, visiting) {
            sub_pkgs.insert(tok);
        }
    }

    visiting.remove(name);
    adds.into_iter().filter(|t| !sub_pkgs.contains(t)).collect()
}

#[cfg(test)]
mod tests {
    use super::compose;
    use std::collections::{HashMap, HashSet};

    fn defs(pairs: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
        pairs
            .iter()
            .map(|(n, lines)| (n.to_string(), lines.iter().map(|l| l.to_string()).collect()))
            .collect()
    }

    fn resolve(name: &str, d: &HashMap<String, Vec<String>>) -> Vec<String> {
        compose(name, d, &mut HashSet::new())
    }

    #[test]
    fn standalone_profile_lists_its_packages() {
        let d = defs(&[("a", &["apt:jq", "apt:htop", "# a comment", ""])]);
        assert_eq!(resolve("a", &d), vec!["apt:jq", "apt:htop"]);
    }

    #[test]
    fn include_unions_and_dedups_preserving_order() {
        let d = defs(&[
            ("base", &["apt:jq"]),
            ("dev", &["include base", "apt:htop", "apt:jq"]),
        ]);
        assert_eq!(resolve("dev", &d), vec!["apt:jq", "apt:htop"]);
    }

    #[test]
    fn minus_subtracts_a_single_package() {
        let d = defs(&[
            ("both", &["apt:jq", "apt:htop"]),
            ("lean", &["include both", "-apt:jq"]),
        ]);
        assert_eq!(resolve("lean", &d), vec!["apt:htop"]);
    }

    #[test]
    fn exclude_subtracts_another_profiles_packages() {
        let d = defs(&[
            ("heavy", &["apt:gdb", "apt:valgrind"]),
            ("base", &["apt:jq", "apt:gdb"]),
            ("slim", &["include base", "exclude heavy"]),
        ]);
        // base brings jq+gdb; excluding heavy (gdb+valgrind) removes gdb.
        assert_eq!(resolve("slim", &d), vec!["apt:jq"]);
    }

    #[test]
    fn relational_plus_and_minus_together() {
        let d = defs(&[
            ("a", &["apt:jq"]),
            ("b", &["apt:htop"]),
            ("combo", &["include a", "include b"]),
            ("lean", &["include combo", "-apt:jq"]),
        ]);
        assert_eq!(resolve("combo", &d), vec!["apt:jq", "apt:htop"]);
        assert_eq!(resolve("lean", &d), vec!["apt:htop"]);
    }

    #[test]
    fn cyclic_includes_terminate() {
        let d = defs(&[
            ("x", &["include y", "apt:one"]),
            ("y", &["include x", "apt:two"]),
        ]);
        // Must not stack-overflow; both packages surface exactly once.
        let mut r = resolve("x", &d);
        r.sort();
        assert_eq!(r, vec!["apt:one", "apt:two"]);
    }

    #[test]
    fn unknown_profile_is_empty_not_a_panic() {
        let d = defs(&[("a", &["include ghost", "apt:jq"])]);
        assert_eq!(resolve("a", &d), vec!["apt:jq"]);
    }

    #[test]
    fn intersect_directive_keeps_only_common_packages() {
        let d = defs(&[
            ("base", &["apt:jq", "apt:htop", "apt:gdb"]),
            ("secure", &["apt:jq", "apt:gdb", "apt:auditd"]),
            ("hardened", &["include base", "intersect secure"]),
        ]);
        // base ∩ secure = {jq, gdb}, in base's order.
        assert_eq!(resolve("hardened", &d), vec!["apt:jq", "apt:gdb"]);
    }

    #[test]
    fn intersect_applies_after_union_before_subtraction() {
        let d = defs(&[
            ("a", &["apt:jq", "apt:htop"]),
            ("b", &["apt:htop", "apt:gdb"]),
            ("common", &["apt:htop", "apt:jq"]),
            // union(a,b) = {jq,htop,gdb}; intersect common = {jq,htop}; minus jq = {htop}.
            ("combo", &["include a", "include b", "intersect common", "-apt:jq"]),
        ]);
        assert_eq!(resolve("combo", &d), vec!["apt:htop"]);
    }

    #[test]
    fn expression_line_unions_into_profile() {
        let d = defs(&[
            ("work", &["apt:vim", "apt:git"]),
            ("gaming", &["apt:git", "apt:steam"]),
            ("security", &["apt:git", "apt:auditd"]),
            // The user's headline example, as a set expression.
            ("locked", &["(work | gaming) & security"]),
        ]);
        // (vim,git,steam) ∩ (git,auditd) = {git}.
        assert_eq!(resolve("locked", &d), vec!["apt:git"]);
    }

    #[test]
    fn function_form_expression_matches_infix() {
        let d = defs(&[
            ("work", &["a", "b"]),
            ("gaming", &["b", "c"]),
            ("security", &["b"]),
            ("f", &["intersect(union(work, gaming), security)"]),
        ]);
        assert_eq!(resolve("f", &d), vec!["b"]);
    }

    #[test]
    fn expression_atoms_can_be_literal_packages() {
        let d = defs(&[("p", &["apt:g++ | cargo:ripgrep"])]);
        // g++ must survive tokenization as a literal atom, not be split on '+'.
        assert_eq!(resolve("p", &d), vec!["apt:g++", "cargo:ripgrep"]);
    }

    #[test]
    fn malformed_expression_is_ignored_not_fatal() {
        // An unbalanced expression is dropped with a warning; other lines still resolve.
        let d = defs(&[("p", &["(work | gaming", "apt:jq"])]);
        assert_eq!(resolve("p", &d), vec!["apt:jq"]);
    }
}
