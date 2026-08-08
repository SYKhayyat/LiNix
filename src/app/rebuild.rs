//! `linix rebuild` — assert the declared set from scratch (X.1).
//!
//! Convergence is blind to state that is wrong while the difference is empty: a half-configured
//! package, an interrupted extraction, a closure whose dependencies were removed by hand. In
//! every such case the machine and the declaration agree at the level `sync` inspects, so no
//! amount of re-running it changes anything. Rebuild stops asking what changed and removes the
//! declared packages so it can install them again.
//!
//! Everything here is pure: scope selection and batch ordering take facts and return a plan.
//! The applying half lives in the caller, which holds the registry and the sync engine.

use crate::core::PackageSpec;

/// What to rebuild. A bare `rebuild` defaults to `All` (K2, ruled 2026-07-24) — but the CLI
/// warns loudly before it does, because the failure mode is software missing from a machine and
/// `--all` is a large thing to reach by pressing enter. The warning is the safeguard the old
/// "refuse and list the forms" design used a refusal for; the owner chose warn-and-proceed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Packages(Vec<Target>),
    Backend(String),
    All,
}

/// A `linix rebuild NAME` argument, after the one parser has had it.
///
/// Split where the backend registry can be consulted rather than at the point of use: a
/// `next_back()` on `:` turns `web:https://host/x.tar.gz` into `//host/x.tar.gz`, and never
/// checks that the prefix names a backend at all (C13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub backend: Option<String>,
    pub name: String,
    /// What the user typed, for the message that names it back to them.
    pub raw: String,
}

impl Target {
    pub fn parse(input: &str, is_known_backend: impl Fn(&str) -> bool) -> Self {
        let (backend, name) = crate::config::parser::split_removal_target(input, is_known_backend);
        Self {
            backend,
            name,
            raw: input.to_string(),
        }
    }

    fn matches(&self, spec: &PackageSpec) -> bool {
        self.name == spec.name && self.backend.as_ref().is_none_or(|b| b == &spec.backend)
    }
}

/// A package left out of the rebuild, and the sentence explaining it. Printed, never silent:
/// a rebuild that quietly skipped half its scope would report success over an unrepaired
/// machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub key: String,
    pub reason: String,
}

/// One backend's packages: all of them come down, then all of them go back up (K1).
#[derive(Debug, Clone)]
pub struct Batch {
    pub backend: String,
    pub specs: Vec<PackageSpec>,
}

impl Batch {
    pub fn names(&self) -> Vec<String> {
        self.specs.iter().map(|s| s.name.clone()).collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub batches: Vec<Batch>,
    pub skipped: Vec<Skipped>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.batches.iter().all(|b| b.specs.is_empty())
    }

    pub fn total(&self) -> usize {
        self.batches.iter().map(|b| b.specs.len()).sum()
    }
}

/// Whether this backend owns the shell and the system libraries.
///
/// `needs_root()` already draws exactly this line — a manager that must be root to install is
/// one installing into the system, and one that need not is installing into a home directory.
/// A second list of "system backends" would be a second answer to a question already answered.
pub type IsFoundation<'a> = &'a dyn Fn(&str) -> bool;

/// Foundation backends first, then the rest, each tier keeping `priority` order (II.6).
///
/// Dependency direction is what fixes this, not blast radius: a `cargo` crate can need a
/// system compiler, and no `apt` package has ever needed a crate. Rebuilding user-space
/// software first would rebuild it against the system state the rebuild is about to replace,
/// leaving it stale the moment the foundation batch lands.
pub fn order_backends(
    backends: &[String],
    priority: &[String],
    is_foundation: IsFoundation,
) -> Vec<String> {
    let rank = |b: &String| priority.iter().position(|p| p == b).unwrap_or(usize::MAX);

    let mut ordered: Vec<String> = backends.to_vec();
    ordered.sort_by_key(|b| (!is_foundation(b), rank(b), b.clone()));
    ordered.dedup();
    ordered
}

