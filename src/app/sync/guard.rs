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

/// Proof that this guard was consulted, and the only thing an effector will remove without.
///
/// **`readme.md:358` promises that every path removing anything goes through one guard. Until
/// this type existed, that sentence was checked by a regex over source text** —
/// `removal_guard_enumeration_tests.rs:91`'s `is_removal_call`, matching `.remove(` with `sudo`
/// on the line, `.remove_repo(`, `.remove_shim(` and `.deprovision(`. `apply/firewall.rs` closes
/// a port with `deny_command`, which matches none of them, and the word `guard` appears nowhere
/// in that file. **The fix for `G-1` replaced a stale list of paths with a stale list of verbs**,
/// and the staleness moved into a predicate with a passing self-test, where nobody re-derives it.
///
/// So the enumeration is the compiler's now. Every effector that removes takes one of these, and
/// the only way to get one is to have asked. Effector six is covered by construction rather than
/// by someone remembering the list.
///
/// This is what `PlanScope` did for planning, applied to removal. `planner.rs:83` states the
/// technique better than this comment can: *"the case that reaps cannot be written without the
/// list that bounds it."*
///
/// **The private field is load-bearing.** `Reaped {}` from outside this module is a compile
/// error, so the token cannot be minted by a caller who would rather not ask.
#[derive(Debug, Clone, Copy)]
pub struct Reaped {
    /// Which command's removal this authorises. Carried so an effector can name it, and so the
    /// token is not silently reusable across two different commands' plans.
    scope: GuardScope,
}

impl Reaped {
    /// What the guard was asked on behalf of.
    pub fn scope(&self) -> GuardScope {
        self.scope
    }

    /// A removal that is not the user's machine changing state.
    ///
    /// The narrow, named escape for the two cases where asking is either impossible or already
    /// done, so that neither has to reach for a wider one:
    ///
    /// - **A test double.** A unit test for an effector is testing the effector, and threading a
    ///   real `Config` and `BackendRegistry` through it to mint a token proves nothing about the
    ///   guard.
    /// - **A rollback compensating its own transaction.** `transaction.rs:993` already calls
    ///   `protection_of` before it removes, deliberately and correctly, and its removals are of
    ///   packages this same run installed seconds ago.
    ///
    /// Named rather than derived, and searchable: `grep -rn "Reaped::for_reason"` is the list of
    /// places that do not ask, which is exactly the list a reviewer wants and exactly what
    /// `is_removal_call` could not produce.
    pub fn for_reason(scope: GuardScope, _why: &'static str) -> Self {
        Reaped { scope }
    }
}

/// Which command is asking. Passed explicitly rather than inferred, so every caller has
/// to declare itself — a new deletion path cannot quietly inherit someone else's
/// exemption — and so a refusal can name what refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardScope {
    Apply,
    RemoveOrphans,
    PurgeUndeclared,
    Sync,
    Watch,
    Upgrade,
    Canary,
    Remove,
    ShellExit,
    ExpirySweep,
    Heal,
    Rebuild,
}

impl GuardScope {
    /// The command a user would recognize, for messages. It has to be what they typed:
    /// a refusal reading "prune refused" to someone running `purge-undeclared` names a
    /// command that does not exist, and gives them nothing to act on.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Apply => "apply",
            Self::RemoveOrphans => "remove-orphans",
            Self::PurgeUndeclared => "purge-undeclared",
            Self::Sync => "sync",
            Self::Watch => "watch",
            Self::Upgrade => "upgrade",
            Self::Canary => "upgrade --canary",
            Self::Remove => "uninstall",
            Self::ShellExit => "shell exit",
            Self::ExpirySweep => "expiry sweep",
            Self::Heal => "heal",
            Self::Rebuild => "rebuild",
        }
    }

    /// Whether a transaction run under this scope is **reconciling the machine against the
    /// manifest** — which decides whether its rollback may leave a removal in place (`U41`).
    ///
    /// A reconciling run removes a package *because nothing declares it*, and that fact is still
    /// true when the rollback fires. Putting it back hands the next sync the same work, which is
    /// the one mechanism in the program that provably un-converges.
    ///
    /// **Two scopes are not reconciling, and each has to be said out loud:**
    ///
    /// - `Rebuild` splits one operation into two transactions so a `Remove` and an `Install` of
    ///   the same package cannot race in one graph. Its removal phase is the first half of a
    ///   reinstall of *declared* packages; leaving one of those removals in place is not
    ///   convergence, it is a machine missing software it still declares.
    /// - `Remove` is a person typing `linix uninstall`. The removal was ordered by hand rather
    ///   than derived from a manifest, so a transaction that failed around it should give the
    ///   package back.
    ///
    /// Exhaustive, and deliberately so: a scope added later must say which it is, because
    /// inheriting `true` here means inheriting "a failed run may leave your software deleted".
    pub fn reconciles(&self) -> bool {
        match self {
            Self::Rebuild | Self::Remove => false,
            Self::Apply
            | Self::RemoveOrphans
            | Self::PurgeUndeclared
            | Self::Sync
            | Self::Watch
            | Self::Upgrade
            | Self::Canary
            | Self::ShellExit
            | Self::ExpirySweep
            | Self::Heal => true,
        }
    }

    /// How a refusal names this run in prose — the `(refused during …)` half of a message.
    ///
    /// Separate from [`as_str`](Self::as_str) because the two answer different questions.
    /// `as_str` is the command to *retype* with a flag on it, so it has to be what the user
    /// typed. This is what the reader needs to understand the refusal, and there the
    /// difference that matters is **whether anybody was there** — `N7` makes an unattended
    /// `watch` tick revert by default, so "refused during watch" is the one phrasing that
    /// buries the fact worth reporting.
    ///
    /// Written out per variant rather than falling back to `as_str` for the rest: a catch-all
    /// arm here is how the label this replaced came to answer `"sync"` for nine of twelve
    /// scopes.
    pub fn during(&self) -> &'static str {
        match self {
            Self::Apply => "an apply",
            Self::RemoveOrphans => "remove-orphans",
            Self::PurgeUndeclared => "purge-undeclared",
            Self::Sync => "sync",
            Self::Watch => "an unattended watch tick",
            Self::Upgrade => "an upgrade",
            Self::Canary => "a canary upgrade",
            Self::Remove => "an uninstall",
            Self::ShellExit => "a shell exit",
            Self::ExpirySweep => "an expiry sweep",
            Self::Heal => "a recovery run",
            Self::Rebuild => "a rebuild",
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
    /// The manager reports a name that cannot be written as a package line, so LiNix can
    /// never declare it — and what it cannot be asked to keep, it must not take away.
    Undeclarable,
    /// The name declares a resource, not a package: a `service:`, `link:` or `setting:`.
    /// `purge-undeclared` deletes packages nobody declared; a service nobody declared is not
    /// something that was installed, it is the state the machine is already in.
    NotAPackage(String),
}

