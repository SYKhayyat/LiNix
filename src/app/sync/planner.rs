use crate::core::{Result, Error, StateRegistry, GraphAction, PackageSpec};
use crate::backends::BackendRegistry;
use crate::config::Config;
use std::collections::{HashMap, VecDeque, HashSet};
use std::sync::Arc;
use std::path::Path;
use tracing::{info, debug, instrument}; // Modernized: Removed unused 'trace'
use version_compare::{Cmp, compare as loose_compare};
use semver::{Version, VersionReq};
use petgraph::graph::NodeIndex;
use petgraph::stable_graph::StableDiGraph;
use petgraph::algo::is_cyclic_directed;
use serde::Serialize;

/// Feature 1: Structured report for dry-run automation.
#[derive(Debug, Serialize, Clone, Default)]
pub struct SyncReport {
    pub install: Vec<ReportEntry>,
    pub remove: Vec<ReportEntry>,
    pub change_count: usize,
}

#[derive(Debug, Serialize, Clone)]
pub struct ReportEntry {
    pub backend: String,
    pub name: String,
    pub version: Option<String>,
    pub source: Option<String>,
}

/// Feature 4: Scope definitions for targeted planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopedFilter {
    None,
    Profile(String),
    Module(String),
    Group(String),
}

/// A high-performance synchronization plan represented as a Directed Acyclic Graph.
#[derive(Debug, Default, Clone)]
pub struct SyncChanges {
    pub graph: StableDiGraph<GraphAction, ()>,
    pub install_map: HashMap<String, NodeIndex>,
    pub removal_tracker: HashSet<String>,
}

impl SyncChanges {
    pub fn is_empty(&self) -> bool {
        self.graph.node_count() == 0
    }

    pub fn total_install(&self) -> usize {
        self.graph.node_weights().filter(|w| matches!(w, GraphAction::Install(_))).count()
    }

    pub fn total_remove(&self) -> usize {
        self.graph.node_weights().filter(|w| matches!(w, GraphAction::Remove { .. })).count()
    }

    pub fn generate_report(&self) -> SyncReport {
        let mut report = SyncReport::default();
        for weight in self.graph.node_weights() {
            match weight {
                GraphAction::Install(spec) => {
                    report.install.push(ReportEntry {
                        backend: spec.backend.clone(),
                        name: spec.name.clone(),
                        version: spec.options.get("version").cloned(),
                        source: spec.options.get("__source").cloned(),
                    });
                }
                GraphAction::Remove { name, backend } => {
                    report.remove.push(ReportEntry {
                        backend: backend.clone(),
                        name: name.clone(),
                        version: None,
                        source: None,
                    });
                }
            }
        }
        report.change_count = report.install.len() + report.remove.len();
        report
    }
}

pub struct ChangePlanner<'a> {
    registry: Arc<BackendRegistry>,
    state: &'a StateRegistry,
    config: &'a Config,
}

impl<'a> ChangePlanner<'a> {
    pub fn new(registry: Arc<BackendRegistry>, state: &'a StateRegistry, config: &'a Config) -> Self {
        Self { registry, state, config }
    }

    #[instrument(skip(self, desired))]
    pub async fn plan(
        &self,
        desired: &HashMap<String, Vec<PackageSpec>>,
        scope: ScopedFilter,
    ) -> Result<SyncChanges> {
        let mut changes = SyncChanges::default();
        let filtered_desired = self.apply_scope_filtering(desired, &scope);
        let expanded_desired = self.expand_transitive_dependencies(&filtered_desired).await?;

        if scope == ScopedFilter::None {
            self.plan_removals_and_expirations(&mut changes).await?;
        }

        let target_specs = self.identify_needed_actions(&expanded_desired).await?;
        self.build_execution_graph(&mut changes, &target_specs).await?;

        if scope == ScopedFilter::None {
            self.plan_drift_removal(&mut changes, &expanded_desired).await?;
        }

        if is_cyclic_directed(&changes.graph) {
            return Err(Error::Transaction("Circular dependency detected in graph construction.".into()));
        }

        Ok(changes)
    }