/// Which declared packages the scope selects, batched by backend and ordered.
///
/// `declared` is the resolved desired state; `installed` answers whether LiNix currently
/// manages that package. A declared package that is not installed is left to `sync` — putting
/// it in a rebuild would mean removing something that is not there in order to install it,
/// which is an install, and `sync` is the command for installs.
pub fn plan(
    scope: &Scope,
    declared: &[PackageSpec],
    installed: &dyn Fn(&str, &str) -> bool,
    priority: &[String],
    is_foundation: IsFoundation,
) -> Plan {
    let mut plan = Plan::default();

    let selected: Vec<&PackageSpec> = declared
        .iter()
        .filter(|s| s.present)
        .filter(|s| match scope {
            Scope::All => true,
            Scope::Backend(b) => &s.backend == b,
            Scope::Packages(targets) => targets.iter().any(|t| t.matches(s)),
        })
        .collect();

    // A named package that is not declared is the user believing something is managed when it
    // is not. Rebuild must say so rather than silently rebuild nothing.
    if let Scope::Packages(targets) = scope {
        for target in targets {
            if !selected.iter().any(|s| target.matches(s)) {
                plan.skipped.push(Skipped {
                    key: target.raw.clone(),
                    reason: "not declared in any active module".to_string(),
                });
            }
        }
    }

    let mut by_backend: std::collections::HashMap<String, Vec<PackageSpec>> = Default::default();
    for spec in selected {
        if !installed(&spec.backend, &spec.name) {
            plan.skipped.push(Skipped {
                key: format!("{}:{}", spec.backend, spec.name),
                reason: "declared but not installed — `sync` installs it".to_string(),
            });
            continue;
        }
        by_backend
            .entry(spec.backend.clone())
            .or_default()
            .push(spec.clone());
    }

    let names: Vec<String> = by_backend.keys().cloned().collect();
    for backend in order_backends(&names, priority, is_foundation) {
        let mut specs = by_backend.remove(&backend).unwrap_or_default();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        plan.batches.push(Batch { backend, specs });
    }

    plan.skipped.sort_by(|a, b| a.key.cmp(&b.key));
    plan
}

/// Drop packages the guard would refuse to remove, recording each.
///
/// Rebuild removes in order to reinstall, but the guard cannot know that, and it is right not
/// to trust it: if the reinstall fails, the machine is genuinely without the package. Narrowing
/// the scope here keeps `rebuild --all` usable on a machine whose `bash` is protected, without
/// weakening the refusal or asking the guard to make an exception for one caller.
pub fn without_protected(plan: &mut Plan, protection: &dyn Fn(&str, &str) -> Option<String>) {
    for batch in &mut plan.batches {
        batch
            .specs
            .retain(|spec| match protection(&batch.backend, &spec.name) {
                Some(reason) => {
                    plan.skipped.push(Skipped {
                        key: format!("{}:{}", batch.backend, spec.name),
                        reason,
                    });
                    false
                }
                None => true,
            });
    }
    plan.batches.retain(|b| !b.specs.is_empty());
    plan.skipped.sort_by(|a, b| a.key.cmp(&b.key));
}

#[cfg(test)]
mod tests {
    use super::*;
    

    fn spec(backend: &str, name: &str) -> PackageSpec {
        PackageSpec {
            name: name.to_string(),
            backend: backend.to_string(),
            options: Default::default(),
            requires: Vec::new(),
            present: true,
        }
    }

    fn foundation(b: &str) -> bool {
        matches!(b, "apt" | "dnf" | "pacman")
    }

    fn all_installed(_: &str, _: &str) -> bool {
        true
    }

    fn targets(raw: &[&str]) -> Vec<Target> {
        raw.iter()
            .map(|s| Target::parse(s, |b| matches!(b, "apt" | "cargo" | "web")))
            .collect()
    }

    #[test]
    fn the_foundation_goes_first_whatever_priority_says() {
        let backends = vec!["cargo".to_string(), "apt".to_string()];
        let priority = vec!["cargo".to_string(), "apt".to_string()];
        assert_eq!(
            order_backends(&backends, &priority, &foundation),
            vec!["apt".to_string(), "cargo".to_string()],
            "a crate can need a system compiler; no apt package needs a crate"
        );
    }

    #[test]
    fn within_a_tier_priority_decides() {
        let backends = vec!["npm".to_string(), "cargo".to_string()];
        let priority = vec!["cargo".to_string(), "npm".to_string()];
        assert_eq!(
            order_backends(&backends, &priority, &foundation),
            vec!["cargo".to_string(), "npm".to_string()]
        );
    }

    #[test]
    fn a_backend_missing_from_priority_still_gets_an_order() {
        // Falling back to usize::MAX puts it last rather than dropping it; sorting by name
        // after that keeps the run reproducible.
        let backends = vec!["zig".to_string(), "npm".to_string(), "cargo".to_string()];
        let priority = vec!["cargo".to_string()];
        assert_eq!(
            order_backends(&backends, &priority, &foundation),
            vec!["cargo".to_string(), "npm".to_string(), "zig".to_string()]
        );
    }

