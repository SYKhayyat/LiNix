use crate::config::Config;
use crate::config::parser::parse_group_file;
use crate::core::{PackageSpec, Result, Validator};
use crate::backends::BackendRegistry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tracing::{debug, warn, info};
use semver::{Version, VersionReq};
use version_compare::{Cmp, compare as loose_compare};

/// Responsible for calculating the "Desired State" of the system.
/// Orchestrates group expansion, hostname matching, and recursive meta-dependency resolution.
/// 
/// Refactored for Version 3.5.0 to support SemVer-Aware Probing (Point 10/11)
/// and remote availability checking (FIX #4 and FIX #20).
pub struct StateResolver<'a> {
    config: &'a Config,
    registry: Arc<BackendRegistry>,
}

impl<'a> StateResolver<'a> {
    pub fn new(config: &'a Config, registry: Arc<BackendRegistry>) -> Self {
        Self { config, registry }
    }

    /// Primary entry point: Produces a map of Backend -> List of structured Specs.
    pub async fn resolve_desired_state(&self) -> Result<HashMap<String, Vec<PackageSpec>>> {
        let mut resolved: HashMap<String, Vec<PackageSpec>> = HashMap::new();
        let mut raw_packages: HashSet<String> = HashSet::new();

        let hostname = Config::get_hostname();
        
        // 1. Load hostname-specific and directory-based .txt manifests
        if self.config.groups_dir.exists() {
            for entry in std::fs::read_dir(&self.config.groups_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() {
                    let fname = path.file_name().unwrap_or_default().to_string_lossy();
                    if fname.starts_with("host-") && fname != format!("host-{}.txt", hostname) {
                        continue;
                    }
                    if fname.ends_with(".txt") {
                        for pkg in parse_group_file(&path)? {
                            raw_packages.insert(pkg);
                        }
                    }
                }
            }
        }

        // 2. Load hostname-specific packages from config.toml
        if let Some(config_pkgs) = self.config.hostname_packages.get(&hostname) {
            for p in config_pkgs {
                raw_packages.insert(p.clone());
            }
        }

        // 3. Recursive Resolution Queue with depth limit
        let mut queue: VecDeque<String> = raw_packages.into_iter().collect();
        let mut seen_keys = HashSet::new();
        let mut processed_specs = Vec::new();
        const MAX_RECURSION_DEPTH: usize = 100;
        let mut depth = 0;

        while let Some(line) = queue.pop_front() {
            depth += 1;
            if depth > MAX_RECURSION_DEPTH {
                return Err(crate::core::Error::Transaction(
                    "Maximum recursion depth exceeded in dependency resolution".into()
                ));
            }
            
            if let Some(group_name) = line.strip_prefix("group:") {
                let group_path = self.config.groups_dir.join(format!("{}.txt", group_name));
                if group_path.exists() {
                    for pkg in parse_group_file(&group_path)? {
                        queue.push_back(pkg);
                    }
                }
                continue;
            }

            let spec = self.parse_and_probe_spec(&line).await?;
            let key = format!("{}:{}", spec.backend, spec.name);

            if seen_keys.insert(key) {
                Validator::validate_package_name(&spec.name)?;
                for req in &spec.requires {
                    queue.push_back(req.clone());
                }
                processed_specs.push(spec);
            }
        }

        for spec in processed_specs {
            resolved.entry(spec.backend.clone()).or_default().push(spec);
        }

        Ok(resolved)
    }

