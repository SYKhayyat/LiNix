// src/app/sync.rs
use crate::app::{LuaHooks, MetricsCollector};
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::config::parser::{load_all_packages, parse_group_file};
use crate::core::{CommandExecutor, PackageCache, Package, PackageSpec, Result, Error, PackageManager, Transaction, StateRegistry};
use crate::core::transaction::PackageOperation;
use crate::utils::progress::ProgressReporter;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Write};
use fs2::FileExt; 
use std::sync::Arc;
use tracing::{info, warn, error};

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

#[derive(Debug, Default, serde::Serialize, serde::Deserialize, Clone)]
pub struct SyncChanges {
    pub to_install: HashMap<String, Vec<PackageSpec>>,
    pub to_remove: HashMap<String, Vec<String>>,
}

impl SyncChanges {
    pub fn is_empty(&self) -> bool { self.to_install.is_empty() && self.to_remove.is_empty() }
    pub fn total_install(&self) -> usize { self.to_install.values().map(|v| v.len()).sum() }
    pub fn total_remove(&self) -> usize { self.to_remove.values().map(|v| v.len()).sum() }
}

impl<'a> SyncEngine<'a> {
    pub fn new(config: &'a Config, registry: Arc<BackendRegistry>, executor: CommandExecutor, cache: Arc<PackageCache>, metrics: MetricsCollector, progress: Arc<dyn ProgressReporter>, hooks: Arc<LuaHooks>) -> Self {
        Self { config, registry, executor, cache, metrics, progress, hooks, use_lockfile: false }
    }

    pub fn with_lockfile(mut self, val: bool) -> Self { self.use_lockfile = val; self }

    /// Normalize version strings to ignore vendor-specific metadata (1.1-build3 vs 1.1)
    fn versions_match(&self, installed: &str, requested: &str) -> bool {
        let normalize = |v: &str| v.split(|c| c == '-' || c == '+' || c == '~' || c == 'v').find(|s| !s.is_empty()).unwrap_or(v).trim();
        normalize(installed) == normalize(requested)
    }

    /// Critical system packages that should never be removed by drift detection.
    fn is_protected_package(&self, name: &str) -> bool {
        let protected = ["linux-image", "kernel", "libc6", "sudo", "bash", "systemd", "winget", "grub", "coreutils", "filesystem", "apt", "pacman", "dnf"];
        protected.iter().any(|&p| name.to_lowercase().contains(p))
    }

    pub async fn sync(&self) -> Result<()> {
		#[cfg(unix)] {
        use fs2::FileExt;
        let stats = fs2::statvfs("/")
            .map_err(|e| Error::Other(format!("Disk check failed: {}", e)))?;
        
        // 500 MB minimum required
        if stats.available_space() < 524288000 { 
            return Err(Error::Other("Sync aborted: Less than 500MB disk space available.".into()));
        }
    }
        // Start Heartbeat to keep sudo authorized during long operations
        let _heartbeat = self.executor.start_sudo_keepalive().await;
        let _ = self.hooks.run_before_sync().await;
        
        let mut state = StateRegistry::load()?;
        let desired = self.calculate_desired_state().await?;
        let mut changes = self.calculate_changes_internal(&desired).await?;

        // Ownership-Aware Drift Detection
        for manager in self.registry.available() {
            let b = manager.name();
            if ["link", "service", "web", "emacs", "github"].contains(&b) { continue; }
            
            let owned_names: Vec<String> = state.packages.iter()
                .filter(|p| p.backend == b)
                .map(|p| p.name.clone())
                .collect();

            let desired_names: HashSet<String> = desired.get(b)
                .map(|specs| specs.iter().map(|s| s.name.clone()).collect())
                .unwrap_or_default();
            
            // Only suggest removing packages that LiNix "owns" but are no longer in config
            let to_purge: Vec<String> = owned_names.into_iter()
                .filter(|name| !desired_names.contains(name) && !self.is_protected_package(name))
                .collect();
            
            if !to_purge.is_empty() {
                changes.to_remove.entry(b.to_string()).or_default().extend(to_purge);
            }
        }

        if changes.is_empty() {
            info!("System is in sync with configuration.");
            return Ok(());
        }

        self.check_binary_conflicts(&changes).await?;
        self.display_changes(&changes);