impl Protection {
    pub fn reason(&self) -> String {
        match self {
            Self::Rule(rule) => format!("protected by config rule `{}`", rule),
            Self::OsEssential(backend) => {
                format!("{} reports it as essential to the system", backend)
            }
            Self::Undeclarable => {
                "its manager reports a name no line can hold, so LiNix cannot manage \
                 it — and removing what you cannot declare is not something you asked for"
                    .to_string()
            }
            Self::NotAPackage(backend) => format!(
                "`{}:` declares a state, not an installed package — undeclaring one is a \
                 `sync` of a line you deleted, never a sweep of what you never declared",
                backend
            ),
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
/// `backend` is `None` when the caller does not know one — `linix protected jq`, where the user
/// named no manager. The config rules match on the name alone and are answered; the OS's
/// essential list is keyed by `backend:name` and cannot be, so it is not consulted. **An unknown
/// backend is this case, never the empty string**: `is_declarable("", "jq")` builds the line
/// `:jq`, which no grammar accepts, so every bare name came back `Undeclarable` before a rule
/// was read.
///
/// `os_essential` holds `backend:name` keys the OS flagged; pass an empty set when that
/// is unknown or irrelevant.
pub fn protection_of(
    config: &Config,
    backend: Option<&str>,
    name: &str,
    os_essential: &HashSet<String>,
) -> Option<Protection> {
    // Before the escape hatch, because neither of these is a policy: a name no line can hold
    // cannot be declared, so LiNix never manages it and `unprotected_packages` has nothing
    // to release. Saying yes here would let `purge-undeclared` remove programs that could
    // never have been adopted in the first place.
    //
    // A resource is refused for a different reason, said out loud because it used to be an
    // accident: the declarability test asked whether a *package* line could hold the name,
    // `service:AppMgmt` is not a package line, and so every running service was refused by a
    // check that was not about services and printed a sentence that was false of them. The
    // moment that sentence was corrected — and it was false, `service:AppMgmt` parses — the
    // refusal would have evaporated and `purge-undeclared` could stop and disable every
    // service on the machine. This is that refusal, made on purpose.
    //
    // By backend, not by round-tripping the name: a `setting:` needs `@value=` before any line
    // will hold it, so the name alone would come back undeclarable and print the same false
    // sentence one backend over.
    if let Some(b) = backend {
        if crate::config::grammar::Statement::RESOURCE_BACKENDS.contains(&b) {
            return Some(Protection::NotAPackage(b.to_string()));
        }
    }
    if !crate::config::grammar::is_declarable(backend, name) {
        return Some(Protection::Undeclarable);
    }

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
    if let Some(b) = backend {
        if os_essential.contains(&format!("{}:{}", b, name)) {
            return Some(Protection::OsEssential(b.to_string()));
        }
    }
    None
}

/// A removal the guard objects to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Objection {
    Protected {
        key: String,
        reason: String,
    },
    TooMany {
        count: usize,
        limit: usize,
        /// The `[guard]` key that set `limit`, so the refusal names the number the reader has
        /// to change. Two ceilings answer this objection now (`Y20`) and a message reading
        /// `max_removals` about a port closure sends the reader to the wrong line.
        setting: &'static str,
    },
    /// The plan installs more packages at once than `max_installs` allows (II.10). The
    /// install-side twin of `TooMany`: a mis-globbed manifest schedules a flood of
    /// installs, and the count is the fact that explains it.
    TooManyInstalls {
        count: usize,
        limit: usize,
    },
    /// A desired package is on the `deny_packages` list (II.10) — never install this.
    Denied {
        key: String,
    },
    /// `pinned_only` is set and a desired package has no explicit `@version=` (II.10).
    Unpinned {
        key: String,
    },
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
    pub fn message(&self, scope: GuardScope, kind: RemovalKind) -> String {
        let noun = match kind {
            RemovalKind::Package => "packages",
            RemovalKind::Extra => "managed resources",
        };
        let mut out = format!("{}: refusing this removal.\n", scope.as_str());

        if let Some(Objection::TooMany {
            count,
            limit,
            setting,
        }) = self
            .objections
            .iter()
            .find(|o| matches!(o, Objection::TooMany { .. }))
        {
            out.push_str(&format!(
                "  - it removes {} {}, over the limit of {} ([guard] {})\n",
                count, noun, limit, setting
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
                "  - …and {} more protected {}(s)\n",
                protected.len() - MAX_LISTED,
                match kind {
                    RemovalKind::Package => "package",
                    RemovalKind::Extra => "resource",
                }
            ));
        }

        // The advice has to be executable. `linix unmanage` takes a package line, so offering
        // it for a `link:` teardown names a command that cannot accept the thing it is about;
        // for an extra the equivalent act is putting the declaration back.
        match kind {
            RemovalKind::Package => out.push_str(
                "\nThis usually means managed state has drifted from your manifests — run \
                 `linix plan` and read it before proceeding.\n\n\
                 What to do:\n  \
                 linix protected <pkg>          why a package is guarded\n  \
                 linix unmanage <pkg>           stop managing it WITHOUT uninstalling it\n  \
                 <command> --allow-mass-removal carry out this removal anyway\n  \
                 [guard] unprotected_packages    exempt a package permanently (preferences.toml)",
            ),
            RemovalKind::Extra => out.push_str(
                "\nThese are resources a declaration put in place — a `link:`, `service:`, \
                 `setting:`, `shim:`, `schedule:` or `repo:` line that is no longer in any \
                 module. `sync` undoes what is no longer declared.\n\n\
                 What to do:\n  \
                 linix plan                     see exactly what would be undone\n  \
                 put the line back              if the deletion was not what you meant\n  \
                 <command> --allow-mass-removal carry out this teardown anyway\n  \
                 [guard] unprotected_packages    exempt one permanently (preferences.toml)",
            ),
        }
        out
    }
}

/// Names the OS itself reports as essential, per backend, for the backends being removed
/// from. Queried live so it tracks the running system rather than a list we maintain.
/// A backend that cannot answer contributes nothing and never blocks the guard.
pub async fn essential_names(
    registry: &Arc<BackendRegistry>,
    backends: &HashSet<String>,
    max_parallel: usize,
) -> HashSet<String> {
    // Each `essential()` is a subprocess and they have nothing to say to one another, so they
    // run at once. This is on every removal path.
    use futures::stream::StreamExt;
    futures::stream::iter(backends.iter().cloned())
        .map(|name| {
            let registry = registry.clone();
            async move {
                let q = registry.get(&name)?.as_queryable()?.clone();
                match q.essential().await {
                    Ok(names) => {
                        debug!(
                            "backend '{}' reports {} essential package(s).",
                            name,
                            names.len()
                        );
                        Some(
                            names
                                .into_iter()
                                .map(|n| format!("{}:{}", name, n))
                                .collect::<Vec<_>>(),
                        )
                    }
                    Err(e) => {
                        // Not fatal: the protected list and the count limit still apply.
                        warn!("backend '{}' essential query failed: {}", name, e);
                        None
                    }
                }
            }
        })
        // `max_parallel`, and a cap that ignores the setting is a cap the user cannot move —
        // `planner.rs:324` states the rule and this was the one fan-out in the tree that did
        // not follow it (AU9). It is on every removal path, which is where a user who has
        // turned the parallelism down most wants it honoured.
        .buffer_unordered(max_parallel.max(1))
        .filter_map(|r| async move { r })
        .flat_map(futures::stream::iter)
        .collect()
        .await
}

/// Inspect a removal set and report what disqualifies it. `removals` are
/// `(backend, name)` pairs. An empty report means the plan may proceed.
pub async fn inspect(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    removals: &[(String, String)],
) -> GuardReport {
    inspect_removals(config, registry, removals, RemovalKind::Package, 0).await
}

/// The one place `--allow-mass-removal` is honoured, and the one thing it clears.
///
/// II.10: the flag answers exactly one refusal — the count. It used to clear every objection,
/// so the flag meaning "yes, 50 packages is what I meant" also deleted python3. A confirmation
/// asks; a refusal says no, and protection is a refusal (V.26): nothing overrides it.
///
/// Both ceilings answer to it, because the question it answers is the same question (`Y20`).
fn allow_the_count(config: &Config, report: &mut GuardReport, scope: GuardScope, noun: &str) {
    if !config.allow_mass_removal {
        return;
    }
    let before = report.objections.len();
    report
        .objections
        .retain(|o| !matches!(o, Objection::TooMany { .. }));
    if before != report.objections.len() {
        warn!(
            "the {} count for '{}' was allowed by --allow-mass-removal.",
            noun,
            scope.as_str()
        );
    }
}

/// What is being taken away. Both kinds answer to `protected_packages`, to OS-essential and
/// to `max_removals`; they differ in one check and in what a refusal tells you to do.
///
/// The distinction exists because [`protection_of`]'s declarability test asks "could a package
/// line ever have held this name?", and for an extra the answer is structurally no — a
/// `link:`/`service:`/`setting:` key is not a package line and never parses as one. Running
/// that test over an extra marks every extra `Undeclarable` and refuses every teardown
/// forever, which is a guard that has stopped being about the user's intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovalKind {
    Package,
    /// A `link:`/`service:`/`setting:`/`shim:`/`schedule:`/`repo:` resource leaving the model,
    /// or a port closed because no `firewall:` line declares it.
    Extra,
}

impl RemovalKind {
    /// The `[guard]` key a refusal about this kind of removal must name, because a refusal that
    /// does not say which setting to change is a refusal a user cannot answer.
    pub fn ceiling_key(self) -> &'static str {
        match self {
            Self::Package => "max_removals",
            Self::Extra => "max_extra_removals",
        }
    }

