// src/app/sync.rs
use crate::app::{LuaHooks, MetricsCollector};
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::config::parser::load_all_packages;
use crate::core::{CommandExecutor, PackageCache, Package, PackageSpec, Result, Error};
use crate::utils::progress::ProgressReporter;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Write};
use std::fs;
use tracing::{info, warn};

pub struct SyncEngine<'a> {
    config: &'a Config,
    registry: &'a BackendRegistry,
    executor: &'a CommandExecutor,
    cache: &'a PackageCache,
    metrics: &'a MetricsCollector,
    progress: &'a dyn ProgressReporter,
    hooks: &'a LuaHooks,
    use_lockfile: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SyncChanges {
    pub to_install: HashMap<String, Vec<PackageSpec>>,
    pub to_remove: HashMap<String, Vec<String>>,
}

impl SyncChanges {
    pub fn is_empty(&self) -> bool {
        self.to_install.is_empty() && self.to_remove.is_empty()
    }
    pub fn total_install(&self) -> usize {
        self.to_install.values().map(|v| v.len()).sum()
    }
}

impl<'a> SyncEngine<'a> {
    pub fn new(config: &'a Config, registry: &'a BackendRegistry, executor: &'a CommandExecutor, cache: &'a PackageCache, metrics: &'a MetricsCollector, progress: &'a dyn ProgressReporter, hooks: &'a LuaHooks) -> Self {
        Self { config, registry, executor, cache, metrics, progress, hooks, use_lockfile: false }
    }

    pub fn with_lockfile(mut self, val: bool) -> Self {
        self.use_lockfile = val;
        self
    }

    /// Resumes an interrupted transaction from the journal file
    pub async fn heal(&self) -> Result<()> {
        let path = dirs::data_dir().unwrap_or_default().join("linix").join("pending.json");
        if !path.exists() { return Ok(()); }
        
        info!("Interrupted transaction found. Healing system state...");
        let data = fs::read_to_string(&path)?;
        let pending: SyncChanges = serde_json::from_str(&data).map_err(|e| Error::Other(e.to_string()))?;
        
        self.execute_changes(&pending).await?;
        let _ = fs::remove_file(path);
        info!("System healed successfully.");
        Ok(())
    }

    /// Performs a full system synchronization based on declarative config
    pub async fn sync(&self) -> Result<()> {
        let _ = self.hooks.run_before_sync().await;
        let desired = self.load_desired_packages()?;
        let changes = self.calculate_changes().await?;
        
        if changes.is_empty() {
            info!("System is in sync.");
            let _ = self.hooks.run_after_sync().await;
            return Ok(());
        }

        self.check_binary_conflicts(&changes).await?;
        self.display_changes(&changes);

        if !self.config.yes && !self.confirm_changes()? { return Err(Error::Cancelled); }

        // Start Transactional Journal
        let journal_path = dirs::data_dir().unwrap_or_default().join("linix").join("pending.json");
        if let Some(p) = journal_path.parent() { let _ = fs::create_dir_all(p); }
        fs::write(&journal_path, serde_json::to_string(&changes).unwrap_or_default())?;

        self.execute_changes(&changes).await?;

        // End Transaction
        let _ = fs::remove_file(journal_path);
        let _ = self.save_lockfile(&desired).await;
        let _ = self.hooks.run_after_sync().await;
        Ok(())
    }

    /// Identifies packages currently installed that aren't in config (Fixes E0599)
    pub async fn find_unmanaged(&self) -> Result<Vec<(String, Vec<String>)>> {
        let desired = self.load_desired_packages()?;
        let mut unmanaged = Vec::new();

        for manager in self.registry.available() {
            let backend = manager.name().to_string();
            let desired_names: HashSet<String> = desired.get(&backend)
                .map(|s| s.iter().map(|x| x.name.clone()).collect())
                .unwrap_or_default();
            
            let installed = self.get_installed_full(manager.as_ref()).await?;
            let list: Vec<String> = installed.into_iter()
                .filter(|p| !desired_names.contains(&p.name))
                .map(|p| p.name)
                .collect();

            if !list.is_empty() { unmanaged.push((backend, list)); }
        }
        Ok(unmanaged)
    }

    /// Triggers cleanup for unmanaged packages (Fixes E0599)
    pub async fn clean(&self) -> Result<()> {
        let list = self.find_unmanaged().await?;
        if list.is_empty() {
            info!("No unmanaged packages detected.");
            return Ok(());
        }

        println!("\n=== Unmanaged Packages (Not in config) ===");
        for (b, pkgs) in &list { println!("[{}] {:?}", b, pkgs); }

        if !self.config.yes && !self.confirm_changes()? { return Err(Error::Cancelled); }

        for (backend_name, packages) in list {
            if let Some(manager) = self.registry.get(&backend_name) {
                manager.remove(&packages, true).await?;
                self.metrics.record_remove(packages.len() as u64);
            }
        }
        Ok(())
    }

    async fn execute_changes(&self, changes: &SyncChanges) -> Result<()> {
        let mut backends: Vec<String> = changes.to_install.keys().cloned().collect();
        backends.sort_by_key(|name| self.get_backend_priority(name));

        for b in backends {
            if let Some(specs) = changes.to_install.get(&b) {
                if let Some(manager) = self.registry.get(&b) {
                    let handle = self.progress.start(specs.len() as u64, &format!("Syncing {}...", b));
                    manager.install_with_options(specs, true).await?;
                    
                    for s in specs {
                        if let Some(bin) = s.options.get("verify_binary") {
                            if !self.executor.command_exists(bin).await { warn!("Binary {} missing after install.", bin); }
                        }
                        handle.inc(1);
                    }
                    self.metrics.record_install(specs.len() as u64);
                    handle.finish();
                }
            }
        }
        Ok(())
    }

