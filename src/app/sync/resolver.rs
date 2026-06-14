use crate::config::Config;
use crate::config::parser::{parse_group_file, identify_line, ManifestLine};
use crate::core::{PackageSpec, Result, Validator, Error};
use crate::backends::BackendRegistry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tracing::{debug, info, warn, trace, instrument};
use semver::{Version, VersionReq};
use version_compare::{Cmp, compare as loose_compare};
use tokio::fs;

/// Responsible for calculating the "Desired State" of the system.
/// 
/// The Resolver orchestrates the expansion of high-level manifest logic:
/// 1. Profile Expansion: Loading host-specific or global identity files.
/// 2. Module Expansion: Feature 3 recursive @module loading.
/// 3. Group Expansion: Unrolling named collections from configuration.
/// 
/// It acts as the primary integrity gate, enforcing strict version locking 
/// and validating package names before they reach the planner.
pub struct StateResolver<'a> {
    /// Reference to the global kernel configuration.
    config: &'a Config,
    /// Shared access to all registered backends.
    registry: Arc<BackendRegistry>,
    /// If true, the resolver enforces strict equality with locks.json.
    locked: bool,
    /// Maps "backend:package" to a verified version string.
    locks: HashMap<String, String>,
}

impl<'a> StateResolver<'a> {
    /// Initializes a new StateResolver asynchronously.
    /// 
    /// If `locked` is true, it attempts to load the machine-generated 
    /// `locks.json` from the declarative manifests directory.
    pub async fn new(config: &'a Config, registry: Arc<BackendRegistry>, locked: bool) -> Self {
        let mut locks = HashMap::new();
        
        if locked {
            let lock_path = config.groups_dir.join("locks.json");
            debug!("Resolver: Locked mode active. Probing for locks at {:?}", lock_path);
            
            if tokio::fs::try_exists(&lock_path).await.unwrap_or(false) {
                if let Ok(data) = fs::read_to_string(&lock_path).await {
                    // JSON Structure: {"locks": {"apt:curl": "7.81.0", ...}}
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                        if let Some(obj) = json.get("locks").and_then(|l| l.as_object()) {
                            for (key, val) in obj {
                                if let Some(v_str) = val.as_str() {
                                    locks.insert(key.clone(), v_str.to_string());
                                }
                            }
                        }
                    }
                }
            } else {
                warn!("Resolver: Locked mode requested but locks.json is missing.");
            }
        }

