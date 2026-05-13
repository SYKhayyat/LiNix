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
/// Refactored for Version 3.4.0 to support SemVer-Aware Probing (Point 10/11).
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
                    // Basic filter: skip host files that don't match current machine
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

        // 3. Recursive Resolution Queue
        let mut queue: VecDeque<String> = raw_packages.into_iter().collect();
        let mut seen_keys = HashSet::new();
        let mut processed_specs = Vec::new();

        while let Some(line) = queue.pop_front() {
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
                if self.probe_backend(b_name, package_name, version_constraint).await {
                    debug!("StateResolver: Probing found '{}' in backend '{}' with constraint {:?}", package_name, b_name, version_constraint);
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

    /// Internal helper to check if a backend contains a specific package 
    /// AND if that package version satisfies the constraint.
    async fn probe_backend(&self, backend_name: &str, package_name: &str, constraint: Option<&str>) -> bool {
        let backend = match self.registry.get(backend_name) {
            Some(b) if b.is_available() => b,
            _ => return false,
        };

        // 1. Check local state (Queryable)
        if let Some(q) = backend.as_queryable() {
            if let Ok(Some(pkg)) = q.info(package_name).await {
                if let Some(req) = constraint {
                    if let Some(ver) = pkg.version.as_deref() {
                        if self.satisfies_constraint(ver, req) { return true; }
                    }
                } else {
                    return true; 
                }
            }
        }

        // 2. Check remote repositories (Searchable)
        if let Some(s) = backend.as_searchable() {
            if let Ok(results) = s.search(package_name).await {
                for pkg in results {
                    if pkg.name == package_name {
                        if let Some(req) = constraint {
                            if let Some(ver) = pkg.version.as_deref() {
                                if self.satisfies_constraint(ver, req) { return true; }
                            }
                        } else {
                            return true;
                        }
                    }
                }
            }
        }

        false
    }

    /// Logic for SemVer constraint matching.
    fn satisfies_constraint(&self, version: &str, constraint: &str) -> bool {
        if constraint == "latest" || constraint == "*" {
            return true;
        }

        // Strict SemVer check
        if let Ok(req) = VersionReq::parse(constraint) {
            if let Ok(ver) = Version::parse(version) {
                return req.matches(&ver);
            }
        }

        // Fallback to exact match
        if version == constraint {
            return true;
        }

        // Fallback to loose alphanumeric comparison
        match loose_compare(version, constraint) {
            Ok(Cmp::Eq) => true,
            Ok(Cmp::Gt) if constraint.starts_with('>') => true,
            _ => false,
        }
    }
}