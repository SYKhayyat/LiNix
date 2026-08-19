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
/// **`README.md:358` promises that every path removing anything goes through one guard. Until
/// this type existed, that sentence was checked by a regex over source text** —
/// `removal_guard_enumeration_tests.rs`'s `is_removal_call`, matching `.remove(` with `sudo`
/// on the line, `.remove_repo(`, `.remove_shim(` and `.deprovision(`. `apply/firewall.rs` closes
/// a port with `deny_command`, which matches none of them, and the word `guard` appears nowhere
/// in that file. **The fix for `G-1` replaced a stale list of paths with a stale list of verbs**,
/// and the staleness moved into a predicate with a passing self-test, where nobody re-derives it.
///
/// So the enumeration is the compiler's now. Every effector that removes takes one of these, and
/// the only way to get one is to have asked. Effector six is covered by construction rather than
/// by someone remembering the list.
///
/// This is what `PlanScope` did for planning, applied to removal. `planner.rs` states the
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
    /// - **A rollback compensating its own transaction.** `transaction.rs` already calls
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
    /// - `Remove` is a person typing `shall uninstall`. The removal was ordered by hand rather
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
    /// The manager reports a name that cannot be written as a package line, so Shall can
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
                "its manager reports a name no line can hold, so Shall cannot manage \
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
/// `backend` is `None` when the caller does not know one — `shall protected jq`, where the user
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
    // cannot be declared, so Shall never manages it and `unprotected_packages` has nothing
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

    // One question, one pass. An explicit un-protect entry wins over everything, including
    // the OS's own essential flag — the user saying "I know, I manage this one myself", and
    // nothing should overrule that, or the escape hatch does not open for exactly the packages
    // someone would need it for. Asking it as two calls meant `protection_rule` re-scanned
    // `unprotected_packages` after `unprotect_rule` had already answered `None` over the same
    // list with the same input.
    match config.protection_of(name) {
        crate::config::ProtectionAnswer::Unprotected(_) => return None,
        crate::config::ProtectionAnswer::Protected(rule) => {
            return Some(Protection::Rule(rule.to_string()))
        }
        crate::config::ProtectionAnswer::Neither => {}
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
    /// The objections a mass flag answered, kept rather than dropped on the floor.
    ///
    /// **A report that only subtracts cannot say what it let through.** Clearing an objection
    /// and never having raised one produce the same `objections` list, so the only difference
    /// between "the flag was needed" and "the flag was idle" used to be a `warn!` line — and a
    /// line is something a caller has to overhear rather than something it can read. Recorded
    /// here so [`allow_the_count`] announces a value it holds.
    pub allowed_by_flag: Vec<Objection>,
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
        let mut out = format!("{}: refusing this removal.\n", scope.as_str());

        // Every count that objected, not the first: a set can be over its own ceiling and over
        // the total at once, and naming one of them sends a user to raise a number that leaves
        // them refused by the other.
        for o in &self.objections {
            if let Objection::TooMany {
                count,
                limit,
                setting,
            } = o
            {
                let (verb, noun) = counted_as(setting);
                out.push_str(&format!(
                    "  - it {} {} {}, over the limit of {} ([guard] {})\n",
                    verb, count, noun, limit, setting
                ));
            }
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
                    RemovalKind::Port => "port",
                }
            ));
        }

        // The advice has to be executable. `shall unmanage` takes a package line, so offering
        // it for a `link:` teardown names a command that cannot accept the thing it is about;
        // for an extra the equivalent act is putting the declaration back.
        match kind {
            RemovalKind::Package => out.push_str(
                "\nThis usually means managed state has drifted from your manifests — run \
                 `shall plan` and read it before proceeding.\n\n\
                 What to do:\n  \
                 shall protected <pkg>          why a package is guarded\n  \
                 shall unmanage <pkg>           stop managing it WITHOUT uninstalling it\n  \
                 <command> --allow-mass-removal carry out this removal anyway\n  \
                 [guard] unprotected_packages    exempt a package permanently (preferences.toml)",
            ),
            RemovalKind::Extra => out.push_str(
                "\nThese are resources a declaration put in place — a `link:`, `service:`, \
                 `setting:`, `shim:`, `schedule:` or `repo:` line that is no longer in any \
                 module. `sync` undoes what is no longer declared.\n\n\
                 What to do:\n  \
                 shall plan                     see exactly what would be undone\n  \
                 put the line back              if the deletion was not what you meant\n  \
                 <command> --allow-mass-removal carry out this teardown anyway\n  \
                 [guard] unprotected_packages    exempt one permanently (preferences.toml)",
            ),
            // Never `put the line back`: nobody declared these — that is *why* they are closing.
            // The act that keeps a port open is declaring it, which is the opposite instruction.
            RemovalKind::Port => out.push_str(
                "\nThese ports are open on the machine and no `firewall:` line declares them, \
                 so `sync` closes them (`N1`). A machine you reach over the network is a \
                 machine this can cut you off from — read the list before you clear it.\n\n\
                 What to do:\n  \
                 shall plan                     see exactly what would be closed\n  \
                 firewall:<port>/<proto>        declare a port you meant to keep open\n  \
                 <command> --allow-mass-removal close this many anyway\n  \
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
                                .iter()
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
        // `planner.rs` states the rule and this was the one fan-out in the tree that did
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
    // A fresh ledger: this is the preview, and a preview that spent a budget would report a
    // different answer the second time it was asked.
    inspect_removals(
        config,
        registry,
        removals,
        RemovalKind::Package,
        &Reaping::new(),
    )
    .await
}

/// The one place the mass flags are honoured over a removal report, and the one thing they
/// clear.
///
/// II.10: a mass flag answers exactly one refusal — the count. It used to clear every objection,
/// so the flag meaning "yes, 50 packages is what I meant" also deleted python3. A confirmation
/// asks; a refusal says no, and protection is a refusal (V.26): nothing overrides it.
///
/// Every removal ceiling answers to `--allow-mass-removal`, because the question it answers is
/// one question (`Y20`). `max_total_changes` answers to that flag *and* to
/// `--allow-mass-install`, because a total is made of both (`N8`) — and to nothing else, which
/// is why this is a match on the setting rather than a blanket retain.
///
/// **What the flag answered is moved, not deleted.** This was a `retain` and a
/// `before != after` around the `warn!`, whose only observable was the line itself: flipped to
/// `==`, the announcement went to every run that did *not* need an override and was withheld
/// from the runs that did, while the report stayed byte for byte the same. A test that read the
/// `tracing` output was written for it and withdrawn the same day — callsite `Interest` is
/// cached globally and other tests create and drop dispatchers on other threads throughout the
/// run, so the capture came back empty in 2 of 3 identical Linux runs. Partitioning answers it
/// without a subscriber: the objections the flag cleared are on the report, so what was allowed
/// is a value a caller reads rather than a line it has to overhear.
fn allow_the_count(config: &Config, report: &mut GuardReport, scope: GuardScope) {
    if !config.allow_mass_removal && !config.allow_mass_install {
        return;
    }
    let (allowed, kept): (Vec<Objection>, Vec<Objection>) = std::mem::take(&mut report.objections)
        .into_iter()
        .partition(|o| match o {
            // `--allow-mass-install` clears the total, because a total is made of installs too,
            // and it clears nothing else: the flag that means "yes, install that many" must not
            // also answer "yes, remove that many". That conflation is II.10's whole point, one
            // ceiling up.
            Objection::TooMany { setting, .. } => {
                config.allow_mass_removal || *setting == TOTAL_KEY
            }
            _ => false,
        });
    report.objections = kept;
    if let Some(said) = announcement(&allowed, config, scope) {
        warn!("{}", said);
    }
    // Extended rather than assigned: nothing calls this twice on one report today, and a report
    // that forgot the first answer when asked a second time would be a silent one.
    report.allowed_by_flag.extend(allowed);
}

/// What a run says when a mass flag answered its count — `None` when none did.
///
/// A function returning the sentence rather than an `if` around the `warn!`, for the same reason
/// the objections are moved rather than dropped: a condition whose only consequence is whether a
/// log line exists can be reversed without failing anything, and the test that would catch it has
/// to subscribe to `tracing` from a binary where the callsite cache is shared with every other
/// test on every other thread.
///
/// **Both halves of the sentence are read off what happened, never off the caller.** The ceiling
/// comes from each objection's own `setting` — [`counted_as`], for the reason its own doc gives —
/// and the flags from the config. Written from the caller's noun and a hardcoded
/// `--allow-mass-removal`, it told a run that passed only `--allow-mass-install` that a *removal*
/// count had been allowed by a *removal* flag: a ceiling, a noun and a flag, none of which were
/// that run's. `shall protected` has always printed the true rule — *"either flag answers
/// `max_total_changes`"* — so the guard's own line was the one surface contradicting it (`J9`).
fn announcement(allowed: &[Objection], config: &Config, scope: GuardScope) -> Option<String> {
    let flags = flags_that_allowed(config)?;
    let counts: Vec<String> = allowed
        .iter()
        .filter_map(|o| match o {
            Objection::TooMany {
                count,
                limit,
                setting,
            } => {
                let (verb, noun) = counted_as(setting);
                Some(format!(
                    "{} {} {}, over the limit of {} ([guard] {})",
                    verb, count, noun, limit, setting
                ))
            }
            _ => None,
        })
        .collect();
    if counts.is_empty() {
        return None;
    }
    Some(format!(
        "'{}' {} — allowed by {}.",
        scope.as_str(),
        counts.join("; "),
        flags
    ))
}

/// The mass flags this run actually passed, as a user would type them — `None` when it passed
/// neither, because then nothing was allowed and the sentence has no subject.
///
/// Both are named when both were passed. Attributing one of them instead would mean deciding
/// which was load-bearing, and for `max_total_changes` — the one ceiling either flag answers
/// (`N8`) — either one of them was.
fn flags_that_allowed(config: &Config) -> Option<&'static str> {
    match (config.allow_mass_removal, config.allow_mass_install) {
        (true, true) => Some("--allow-mass-removal and --allow-mass-install"),
        (true, false) => Some("--allow-mass-removal"),
        (false, true) => Some("--allow-mass-install"),
        (false, false) => None,
    }
}

