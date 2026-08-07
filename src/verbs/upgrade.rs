use crate::verbs::prelude::*;

/// Everything `handle_upgrade` needs, bundled so the dispatch site stays readable and the
/// handler doesn't grow an unwieldy positional signature.
pub struct UpgradeRequest<'a> {
    pub packages: &'a [String],
    pub backend: Option<&'a str>,
    pub all: bool,
    pub security: bool,
    pub except: &'a [String],
    pub profile: &'a Option<String>,
    pub module: &'a Option<String>,
    pub json: bool,
    pub canary: bool,
    pub test: &'a Option<String>,
}

impl UpgradeRequest<'_> {
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
    app: &App,
    backend: &str,
    name: &str,
    version: Option<&str>,
) -> Result<bool> {
    let spec_str = format!("{}:{}", backend, name);
    let resolved = app.resolve_spec(&spec_str).await?;
    let mut acted = false;
    for mut spec in resolved {
        if let Some(v) = version {
            spec.options.insert("version".to_string(), v.to_string());
        }
        // II.7c: a manager this machine does not have upgrades nothing, and says so. It was a
        // bare `if let` — so `upgrade` walked past every package on an absent manager without a
        // word and reported the ones it did as the whole job.
        let Some(b) = app.registry.get(&spec.backend).filter(|b| b.is_available()) else {
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
                &app.journal,
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
) -> Result<()> {
    // Snapshot the managed set once so we can resolve names → backends without holding the lock.
    let managed: Vec<(String, String)> = {
        let state = app.state.lock().await;
        state
            .packages
            .iter()
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
            return Ok(());
        }
    }

    // Apply --backend as a filter even when explicit packages were given, and drop excludes.
    // Held packages are skipped for a broad (--backend) upgrade, but an EXPLICITLY named
    // package overrides its hold (with a warning) — naming it is a clear intent to upgrade.
    let explicit = !packages.is_empty();

    // Dry-run: describe the upgrades (after filters/holds) without touching anything.
    if app.config.dry_run {
        println!("[DRY-RUN] would upgrade:");
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
            if !explicit && app.state.lock().await.is_held(b, name) {
                continue;
            }
            println!("  ↑ {}:{}", b, name);
            n += 1;
        }
        if n == 0 {
            println!("  (nothing)");
        }
        return Ok(());
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
        if app.state.lock().await.is_held(&b, &n) {
            if explicit {
                eprintln!(
                    "upgrade: '{}:{}' is held — upgrading anyway because you named it (still held; `linix unhold` to change).",
                    b, n
                );
            } else {
                println!(
                    "upgrade: skipping held {}:{} (`linix unhold` to allow).",
                    b, n
                );
                skipped += 1;
                continue;
            }
        }
        if upgrade_one(app, &b, &n, None).await? {
            upgraded += 1;
        }
    }

    app.state.lock().await.save()?;
    println!(
        "Upgraded {} package(s){}.",
        upgraded,
        if skipped > 0 {
            format!(" ({} held back by --except)", skipped)
        } else {
            String::new()
        }
    );
    perform_maintenance(app).await
}