    fn ceiling(self, config: &Config) -> usize {
        match self {
            Self::Package => config.guard.max_removals,
            Self::Extra => config.guard.max_extra_removals,
        }
    }
}

/// What one command has taken away so far, so a ceiling is a budget for the command rather than
/// for each phase.
///
/// **A sync removes in four places** — the transaction's packages, the extras teardown, the
/// firewall's undeclared ports, and `repo remove` on the imperative path — and each used to
/// check its own list against the ceiling. `inspect_removals` took an `also_removing: usize` for
/// exactly this reason, which made the count something every caller assembled by hand: two
/// passed the right number and **`apply/firewall.rs` passed `0`**, so four packages and four
/// ports under a limit of five were invisible to every guard call in the run.
///
/// One value, owned by the command, incremented where the guard clears a removal. A caller
/// cannot pass the wrong number because a caller no longer passes a number.
///
/// **Two counts, because there are two ceilings** (`Y20`). `max_removals` is about software
/// leaving the machine; `max_extra_removals` is about the resources a declaration put in place.
/// Sharing one budget would make the stricter of the two govern both, so a server whose first
/// firewall declaration closes forty ports could not also remove a package. Both are answered by
/// `--allow-mass-removal`, because "yes, that many, I meant it" is one question.
#[derive(Debug, Default)]
pub struct Reaping {
    packages: std::sync::atomic::AtomicUsize,
    extras: std::sync::atomic::AtomicUsize,
}