    fn get_backend_priority(&self, name: &str) -> i32 {
        match name { "apt" | "dnf" | "pacman" | "apk" => 1, "brew" | "flatpak" | "snap" => 2, _ => 3 }
    }

    async fn check_binary_conflicts(&self, changes: &SyncChanges) -> Result<()> {
        for (b, specs) in &changes.to_install {
            for s in specs {
                let bin = s.options.get("verify_binary").unwrap_or(&s.name);
                if let Ok(path) = which::which(bin) {
                    if !path.to_string_lossy().contains(".local/share/linix") {
                        warn!("Conflict: {} exists at {:?}. Manager {} might shadow it.", bin, path, b);
                        if !self.config.yes { return Err(Error::Other(format!("Conflict on {}", bin))); }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn load_desired_packages(&self) -> Result<HashMap<String, Vec<PackageSpec>>> {
        let mut packages_by_backend: HashMap<String, Vec<PackageSpec>> = HashMap::new();
        let raw_lines = load_all_packages(&self.config.groups_dir)?;
        let mut queue: VecDeque<String> = raw_lines.into_iter().collect();
        let mut expanded = Vec::new();
        let mut visited = HashSet::new();
        while let Some(line) = queue.pop_front() {
            if let Some(group_pkgs) = self.config.groups.get(&line) {
                if visited.insert(line.clone()) { for pkg in group_pkgs { queue.push_back(pkg.clone()); } }
                continue;
            }
            expanded.push(line);
        }
        let locked = if self.use_lockfile { self.load_lockfile().unwrap_or_default() } else { HashMap::new() };
        for line in expanded {
            let mut spec = self.parse_package_spec(&line);
            if let Some(real) = self.config.aliases.get(&spec.backend) { spec.backend = real.clone(); }
            if self.use_lockfile {
                if let Some(v) = locked.get(&spec.backend).and_then(|b| b.get(&spec.name)) {
                    spec.options.insert("version".to_string(), v.clone());
                }
            }
            packages_by_backend.entry(spec.backend.clone()).or_default().push(spec);
        }
        Ok(packages_by_backend)
    }

    async fn get_installed_full(&self, manager: &dyn crate::core::PackageManager) -> Result<Vec<Package>> {
        // MATURE: Use the cache to keep track of previous list results for this specific session
        if let Some(cached) = self.cache.get_installed(manager.name()).await {
            return Ok(cached.into_iter().map(|n| Package { name: n, version: None, backend: manager.name().to_string(), description: None, repository: None, size: None }).collect());
        }
        let pkgs = manager.list_installed().await?;
        let names: Vec<String> = pkgs.iter().map(|p| p.name.clone()).collect();
        self.cache.set_installed(manager.name().to_string(), names).await;
        Ok(pkgs)
    }

    async fn get_installed_names(&self, manager: &dyn crate::core::PackageManager) -> Result<HashSet<String>> {
        let pkgs = self.get_installed_full(manager).await?;
        Ok(pkgs.into_iter().map(|p| p.name).collect())
    }

    pub async fn calculate_changes(&self) -> Result<SyncChanges> {
        let desired = self.load_desired_packages()?;
        let mut changes = SyncChanges::default();
        for manager in self.registry.available() {
            let backend = manager.name().to_string();
            let specs = desired.get(&backend).cloned().unwrap_or_default();
            if specs.is_empty() { continue; }
            let installed = self.get_installed_names(manager.as_ref()).await?;
            let to_install: Vec<_> = specs.into_iter().filter(|s| !installed.contains(&s.name)).collect();
            if !to_install.is_empty() { changes.to_install.insert(backend, to_install); }
        }
        Ok(changes)
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
        let _ = fs::write(self.config.groups_dir.join("linix.lock"), serde_json::to_string_pretty(&locked).unwrap_or_default());
        Ok(())
    }

    fn load_lockfile(&self) -> Result<HashMap<String, HashMap<String, String>>> {
        let path = self.config.groups_dir.join("linix.lock");
        if !path.exists() { return Ok(HashMap::new()); }
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }

    fn parse_package_spec(&self, line: &str) -> PackageSpec {
        let (b_part, rest) = line.find(':').map(|i| (&line[..i], &line[i+1..])).unwrap_or(("", line));
        let (n_part, o_part) = rest.find('@').map(|i| (&rest[..i], &rest[i+1..])).unwrap_or((rest, ""));
        let backend = if b_part.is_empty() { self.detect_system_backend() } else { b_part.to_string() };
        let mut options = HashMap::new();
        for pair in o_part.split(',').filter(|s| !s.is_empty()) {
            let (k, v) = pair.find('=').map(|i| (&pair[..i], &pair[i+1..])).unwrap_or((pair, "true"));
            options.insert(k.to_string(), v.to_string());
        }
        PackageSpec { name: n_part.to_string(), backend, options }
    }

    fn detect_system_backend(&self) -> String { "apt".to_string() }

    fn confirm_changes(&self) -> Result<bool> {
        print!("Proceed? [y/N] ");
        let _ = io::stdout().flush();
        let mut input = String::new();
        let _ = io::stdin().read_line(&mut input);
        Ok(input.trim().to_lowercase() == "y")
    }

    fn display_changes(&self, changes: &SyncChanges) {
        println!("\nSync Plan: {} to install.", changes.total_install());
        for (b, specs) in &changes.to_install {
            for s in specs { println!("  + [{}] {}", b, s.name); }
        }
    }
}