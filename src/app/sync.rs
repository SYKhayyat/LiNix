// src/app/sync.rs
use crate::app::{LuaHooks, MetricsCollector};
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::config::parser::load_all_packages;
use crate::core::{CommandExecutor, PackageCache, PackageSpec, Result, Error};
use crate::utils::progress::ProgressReporter;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Write};
use std::fs;
use tracing::{info, warn};

pub struct SyncEngine<'a> {
    config: &'a Config,
    registry: &'a BackendRegistry,
    executor: &'a CommandExecutor,
    cache: &'a PackageCache, // Now used in get_installed_names
    metrics: &'a MetricsCollector,
    progress: &'a dyn ProgressReporter,
    hooks: &'a LuaHooks,
    use_lockfile: bool,
}

#[derive(Debug, Default)]
pub struct SyncChanges {
    pub to_install: HashMap<String, Vec<PackageSpec>>,
    pub to_remove: HashMap<String, Vec<String>>,
}

impl SyncChanges {
    pub fn is_empty(&self) -> bool {
        self.to_install.is_empty() && self.to_remove.is_empty()
    }
    pub fn total_install(&self) -> usize { self.to_install.values().map(|v| v.len()).sum() }
}

impl<'a> SyncEngine<'a> {
    pub fn new(config: &'a Config, registry: &'a BackendRegistry, executor: &'a CommandExecutor, cache: &'a PackageCache, metrics: &'a MetricsCollector, progress: &'a dyn ProgressReporter, hooks: &'a LuaHooks) -> Self {
        Self { config, registry, executor, cache, metrics, progress, hooks, use_lockfile: false }
    }

    pub fn with_lockfile(mut self, use_lockfile: bool) -> Self {
        self.use_lockfile = use_lockfile;
        self
    }

    pub async fn sync(&self) -> Result<()> {
        let _ = self.hooks.run_before_sync().await;
        let desired = self.load_desired_packages()?;
        let _ = self.create_snapshot(&desired);

        let changes = self.calculate_changes().await?;
        if changes.is_empty() {
            info!("System is in sync.");
            let _ = self.hooks.run_after_sync().await;
            return Ok(());
        }

        self.display_changes(&changes);
        if !self.config.yes && !self.confirm_changes()? { return Err(Error::Cancelled); }

        self.execute_changes(&changes).await?;
        let _ = self.save_lockfile(&desired).await;
        let _ = self.hooks.run_after_sync().await;
        Ok(())
    }

    pub fn load_desired_packages(&self) -> Result<HashMap<String, Vec<PackageSpec>>> {
        let mut packages_by_backend: HashMap<String, Vec<PackageSpec>> = HashMap::new();
        let raw_lines = load_all_packages(&self.config.groups_dir)?;
        
        let mut queue: VecDeque<String> = raw_lines.into_iter().collect();
        let mut expanded_lines = Vec::new();
        let mut visited = HashSet::new();

        while let Some(line) = queue.pop_front() {
            if let Some(group_pkgs) = self.config.groups.get(&line) {
                if visited.insert(line.clone()) {
                    for pkg in group_pkgs { queue.push_back(pkg.clone()); }
                }
                continue;
            }
            expanded_lines.push(line);
        }

        let locked_versions = if self.use_lockfile { self.load_lockfile().unwrap_or_default() } else { HashMap::new() };

        for line in expanded_lines {
            let mut spec = self.parse_package_spec(&line);
            if let Some(real) = self.config.aliases.get(&spec.backend) { spec.backend = real.clone(); }
            if self.use_lockfile {
                if let Some(ver) = locked_versions.get(&spec.backend).and_then(|b| b.get(&spec.name)) {
                    spec.options.insert("version".to_string(), ver.clone());
                }
            }
            packages_by_backend.entry(spec.backend.clone()).or_default().push(spec);
        }
        Ok(packages_by_backend)
    }

