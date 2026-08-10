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

/// The resource half of a frozen plan (N-2): `link:`, `service:`, `setting:`, `shim:`,
/// `schedule:` and `repo:` lines.
///
/// Keys in the extras ledger's own vocabulary (`link:<resolved destination>`,
/// `service:<name>`), because those are what the teardown acts on and what the guard counts —
/// a plan naming resources some other way would be a third vocabulary for one set of things.
///
/// `plan --help` promises that "the exact plan you inspect is the one you later `apply`". It
/// froze `{"installs": [], "removals": []}` over three unapplied `link:` lines and printed
/// `system already matches desired state`, while `--dry-run sync` on the same tree named all
/// three — and the guard's refusal text sends a user here to see what would be undone.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanResources {
    /// Declared and not in effect.
    #[serde(default)]
    pub place: Vec<String>,
    /// Applied before and declared nowhere now.
    #[serde(default)]
    pub undo: Vec<String>,
}

impl PlanResources {
    pub fn is_empty(&self) -> bool {
        self.place.is_empty() && self.undo.is_empty()
    }

    pub fn len(&self) -> usize {
        self.place.len() + self.undo.len()
    }
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
    /// The resources this plan would place and undo. `serde(default)` so a plan written before
    /// the field existed still reads as one with no resource work.
    #[serde(default)]
    pub resources: PlanResources,
    /// The variables this plan resolved against (Part IX, IX.6). `apply` resolves the model
    /// against these rather than running the provider again: a provider may read the clock or
    /// shell out, so a fresh resolution at apply time could disagree with the preview and make
    /// every plan with a moving variable spuriously fail its drift check. Auxiliary to the hash
    /// — the executed operations are `installs`/`removals`, which the hash protects.
    #[serde(default)]
    pub vars: std::collections::BTreeMap<String, crate::model::vars::Value>,
}

impl SavedPlan {
    /// Freeze computed sync changes into a saved plan.
    pub fn from_changes(
        changes: &SyncChanges,
        resources: &crate::app::apply::ResourceChanges,
        created_at: Option<i64>,
    ) -> Self {
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
        let resources = PlanResources {
            place: resources.place.clone(),
            undo: resources.undo.clone(),
        };
        let desired_hash = hash_plan(&installs, &removals, &resources);
        Self {
            schema: PLAN_SCHEMA,
            created_at,
            desired_hash,
            installs,
            removals,
            resources,
            vars: std::collections::BTreeMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.installs.is_empty() && self.removals.is_empty() && self.resources.is_empty()
    }

    /// Recompute the hash from the plan's own contents (to detect a hand-edited file).
    pub fn recomputed_hash(&self) -> String {
        hash_plan(&self.installs, &self.removals, &self.resources)
    }
}

/// Stable content hash over the plan's operations. Order-independent; ignores internal
/// `__`-prefixed provenance options so they don't perturb equality. Pure — unit tested.
pub fn hash_plan(
    installs: &[PackageSpec],
    removals: &[PlanRemoval],
    resources: &PlanResources,
) -> String {
    let mut keys: Vec<String> = Vec::new();
    for s in installs {
        let mut opts: Vec<String> = s
            .options
            .iter()
            .filter(|(k, _)| !k.starts_with("__"))
            .map(|(k, v)| format!("{}={}", k, v.join(",")))
            .collect();
        opts.sort();
        keys.push(format!("I:{}:{}|{}", s.backend, s.name, opts.join(",")));
    }
    for r in removals {
        keys.push(format!("R:{}:{}", r.backend, r.name));
    }
    // In the hash, so a hand-edited resource list is caught by the same integrity check the
    // package lists get. Left out, `apply` would happily place resources nobody reviewed.
    for key in &resources.place {
        keys.push(format!("P:{}", key));
    }
    for key in &resources.undo {
        keys.push(format!("U:{}", key));
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

    fn spec(backend: &str, name: &str, opt: Option<(&str, &str)>) -> PackageSpec {
        let mut options = crate::config::grammar::Options::default();
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
        assert_eq!(
            hash_plan(&a, &[], &PlanResources::default()),
            hash_plan(&b, &[], &PlanResources::default())
        );
    }

    #[test]
    fn hash_ignores_internal_provenance_options() {
        let a = vec![spec("apt", "htop", Some(("__source", "local.txt")))];
        let b = vec![spec("apt", "htop", Some(("__source", "module:dev")))];
        assert_eq!(
            hash_plan(&a, &[], &PlanResources::default()),
            hash_plan(&b, &[], &PlanResources::default())
        );
    }

    #[test]
    fn a_plan_carries_its_variables_across_a_round_trip() {
        use crate::model::vars::Value;
        let mut plan = SavedPlan::from_changes(
            &crate::app::sync::planner::SyncChanges::default(),
            &crate::app::apply::ResourceChanges::default(),
            Some(1),
        );
        plan.vars
            .insert("role".to_string(), Value::Str("travel".into()));
        plan.vars.insert("cores".to_string(), Value::Num(8.0));
        let json = serde_json::to_string(&plan).unwrap();
        let back: SavedPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(back.vars["role"], Value::Str("travel".into()));
        assert_eq!(back.vars["cores"], Value::Num(8.0));
    }

    #[test]
    fn a_plan_with_no_vars_field_deserializes_to_empty() {
        // The field is `serde(default)`, so a plan written before it existed still reads.
        let raw =
            r#"{"schema":1,"created_at":null,"desired_hash":"x","installs":[],"removals":[]}"#;
        let plan: SavedPlan = serde_json::from_str(raw).unwrap();
        assert!(plan.vars.is_empty());
    }

    #[test]
    fn hash_changes_with_real_options_and_removals() {
        let base = vec![spec("apt", "htop", None)];
        let versioned = vec![spec("apt", "htop", Some(("version", "3.0")))];
        assert_ne!(
            hash_plan(&base, &[], &PlanResources::default()),
            hash_plan(&versioned, &[], &PlanResources::default())
        );

        let with_removal = hash_plan(
            &base,
            &[PlanRemoval {
                backend: "apt".into(),
                name: "vim".into(),
            }],
            &PlanResources::default(),
        );
        assert_ne!(
            hash_plan(&base, &[], &PlanResources::default()),
            with_removal
        );
    }
}
