use crate::backends::BackendRegistry;
use crate::config::parser::{
    identify_line, is_reserved_manifest, parse_group_file, ManifestLine,
};
use crate::config::Config;
use crate::core::{Error, PackageSpec, Result, Validator};
use semver::{Version, VersionReq};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::fs;
use tracing::{debug, info, instrument, trace, warn};
use version_compare::{compare as loose_compare, Cmp};

pub struct StateResolver<'a> {
    config: &'a Config,
    registry: Arc<BackendRegistry>,
    /// When true, a package with no entry in locks.json is an error rather than a free
    /// resolve — the whole point of a locked run is that nothing floats.
    locked: bool,
    /// "backend:package" -> version.
    locks: HashMap<String, String>,
}

impl<'a> StateResolver<'a> {
    pub async fn new(config: &'a Config, registry: Arc<BackendRegistry>, locked: bool) -> Self {
        let mut locks = HashMap::new();

        if locked {
            let lock_path = config.groups_dir.join("locks.json");
            debug!(
                "Resolver: Locked mode active. Probing for locks at {:?}",
                lock_path
            );

            if tokio::fs::try_exists(&lock_path).await.unwrap_or(false) {
                if let Ok(data) = fs::read_to_string(&lock_path).await {
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

    #[instrument(skip(self))]
    pub async fn resolve_desired_state(&self) -> Result<HashMap<String, Vec<PackageSpec>>> {
        let mut resolved: HashMap<String, Vec<PackageSpec>> = HashMap::new();
        let mut raw_inputs: VecDeque<(String, String)> = VecDeque::new();
        let mut seen_keys: HashSet<String> = HashSet::new();

        let hostname = Config::get_hostname();
        info!(
            "Resolver: Expanding manifest closure for host '{}'.",
            hostname
        );

        let dir = &self.config.groups_dir;
        if tokio::fs::try_exists(dir).await.unwrap_or(false) {
            // Sorted, not `read_dir` order: the filesystem yields hash order on ext4 and
            // B-tree order on NTFS, and this order is load-bearing — later lines override
            // earlier ones, so two files pinning one package would be resolved by the disk
            // and could differ between machines holding identical files.
            let mut names: Vec<String> = Vec::new();
            let mut entries = fs::read_dir(dir).await.map_err(Error::from)?;
            while let Some(entry) = entries.next_entry().await.map_err(Error::from)? {
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
            names.sort();

            for fname in names {
                if !fname.ends_with(".txt") || fname.ends_with(".module.txt") {
                    continue;
                }
                // LiNix's own files live in this folder too and are not wish lists.
                if is_reserved_manifest(&fname) {
                    continue;
                }
                let source_id = format!("manifest:{}", fname);
                for line in parse_group_file(&dir.join(&fname)).await? {
                    raw_inputs.push_back((line, source_id.clone()));
                }
            }
        }


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
                ManifestLine::Module(mod_spec) => {
                    let mod_name = mod_spec.trim().to_string();
                    let mod_file = self
                        .config
                        .modules_dir
                        .join(format!("{}.module.txt", mod_name));
                    trace!(
                        "Resolver: Expanding @module '{}' requested by {}",
                        mod_name,
                        source
                    );

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

                ManifestLine::Package(pkg_str) => {
                    let mut spec = self.parse_and_probe_spec(&pkg_str).await?;

                    // Per-host backend gating: if this host manages only a subset of
                    // backends, manifest entries for the others are ignored here (not an
                    // error) — the intended way to keep e.g. npm/cargo out of a server.
                    if !self.config.is_backend_enabled(&spec.backend) {
                        debug!(
                            "Resolver: Skipping '{}:{}' — backend not enabled on host '{}'.",
                            spec.backend, spec.name, hostname
                        );
                        continue;
                    }

                    let unique_id = format!("{}:{}", spec.backend, spec.name);

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
                            spec.options
                                .insert("version".to_string(), locked_ver.clone());
                        } else {
                            return Err(Error::Validation(format!(
                                "Locked Mode Error: '{}' is missing from locks.json.",
                                unique_id
                            )));
                        }
                    }

                    if seen_keys.insert(unique_id.clone()) {
                        Validator::validate_package_name_for(&spec.name, &spec.backend)?;

                        spec.options.insert("__source".to_string(), source.clone());

                        for req in &spec.requires {
                            raw_inputs.push_back((req.clone(), format!("dep:{}", spec.name)));
                        }

                        resolved.entry(spec.backend.clone()).or_default().push(spec);
                    } else {
                        // Already resolved from another source. Instead of dropping this
                        // origin (first-write-wins, which could hide the package from
                        // `upgrade --module X` if a different source enqueued it first),
                        // MERGE this source into the existing spec's `__source` tag.
                        // Sources are ';'-joined; the planner's scope matcher splits on ';'.
                        if let Some(specs) = resolved.get_mut(&spec.backend) {
                            if let Some(existing) = specs.iter_mut().find(|s| s.name == spec.name) {
                                let entry =
                                    existing.options.entry("__source".to_string()).or_default();
                                merge_source_tag(entry, &source);
                            }
                        }
                    }
                }
            }
        }


        debug!(
            "Resolver: Desired state calculated successfully in {} expansion steps.",
            iterations
        );
        Ok(resolved)
    }

    /// Accepted syntax: `[backend:]name[@key=val,key2=val2]`, where `requires` is the one
    /// option whose value is a `;`-separated list rather than a scalar.
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
            let key = k.trim().to_string();
            let val = v.trim().to_string();

            if key == "requires" {
                requires = val.split(';').map(|s| s.to_string()).collect();
            } else {
                options.insert(key, val);
            }
        }

        let backend = if let Some(b) = b_part {
            self.config
                .aliases
                .get(b)
                .cloned()
                .unwrap_or_else(|| b.to_string())
        } else {
            let mut detected_backend = None;
            let ver_constraint = options.get("version").map(|s| s.as_str());

            for b_priority_name in &self.config.backend_priority {
                if self
                    .remote_package_exists(b_priority_name, package_name, ver_constraint)
                    .await
                {
                    detected_backend = Some(b_priority_name.clone());
                    break;
                }
            }

            detected_backend.unwrap_or_else(|| {
                self.config
                    .default_backend
                    .clone()
                    .unwrap_or_else(|| "apt".to_string())
            })
        };

        Ok(PackageSpec {
            name: package_name.to_string(),
            backend,
            options,
            requires,
            present: true,
        })
    }

