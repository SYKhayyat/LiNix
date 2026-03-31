// src/app/sync.rs
use crate::app::{LuaHooks, MetricsCollector};
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::config::parser::{load_all_packages, parse_group_file};
use crate::core::{CommandExecutor, PackageCache, Package, PackageSpec, Result, Error, PackageManager};
use crate::utils::progress::ProgressReporter;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Write};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{info, warn};

pub struct SyncEngine<'a> {
    config: &'a Config,
    registry: Arc<BackendRegistry>,
    executor: CommandExecutor,
    cache: Arc<PackageCache>,
    metrics: MetricsCollector,
    progress: Arc<dyn ProgressReporter>,
    hooks: Arc<LuaHooks>,
    use_lockfile: bool,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct SyncChanges {
    pub to_install: HashMap<String, Vec<PackageSpec>>,
    pub to_remove: HashMap<String, Vec<String>>,
}

impl SyncChanges {
    pub fn is_empty(&self) -> bool { self.to_install.is_empty() && self.to_remove.is_empty() }
    pub fn total_install(&self) -> usize { self.to_install.values().map(|v| v.len()).sum() }
}

impl<'a> SyncEngine<'a> {
    pub fn new(config: &'a Config, registry: Arc<BackendRegistry>, executor: CommandExecutor, cache: Arc<PackageCache>, metrics: MetricsCollector, progress: Arc<dyn ProgressReporter>, hooks: Arc<LuaHooks>) -> Self {
        Self { config, registry, executor, cache, metrics, progress, hooks, use_lockfile: false }
    }

    pub fn with_lockfile(mut self, val: bool) -> Self {
        self.use_lockfile = val;
        self
    }

    pub async fn sync(&self) -> Result<()> {
        let _ = self.hooks.run_before_sync().await;
        
        if let Ok(drift) = self.detect_drift().await {
            if !drift.is_empty() {
                warn!("System Drift Detected! Manual installs found outside config:");
                for (b, pkgs) in drift { warn!("[{}] {:?}", b, pkgs); }
            }
        }
        
        let desired = self.calculate_desired_state().await?;
        let mut changes = self.calculate_changes(desired.clone()).await?;

        if self.config.remove_bloatware && self.config.bloatware_file.exists() {
            let bloat_list = parse_group_file(&self.config.bloatware_file)?;
            let system_default = self.detect_system_backend().await;
            for line in bloat_list {
                let spec = self.parse_package_spec(&line);
                let b = if spec.backend.is_empty() { system_default.clone() } else { spec.backend };
                if let Some(mgr) = self.registry.get(&b) {
                    let inst = self.get_installed_full(mgr.as_ref()).await?;
                    if inst.iter().any(|p| p.name == spec.name) {
                        changes.to_remove.entry(b).or_default().push(spec.name);
                    }
                }
            }
        }
        
        if changes.is_empty() {
            info!("System is in sync.");
            return Ok(());
        }

        self.check_binary_conflicts(&changes).await?;

        let journal_path = dirs::data_dir().unwrap_or_default().join("linix").join("pending.json");
        if let Some(p) = journal_path.parent() { let _ = tokio::fs::create_dir_all(p).await; }
        tokio::fs::write(&journal_path, serde_json::to_string(&changes)?).await?;

        self.display_changes(&changes);
        if !self.config.yes && !self.confirm_changes()? { return Err(Error::Cancelled); }

        self.execute_changes(&changes).await?;

        let _ = tokio::fs::remove_file(journal_path).await;
        let _ = self.save_lockfile(&desired).await;
        let _ = self.hooks.run_after_sync().await;
        Ok(())
    }

    async fn execute_changes(&self, changes: &SyncChanges) -> Result<()> {
        let mut system_backends = Vec::new();
        let mut parallel_backends = Vec::new();

        for b in changes.to_install.keys() {
            match b.as_str() {
                "apt" | "pacman" | "dnf" | "zypper" | "apk" => system_backends.push(b),
                _ => parallel_backends.push(b),
            }
        }

        for b_name in system_backends {
            if let Some(mgr) = self.registry.get(b_name) {
                let specs = &changes.to_install[b_name];
                let pb = self.progress.start(specs.len() as u64, &format!("Syncing {}", b_name));
                mgr.install_with_options(specs, true).await?;
                pb.finish();
            }
        }

        let semaphore = Arc::new(Semaphore::new(self.config.max_parallel));
        let mut tasks = tokio::task::JoinSet::new();

        for b_name in parallel_backends {
            let b_name = b_name.clone();
            let manager = self.registry.get(&b_name).unwrap();
            let specs = changes.to_install[&b_name].clone();
            let sem = semaphore.clone();
            let metrics = self.metrics.clone();
            let executor = self.executor.clone();
            let progress = self.progress.clone();

            tasks.spawn(async move {
                let _permit = sem.acquire().await;
                let pb = progress.start(specs.len() as u64, &format!("Syncing {}", b_name));
                manager.install_with_options(&specs, true).await?;
                for spec in &specs {
                    let bin = spec.options.get("verify_binary").unwrap_or(&spec.name);
                    if !executor.command_exists(bin).await {
                        warn!("Success reported by {}, but binary '{}' not found", b_name, bin);
                    }
                    pb.inc(1);
                }
                metrics.record_install(specs.len() as u64);
                pb.finish();
                Result::Ok(())
            });
        }
        while let Some(res) = tasks.join_next().await { res.map_err(|e| Error::Other(e.to_string()))??; }
        Ok(())
    }