    fn apply_scope_filtering(&self, desired: &HashMap<String, Vec<PackageSpec>>, scope: &ScopedFilter) -> HashMap<String, Vec<PackageSpec>> {
        if matches!(scope, ScopedFilter::None) { return desired.clone(); }
        let prefix = match scope {
            ScopedFilter::Profile(p) => format!("manifest:{}", p),
            ScopedFilter::Module(m) => format!("module:{}", m),
            ScopedFilter::Group(g) => format!("group:{}", g),
            _ => String::new(),
        };

        let mut filtered = HashMap::new();
        for (backend, specs) in desired {
            let matched: Vec<PackageSpec> = specs.iter()
                .filter(|s| s.options.get("__source").map_or(false, |src| src.contains(&prefix)))
                .cloned().collect();
            if !matched.is_empty() { filtered.insert(backend.clone(), matched); }
        }
        filtered
    }

    async fn plan_removals_and_expirations(&self, changes: &mut SyncChanges) -> Result<()> {
        for (backend, name) in self.state.get_expired_packages() {
            let key = format!("{}:{}", backend, name);
            if changes.removal_tracker.insert(key) {
                info!("Planner: Lease for '{}:{}' has expired. Scheduling removal.", backend, name);
                changes.graph.add_node(GraphAction::Remove { name, backend });
            }
        }

        if self.config.remove_bloatware {
            if let Ok(bloat) = self.load_bloatware().await {
                for entry in bloat {
                    let (backend, name) = entry.split_once(':')
                        .map(|(b, n)| (b.to_string(), n.to_string()))
                        .unwrap_or_else(|| (self.config.default_backend.clone().unwrap_or_else(|| "apt".into()), entry));
                    
                    if self.state.is_managed(&backend, &name) {
                        let key = format!("{}:{}", backend, name);
                        // Resolves E0382: Clone key before inserting into HashSet
                        if changes.removal_tracker.insert(key.clone()) {
                            debug!("Planner: Scheduling bloatware removal: {}", key);
                            changes.graph.add_node(GraphAction::Remove { name, backend });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    async fn identify_needed_actions(&self, expanded: &HashMap<String, PackageSpec>) -> Result<Vec<PackageSpec>> {
        let mut targets = Vec::new();
        for spec in expanded.values() {
            let b_cap = self.registry.get(&spec.backend).ok_or_else(|| Error::BackendNotFound(spec.backend.clone()))?;
            let is_missing = if let Some(q) = b_cap.as_queryable() {
                match q.info(&spec.name).await {
                    Ok(Some(p)) => {
                        if let Some(req_v) = spec.options.get("version") {
                            p.version.as_deref().map_or(true, |inst_v| !self.satisfies_constraint(inst_v, req_v))
                        } else {
                            if spec.backend == "link" && spec.options.get("template") == Some(&"true".into()) {
                                self.template_needs_update(spec).await
                            } else { false }
                        }
                    }
                    _ => true,
                }
            } else { true };
            if is_missing { targets.push(spec.clone()); }
        }
        Ok(targets)
    }

    async fn build_execution_graph(&self, changes: &mut SyncChanges, targets: &[PackageSpec]) -> Result<()> {
        for spec in targets {
            let key = format!("{}:{}", spec.backend, spec.name);
            let idx = changes.graph.add_node(GraphAction::Install(spec.clone()));
            changes.install_map.insert(key, idx);
        }

        for spec in targets {
            let child_key = format!("{}:{}", spec.backend, spec.name);
            let child_idx = *changes.install_map.get(&child_key)
                .ok_or_else(|| Error::Transaction(format!("Consistency Error: Node {} missing.", child_key)))?;

            for req in &spec.requires {
                if let Some(&parent_idx) = changes.install_map.get(req) {
                    changes.graph.add_edge(parent_idx, child_idx, ());
                }
            }
            
            if let Some(b) = self.registry.get(&spec.backend) {
                if let Some(p) = b.as_metadata_provider() {
                    if let Ok(native_deps) = p.get_dependencies(&spec.name).await {
                        for dep in native_deps {
                            let dep_key = format!("{}:{}", spec.backend, dep);
                            if let Some(&parent_idx) = changes.install_map.get(&dep_key) {
                                changes.graph.add_edge(parent_idx, child_idx, ());
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    async fn plan_drift_removal(&self, changes: &mut SyncChanges, expanded: &HashMap<String, PackageSpec>) -> Result<()> {
        for managed in &self.state.packages {
            let key = format!("{}:{}", managed.backend, managed.name);
            if !expanded.contains_key(&key) && !self.config.is_protected(&managed.name) {
                if !changes.install_map.contains_key(&key) && changes.removal_tracker.insert(key.clone()) {
                    debug!("Planner: Scheduling drift removal: {}", key);
                    changes.graph.add_node(GraphAction::Remove { name: managed.name.clone(), backend: managed.backend.clone() });
                }
            }
        }
        Ok(())
    }

    async fn expand_transitive_dependencies(&self, desired: &HashMap<String, Vec<PackageSpec>>) -> Result<HashMap<String, PackageSpec>> {
        let mut expanded = HashMap::new();
        let mut queue = VecDeque::new();
        let mut seen = HashSet::new();
        for specs in desired.values() { for spec in specs { queue.push_back(spec.clone()); } }
        while let Some(spec) = queue.pop_front() {
            let key = format!("{}:{}", spec.backend, spec.name);
            if !seen.insert(key.clone()) { continue; }
            if let Some(b) = self.registry.get(&spec.backend) {
                if let Some(p) = b.as_metadata_provider() {
                    if let Ok(deps) = p.get_dependencies(&spec.name).await {
                        for dep in deps {
                            let dep_key = format!("{}:{}", spec.backend, dep);
                            if seen.contains(&dep_key) { continue; }
                            queue.push_back(PackageSpec { name: dep, backend: spec.backend.clone(), options: HashMap::new(), requires: Vec::new() });
                        }
                    }
                }
            }
            expanded.insert(key, spec);
        }
        Ok(expanded)
    }

    fn satisfies_constraint(&self, installed: &str, constraint: &str) -> bool {
        if constraint == "latest" || constraint == "*" || constraint.is_empty() { return true; }
        if let Ok(req) = VersionReq::parse(constraint) {
            if let Ok(ver) = Version::parse(installed) { return req.matches(&ver); }
        }
        if installed == constraint { return true; }
        match loose_compare(installed, constraint) { Ok(Cmp::Eq) => true, Ok(Cmp::Gt) if constraint.starts_with('>') => true, _ => false }
    }

    async fn template_needs_update(&self, spec: &PackageSpec) -> bool {
        let target = match spec.options.get("target") { Some(s) => Path::new(s), None => return true };
        let source = Path::new(&spec.name);
        if !tokio::fs::try_exists(target).await.unwrap_or(false) { return true; }
        let s_hash = crate::core::security::generate_checksum(source);
        let t_hash = crate::core::security::generate_checksum(target);
        match (s_hash, t_hash) { (Ok(s), Ok(t)) => s != t, _ => true }
    }

    async fn load_bloatware(&self) -> Result<Vec<String>> {
        if !tokio::fs::try_exists(&self.config.bloatware_file).await.unwrap_or(false) { return Ok(Vec::new()); }
        let content = tokio::fs::read_to_string(&self.config.bloatware_file).await?;
        Ok(content.lines().map(|l| l.trim()).filter(|l| !l.is_empty() && !l.starts_with('#')).map(|l| l.to_string()).collect())
    }
}