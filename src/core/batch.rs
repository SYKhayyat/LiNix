//! One manager command: how it is run, and what happens to the packages on it when it fails.
//!
//! **Lifted out of `transaction.rs` by `M3`, and the boundary is a subject rather than a line
//! count.** That file schedules a DAG - waves, batching, rollback, the journal. This one is
//! about a single command line: the retry loop around it, and the narrowing that works out
//! which of its packages were actually the problem when it comes back failed.
//!
//! Nothing here touches the WAL or the hooks, deliberately. `execute_batch_with_retry` owns
//! those, because `narrow_batch` asks the manager again and a re-ask must not re-open a journal
//! entry or fire `before_install` twice - a narrowing is a retry with a shorter command line,
//! and a retry has never done either.

use super::transaction::ContinuePast;
use super::transaction::{
    backoff_for, falsify_transience, lock_wait_verdict, wait_for_manager_lock, LockBudget,
    LockWait, TransactionConfig,
};
use crate::core::{Error, PackageSpec, Retryability};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
/// What a batch does after its command fails for a reason Shall classed as passing.
///
/// **A failed batch fails every package on its command line** (II.19 puts them there), so the
/// twenty-nine packages beside the one bad member are lost for that run. Narrowing gets them
/// back by asking the manager again with a shorter list - and the shorter list is not free.
/// Measured on Ubuntu and recorded on `execute_batch_with_retry`: `apt install <8>` as one
/// command is 3,161 ms, and those same eight one at a time are 31,901 ms. Ten times, and
/// superlinear, because each invocation re-reads the cache, re-takes the dpkg lock and
/// re-resolves a graph the batch resolves once.
///
/// So the strategy matters more than the switch, which is why this is a kind and not a bool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BatchRecovery {
    /// Ask once. A failed batch fails as a unit, which is what every run did before `M3`.
    Off,
    /// Halve the batch, ask about each half, and recurse only into a half that failed.
    ///
    /// **The stopping rule falls out of the shape.** One bad member can only be in ONE half, so
    /// two halves that both fail is not a bad member - it is the manager, its index or its lock,
    /// and every further question gets the same answer. That case stops at two extra commands
    /// instead of thirty, which matters because it is the case `M2` is named after: a rotated
    /// signing key fails every package equally.
    ///
    /// One bad member in thirty costs about `2*log2(30)` commands rather than thirty, and every
    /// one of them is still a batch, so apt's amortisation is kept where splitting to singles
    /// throws it away.
    #[default]
    Bisect,
    /// One command per member, always, whatever the first answers say.
    ///
    /// The thorough answer and the expensive one: on a manager-wide failure it pays the full
    /// 10x to learn what the first command already said. For a fast local mirror where the
    /// wall-clock does not matter and losing a run's installs does.
    Every,
}

impl BatchRecovery {
    /// Whether a failed command is worth asking again in pieces.
    ///
    /// Only a failure Shall classed as passing, only when there is more than one member to tell
    /// apart, and only when the run would carry on past it anyway.
    ///
    /// **That last condition is not an optimisation.** A `Permanent` failure ends the
    /// transaction whatever the pieces say, so narrowing one spends commands to fill in a report
    /// nobody reaches. And a run configured all-or-nothing means it: narrowing there would
    /// install the good members of a batch on a machine whose owner asked for a plan that either
    /// lands or does not, which is the opposite of what the key off is for.
    pub fn narrows(self, error: &Error, members: usize, continue_past: ContinuePast) -> bool {
        members > 1
            && self != Self::Off
            && continue_past.carries_on(true)
            && matches!(
                error.retryability(),
                Retryability::Transient | Retryability::Exhausted
            )
    }
}

/// What one manager command came back with.
///
/// Three states and not `Result`, because cancellation is neither: the WAL entry it leaves needs
/// a different sentence from the one a failure leaves, and squeezing it into an `Err` was how the
/// distinction got lost the first time it was tried.
pub(super) enum CommandOutcome {
    /// The manager did the work.
    Done { attempt: u32 },
    /// The run was cancelled before this command ran.
    Cancelled { attempt: u32 },
    /// The manager failed. The error has already been through `falsify_transience`, so a
    /// `Transient` that survived its retries reads `Exhausted` here.
    Failed { attempt: u32, error: Error },
}

impl CommandOutcome {
    pub(super) fn attempt(&self) -> u32 {
        match self {
            Self::Done { attempt } | Self::Cancelled { attempt } | Self::Failed { attempt, .. } => {
                *attempt
            }
        }
    }
}

