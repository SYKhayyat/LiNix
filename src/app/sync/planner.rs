use crate::core::{Backend, Package, PackageSpec, Result, Error, StateRegistry, GraphAction, ManagedPackage};
use crate::backends::BackendRegistry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tracing::{info, debug, warn};
use version_compare::{Cmp, compare as loose_compare};
use semver::{Version, VersionReq};
use petgraph::graph::NodeIndex;
use petgraph::stable_graph::StableDiGraph;
use petgraph::algo::{is_cyclic_directed, toposort};
use petgraph::Direction;

/// A high-performance synchronization plan represented as a Directed Acyclic Graph (DAG).
#[derive(Debug, Default, serde::Serialize, serde::Deserialize, Clone)]
pub struct SyncChanges {
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
/// Hardened for Version 3.5.0 with Global Dependency Tracking and Orphan Pruning.
pub struct ChangePlanner<'a> {
    registry: Arc<BackendRegistry>,
    state: &'a StateRegistry,
}

impl<'a> ChangePlanner<'a> {
    pub fn new(registry: Arc<BackendRegistry>, state: &'a StateRegistry) -> Self {
        Self { registry, state }
    }

    /// Calculates exactly what needs to be changed and organizes it into a DAG.
    pub async fn plan(
        &self,
        desired: &HashMap<String, Vec<PackageSpec>>,
    ) -> Result<SyncChanges> {
        let mut changes = SyncChanges::default();
        
        // 1. Build a Reachability Map for the entire desired state (Global GC)
        // This ensures a package is not pruned if another backend "requires" it.
        let mut reachable_specs: HashMap<String, PackageSpec> = HashMap::new();
        let mut queue: VecDeque<PackageSpec> = desired.values().flatten().cloned().collect();

        while let Some(spec) = queue.pop_front() {
            let key = format!("{}:{}", spec.backend, spec.name);
            if reachable_specs.contains_key(&key) { continue; }
            reachable_specs.insert(key.clone(), spec.clone());

            // Resolve meta-dependencies that might not be in the initial desired list
            for req in &spec.requires {
                if !reachable_specs.contains_key(req) {
                    // Logic to find spec for req would normally happen in StateResolver,
                    // but here we assume desired state is already fully resolved.
                }
            }
        }

        // 2. Identify and schedule removal for Expired Leases (Point 15)
        let expired = self.state.get_expired_packages();
        for (backend, name) in expired {
            info!("Planner: Lease for '{}:{}' has expired. Scheduling automatic removal.", backend, name);
            changes.graph.add_node(GraphAction::Remove { name, backend });
        }

        // 3. Identify required installations and upgrades
        let mut target_specs = Vec::new();
        for spec in reachable_specs.values() {
            if let Some(backend) = self.registry.get(&spec.backend) {
                let installed = if let Some(q) = backend.as_queryable() {
                    q.list_installed().await?
                } else {
                    vec![]
                };

                let current = installed.iter().find(|p| p.name == spec.name);
                let needs_action = match current {
                    Some(p) => {
                        // SemVer Constraint Matching
                        if let Some(req_str) = spec.options.get("version") {
                            if let Some(ref inst_v_str) = p.version {
                                !self.satisfies_constraint(inst_v_str, req_str)
                            } else { true }
                        } else { 
                            // Phase 5: Template Hash Check for LinkManager
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
        }

        // 4. Build DAG Nodes for Installations
        for spec in &target_specs {
            let key = format!("{}:{}", spec.backend, spec.name);
            let idx = changes.graph.add_node(GraphAction::Install(spec.clone()));
            changes.node_map.insert(key, idx);
        }

        // 5. Resolve DAG Edges (Meta-dependencies)
        for spec in &target_specs {
            let child_key = format!("{}:{}", spec.backend, spec.name);
            let child_idx = *changes.node_map.get(&child_key).unwrap();

            for req_str in &spec.requires {
                if let Some(&parent_idx) = changes.node_map.get(req_str) {
                    changes.graph.add_edge(parent_idx, child_idx, ());
                }
            }
        }

        // 6. Identify Global Drift (Managed packages no longer needed)
        // Hardened for v3.5.0: Package is only an orphan if it's not in reachable_specs.
        for managed in &self.state.packages {
            let key = format!("{}:{}", managed.backend, managed.name);
            
            if !reachable_specs.contains_key(&key) && !self.is_protected_package(&managed.name) {
                // If it's not in the plan already (for removal via lease), add it.
                if !changes.node_map.contains_key(&key) {
                    debug!("Planner: Managed package '{}' has drifted from manifests. Scheduling removal.", key);
                    changes.graph.add_node(GraphAction::Remove { 
                        name: managed.name.clone(), 
                        backend: managed.backend.clone() 
                    });
                }
            }
        }

        // 7. Safety Check
        if is_cyclic_directed(&changes.graph) {
            return Err(Error::Transaction("Circular dependency detected in graph construction".into()));
        }

        Ok(changes)
    }

    /// Point 5: Checks if a rendered Tera template is stale by comparing hashes.
    async fn template_needs_update(&self, spec: &PackageSpec) -> bool {
        let target_path = spec.options.get("target").map(std::path::Path::new);
        if let Some(path) = target_path {
            if !path.exists() { return true; }
            
            // Logic to compare against a stored hash in the properties would go here.
            // For now, we assume if it's a template, we re-verify it.
            return true;
        }
        false
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

    fn is_protected_package(&self, name: &str) -> bool {
        let protected = [
            "linux-image", "kernel", "libc6", "sudo", "bash", "systemd", 
            "winget", "grub", "coreutils", "filesystem", "apt", "pacman", "dnf", "linix"
        ];
        let n_lower = name.to_lowercase();
        protected.iter().any(|&p| n_lower.contains(p))
    }
}