//! The removal guard: the last check between a plan and a purged system.
//!
//! Drift removal is derived from managed state, and managed state can be wrong — a
//! mis-scoped manifest, a bad adoption, a state file from another machine. When it is
//! wrong the planner does not produce a *small* mistake; it schedules every managed
//! package for removal and the engine carries it out one purge at a time.
//!
//! Two rules shape this module:
//!
//! 1. *Every* path that deletes is guarded, not just the reviewed ones. A guard on one
//!    command is a guard on nothing: the bug that motivated this arrived through `prune`,
//!    which nobody thought to check.
//! 2. `--yes` never overrides it. `-y` means "don't ask me questions", which every script
//!    and CI job passes; it must not also mean "yes, purge the system". The dedicated
//!    `--allow-mass-removal` is the only override, and it cannot be set permanently in
//!    config.

use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::core::{Error, Result};
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{debug, warn};

/// Which command is asking. Passed explicitly rather than inferred, so every caller has
/// to declare itself — a new deletion path cannot quietly inherit someone else's
/// exemption — and so a refusal can name what refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardScope {
    Apply,
    Prune,
    Sync,
    Watch,
    Upgrade,
    Rollback,
    Canary,
    Remove,
    ShellExit,
    Leases,
}

impl GuardScope {
    /// The command name a user would recognize, for messages.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Apply => "apply",
            Self::Prune => "prune",
            Self::Sync => "sync",
            Self::Watch => "watch",
            Self::Upgrade => "upgrade",
            Self::Rollback => "rollback",
            Self::Canary => "canary",
            Self::Remove => "remove",
            Self::ShellExit => "shell-exit",
            Self::Leases => "leases",
        }
    }

}

/// Why a single package may not be removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Protection {
    /// A `protected_packages` rule matched. Carries the rule so a refusal can cite it.
    Rule(String),
    /// The backend reports the OS itself treats this as essential.
    OsEssential(String),
}

impl Protection {
    pub fn reason(&self) -> String {
        match self {
            Self::Rule(rule) => format!("protected by config rule `{}`", rule),
            Self::OsEssential(backend) => {
                format!("{} reports it as essential to the system", backend)
            }
        }
    }
}

/// The single decision function: may `name` be removed from `backend`?
///
/// Everything that asks "is this protected?" must route through here — the `protected`
/// command included. When the inspector and the enforcer answer separately they drift
/// apart, and an inspector that contradicts the guard is worse than none, because it is
/// believed.
///
/// `os_essential` holds `backend:name` keys the OS flagged; pass an empty set when that
/// is unknown or irrelevant.
pub fn protection_of(
    config: &Config,
    backend: &str,
    name: &str,
    os_essential: &HashSet<String>,
) -> Option<Protection> {
    // An explicit un-protect entry wins over everything, including the OS's own essential
    // flag. It is the user saying "I know, I manage this one myself", and nothing should
    // be able to overrule that — otherwise the escape hatch does not open for exactly the
    // packages someone would need it for.
    if config.unprotect_rule(name).is_some() {
        return None;
    }
    if let Some(rule) = config.protection_rule(name) {
        return Some(Protection::Rule(rule.to_string()));
    }
    if os_essential.contains(&format!("{}:{}", backend, name)) {
        return Some(Protection::OsEssential(backend.to_string()));
    }
    None
}

/// A removal the guard objects to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Objection {
    Protected { key: String, reason: String },
    TooMany { count: usize, limit: usize },
    /// The plan installs more packages at once than `max_installs` allows (II.10). The
    /// install-side twin of `TooMany`: a mis-globbed manifest schedules a flood of
    /// installs, and the count is the fact that explains it.
    TooManyInstalls { count: usize, limit: usize },
}

/// The guard's verdict over a removal set.
#[derive(Debug, Default, Clone)]
pub struct GuardReport {
    pub objections: Vec<Objection>,
}

/// How many individual packages a refusal names before summarizing. A mass-removal plan
/// would otherwise print hundreds of lines above the one number that explains it.
const MAX_LISTED: usize = 10;

impl GuardReport {
    pub fn is_empty(&self) -> bool {
        self.objections.is_empty()
    }