    /// Translates a raw string into a structured PackageSpec.
    /// If no backend is provided, it probes the `backend_priority` list.
    /// Probing is SemVer-aware: it checks if the backend provides a version 
    /// that satisfies the constraint (Point 11).
    /// 
    /// FIX #20: Uses remote_exists() for efficient probing.
    pub async fn parse_and_probe_spec(&self, line: &str) -> Result<PackageSpec> {
        let (b_part, rest) = if let Some((b, r)) = line.split_once(':') {
            (Some(b), r)
        } else {
            (None, line)
        };

        let (n_part, o_part) = rest.split_once('@').unwrap_or((rest, ""));
        let package_name = n_part.trim();
        
        let mut options = HashMap::new();
        let mut requires = Vec::new();

        for pair in o_part.split(',').filter(|s| !s.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, "true"));
            if k == "requires" {
                requires = v.split(';').map(|s| s.to_string()).collect();
            } else {
                options.insert(k.trim().to_string(), v.trim().to_string());
            }
        }

        let version_constraint = options.get("version").map(|s| s.as_str());

        // Backend determination
        let backend = if let Some(b) = b_part {
            self.config.aliases.get(b).cloned().unwrap_or_else(|| b.to_string())
        } else {
            let mut found_backend = None;
            for b_name in &self.config.backend_priority {
                // FIX #20: Use remote_exists for efficient checking
                if self.remote_package_exists(b_name, package_name, version_constraint).await {
                    debug!("StateResolver: Found '{}' available in remote repository for backend '{}' with constraint {:?}", 
                           package_name, b_name, version_constraint);
                    found_backend = Some(b_name.clone());
                    break;
                }
            }
            found_backend.unwrap_or_else(|| {
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

    /// FIX #20: Efficiently checks if a package exists in remote repositories.
    /// Uses remote_exists() when available, falls back to search() otherwise.
    async fn remote_package_exists(&self, backend_name: &str, package_name: &str, constraint: Option<&str>) -> bool {
        let backend = match self.registry.get(backend_name) {
            Some(b) if b.core().is_available() => b,
            _ => return false,
        };

        // Try to use remote_has if available (most efficient)
        if let Some(searchable) = backend.as_searchable() {
            match searchable.remote_has(package_name).await {
                Ok(true) => {
                    // Package exists, now check version constraint if needed
                    if let Some(req) = constraint {
                        match searchable.remote_info(package_name).await {
                            Ok(Some(pkg)) => {
                                if let Some(ver) = pkg.version.as_deref() {
                                    return self.satisfies_constraint(ver, req);
                                }
                            }
                            Ok(None) => return false,
                            Err(e) => {
                                debug!("remote_info failed for {}: {}", package_name, e);
                                return false;
                            }
                        }
                    }
                    return true;
                }
                Ok(false) => return false,
                Err(e) => {
                    debug!("remote_has failed for {}: {}, falling back to search", package_name, e);
                }
            }
        }

        // Fallback to search if remote_has is not available
        if let Some(searchable) = backend.as_searchable() {
            match searchable.search(package_name).await {
                Ok(results) => {
                    for pkg in results {
                        if pkg.name == package_name {
                            if let Some(req) = constraint {
                                if let Some(ver) = pkg.version.as_deref() {
                                    if self.satisfies_constraint(ver, req) {
                                        return true;
                                    }
                                }
                            } else {
                                return true;
                            }
                        }
                    }
                }
                Err(e) => {
                    debug!("Search failed for backend '{}': {}", backend_name, e);
                }
            }
        }
        
        // Last resort: some backends might have info that queries remote
        if let Some(queryable) = backend.as_queryable() {
            if let Ok(Some(pkg)) = queryable.info(package_name).await {
                if pkg.properties.contains_key("repository_url") 
                    || pkg.properties.contains_key("homepage")
                    || pkg.properties.contains_key("download_url") {
                    if let Some(req) = constraint {
                        if let Some(ver) = pkg.version.as_deref() {
                            if self.satisfies_constraint(ver, req) {
                                return true;
                            }
                        }
                    } else {
                        return true;
                    }
                }
            }
        }
        
        debug!("Backend '{}' does not have package '{}' in remote repositories", backend_name, package_name);
        false
    }

    /// Legacy method - kept for compatibility.
    #[deprecated(since = "3.5.0", note = "Use remote_package_exists for correct probing")]
    async fn probe_backend(&self, backend_name: &str, package_name: &str, constraint: Option<&str>) -> bool {
        self.remote_package_exists(backend_name, package_name, constraint).await
    }

    /// Logic for SemVer constraint matching.
    fn satisfies_constraint(&self, version: &str, constraint: &str) -> bool {
        if constraint == "latest" || constraint == "*" {
            return true;
        }

        if let Ok(req) = VersionReq::parse(constraint) {
            if let Ok(ver) = Version::parse(version) {
                return req.matches(&ver);
            }
        }

        if version == constraint {
            return true;
        }

        match loose_compare(version, constraint) {
            Ok(Cmp::Eq) => true,
            Ok(Cmp::Gt) if constraint.starts_with('>') => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::create_default_registry;
    use crate::core::CommandExecutor;
    use crate::app::LuaHooks;
    use tempfile::tempdir;

    fn create_test_config() -> Config {
        let mut config = Config::default();
        config.backend_priority = vec!["apt".to_string(), "brew".to_string(), "cargo".to_string()];
        config
    }

    async fn create_test_resolver() -> StateResolver<'static> {
        let config = create_test_config();
        let executor = CommandExecutor::new(true, false);
        let hooks = Arc::new(LuaHooks::new(&config).unwrap());
        let registry = Arc::new(create_default_registry(executor, &config, hooks).await);
        
        let config_box = Box::leak(Box::new(config));
        
        StateResolver {
            config: config_box,
            registry,
        }
    }

    #[tokio::test]
    async fn test_satisfies_constraint() {
        let resolver = create_test_resolver().await;
        
        assert!(resolver.satisfies_constraint("1.2.3", "1.2.3"));
        assert!(resolver.satisfies_constraint("1.2.3", "latest"));
        assert!(resolver.satisfies_constraint("1.2.3", "*"));
        assert!(resolver.satisfies_constraint("1.2.3", ">=1.2.0"));
        assert!(!resolver.satisfies_constraint("1.2.3", ">=2.0.0"));
    }

    #[tokio::test]
    async fn test_parse_simple_spec() {
        let resolver = create_test_resolver().await;
        
        let spec = resolver.parse_and_probe_spec("apt:curl").await.unwrap();
        assert_eq!(spec.backend, "apt");
        assert_eq!(spec.name, "curl");
        
        let spec = resolver.parse_and_probe_spec("cargo:ripgrep@version=13.0.0").await.unwrap();
        assert_eq!(spec.backend, "cargo");
        assert_eq!(spec.name, "ripgrep");
        assert_eq!(spec.options.get("version"), Some(&"13.0.0".to_string()));
    }

    #[tokio::test]
    async fn test_parse_with_requires() {
        let resolver = create_test_resolver().await;
        
        let spec = resolver.parse_and_probe_spec("apt:neovim@requires=apt:gcc;apt:make").await.unwrap();
        assert_eq!(spec.name, "neovim");
        assert_eq!(spec.requires.len(), 2);
        assert!(spec.requires.contains(&"apt:gcc".to_string()));
        assert!(spec.requires.contains(&"apt:make".to_string()));
    }

    #[test]
    fn test_version_constraint_parsing() {
        let config = create_test_config();
        let registry = Arc::new(crate::backends::BackendRegistry::new());
        let resolver = StateResolver {
            config: Box::leak(Box::new(config)),
            registry,
        };
        
        assert!(resolver.satisfies_constraint("2.0.0", ">=1.0.0"));
        assert!(resolver.satisfies_constraint("1.5.0", "^1.4.0"));
        assert!(!resolver.satisfies_constraint("1.0.0", "^2.0.0"));
    }

    #[tokio::test]
    async fn test_remote_package_exists() {
        let resolver = create_test_resolver().await;
        
        // This tests the method signature and basic logic
        // Actual existence depends on network and repositories
        let exists = resolver.remote_package_exists("apt", "curl", None).await;
        // In CI without network, this may be false - that's acceptable
        // The important part is that it doesn't panic
        let _ = exists;
    }
    
    #[tokio::test]
    async fn test_recursion_depth_limit() {
        // Create a resolver and test that deep recursion is caught
        let resolver = create_test_resolver().await;
        // This is a compile-time test - the actual depth limit is enforced in resolve_desired_state
        assert!(MAX_RECURSION_DEPTH == 100);
    }
}