use crate::app::sync::resolver::StateResolver;
use crate::verbs::perform_maintenance;
use crate::verbs::prelude::*;
use crate::verbs::setup::handle_canary;
use crate::verbs::sync::{enforce_policy, print_flight_plan};

/// Everything `handle_upgrade` needs, bundled so the dispatch site stays readable and the
/// handler doesn't grow an unwieldy positional signature.
pub struct UpgradeRequest<'a> {
    pub packages: &'a [String],
    pub backend: Option<&'a str>,
    pub all: bool,
    pub security: bool,
    pub except: &'a [String],
    /// Run the native whole-system upgrade knowing it cannot honour holds (B9).
    pub ignore_holds: bool,
    pub profile: &'a Option<String>,
    pub module: &'a Option<String>,
    pub out: Output,
    pub canary: bool,
    pub test: &'a Option<String>,
    /// `--steps` / `--no-steps`: whether to run the declared non-package steps. `None` is the
    /// default, which is "yes on a whole-machine upgrade, no on a narrowed one".
    pub steps: Option<bool>,
}

impl UpgradeRequest<'_> {
    /// Did the caller narrow this run to less than the whole machine?
    ///
    /// **Not the same question as `scope()`**, which answers only about a profile or a module.
    /// Naming a package, a manager, or `--security` narrows the run just as surely, and it was
    /// the package case that made `shall upgrade curl` fire every firmware step in the config.
    fn narrowed(&self) -> bool {
        !self.packages.is_empty()
            || self.backend.is_some()
            || self.security
            || self.canary
            || self.scope().is_some()
    }

    fn scope(&self) -> Option<PlannerScope> {
        if let Some(p) = self.profile {
            Some(PlannerScope::Profile(p.clone()))
        } else {
            self.module
                .as_ref()
                .map(|m| PlannerScope::Module(m.clone()))
        }
    }
}

/// True if `except` names this package, matching either the bare name or `backend:name`.
pub fn upgrade_excluded(except: &[String], backend: &str, name: &str) -> bool {
    let qualified = format!("{}:{}", backend, name);
    except
        .iter()
        .any(|e| e == name || e == &qualified || e.eq_ignore_ascii_case(name))
}

/// Upgrade a single managed package by routing through the normal install path. When
/// `version` is `Some`, pin to exactly that version (`options["version"]`, which pin-capable
/// backends honor) — used by `--security` to land on the fixed version rather than blindly
/// jumping to latest. `None` means "newest the backend offers".
pub async fn upgrade_one(
    journal: &Arc<tokio::sync::Mutex<crate::core::Journal>>,
    registry: &Arc<BackendRegistry>,
    resolver: &StateResolver<'_>,
    backend: &str,
    name: &str,
    version: Option<&str>,
) -> Result<bool> {
    let spec_str = format!("{}:{}", backend, name);
    let resolved = resolver.resolve_spec(&spec_str).await?;
    let mut acted = false;
    for mut spec in resolved {
        if let Some(v) = version {
            spec.options.set("version".to_string(), v.to_string());
        }
        // II.7c: a manager this machine does not have upgrades nothing, and says so. It was a
        // bare `if let` — so `upgrade` walked past every package on an absent manager without a
        // word and reported the ones it did as the whole job.
        let Some(b) = registry.get(&spec.backend).filter(|b| b.is_available()) else {
            warn!(
                "`{}` is not on this machine, so {}:{} cannot be upgraded here.",
                spec.backend, spec.backend, spec.name
            );
            continue;
        };
        if let Some(inst) = b.as_installable() {
            info!(
                "Upgrading {}:{} to {}...",
                spec.backend,
                spec.name,
                version.unwrap_or("latest")
            );
            // An upgrade is an install of a package that is already there, and an
            // interrupted one leaves the manager holding a half-replaced package with no
            // declaration describing the version it was moving to. The `@version=` pin
            // `--security` sets is inside the spec, so the recorded action is the upgrade
            // rather than a reinstall of whatever is newest.
            crate::core::journalled(
                journal,
                vec![crate::core::JournalAction::Install(spec.clone())],
                inst.install(std::slice::from_ref(&spec), b.sudo_for_write()),
            )
            .await?;
            acted = true;
        }
    }
    Ok(acted)
}