        if !self.config.yes && !self.confirm_changes()? { return Err(Error::Cancelled); }

        // Journal changes for recovery
        let journal_path = dirs::data_dir().unwrap_or_default().join("linix").join("pending.json");
        if let Some(p) = journal_path.parent() { let _ = tokio::fs::create_dir_all(p).await; }
        tokio::fs::write(&journal_path, serde_json::to_string(&changes)?).await?;

        self.execute_changes(&changes, &mut state).await?;

        let _ = tokio::fs::remove_file(journal_path).await;
        let _ = self.save_lockfile(&desired).await;
        let _ = state.save()?;
        let _ = self.hooks.run_after_sync().await;
        
        self.metrics.print_summary();
        Ok(())
    }

    async fn execute_changes(&self, changes: &SyncChanges, state: &mut StateRegistry) -> Result<()> {
        let mut tx = Transaction::new();

        for (backend, pkgs) in &changes.to_remove {
            if let Some(mgr) = self.registry.get(backend) {
                tx.add(Box::new(PackageOperation {
                    manager: mgr.clone(),
                    packages: pkgs.clone(),
                    is_install: false,
                    sudo: true,
                }));
                for p in pkgs { state.remove(backend, p); }
            }
        }

        for (backend, specs) in &changes.to_install {
            if let Some(mgr) = self.registry.get(backend) {
                let names: Vec<String> = specs.iter().map(|s| s.name.clone()).collect();
                tx.add(Box::new(PackageOperation {
                    manager: mgr.clone(),
                    packages: names.clone(),
                    is_install: true,
                    sudo: true,
                }));
                for n in names { state.add(backend, &n, None); }
            }
        }

        let pb = self.progress.spinner("Applying system changes...");
        let result = tx.execute().await;
        pb.finish();
        
        if result.is_ok() {
            self.metrics.record_install(changes.total_install() as u64);
            self.metrics.record_remove(changes.total_remove() as u64);
        }
        result
    }

    pub async fn calculate_changes_internal(&self, desired: &HashMap<String, Vec<PackageSpec>>) -> Result<SyncChanges> {
        let mut changes = SyncChanges::default();
        let lockfile_path = self.config.groups_dir.join("linix.lock");
        let locked_data: HashMap<String, HashMap<String, String>> = if self.use_lockfile && lockfile_path.exists() {
            serde_json::from_str(&tokio::fs::read_to_string(&lockfile_path).await.unwrap_or_default()).unwrap_or_default()
        } else { HashMap::new() };

        for (backend, specs) in desired {
            if let Some(manager) = self.registry.get(backend) {
                let installed = self.get_installed_full(manager.as_ref()).await?;
                let mut to_install = Vec::new();
                
                for mut spec in specs.clone() {
                    if let Some(v) = locked_data.get(backend).and_then(|m| m.get(&spec.name)) {
                        spec.options.insert("version".into(), v.clone());
                    }

                    let current = installed.iter().find(|p| p.name == spec.name);
                    match current {
                        Some(p) => {
                            if let Some(req_v) = spec.options.get("version") {
                                if let Some(ref inst_v) = p.version {
                                    if !self.versions_match(inst_v, req_v) { to_install.push(spec); }
                                }
                            }
                        }
                        None => to_install.push(spec),
                    }
                }
                if !to_install.is_empty() { changes.to_install.insert(backend.clone(), to_install); }
            }
        }
        Ok(changes)
    }

    async fn calculate_desired_state(&self) -> Result<HashMap<String, Vec<PackageSpec>>> {
        let mut resolved: HashMap<String, Vec<PackageSpec>> = HashMap::new();
        let raw_lines = load_all_packages(&self.config.groups_dir)?;
        let mut queue: VecDeque<String> = raw_lines.into_iter().collect();
        let mut visited_groups = HashSet::new();
        let mut final_lines = Vec::new();

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
            final_lines.push(line);
        }

        for line in final_lines {
            let spec = self.parse_package_spec(&line);
            resolved.entry(spec.backend.clone()).or_default().push(spec);
        }
        Ok(resolved)
    }

    async fn check_binary_conflicts(&self, changes: &SyncChanges) -> Result<()> {
        for (backend, specs) in &changes.to_install {
            for spec in specs {
                let bin = spec.options.get("verify_binary").unwrap_or(&spec.name);
                for other in self.registry.available() {
                    if other.name() == backend { continue; }
                    let inst = self.get_installed_full(other.as_ref()).await?;
                    if inst.iter().any(|p| p.name == *bin) {
                        return Err(Error::Validation(format!("Conflict: '{}' is already in use by {}.", bin, other.name())));
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

    pub async fn heal(&self) -> Result<()> {
        let path = dirs::data_dir().unwrap_or_default().join("linix").join("pending.json");
        if !path.exists() { return Ok(()); }
        warn!("LiNix detected an interrupted transaction. Healing system state...");
        let data = tokio::fs::read_to_string(&path).await?;
        let pending: SyncChanges = serde_json::from_str(&data)?;
        let mut state = StateRegistry::load()?;
        self.execute_changes(&pending, &mut state).await?;
        let _ = tokio::fs::remove_file(path).await;
        state.save()
    }

    pub async fn clean(&self) -> Result<()> {
        let desired = self.calculate_desired_state().await?;
        let mut state = StateRegistry::load()?;
        for manager in self.registry.available() {
            let b = manager.name();
            let owned_names: Vec<String> = state.packages.iter().filter(|p| p.backend == b).map(|p| p.name.clone()).collect();
            let desired_names: HashSet<String> = desired.get(b).map(|s| s.iter().map(|x| x.name.clone()).collect()).unwrap_or_default();
            let to_clean: Vec<String> = owned_names.into_iter().filter(|n| !desired_names.contains(n)).collect();
            
            if !to_clean.is_empty() {
                info!("Cleaning untracked packages from {}: {:?}", b, to_clean);
                manager.remove(&to_clean, true).await?;
                for p in to_clean { state.remove(b, &p); }
            }
        }
        state.save()
    }

    async fn save_lockfile(&self, desired: &HashMap<String, Vec<PackageSpec>>) -> Result<()> {
        let mut locked = HashMap::new();
        for (backend, specs) in desired {
            if let Some(m) = self.registry.get(backend) {
                if let Ok(inst) = m.list_installed().await {
                    let mut map = HashMap::new();
                    for s in specs {
                        if let Some(p) = inst.iter().find(|x| x.name == s.name) {
                            if let Some(v) = &p.version { map.insert(s.name.clone(), v.clone()); }
                        }
                    }
                    locked.insert(backend.clone(), map);
                }
            }
        }
        let _ = tokio::fs::write(self.config.groups_dir.join("linix.lock"), serde_json::to_string_pretty(&locked)?).await;
        Ok(())
    }

    fn parse_package_spec(&self, line: &str) -> PackageSpec {
        let (b_part, rest) = line.split_once(':').unwrap_or(("", line));
        let (n_part, o_part) = rest.split_once('@').unwrap_or((rest, ""));
        let mut options = HashMap::new();
        for pair in o_part.split(',').filter(|s| !s.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, "true"));
            options.insert(k.to_string(), v.to_string());
        }
        let clean_name = n_part.trim_end_matches('/').to_string();
        PackageSpec { name: clean_name, backend: b_part.to_string(), options }
    }

    async fn get_installed_full(&self, mgr: &dyn PackageManager) -> Result<Vec<Package>> {
        let pkgs = mgr.list_installed().await?;
        self.cache.set_installed(mgr.name().to_string(), pkgs.iter().map(|p| p.name.clone()).collect()).await;
        Ok(pkgs)
    }

    fn confirm_changes(&self) -> Result<bool> {
        print!("Proceed with changes? [y/N] ");
        let _ = io::stdout().flush();
        let mut input = String::new();
        let _ = io::stdin().read_line(&mut input).ok();
        Ok(input.trim().to_lowercase() == "y")
    }

    fn display_changes(&self, changes: &SyncChanges) {
        if !changes.to_remove.is_empty() {
            println!("\nREMOVALS:");
            for (b, names) in &changes.to_remove { for n in names { println!("  - [{}] {}", b, n); } }
        }
        if !changes.to_install.is_empty() {
            println!("\nINSTALLATIONS:");
            for (b, specs) in &changes.to_install { for s in specs { println!("  + [{}] {}", b, s.name); } }
        }
    }
}