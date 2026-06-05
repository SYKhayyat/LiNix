use crate::core::{Result, Error, StateRegistry, GraphAction, PackageSpec};
use crate::backends::BackendRegistry;
use crate::config::Config;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::path::Path;
use tracing::{info, debug};
use version_compare::{Cmp, compare as loose_compare};
use semver::{Version, VersionReq};
use petgraph::graph::NodeIndex;
use petgraph::stable_graph::StableDiGraph;
use petgraph::algo::is_cyclic_directed;
use sha2::{Sha256, Digest};
use std::fs::File;
use std::io::{BufReader, Read};

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
        
        // 1. Build a Reachability Map for the entire desired state
        let mut reachable_specs: HashMap<String, PackageSpec> = HashMap::new();
        let mut queue: VecDeque<PackageSpec> = desired.values().flatten().cloned().collect();

        while let Some(spec) = queue.pop_front() {
            let key = format!("{}:{}", spec.backend, spec.name);
            if reachable_specs.contains_key(&key) { continue; }
            reachable_specs.insert(key.clone(), spec.clone());
        }

        // 2. Identify and schedule removal for Expired Leases
        let expired = self.state.get_expired_packages();
        for (backend, name) in expired {
            info!("Planner: Lease for '{}:{}' has expired. Scheduling automatic removal.", backend, name);
            changes.graph.add_node(GraphAction::Remove { name, backend });
        }

        // 3. Load bloatware packages and schedule for removal
        if self.config.remove_bloatware {
            let bloatware = self.load_bloatware().await?;
            for pkg_str in bloatware {
                let (backend, name) = if pkg_str.contains(':') {
                    let parts: Vec<&str> = pkg_str.splitn(2, ':').collect();
                    (parts[0].to_string(), parts[1].to_string())
                } else {
                    (self.config.default_backend.clone().unwrap_or_else(|| "apt".to_string()), pkg_str)
                };
                
                if self.state.is_managed(&backend, &name) {
                    info!("Planner: Bloatware '{}:{}' found in managed state. Scheduling removal.", backend, name);
                    changes.graph.add_node(GraphAction::Remove { name: name.clone(), backend: backend.clone() });
                }
            }
        }

        // 4. Identify required installations and upgrades
        let mut target_specs = Vec::new();
        for spec in reachable_specs.values() {
            if let Some(backend_cap) = self.registry.get(&spec.backend) {
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
        }

        // 5. Build DAG Nodes for Installations
        for spec in &target_specs {
            let key = format!("{}:{}", spec.backend, spec.name);
            let idx = changes.graph.add_node(GraphAction::Install(spec.clone()));
            changes.node_map.insert(key, idx);
        }

        // 6. Resolve DAG Edges (Meta-dependencies)
        for spec in &target_specs {
            let child_key = format!("{}:{}", spec.backend, spec.name);
            let child_idx = *changes.node_map.get(&child_key).unwrap();

            for req_str in &spec.requires {
                if let Some(&parent_idx) = changes.node_map.get(req_str) {
                    changes.graph.add_edge(parent_idx, child_idx, ());
                }
            }
        }

        // 7. Identify Global Drift (Managed packages no longer needed)
        for managed in &self.state.packages {
            let key = format!("{}:{}", managed.backend, managed.name);
            if !reachable_specs.contains_key(&key) && !self.config.is_protected(&managed.name) {
                if !changes.node_map.contains_key(&key) {
                    debug!("Planner: Managed package '{}' has drifted from manifests. Scheduling removal.", key);
                    changes.graph.add_node(GraphAction::Remove { 
                        name: managed.name.clone(), 
                        backend: managed.backend.clone() 
                    });
                }
            }
        }

        // 8. Safety Check
        if is_cyclic_directed(&changes.graph) {
            return Err(Error::Transaction("Circular dependency detected in graph construction".into()));
        }

        Ok(changes)
    }

    async fn template_needs_update(&self, spec: &PackageSpec) -> bool {
        let target_path = spec.options.get("target").map(Path::new);
        let source_path = Path::new(&spec.name);
        
        let target_path = match target_path {
            Some(p) => p,
            None => return true,
        };
        
        if !target_path.exists() { return true; }
        
        let source_hash = self.compute_hash(source_path);
        let target_hash = self.compute_hash(target_path);
        
        source_hash != target_hash
    }
    
    fn compute_hash(&self, path: &Path) -> Option<String> {
        let file = File::open(path).ok()?;
        let mut reader = BufReader::new(file);
        let mut hasher = Sha256::new();
        let mut buffer = [0; 8192];
        
        loop {
            let bytes_read = reader.read(&mut buffer).ok()?;
            if bytes_read == 0 { break; }
            hasher.update(&buffer[..bytes_read]);
        }
        
        Some(hex::encode(hasher.finalize()))
    }

    async fn load_bloatware(&self) -> Result<Vec<String>> {
        let bloatware_path = &self.config.bloatware_file;
        if !bloatware_path.exists() {
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