// src/app/sync/saved_plan.rs
//
// A saved, reviewable plan artifact — Terraform's `plan -out` / `apply plan` for packages.
// `linix plan --out p.json` freezes exactly what a sync would do; `linix apply p.json`
// executes that captured set. A content hash lets `apply` detect that the world changed
// since the plan was captured.

use crate::app::sync::planner::SyncChanges;
use crate::core::{GraphAction, PackageSpec};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Bump when the on-disk plan format changes incompatibly.
pub const PLAN_SCHEMA: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PlanRemoval {
    pub backend: String,
    pub name: String,
}

/// A frozen, serializable sync plan.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SavedPlan {
    pub schema: u32,
    /// Unix seconds the plan was captured (passed in — no clock in the pure builder).
    pub created_at: Option<i64>,
    /// Stable content hash of the operations, for drift detection at apply time.
    pub desired_hash: String,
    pub installs: Vec<PackageSpec>,
    pub removals: Vec<PlanRemoval>,
}

impl SavedPlan {
    /// Freeze computed sync changes into a saved plan.
    pub fn from_changes(changes: &SyncChanges, created_at: Option<i64>) -> Self {
        let mut installs = Vec::new();
        let mut removals = Vec::new();
        for w in changes.graph.node_weights() {
            match w {
                GraphAction::Install(spec) => installs.push(spec.clone()),
                GraphAction::Remove { name, backend } => removals.push(PlanRemoval {
                    backend: backend.clone(),
                    name: name.clone(),
                }),
            }
        }
        let desired_hash = hash_plan(&installs, &removals);
        Self {
            schema: PLAN_SCHEMA,
            created_at,
            desired_hash,
            installs,
            removals,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.installs.is_empty() && self.removals.is_empty()
    }

    /// Recompute the hash from the plan's own contents (to detect a hand-edited file).
    pub fn recomputed_hash(&self) -> String {
        hash_plan(&self.installs, &self.removals)
    }
}

/// Stable content hash over the plan's operations. Order-independent; ignores internal
/// `__`-prefixed provenance options so they don't perturb equality. Pure — unit tested.
pub fn hash_plan(installs: &[PackageSpec], removals: &[PlanRemoval]) -> String {
    let mut keys: Vec<String> = Vec::new();
    for s in installs {
        let mut opts: Vec<String> = s
            .options
            .iter()
            .filter(|(k, _)| !k.starts_with("__"))
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        opts.sort();
        keys.push(format!("I:{}:{}|{}", s.backend, s.name, opts.join(",")));
    }
    for r in removals {
        keys.push(format!("R:{}:{}", r.backend, r.name));
    }
    keys.sort();

    let mut h = Sha256::new();
    for k in &keys {
        h.update(k.as_bytes());
        h.update(b"\n");
    }
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn spec(backend: &str, name: &str, opt: Option<(&str, &str)>) -> PackageSpec {
        let mut options = HashMap::new();
        if let Some((k, v)) = opt {
            options.insert(k.to_string(), v.to_string());
        }
        PackageSpec {
            name: name.into(),
            backend: backend.into(),
            options,
            requires: vec![],
            present: true,
        }
    }

    #[test]
    fn hash_is_order_independent() {
        let a = vec![spec("apt", "htop", None), spec("cargo", "ripgrep", None)];
        let b = vec![spec("cargo", "ripgrep", None), spec("apt", "htop", None)];
        assert_eq!(hash_plan(&a, &[]), hash_plan(&b, &[]));
    }

    #[test]
    fn hash_ignores_internal_provenance_options() {
        let a = vec![spec("apt", "htop", Some(("__source", "local.txt")))];
        let b = vec![spec("apt", "htop", Some(("__source", "module:dev")))];
        assert_eq!(hash_plan(&a, &[]), hash_plan(&b, &[]));
    }

    #[test]
    fn hash_changes_with_real_options_and_removals() {
        let base = vec![spec("apt", "htop", None)];
        let versioned = vec![spec("apt", "htop", Some(("version", "3.0")))];
        assert_ne!(hash_plan(&base, &[]), hash_plan(&versioned, &[]));

        let with_removal = hash_plan(
            &base,
            &[PlanRemoval {
                backend: "apt".into(),
                name: "vim".into(),
            }],
        );
        assert_ne!(hash_plan(&base, &[]), with_removal);
    }
}