/// One manager command over these packages, with the retry loop and the manager-lock wait.
///
/// **This does the command and nothing else.** The WAL entries, the hooks and the `TaskResult`s
/// belong to `execute_batch_with_retry`, and they have to, because `BatchRecovery` calls this
/// again over half a failed batch: re-opening a journal entry or firing `before_install` twice
/// would make a narrowing observably different from the retries this loop already does, which it
/// is not.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_one_command(
    specs: &[PackageSpec],
    names: &[String],
    backend_cap: &Arc<crate::core::BackendCapabilities>,
    b_name: &str,
    is_install: bool,
    config: &TransactionConfig,
    reaped: Option<crate::app::sync::guard::Reaped>,
    cancel_token: &CancellationToken,
) -> CommandOutcome {
    let mut attempt = 0;
    let mut last_error = None;
    let mut lock_budget = LockBudget::of(config.manager_lock_wait);

    // **A range, not a counter the body increments.** `while attempt <= max_retries` with
    // `attempt += 1` at the top is the same loop until the increment is wrong, and then it is
    // not a loop at all: read as `*=`, the counter stays at nought and a batch whose command
    // fails retries for ever. The mutation sweep reported that as a *timeout* - neither caught
    // nor survived, a shard red for two hours while naming no defect.
    //
    // The tries are `max_retries + 1`: the first attempt is not a retry, which is the same
    // arithmetic `retries_behind` reads back out.
    for this_attempt in 1..=config.max_retries.saturating_add(1) {
        attempt = this_attempt;
        if cancel_token.is_cancelled() {
            return CommandOutcome::Cancelled { attempt };
        }

        if attempt > 1 {
            // **Another package manager is not a failure to back off from - it is one to wait
            // for.** A backoff is for a flake; this is a second program holding a lock it will
            // hand back when its own transaction finishes, and three doublings of half a second
            // do not outlast an `apt upgrade`. Only ever entered against a holder proved to be
            // alive: a lock left behind by a killed run is reported at once, because waiting on
            // it would never end.
            let budget = lock_budget.remaining();
            match lock_wait_verdict(&last_error, b_name, budget, &|b| {
                crate::app::stale_lock::held_for_on_this_machine(b)
            }) {
                LockWait::Wait(who) => {
                    match wait_for_manager_lock(b_name, &who, budget, cancel_token).await {
                        Ok(spent) => lock_budget.spend(spent),
                        Err(err) => {
                            last_error = Some(err);
                            break;
                        }
                    }
                }
                LockWait::Hopeless(err) => {
                    last_error = Some(err);
                    break;
                }
                LockWait::Backoff => {
                    let backoff = backoff_for(attempt, config.initial_backoff, config.max_backoff);
                    tokio::time::sleep(backoff).await;
                }
            }
        }

        // One node's timeout, scaled by how many packages the command carries: eight packages in
        // one `apt install` legitimately take longer than one, and a bound sized for one would
        // turn the batching win into a timeout.
        let deadline = config
            .node_timeout
            .saturating_mul(names.len().clamp(1, 16) as u32);
        let result = tokio::time::timeout(deadline, async {
            let Some(handler) = backend_cap.as_installable() else {
                return Err(Error::Transaction(format!(
                    "Backend '{}' is not {}.",
                    b_name,
                    if is_install {
                        "installable"
                    } else {
                        "removable"
                    }
                )));
            };
            if is_install {
                handler.install(specs, backend_cap.sudo_for_write()).await
            } else {
                let sudo = backend_cap.sudo_for_write();
                let Some(reaped) = reaped else {
                    return Err(Error::Refused(format!(
                        "a plan containing removals reached the executor without passing the \
                         removal guard - refusing to remove {}. This is a defect in whichever \
                         command built the plan, not in the config: the guard runs once over a \
                         whole plan (`max_removals` is a ceiling over a plan, not over one \
                         command), and the engine hands the executor the proof it ran.",
                        names.join(", ")
                    )));
                };
                if config.purge && handler.supports_purge() {
                    handler.purge(names, sudo, reaped).await
                } else {
                    handler.remove(names, sudo, reaped).await
                }
            }
        })
        .await;

        match result {
            Ok(Ok(())) => return CommandOutcome::Done { attempt },
            Ok(Err(e)) => {
                // A name no repository carries is not found by waiting; three rounds of backoff
                // only delay the report and hold the manager's lock while they do it. `Unknown`
                // still retries - that is what every failure did before this distinction existed,
                // and only a classified verdict overrides it.
                let give_up = e.retryability() == Retryability::Permanent;
                last_error = Some(e);
                if give_up {
                    break;
                }
            }
            Err(_) => {
                last_error = Some(Error::Transaction(format!(
                    "`{}` did not finish {} package(s) within {:?}.",
                    b_name,
                    names.len(),
                    deadline
                )));
            }
        }
    }

    CommandOutcome::Failed {
        attempt,
        error: falsify_transience(
            last_error.unwrap_or(Error::Transaction("Unknown error".into())),
            attempt,
        ),
    }
}

