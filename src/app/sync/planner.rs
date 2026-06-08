use crate::core::{Result, Error, StateRegistry, GraphAction, PackageSpec};
use crate::backends::BackendRegistry;
use crate::config::Config;
use std::collections::{HashMap, VecDeque, HashSet};
use std::sync::Arc;
use std::path::Path;
use tracing::{info, debug};
use version_compare::{Cmp, compare as loose_compare};
use semver::{Version, VersionReq};
use petgraph::graph::NodeIndex;
use petgraph::stable_graph::StableDiGraph;
use petgraph::algo::is_cyclic_directed;
use sha2::{Sha256, Digest};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, BufReader};

/// A high-performance synchronization plan represented as a Directed Acyclic Graph (DAG).
#[derive(Debug, Default, serde::Serialize, serde::Deserialize, Clone)]
pub struct SyncChanges {
    #[serde(skip)]
    pub graph: StableDiGraph<GraphAction, ()>,
    #[serde(skip)]
    pub node_map: HashMap<String, NodeIndex>,
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
}

/// The brain of the LiNix engine. Calculates the delta between current and desired state.
/// 
/// Hardened for Phase 1.1: Implements Recursive Native Dependency Resolution.
/// Refactored for Technical Debt Cleanup: Split monolithic plan() into logical sub-tasks.
pub struct ChangePlanner<'a> {
    registry: Arc<BackendRegistry>,
    state: &'a StateRegistry,
    config: &'a Config,
}

impl<'a> ChangePlanner<'a> {
    pub fn new(registry: Arc<BackendRegistry>, state: &'a StateRegistry, config: &'a Config) -> Self {
        Self { registry, state, config }
    }

    /// Calculates exactly what needs to be changed and organizes it into a DAG.
    pub async fn plan(
        &self,
        desired: &HashMap<String, Vec<PackageSpec>>,
    ) -> Result<SyncChanges> {
        let mut changes = SyncChanges::default();
        
        // 1. Recursive Transitive Dependency Expansion
        let expanded_desired = self.expand_transitive_dependencies(desired).await?;

        // 2. Process Expirations and Bloatware
        self.plan_removals_and_expirations(&mut changes).await?;

        // 3. Identify required installations and upgrades
        let target_specs = self.identify_needed_actions(&expanded_desired).await?;

        // 4. Build DAG Nodes and Edges
        self.build_execution_graph(&mut changes, &target_specs).await?;

        // 5. Identify Global Drift (Managed packages no longer needed)
        self.plan_drift_removal(&mut changes, &expanded_desired).await?;

        // 6. Safety Check
        if is_cyclic_directed(&changes.graph) {
            return Err(Error::Transaction("Circular dependency detected in graph construction".into()));
        }

        Ok(changes)
    }

    async fn plan_removals_and_expirations(&self, changes: &mut SyncChanges) -> Result<()> {
        // Expired Leases
        let expired = self.state.get_expired_packages();
        for (backend, name) in expired {
            info!("Planner: Lease for '{}:{}' has expired. Scheduling automatic removal.", backend, name);
            changes.graph.add_node(GraphAction::Remove { name, backend });
        }

        // Bloatware removal
        if self.config.remove_bloatware {
            let bloatware = self.load_bloatware().await?;
            for pkg_str in bloatware {
                let (backend, name) = if let Some((b, n)) = pkg_str.split_once(':') {
                    (b.to_string(), n.to_string())
                } else {
                    (self.config.default_backend.clone().unwrap_or_else(|| "apt".to_string()), pkg_str)
                };
                
                if self.state.is_managed(&backend, &name) {
                    info!("Planner: Bloatware '{}:{}' found in managed state. Scheduling removal.", backend, name);
                    changes.graph.add_node(GraphAction::Remove { name: name.clone(), backend: backend.clone() });
                }
            }
        }
        Ok(())
    }

    async fn identify_needed_actions(&self, expanded_desired: &HashMap<String, PackageSpec>) -> Result<Vec<PackageSpec>> {
        let mut target_specs = Vec::new();
        for spec in expanded_desired.values() {
            let backend_cap = self.registry.get(&spec.backend)
                .ok_or_else(|| Error::BackendNotFound(spec.backend.clone()))?;

            let installed = if let Some(q) = backend_cap.as_queryable() {
                q.list_installed().await?
            } else {
                vec![]
            };

            let current = installed.iter().find(|p| p.name == spec.name);
            let needs_action = match current {
                Some(p) => {
                    if let Some(req_str) = spec.options.get("version") {
                        if let Some(ref inst_v_str) = p.version {
                            !self.satisfies_constraint(inst_v_str, req_str)
                        } else { true }
                    } else { 
                        if spec.backend == "link" && spec.options.get("template") == Some(&"true".to_string()) {
                            self.template_needs_update(spec).await
                        } else {
                            false 
                        }
                    }
                }
                None => true,
            };

            if needs_action {
                target_specs.push(spec.clone());
            }
        }
        Ok(target_specs)
    }