/// Upgrade an explicit set of managed packages (or one backend's worth) to latest.
pub async fn upgrade_targeted(
    app: &App,
    packages: &[String],
    backend: Option<&str>,
    except: &[String],
) -> Result<Option<usize>> {
    // Snapshot the managed set once so we can resolve names → backends without holding the lock.
    let managed: Vec<(String, String)> = {
        let state = app.state.lock().await;
        state
            .managed()
            .map(|p| (p.backend.clone(), p.name.clone()))
            .collect()
    };

    let mut targets: Vec<(String, String)> = Vec::new();
    if !packages.is_empty() {
        for req in packages {
            let (want_backend, want_name) =
                crate::config::parser::split_removal_target(req, |b| app.registry.get(b).is_some());
            let hit = managed
                .iter()
                .find(|(b, n)| n == &want_name && want_backend.as_ref().is_none_or(|wb| wb == b));
            match hit {
                Some((b, n)) => targets.push((b.clone(), n.clone())),
                None => {
                    // Not currently managed — still honor an explicit, backend-qualified
                    // upgrade by resolving it fresh; otherwise warn and skip.
                    match want_backend {
                        Some(b) => targets.push((b, want_name)),
                        None => {
                            eprintln!("upgrade: '{}' is not a managed package — skipping.", req)
                        }
                    }
                }
            }
        }
    } else if let Some(scope) = backend {
        for (b, n) in &managed {
            if b == scope {
                targets.push((b.clone(), n.clone()));
            }
        }
        if targets.is_empty() {
            println!("No managed packages under backend '{}'.", scope);
            return Ok(Some(0));
        }
    }

    // Apply --backend as a filter even when explicit packages were given, and drop excludes.
    // Held packages are skipped for a broad (--backend) upgrade, but an EXPLICITLY named
    // package overrides its hold (with a warning) — naming it is a clear intent to upgrade.
    let explicit = !packages.is_empty();

    // **Both sources, asked once.** The ledger `shall hold` writes and the `@hold=true` lines
    // the manifest declares. This command read only the first, so a package the user froze
    // declaratively was upgraded by every `shall upgrade` — and the first fix taught two of this
    // file's readers about the declaration and left a third, `remediate`, building its own
    // closure over the ledger where nothing grepping for `is_held` would find it.
    let holds = app.holds().await;

    // Dry-run: describe the upgrades (after filters/holds) without touching anything.
    if app.config.dry_run {
        crate::would_print!("would upgrade:");
        let mut n = 0;
        for (b, name) in &targets {
            if let Some(scope) = backend {
                if b != scope {
                    continue;
                }
            }
            if upgrade_excluded(except, b, name) {
                continue;
            }
            if !explicit && holds.contains(b, name) {
                continue;
            }
            println!("  ↑ {}:{}", b, name);
            n += 1;
        }
        if n == 0 {
            println!("  (nothing)");
        }
        return Ok(Some(0));
    }

    let mut upgraded = 0usize;
    let mut skipped = 0usize;
    for (b, n) in targets {
        if let Some(scope) = backend {
            if b != scope {
                continue;
            }
        }
        if upgrade_excluded(except, &b, &n) {
            skipped += 1;
            continue;
        }
        if holds.contains(&b, &n) {
            // Which command releases it, asked of the hold rather than assumed: telling
            // somebody to run `shall unhold` against a manifest line sends them to a command
            // that will report nothing to do.
            let release = holds.release(&b, &n);
            if explicit {
                eprintln!(
                    "upgrade: '{b}:{n}' is held — upgrading anyway because you named it (still \
                     held; {release} to change)."
                );
            } else {
                println!("upgrade: skipping held {b}:{n} ({release} to allow).");
                skipped += 1;
                continue;
            }
        }
        if upgrade_one(
            &app.journal,
            &app.registry,
            &app.resolver().await,
            &b,
            &n,
            None,
        )
        .await?
        {
            upgraded += 1;
        }
    }

    crate::core::save_off_the_runtime(&app.state).await?;
    println!(
        "Upgraded {} package(s){}.",
        upgraded,
        if skipped > 0 {
            format!(" ({} held back by --except)", skipped)
        } else {
            String::new()
        }
    );
    perform_maintenance(app).await?;
    Ok(Some(upgraded))
}