/// What is being taken away. Every kind answers to `protected_packages` and to a ceiling; they
/// differ in one check, in which ceiling, and in what a refusal tells you to do.
///
/// The package/extra distinction exists because [`protection_of`]'s declarability test asks
/// "could a package line ever have held this name?", and for an extra the answer is structurally
/// no — a `link:`/`service:`/`setting:` key is not a package line and never parses as one.
/// Running that test over an extra marks every extra `Undeclarable` and refuses every teardown
/// forever, which is a guard that has stopped being about the user's intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovalKind {
    Package,
    /// A `link:`/`service:`/`setting:`/`shim:`/`schedule:`/`repo:` resource leaving the model.
    Extra,
    /// A port closed because no `firewall:` line declares it (`N8`).
    Port,
}

impl RemovalKind {
    /// The `[guard]` key a refusal about this kind of removal must name, because a refusal that
    /// does not say which setting to change is a refusal a user cannot answer.
    pub fn ceiling_key(self) -> &'static str {
        match self {
            Self::Package => "max_removals",
            Self::Extra => "max_extra_removals",
            Self::Port => "max_port_closures",
        }
    }

    fn ceiling(self, config: &Config) -> usize {
        match self {
            Self::Package => config.guard.max_removals,
            Self::Extra => config.guard.max_extra_removals,
            Self::Port => config.guard.max_port_closures,
        }
    }
}

/// The `[guard]` key for the ceiling over everything one command changes (`N8`).
const TOTAL_KEY: &str = "max_total_changes";