impl Reaping {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many of this kind the command has already cleared.
    pub fn so_far(&self, kind: RemovalKind) -> usize {
        self.counter(kind).load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Record a cleared set. Called by the `enforce*` family and by nothing else: a removal
    /// counted without being checked would raise the total for everyone behind it while
    /// answering to nothing itself.
    fn record(&self, kind: RemovalKind, n: usize) {
        self.counter(kind)
            .fetch_add(n, std::sync::atomic::Ordering::Relaxed);
    }

    fn counter(&self, kind: RemovalKind) -> &std::sync::atomic::AtomicUsize {
        match kind {
            RemovalKind::Package => &self.packages,
            RemovalKind::Extra => &self.extras,
        }
    }
}

/// The identities a `protected_packages` rule is matched against for one removal.
///
/// A package contributes its name and nothing else. An extra whose identity is a path also
/// contributes that path's final component, so `protected_packages = ["vimrc"]` protects
/// `link:/home/u/.vimrc` — a user names the thing, not the absolute path LiNix happens to
/// key it by, and a rule that only matched the full path would silently protect nothing.
fn protected_names(kind: RemovalKind, name: &str) -> Vec<&str> {
    let mut names = vec![name];
    if kind == RemovalKind::Extra {
        if let Some(base) = name.rsplit(['/', '\\']).next() {
            if base != name && !base.is_empty() {
                names.push(base);
            }
        }
    }
    names
}

/// Inspect a removal set of one kind, counting `already` items of that kind the command has
/// cleared before it against the same ceiling.
///
/// **This is the pure half and it stays pure**: a preview may ask it without spending anyone's
/// budget. The `enforce*` family is what reads `already` off the command's [`Reaping`] and
/// writes back to it, so nothing outside this module hand-assembles the number.
pub async fn inspect_removals(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    removals: &[(String, String)],
    kind: RemovalKind,
    already: usize,
) -> GuardReport {
    let mut report = GuardReport::default();
    if removals.is_empty() {
        return report;
    }

    let os_essential = match kind {
        RemovalKind::Package => {
            let backends: HashSet<String> = removals.iter().map(|(b, _)| b.clone()).collect();
            essential_names(registry, &backends, config.max_parallel).await
        }
        // `service`/`link`/`setting` are not package managers and have no essential list to
        // ask for; querying them would be a round trip that can only return nothing.
        RemovalKind::Extra => HashSet::new(),
    };

    for (backend, name) in removals {
        let protection = match kind {
            RemovalKind::Package => protection_of(config, Some(backend), name, &os_essential),
            RemovalKind::Extra => protected_names(kind, name).into_iter().find_map(|n| {
                config
                    .protection_rule(n)
                    .map(|r| Protection::Rule(r.to_string()))
            }),
        };
        if let Some(p) = protection {
            report.objections.push(Objection::Protected {
                key: format!("{}:{}", backend, name),
                reason: p.reason(),
            });
        }
    }

    let total = removals.len() + already;
    let limit = kind.ceiling(config);
    if limit > 0 && total > limit {
        report.objections.push(Objection::TooMany {
            count: total,
            limit,
            setting: kind.ceiling_key(),
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
    reaping: &Reaping,
    scope: GuardScope,
) -> Result<Reaped> {
    enforce_kind(
        config,
        registry,
        removals,
        RemovalKind::Package,
        reaping,
        scope,
    )
    .await
}

/// Enforce the guard over the extras a sync is about to undo (`link:`, `service:`, `setting:`,
/// `shim:`, `schedule:`, `repo:`).
///
/// `also_removing` is the number of packages the same command already plans to remove, so the
/// ceiling is checked once against the whole command rather than once per phase.
///
/// This exists because the teardown loop in `app/apply/extras.rs` runs outside the transaction
/// and therefore outside the plan-time `enforce` that covers packages. Ten call sites can reach
/// a backend `remove`; that one was the only one no guard stood in front of, and a `link:` whose
/// target is a decrypted secret is not a smaller loss than a package.
pub async fn enforce_extras(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    removals: &[(String, String)],
    reaping: &Reaping,
    scope: GuardScope,
) -> Result<Reaped> {
    enforce_kind(
        config,
        registry,
        removals,
        RemovalKind::Extra,
        reaping,
        scope,
    )
    .await
}

/// Both halves of the guard, which differ only in which ceiling they answer to and in what the
/// refusal tells you to do.
///
/// **The count comes off the command's [`Reaping`] and goes back onto it here**, which is what
/// makes the ceiling a budget for the command. Two entry points wrap this rather than one taking
/// a `RemovalKind`, because a caller choosing the kind is a caller who can choose it wrong — a
/// package teardown reported as an extra escapes `protection_of`'s declarability test.
async fn enforce_kind(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    removals: &[(String, String)],
    kind: RemovalKind,
    reaping: &Reaping,
    scope: GuardScope,
) -> Result<Reaped> {
    let mut report = inspect_removals(
        config,
        registry,
        removals,
        kind,
        reaping.so_far(kind),
    )
    .await;

    allow_the_count(
        config,
        &mut report,
        scope,
        match kind {
            RemovalKind::Package => "removal",
            RemovalKind::Extra => "teardown",
        },
    );

    if report.is_empty() {
        // Recorded only once the set is cleared: a refused command stops here, and a removal
        // that was never allowed must not raise the total anything behind it is measured
        // against.
        reaping.record(kind, removals.len());
        return Ok(Reaped { scope });
    }
    refuse(report.message(scope, kind))
}

/// Turn a refusal into the error every command reports.
///
/// **Every guard entry point comes through here**, so `Error::Refused` — U21's exit code 3 — is
/// a property of the guard rather than of each caller remembering to pick the right variant.
/// The install ceiling returned `Error::Other` until this existed, which made the one refusal
/// in II.10 that is about installs exit 1 while its eight siblings exited 3.
///
/// It does **not** fire `on_guard_refusal`. Announcing a refusal is a side effect, and a side
/// effect inside a decision function runs wherever the decision is evaluated — including in
/// tests, which call this with a default `Config` whose `config_root()` is the developer's own
/// `~/.config/linix`. That would have `cargo test` executing the developer's real hooks. The
/// event is fired once, where `Error::Refused` becomes an exit code (`finish`), which is the
/// layer where effects belong.
///
/// **Note what this function is and is not.** It is where every *guard* refusal is built. It is
/// not where every refusal in the program is built — the SEC/T series constructs its own, and
/// for nine sites those were `Error::Validation`, so they exited 1 and the hook never heard
/// them. What makes the promise true is the variant, not this function, and what checks it is
/// `tests/grader_refusal_exit_code_tests.rs`.
fn refuse<T>(message: String) -> Result<T> {
    Err(Error::Refused(message))
}

/// Inspect the *desired* state against the `[guard]` install rules (II.10) that do not need
/// runtime state: `deny_packages` and `pinned_only`. The two that do — `require_snapshot`
/// and `deny_vulnerable` — are enforced by the caller, which holds the snapshot provider and
/// the audit report. Returns one objection per offending package; an empty vec means the
/// spec-level rules pass.
pub fn inspect_desired(
    guard: &crate::config::GuardSettings,
    desired: &std::collections::HashMap<String, Vec<crate::core::PackageSpec>>,
) -> Vec<Objection> {
    let mut objections = Vec::new();
    for specs in desired.values() {
        for s in specs {
            let key = format!("{}:{}", s.backend, s.name);
            if guard
                .deny_packages
                .iter()
                .any(|d| d.eq_ignore_ascii_case(&s.name))
            {
                objections.push(Objection::Denied { key: key.clone() });
            }
            if guard.pinned_only {
                let pinned = s
                    .options
                    .one("version")
                    .map(|v| !v.is_empty() && v != "latest" && v != "*")
                    .unwrap_or(false);
                if !pinned {
                    objections.push(Objection::Unpinned { key });
                }
            }
        }
    }
    objections
}

/// A one-line, human-readable reason for an install-side objection, for the caller's
/// violation list. (Removal objections render through [`GuardReport::message`] instead.)
pub fn describe_objection(o: &Objection) -> String {
    match o {
        Objection::Denied { key } => format!("{} — denied by policy (deny_packages)", key),
        Objection::Unpinned { key } => {
            format!("{} — pinned_only requires an explicit @version=", key)
        }
        Objection::Protected { key, reason } => format!("{} — {}", key, reason),
        Objection::TooMany {
            count,
            limit,
            setting,
        } => format!("removes {} items, over {} ({})", count, setting, limit),
        Objection::TooManyInstalls { count, limit } => {
            format!("installs {} packages, over max_installs ({})", count, limit)
        }
    }
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
    if config.guard.max_installs == 0 || count <= config.guard.max_installs {
        return Ok(());
    }
    if config.allow_mass_install {
        warn!(
            "the install count for '{}' ({}) was allowed by --allow-mass-install.",
            scope.as_str(),
            count
        );
        return Ok(());
    }

    refuse(format!(
        "{}: refusing this install.\n  \
         - it installs {} packages, over the limit of {} (config: max_installs)\n\n\
         This usually means a manifest matched more than you meant — run `linix plan` and \
         read the counts before proceeding.\n\n\
         What to do:\n  \
         linix plan                     see exactly what would be installed\n  \
         {} --allow-mass-install carry out this install anyway",
        scope.as_str(),
        count,
        config.guard.max_installs,
        scope.as_str(),
    ))
}

/// Enforce for `purge-undeclared`, where the count is not the question (II.11).
///
/// `max_removals` catches accidents, and this command is the opposite of an accident: you
/// typed its name and confirmed it. **`protected_packages` and OS-essential still apply** —
/// those are not "are you sure", and the ratio check (II.11) is what asks whether you meant
/// it at all.
pub async fn enforce_deliberate(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    removals: &[(String, String)],
    reaping: &Reaping,
    scope: GuardScope,
) -> Result<Reaped> {
    let mut report = inspect(config, registry, removals).await;
    report
        .objections
        .retain(|o| !matches!(o, Objection::TooMany { .. }));
    if report.is_empty() {
        // Still recorded. The count is not the question *for this command*, but a purge is one
        // phase of a run that goes on to tear extras down, and the budget those answer to has
        // to know what has already gone.
        reaping.record(RemovalKind::Package, removals.len());
        return Ok(Reaped { scope });
    }
    refuse(report.message(scope, RemovalKind::Package))
}

/// Split an extras-ledger key (`link:/home/u/.vimrc`, `repo:apt:ppa:x/y`) into the
/// `(kind, id)` pair the guard inspects.
///
/// **A key this build cannot parse is carried through under an empty kind rather than dropped.**
/// The guard must never silently stop covering something it could not read: an unreadable row
/// still names a thing on the machine, and the count and the protection rules both still apply
/// to it. Only the *kind* is unknown, and the guard does not dispatch on kind.
pub fn extra_removal_pairs(keys: &[String]) -> Vec<(String, String)> {
    use crate::core::extras_lock::ExtraKey;
    keys.iter()
        .map(|k| match k.parse::<ExtraKey>() {
            Ok(key) => (key.kind.to_string(), key.subject),
            Err(()) => (String::new(), k.clone()),
        })
        .collect()
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

    #[test]
    fn a_name_no_line_can_hold_is_never_removed() {
        // LiNix must not remove what it could never have been asked to keep: a name that
        // cannot be written down cannot be declared, so it is unmanaged forever and a standing
        // `purge-undeclared` candidate through no fault of its owner.
        //
        // **Y7 moved the boundary, and this test is what says where it is now.** A name with a
        // space in it — `ARP\Machine\X64\Android Studio`, which is what `winget list` answers —
        // is declarable today, because it can be quoted. What is still beyond a line is a name
        // carrying a quote or a control character, and those are what keep this branch alive.
        let cfg = Config::default();
        let empty = HashSet::new();

        for holdable in [
            r"ARP\Machine\X64\Android Studio",
            "7zip.7zip",
            r"MSIX\Microsoft.BingSearch_1.1.43.0_x64__8wekyb3d8bbwe",
        ] {
            assert!(
                protection_of(&cfg, Some("winget"), holdable, &empty).is_none(),
                "`{holdable}` can be declared, so it is protected by the ordinary rules and \
                 not by undeclarability"
            );
        }

        for unholdable in ["Some \"Quoted\" Program", "two\nlines"] {
            assert!(
                matches!(
                    protection_of(&cfg, Some("winget"), unholdable, &empty),
                    Some(Protection::Undeclarable)
                ),
                "`{unholdable}` cannot be written as a line and must never be removed"
            );
        }
    }

    /// `purge-undeclared` sweeps everything LiNix does not manage, and it builds its list from
    /// `list_installed` — which for `service` is every running service. The only thing that
    /// ever stopped it was the declarability test asking a question about *package* lines and
    /// getting the right answer for the wrong reason. Correcting that sentence would have
    /// handed the sweep 155 Windows services; this is the refusal made on purpose instead.
    #[test]
    fn a_resource_is_refused_by_a_rule_and_not_by_an_accident() {
        let cfg = Config {
            // Even the escape hatch, wide open. A `service:` is not a package the user could
            // be declaring they manage themselves; there is nothing here to release.
            guard: crate::config::GuardSettings {
                unprotected_packages: vec!["*".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        let empty = HashSet::new();

        for (backend, name) in [
            ("service", "AppMgmt"),
            ("service", "sshd"),
            ("link", "/home/u/.vimrc"),
            ("setting", "org.gnome.desktop.interface/clock-format"),
        ] {
            let p = protection_of(&cfg, Some(backend), name, &empty);
            assert!(
                matches!(p, Some(Protection::NotAPackage(_))),
                "`{backend}:{name}` must be refused as a resource, not swept: {p:?}"
            );
            let reason = p.unwrap().reason();
            assert!(
                !reason.contains("no line can hold"),
                "`{backend}:{name}` parses — saying no line can hold it is false: {reason}"
            );
        }

        // And the refusal is about resources, not about everything: a package is still judged
        // by the ordinary rules, or the guard has stopped being about the user's intent.
        assert!(protection_of(&cfg, Some("apt"), "jq", &empty).is_none());
    }

    /// The other half of the same rule: `sync` undoing a `service:` line the user deleted is a
    /// teardown (`RemovalKind::Extra`) and must still go through. A guard that refuses both
    /// directions has made the declaration unremovable, which is the bug `plan.rs` already hit
    /// once when it ran the package test over resource keys.
    #[tokio::test]
    async fn undeclaring_a_resource_is_still_allowed() {
        let registry = Arc::new(BackendRegistry::default());
        let report = inspect_removals(
            &Config::default(),
            &registry,
            &[("service".to_string(), "sshd".to_string())],
            RemovalKind::Extra,
            0,
        )
        .await;
        assert!(
            report.is_empty(),
            "deleting the line is how a service is torn down: {:?}",
            report.objections
        );
    }

    #[test]
    fn unprotecting_cannot_release_a_name_that_cannot_be_declared() {
        // `unprotected_packages` says "I manage this one myself". You cannot manage what you
        // cannot write down, so this is the one protection the escape hatch does not open.
        let cfg = Config {
            guard: crate::config::GuardSettings {
                unprotected_packages: vec!["*".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        // Quotable now, so `*` really does release it — the escape hatch works on every name
        // the user could have written.
        assert!(protection_of(&cfg, Some("winget"), "Some Program 1.0", &HashSet::new()).is_none());
        // And still refuses on the one class no `*` can reach.
        assert!(matches!(
            protection_of(
                &cfg,
                Some("winget"),
                "Some \"Program\" 1.0",
                &HashSet::new()
            ),
            Some(Protection::Undeclarable)
        ));
    }

    /// Both ceilings set to the same number, so a test that says "a limit of two" means it for
    /// whichever kind it is about. `Y20` split them; a helper that moved only one would have
    /// left every extras test measuring against the default of twenty and passing for no
    /// reason.
    fn config_with(max: usize) -> Config {
        Config {
            guard: crate::config::GuardSettings {
                protected_packages: vec!["python3".into(), "libpam*".into()],
                unprotected_packages: Vec::new(),
                max_removals: max,
                max_extra_removals: max,
                ..Default::default()
            },
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
                limit: 20,
                setting: "max_removals"
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
        cfg.guard.unprotected_packages = vec!["libpam-modules".into()];
        let none = HashSet::new();
        // libpam* still protects the rest of the family...
        assert!(protection_of(&cfg, Some("apt"), "libpam0g", &none).is_some());
        // ...but the explicit opt-out wins for the one the user named.
        assert!(protection_of(&cfg, Some("apt"), "libpam-modules", &none).is_none());
    }

    #[test]
    fn unprotect_wins_over_the_os_essential_flag() {
        // The documented promise: un-protect beats *everything*, OS flags included.
        // Previously the OS check ran in an `else if` and fired anyway.
        let mut cfg = config_with(20);
        cfg.guard.unprotected_packages = vec!["dash".into()];
        let os: HashSet<String> = ["apt:dash".to_string()].into_iter().collect();
        assert!(protection_of(&cfg, Some("apt"), "dash", &os).is_none());
        // An essential package the user did NOT exempt is still protected.
        let os2: HashSet<String> = ["apt:base-files".to_string()].into_iter().collect();
        assert!(protection_of(&cfg, Some("apt"), "base-files", &os2).is_some());
    }

    #[tokio::test]
    async fn yes_does_not_override_the_guard() {
        // The whole point: -y is what every script passes. It must not mean "purge".
        let reg = Arc::new(BackendRegistry::new());
        let mut cfg = config_with(20);
        cfg.yes = true;
        assert!(enforce(&cfg, &reg, &pairs(&["python3"]), &Reaping::new(), GuardScope::Apply)
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
            enforce(&cfg, &reg, &pairs(&["jq", "htop", "bat"]), &Reaping::new(), GuardScope::Sync)
                .await
                .is_ok(),
            "the flag must let a big-but-ordinary removal through"
        );

        // A protected package, even when the flag is set and the count is fine.
        assert!(
            enforce(&cfg, &reg, &pairs(&["python3"]), &Reaping::new(), GuardScope::Sync)
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
                &Reaping::new(),
                GuardScope::Sync
            )
            .await
            .is_err(),
            "the flag must not carry a protected package in on the back of the count"
        );
    }

    /// Every scope names itself, in both vocabularies, and no two share an answer.
    ///
    /// **This is the assertion that would have caught the dead round trip.** The firewall
    /// teardown converted its scope to a string and back through two functions whose
    /// vocabularies did not overlap: the producer emitted `"an unattended watch tick"` and the
    /// consumer matched `"watch"`, so both named arms were unreachable and every teardown —
    /// including `N7`'s unattended tick, which reverts by default with nobody watching — was
    /// guarded and reported as `sync`. The scope is passed as the enum now, and this holds the
    /// two labels apart so a catch-all arm cannot quietly reintroduce the collapse.
    #[test]
    fn every_scope_names_itself_and_no_two_answer_alike() {
        let all = [
            GuardScope::Apply,
            GuardScope::Sync,
            GuardScope::RemoveOrphans,
            GuardScope::PurgeUndeclared,
            GuardScope::Watch,
            GuardScope::Upgrade,
            GuardScope::Canary,
            GuardScope::Remove,
            GuardScope::ShellExit,
            GuardScope::ExpirySweep,
            GuardScope::Heal,
            GuardScope::Rebuild,
        ];

        let mut commands = std::collections::BTreeSet::new();
        let mut prose = std::collections::BTreeSet::new();
        for scope in all {
            assert!(
                commands.insert(scope.as_str()),
                "{:?} shares `as_str` with another scope, so a refusal tells the user to retype \
                 a different command",
                scope
            );
            assert!(
                prose.insert(scope.during()),
                "{:?} shares `during` with another scope — a catch-all arm has collapsed them, \
                 which is how the label this replaced answered `sync` for nine of twelve",
                scope
            );
        }
        assert_eq!(commands.len(), all.len());
        assert_eq!(prose.len(), all.len());

        // The one distinction the whole mechanism exists for.
        assert_eq!(GuardScope::Watch.during(), "an unattended watch tick");
        assert_eq!(GuardScope::Sync.during(), "sync");
    }

    #[tokio::test]
    async fn no_setting_can_opt_a_command_out_of_the_guard() {
        // `[guard.enforce_on]` used to do exactly this: a config key that switched the
        // guard off per command, so `enforce_on.sync = false` — copied from a dotfiles repo
        // — made a routine sync remove python3. II.10 lists ten refusals and that was not
        // one of them; V.21 says no setting anyone can flip makes sync dangerous.
        let reg = Arc::new(BackendRegistry::new());
        let cfg = config_with(20);
        for scope in [
            GuardScope::Apply,
            GuardScope::Sync,
            GuardScope::RemoveOrphans,
            GuardScope::PurgeUndeclared,
            GuardScope::Watch,
            GuardScope::Upgrade,
            GuardScope::Canary,
            GuardScope::Remove,
            GuardScope::ShellExit,
            GuardScope::ExpirySweep,
            GuardScope::Heal,
            GuardScope::Rebuild,
        ] {
            assert!(
                enforce(&cfg, &reg, &pairs(&["python3"]), &Reaping::new(), scope)
                    .await
                    .is_err(),
                "{:?} must be guarded, and nothing may turn that off",
                scope
            );
        }
    }

    #[tokio::test]
    async fn a_deliberate_purge_ignores_the_count_but_never_protection() {
        // II.11: `max_removals` catches accidents, and `purge-undeclared` is the opposite of
        // an accident — you typed its name. `protected_packages` and OS-essential still
        // apply, and the ratio check is what asks whether you meant it at all.
        let reg = Arc::new(BackendRegistry::new());
        let cfg = config_with(2);

        assert!(
            enforce_deliberate(
                &cfg,
                &reg,
                &pairs(&["a", "b", "c", "d"]),
                &Reaping::new(),
                GuardScope::PurgeUndeclared
            )
            .await
            .is_ok(),
            "the count is not the question here"
        );
        assert!(
            enforce_deliberate(
                &cfg,
                &reg,
                &pairs(&["python3"]),
                &Reaping::new(),
                GuardScope::PurgeUndeclared
            )
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
        assert!(enforce_installs(&cfg, 10_000, GuardScope::Sync)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn install_over_the_ceiling_is_refused() {
        let mut cfg = config_with(20);
        cfg.guard.max_installs = 50;
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
        cfg.guard.max_installs = 50;
        assert!(enforce_installs(&cfg, 50, GuardScope::Sync).await.is_ok());
    }

    #[tokio::test]
    async fn allow_mass_install_clears_the_install_ceiling() {
        // Symmetric to --allow-mass-removal answering the removal count.
        let mut cfg = config_with(20);
        cfg.guard.max_installs = 50;
        cfg.allow_mass_install = true;
        assert!(enforce_installs(&cfg, 5_000, GuardScope::Sync)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn yes_does_not_override_the_install_ceiling() {
        // -y is what every script passes; it must not green-light a manifest-globbed flood.
        let mut cfg = config_with(20);
        cfg.guard.max_installs = 50;
        cfg.yes = true;
        assert!(enforce_installs(&cfg, 5_000, GuardScope::Sync)
            .await
            .is_err());
    }

    fn desired(
        specs: &[(&str, &str, Option<&str>)],
    ) -> std::collections::HashMap<String, Vec<crate::core::PackageSpec>> {
        let mut m: std::collections::HashMap<String, Vec<crate::core::PackageSpec>> =
            std::collections::HashMap::new();
        for (backend, name, version) in specs {
            let mut options = crate::config::grammar::Options::default();
            if let Some(v) = version {
                options.set("version", v.to_string());
            }
            m.entry(backend.to_string())
                .or_default()
                .push(crate::core::PackageSpec {
                    name: name.to_string(),
                    backend: backend.to_string(),
                    options,
                    requires: vec![],
                    present: true,
                });
        }
        m
    }

    #[test]
    fn deny_packages_refuses_an_install_case_insensitively() {
        let guard = crate::config::GuardSettings {
            deny_packages: vec!["LeftPad".into()],
            ..Default::default()
        };
        let os = inspect_desired(&guard, &desired(&[("npm", "leftpad", None)]));
        assert!(
            matches!(os.as_slice(), [Objection::Denied { .. }]),
            "{:?}",
            os
        );
    }

    #[test]
    fn pinned_only_requires_a_concrete_version() {
        let guard = crate::config::GuardSettings {
            pinned_only: true,
            ..Default::default()
        };
        let os = inspect_desired(
            &guard,
            &desired(&[
                ("apt", "curl", None),           // no version -> refused
                ("apt", "wget", Some("latest")), // floating -> refused
                ("apt", "jq", Some("1.6")),      // pinned -> ok
            ]),
        );
        assert_eq!(os.len(), 2, "{:?}", os);
        assert!(os.iter().all(|o| matches!(o, Objection::Unpinned { .. })));
    }

    #[test]
    fn an_empty_guard_table_objects_to_nothing() {
        let guard = crate::config::GuardSettings::default();
        assert!(guard.is_empty());
        assert!(inspect_desired(&guard, &desired(&[("apt", "curl", None)])).is_empty());
    }

    /// Every kind the extras teardown can undo, keyed the way the ledger keys it.
    fn extras(keys: &[&str]) -> Vec<(String, String)> {
        extra_removal_pairs(&keys.iter().map(|k| k.to_string()).collect::<Vec<_>>())
    }

    #[tokio::test]
    async fn no_extra_is_refused_merely_for_not_being_a_package_line() {
        // The trap this kind exists to avoid: `protection_of`'s declarability test asks whether
        // a package line could hold the name, and no extras key can — `link:/home/u/.vimrc` is
        // not a package line and never parses as one. Running that test over extras marks all
        // six kinds `Undeclarable` and refuses every teardown on every machine forever, which
        // is a guard that has stopped being about what the user asked for.
        let reg = Arc::new(BackendRegistry::new());
        let cfg = config_with(20);
        let all_six = extras(&[
            "link:/home/u/.vimrc",
            "service:nginx",
            "setting:org.gnome.desktop.interface color-scheme",
            "shim:rg",
            "schedule:nightly-sync",
            "repo:apt:ppa:x/y",
        ]);
        let report = inspect_removals(&cfg, &reg, &all_six, RemovalKind::Extra, 0).await;
        assert!(
            report.is_empty(),
            "an ordinary teardown of one of each kind must be allowed: {:?}",
            report.objections
        );
    }

    #[tokio::test]
    async fn a_protected_name_stops_a_teardown_of_every_kind() {
        // V.26: protection is a refusal nothing overrides, and the ruling of 2026-07-28 is that
        // it covers resources as well as packages. One case per kind, because a guard that
        // holds for `link:` and not for `service:` is the shape this whole finding is about.
        let reg = Arc::new(BackendRegistry::new());
        let mut cfg = config_with(0); // count disabled, so only protection can object
        cfg.guard.protected_packages = vec!["keep".into()];

        for key in [
            "link:/home/u/keep",
            r"link:C:\Users\u\keep",
            "service:keep",
            "shim:keep",
            "schedule:keep",
            "setting:keep",
        ] {
            let report = inspect_removals(&cfg, &reg, &extras(&[key]), RemovalKind::Extra, 0).await;
            assert!(
                matches!(report.objections.as_slice(), [Objection::Protected { .. }]),
                "`{}` was not protected by `protected_packages = [\"keep\"]`: {:?}",
                key,
                report.objections
            );
        }

        // And the control: a name the rule does not match is still removable, or the assertion
        // above would pass for a guard that refuses everything.
        let report = inspect_removals(
            &cfg,
            &reg,
            &extras(&["link:/home/u/other"]),
            RemovalKind::Extra,
            0,
        )
        .await;
        assert!(report.is_empty(), "{:?}", report.objections);
    }

    /// **A ceiling is a budget for the command.** A sync tears extras down in two places — the
    /// firewall's undeclared ports and the ledger's drift — and each used to check only its own
    /// list, so a limit of five could be passed twice by a run that exceeded it once.
    ///
    /// The number is no longer a parameter. `Reaping` carries it, and this test drives the real
    /// one through two `enforce_extras` calls the way a sync does.
    #[tokio::test]
    async fn two_teardown_phases_of_one_command_share_one_budget() {
        let reg = Arc::new(BackendRegistry::new());
        let cfg = config_with(5);
        let reaping = Reaping::new();

        assert!(
            enforce_extras(
                &cfg,
                &reg,
                &extras(&["link:/a", "link:/b", "link:/c"]),
                &reaping,
                GuardScope::Sync
            )
            .await
            .is_ok(),
            "three teardowns under a limit of five must pass on their own"
        );
        assert_eq!(reaping.so_far(RemovalKind::Extra), 3);

        let err = enforce_extras(
            &cfg,
            &reg,
            &extras(&["link:/d", "link:/e", "link:/f"]),
            &reaping,
            GuardScope::Sync,
        )
        .await
        .expect_err("three more, after three, is six over a limit of five");
        assert!(err.to_string().contains("6 managed resources"), "{err}");
        assert!(
            err.to_string().contains("max_extra_removals"),
            "the refusal has to name the setting the reader must change: {err}"
        );
        assert_eq!(
            reaping.so_far(RemovalKind::Extra),
            3,
            "a refused set must not raise the total anything behind it is measured against"
        );
    }

    /// **`Y20`: two ceilings, and neither spends the other's budget.** Software leaving the
    /// machine and a perimeter tightening are different events. Sharing one number would make
    /// the stricter govern both, so a server whose first `firewall:` declaration closes forty
    /// ports could not also remove a package.
    #[tokio::test]
    async fn packages_and_teardowns_answer_to_their_own_ceilings() {
        let reg = Arc::new(BackendRegistry::new());
        let mut cfg = config_with(3);
        cfg.guard.max_extra_removals = 3;
        let reaping = Reaping::new();

        assert!(
            enforce(&cfg, &reg, &pairs(&["a", "b", "c"]), &reaping, GuardScope::Sync)
                .await
                .is_ok()
        );
        assert!(
            enforce_extras(
                &cfg,
                &reg,
                &extras(&["link:/a", "link:/b", "link:/c"]),
                &reaping,
                GuardScope::Sync
            )
            .await
            .is_ok(),
            "three packages must not have spent the teardown budget"
        );
        assert_eq!(reaping.so_far(RemovalKind::Package), 3);
        assert_eq!(reaping.so_far(RemovalKind::Extra), 3);

        // And the package ceiling is still the package ceiling.
        let err = enforce(&cfg, &reg, &pairs(&["d"]), &reaping, GuardScope::Sync)
            .await
            .expect_err("a fourth package is over a limit of three");
        assert!(err.to_string().contains("max_removals"), "{err}");
        assert!(
            !err.to_string().contains("max_extra_removals"),
            "a package refusal must not send the reader to the teardown setting: {err}"
        );
    }

    #[tokio::test]
    async fn allow_mass_removal_answers_a_teardown_count_but_never_its_protection() {
        // The extras half of `allow_mass_removal_answers_the_count_and_nothing_else`, and for
        // the same reason: the flag means "yes, that many is what I meant", never "yes, delete
        // the one I told you to keep".
        let reg = Arc::new(BackendRegistry::new());
        let mut cfg = config_with(1);
        cfg.guard.protected_packages = vec!["keep".into()];
        cfg.allow_mass_removal = true;

        assert!(
            enforce_extras(
                &cfg,
                &reg,
                &extras(&["link:/a", "link:/b"]),
                &Reaping::new(),
                GuardScope::Sync
            )
            .await
            .is_ok(),
            "the flag must let a big-but-ordinary teardown through"
        );
        assert!(
            enforce_extras(&cfg, &reg, &extras(&["link:/keep"]), &Reaping::new(), GuardScope::Sync)
                .await
                .is_err(),
            "nothing overrides protection — not even --allow-mass-removal"
        );
    }

    #[tokio::test]
    async fn a_teardown_refusal_does_not_advise_a_command_that_cannot_take_it() {
        // `linix unmanage` takes a package line. Offering it for a `link:` teardown names a
        // command that cannot accept the thing the refusal is about.
        let reg = Arc::new(BackendRegistry::new());
        let cfg = config_with(1);
        let err = enforce_extras(
            &cfg,
            &reg,
            &extras(&["link:/a", "link:/b"]),
            &Reaping::new(),
            GuardScope::Sync,
        )
        .await
        .expect_err("two removals over a limit of one must be refused");
        let msg = err.to_string();
        assert!(msg.contains("managed resources"), "{}", msg);
        assert!(!msg.contains("linix unmanage"), "{}", msg);
        assert!(msg.contains("linix plan"), "{}", msg);
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
                setting: "max_removals",
            }))
            .collect();
        let msg =
            GuardReport { objections }.message(GuardScope::PurgeUndeclared, RemovalKind::Package);
        let count_line = msg.find("removes 25 packages").expect("count line present");
        let first_pkg = msg.find("apt:pkg0").expect("a package listed");
        assert!(count_line < first_pkg, "the count must lead");
        assert!(msg.contains("…and 15 more"), "the list must be capped");
    }
}