/// Upgrade exactly the packages `audit` reports as vulnerable, to a non-vulnerable version.
/// Honors `--except`. This is the `audit → upgrade` bridge.
pub async fn upgrade_security(app: &App, except: &[String], out: Output) -> Result<Option<usize>> {
    let report = crate::app::insight::audit(&app.config, &app.registry, &app.state).await?;
    if report.findings.is_empty() {
        if out.is_json() {
            println!("{}", serde_json::json!({ "upgraded": [], "vulnerable": 0 }));
        } else {
            println!(
                "No known vulnerabilities across {} scanned package(s) — nothing to upgrade.",
                report.scanned
            );
        }
        return Ok(Some(0));
    }

    // Aggregate advisories per package. A package can have several; to be safe from ALL of
    // them we must reach at least the HIGHEST fixed version across its advisories, so we take
    // the max `fixed` (not the first). Packages with no reported fix pin to None (→ latest).
    use version_compare::{compare, Cmp};
    // **Both sources, and this reader was the one nobody found.** It copied the ledger out of
    // `StateRegistry::held` into a closure of its own, so it contained neither `is_held(` nor
    // anything else a grep for the ledger's readers would match — and a package the manifest
    // froze with `@hold=true` was silently remediated by `upgrade --security`, which is a
    // change to a declared package against the declaration.
    let holds = app.holds().await;
    let mut order: Vec<String> = Vec::new();
    let mut agg: std::collections::HashMap<String, (String, String, Option<String>)> =
        std::collections::HashMap::new();
    let mut excluded_keys = std::collections::HashSet::new();
    let mut held_keys = std::collections::HashSet::new();
    for f in &report.findings {
        let key = format!("{}:{}", f.backend, f.name);
        if upgrade_excluded(except, &f.backend, &f.name) {
            excluded_keys.insert(key);
            continue;
        }
        // A held package is NOT silently remediated — hold is an explicit "don't touch". We
        // surface it loudly so the user can `unhold` and re-run if they want the fix.
        if holds.contains(&f.backend, &f.name) {
            held_keys.insert(key);
            continue;
        }
        let entry = agg.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            (f.backend.clone(), f.name.clone(), None)
        });
        if let Some(new_fixed) = &f.fixed {
            // Keep the larger of the current best and this advisory's fixed version.
            let keep_current =
                matches!(&entry.2, Some(cur) if compare(cur, new_fixed) == Ok(Cmp::Ge));
            if !keep_current {
                entry.2 = Some(new_fixed.clone());
            }
        }
    }
    let plan: Vec<(String, String, Option<String>)> =
        order.into_iter().filter_map(|k| agg.remove(&k)).collect();
    let seen_total = plan.len() + excluded_keys.len() + held_keys.len();
    let excepted = excluded_keys.len();
    if out.is_human() {
        println!(
            "Security upgrade: {} vulnerable package(s){}.",
            plan.len(),
            if excepted > 0 {
                format!(", {} held back by --except", excepted)
            } else {
                String::new()
            }
        );
        // Vulnerable AND held: neither auto-fixed nor silently ignored — call it out.
        if !held_keys.is_empty() {
            eprintln!(
                "warning: {} vulnerable package(s) are HELD and were NOT upgraded: {}. \
                 `shall unhold <pkg>` then re-run to remediate.",
                held_keys.len(),
                {
                    let mut v: Vec<_> = held_keys.iter().cloned().collect();
                    v.sort();
                    v.join(", ")
                }
            );
        }
    }

    // Dry-run: show the remediation plan without installing.
    if app.config.dry_run {
        if out.is_human() {
            crate::would_print!("would upgrade to remediate:");
            for (backend, name, fixed) in &plan {
                match fixed {
                    Some(v) => println!("  ↑ {}:{} → {}", backend, name, v),
                    None => println!("  ↑ {}:{} → latest", backend, name),
                }
            }
            if plan.is_empty() {
                println!("  (nothing)");
            }
        }
        return Ok(Some(0));
    }

    let mut upgraded = Vec::new();
    for (backend, name, fixed) in plan {
        // Pin to the fixed version when OSV reports one; pin-capable backends land exactly
        // there, and those that ignore the pin fall back to latest (still ≥ fixed).
        match upgrade_one(
            &app.journal,
            &app.registry,
            &app.resolver().await,
            &backend,
            &name,
            fixed.as_deref(),
        )
        .await
        {
            Ok(true) => upgraded.push(serde_json::json!({
                "backend": backend, "name": name, "pinned_to": fixed,
            })),
            Ok(false) => {}
            // Per the agreed policy: a package we can't remediate is a warning, not a stop.
            Err(e) => eprintln!("  warning: could not upgrade {}:{}: {}", backend, name, e),
        }
    }
    crate::core::save_off_the_runtime(&app.state).await?;

    if out.is_json() {
        let mut held_list: Vec<_> = held_keys.iter().cloned().collect();
        held_list.sort();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "vulnerable": seen_total,
                "upgraded": upgraded,
                "held_unremediated": held_list,
            }))?
        );
    } else {
        println!(
            "Upgraded {} package(s) to remediate advisories.",
            upgraded.len()
        );
    }
    let moved = upgraded.len();
    perform_maintenance(app).await?;
    Ok(Some(moved))
}