/// What a ceiling counted, as the verb and noun a refusal says it with. Derived from the setting
/// rather than from the caller, so the sentence and the key it names cannot describe different
/// things — a message reading `removes 40 packages` above `[guard] max_port_closures` is worse
/// than no message.
fn counted_as(setting: &str) -> (&'static str, &'static str) {
    match setting {
        "max_removals" => ("removes", "packages"),
        "max_extra_removals" => ("removes", "managed resources"),
        "max_port_closures" => ("closes", "ports"),
        _ => ("makes", "changes in total"),
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
/// **A count per kind, because there is a ceiling per kind** (`Y20`, `N8`). `max_removals` is
/// about software leaving the machine; `max_extra_removals` is about the resources a declaration
/// put in place; `max_port_closures` is about reachability. Sharing one budget makes the
/// strictest govern all of them, so a server whose first firewall declaration closes forty ports
/// could not also remove a package. All are answered by `--allow-mass-removal`, because "yes,
/// that many, I meant it" is one question.
///
/// **And one count of everything, because a command can pass every per-kind ceiling and still
/// do more than anyone meant** (`N8`). `additions` is what the per-kind ceilings do not cover —
/// installs and upgrades, resources created, ports opened — held here rather than anywhere else
/// because `max_total_changes` is a budget for the command, and the command is what owns this.
#[derive(Debug, Default)]
pub struct Reaping {
    packages: std::sync::atomic::AtomicUsize,
    extras: std::sync::atomic::AtomicUsize,
    ports: std::sync::atomic::AtomicUsize,
    additions: std::sync::atomic::AtomicUsize,
}

impl Reaping {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many of this kind the command has already cleared.
    pub fn so_far(&self, kind: RemovalKind) -> usize {
        self.counter(kind)
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Everything the command has changed so far, of every kind — what `max_total_changes` is
    /// measured against.
    pub fn changes_so_far(&self) -> usize {
        use std::sync::atomic::Ordering::Relaxed;
        self.packages.load(Relaxed)
            + self.extras.load(Relaxed)
            + self.ports.load(Relaxed)
            + self.additions.load(Relaxed)
    }

    /// Record a cleared set. Called by the `enforce*` family and by nothing else: a removal
    /// counted without being checked would raise the total for everyone behind it while
    /// answering to nothing itself.
    fn record(&self, kind: RemovalKind, n: usize) {
        self.counter(kind)
            .fetch_add(n, std::sync::atomic::Ordering::Relaxed);
    }

    /// Record changes that are not removals — installs and upgrades, resources created, ports
    /// opened. They answer to no ceiling of their own beyond `max_installs`, but they are
    /// changes, and `max_total_changes` counts changes.
    fn record_addition(&self, n: usize) {
        self.additions
            .fetch_add(n, std::sync::atomic::Ordering::Relaxed);
    }

    fn counter(&self, kind: RemovalKind) -> &std::sync::atomic::AtomicUsize {
        match kind {
            RemovalKind::Package => &self.packages,
            RemovalKind::Extra => &self.extras,
            RemovalKind::Port => &self.ports,
        }
    }
}

/// The `max_total_changes` objection, or `None` when the command still has room.
///
/// One function, called by every gate, because a total assembled at some gates and not others
/// is a total that reports whichever subset happened to run — the `also_removing: usize` shape
/// that `S55` was, one level up.
fn too_many_changes(config: &Config, reaping: &Reaping, adding: usize) -> Option<Objection> {
    let limit = config.guard.max_total_changes;
    let total = reaping.changes_so_far() + adding;
    (limit > 0 && total > limit).then_some(Objection::TooMany {
        count: total,
        limit,
        setting: TOTAL_KEY,
    })
}

/// The identities a `protected_packages` rule is matched against for one removal.
///
/// A package contributes its name and nothing else. An extra whose identity is a path also
/// contributes that path's final component, so `protected_packages = ["vimrc"]` protects
/// `link:/home/u/.vimrc` — a user names the thing, not the absolute path Shall happens to
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

/// Inspect a removal set of one kind against its own ceiling and against the command's total,
/// reading what the command has already cleared off `reaping`.
///
/// **This is the pure half and it stays pure**: it reads the ledger and never writes to it, so a
/// preview may ask without spending anyone's budget. The `enforce*` family is what records.
/// Taking the ledger rather than a number is the point — `already: usize` was a parameter three
/// callers assembled by hand and one answered with a `0` (`S55`).
pub async fn inspect_removals(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    removals: &[(String, String)],
    kind: RemovalKind,
    reaping: &Reaping,
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
        // ask for; querying them would be a round trip that can only return nothing. Neither is
        // a firewall.
        RemovalKind::Extra | RemovalKind::Port => HashSet::new(),
    };

    for (backend, name) in removals {
        let protection = match kind {
            RemovalKind::Package => protection_of(config, Some(backend), name, &os_essential),
            RemovalKind::Extra | RemovalKind::Port => {
                protected_names(kind, name).into_iter().find_map(|n| {
                    config
                        .protection_rule(n)
                        .map(|r| Protection::Rule(r.to_string()))
                })
            }
        };
        if let Some(p) = protection {
            report.objections.push(Objection::Protected {
                key: format!("{}:{}", backend, name),
                reason: p.reason(),
            });
        }
    }

    let total = removals.len() + reaping.so_far(kind);
    let limit = kind.ceiling(config);
    if limit > 0 && total > limit {
        report.objections.push(Objection::TooMany {
            count: total,
            limit,
            setting: kind.ceiling_key(),
        });
    }
    // Both ceilings, when both are busted: a refusal naming one number a user raises, only to
    // meet the other on the next run, is a refusal that lied about what it wanted.
    report
        .objections
        .extend(too_many_changes(config, reaping, removals.len()));

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

/// What `apply` would refuse about this plan, as messages, changing nothing.
///
/// **Here rather than in `verbs/plan.rs`, because the preview's only value is being the same
/// question.** The call site there ran one `inspect` over both removal lists, which put the
/// package rules over resource keys and predicted a refusal `apply` did not perform; then it
/// asked kind by kind with a hand-written `also_removing` for each. Both were the preview
/// drifting from the enforcer while looking like it agreed. One ledger, the kinds in the order
/// the engine gates them, and the numbers come off the ledger.
///
/// **Firewall ports are absent on purpose**: they are computed against the live machine at
/// apply time, so a plan file cannot hold them and this cannot count them. A perimeter change
/// can therefore push a run over `max_total_changes` that previewed clean — which is the same
/// thing `plan` has always said about a machine that moves under it.
pub async fn preview_refusals(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    installs: usize,
    package_removals: &[(String, String)],
    extra_removals: &[(String, String)],
    scope: GuardScope,
) -> Vec<String> {
    let reaping = Reaping::new();
    let mut refusals = Vec::new();
    // The engine's order: packages, then the install ceiling, then the extras teardown.
    for (pairs, kind) in [
        (package_removals, RemovalKind::Package),
        (extra_removals, RemovalKind::Extra),
    ] {
        let mut report = inspect_removals(config, registry, pairs, kind, &reaping).await;
        allow_the_count(config, &mut report, scope);
        if !report.is_empty() {
            refusals.push(report.message(scope, kind));
        }
        // Recorded whether or not it objected. A refused phase stops the engine, so the later
        // ones would never run — but a preview that reported the first refusal and went quiet
        // about the rest would have to be run once per fix.
        reaping.record(kind, pairs.len());
    }
    if let Err(e) = enforce_installs(config, installs, &reaping, scope).await {
        refusals.push(e.to_string());
    }
    refusals
}

/// Enforce the guard over the ports a sync is about to close because no `firewall:` line
/// declares them (`N8`).
///
/// Its own entry point rather than a `RemovalKind` argument at the call site, for the reason the
/// other two are: a caller that picks the kind is a caller that can pick it wrong, and a port
/// reported as an extra spends the resource budget and names the wrong `[guard]` key in the
/// refusal.
pub async fn enforce_ports(
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
        RemovalKind::Port,
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
    let mut report = inspect_removals(config, registry, removals, kind, reaping).await;

    allow_the_count(config, &mut report, scope);

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
/// `~/.config/shall`. That would have `cargo test` executing the developer's real hooks. The
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
pub async fn enforce_installs(
    config: &Config,
    count: usize,
    reaping: &Reaping,
    scope: GuardScope,
) -> Result<()> {
    // The total counts installs, so this gate answers `max_total_changes` before its own
    // ceiling: a run that is inside `max_installs` and past the total is still past the total.
    enforce_total(config, count, reaping, scope, "install")?;
    if config.guard.max_installs == 0 || count <= config.guard.max_installs {
        reaping.record_addition(count);
        return Ok(());
    }
    if config.allow_mass_install {
        warn!(
            "the install count for '{}' ({}) was allowed by --allow-mass-install.",
            scope.as_str(),
            count
        );
        reaping.record_addition(count);
        return Ok(());
    }

    refuse(format!(
        "{}: refusing this install.\n  \
         - it installs {} packages, over the limit of {} (config: max_installs)\n\n\
         This usually means a manifest matched more than you meant — run `shall plan` and \
         read the counts before proceeding.\n\n\
         What to do:\n  \
         shall plan                     see exactly what would be installed\n  \
         {} --allow-mass-install carry out this install anyway",
        scope.as_str(),
        count,
        config.guard.max_installs,
        scope.as_str(),
    ))
}

/// The gate for changes that take nothing away — a resource created or rewritten, a port opened.
///
/// They answer to no ceiling of their own: `max_total_changes` is the only number that counts
/// them, and nothing here can be `protected` (a thing that does not exist yet cannot be a thing
/// you asked Shall to keep). It exists so the total is a total. Before it, a sync could install
/// forty packages, write forty links and open forty ports under a `max_total_changes` of ten,
/// because the only gates on the way counted removals.
pub async fn enforce_additions(
    config: &Config,
    count: usize,
    reaping: &Reaping,
    scope: GuardScope,
) -> Result<()> {
    if count == 0 {
        return Ok(());
    }
    enforce_total(config, count, reaping, scope, "change")?;
    reaping.record_addition(count);
    Ok(())
}

/// `max_total_changes`, for the gates that have no per-kind ceiling of their own.
///
/// The removal gates check it inside [`inspect_removals`] instead, where it joins the rest of
/// the report and gets rendered with the protections and the per-kind count in one refusal.
fn enforce_total(
    config: &Config,
    count: usize,
    reaping: &Reaping,
    scope: GuardScope,
    noun: &str,
) -> Result<()> {
    let Some(Objection::TooMany {
        count,
        limit,
        setting,
    }) = too_many_changes(config, reaping, count)
    else {
        return Ok(());
    };
    // Either flag answers it, because both say the same sentence — "yes, that many, I meant it"
    // (`Y20`) — and a total is made of removals and installs both. A third flag for the third
    // ceiling would be a third way to say one thing.
    if config.allow_mass_removal || config.allow_mass_install {
        if let Some(said) = announcement(
            &[Objection::TooMany {
                count,
                limit,
                setting,
            }],
            config,
            scope,
        ) {
            warn!("{}", said);
        }
        return Ok(());
    }
    refuse(format!(
        "{}: refusing this {}.\n  \
         - it makes {} changes in total, over the limit of {} ([guard] {})\n\n\
         This ceiling counts everything one command does — installs and upgrades, packages \
         removed, resources torn down or written, ports opened and closed. The per-kind limits \
         each passed; the total did not.\n\n\
         What to do:\n  \
         shall plan                     see exactly what would change\n  \
         [guard] {}       raise or clear the total (preferences.toml)\n  \
         <command> --allow-mass-removal carry out this run anyway\n  \
         <command> --allow-mass-install the same — this total answers to either flag",
        scope.as_str(),
        noun,
        count,
        limit,
        TOTAL_KEY,
        TOTAL_KEY,
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
        // Shall must not remove what it could never have been asked to keep: a name that
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

    /// `purge-undeclared` sweeps everything Shall does not manage, and it builds its list from
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
            &Reaping::new(),
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

    /// Every per-kind ceiling set to the same number, so a test that says "a limit of two" means
    /// it for whichever kind it is about. `Y20` split them and `N8` split ports off again; a
    /// helper that moved only one would have left the other kinds' tests measuring against the
    /// untouched default of twenty and passing for no reason.
    fn config_with(max: usize) -> Config {
        Config {
            guard: crate::config::GuardSettings {
                protected_packages: vec!["python3".into(), "libpam*".into()],
                unprotected_packages: Vec::new(),
                max_removals: max,
                max_extra_removals: max,
                max_port_closures: max,
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
        assert!(enforce(
            &cfg,
            &reg,
            &pairs(&["python3"]),
            &Reaping::new(),
            GuardScope::Apply
        )
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
            enforce(
                &cfg,
                &reg,
                &pairs(&["jq", "htop", "bat"]),
                &Reaping::new(),
                GuardScope::Sync
            )
            .await
            .is_ok(),
            "the flag must let a big-but-ordinary removal through"
        );

        // A protected package, even when the flag is set and the count is fine.
        assert!(
            enforce(
                &cfg,
                &reg,
                &pairs(&["python3"]),
                &Reaping::new(),
                GuardScope::Sync
            )
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
        assert!(
            enforce_installs(&cfg, 10_000, &Reaping::new(), GuardScope::Sync)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn install_over_the_ceiling_is_refused() {
        let mut cfg = config_with(20);
        cfg.guard.max_installs = 50;
        let err = enforce_installs(&cfg, 51, &Reaping::new(), GuardScope::Sync)
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
        assert!(
            enforce_installs(&cfg, 50, &Reaping::new(), GuardScope::Sync)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn allow_mass_install_clears_the_install_ceiling() {
        // Symmetric to --allow-mass-removal answering the removal count.
        let mut cfg = config_with(20);
        cfg.guard.max_installs = 50;
        cfg.allow_mass_install = true;
        assert!(
            enforce_installs(&cfg, 5_000, &Reaping::new(), GuardScope::Sync)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn yes_does_not_override_the_install_ceiling() {
        // -y is what every script passes; it must not green-light a manifest-globbed flood.
        let mut cfg = config_with(20);
        cfg.guard.max_installs = 50;
        cfg.yes = true;
        assert!(
            enforce_installs(&cfg, 5_000, &Reaping::new(), GuardScope::Sync)
                .await
                .is_err()
        );
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

    /// **Which lines were refused, not how many.** This asserted `os.len() == 2` and nothing
    /// else, so swapping `v != "latest"` for `v == "latest"` kept the count at two while
    /// reversing the verdict on every line: `@version=latest` passed as pinned and `@version=1.6`
    /// was refused as floating. A count is invariant under a permutation, which is exactly what
    /// a comparison flip produces.
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
                ("apt", "tree", Some("*")),      // floating -> refused
                ("apt", "htop", Some("")),       // empty is not a version -> refused
                ("apt", "jq", Some("1.6")),      // pinned -> ok
            ]),
        );

        let mut refused: Vec<&str> = os
            .iter()
            .map(|o| match o {
                Objection::Unpinned { key } => key.as_str(),
                other => panic!("pinned_only raised something that is not Unpinned: {other:?}"),
            })
            .collect();
        refused.sort_unstable();
        assert_eq!(
            refused,
            ["apt:curl", "apt:htop", "apt:tree", "apt:wget"],
            "a concrete version is the only thing that satisfies pinned_only, and `apt:jq` is \
             the only line here that has one"
        );
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
        let report =
            inspect_removals(&cfg, &reg, &all_six, RemovalKind::Extra, &Reaping::new()).await;
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
            let report = inspect_removals(
                &cfg,
                &reg,
                &extras(&[key]),
                RemovalKind::Extra,
                &Reaping::new(),
            )
            .await;
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
            &Reaping::new(),
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

        assert!(enforce(
            &cfg,
            &reg,
            &pairs(&["a", "b", "c"]),
            &reaping,
            GuardScope::Sync
        )
        .await
        .is_ok());
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
            enforce_extras(
                &cfg,
                &reg,
                &extras(&["link:/keep"]),
                &Reaping::new(),
                GuardScope::Sync
            )
            .await
            .is_err(),
            "nothing overrides protection — not even --allow-mass-removal"
        );
    }

    #[tokio::test]
    async fn a_teardown_refusal_does_not_advise_a_command_that_cannot_take_it() {
        // `shall unmanage` takes a package line. Offering it for a `link:` teardown names a
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
        assert!(!msg.contains("shall unmanage"), "{}", msg);
        assert!(msg.contains("shall plan"), "{}", msg);
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
        let msg = GuardReport {
            objections,
            ..Default::default()
        }
        .message(GuardScope::PurgeUndeclared, RemovalKind::Package);
        let count_line = msg.find("removes 25 packages").expect("count line present");
        let first_pkg = msg.find("apt:pkg0").expect("a package listed");
        assert!(count_line < first_pkg, "the count must lead");
        assert!(msg.contains("…and 15 more"), "the list must be capped");
    }

    fn ports(specs: &[&str]) -> Vec<(String, String)> {
        specs
            .iter()
            .map(|s| ("firewall".to_string(), s.to_string()))
            .collect()
    }

    /// **`N8`: a port closure is a removal, and it answers to its own ceiling.** Under `Y20` it
    /// spent `max_extra_removals`, so the run that first declares a perimeter consumed a budget
    /// meant for `link:`/`service:` teardowns and refused a resource change it had nothing to
    /// do with.
    #[tokio::test]
    async fn a_port_closure_answers_to_the_port_ceiling_and_no_other() {
        let reg = Arc::new(BackendRegistry::new());
        let mut cfg = config_with(3);
        cfg.guard.max_port_closures = 2;
        let reaping = Reaping::new();

        let err = enforce_ports(
            &cfg,
            &reg,
            &ports(&["22/tcp", "80/tcp", "443/tcp"]),
            &reaping,
            GuardScope::Sync,
        )
        .await
        .expect_err("three closures over a limit of two must be refused");
        let msg = err.to_string();
        assert!(msg.contains("closes 3 ports"), "{msg}");
        assert!(
            msg.contains("max_port_closures"),
            "the refusal must name the setting the reader changes: {msg}"
        );
        assert!(
            !msg.contains("max_extra_removals") && !msg.contains("max_removals"),
            "a port refusal must not send the reader to another kind's setting: {msg}"
        );
        assert_eq!(
            reaping.so_far(RemovalKind::Port),
            0,
            "refused, so not spent"
        );
    }

    /// The three removal budgets are three budgets. Nineteen of each passes; the count that
    /// objects is the total, and only when one is set.
    #[tokio::test]
    async fn each_removal_kind_spends_only_its_own_budget() {
        let reg = Arc::new(BackendRegistry::new());
        let cfg = config_with(3);
        let reaping = Reaping::new();

        assert!(enforce(
            &cfg,
            &reg,
            &pairs(&["a", "b", "c"]),
            &reaping,
            GuardScope::Sync
        )
        .await
        .is_ok());
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
            "packages must not have spent the teardown budget"
        );
        assert!(
            enforce_ports(
                &cfg,
                &reg,
                &ports(&["22/tcp", "80/tcp", "443/tcp"]),
                &reaping,
                GuardScope::Sync
            )
            .await
            .is_ok(),
            "neither of the first two may have spent the port budget"
        );
        assert_eq!(reaping.so_far(RemovalKind::Package), 3);
        assert_eq!(reaping.so_far(RemovalKind::Extra), 3);
        assert_eq!(reaping.so_far(RemovalKind::Port), 3);
        assert_eq!(reaping.changes_so_far(), 9);
    }

    /// **`N8`: the ceiling over everything.** Nine changes pass three ceilings of three and are
    /// still nine changes. Off by default, so this is what setting it buys.
    #[tokio::test]
    async fn the_total_catches_what_every_per_kind_ceiling_passes() {
        let reg = Arc::new(BackendRegistry::new());
        let mut cfg = config_with(3);
        cfg.guard.max_total_changes = 8;
        let reaping = Reaping::new();

        assert!(enforce(
            &cfg,
            &reg,
            &pairs(&["a", "b", "c"]),
            &reaping,
            GuardScope::Sync
        )
        .await
        .is_ok());
        assert!(enforce_extras(
            &cfg,
            &reg,
            &extras(&["link:/a", "link:/b", "link:/c"]),
            &reaping,
            GuardScope::Sync
        )
        .await
        .is_ok());
        let err = enforce_ports(
            &cfg,
            &reg,
            &ports(&["22/tcp", "80/tcp", "443/tcp"]),
            &reaping,
            GuardScope::Sync,
        )
        .await
        .expect_err("nine changes over a total of eight must be refused");
        let msg = err.to_string();
        assert!(msg.contains("makes 9 changes in total"), "{msg}");
        assert!(msg.contains("max_total_changes"), "{msg}");
        assert!(
            !msg.contains("max_port_closures"),
            "three ports are inside the port ceiling — naming it would send the reader to a \
             number that is not the problem: {msg}"
        );
    }

    /// Zero is off, and off is the default. A machine that never asked for a total must not
    /// start refusing the sync it ran yesterday.
    #[tokio::test]
    async fn the_total_is_off_by_default_and_when_zero() {
        assert_eq!(Config::default().guard.max_total_changes, 0);
        let reg = Arc::new(BackendRegistry::new());
        let cfg = config_with(0);
        let reaping = Reaping::new();
        let many: Vec<String> = (0..500).map(|i| format!("pkg{}", i)).collect();
        let refs: Vec<&str> = many.iter().map(|s| s.as_str()).collect();
        assert!(
            enforce(&cfg, &reg, &pairs(&refs), &reaping, GuardScope::Sync)
                .await
                .is_ok()
        );
        assert!(enforce_installs(&cfg, 5_000, &reaping, GuardScope::Sync)
            .await
            .is_ok());
        assert!(enforce_additions(&cfg, 5_000, &reaping, GuardScope::Sync)
            .await
            .is_ok());
    }

    /// The total counts what the per-kind ceilings never look at: installs and upgrades,
    /// resources written, ports opened. A total that only counted removals would be the removal
    /// ceiling with a longer name.
    #[tokio::test]
    async fn the_total_counts_additions_as_well_as_removals() {
        let reg = Arc::new(BackendRegistry::new());
        let mut cfg = config_with(0);
        cfg.guard.max_total_changes = 10;
        let reaping = Reaping::new();

        assert!(enforce_installs(&cfg, 6, &reaping, GuardScope::Sync)
            .await
            .is_ok());
        assert!(enforce_additions(&cfg, 3, &reaping, GuardScope::Sync)
            .await
            .is_ok());
        assert_eq!(reaping.changes_so_far(), 9);

        let err = enforce(&cfg, &reg, &pairs(&["a", "b"]), &reaping, GuardScope::Sync)
            .await
            .expect_err("nine changes plus two removals is eleven, over ten");
        assert!(
            err.to_string().contains("makes 11 changes in total"),
            "{err}"
        );

        // And the additive gates answer to it in their own right.
        let err = enforce_additions(&cfg, 2, &reaping, GuardScope::Sync)
            .await
            .expect_err("nine plus two is eleven whichever gate counts it");
        assert!(err.to_string().contains("max_total_changes"), "{err}");
    }

    /// Either flag answers the total, because a total is made of both. Neither answers a
    /// protection, and `--allow-mass-install` does not answer a removal count.
    #[tokio::test]
    async fn both_mass_flags_answer_the_total_and_only_the_total() {
        let reg = Arc::new(BackendRegistry::new());
        let mut cfg = config_with(2);
        cfg.guard.max_total_changes = 2;
        cfg.allow_mass_install = true;

        assert!(
            enforce_installs(&cfg, 50, &Reaping::new(), GuardScope::Sync)
                .await
                .is_ok(),
            "--allow-mass-install must answer the total"
        );
        let err = enforce(
            &cfg,
            &reg,
            &pairs(&["a", "b", "c"]),
            &Reaping::new(),
            GuardScope::Sync,
        )
        .await
        .expect_err("the install flag must not answer a removal count");
        assert!(err.to_string().contains("max_removals"), "{err}");
        assert!(
            !err.to_string().contains("max_total_changes"),
            "the total it *did* answer must not still be in the refusal: {err}"
        );

        cfg.allow_mass_install = false;
        cfg.allow_mass_removal = true;
        assert!(
            enforce(
                &cfg,
                &reg,
                &pairs(&["a", "b", "c"]),
                &Reaping::new(),
                GuardScope::Sync
            )
            .await
            .is_ok(),
            "--allow-mass-removal answers both counts at once"
        );
    }

    /// What a mass flag answered is **on the report**, not only in the log.
    ///
    /// The three states this distinguishes look identical from the objection list alone: an
    /// objection that was never raised, one that was raised and cleared, and one that was raised
    /// and stands. Two of the three end with the same empty list, which is why the only witness
    /// used to be a `warn!` — and a line nothing can read is a fact nothing can check.
    #[tokio::test]
    async fn a_mass_flag_moves_the_count_onto_the_report_rather_than_deleting_it() {
        let reg = Arc::new(BackendRegistry::new());
        let mut cfg = config_with(2);
        let over_the_limit = pairs(&["a", "b", "c"]);

        // No flag: the count objects, and nothing has been answered.
        let mut report = inspect_removals(
            &cfg,
            &reg,
            &over_the_limit,
            RemovalKind::Package,
            &Reaping::new(),
        )
        .await;
        allow_the_count(&cfg, &mut report, GuardScope::Sync);
        assert!(
            matches!(
                report.objections.as_slice(),
                [Objection::TooMany { setting, .. }] if *setting == "max_removals"
            ),
            "{:?}",
            report.objections
        );
        assert!(
            report.allowed_by_flag.is_empty(),
            "no flag was passed, so nothing was allowed: {:?}",
            report.allowed_by_flag
        );

        // With it: the same objection is answered, and the report says which one and how far
        // over it was — the numbers, so a permutation of the list cannot pass for a clearance.
        cfg.allow_mass_removal = true;
        let mut report = inspect_removals(
            &cfg,
            &reg,
            &over_the_limit,
            RemovalKind::Package,
            &Reaping::new(),
        )
        .await;
        allow_the_count(&cfg, &mut report, GuardScope::Sync);
        assert!(report.is_empty(), "{:?}", report.objections);
        assert!(
            matches!(
                report.allowed_by_flag.as_slice(),
                [Objection::TooMany {
                    count: 3,
                    limit: 2,
                    setting,
                }] if *setting == "max_removals"
            ),
            "{:?}",
            report.allowed_by_flag
        );

        // A protection is not a count. The flag neither clears it nor claims to have.
        let mut report = inspect_removals(
            &cfg,
            &reg,
            &pairs(&["python3"]),
            RemovalKind::Package,
            &Reaping::new(),
        )
        .await;
        allow_the_count(&cfg, &mut report, GuardScope::Sync);
        assert!(
            matches!(report.objections.as_slice(), [Objection::Protected { .. }]),
            "{:?}",
            report.objections
        );
        assert!(
            report.allowed_by_flag.is_empty(),
            "a refusal is not something a flag answers (V.26): {:?}",
            report.allowed_by_flag
        );
    }

    /// The identities a `protected_packages` rule is offered, and no others.
    ///
    /// **Asserted on the list, because the caller cannot see it.** `inspect_removals` runs
    /// `find_map` over the result, so a redundant candidate is invisible there: a duplicate name
    /// answers the same question twice and an empty string matches no rule. Both guards in this
    /// function are therefore unfalsifiable one level up — which is exactly how
    /// `base != name && !base.is_empty()` came to survive being read as `||`.
    #[test]
    fn a_removal_offers_a_basename_only_when_it_has_one_to_offer() {
        // A package contributes its name and nothing else, whatever the name looks like. A
        // `link:` path and an `apt:` package can be the same string and mean different things.
        assert_eq!(
            protected_names(RemovalKind::Package, "/home/u/.vimrc"),
            ["/home/u/.vimrc"]
        );
        // A port is not a path: `22/tcp` splits into something, and `tcp` is not an identity
        // anybody wrote a rule about.
        assert_eq!(protected_names(RemovalKind::Port, "22/tcp"), ["22/tcp"]);

        // An extra whose identity is a path contributes both, on either separator, so a user may
        // name the thing rather than the absolute path Shall happens to key it by.
        assert_eq!(
            protected_names(RemovalKind::Extra, "/home/u/.vimrc"),
            ["/home/u/.vimrc", ".vimrc"]
        );
        assert_eq!(
            protected_names(RemovalKind::Extra, "C:\\Users\\u\\vimrc"),
            ["C:\\Users\\u\\vimrc", "vimrc"]
        );

        // An extra that is already a bare name contributes it once.
        assert_eq!(protected_names(RemovalKind::Extra, "vimrc"), ["vimrc"]);
        // And one ending in a separator has no basename to offer, rather than an empty one.
        assert_eq!(
            protected_names(RemovalKind::Extra, "/home/u/"),
            ["/home/u/"]
        );
    }

    /// A run that needed no override says nothing, and one that did names what it overrode.
    #[test]
    fn the_announcement_is_made_only_by_a_run_that_needed_one() {
        let mut cfg = config_with(10);
        cfg.allow_mass_removal = true;
        assert_eq!(
            announcement(&[], &cfg, GuardScope::Sync),
            None,
            "announcing an override on every run that did not need one is the same defect \
             pointing the other way"
        );
        let said = announcement(
            &[Objection::TooMany {
                count: 40,
                limit: 10,
                setting: "max_extra_removals",
            }],
            &cfg,
            GuardScope::PurgeUndeclared,
        )
        .expect("an objection a flag cleared is announced");
        assert!(said.contains("managed resources"), "{said}");
        assert!(
            said.contains("purge-undeclared"),
            "the line has to name the command that was allowed: {said}"
        );
    }

    /// The line names the flag **this run passed**, and the ceiling **that objection carried**.
    ///
    /// Both halves used to come from somewhere else: the flag was the literal
    /// `--allow-mass-removal` whatever was passed, and the noun was the caller's, so a run of
    /// `sync --allow-mass-install` — which answers `max_total_changes` and nothing else (`N8`) —
    /// read *"the removal count … was allowed by --allow-mass-removal"*, naming a ceiling it had
    /// not cleared and a flag it had not been given (`J9`).
    ///
    /// **Asserted with `!contains` in both directions.** A sentence that names both flags every
    /// time passes every `contains` here and is exactly as wrong as the literal it replaced;
    /// only the absent half distinguishes them.
    #[test]
    fn the_announcement_names_the_flag_the_run_passed_and_the_ceiling_it_cleared() {
        let total = |c, l| Objection::TooMany {
            count: c,
            limit: l,
            setting: TOTAL_KEY,
        };
        let per_kind = |c, l| Objection::TooMany {
            count: c,
            limit: l,
            setting: "max_removals",
        };

        // Install-only: the flag answers the total, so the total is what the line may name.
        let mut cfg = config_with(10);
        cfg.allow_mass_install = true;
        let said = announcement(&[total(62, 50)], &cfg, GuardScope::Sync).expect("a cleared count");
        assert!(said.contains("--allow-mass-install"), "{said}");
        assert!(
            !said.contains("--allow-mass-removal"),
            "a run that never typed it must not be told it was used: {said}"
        );
        assert!(
            said.contains("makes 62 changes in total") && said.contains(TOTAL_KEY),
            "the ceiling comes off the objection, not off the caller's noun: {said}"
        );
        assert!(
            !said.contains("removal count"),
            "an install-only run removed nothing: {said}"
        );

        // Removal-only, the mirror image, on a per-kind ceiling only that flag can clear.
        let mut cfg = config_with(10);
        cfg.allow_mass_removal = true;
        let said =
            announcement(&[per_kind(40, 10)], &cfg, GuardScope::Sync).expect("a cleared count");
        assert!(said.contains("--allow-mass-removal"), "{said}");
        assert!(
            !said.contains("--allow-mass-install"),
            "the sentence must not name every flag that exists: {said}"
        );
        assert!(said.contains("removes 40 packages"), "{said}");

        // Both passed: both named, because for the one ceiling either answers, either did.
        let mut cfg = config_with(10);
        cfg.allow_mass_removal = true;
        cfg.allow_mass_install = true;
        let said = announcement(&[total(62, 50)], &cfg, GuardScope::Sync).expect("a cleared count");
        assert!(
            said.contains("--allow-mass-removal") && said.contains("--allow-mass-install"),
            "{said}"
        );

        // Two ceilings cleared at once: both are named, as the refusal names both when they
        // stand. A run told about one of them raises that number and meets the other.
        let said = announcement(&[per_kind(40, 10), total(62, 50)], &cfg, GuardScope::Sync)
            .expect("a cleared count");
        assert!(
            said.contains("max_removals") && said.contains(TOTAL_KEY),
            "{said}"
        );

        // Neither flag: there is no sentence at all, rather than a sentence with a placeholder
        // where the flag belongs. An announcement is a report that an override was used, so a
        // run that used none has nothing to announce however many objections it is handed.
        let cfg = config_with(10);
        assert_eq!(
            announcement(&[total(62, 50)], &cfg, GuardScope::Sync),
            None,
            "no flag was passed, so nothing allowed anything"
        );
    }

    /// The way past `max_total_changes` is **either** flag, and the refusal that blocks a run
    /// has to say so.
    ///
    /// It named `--allow-mass-removal` alone, which is the more expensive half of `J9`: this is
    /// the instruction someone reads while blocked, so a pure-install run was told that the way
    /// to get its installs through was to authorize mass deletion. `shall protected` has printed
    /// the true rule all along — the guard's own refusal was the surface contradicting it.
    #[test]
    fn the_total_ceiling_refusal_offers_both_flags() {
        let mut cfg = config_with(10);
        cfg.guard.max_total_changes = 2;
        let err = enforce_total(&cfg, 3, &Reaping::new(), GuardScope::Sync, "install")
            .expect_err("three changes over a total of two is a refusal");
        let err = err.to_string();
        assert!(err.contains("--allow-mass-removal"), "{err}");
        assert!(
            err.contains("--allow-mass-install"),
            "either flag answers this ceiling, and the blocked run has to be told: {err}"
        );

        // The per-kind ceilings are *not* siblings of this: `max_removals`,
        // `max_extra_removals` and `max_port_closures` answer to `--allow-mass-removal` alone
        // (`Y20`), so offering the install flag there would name one that does not work.
        let report = GuardReport {
            objections: vec![Objection::TooMany {
                count: 40,
                limit: 10,
                setting: "max_removals",
            }],
            allowed_by_flag: Vec::new(),
        };
        let refusal = report.message(GuardScope::Sync, RemovalKind::Package);
        assert!(refusal.contains("--allow-mass-removal"), "{refusal}");
        assert!(
            !refusal.contains("--allow-mass-install"),
            "a removal ceiling does not answer to it: {refusal}"
        );
    }

    /// A refusal that names two ceilings when two are busted. Naming one sends a user to raise
    /// a number and meet the other on the next run.
    #[tokio::test]
    async fn a_set_over_two_ceilings_is_told_about_both() {
        let reg = Arc::new(BackendRegistry::new());
        let mut cfg = config_with(2);
        cfg.guard.max_total_changes = 2;
        let err = enforce(
            &cfg,
            &reg,
            &pairs(&["a", "b", "c"]),
            &Reaping::new(),
            GuardScope::Sync,
        )
        .await
        .expect_err("three is over both");
        let msg = err.to_string();
        assert!(msg.contains("max_removals"), "{msg}");
        assert!(msg.contains("max_total_changes"), "{msg}");
    }

    /// Protection reaches a port the same way it reaches a resource: `protected_packages` names
    /// the thing, and the count is not what is being asked.
    #[tokio::test]
    async fn a_protected_port_is_refused_whatever_the_count() {
        let reg = Arc::new(BackendRegistry::new());
        let mut cfg = config_with(20);
        cfg.guard.protected_packages = vec!["22/tcp".into()];
        cfg.allow_mass_removal = true;
        let err = enforce_ports(
            &cfg,
            &reg,
            &ports(&["22/tcp"]),
            &Reaping::new(),
            GuardScope::Sync,
        )
        .await
        .expect_err("a protected port is a refusal, not a count");
        assert!(err.to_string().contains("22/tcp"), "{err}");
    }

    /// The preview is the enforcer's question, asked without spending anything — including the
    /// total, which is what a two-list preview with a hand-written `also_removing` could never
    /// get right.
    #[tokio::test]
    async fn the_preview_sees_the_total_the_enforcer_will() {
        let reg = Arc::new(BackendRegistry::new());
        let mut cfg = config_with(0);
        cfg.guard.max_total_changes = 5;
        let refusals = preview_refusals(
            &cfg,
            &reg,
            2,
            &pairs(&["a", "b"]),
            &extras(&["link:/a", "link:/b"]),
            GuardScope::Apply,
        )
        .await;
        assert!(
            refusals
                .iter()
                .any(|r| r.contains("makes 6 changes in total")),
            "two installs, two packages and two teardowns is six: {refusals:?}"
        );
    }

    // ---- The OS-essential protection, driven against a backend that reports one. -----------
    //
    // **`cargo mutants` over this file on 2026-08-13 found the one hole in it.** 125 mutants:
    // 101 killed by the unit tests, 11 rejected by the compiler, and of the 13 survivors the
    // integration suite killed all but four. Three of the four are this function:
    //
    // ```text
    // MISSED  replace essential_names -> HashSet<String> with HashSet::new()
    // MISSED  replace essential_names -> HashSet<String> with HashSet::from_iter([String::new()])
    // MISSED  replace essential_names -> HashSet<String> with HashSet::from_iter(["xyzzy".into()])
    // ```
    //
    // Measured rather than asserted: a Linux binary carrying the first mutation was built and
    // run against a clean one on the same image and scenario. The clean binary refused with
    // exit 3 and five protections — `tar`, `sed`, `grep`, `gzip`, `findutils`. The mutant was
    // silent, exited 1, and attempted the removal. **The machine survived only because apt
    // refuses to remove its own `Essential: yes` packages** — a second line of defence Shall
    // does not own, does not check for, and cannot assume of every backend.
    //
    // **Nothing in this repository caught it.** Not the 1,814 lib tests, not the 535
    // integration tests, and not the 425-check container harness — the mutant binary swept the
    // `tools` image at 424 pass / 1 fail, the same single failure the clean binary has. Even
    // `protected includes a system essential`, which the harness greps, passes: that command
    // reports the *static* config rules and is byte-identical between the two binaries.
    //
    // **The reason is a missing input, not a missing assertion.** The suite is hermetic and no
    // mock had ever reported an essential set, so `essential_names` returned empty in every test
    // that has ever run — and a function that always returns empty is indistinguishable from one
    // hard-coded to. `Essentials` below is that input.

    /// A backend that answers the essential query, which no other test fixture in the tree does.
    struct Essentials {
        name: String,
        essential: Vec<String>,
        listings: crate::core::installed::InstalledListings,
    }

    #[async_trait::async_trait]
    impl crate::core::manager::BackendCore for Essentials {
        fn name(&self) -> &str {
            &self.name
        }
        fn is_available(&self) -> bool {
            true
        }
        fn probes(&self) -> Vec<String> {
            Vec::new()
        }
        fn needs_root(&self) -> bool {
            false
        }
    }

    #[async_trait::async_trait]
    impl crate::core::manager::Queryable for Essentials {
        fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
            (&self.listings, &self.name)
        }
        async fn fetch_installed(&self) -> crate::core::Result<Vec<crate::core::Package>> {
            Ok(Vec::new())
        }
        async fn list_manual(&self) -> crate::core::Result<Vec<crate::core::Package>> {
            Ok(Vec::new())
        }
        async fn info(&self, _: &str) -> crate::core::Result<Option<crate::core::Package>> {
            Ok(None)
        }
        async fn fetch_essential(&self) -> crate::core::Result<Vec<String>> {
            Ok(self.essential.clone())
        }
    }

    fn registry_reporting(backend: &str, essential: &[&str]) -> Arc<BackendRegistry> {
        let fake = Arc::new(Essentials {
            name: backend.to_string(),
            essential: essential.iter().map(|s| s.to_string()).collect(),
            listings: Default::default(),
        });
        let mut reg = BackendRegistry::new();
        reg.register(Arc::new(
            crate::core::manager::BackendCapabilities::builder(fake.clone())
                .with_queryable(fake)
                .build(),
        ));
        Arc::new(reg)
    }

    /// `essential_names` returns what the backend said, qualified by backend.
    ///
    /// Kills all three surviving mutants directly: `HashSet::new()` fails the emptiness check,
    /// `[String::new()]` and `["xyzzy"]` fail the equality.
    #[tokio::test]
    async fn essential_names_reports_what_the_backend_says_is_essential() {
        let registry = registry_reporting("apt", &["tar", "sed", "grep"]);
        let backends: HashSet<String> = ["apt".to_string()].into_iter().collect();

        let names = essential_names(&registry, &backends, 4).await;

        let want: HashSet<String> = ["apt:tar", "apt:sed", "apt:grep"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(
            names, want,
            "the essential set must be exactly what the backend reported, qualified by backend"
        );
    }

    /// A backend that reports nothing essential contributes nothing — the control that makes the
    /// test above a measurement rather than a coincidence.
    #[tokio::test]
    async fn a_backend_with_no_essential_concept_contributes_nothing() {
        let registry = registry_reporting("cargo", &[]);
        let backends: HashSet<String> = ["cargo".to_string()].into_iter().collect();
        assert!(essential_names(&registry, &backends, 4).await.is_empty());
    }

    /// And the protection those names buy: a removal of one is refused, by the OS-essential
    /// rule rather than by a config rule.
    ///
    /// This is the assertion the container measured on two binaries. It is here so that the
    /// next person to delete `essential_names` finds out on their own machine in 30ms rather
    /// than from apt on somebody's laptop.
    #[tokio::test]
    async fn a_package_the_os_calls_essential_is_never_removed() {
        let registry = registry_reporting("apt", &["tar", "sed", "grep", "gzip", "findutils"]);
        // No `protected_packages` rule covers these — the static config list is a different
        // mechanism and the container measurement turned specifically on the five it misses.
        let cfg = Config::default();
        let reaping = Reaping::new();

        let report = inspect_removals(
            &cfg,
            &registry,
            &pairs(&["tar", "sed", "grep", "gzip", "findutils"]),
            RemovalKind::Package,
            &reaping,
        )
        .await;

        assert!(
            !report.is_empty(),
            "five packages the OS reports as essential were planned for removal and the guard \
             said nothing"
        );
        let message = report.message(GuardScope::Sync, RemovalKind::Package);
        for name in ["tar", "sed", "grep", "gzip", "findutils"] {
            assert!(
                message.contains(name),
                "`{name}` is essential to the running system and the refusal does not name \
                 it:\n{message}"
            );
        }
    }

    /// The ceiling triggers **above** `max_removals`, not at it.
    ///
    /// The second finding in the same mutation run: three of the six survivors were
    /// numeric-boundary mutations, and `too_many_changes`'s `>`→`>=` is the behavioural one —
    /// it moves the refusal to fire *at* the limit instead of past it. It errs safe, which is
    /// why it is a boundary test and not a bug report, and it says the same thing about the
    /// other two: the thresholds were tested for "well over" and "well under" and never at the
    /// limit itself, which is the only place a ceiling is interesting.
    #[tokio::test]
    async fn the_removal_ceiling_fires_past_the_limit_and_not_at_it() {
        let cfg = Config {
            guard: crate::config::GuardSettings {
                max_removals: 3,
                ..Default::default()
            },
            ..Default::default()
        };
        let registry = registry_reporting("apt", &[]);

        // Exactly at the limit: allowed.
        let reaping = Reaping::new();
        let at = inspect_removals(
            &cfg,
            &registry,
            &pairs(&["a", "b", "c"]),
            RemovalKind::Package,
            &reaping,
        )
        .await;
        assert!(
            at.objections.is_empty(),
            "three removals under a ceiling of three is at the limit, not over it: {:?}",
            at.objections
        );

        // One past it: refused.
        let reaping = Reaping::new();
        let over = inspect_removals(
            &cfg,
            &registry,
            &pairs(&["a", "b", "c", "d"]),
            RemovalKind::Package,
            &reaping,
        )
        .await;
        assert!(
            !over.objections.is_empty(),
            "four removals under a ceiling of three must be refused"
        );
    }

    /// The same boundary, asked of the function the boundary lives in.
    ///
    /// [`the_removal_ceiling_fires_past_the_limit_and_not_at_it`] was written to kill
    /// `too_many_changes`'s `>`→`>=`, and did not: it sets `max_removals`, which is a
    /// *per-kind* ceiling checked elsewhere, so `too_many_changes` — which reads
    /// `max_total_changes` and nothing else — never saw a total equal to its limit. The mutant
    /// was still alive nine days later. A boundary test has to be pointed at the comparison it
    /// names, not at a ceiling that sounds like it.
    #[test]
    fn the_total_ceiling_fires_past_the_limit_and_not_at_it() {
        let cfg = |limit| Config {
            guard: crate::config::GuardSettings {
                max_total_changes: limit,
                ..Default::default()
            },
            ..Default::default()
        };

        assert!(
            too_many_changes(&cfg(3), &Reaping::new(), 3).is_none(),
            "three changes under a total of three is at the limit, not over it"
        );
        assert!(
            too_many_changes(&cfg(3), &Reaping::new(), 4).is_some(),
            "four changes under a total of three must be refused"
        );
        // The other half of the same guard: nought is off, not a ceiling of nought.
        assert!(
            too_many_changes(&cfg(0), &Reaping::new(), 9_999).is_none(),
            "max_total_changes = 0 disables the total; it does not refuse everything"
        );
        // And a limit of one, where `>` and `>=` disagree about the smallest set there is.
        assert!(too_many_changes(&cfg(1), &Reaping::new(), 1).is_none());
        assert!(too_many_changes(&cfg(1), &Reaping::new(), 2).is_some());
    }

    /// The cap says "…and N more" only when there are more.
    ///
    /// `protected.len() > MAX_LISTED` reads `>=` just as happily, and every test of this message
    /// used 25 objections against a cap of 10 — far enough over that both comparisons agree.
    /// At exactly the cap they disagree, and the mutant's version prints "…and 0 more
    /// protected package(s)" under a list that already showed every one of them.
    #[test]
    fn the_capped_list_claims_more_only_when_there_is_more() {
        let protected = |n: usize| GuardReport {
            objections: (0..n)
                .map(|i| Objection::Protected {
                    key: format!("apt:pkg{}", i),
                    reason: "protected by config rule `x`".into(),
                })
                .collect(),
            ..Default::default()
        };

        let exactly = protected(MAX_LISTED).message(GuardScope::Sync, RemovalKind::Package);
        assert!(
            !exactly.contains("…and"),
            "ten protected packages all fit under a cap of ten, so there is no remainder to \
             announce:\n{exactly}"
        );
        assert!(
            exactly.contains(&format!("apt:pkg{}", MAX_LISTED - 1)),
            "the last one that fits must still be listed:\n{exactly}"
        );

        let one_over = protected(MAX_LISTED + 1).message(GuardScope::Sync, RemovalKind::Package);
        assert!(
            one_over.contains("…and 1 more protected package(s)"),
            "eleven against a cap of ten leaves exactly one unlisted:\n{one_over}"
        );
    }

    /// Every objection renders a reason that names itself.
    ///
    /// `describe_objection` is the install side's whole explanation of a refusal, and nothing
    /// asserted a word of it — replacing the body with `String::new()` and with `"xyzzy"` both
    /// passed the suite. A refusal a user cannot read is a refusal that will be read as a bug
    /// in Shall.
    #[test]
    fn every_objection_describes_itself() {
        let cases = [
            (
                Objection::Denied {
                    key: "apt:telnet".into(),
                },
                vec!["apt:telnet", "deny_packages"],
            ),
            (
                Objection::Unpinned {
                    key: "apt:curl".into(),
                },
                vec!["apt:curl", "pinned_only", "@version="],
            ),
            (
                Objection::Protected {
                    key: "apt:python3".into(),
                    reason: "an OS essential".into(),
                },
                vec!["apt:python3", "an OS essential"],
            ),
            (
                Objection::TooMany {
                    count: 42,
                    limit: 20,
                    setting: "max_removals",
                },
                vec!["42", "20", "max_removals"],
            ),
            (
                Objection::TooManyInstalls {
                    count: 99,
                    limit: 50,
                },
                vec!["99", "50", "max_installs"],
            ),
        ];

        let mut seen: Vec<String> = Vec::new();
        for (objection, must_name) in cases {
            let text = describe_objection(&objection);
            for needle in must_name {
                assert!(
                    text.contains(needle),
                    "`{text}` does not name `{needle}`, so the user cannot tell what refused \
                     them or what to change"
                );
            }
            assert!(
                !seen.contains(&text),
                "two objections describe themselves identically (`{text}`), so the sentence \
                 does not identify which one fired"
            );
            seen.push(text);
        }
    }
}