    /// A refusal that says what is wrong and how to proceed. Leads with the count, since
    /// that is the fact that explains the rest.
    pub fn message(&self, scope: GuardScope) -> String {
        let mut out = format!("{}: refusing this removal.\n", scope.as_str());

        if let Some(Objection::TooMany { count, limit }) = self
            .objections
            .iter()
            .find(|o| matches!(o, Objection::TooMany { .. }))
        {
            out.push_str(&format!(
                "  - it removes {} packages, over the limit of {} (config: max_removals)\n",
                count, limit
            ));
        }

        let protected: Vec<&Objection> = self
            .objections
            .iter()
            .filter(|o| matches!(o, Objection::Protected { .. }))
            .collect();
        for o in protected.iter().take(MAX_LISTED) {
            if let Objection::Protected { key, reason } = o {
                out.push_str(&format!("  - {} would be removed ({})\n", key, reason));
            }
        }
        if protected.len() > MAX_LISTED {
            out.push_str(&format!(
                "  - …and {} more protected package(s)\n",
                protected.len() - MAX_LISTED
            ));
        }

        out.push_str(
            "\nThis usually means managed state has drifted from your manifests — run \
             `linix plan` and read it before proceeding.\n\n\
             What to do:\n  \
             linix protected <pkg>          why a package is guarded\n  \
             linix unmanage <pkg>           stop managing it WITHOUT uninstalling it\n  \
             <command> --allow-mass-removal carry out this removal anyway\n  \
             unprotected_packages           exempt a package permanently (config.toml)",
        );
        out
    }
}

/// Names the OS itself reports as essential, per backend, for the backends being removed
/// from. Queried live so it tracks the running system rather than a list we maintain.
/// A backend that cannot answer contributes nothing and never blocks the guard.
pub async fn essential_names(
    registry: &Arc<BackendRegistry>,
    backends: &HashSet<String>,
) -> HashSet<String> {
    let mut out = HashSet::new();
    for name in backends {
        let Some(backend) = registry.get(name) else {
            continue;
        };
        let Some(q) = backend.as_queryable() else {
            continue;
        };
        match q.essential().await {
            Ok(names) => {
                debug!(
                    "Guard: backend '{}' reports {} essential package(s).",
                    name,
                    names.len()
                );
                out.extend(names.into_iter().map(|n| format!("{}:{}", name, n)));
            }
            Err(e) => {
                // Not fatal: the protected list and the count limit still apply.
                warn!("Guard: backend '{}' essential query failed: {}", name, e);
            }
        }
    }
    out
}

/// Inspect a removal set and report what disqualifies it. `removals` are
/// `(backend, name)` pairs. An empty report means the plan may proceed.
pub async fn inspect(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    removals: &[(String, String)],
) -> GuardReport {
    let mut report = GuardReport::default();
    if removals.is_empty() {
        return report;
    }

    let backends: HashSet<String> = removals.iter().map(|(b, _)| b.clone()).collect();
    let os_essential = essential_names(registry, &backends).await;

    for (backend, name) in removals {
        if let Some(p) = protection_of(config, backend, name, &os_essential) {
            report.objections.push(Objection::Protected {
                key: format!("{}:{}", backend, name),
                reason: p.reason(),
            });
        }
    }

    if config.max_removals > 0 && removals.len() > config.max_removals {
        report.objections.push(Objection::TooMany {
            count: removals.len(),
            limit: config.max_removals,
        });
    }

    report
}

/// Enforce the guard for `scope`. `Ok(())` means the removal may proceed.
///
/// The override is `config.allow_mass_removal` (the `--allow-mass-removal` flag), never
/// `--yes`.
pub async fn enforce(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    removals: &[(String, String)],
    scope: GuardScope,
) -> Result<()> {
    let mut report = inspect(config, registry, removals).await;

    // II.10: `--allow-mass-removal` answers exactly one refusal — the count. It used to
    // clear every objection, so the flag for "yes, 50 packages is what I meant" also
    // deleted python3. A confirmation asks; a refusal says no, and protection is a
    // refusal (V.26): nothing overrides it.
    if config.allow_mass_removal {
        let before = report.objections.len();
        report
            .objections
            .retain(|o| !matches!(o, Objection::TooMany { .. }));
        if before != report.objections.len() {
            warn!(
                "Guard: the removal count for '{}' was allowed by --allow-mass-removal.",
                scope.as_str()
            );
        }
    }

    if report.is_empty() {
        return Ok(());
    }
    Err(Error::Other(report.message(scope)))
}