/// `shall upgrade` — move packages forward, then record where they landed.
///
/// The recording is not decoration. A pin that nobody updates fights the upgrade that just ran:
/// `sync` reads the recorded version back as `@version=`, finds the installed one no longer
/// satisfies it, and plans the package straight back down. Every mode below moves versions, so
/// every mode below is followed by this (Z2). Only packages that were already pinned are
/// touched — an upgrade is not a `lock`.
pub async fn handle_upgrade(app: &App, req: UpgradeRequest<'_>) -> Result<()> {
    let out = req.out;
    // Whether the steps run is decided from the request, before the modes consume it.
    let run_steps = req.steps.unwrap_or(!req.narrowed());
    let moved = upgrade_modes(app, req).await?;
    upgrade_steps(app, moved, run_steps).await?;
    let repinned =
        crate::verbs::plan::refresh_version_locks(&app.config, &app.registry, &app.state).await?;
    if repinned > 0 && out.is_human() {
        println!("Lock: re-recorded {} version pin(s).", repinned);
    }
    Ok(())
}

/// The declared steps that are not packages (`H6`) — firmware, a plugin manager, a tracked
/// repository, `rustup` components.
///
/// **`upgrade` ran none of them, and that was the whole gap.** Every mode above moves *managed
/// packages*, which is the one thing a manager can be asked to do; the rest of what a machine
/// needs upgrading is a command, and a command is an `exec:` line. Those lines existed, were
/// approval-gated, were journalled, and were run only by `sync` — so a user running
/// `shall upgrade` weekly never ran their firmware step however correctly they wrote it.
///
/// **Per step, never inherited.** Only lines carrying `@on=upgrade` or `@on=both` run here.
/// Widening `upgrade` to every `exec:` would make a verb that has never executed user scripts
/// start executing every script in every manifest that already exists, and the approval ledger
/// cannot object on a user's behalf: it answers *what* may run, never *which verb* may run it.
///
/// After the packages, deliberately: a firmware tool or a `rustup` component is usually the
/// thing you want brought forward *once the packages under it have moved*, which is the same
/// order `sync` runs its verb phase in.
///
/// **A scoped upgrade runs none of them.** `shall upgrade curl` asks for one package, and
/// firing every `@on=upgrade` step in the config alongside it — firmware, `rustup`, whatever
/// else the machine declares — is a great deal more than was asked for. The steps belong to
/// "bring this machine forward", which is what a bare `upgrade` means and a named package does
/// not. `--steps` asks for them anyway on a scoped run, and `--no-steps` declines them on an
/// unscoped one, so neither direction is unreachable.
async fn upgrade_steps(app: &App, moved: Option<usize>, run_them: bool) -> Result<()> {
    use crate::model::exec::Verb;

    if !run_them {
        return Ok(());
    }
    let state = app.resolver().await.resolve_model().await?;
    if state.execs_for(Verb::Upgrade).next().is_none() {
        return Ok(());
    }
    app.execs().apply(&state, Verb::Upgrade, moved).await?;
    Ok(())
}

