// src/app/policy.rs
//
// A declarative policy gate. Rules live in `<groups_dir>/policy.toml`; when present they
// are enforced before any change (`sync`/`upgrade`) so mistakes are stopped automatically
// instead of relying on everyone remembering. Example policy.toml:
//
//   pinned_only     = true
//   deny_packages   = ["leftpad"]
//   allow_backends  = ["apt", "cargo"]   # empty = all backends allowed
//   require_snapshot = true
//   deny_vulnerable = false              # when true, block installs of OSV-known-vulnerable pkgs

use crate::core::{Error, PackageSpec, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Policy {
    /// Package names that may never be installed (case-insensitive).
    #[serde(default)]
    pub deny_packages: Vec<String>,
    /// If non-empty, only these backends are permitted.
    #[serde(default)]
    pub allow_backends: Vec<String>,
    /// Every desired package must carry an explicit version pin.
    #[serde(default)]
    pub pinned_only: bool,
    /// A snapshot provider must be available before applying changes.
    #[serde(default)]
    pub require_snapshot: bool,
    /// Block applying changes when any managed package is known-vulnerable (via `audit`).
    #[serde(default)]
    pub deny_vulnerable: bool,
}

impl Policy {
    /// Load policy.toml if it exists. Missing file → no policy (Ok(None)).
    pub async fn load(path: &Path) -> Result<Option<Self>> {
        match tokio::fs::read_to_string(path).await {
            Ok(s) => toml::from_str(&s)
                .map(Some)
                .map_err(|e| Error::Config(format!("Invalid policy.toml: {}", e))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::Io(format!("Reading policy.toml: {}", e))),
        }
    }

    /// True when no rule is active (so enforcement is a no-op).
    pub fn is_empty(&self) -> bool {
        self.deny_packages.is_empty()
            && self.allow_backends.is_empty()
            && !self.pinned_only
            && !self.require_snapshot
            && !self.deny_vulnerable
    }

    /// Pure check of the spec-level rules (deny_packages / allow_backends / pinned_only).
    /// Returns a human-readable violation per offending package. Snapshot- and
    /// vulnerability-based rules are enforced by the caller (they need runtime state).
    pub fn check_specs(&self, desired: &HashMap<String, Vec<PackageSpec>>) -> Vec<String> {
        let mut violations = Vec::new();
        for specs in desired.values() {
            for s in specs {
                if self
                    .deny_packages
                    .iter()
                    .any(|d| d.eq_ignore_ascii_case(&s.name))
                {
                    violations.push(format!(
                        "{}:{} — denied by policy (deny_packages)",
                        s.backend, s.name
                    ));
                }
                if !self.allow_backends.is_empty()
                    && !self.allow_backends.iter().any(|b| b == &s.backend)
                {
                    violations.push(format!(
                        "{}:{} — backend '{}' is not in allow_backends",
                        s.backend, s.name, s.backend
                    ));
                }
                if self.pinned_only {
                    let pinned = s
                        .options
                        .get("version")
                        .map(|v| !v.is_empty() && v != "latest" && v != "*")
                        .unwrap_or(false);
                    if !pinned {
                        violations.push(format!(
                            "{}:{} — pinned_only requires an explicit version",
                            s.backend, s.name
                        ));
                    }
                }
            }
        }
        violations
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(backend: &str, name: &str, version: Option<&str>) -> PackageSpec {
        let mut options = HashMap::new();
        if let Some(v) = version {
            options.insert("version".to_string(), v.to_string());
        }
        PackageSpec {
            name: name.into(),
            backend: backend.into(),
            options,
            requires: vec![],
        }
    }

    fn desired(specs: Vec<PackageSpec>) -> HashMap<String, Vec<PackageSpec>> {
        let mut m: HashMap<String, Vec<PackageSpec>> = HashMap::new();
        for s in specs {
            m.entry(s.backend.clone()).or_default().push(s);
        }
        m
    }

    #[test]
    fn deny_packages_is_case_insensitive() {
        let pol = Policy {
            deny_packages: vec!["LeftPad".into()],
            ..Default::default()
        };
        let v = pol.check_specs(&desired(vec![spec("npm", "leftpad", None)]));
        assert_eq!(v.len(), 1);
        assert!(v[0].contains("deny_packages"));
    }

    #[test]
    fn allow_backends_blocks_others_but_empty_allows_all() {
        let pol = Policy {
            allow_backends: vec!["apt".into()],
            ..Default::default()
        };
        let v = pol.check_specs(&desired(vec![spec("cargo", "ripgrep", Some("1.0"))]));
        assert_eq!(v.len(), 1);
        assert!(v[0].contains("allow_backends"));

        let open = Policy::default();
        assert!(open
            .check_specs(&desired(vec![spec("cargo", "ripgrep", None)]))
            .is_empty());
    }

    #[test]
    fn pinned_only_requires_concrete_version() {
        let pol = Policy {
            pinned_only: true,
            ..Default::default()
        };
        let v = pol.check_specs(&desired(vec![
            spec("apt", "curl", None),           // no version -> violation
            spec("apt", "wget", Some("latest")), // floating -> violation
            spec("apt", "jq", Some("1.6")),      // pinned -> ok
        ]));
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn empty_policy_is_noop() {
        assert!(Policy::default().is_empty());
    }
}