    async fn remote_package_exists(
        &self,
        backend_name: &str,
        package_name: &str,
        constraint: Option<&str>,
    ) -> bool {
        let backend_cap = match self.registry.get(backend_name) {
            Some(b) if b.is_available() => b,
            _ => return false,
        };

        if let Some(searchable) = backend_cap.as_searchable() {
            if let Ok(true) = searchable.remote_has(package_name).await {
                if let Some(req) = constraint {
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

            // `remote_has` returning false is not proof of absence — a backend may not
            // implement it — so an inconclusive answer falls through to a real search.
            if let Ok(results) = searchable.search(package_name).await {
                return results.iter().any(|pkg| {
                    if pkg.name == package_name {
                        match constraint {
                            Some(req) => pkg
                                .version
                                .as_deref()
                                .is_some_and(|v| self.satisfies_constraint(v, req)),
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

    fn satisfies_constraint(&self, version: &str, constraint: &str) -> bool {
        if constraint == "latest" || constraint == "*" || constraint.is_empty() {
            return true;
        }

        // SemVer first, then literal, then loose: package managers ship versions SemVer
        // cannot parse (epochs, distro suffixes), and those must still be comparable.
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

/// Merge an additional origin into a package's `;`-joined `__source` tag, de-duplicated.
/// This lets a package that appears in multiple sources (e.g. a manifest AND a module)
/// remain matchable by every scope it belongs to. The planner splits this tag on `;`.
fn merge_source_tag(existing: &mut String, source: &str) {
    if existing.split(';').any(|s| s == source) {
        return;
    }
    if existing.is_empty() {
        existing.push_str(source);
    } else {
        existing.push(';');
        existing.push_str(source);
    }
}

#[cfg(test)]
mod wish_list_tests {
    use crate::backends::BackendRegistry;
    use crate::config::Config;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn write(dir: &std::path::Path, name: &str, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[tokio::test]
    async fn manifests_are_read_in_sorted_order_not_filesystem_order() {
        // C4. `read_dir` yields whatever order the filesystem feels like, and the FIRST
        // declaration of a package wins — so with two files pinning one package, the winner
        // was decided by the filesystem and could differ between two machines holding
        // byte-identical files. Sorted by name, it is a rule someone can predict.
        let tmp = tempdir().unwrap();
        let global = tmp.path().join("global");
        // Created in reverse order on purpose: creation order must not decide the winner.
        write(&global, "z-last.txt", "apt:curl@version=2.0\n");
        write(&global, "a-first.txt", "apt:curl@version=1.0\n");

        let cfg = Config {
            groups_dir: global.clone(),
            ..Config::default()
        };
        let reg = Arc::new(BackendRegistry::new());
        let resolver = super::StateResolver::new(&cfg, reg, false).await;
        let by_backend = resolver.resolve_desired_state().await.unwrap();
        let curl = by_backend
            .get("apt")
            .and_then(|v| v.iter().find(|s| s.name == "curl"))
            .expect("curl resolved");
        // a-first.txt sorts before z-last.txt, and the first declaration wins.
        assert_eq!(
            curl.options.get("version").map(String::as_str),
            Some("1.0"),
            "sorted read order must decide the winner, not the filesystem"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::merge_source_tag;

    #[test]
    fn merges_and_dedups_sources() {
        let mut s = String::from("manifest:base.txt");
        merge_source_tag(&mut s, "module:dev");
        assert_eq!(s, "manifest:base.txt;module:dev");

        // duplicate source is not appended again
        merge_source_tag(&mut s, "module:dev");
        assert_eq!(s, "manifest:base.txt;module:dev");

        // a third distinct source joins
        merge_source_tag(&mut s, "group:editors");
        assert_eq!(s, "manifest:base.txt;module:dev;group:editors");

        // empty start
        let mut empty = String::new();
        merge_source_tag(&mut empty, "module:x");
        assert_eq!(empty, "module:x");
    }
}