    pub async fn detect_drift(&self) -> Result<Vec<(String, Vec<String>)>> {
        let desired_map = self.calculate_desired_state().await?;
        let mut drift = Vec::new();
        for manager in self.registry.available() {
            let backend = manager.name().to_string();
            if ["link", "service", "web", "emacs"].contains(&backend.as_str()) { continue; }
            let desired_names: HashSet<String> = desired_map.get(&backend)
                .map(|specs| specs.iter().map(|s| s.name.clone()).collect()).unwrap_or_default();
            let installed = manager.list_installed().await?;
            let untracked: Vec<String> = installed.into_iter()
                .filter(|p| !desired_names.contains(&p.name)).map(|p| p.name).collect();
            if !untracked.is_empty() { drift.push((backend, untracked)); }
        }
        Ok(drift)
    }

    pub async fn find_unmanaged(&self) -> Result<Vec<(String, Vec<String>)>> {
        self.detect_drift().await
    }

    pub async fn clean(&self) -> Result<()> {
        let list = self.find_unmanaged().await?;
        if list.is_empty() { return Ok(()); }
        for (backend_name, packages) in list {
            if let Some(manager) = self.registry.get(&backend_name) {
                manager.remove(&packages, true).await?;
                self.metrics.record_remove(packages.len() as u64);
            }
        }
        Ok(())
    }

    async fn calculate_desired_state(&self) -> Result<HashMap<String, Vec<PackageSpec>>> {
        let mut resolved: HashMap<String, Vec<PackageSpec>> = HashMap::new();
        let raw_lines = load_all_packages(&self.config.groups_dir)?;
        let mut queue: VecDeque<String> = raw_lines.into_iter().collect();
        let mut expanded_specs = Vec::new();
        let mut visited_groups = HashSet::new();

        while let Some(line) = queue.pop_front() {
            if let Some(group_name) = line.strip_prefix("group:") {
                if visited_groups.insert(group_name.to_string()) {
                    let group_file = self.config.groups_dir.join(format!("{}.txt", group_name));
                    if group_file.exists() {
                        for pkg in parse_group_file(&group_file)? { queue.push_back(pkg); }
                    }
                }
                continue;
            }
            expanded_specs.push(line);
        }

        for line in expanded_specs {
            let mut spec = self.parse_package_spec(&line);
            if spec.backend.is_empty() {
                if let Some(found) = self.discover_backend(&spec.name).await { spec.backend = found; }
                else { continue; }
            }
            resolved.entry(spec.backend.clone()).or_default().push(spec);
        }
        Ok(resolved)
    }

    async fn discover_backend(&self, name: &str) -> Option<String> {
        for backend_name in &self.config.backend_priority {
            if let Some(manager) = self.registry.get(backend_name) {
                if !manager.is_available() { continue; }
                if let Ok(results) = manager.search(name).await {
                    if results.iter().any(|p| p.name == name) { return Some(backend_name.clone()); }
                }
            }
        }
        None
    }

    pub async fn calculate_changes(&self, desired: HashMap<String, Vec<PackageSpec>>) -> Result<SyncChanges> {
        let mut changes = SyncChanges::default();
        for (backend, specs) in desired {
            if let Some(manager) = self.registry.get(&backend) {
                let installed = self.get_installed_full(manager.as_ref()).await?;
                let mut to_install = Vec::new();
                for spec in specs {
                    let current = installed.iter().find(|p| p.name == spec.name);
                    match current {
                        Some(p) => {
                            if let Some(req_v) = spec.options.get("version") {
                                if p.version.as_ref() != Some(req_v) { to_install.push(spec); }
                            }
                        }
                        None => to_install.push(spec),
                    }
                }
                if !to_install.is_empty() { changes.to_install.insert(backend, to_install); }
            }
        }
        Ok(changes)
    }