/// Upgrade exactly the packages `audit` reports as vulnerable, to a non-vulnerable version.
/// Honors `--except`. This is the `audit → upgrade` bridge.
pub async fn upgrade_security(app: &App, except: &[String], json: bool) -> Result<()> {
    let report = crate::app::insight::audit(app).await?;
    if report.findings.is_empty() {
        if json {
            println!("{}", serde_json::json!({ "upgraded": [], "vulnerable": 0 }));
        } else {
            println!(
                "No known vulnerabilities across {} scanned package(s) — nothing to upgrade.",
                report.scanned
            );
        }
        return Ok(());
    }

    // Aggregate advisories per package. A package can have several; to be safe from ALL of
    // them we must reach at least the HIGHEST fixed version across its advisories, so we take
    // the max `fixed` (not the first). Packages with no reported fix pin to None (→ latest).
    use version_compare::{compare, Cmp};
    let held: Vec<String> = app.state.lock().await.held.clone();
    let is_held = |backend: &str, name: &str| {
        let q = format!("{}:{}", backend, name);
        held.iter().any(|k| k == name || k == &q)
    };
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
        if is_held(&f.backend, &f.name) {
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
    if !json {
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
                 `linix unhold <pkg>` then re-run to remediate.",
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
        if !json {
            println!("[DRY-RUN] would upgrade to remediate:");
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
        return Ok(());
    }

    let mut upgraded = Vec::new();
    for (backend, name, fixed) in plan {
        // Pin to the fixed version when OSV reports one; pin-capable backends land exactly
        // there, and those that ignore the pin fall back to latest (still ≥ fixed).
        match upgrade_one(app, &backend, &name, fixed.as_deref()).await {
            Ok(true) => upgraded.push(serde_json::json!({
                "backend": backend, "name": name, "pinned_to": fixed,
            })),
            Ok(false) => {}
            // Per the agreed policy: a package we can't remediate is a warning, not a stop.
            Err(e) => eprintln!("  warning: could not upgrade {}:{}: {}", backend, name, e),
        }
    }
    app.state.lock().await.save()?;

    if json {
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
    perform_maintenance(app).await
}

/// `linix upgrade` — move packages forward, then record where they landed.
///
/// The recording is not decoration. A pin that nobody updates fights the upgrade that just ran:
/// `sync` reads the recorded version back as `@version=`, finds the installed one no longer
/// satisfies it, and plans the package straight back down. Every mode below moves versions, so
/// every mode below is followed by this (Z2). Only packages that were already pinned are
/// touched — an upgrade is not a `lock`.
pub async fn handle_upgrade(app: &App, req: UpgradeRequest<'_>) -> Result<()> {
    let json = req.json;
    upgrade_modes(app, req).await?;
    let moved = crate::verbs::plan::refresh_version_locks(app).await?;
    if moved > 0 && !json {
        println!("Lock: re-recorded {} version pin(s).", moved);
    }
    Ok(())
}

async fn upgrade_modes(app: &App, req: UpgradeRequest<'_>) -> Result<()> {
    // First, before any mode: `upgrade --backend aptt` used to scope to nothing and report
    // that everything was up to date (Q9).
    app.require_known_backend(req.backend)?;
    // And the same ruling on the form it takes positionally, which that enumeration missed:
    // `upgrade nosuchbackend:foo` answered "not a managed package — skipping" at exit 0.
    app.require_known_spec_backends(req.packages).await?;
    app.require_known_spec_backends(req.except).await?;

    // Canary keeps its own health-gated, scoped path.
    if req.canary {
        return handle_canary(app, req.scope(), req.test).await;
    }

    // Mode 1: audit-driven security upgrade.
    if req.security {
        return upgrade_security(app, req.except, req.json).await;
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
        // Native batch upgrades (`apt upgrade`, `brew upgrade`, …) run inside each manager and
        // can't be told to skip individual packages, so LiNix holds aren't enforced here. Be
        // honest about it rather than pretend the hold was respected.
        let held_count = app.state.lock().await.held.len();
        if held_count > 0 {
            eprintln!(
                "note: {} package hold(s) are NOT enforced by the native whole-system upgrade. \
                 Use `linix upgrade --backend <b>` or per-package upgrades to honor holds.",
                held_count
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
            println!(
                "[DRY-RUN] would run each backend's native whole-system upgrade (e.g. `apt upgrade`)."
            );
            return Ok(());
        }
        return app.upgrade().await.map_err(Into::into);
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
    let json = req.json;

    let resolver =
        crate::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
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
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&changes.generate_report())?
            );
        } else {
            print_flight_plan(app, &changes);
            println!("(dry-run: scoped upgrade previewed; nothing applied.)");
        }
        return Ok(());
    }

    if !json && !changes.is_empty() {
        print_flight_plan(app, &changes);
    }

    if !changes.is_empty() {
        app.sync_engine()
            .await
            .sync(changes, crate::app::sync::guard::GuardScope::Upgrade)
            .await?;
        perform_maintenance(app).await?;
    }
    Ok(())
}

pub async fn handle_update(app: &App) -> Result<()> {
    app.update().await.map_err(|e| e.into())
}