/// Refuse an oversized install set (II.10). The install-side twin of the count check in
/// [`enforce`]: `max_installs` catches a manifest that accidentally globs its way into tens
/// of thousands of installs. `Ok(())` means the install may proceed.
///
/// The override is `config.allow_mass_install` (`--allow-mass-install`), never `--yes` —
/// the same rule the removal ceiling follows, and for the same reason: `-y` is what every
/// script passes.
///
/// Unlike removals, installs have no protection or OS-essential dimension — nothing is
/// *installed* that the system forbids here — so the only question is the count, and `0`
/// (unset) disables it.
pub async fn enforce_installs(config: &Config, count: usize, scope: GuardScope) -> Result<()> {
    if config.max_installs == 0 || count <= config.max_installs {
        return Ok(());
    }
    if config.allow_mass_install {
        warn!(
            "Guard: the install count for '{}' ({}) was allowed by --allow-mass-install.",
            scope.as_str(),
            count
        );
        return Ok(());
    }

    Err(Error::Other(format!(
        "{}: refusing this install.\n  \
         - it installs {} packages, over the limit of {} (config: max_installs)\n\n\
         This usually means a manifest matched more than you meant — run `linix plan` and \
         read the counts before proceeding.\n\n\
         What to do:\n  \
         linix plan                     see exactly what would be installed\n  \
         {} --allow-mass-install carry out this install anyway",
        scope.as_str(),
        count,
        config.max_installs,
        scope.as_str(),
    )))
}

/// Enforce for `purge-unmanaged`, where the count is not the question (II.11).
///
/// `max_removals` catches accidents, and this command is the opposite of an accident: you
/// typed its name and confirmed it. **`protected_packages` and OS-essential still apply** —
/// those are not "are you sure", and the ratio check (II.11) is what asks whether you meant
/// it at all.
pub async fn enforce_deliberate(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    removals: &[(String, String)],
    scope: GuardScope,
) -> Result<()> {
    let mut report = inspect(config, registry, removals).await;
    report
        .objections
        .retain(|o| !matches!(o, Objection::TooMany { .. }));
    if report.is_empty() {
        return Ok(());
    }
    Err(Error::Other(report.message(scope)))
}

