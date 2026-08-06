use crate::verbs::prelude::*;

/// `unmanaged` — **what `adopt` would adopt** (II.8), which is the definition E6 asks for.
///
/// The wider question — every installed package nothing declares, dependency closure and all —
/// is `undeclared`, and `purge-undeclared` is what acts on it (II.11, `Q31`). One word per
/// question: while both wore this one, the two answers differed by a factor of four and the
/// number printed here was not the number the delete command would act on.
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

    // Every skip carries its own reason, and this line used to attribute all of them to the
    // one cause it happened to know about — a count explained by a reason belonging to none
    // of its inputs. Printed from the reasons themselves, as `adopt` prints them.
    if !found.skipped.is_empty() {
        println!();
        linix::app::adopt::print_left_alone(&found.skipped);
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
/// Every backend's own health probe, run concurrently, in registry order.
///
/// `check_health` is a real probe for several backends and there are ~55 of them, so asking
/// them one at a time cost the sum of every manager's answer — and the `check` rollup and the
/// `check health` detail view each did their own serial pass, so a machine paid it twice. They
/// share this one now, which is also what keeps the two views from disagreeing about the same
/// machine.
async fn probe_all_health(app: &App) -> Vec<(String, linix::core::HealthReport)> {
    use futures::stream::StreamExt;
    futures::stream::iter(app.registry.all())
        .map(|b| async move {
            let report = match b.core().check_health().await {
                Ok(r) => r,
                Err(e) => linix::core::HealthReport {
                    status: linix::core::HealthStatus::Critical,
                    message: Some(format!("health probe errored: {}", e)),
                },
            };
            (b.name().to_string(), report)
        })
        .buffered(app.config.max_parallel.max(1))
        .collect()
        .await
}

pub(crate) async fn check_summary(app: &App, json: bool) -> Result<()> {
    use linix::app::check::{Finding, Section};

    // The unmanaged section crawls every manager, so this run asks all of them whatever
    // happens; asking them together is what keeps the ones only that section wants from
    // waiting out the drift plan first (`App::warm_installed`).
    app.warm_installed().await;

    let mut findings: Vec<Finding> = Vec::new();

    // config — does everything the active profiles reach resolve?
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let state = match resolver.resolve_model().await {
        Ok(state) => {
            findings.push(
                Finding::ok(
                    Section::Config,
                    format!("{} package(s) declared", state.total_present()),
                )
                .counting([("declared", state.total_present())]),
            );
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
        let hosts = app.host_backends().await;
        let changes = {
            let guard = app.state.lock().await;
            linix::app::sync::planner::ChangePlanner::new(app.registry.clone(), &guard, &app.config)
                .plan(&state.packages, PlanScope::Whole(hosts))
                .await
        };
        // N-2: the model is packages *and* resources. Asking only the package planner is how
        // `check` came to report that the machine matched while a declared `link:` was not on
        // disk — and again after one LiNix had placed was deleted behind its back.
        let resources = app.extras().changes(state).await;
        match (changes, resources) {
            // A skip is drift the planner declined to act on, so it belongs here and not in a
            // clean bill of health: `check` reported `the machine matches your files` about a
            // machine holding a managed, undeclared, protected package (AU1). The command
            // named is the one that explains the decision, because `sync` is precisely the
            // command that will not act on it.
            (Ok(c), Ok(_)) if !c.skipped.is_empty() => findings.push(
                Finding::attention(
                    Section::Drift,
                    format!(
                        "{} package(s) installed and declared nowhere that `sync` will not \
                         remove: {}",
                        c.skipped.len(),
                        c.skipped
                            .iter()
                            .map(|s| format!("{} ({})", s.key, s.reason))
                            .collect::<Vec<_>>()
                            .join("; ")
                    ),
                    "linix protected",
                )
                .counting([
                    ("install", c.total_install()),
                    ("remove", c.total_remove()),
                    ("skipped", c.skipped.len()),
                ]),
            ),
            (Ok(c), Ok(r)) if c.is_empty() && r.is_empty() => {
                findings.push(
                    Finding::ok(
                        Section::Drift,
                        match r.unverifiable.len() {
                            0 => "the machine matches your files".to_string(),
                            // Not "it matches": LiNix looked at the packages and at every
                            // resource it can read back, and these it cannot. Saying so is the
                            // difference between a converged machine and an unexamined one.
                            n => format!(
                                "the machine matches your files, except {} resource(s) LiNix \
                                 cannot read back ({})",
                                n,
                                r.unverifiable.join(", ")
                            ),
                        },
                    )
                    // Zeroes, spelled out. A consumer that has to treat "the key is absent" and
                    // "the count is nought" as the same thing will one day be handed a real
                    // absence and call the machine converged.
                    .counting([
                        ("install", 0),
                        ("remove", 0),
                        ("skipped", 0),
                        ("unverifiable", r.unverifiable.len()),
                    ]),
                );
            }
            (Ok(c), Ok(r)) => findings.push(
                Finding::attention(
                    Section::Drift,
                    format!(
                        "{} to install, {} to remove, {}",
                        c.total_install(),
                        c.total_remove(),
                        r.summary()
                    ),
                    "linix sync",
                )
                .counting([
                    ("install", c.total_install()),
                    ("remove", c.total_remove()),
                    ("skipped", c.skipped.len()),
                    ("unverifiable", r.unverifiable.len()),
                ]),
            ),
            (Err(e), _) | (_, Err(e)) => findings.push(Finding::attention(
                Section::Drift,
                format!("could not be planned — {}", e),
                "linix check drift",
            )),
        }

        // absent — declarations that are in force.
        let absent = state.absent().count();
        findings.push(
            if absent == 0 {
                Finding::ok(Section::Absent, "nothing is declared absent")
            } else {
                Finding::ok(Section::Absent, format!("{} line(s) in force", absent))
            }
            .counting([("absent", absent)]),
        );

        // conflicts — the same package declared two ways.
        let specs: Vec<linix::core::PackageSpec> =
            state.packages.values().flatten().cloned().collect();
        let conflicts = linix::app::conflicts::detect_conflicts(&specs);
        findings.push(
            if conflicts.is_empty() {
                Finding::ok(Section::Conflicts, "none")
            } else {
                Finding::attention(
                    Section::Conflicts,
                    format!("{} package(s) declared two ways", conflicts.len()),
                    "linix check conflicts",
                )
            }
            .counting([("conflicts", conflicts.len())]),
        );
    }

    // unmanaged — what adopt would take.
    match app.adopter().discover().await {
        Ok(found) if found.adopt.is_empty() => findings.push(
            Finding::ok(Section::Unmanaged, "everything you chose is managed")
                .counting([("unmanaged", 0)]),
        ),
        Ok(found) => findings.push(
            Finding::attention(
                Section::Unmanaged,
                format!("{} package(s) `linix adopt` would take", found.adopt.len()),
                "linix adopt",
            )
            .counting([("unmanaged", found.adopt.len())]),
        ),
        Err(e) => findings.push(Finding::attention(
            Section::Unmanaged,
            format!("could not be crawled — {}", e),
            "linix check unmanaged",
        )),
    }

    // health — can each backend run?
    //
    // This rollup used to skip `critical` entirely, with a comment explaining that most
    // backends are critical on any real machine because the manager is not installed. That was
    // true and it was the wrong cure: the rollup said `25 backend(s) ready` while `check
    // health` called the same machine `23 critical`, and neither number was wrong on its own
    // terms. Now that "not installed" is `Absent` (Q2), a `critical` is a real one and the
    // rollup can report it.
    let mut ok = 0usize;
    let mut degraded = 0usize;
    let mut critical = 0usize;
    // Concurrent: `check_health` is a real probe for several backends — `psresource` asks
    // PowerShell about its cmdlets, a `generic` backend probes its binary — and there are ~55
    // of them with nothing to say to one another.
    for r in probe_all_health(app).await {
        match r.1.status {
            linix::core::HealthStatus::Ok => ok += 1,
            linix::core::HealthStatus::Degraded => degraded += 1,
            linix::core::HealthStatus::Critical => critical += 1,
            linix::core::HealthStatus::Absent => {}
        }
    }
    findings.push(
        if critical > 0 {
            Finding::attention(
                Section::Health,
                format!("{} ready, {} cannot run", ok, critical),
                "linix check health",
            )
        } else if degraded > 0 {
            Finding::attention(
                Section::Health,
                format!("{} ready, {} degraded", ok, degraded),
                "linix check health",
            )
        } else {
            Finding::ok(Section::Health, format!("{} backend(s) ready", ok))
        }
        .counting([
            ("ready", ok),
            ("degraded", degraded),
            ("critical", critical),
        ]),
    );

    // security — anything managed with a known advisory.
    match linix::app::insight::audit(app).await {
        Ok(report) if report.findings.is_empty() => findings.push(
            Finding::ok(Section::Security, "no known advisories").counting([("advisories", 0)]),
        ),
        Ok(report) => findings.push(
            Finding::attention(
                Section::Security,
                format!("{} package(s) with advisories", report.findings.len()),
                "linix check security",
            )
            .counting([("advisories", report.findings.len())]),
        ),
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
    findings.push(
        if unapproved == 0 {
            Finding::ok(Section::Approvals, "every event hook will run")
        } else {
            Finding::attention(
                Section::Approvals,
                format!("{} event hook(s) will not run until approved", unapproved),
                "linix lock",
            )
        }
        .counting([("unapproved", unapproved)]),
    );

    if json {
        let rows: Vec<_> = findings
            .iter()
            .map(|f| {
                serde_json::json!({
                    "section": f.section.as_str(),
                    "ok": f.ok,
                    "summary": f.summary,
                    "next": f.next,
                    // Always present, even when empty: a consumer that has to distinguish
                    // "no counts" from "the key is missing" writes the branch wrong once.
                    "counts": f.counts,
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
        Absent => "absent",
    }
}

/// The status label, colored for a terminal (green/yellow/red) and plain otherwise / under
/// NO_COLOR. Centralizing color here keeps the doctor output readable without a color crate.
pub(crate) fn status_label_colored(s: linix::core::HealthStatus) -> String {
    use linix::core::HealthStatus::*;
    use linix::utils::style::{color_enabled, paint, DIM, GREEN, RED, YELLOW};
    let code = match s {
        Ok => GREEN,
        Degraded => YELLOW,
        Critical => RED,
        // Not a colour a reader scans for. A manager nobody installed is information, not an
        // alarm, and painting it red is what made 23 of them shout on a healthy machine.
        Absent => DIM,
    };
    paint(color_enabled(), code, status_label(s))
}

/// Count backends by status. Pure — unit tested.
pub(crate) fn doctor_tally(
    reports: &[(String, linix::core::HealthReport)],
) -> (usize, usize, usize, usize) {
    use linix::core::HealthStatus::*;
    let mut ok = 0;
    let mut degraded = 0;
    let mut critical = 0;
    let mut absent = 0;
    for (_, r) in reports {
        match r.status {
            Ok => ok += 1,
            Degraded => degraded += 1,
            Critical => critical += 1,
            Absent => absent += 1,
        }
    }
    (ok, degraded, critical, absent)
}

/// The `health` section of `check`: can each backend actually run, and is the repo intact?
///
/// Reports only. What it used to repair under `--fix` is `heal`'s now (U9).
pub(crate) async fn check_health(app: &App, json: bool) -> Result<()> {
    use linix::core::{HealthReport, HealthStatus};

    // ---- Per-backend health, via each backend's own probe (not a shallow is_available). ----
    // See `probe_all_health` for why it is concurrent.
    // Absent means "not installed, and nothing asked for it" — so a manager listed in
    // `priority` is not absent, it is broken. The user named it; LiNix cannot use it. That
    // second half is what keeps Q2 from being a way to hide real failures: the state depends
    // on whether the machine was asked for the manager, not only on whether it is there.
    let wanted: std::collections::HashSet<String> =
        app.priority_backends().await.into_iter().collect();
    let mut reports: Vec<(String, HealthReport)> = probe_all_health(app).await;
    for (name, report) in reports.iter_mut() {
        // A set, not a scan: this ran `wanted.iter().any(...)` once per backend, inside the
        // loop over every backend.
        if report.status == HealthStatus::Absent && wanted.contains(name) {
            report.status = HealthStatus::Critical;
            report.message = Some(format!(
                "{} — and `priority` lists it, so LiNix was told to use it",
                report.message.as_deref().unwrap_or("it cannot run")
            ));
        }
    }

    // A backend that says it is healthy and cannot answer its cheapest real question is
    // lying, whatever the reason. `psresource` claimed `[READY]` for months on the strength of
    // PowerShell existing, and every operation then died on a cmdlet that was never there —
    // a probe can only be as good as the question it asks, and this asks the backend to do its
    // job instead. It costs one `list` per healthy backend and it is the check that would have
    // caught psresource without anyone having to think about PowerShell.
    {
        use futures::stream::{self, StreamExt};
        let healthy: Vec<String> = reports
            .iter()
            .filter(|(_, r)| r.status == HealthStatus::Ok)
            .map(|(n, _)| n.clone())
            .collect();
        let probed: Vec<(String, Option<String>)> = stream::iter(healthy)
            .map(|name| {
                let registry = app.registry.clone();
                async move {
                    let Some(b) = registry.get(&name) else {
                        return (name, None);
                    };
                    let Some(q) = b.as_queryable() else {
                        return (name, None); // nothing to ask; not a claim it failed
                    };
                    // Bounded, because `check` is a read-only command and a wedged manager
                    // must not hold the whole report open.
                    // 60s, and the number is evidence rather than taste: `list` measured
                    // 2-7s per backend on this machine, and an earlier 20s cap with eight in
                    // flight timed out scoop and winget — which take 1.2s each on their own.
                    // A limit tight enough to fail on contention manufactures the defect it
                    // claims to find.
                    let answer = tokio::time::timeout(
                        std::time::Duration::from_secs(60),
                        q.list_installed(),
                    )
                    .await;
                    let complaint = match answer {
                        Ok(Ok(_)) => None,
                        Ok(Err(e)) => Some(format!("says it is ready but cannot list: {}", e)),
                        Err(_) => Some("says it is ready but `list` did not answer in 60s".into()),
                    };
                    (name, complaint)
                }
            })
            // The knob, not a number. The 60s timeout above is chosen and defended; this cap
            // was neither, and a machine told to run twenty at once was running four.
            .buffer_unordered(app.config.max_parallel.max(1))
            .collect()
            .await;

        // An index rather than a scan per complaint: the outer loop is over backends and so
        // was the inner one.
        let at: std::collections::HashMap<&str, usize> = reports
            .iter()
            .enumerate()
            .map(|(i, (n, _))| (n.as_str(), i))
            .collect();
        let updates: Vec<(usize, String)> = probed
            .into_iter()
            .filter_map(|(name, complaint)| Some((*at.get(name.as_str())?, complaint?)))
            .collect();
        for (i, complaint) in updates {
            reports[i].1.status = HealthStatus::Critical;
            reports[i].1.message = Some(complaint);
        }
    }

    // And the other way a backend can be here, answer `list`, and install nothing: the setup
    // it needs was never done (Q11). `opam` passes every probe above with no switch and then
    // fails every install with `No switch is currently set` — READY, and unable to do the one
    // thing it is for. Degraded rather than Critical because reads genuinely work and the fix
    // is one command, which the message carries.
    //
    // Manager-level rows only. `asdf`'s prerequisite is a plugin per declared tool, which is a
    // question about a line rather than about the machine, and `check health` has no lines.
    {
        let rows = app.prereqs().rows();
        let os = std::env::consts::OS;
        for (name, report) in reports.iter_mut() {
            if report.status != HealthStatus::Ok {
                continue;
            }
            for row in linix::model::prereq::for_manager(&rows, name, os) {
                if row.is_per_package() {
                    continue;
                }
                let cmd = row.probe_command("");
                let Some((program, args)) = cmd.split_first() else {
                    continue;
                };
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                if app.executor.run(program, &refs, false).await.is_ok() {
                    continue;
                }
                report.status = HealthStatus::Degraded;
                report.message = Some(format!(
                    "installed, but it needs {} before it can install anything — `{}`",
                    row.missing_line(""),
                    row.command_line("")
                ));
            }
        }
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

    let (ok, degraded, critical, absent) = doctor_tally(&reports);
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
                "summary": { "ok": ok, "degraded": degraded, "critical": critical, "absent": absent },
            }))?
        );
        return Ok(());
    }

    println!(
        "Backends: {} OK, {} degraded, {} critical, {} not installed (of {} total).",
        ok,
        degraded,
        critical,
        absent,
        reports.len()
    );
    // Readiness roster: one `[READY] <backend>` line per healthy backend, printed at column 0
    // (unindented, uncolored) so it is both human-readable AND machine-greppable —
    // `linix check health | grep '^\[READY\]'` enumerates every usable backend on this host. Without
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
            ("pacman".to_string(), rep(HealthStatus::Absent)),
            ("dnf".to_string(), rep(HealthStatus::Absent)),
        ];
        assert_eq!(doctor_tally(&reports), (2, 1, 1, 2));
    }

    /// The whole point of Q2: a manager nobody installed is not a fault, so it cannot be
    /// counted as one. Twenty-three of these on an ordinary Windows box read as `23 critical`.
    #[test]
    fn a_machine_with_nothing_wrong_has_no_criticals() {
        let reports: Vec<_> = ["apt", "brew", "pacman", "dnf", "zypper"]
            .iter()
            .map(|n| (n.to_string(), rep(HealthStatus::Absent)))
            .chain(std::iter::once((
                "scoop".to_string(),
                rep(HealthStatus::Ok),
            )))
            .collect();
        let (ok, degraded, critical, absent) = doctor_tally(&reports);
        assert_eq!((ok, degraded, critical), (1, 0, 0));
        assert_eq!(absent, 5);
    }

    #[test]
    fn status_labels_are_stable() {
        assert_eq!(status_label(HealthStatus::Ok), "OK");
        assert_eq!(status_label(HealthStatus::Degraded), "WARN");
        assert_eq!(status_label(HealthStatus::Critical), "FAIL");
        assert_eq!(status_label(HealthStatus::Absent), "absent");
    }

    /// E18/E19: one condition had two message families and the busier one named the backend
    /// rather than the program it probed, so `lvm` told you to install `lvm` while looking for
    /// `lvs`. Both implementations are now one function, and it is told what was probed.
    #[test]
    fn a_missing_program_is_named_by_the_program_that_was_probed() {
        use linix::core::missing_program;

        let r = missing_program("lvm", &["lvs".to_string()]);
        assert_eq!(r.status, HealthStatus::Absent);
        let m = r.message.unwrap();
        assert!(m.contains("`lvs`"), "{m}");
        assert!(!m.contains("Binary for"), "the old message survived: {m}");

        // An absolute path is not "not on PATH" (U16).
        let m = missing_program("vendor", &["/opt/vendor/thing".to_string()])
            .message
            .unwrap();
        assert!(m.contains("does not exist or is not executable"), "{m}");

        // Two programs, and no claim about how many of them are needed.
        let m = missing_program("krew", &["kubectl".into(), "kubectl-krew".into()])
            .message
            .unwrap();
        assert!(
            m.contains("`kubectl`") && m.contains("`kubectl-krew`"),
            "{m}"
        );

        // A backend that probes nothing must not be described as missing a program.
        let m = missing_program("appimage", &[]).message.unwrap();
        assert!(!m.contains('`') || m.contains("`appimage`"), "{m}");
    }
}