    pub async fn find_unmanaged(&self) -> Result<Vec<(String, Vec<String>)>> {
        let desired = self.load_desired_packages()?;
        let mut unmanaged = Vec::new();
        for manager in self.registry.available() {
            let backend = manager.name().to_string();
            let desired_names: HashSet<String> = desired.get(&backend)
                .map(|specs| specs.iter().map(|s| s.name.clone()).collect())
                .unwrap_or_default();
            
            let installed = self.get_installed_names(manager.as_ref()).await?;
            let unmanaged_pkgs: Vec<String> = installed.into_iter()
                .filter(|name| !desired_names.contains(name))
                .collect();
            
            if !unmanaged_pkgs.is_empty() { unmanaged.push((backend, unmanaged_pkgs)); }
        }
        Ok(unmanaged)
    }

    async fn get_installed_names(&self, manager: &dyn crate::core::PackageManager) -> Result<HashSet<String>> {
        let name = manager.name();
        // FIXED: Now using the cache field to resolve the warning
        if let Some(cached) = self.cache.get_installed(name).await {
            return Ok(cached.into_iter().collect());
        }
        let pkgs: Vec<String> = manager.list_installed().await?.into_iter().map(|p| p.name).collect();
        self.cache.set_installed(name.to_string(), pkgs.clone()).await;
        Ok(pkgs.into_iter().collect())
    }

    pub async fn clean(&self) -> Result<()> {
        let unmanaged = self.find_unmanaged().await?;
        if unmanaged.is_empty() { return Ok(()); }
        for (backend, packages) in unmanaged {
            if let Some(manager) = self.registry.get(&backend) {
                manager.remove(&packages, true).await?;
                self.metrics.record_remove(packages.len() as u64);
            }
        }
        Ok(())
    }

    pub async fn calculate_changes(&self) -> Result<SyncChanges> {
        let desired = self.load_desired_packages()?;
        let mut changes = SyncChanges::default();
        for manager in self.registry.available() {
            let backend = manager.name().to_string();
            let specs = desired.get(&backend).cloned().unwrap_or_default();
            if specs.is_empty() { continue; }

            let installed = self.get_installed_names(manager.as_ref()).await?;
            let to_install: Vec<PackageSpec> = specs.into_iter()
                .filter(|s| !installed.contains(&s.name)).collect();
            
            if !to_install.is_empty() { changes.to_install.insert(backend, to_install); }
        }
        Ok(changes)
    }

    async fn execute_changes(&self, changes: &SyncChanges) -> Result<()> {
        for (backend, specs) in &changes.to_install {
            if let Some(manager) = self.registry.get(backend) {
                let handle = self.progress.start(specs.len() as u64, &format!("Installing via {}", backend));
                manager.install_with_options(specs, true).await?;
                for spec in specs {
                    if let Some(bin) = spec.options.get("verify_binary") {
                        if !self.executor.command_exists(bin).await {
                            warn!("Binary '{}' not found after installing {}", bin, spec.name);
                        }
                    }
                    handle.inc(1);
                }
                self.metrics.record_install(specs.len() as u64);
                handle.finish();
            }
        }
        Ok(())
    }

    fn create_snapshot(&self, desired: &HashMap<String, Vec<PackageSpec>>) -> Result<()> {
        let ts = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
        let dir = dirs::data_dir().unwrap_or_default().join("linix").join("snapshots");
        let _ = fs::create_dir_all(&dir);
        let _ = fs::write(dir.join(format!("snap_{}.json", ts)), serde_json::to_string_pretty(desired).unwrap_or_default());
        Ok(())
    }

    async fn save_lockfile(&self, desired: &HashMap<String, Vec<PackageSpec>>) -> Result<()> {
        let mut locked: HashMap<String, HashMap<String, String>> = HashMap::new();
        for manager in self.registry.available() {
            if let Some(specs) = desired.get(manager.name()) {
                if let Ok(installed) = manager.list_installed().await {
                    let mut map = HashMap::new();
                    for s in specs {
                        if let Some(p) = installed.iter().find(|i| i.name == s.name) {
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
        println!("Sync Plan: {} to install.", changes.total_install());
    }
}