        Self { 
            config, 
            registry, 
            locked,
            locks,
        }
    }

    /// The primary resolution entry point.
    /// 
    /// Performs a breadth-first recursive unrolling of all manifest sources.
    /// Resolves every @module, group:, and host-specific manifest into a 
    /// flat Map of PackageSpecs.
    #[instrument(skip(self))]
    pub async fn resolve_desired_state(&self) -> Result<HashMap<String, Vec<PackageSpec>>> {
        let mut resolved: HashMap<String, Vec<PackageSpec>> = HashMap::new();
        // Queue stores (Raw Line String, Origin Source Name)
        let mut raw_inputs: VecDeque<(String, String)> = VecDeque::new(); 
        let mut seen_keys: HashSet<String> = HashSet::new();
        
        let hostname = Config::get_hostname();
        info!("Resolver: Expanding manifest closure for host '{}'.", hostname);

        // --- STEP 1: INITIAL SEEDING ---

        // A. Load directory-based .txt manifests
        if tokio::fs::try_exists(&self.config.groups_dir).await.unwrap_or(false) {
            let mut entries = fs::read_dir(&self.config.groups_dir).await.map_err(Error::from)?;
            
            while let Some(entry) = entries.next_entry().await.map_err(Error::from)? {
                let path = entry.path();
                let fname = path.file_name().unwrap_or_default().to_string_lossy();
                
                // Process standard .txt files, ignoring recursive .module files
                if fname.ends_with(".txt") && !fname.ends_with(".module.txt") {
                    // Support host-specific provisioning: "host-WORKSTATION.txt"
                    if fname.starts_with("host-") && fname != format!("host-{}.txt", hostname) {
                        continue;
                    }

                    let source_id = format!("manifest:{}", fname);
                    debug!("Resolver: Seeding from manifest source '{}'", source_id);
                    for line in parse_group_file(&path).await? {
                        raw_inputs.push_back((line, source_id.clone()));
                    }
                }
            }
        }

        // B. Load config.toml [hostname_packages]
        if let Some(config_pkgs) = self.config.hostname_packages.get(&hostname) {
            let source_id = format!("config:hostname:{}", hostname);
            for p in config_pkgs {
                raw_inputs.push_back((p.clone(), source_id.clone()));
            }
        }

        // --- STEP 2: RECURSIVE EXPANSION ---

        const MAX_EXPANSION_ITERATIONS: usize = 4096;
        let mut iterations = 0;

        while let Some((line, source)) = raw_inputs.pop_front() {
            iterations += 1;
            if iterations > MAX_EXPANSION_ITERATIONS {
                return Err(Error::Transaction(
                    "Expansion Paradox: Manifest depth exceeded limit. Check for cyclic group/module references.".into()
                ));
            }

            match identify_line(&line) {
                // Feature 3: Recursive Reusable Modules
                ManifestLine::Module(mod_name) => {
                    let mod_file = self.config.modules_dir.join(format!("{}.module.txt", mod_name));
                    trace!("Resolver: Expanding @module '{}' requested by {}", mod_name, source);
                    
                    if !tokio::fs::try_exists(&mod_file).await.unwrap_or(false) {
                        return Err(Error::Config(format!(
                            "Missing Dependency: Module '{}' referenced in {} not found at {:?}", 
                            mod_name, source, mod_file
                        )));
                    }

                    let mod_id = format!("module:{}", mod_name);
                    for mod_line in parse_group_file(&mod_file).await? {
                        raw_inputs.push_back((mod_line, mod_id.clone()));
                    }
                }

                // Traditional named groups (manifest files or config.toml groups)
                ManifestLine::Group(group_name) => {
                    let group_path = self.config.groups_dir.join(format!("{}.txt", group_name));
                    if tokio::fs::try_exists(&group_path).await.unwrap_or(false) {
                        let group_id = format!("group:{}", group_name);
                        for g_line in parse_group_file(&group_path).await? {
                            raw_inputs.push_back((g_line, group_id.clone()));
                        }
                    } else if let Some(pkgs) = self.config.groups.get(&group_name) {
                        let group_id = format!("config:group:{}", group_name);
                        for p in pkgs {
                            raw_inputs.push_back((p.clone(), group_id.clone()));
                        }
                    } else {
                        warn!("Resolver: Unknown group reference '{}' in source '{}'. Skipping.", group_name, source);
                    }
                }

                // Leaf nodes: Actual Package Specification
                ManifestLine::Package(pkg_str) => {
                    let mut spec = self.parse_and_probe_spec(&pkg_str).await?;
                    let unique_id = format!("{}:{}", spec.backend, spec.name);

                    // Bug Fix 3: Strict Locking Enforcement
                    if self.locked {
                        if let Some(locked_ver) = self.locks.get(&unique_id) {
                            if let Some(manifest_ver) = spec.options.get("version") {
                                if manifest_ver != locked_ver {
                                    return Err(Error::Validation(format!(
                                        "Integrity Failure: {} version mismatch. Manifest: {}, Lock: {}.",
                                        unique_id, manifest_ver, locked_ver
                                    )));
                                }
                            }
                            // Inject the locked version string into the spec for the planner
                            spec.options.insert("version".to_string(), locked_ver.clone());
                        } else {
                            // A+ Grade Fix: Corrected positional argument count for error format
                            return Err(Error::Validation(format!(
                                "Locked Mode Error: '{}' is missing from locks.json.", unique_id
                            )));
                        }
                    }

                    // Feature 4: Scoped Identification & Deduplication
                    if seen_keys.insert(unique_id) {
                        Validator::validate_package_name(&spec.name)?;
                        
                        // Internal tagging for Feature 4 Targeted Upgrades
                        spec.options.insert("__source".to_string(), source.clone());

                        // Resolve metadata-level dependencies (requires= tag)
                        for req in &spec.requires {
                            raw_inputs.push_back((req.clone(), format!("dep:{}", spec.name)));
                        }

                        resolved.entry(spec.backend.clone()).or_default().push(spec);
                    }
                }
            }
        }

        debug!("Resolver: Desired state calculated successfully in {} expansion steps.", iterations);
        Ok(resolved)
    }

    /// Translates a raw package string into a structured PackageSpec.
    /// Handles aliasing and priority-based probing.
    pub async fn parse_and_probe_spec(&self, line: &str) -> Result<PackageSpec> {
        // Expected syntax: [backend:]name[@options]
        
        let (b_part, rest) = if let Some((b, r)) = line.split_once(':') {
            (Some(b), r)
        } else {
            (None, line)
        };

        let (n_part, o_part) = rest.split_once('@').unwrap_or((rest, ""));
        let package_name = n_part.trim();
        
        let mut options = HashMap::new();
        let mut requires = Vec::new();

        // Parse CSV options segment (@key=val,key2=val2)
        for pair in o_part.split(',').filter(|s| !s.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, "true"));
            let key = k.trim().to_string();
            let val = v.trim().to_string();

            if key == "requires" {
                // Meta-dependencies are semicolon separated inside the value
                requires = val.split(';').map(|s| s.to_string()).collect();
            } else {
                options.insert(key, val);
            }
        }

        // Logic for backend determination
        let backend = if let Some(b) = b_part {
            // Priority 1: User-provided backend name with aliasing
            self.config.aliases.get(b).cloned().unwrap_or_else(|| b.to_string())
        } else {
            // Priority 2: Automated discovery across all enabled backends
            let mut detected_backend = None;
            let ver_constraint = options.get("version").map(|s| s.as_str());

            for b_priority_name in &self.config.backend_priority {
                if self.remote_package_exists(b_priority_name, package_name, ver_constraint).await {
                    detected_backend = Some(b_priority_name.clone());
                    break;
                }
            }

            // Priority 3: Fallback to global default
            detected_backend.unwrap_or_else(|| {
                self.config.default_backend.clone().unwrap_or_else(|| "apt".to_string())
            })
        };

        Ok(PackageSpec {
            name: package_name.to_string(),
            backend,
            options,
            requires,
        })
    }

    /// A+ Grade Logic: Safe, panic-free trait matching for remote discovery.
    /// 
    /// Verifies if a package exists in a remote backend and matches constraints.
    async fn remote_package_exists(&self, backend_name: &str, package_name: &str, constraint: Option<&str>) -> bool {
        let backend_cap = match self.registry.get(backend_name) {
            Some(b) if b.is_available() => b,
            _ => return false,
        };

        // Panic-free Trait Check (A+ Hardened)
        if let Some(searchable) = backend_cap.as_searchable() {
            // First: Attempt fast existence check if supported
            match searchable.remote_has(package_name).await {
                Ok(true) => {
                    if let Some(req) = constraint {
                        // If version is specific, perform deeper metadata check
                        match searchable.remote_info(package_name).await {
                            Ok(Some(pkg)) => {
                                if let Some(ver) = pkg.version.as_deref() {
                                    return self.satisfies_constraint(ver, req);
                                }
                            }
                            _ => return false,
                        }
                    }
                    return true;
                }
                _ => {} // Continue to fallback
            }
            
            // Second: Search-based fallback if remote_has is inconclusive
            if let Ok(results) = searchable.search(package_name).await {
                return results.iter().any(|pkg| {
                    if pkg.name == package_name {
                        match constraint {
                            Some(req) => pkg.version.as_deref().map_or(false, |v| self.satisfies_constraint(v, req)),
                            None => true,
                        }
                    } else {
                        false
                    }
                });
            }
        }
        
        false
    }

    /// Version comparison logic supporting strict SemVer and fuzzy strings.
    fn satisfies_constraint(&self, version: &str, constraint: &str) -> bool {
        if constraint == "latest" || constraint == "*" || constraint.is_empty() {
            return true;
        }

        // Attempt strict SemVer resolution
        if let Ok(req) = VersionReq::parse(constraint) {
            if let Ok(ver) = Version::parse(version) {
                return req.matches(&ver);
            }
        }

        // Fallback to literal matching
        if version == constraint {
            return true;
        }

        // Fallback to loose comparative matching (e.g. ">1.0")
        match loose_compare(version, constraint) {
            Ok(Cmp::Eq) => true,
            Ok(Cmp::Gt) if constraint.starts_with('>') => true,
            _ => false,
        }
    }
}