/// Returns how many packages moved, or `None` where the mode cannot know.
///
/// The native whole-system path hands the work to `apt upgrade` and its siblings, which report
/// no per-package count Shall can trust — so it answers `None`, and `None` is not zero. An
/// `@after=` step is run on an uncountable path rather than skipped, because that path is the
/// one that moves the most.
async fn upgrade_modes(app: &App, req: UpgradeRequest<'_>) -> Result<Option<usize>> {
    // First, before any mode: `upgrade --backend aptt` used to scope to nothing and report
    // that everything was up to date (Q9).
    app.resolver().await.require_known_backend(req.backend)?;
    // And the same ruling on the form it takes positionally, which that enumeration missed:
    // `upgrade nosuchbackend:foo` answered "not a managed package — skipping" at exit 0.
    app.resolver()
        .await
        .require_known_spec_backends(req.packages)
        .await?;
    app.resolver()
        .await
        .require_known_spec_backends(req.except)
        .await?;

    // Canary keeps its own health-gated, scoped path.
    if req.canary {
        return handle_canary(app, req.scope(), req.test)
            .await
            .map(|()| None);
    }

    // Mode 1: audit-driven security upgrade.
    if req.security {
        return upgrade_security(app, req.except, req.out).await; // counts its own
    }

    // Mode 2: explicit packages, or a --backend scope → targeted managed upgrade.
    if !req.packages.is_empty() || req.backend.is_some() {
        return upgrade_targeted(app, req.packages, req.backend, req.except).await;
    }

    // Mode 3: --all, or a bare `upgrade` with no declarative scope → native whole-system
    // batch upgrade across every backend (this is the path that actually bumps
    // `latest`-pinned packages, which the constraint-driven planner never touches).
    if req.all || req.scope().is_none() {
        if !req.except.is_empty() {
            eprintln!(
                "note: --except is ignored for the native whole-system upgrade; \
                 pass package names or use --backend/--security to scope exclusions."
            );
        }
        // **Native batch upgrades cannot honour a hold, so this run is refused rather than
        // noted.** `apt upgrade` and its siblings run inside each manager and cannot be told to
        // skip a package: `upgrade` filters its own plan against the holds, correctly, and then
        // hands the whole-system path to a manager that never sees the filtered plan.
        //
        // It used to print a `note:` and carry on at exit 0, under a summary reading
        // `Status: SUCCESS  Installs: 0` while a held package moved two major versions. The
        // observation existed on the path the user runs — which is more than B0 or B1 managed —
        // and it was a note, so a weekly `shall upgrade` bumped the version somebody had pinned
        // precisely to stop that, and nothing in the exit code said so (B9).
        //
        // Refused, not removed: `--ignore-holds` is the explicit opt-in for "yes, upgrade the
        // whole machine anyway", and every scoped form of the verb honours holds and needs no
        // flag. Both hold sources are counted — the ledger's and the manifest's — because
        // counting only the ledger left somebody whose holds were all declared with no warning
        // at all and holds exactly as unenforced.
        let holds = app.holds().await;
        if !holds.is_empty() && !req.ignore_holds {
            return Err(anyhow::anyhow!(
                "{} package(s) are held, and the native whole-system upgrade cannot skip \
                 them:\n{}\n\nNothing was upgraded. Either scope the upgrade so the holds \
                 bind — `shall upgrade --backend <b>`, or name the packages — or run \
                 `shall upgrade --ignore-holds` to upgrade the whole machine anyway.",
                holds.len(),
                holds
                    .describe()
                    .iter()
                    .map(|l| format!("  {l}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        if !holds.is_empty() {
            eprintln!(
                "note: --ignore-holds — {} package hold(s) are not enforced by the native \
                 whole-system upgrade and may be bumped by it.",
                holds.len()
            );
        }
        // `apt upgrade` is a change path, so it passes the `[guard]` gate like every other
        // one. `deny_packages` is close to meaningless against "upgrade everything";
        // `require_snapshot` is not, and a gate honoured by some change paths is a gate on
        // nothing.
        let resolver = crate::app::sync::resolver::StateResolver::new(
            &app.config,
            app.registry.clone(),
            false,
        )
        .await;
        let desired = resolver.resolve_desired_state().await?;
        enforce_policy(app, &desired).await?;

        if app.config.dry_run {
            crate::would_print!(
                "would run each backend's native whole-system upgrade (e.g. `apt upgrade`)."
            );
            return Ok(None);
        }
        // **`None`, not `Some(0)`.** Each manager's own upgrade-all reports no per-package
        // count Shall can trust, and reporting zero here would make every `@after=` step skip
        // after the run that moves the most.
        return app
            .managers()
            .await
            .upgrade()
            .await
            .map(|()| None)
            .map_err(Into::into);
    }

    // Mode 4: scoped declarative upgrade (profile/module/group) via the change planner.
    //
    // Mode 3 above has already returned for every unscoped call, so this is a `Scope` and not
    // an `Option<Scope>` — said here rather than left to the reader, because a plan built from
    // `None` reaps, and "unreachable" is what the four unscoped-removal sites all were until
    // one of them was reached.
    // An error and not an early `Ok(())`: this branch reports success over an upgrade that did
    // not happen, and a silent success is the thing that hid every finding this change came
    // from. If mode 3 ever stops catching the unscoped call, that is a bug someone should be
    // told about rather than a run that quietly did nothing.
    let Some(scope) = req.scope() else {
        return Err(anyhow::anyhow!(
            "internal: the scoped upgrade was reached without a scope, which mode 3 exists to \
             prevent. Nothing was upgraded. Please report this."
        ));
    };
    let out = req.out;

    let resolver = app.resolver().await;
    let desired = resolver.resolve_desired_state().await?;
    enforce_policy(app, &desired).await?;

    let changes = {
        let state_guard = app.state.lock().await;
        let planner = crate::app::sync::planner::ChangePlanner::new(
            app.registry.clone(),
            &state_guard,
            &app.config,
        );
        planner.plan(&desired, PlanScope::Narrowed(scope)).await?
    };

    if app.config.dry_run {
        if out.is_json() {
            println!(
                "{}",
                serde_json::to_string_pretty(&changes.generate_report())?
            );
        } else {
            print_flight_plan(&app.config, &app.registry, &changes);
            println!("(dry-run: scoped upgrade previewed; nothing applied.)");
        }
        return Ok(Some(0));
    }

    if out.is_human() && !changes.is_empty() {
        print_flight_plan(&app.config, &app.registry, &changes);
    }

    // The plan is the count: this mode goes through the change planner, so what it intends to
    // move is enumerable before it runs, unlike the native path below it.
    let moved = changes.total_install();
    if !changes.is_empty() {
        app.sync_engine()
            .sync(changes, crate::app::sync::guard::GuardScope::Upgrade)
            .await?;
        perform_maintenance(app).await?;
    }
    Ok(Some(moved))
}

pub async fn handle_update(managers: &crate::app::Managers<'_>) -> Result<()> {
    managers.update().await.map_err(|e| e.into())
}

#[cfg(test)]
mod steps_scope_tests {
    use super::*;

    fn req<'a>(
        packages: &'a [String],
        backend: Option<&'a str>,
        security: bool,
        steps: Option<bool>,
    ) -> UpgradeRequest<'a> {
        const NONE: &Option<String> = &None;
        UpgradeRequest {
            packages,
            backend,
            all: false,
            security,
            except: &[],
            ignore_holds: false,
            profile: NONE,
            module: NONE,
            out: Output::Human,
            canary: false,
            test: NONE,
            steps,
        }
    }

    fn named(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// **`shall upgrade curl` is not a whole-machine upgrade**, so it does not run the
    /// `@on=upgrade` steps. Every way of narrowing a run, because the one that caused this —
    /// naming a package — was not what `scope()` was asking about, and a check written against
    /// `scope()` alone would still fire firmware for `upgrade curl`.
    #[test]
    fn every_way_of_narrowing_a_run_stops_the_steps() {
        let curl = named(&["curl"]);
        assert!(req(&curl, None, false, None).narrowed(), "a named package");
        assert!(
            req(&[], Some("apt"), false, None).narrowed(),
            "a named manager"
        );
        assert!(req(&[], None, true, None).narrowed(), "--security");
    }

    /// And a bare `shall upgrade` is the whole machine, which is what the steps belong to.
    /// The control: without it, `narrowed()` returning true unconditionally passes the test
    /// above and silently removes the feature.
    #[test]
    fn an_unscoped_upgrade_is_not_narrowed() {
        assert!(!req(&[], None, false, None).narrowed());
    }

    /// Neither direction is unreachable: `--steps` asks for them on a narrowed run, `--no-steps`
    /// declines them on a whole-machine one. A default that cannot be overridden is a rule.
    #[test]
    fn the_flags_reach_both_answers_the_default_does_not_give() {
        let curl = named(&["curl"]);

        let default_narrow = req(&curl, None, false, None);
        assert!(!default_narrow.steps.unwrap_or(!default_narrow.narrowed()));

        let asked = req(&curl, None, false, Some(true));
        assert!(
            asked.steps.unwrap_or(!asked.narrowed()),
            "`--steps` must run them on a run that named a package"
        );

        let declined = req(&[], None, false, Some(false));
        assert!(
            !declined.steps.unwrap_or(!declined.narrowed()),
            "`--no-steps` must decline them on a whole-machine run"
        );
    }
}
