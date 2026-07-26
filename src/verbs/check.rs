use crate::verbs::prelude::*;

/// `unmanaged` — **what `adopt` would adopt** (II.8), which is the definition E6 asks for.
///
/// It used to answer a different question: every installed package LiNix does not manage,
/// dependency closure and all. On a stock Ubuntu that is ~476 packages where `adopt` takes
/// ~103 — so `unmanaged` and `adopt` disagreed by a factor of four about the same word, and
/// the number you read here was not the number `adopt` would act on. Same crawl, one answer.
pub(crate) async fn handle_unmanaged(app: &App) -> Result<()> {
    let found = app.adopter().discover().await?;

    if found.adopt.is_empty() {
        println!("Nothing to adopt: LiNix already manages everything you chose to install.");
    } else {
        println!(
            "{} package(s) `linix adopt` would take:\n",
            found.adopt.len()
        );
        println!("{:<15} PACKAGE", "BACKEND");
        for p in &found.adopt {
            println!("{:<15} {}", p.backend, p.name);
        }
        println!("\nThis is an estimate — each backend's answer came from:");
        for (backend, source) in &found.sources {
            println!("  {:<10} {}", backend, source);
        }
    }

    if !found.skipped.is_empty() {
        println!(
            "\n{} package(s) the OS reports as essential are left alone.",
            found.skipped.len()
        );
    }
    Ok(())
}

/// `check` (II.8): parse everything the active profiles reach and report errors, changing
/// nothing. Resolution is where every parse/validation error surfaces — a bad line, an
/// unknown option, a `use` cycle — so a clean resolve IS a clean parse; this just says so,
/// and prints the counts a reader wants before running `sync`.
/// `linix check` — the one command that looks (U9, 7i).
///
/// With no section it runs every question and prints a line each: the verdict, and the command
/// that acts on it. With a section it prints that section's detail. It never changes anything;
/// `linix heal` is what repairs.
pub(crate) async fn handle_check(app: &App, section: Option<&str>, json: bool) -> Result<()> {
    use linix::app::check::Section;

    let Some(name) = section else {
        return check_summary(app, json).await;
    };
    let Some(section) = Section::parse(name) else {
        anyhow::bail!(
            "`{}` is not a section of `check`. Sections: {}.",
            name,
            Section::vocabulary()
        );
    };
    match section {
        Section::Config => check_config(app).await,
        Section::Drift => handle_status(app, json).await,
        Section::Unmanaged => handle_unmanaged(app).await,
        Section::Absent => handle_absent(app).await,
        Section::Conflicts => handle_conflicts(app, json).await,
        Section::Health => check_health(app, json).await,
        Section::Security => handle_audit(app, json).await,
        Section::Approvals => check_approvals(app, json).await,
    }
}

/// `check approvals` — the event hooks that will not run because they are unapproved (II.12).
///
/// Only event hooks: they warn-and-skip, so they are the one supply-chain item that fails
/// silently. The others (`exec:`, adapters, the `vars` provider, package hooks) block a sync
/// loudly, so a user meets those the moment they run `sync` — this is for the ones nobody meets
/// until the machine drifts and the hook that should have told them does nothing.
pub(crate) async fn check_approvals(app: &App, json: bool) -> Result<()> {
    let hooks = linix::app::events::EventHooks::load(&app.config);
    let unapproved = hooks.unapproved();

    if json {
        let rows: Vec<_> = unapproved
            .iter()
            .map(|h| serde_json::json!({ "event": h.event.as_str(), "origin": h.origin }))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!(rows))?
        );
        return Ok(());
    }

    if unapproved.is_empty() {
        println!("Every event hook is approved and will run.");
        return Ok(());
    }
    println!(
        "{} event hook(s) are unapproved and will NOT run until you `linix lock`:",
        unapproved.len()
    );
    for h in &unapproved {
        println!("  {} at {}", h.event, h.origin);
    }
    // A read-only command that found work exits 2 (U21), like every other `check` section.
    Err(linix::core::Error::Differences(String::new()).into())
}