/// Ask the manager again in pieces, and work out which members were actually the problem.
///
/// Returns one verdict per position, in the order the caller handed them over.
///
/// **`Bisect` halves, and stops when halving stops answering.** One bad member can only be in
/// ONE half, so two halves that both fail is not a member - it is the manager, its index or its
/// lock, and thirty more questions get the same answer thirty more times. That is the case `M2`
/// is named after, and it costs two extra commands here instead of thirty.
///
/// The ranges are contiguous because the halves of a contiguous range are, which is why this
/// needs no index vectors and no recursion: a worklist of `(lo, hi)` says everything.
#[allow(clippy::too_many_arguments)]
pub(super) async fn narrow_batch(
    specs: &[PackageSpec],
    names: &[String],
    backend_cap: &Arc<crate::core::BackendCapabilities>,
    b_name: &str,
    is_install: bool,
    config: &TransactionConfig,
    reaped: Option<crate::app::sync::guard::Reaped>,
    cancel_token: &CancellationToken,
) -> Vec<std::result::Result<(), Error>> {
    let n = names.len();
    let mut verdict: Vec<Option<std::result::Result<(), Error>>> = vec![None; n];

    // One command per member, whatever the halves would have said. Written as its own loop
    // rather than as a bisection that always recurses, because a bisection that never prunes
    // pays for every interior level as well - which is slower than the thing it is imitating.
    if config.batch_recovery == BatchRecovery::Every {
        for p in 0..n {
            let out = run_one_command(
                &specs[p..(p + 1).min(specs.len())],
                &names[p..p + 1],
                backend_cap,
                b_name,
                is_install,
                config,
                reaped,
                cancel_token,
            )
            .await;
            verdict[p] = Some(match out {
                CommandOutcome::Done { .. } => Ok(()),
                CommandOutcome::Cancelled { .. } => Err(Error::Cancelled),
                CommandOutcome::Failed { error, .. } => Err(error),
            });
        }
        return verdict
            .into_iter()
            .map(|v| v.expect("every position"))
            .collect();
    }

    let ask = |lo: usize, hi: usize| async move {
        run_one_command(
            &specs[lo.min(specs.len())..hi.min(specs.len())],
            &names[lo..hi],
            backend_cap,
            b_name,
            is_install,
            config,
            reaped,
            cancel_token,
        )
        .await
    };

    // A command's answer as `None` for done and `Some(error)` for not, because everything below
    // asks only that question of it.
    fn err_of(out: CommandOutcome) -> Option<Error> {
        match out {
            CommandOutcome::Done { .. } => None,
            CommandOutcome::Cancelled { .. } => Some(Error::Cancelled),
            CommandOutcome::Failed { error, .. } => Some(error),
        }
    }

    let mut work: Vec<(usize, usize)> = vec![(0, n)];
    while let Some((lo, hi)) = work.pop() {
        if hi - lo == 1 {
            verdict[lo] = Some(err_of(ask(lo, hi).await).map_or(Ok(()), Err));
            continue;
        }
        let mid = lo + (hi - lo) / 2;
        let left = err_of(ask(lo, mid).await);
        let right = err_of(ask(mid, hi).await);

        // **Both halves failed, so it is not a member.** One bad member can only be in one half.
        // Two failing halves is the manager, its index or its lock, and every further question
        // gets the same answer - which is the case `M2` is named after, and the reason this stops
        // at two extra commands rather than thirty. Each half's own error is kept for its own
        // members: it is about a shorter list than the parent's, so it is closer to the truth.
        if let (Some(le), Some(re)) = (left.clone(), right.clone()) {
            for (p, item) in verdict.iter_mut().enumerate().take(hi).skip(lo) {
                if item.is_none() {
                    *item = Some(Err(if p < mid { le.clone() } else { re.clone() }));
                }
            }
            continue;
        }

        for (half, (a, b)) in [(left, (lo, mid)), (right, (mid, hi))] {
            match half {
                // **A failed half of one has already been asked.** Pushing it would send the
                // identical command again to learn what this attempt just said - one wasted
                // invocation per narrowing, on a path whose whole justification is that
                // invocations are expensive.
                Some(error) if b - a == 1 => verdict[a] = Some(Err(error)),
                Some(_) => work.push((a, b)),
                None => {
                    for item in verdict.iter_mut().take(b).skip(a) {
                        if item.is_none() {
                            *item = Some(Ok(()));
                        }
                    }
                }
            }
        }
    }
    verdict.into_iter().map(|v| v.unwrap_or(Ok(()))).collect()
}