    #[test]
    fn a_batch_holds_one_backend_and_the_batches_are_ordered() {
        let declared = vec![
            spec("cargo", "ripgrep"),
            spec("apt", "curl"),
            spec("cargo", "fd"),
        ];
        let p = plan(
            &Scope::All,
            &declared,
            &all_installed,
            &["apt".into(), "cargo".into()],
            &foundation,
        );
        assert_eq!(p.batches.len(), 2);
        assert_eq!(p.batches[0].backend, "apt");
        assert_eq!(p.batches[1].names(), vec!["fd", "ripgrep"]);
    }

    #[test]
    fn an_absent_line_is_not_rebuilt() {
        // `absent:` declares a package must NOT exist. Removing and reinstalling it would
        // install the one thing the declaration forbids.
        let mut gone = spec("apt", "telnet");
        gone.present = false;
        let p = plan(
            &Scope::All,
            &[gone, spec("apt", "curl")],
            &all_installed,
            &["apt".into()],
            &foundation,
        );
        assert_eq!(p.batches[0].names(), vec!["curl"]);
    }

    #[test]
    fn declared_but_not_installed_is_left_to_sync() {
        let p = plan(
            &Scope::All,
            &[spec("apt", "curl")],
            &|_, name| name != "curl",
            &["apt".into()],
            &foundation,
        );
        assert!(p.is_empty());
        assert_eq!(p.skipped[0].key, "apt:curl");
        assert!(p.skipped[0].reason.contains("sync"));
    }

    #[test]
    fn a_package_nobody_declared_is_named_not_ignored() {
        let p = plan(
            &Scope::Packages(targets(&["nosuch"])),
            &[spec("apt", "curl")],
            &all_installed,
            &["apt".into()],
            &foundation,
        );
        assert!(p.is_empty());
        assert_eq!(p.skipped[0].key, "nosuch");
        assert!(p.skipped[0].reason.contains("not declared"));
    }

    /// `split(':').next_back()` degraded this to `//host/x.tar.gz` and matched nothing —
    /// and never checked the prefix named a backend at all.
    #[test]
    fn a_name_with_colons_in_it_survives_the_split() {
        let t = &targets(&["web:https://host/x.tar.gz"])[0];
        assert_eq!(t.backend.as_deref(), Some("web"));
        assert_eq!(t.name, "https://host/x.tar.gz");
    }

    #[test]
    fn an_unknown_prefix_is_part_of_the_name_not_a_backend() {
        let t = &targets(&["nosuchbackend:fd"])[0];
        assert_eq!(t.backend, None);
        assert_eq!(t.name, "nosuchbackend:fd");
    }

    #[test]
    fn a_qualified_name_selects_only_that_backends_copy() {
        let declared = vec![spec("cargo", "fd"), spec("apt", "fd")];
        let p = plan(
            &Scope::Packages(targets(&["cargo:fd"])),
            &declared,
            &all_installed,
            &["apt".into(), "cargo".into()],
            &foundation,
        );
        assert_eq!(p.batches.len(), 1);
        assert_eq!(p.batches[0].backend, "cargo");
    }

    #[test]
    fn backend_scope_takes_that_backend_whole() {
        let declared = vec![
            spec("cargo", "fd"),
            spec("apt", "curl"),
            spec("cargo", "rg"),
        ];
        let p = plan(
            &Scope::Backend("cargo".into()),
            &declared,
            &all_installed,
            &["apt".into(), "cargo".into()],
            &foundation,
        );
        assert_eq!(p.batches.len(), 1);
        assert_eq!(p.batches[0].names(), vec!["fd", "rg"]);
    }

    #[test]
    fn a_protected_package_is_dropped_and_reported() {
        let mut p = plan(
            &Scope::All,
            &[spec("apt", "bash"), spec("apt", "curl")],
            &all_installed,
            &["apt".into()],
            &foundation,
        );
        without_protected(&mut p, &|_, name| {
            (name == "bash").then(|| "protected by config rule `bash`".to_string())
        });
        assert_eq!(p.batches[0].names(), vec!["curl"]);
        assert_eq!(p.skipped[0].key, "apt:bash");
    }

    #[test]
    fn a_batch_emptied_by_the_guard_is_not_a_batch() {
        // An empty batch would print "rebuilding apt (0 packages)" and run two no-op syncs.
        let mut p = plan(
            &Scope::All,
            &[spec("apt", "bash")],
            &all_installed,
            &["apt".into()],
            &foundation,
        );
        without_protected(&mut p, &|_, _| Some("protected".to_string()));
        assert!(p.batches.is_empty());
        assert!(p.is_empty());
    }
}