    fn parse_package_spec(&self, line: &str) -> PackageSpec {
        let (b_part, rest) = line.split_once(':').unwrap_or(("", line));
        let (n_part, o_part) = rest.split_once('@').unwrap_or((rest, ""));
        let mut options = HashMap::new();
        for pair in o_part.split(',').filter(|s| !s.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, "true"));
            options.insert(k.to_string(), v.to_string());
        }
        PackageSpec { name: n_part.to_string(), backend: b_part.to_string(), options }
    }

    async fn detect_system_backend(&self) -> String {
        if cfg!(target_os = "windows") { return "winget".into(); }
        if cfg!(target_os = "macos") { return "brew".into(); }
        if self.executor.command_exists("apt").await { return "apt".into(); }
        if self.executor.command_exists("pacman").await { return "pacman".into(); }
        "apt".to_string()
    }

    async fn get_installed_full(&self, manager: &dyn PackageManager) -> Result<Vec<Package>> {
        if let Some(cached) = self.cache.get_installed(manager.name()).await {
            return Ok(cached.into_iter().map(|n| Package::new(n, manager.name())).collect());
        }
        let pkgs = manager.list_installed().await?;
        let names: Vec<String> = pkgs.iter().map(|p| p.name.clone()).collect();
        self.cache.set_installed(manager.name().to_string(), names).await;
        Ok(pkgs)
    }

    async fn check_binary_conflicts(&self, changes: &SyncChanges) -> Result<()> {
        for (backend, specs) in &changes.to_install {
            if let Some(mgr) = self.registry.get(backend) {
                for spec in specs {
                    let bin_name = spec.options.get("verify_binary").unwrap_or(&spec.name);
                    if self.executor.command_exists(bin_name).await {
                        let inst = self.get_installed_full(mgr.as_ref()).await?;
                        if !inst.iter().any(|p| p.name == spec.name) {
                            warn!("Warning: Binary '{}' already exists and might be shadowed by {} install.", bin_name, backend);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn export_system(&self) -> Result<String> {
        let mut output = String::from("# LiNix System Export\n\n");
        for manager in self.registry.available() {
            let pkgs = manager.list_manual().await?;
            if !pkgs.is_empty() {
                output.push_str(&format!("# From {}\n", manager.name()));
                for p in pkgs { output.push_str(&format!("{}:{}\n", manager.name(), p.name)); }
                output.push('\n');
            }
        }
        Ok(output)
    }

    async fn save_lockfile(&self, desired: &HashMap<String, Vec<PackageSpec>>) -> Result<()> {
        let mut locked: HashMap<String, HashMap<String, String>> = HashMap::new();
        for manager in self.registry.available() {
            if let Some(specs) = desired.get(manager.name()) {
                if let Ok(inst) = manager.list_installed().await {
                    let mut map = HashMap::new();
                    for s in specs {
                        if let Some(p) = inst.iter().find(|x| x.name == s.name) {
                            if let Some(v) = &p.version { map.insert(s.name.clone(), v.clone()); }
                        }
                    }
                    locked.insert(manager.name().to_string(), map);
                }
            }
        }
        let _ = tokio::fs::write(self.config.groups_dir.join("linix.lock"), serde_json::to_string_pretty(&locked)?).await;
        Ok(())
    }

    pub async fn heal(&self) -> Result<()> {
        let path = dirs::data_dir().unwrap_or_default().join("linix").join("pending.json");
        if !path.exists() { return Ok(()); }
        let data = tokio::fs::read_to_string(&path).await?;
        let pending: SyncChanges = serde_json::from_str(&data).map_err(|e| Error::Other(e.to_string()))?;
        self.execute_changes(&pending).await?;
        let _ = tokio::fs::remove_file(path).await;
        Ok(())
    }

    fn confirm_changes(&self) -> Result<bool> {
        print!("Proceed? [y/N] ");
        let _ = io::stdout().flush();
        let mut input = String::new();
        let _ = io::stdin().read_line(&mut input).ok();
        Ok(input.trim().to_lowercase() == "y")
    }

    fn display_changes(&self, changes: &SyncChanges) {
        if !changes.to_remove.is_empty() {
            println!("\nRemoval Plan:");
            for (b, names) in &changes.to_remove { println!("  - [{}] {:?}", b, names); }
        }
        if !changes.to_install.is_empty() {
            println!("\nInstallation Plan:");
            for (b, specs) in &changes.to_install { for s in specs { println!("  + [{}] {}", b, s.name); } }
        }
    }
}