/// Every section's verdict, one line each. The summary is deliberately cheap to read: a reader
/// wants to know whether anything needs them, and if so which command to run.
pub(crate) async fn check_summary(app: &App, json: bool) -> Result<()> {
    use linix::app::check::{Finding, Section};

    let mut findings: Vec<Finding> = Vec::new();

    // config — does everything the active profiles reach resolve?
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let state = match resolver.resolve_model().await {
        Ok(state) => {
            findings.push(Finding::ok(
                Section::Config,
                format!("{} package(s) declared", state.total_present()),
            ));
            Some(state)
        }
        Err(e) => {
            // A config that does not resolve makes every section below it meaningless, so say
            // so plainly and stop rather than reporting "0 drift" from a model that failed.
            findings.push(Finding::attention(
                Section::Config,
                format!("does not resolve — {}", e),
                "linix check config",
            ));
            None
        }
    };

    if let Some(state) = state.as_ref() {
        // drift — what a sync would change.
        let enabled = app.priority_backends().await;
        let changes = {
            let guard = app.state.lock().await;
            linix::app::sync::planner::ChangePlanner::new(app.registry.clone(), &guard, &app.config)
                .with_enabled(enabled)
                .plan(&state.packages, None)
                .await
        };
        match changes {
            Ok(c) if c.is_empty() => findings.push(Finding::ok(
                Section::Drift,
                "the machine matches your files",
            )),
            Ok(c) => findings.push(Finding::attention(
                Section::Drift,
                format!(
                    "{} to install, {} to remove",
                    c.total_install(),
                    c.total_remove()
                ),
                "linix sync",
            )),
            Err(e) => findings.push(Finding::attention(
                Section::Drift,
                format!("could not be planned — {}", e),
                "linix check drift",
            )),
        }

        // absent — declarations that are in force.
        let absent = state.absent().count();
        findings.push(if absent == 0 {
            Finding::ok(Section::Absent, "nothing is declared absent")
        } else {
            Finding::ok(Section::Absent, format!("{} line(s) in force", absent))
        });

        // conflicts — the same package declared two ways.
        let specs: Vec<linix::core::PackageSpec> =
            state.packages.values().flatten().cloned().collect();
        let conflicts = linix::app::conflicts::detect_conflicts(&specs);
        findings.push(if conflicts.is_empty() {
            Finding::ok(Section::Conflicts, "none")
        } else {
            Finding::attention(
                Section::Conflicts,
                format!("{} package(s) declared two ways", conflicts.len()),
                "linix check conflicts",
            )
        });
    }

    // unmanaged — what adopt would take.
    match app.adopter().discover().await {
        Ok(found) if found.adopt.is_empty() => findings.push(Finding::ok(
            Section::Unmanaged,
            "everything you chose is managed",
        )),
        Ok(found) => findings.push(Finding::attention(
            Section::Unmanaged,
            format!("{} package(s) LiNix does not manage", found.adopt.len()),
            "linix adopt",
        )),
        Err(e) => findings.push(Finding::attention(
            Section::Unmanaged,
            format!("could not be crawled — {}", e),
            "linix check unmanaged",
        )),
    }

    // health — can each backend run?
    // `critical` is deliberately not counted here: on any real machine most backends are
    // critical because that manager simply is not installed, which is the ordinary state and
    // not something a summary should report as wrong. `check health` lists them.
    let mut ok = 0usize;
    let mut degraded = 0usize;
    for b in app.registry.all() {
        if let Ok(r) = b.core().check_health().await {
            match r.status {
                linix::core::HealthStatus::Ok => ok += 1,
                linix::core::HealthStatus::Degraded => degraded += 1,
                linix::core::HealthStatus::Critical => {}
            }
        }
    }
    findings.push(if degraded == 0 {
        Finding::ok(Section::Health, format!("{} backend(s) ready", ok))
    } else {
        Finding::attention(
            Section::Health,
            format!("{} ready, {} degraded", ok, degraded),
            "linix check health",
        )
    });

    // security — anything managed with a known advisory.
    match linix::app::insight::audit(app).await {
        Ok(report) if report.findings.is_empty() => {
            findings.push(Finding::ok(Section::Security, "no known advisories"))
        }
        Ok(report) => findings.push(Finding::attention(
            Section::Security,
            format!("{} package(s) with advisories", report.findings.len()),
            "linix check security",
        )),
        // The advisory database is a network call: not reaching it is a gap in the report,
        // never a clean bill of health.
        Err(e) => findings.push(Finding::attention(
            Section::Security,
            format!("could not be checked — {}", e),
            "linix check security",
        )),
    }

    // approvals — event hooks that are unapproved and so will silently not run (II.12).
    let unapproved = linix::app::events::EventHooks::load(&app.config)
        .unapproved()
        .len();
    findings.push(if unapproved == 0 {
        Finding::ok(Section::Approvals, "every event hook will run")
    } else {
        Finding::attention(
            Section::Approvals,
            format!("{} event hook(s) will not run until approved", unapproved),
            "linix lock",
        )
    });

    if json {
        let rows: Vec<_> = findings
            .iter()
            .map(|f| {
                serde_json::json!({
                    "section": f.section.as_str(),
                    "ok": f.ok,
                    "summary": f.summary,
                    "next": f.next,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!(rows))?
        );
        return Ok(());
    }

    for f in &findings {
        println!("{}", f.line());
    }
    if findings.iter().all(|f| f.ok) {
        println!(
            "
Nothing needs you."
        );
        return Ok(());
    }
    // U21: a read-only command that looked and found work exits 2, not 0 — a script asking
    // "is this machine converged?" needs an answer it can branch on. The message is empty
    // because the findings are already on stdout; `finish` prints nothing further.
    Err(linix::core::Error::Differences(String::new()).into())
}

/// The `config` section: does every file the active profiles reach parse and resolve?
pub(crate) async fn check_config(app: &App) -> Result<()> {
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let state = resolver.resolve_model().await?;
    // `check` claims to parse everything the active profiles reach, and a `schedule:` line is
    // only validated where it is provisioned — so a missing `cron`, or a `run` a timer may not
    // run, surfaced at sync time on a file `check` had already called clean.
    for (name, opts, origin) in state.schedules() {
        linix::model::schedule::schedule_config(
            name,
            opts,
            origin,
            &app.config.guard.never_unattended,
        )?;
    }

    // II.3/II.7: resolution reads only what the active profiles reach; `check` reads
    // everything, cycles included. A module nobody activates is still a file that has to
    // hold up, and finding out otherwise on the day you activate it is the worst moment to
    // find out. Every error is listed, not just the first: they are independent files.
    let unreached = resolver.parse_everything().await?;
    if !unreached.is_empty() {
        println!("{} file(s) do not check out:\n", unreached.len());
        for e in &unreached {
            println!("  {}\n", e);
        }
        return Err(anyhow::anyhow!(
            "{} file(s) in `modules/` or `profiles/` do not check out. They are not active, \
             and they are still broken.",
            unreached.len()
        ));
    }

    println!(
        "OK: every module and profile checks out, reached or not. {} present, {} absent, {} repo/shim/service/link/schedule line(s).",
        state.total_present(),
        state.absent().count(),
        state.extras.len()
    );

    // II.15: a pattern is the one line whose meaning is not on the line. The count is.
    let patterns = state.regex_expansions();
    if !patterns.is_empty() {
        println!(
            "\n{} pattern(s), frozen in `locks/regex.toml`:",
            patterns.len()
        );
        for (pattern, count) in &patterns {
            println!("  {:<28} {} package(s)", pattern, count);
        }
        println!("  (delete an entry from the lock to match again.)");
    }

    if !state.lapsed.is_empty() {
        println!(
            "\n{} dated line(s) have lapsed and no longer count:",
            state.lapsed.len()
        );
        for (key, origin) in &state.lapsed {
            println!("  {} at {}", key, origin);
        }
    }

    // W5: a variable defined but referenced by no `when` or value anywhere is probably a
    // leftover from a block deleted on this branch. A note, never an error — an unused default
    // breaks nothing, and on a fleet the reference may still live on another branch.
    if !state.vars.is_empty() {
        let referenced = referenced_variable_names(&app.config.config_root());
        let mut unused: Vec<&String> = state
            .vars
            .keys()
            .filter(|k| !referenced.contains(*k))
            .collect();
        unused.sort();
        if !unused.is_empty() {
            println!(
                "\nNote: {} variable(s) defined but never referenced by a `when` or a value:",
                unused.len()
            );
            for name in unused {
                println!("  ${}", name);
            }
            println!(
                "  (harmless — but often the sign of a `when` block that was deleted on this branch.)"
            );
        }
    }
    Ok(())
}

/// Every variable name a `$name` references anywhere in the repo's model files — for the `check`
/// unused-variable note (W5). Read statically across all files, so a name used only in another
/// host's `when` block still counts as used and is not flagged.
pub(crate) fn referenced_variable_names(
    config_root: &std::path::Path,
) -> std::collections::HashSet<String> {
    let mut files: Vec<std::path::PathBuf> = ["active", "priority", "schedules", "vars"]
        .iter()
        .map(|n| config_root.join(n))
        .collect();
    for dir in ["modules", "profiles"] {
        if let Ok(entries) = std::fs::read_dir(config_root.join(dir)) {
            files.extend(
                entries
                    .flatten()
                    .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                    .map(|e| e.path()),
            );
        }
    }
    let mut refs = std::collections::HashSet::new();
    for f in files {
        if let Ok(body) = std::fs::read_to_string(&f) {
            refs.extend(linix::model::vars::referenced_names(&body));
        }
    }
    refs
}

/// `linix eval` — the resolved configuration, as JSON (U17).
///
/// Deliberately *only* a resolution: no lock is taken (it is in `READ_ONLY_COMMANDS`), no
/// backend is asked what is installed, nothing is written. It answers what the configuration
/// says, which is the half of `plan`'s question that does not depend on the machine — and the
/// half a script can act on.
pub(crate) async fn handle_eval(app: &App) -> Result<()> {
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let state = resolver.resolve_model().await?;
    let doc = linix::app::eval::Evaluation::of(&state, &app.config.config_root());
    print!("{}", doc.render()?);
    Ok(())
}

pub(crate) async fn handle_vars(app: &App) -> Result<()> {
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let Some(selected) = resolver.vars_provider()? else {
        println!(
            "No variable provider in this repo, so no variables.\n  \
             Create a `vars` file, or point `[vars] source` at one."
        );
        return Ok(());
    };
    let name = selected
        .path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("vars");
    let kind = match selected.kind {
        linix::model::vars_provider::Kind::LineFile => "line file",
        linix::model::vars_provider::Kind::External => "external program",
        linix::model::vars_provider::Kind::Embedded => "embedded script",
    };
    let (vars, origins) = resolver.resolve_vars_with_origins().await?;
    if vars.is_empty() {
        println!("`{}` ({}) resolved no variables.", name, kind);
        return Ok(());
    }
    println!("Variables from `{}` ({}):", name, kind);
    let width = vars.keys().map(|k| k.len()).max().unwrap_or(0);
    for (k, v) in &vars {
        let source = origins.get(k).map(short_origin).unwrap_or_default();
        println!(
            "  ${:<width$} = {}   [{}]   set at {}",
            k,
            v,
            v.type_name(),
            source,
            width = width
        );
    }
    Ok(())
}

/// An origin as `linix vars`/`why` show it: the filename and, when it is a real line rather than
/// a whole-provider attribution, the line number — `vars:6`, or `vars.linix` for a script.
pub(crate) fn short_origin(origin: &linix::config::grammar::Origin) -> String {
    let file = origin
        .file
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("vars");
    if origin.line == 0 {
        file.to_string()
    } else {
        format!("{}:{}", file, origin.line)
    }
}

pub(crate) async fn handle_absent(app: &App) -> Result<()> {
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let state = resolver.resolve_model().await?;
    let mut absent: Vec<_> = state.absent().collect();
    if absent.is_empty() {
        println!("No `absent:` lines are in force.");
        return Ok(());
    }
    absent.sort_by(|a, b| (&a.backend, &a.name).cmp(&(&b.backend, &b.name)));
    println!(
        "{} `absent:` line(s) in force — kept off this machine:\n",
        absent.len()
    );
    println!("{:<15} {:<25} SOURCE", "BACKEND", "PACKAGE");
    for spec in absent {
        let source = spec
            .options
            .get("__source")
            .map(String::as_str)
            .unwrap_or("?");
        println!("{:<15} {:<25} {}", spec.backend, spec.name, source);
    }
    Ok(())
}

pub(crate) async fn handle_conflicts(app: &App, json: bool) -> Result<()> {
    use linix::app::conflicts::{detect_conflicts, ConflictKind};

    // Resolve the full desired state (all manifests/modules/groups), flatten to specs.
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let desired = resolver.resolve_desired_state().await?;
    let specs: Vec<linix::core::PackageSpec> = desired.into_values().flatten().collect();
    let conflicts = detect_conflicts(&specs);

    if json {
        println!("{}", serde_json::to_string_pretty(&conflicts)?);
        return Ok(());
    }

    if conflicts.is_empty() {
        println!(
            "No cross-backend conflicts detected across {} desired package(s).",
            specs.len()
        );
        return Ok(());
    }

    println!("Cross-backend conflicts ({}):", conflicts.len());
    for c in &conflicts {
        let label = match c.kind {
            ConflictKind::VersionMismatch => "VERSION MISMATCH",
            ConflictKind::MultipleProviders => "MULTIPLE PROVIDERS",
        };
        let providers = c
            .providers
            .iter()
            .map(|(b, v)| match v {
                Some(v) => format!("{}@{}", b, v),
                None => b.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        println!("  [{}] {} — provided by: {}", label, c.name, providers);
    }
    println!(
        "\nResolve by removing the duplicate from one backend, or pinning both to the same \
         version. (Shadowing means whichever is first on PATH wins.)"
    );
    Ok(())
}

/// Short label for a health status (human output).
pub(crate) fn status_label(s: linix::core::HealthStatus) -> &'static str {
    use linix::core::HealthStatus::*;
    match s {
        Ok => "OK",
        Degraded => "WARN",
        Critical => "FAIL",
    }
}

/// The status label, colored for a terminal (green/yellow/red) and plain otherwise / under
/// NO_COLOR. Centralizing color here keeps the doctor output readable without a color crate.
pub(crate) fn status_label_colored(s: linix::core::HealthStatus) -> String {
    use linix::core::HealthStatus::*;
    use linix::utils::style::{color_enabled, paint, GREEN, RED, YELLOW};
    let code = match s {
        Ok => GREEN,
        Degraded => YELLOW,
        Critical => RED,
    };
    paint(color_enabled(), code, status_label(s))
}

/// Count backends by status. Pure — unit tested.
pub(crate) fn doctor_tally(
    reports: &[(String, linix::core::HealthReport)],
) -> (usize, usize, usize) {
    use linix::core::HealthStatus::*;
    let mut ok = 0;
    let mut degraded = 0;
    let mut critical = 0;
    for (_, r) in reports {
        match r.status {
            Ok => ok += 1,
            Degraded => degraded += 1,
            Critical => critical += 1,
        }
    }
    (ok, degraded, critical)
}

/// The `health` section of `check`: can each backend actually run, and is the repo intact?
///
/// Reports only. What it used to repair under `--fix` is `heal`'s now (U9).
pub(crate) async fn check_health(app: &App, json: bool) -> Result<()> {
    use linix::core::{HealthReport, HealthStatus};

    // ---- Per-backend health, via each backend's own probe (not a shallow is_available). ----
    let mut reports: Vec<(String, HealthReport)> = Vec::new();
    for b in app.registry.all() {
        let report = match b.core().check_health().await {
            Ok(r) => r,
            Err(e) => HealthReport {
                status: HealthStatus::Critical,
                message: Some(format!("health probe errored: {}", e)),
            },
        };
        reports.push((b.name().to_string(), report));
    }

    // ---- System-level checks. Reported, never repaired: that is `heal`'s job (U9). ----
    let mut system: Vec<(String, HealthStatus, Option<String>)> = Vec::new();

    for (label, dir) in [
        ("config root", app.config.config_root()),
        ("modules dir", app.config.config_root().join("modules")),
        ("profiles dir", app.config.config_root().join("profiles")),
    ] {
        if dir.exists() {
            system.push((label.into(), HealthStatus::Ok, None));
        } else {
            system.push((
                label.into(),
                HealthStatus::Degraded,
                Some(format!("missing: {} (run `linix heal`)", dir.display())),
            ));
        }
    }

    // ---- Lockfile integrity: does locks/versions.json still match the managed set? ----
    {
        let lock_path = app.config.config_root().join("locks").join("versions.json");
        if !lock_path.exists() {
            system.push((
                "lockfile".into(),
                HealthStatus::Ok,
                Some("none yet (run `linix lock` to pin versions)".into()),
            ));
        } else {
            let managed: std::collections::HashSet<String> = {
                let state = app.state.lock().await;
                state
                    .packages
                    .iter()
                    .map(|p| format!("{}:{}", p.backend, p.name))
                    .collect()
            };
            let locked_keys: std::collections::HashSet<String> =
                match tokio::fs::read_to_string(&lock_path).await {
                    Ok(data) => serde_json::from_str::<serde_json::Value>(&data)
                        .ok()
                        .and_then(|v| {
                            v.get("locks")
                                .and_then(|l| l.as_object())
                                .map(|o| o.keys().cloned().collect())
                        })
                        .unwrap_or_default(),
                    Err(_) => std::collections::HashSet::new(),
                };
            let missing = managed.difference(&locked_keys).count();
            let stale = locked_keys.difference(&managed).count();
            if missing == 0 && stale == 0 {
                system.push(("lockfile".into(), HealthStatus::Ok, None));
            } else {
                system.push((
                    "lockfile".into(),
                    HealthStatus::Degraded,
                    Some(format!(
                        "drifted: {} unpinned / {} stale (run `linix lock`, or `linix heal`)",
                        missing, stale
                    )),
                ));
            }
        }
    }

    // Git is not a dependency (X.5): its absence is reported, not treated as a fault. What is
    // unavailable without it is exactly the history-and-rollback set, and `doctor` is where
    // K8 says the standing notice lives — not on `sync`, which runs unattended.
    {
        let git = app.git_manager();
        if !linix::core::GitManager::git_available() {
            system.push((
                "git".into(),
                HealthStatus::Degraded,
                Some(
                    "not installed. LiNix runs without it; generations, `rollback` and `diff` \
                     are unavailable until it is present."
                        .into(),
                ),
            ));
        } else if !git.is_repo() {
            system.push((
                "git".into(),
                HealthStatus::Degraded,
                Some(
                    "this config is not a git repo, so there is no history to roll back to. \
                     `linix git init` here turns it on."
                        .into(),
                ),
            ));
        } else {
            system.push(("git".into(), HealthStatus::Ok, None));
        }
    }

    let (ok, degraded, critical) = doctor_tally(&reports);
    if ok == 0 {
        system.push((
            "package managers".into(),
            HealthStatus::Critical,
            Some("no usable backend detected on this host".into()),
        ));
    }

    // ---- Output ----
    if json {
        let backends: Vec<_> = reports
            .iter()
            .map(|(n, r)| serde_json::json!({ "backend": n, "status": r.status, "message": r.message }))
            .collect();
        let sys: Vec<_> = system
            .iter()
            .map(|(n, s, m)| serde_json::json!({ "check": n, "status": s, "message": m }))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "backends": backends,
                "system": sys,
                "summary": { "ok": ok, "degraded": degraded, "critical": critical },
            }))?
        );
        return Ok(());
    }

    println!(
        "Backends: {} OK, {} degraded, {} critical (of {} total).",
        ok,
        degraded,
        critical,
        reports.len()
    );
    // Readiness roster: one `[READY] <backend>` line per healthy backend, printed at column 0
    // (unindented, uncolored) so it is both human-readable AND machine-greppable —
    // `linix doctor | grep '^\[READY\]'` enumerates every usable backend on this host. Without
    // this, a healthy `doctor` printed nothing about which package managers actually work.
    for (name, r) in &reports {
        if r.status == HealthStatus::Ok {
            println!("[READY] {}", name);
        }
    }
    // Then surface only the backends that need attention — a long OK list here would be noise.
    for (name, r) in &reports {
        if r.status != HealthStatus::Ok {
            println!(
                "  [{}] {}{}",
                status_label_colored(r.status),
                name,
                r.message
                    .as_deref()
                    .map(|m| format!(" — {}", m))
                    .unwrap_or_default()
            );
        }
    }

    println!("\nSystem:");
    for (name, s, m) in &system {
        println!(
            "  [{}] {}{}",
            status_label_colored(*s),
            name,
            m.as_deref()
                .map(|m| format!(" — {}", m))
                .unwrap_or_default()
        );
    }

    let sys_critical = system.iter().any(|(_, s, _)| *s == HealthStatus::Critical);
    if critical > 0 || sys_critical {
        println!("\nSome checks are CRITICAL. Install the missing tools, or run `linix heal`.");
    } else if degraded > 0 {
        println!("\nAll critical checks pass; some backends are degraded (see WARN above).");
    } else {
        println!("\nAll checks pass. System is healthy.");
    }
    Ok(())
}

#[cfg(test)]
mod doctor_tests {
    use super::*;
    use linix::core::{HealthReport, HealthStatus};

    fn rep(status: HealthStatus) -> HealthReport {
        HealthReport {
            status,
            message: None,
        }
    }

    #[test]
    fn tally_counts_each_status() {
        let reports = vec![
            ("apt".to_string(), rep(HealthStatus::Ok)),
            ("brew".to_string(), rep(HealthStatus::Ok)),
            ("snap".to_string(), rep(HealthStatus::Degraded)),
            ("nix".to_string(), rep(HealthStatus::Critical)),
        ];
        assert_eq!(doctor_tally(&reports), (2, 1, 1));
    }

    #[test]
    fn status_labels_are_stable() {
        assert_eq!(status_label(HealthStatus::Ok), "OK");
        assert_eq!(status_label(HealthStatus::Degraded), "WARN");
        assert_eq!(status_label(HealthStatus::Critical), "FAIL");
    }
}