/// Pull the `(backend, name)` removal pairs out of a planned change set.
pub fn removal_pairs(changes: &super::planner::SyncChanges) -> Vec<(String, String)> {
    use crate::core::GraphAction;
    changes
        .graph
        .node_weights()
        .filter_map(|w| match w {
            GraphAction::Remove { name, backend } => Some((backend.clone(), name.clone())),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(names: &[&str]) -> Vec<(String, String)> {
        names
            .iter()
            .map(|n| ("apt".to_string(), n.to_string()))
            .collect()
    }

    fn config_with(max: usize) -> Config {
        Config {
            protected_packages: vec!["python3".into(), "libpam*".into()],
            unprotected_packages: Vec::new(),
            max_removals: max,
            ..Config::default()
        }
    }

    #[tokio::test]
    async fn small_ordinary_removal_is_allowed() {
        let reg = Arc::new(BackendRegistry::new());
        let report = inspect(&config_with(20), &reg, &pairs(&["jq", "htop"])).await;
        assert!(report.is_empty(), "{:?}", report.objections);
    }

    #[tokio::test]
    async fn protected_package_is_refused_even_when_alone() {
        // The count limit cannot catch this one: it is a single removal.
        let reg = Arc::new(BackendRegistry::new());
        let report = inspect(&config_with(20), &reg, &pairs(&["python3"])).await;
        assert!(matches!(
            report.objections.as_slice(),
            [Objection::Protected { .. }]
        ));
    }

    #[tokio::test]
    async fn mass_removal_is_refused_even_when_nothing_is_protected() {
        let reg = Arc::new(BackendRegistry::new());
        let many: Vec<String> = (0..30).map(|i| format!("pkg{}", i)).collect();
        let refs: Vec<&str> = many.iter().map(|s| s.as_str()).collect();
        let report = inspect(&config_with(20), &reg, &pairs(&refs)).await;
        assert!(matches!(
            report.objections.as_slice(),
            [Objection::TooMany {
                count: 30,
                limit: 20
            }]
        ));
    }

    #[tokio::test]
    async fn max_removals_zero_disables_the_count_check() {
        let reg = Arc::new(BackendRegistry::new());
        let many: Vec<String> = (0..500).map(|i| format!("pkg{}", i)).collect();
        let refs: Vec<&str> = many.iter().map(|s| s.as_str()).collect();
        assert!(inspect(&config_with(0), &reg, &pairs(&refs))
            .await
            .is_empty());
    }

    #[test]
    fn unprotect_wins_over_a_config_rule() {
        let mut cfg = config_with(20);
        cfg.unprotected_packages = vec!["libpam-modules".into()];
        let none = HashSet::new();
        // libpam* still protects the rest of the family...
        assert!(protection_of(&cfg, "apt", "libpam0g", &none).is_some());
        // ...but the explicit opt-out wins for the one the user named.
        assert!(protection_of(&cfg, "apt", "libpam-modules", &none).is_none());
    }

    #[test]
    fn unprotect_wins_over_the_os_essential_flag() {
        // The documented promise: un-protect beats *everything*, OS flags included.
        // Previously the OS check ran in an `else if` and fired anyway.
        let mut cfg = config_with(20);
        cfg.unprotected_packages = vec!["dash".into()];
        let os: HashSet<String> = ["apt:dash".to_string()].into_iter().collect();
        assert!(protection_of(&cfg, "apt", "dash", &os).is_none());
        // An essential package the user did NOT exempt is still protected.
        let os2: HashSet<String> = ["apt:base-files".to_string()].into_iter().collect();
        assert!(protection_of(&cfg, "apt", "base-files", &os2).is_some());
    }

    #[tokio::test]
    async fn yes_does_not_override_the_guard() {
        // The whole point: -y is what every script passes. It must not mean "purge".
        let reg = Arc::new(BackendRegistry::new());
        let mut cfg = config_with(20);
        cfg.yes = true;
        assert!(enforce(&cfg, &reg, &pairs(&["python3"]), GuardScope::Apply)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn allow_mass_removal_answers_the_count_and_nothing_else() {
        // II.10: `--allow-mass-removal` is the answer to ONE refusal — the count. It used
        // to clear every objection, so the flag meaning "yes, 50 is what I meant" also
        // deleted python3. A confirmation asks; a refusal says no (V.26).
        let reg = Arc::new(BackendRegistry::new());
        let mut cfg = config_with(2);
        cfg.allow_mass_removal = true;

        // The count alone: allowed, because that is what the flag is for.
        assert!(
            enforce(&cfg, &reg, &pairs(&["jq", "htop", "bat"]), GuardScope::Sync)
                .await
                .is_ok(),
            "the flag must let a big-but-ordinary removal through"
        );

        // A protected package, even when the flag is set and the count is fine.
        assert!(
            enforce(&cfg, &reg, &pairs(&["python3"]), GuardScope::Sync)
                .await
                .is_err(),
            "nothing overrides protection — not even --allow-mass-removal"
        );

        // And a big removal that also touches a protected package is still refused.
        assert!(
            enforce(
                &cfg,
                &reg,
                &pairs(&["jq", "htop", "bat", "python3"]),
                GuardScope::Sync
            )
            .await
            .is_err(),
            "the flag must not carry a protected package in on the back of the count"
        );
    }

    #[tokio::test]
    async fn no_setting_can_opt_a_command_out_of_the_guard() {
        // `[guard.enforce_on]` used to do exactly this: a config key that switched the
        // guard off per command, so `enforce_on.sync = false` — copied from a dotfiles repo
        // — made a routine sync remove python3. II.10 lists nine refusals and that was not
        // one of them; V.21 says no setting anyone can flip makes sync dangerous.
        let reg = Arc::new(BackendRegistry::new());
        let cfg = config_with(20);
        for scope in [
            GuardScope::Apply,
            GuardScope::Sync,
            GuardScope::Prune,
            GuardScope::Watch,
            GuardScope::Upgrade,
            GuardScope::Rollback,
            GuardScope::Canary,
            GuardScope::Remove,
            GuardScope::ShellExit,
            GuardScope::Leases,
        ] {
            assert!(
                enforce(&cfg, &reg, &pairs(&["python3"]), scope).await.is_err(),
                "{:?} must be guarded, and nothing may turn that off",
                scope
            );
        }
    }

    #[tokio::test]
    async fn a_deliberate_purge_ignores_the_count_but_never_protection() {
        // II.11: `max_removals` catches accidents, and `purge-unmanaged` is the opposite of
        // an accident — you typed its name. `protected_packages` and OS-essential still
        // apply, and the ratio check is what asks whether you meant it at all.
        let reg = Arc::new(BackendRegistry::new());
        let cfg = config_with(2);

        assert!(
            enforce_deliberate(&cfg, &reg, &pairs(&["a", "b", "c", "d"]), GuardScope::Prune)
                .await
                .is_ok(),
            "the count is not the question here"
        );
        assert!(
            enforce_deliberate(&cfg, &reg, &pairs(&["python3"]), GuardScope::Prune)
                .await
                .is_err(),
            "protection still applies to a deliberate purge"
        );
    }

    #[tokio::test]
    async fn install_ceiling_is_off_by_default() {
        // max_installs defaults to 0 (unset). Installs are additive and far less dangerous
        // than removals, so the ceiling stays off until a user asks for it.
        let cfg = config_with(20); // max_installs is 0 here
        assert!(enforce_installs(&cfg, 10_000, GuardScope::Sync).await.is_ok());
    }

    #[tokio::test]
    async fn install_over_the_ceiling_is_refused() {
        let mut cfg = config_with(20);
        cfg.max_installs = 50;
        let err = enforce_installs(&cfg, 51, GuardScope::Sync)
            .await
            .expect_err("51 installs over a limit of 50 must be refused");
        let msg = err.to_string();
        assert!(msg.contains("installs 51 packages"), "{}", msg);
        assert!(msg.contains("max_installs"), "{}", msg);
        assert!(msg.contains("--allow-mass-install"), "{}", msg);
    }

    #[tokio::test]
    async fn install_at_the_ceiling_is_allowed() {
        // The limit is inclusive: exactly `max_installs` is fine; over it is not.
        let mut cfg = config_with(20);
        cfg.max_installs = 50;
        assert!(enforce_installs(&cfg, 50, GuardScope::Sync).await.is_ok());
    }

    #[tokio::test]
    async fn allow_mass_install_clears_the_install_ceiling() {
        // Symmetric to --allow-mass-removal answering the removal count.
        let mut cfg = config_with(20);
        cfg.max_installs = 50;
        cfg.allow_mass_install = true;
        assert!(enforce_installs(&cfg, 5_000, GuardScope::Sync).await.is_ok());
    }

    #[tokio::test]
    async fn yes_does_not_override_the_install_ceiling() {
        // -y is what every script passes; it must not green-light a manifest-globbed flood.
        let mut cfg = config_with(20);
        cfg.max_installs = 50;
        cfg.yes = true;
        assert!(enforce_installs(&cfg, 5_000, GuardScope::Sync).await.is_err());
    }

    #[test]
    fn refusal_message_leads_with_the_count_and_caps_the_list() {
        let objections = (0..25)
            .map(|i| Objection::Protected {
                key: format!("apt:pkg{}", i),
                reason: "protected by config rule `x`".into(),
            })
            .chain(std::iter::once(Objection::TooMany {
                count: 25,
                limit: 20,
            }))
            .collect();
        let msg = GuardReport { objections }.message(GuardScope::Prune);
        let count_line = msg.find("removes 25 packages").expect("count line present");
        let first_pkg = msg.find("apt:pkg0").expect("a package listed");
        assert!(count_line < first_pkg, "the count must lead");
        assert!(msg.contains("…and 15 more"), "the list must be capped");
    }
}