    async fn build_execution_graph(&self, changes: &mut SyncChanges, target_specs: &[PackageSpec]) -> Result<()> {
        // Build Nodes
        for spec in target_specs {
            let key = format!("{}:{}", spec.backend, spec.name);
            let idx = changes.graph.add_node(GraphAction::Install(spec.clone()));
            changes.node_map.insert(key, idx);
        }

        // Resolve Edges
        for spec in target_specs {
            let child_key = format!("{}:{}", spec.backend, spec.name);
            let child_idx = *changes.node_map.get(&child_key)
                .ok_or_else(|| Error::Transaction(format!("Missing node in map: {}", child_key)))?;

            for req_str in &spec.requires {
                if let Some(&parent_idx) = changes.node_map.get(req_str) {
                    changes.graph.add_edge(parent_idx, child_idx, ());
                }
            }
            
            let backend_cap = self.registry.get(&spec.backend).unwrap();
            if let Some(provider) = backend_cap.as_metadata_provider() {
                let native_deps = provider.get_dependencies(&spec.name).await?;
                for dep_name in native_deps {
                    let dep_key = format!("{}:{}", spec.backend, dep_name);
                    if let Some(&parent_idx) = changes.node_map.get(&dep_key) {
                        changes.graph.add_edge(parent_idx, child_idx, ());
                    }
                }
            }
        }
        Ok(())
    }

    async fn plan_drift_removal(&self, changes: &mut SyncChanges, expanded_desired: &HashMap<String, PackageSpec>) -> Result<()> {
        for managed in &self.state.packages {
            let key = format!("{}:{}", managed.backend, managed.name);
            if !expanded_desired.contains_key(&key) && !self.config.is_protected(&managed.name) {
                if !changes.node_map.contains_key(&key) {
                    debug!("Planner: Managed package '{}' has drifted from manifests. Scheduling removal.", key);
                    changes.graph.add_node(GraphAction::Remove { 
                        name: managed.name.clone(), 
                        backend: managed.backend.clone() 
                    });
                }
            }
        }
        Ok(())
    }

    /// Phase 1.1: Recursively expands the requested package set to include backend-native dependencies.
    pub(crate) async fn expand_transitive_dependencies(&self, desired: &HashMap<String, Vec<PackageSpec>>) -> Result<HashMap<String, PackageSpec>> {
        let mut expanded = HashMap::new();
        let mut queue = VecDeque::new();
        let mut seen = HashSet::new();

        for specs in desired.values() {
            for spec in specs {
                queue.push_back(spec.clone());
            }
        }

        while let Some(spec) = queue.pop_front() {
            let key = format!("{}:{}", spec.backend, spec.name);
            if !seen.insert(key.clone()) { 
                continue; 
            }

            if let Some(backend_cap) = self.registry.get(&spec.backend) {
                if let Some(provider) = backend_cap.as_metadata_provider() {
                    let deps = provider.get_dependencies(&spec.name).await?;
                    for dep_name in deps {
                        let dep_key = format!("{}:{}", spec.backend, dep_name);
                        
                        if seen.contains(&dep_key) {
                            debug!("Planner: Native dependency cycle detected for '{}'. Skipping recursion.", dep_key);
                            continue;
                        }

                        let dep_spec = PackageSpec {
                            name: dep_name,
                            backend: spec.backend.clone(),
                            options: HashMap::new(),
                            requires: Vec::new(),
                        };
                        queue.push_back(dep_spec);
                    }
                }
            }

            expanded.insert(key, spec);
        }

        Ok(expanded)
    }

    pub(crate) async fn template_needs_update(&self, spec: &PackageSpec) -> bool {
        let target_path_str = match spec.options.get("target") {
            Some(s) => s,
            None => return true,
        };
        
        let target_path = Path::new(target_path_str);
        let source_path = Path::new(&spec.name);
        
        if !tokio::fs::try_exists(target_path).await.unwrap_or(false) { return true; }
        
        let source_hash = self.compute_hash(source_path).await;
        let target_hash = self.compute_hash(target_path).await;
        
        source_hash != target_hash
    }
    
    async fn compute_hash(&self, path: &Path) -> Option<String> {
        let file = File::open(path).await.ok()?;
        let mut reader = BufReader::new(file);
        let mut hasher = Sha256::new();
        let mut buffer = [0; 8192];
        
        loop {
            let bytes_read = reader.read(&mut buffer).await.ok()?;
            if bytes_read == 0 { break; }
            hasher.update(&buffer[..bytes_read]);
        }
        
        Some(hex::encode(hasher.finalize()))
    }

    async fn load_bloatware(&self) -> Result<Vec<String>> {
        let bloatware_path = &self.config.bloatware_file;
        if !tokio::fs::try_exists(bloatware_path).await.unwrap_or(false) {
            return Ok(Vec::new());
        }
        
        let content = tokio::fs::read_to_string(bloatware_path).await?;
        Ok(content.lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.to_string())
            .collect())
    }

    fn satisfies_constraint(&self, installed: &str, constraint: &str) -> bool {
        if constraint == "latest" || constraint == "*" {
            return true;
        }

        if let Ok(req) = VersionReq::parse(constraint) {
            if let Ok(ver) = Version::parse(installed) {
                return req.matches(&ver);
            }
        }

        if installed == constraint {
            return true;
        }

        match loose_compare(installed, constraint) {
            Ok(Cmp::Eq) => true,
            Ok(Cmp::Gt) if constraint.starts_with('>') => true,
            _ => false,
        }
    }
}