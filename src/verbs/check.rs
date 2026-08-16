use crate::verbs::inventory::handle_audit;
use crate::verbs::plan::handle_status;
use crate::verbs::prelude::*;

/// `unmanaged` — **what `adopt` would adopt** (II.8), which is the definition E6 asks for.
///
/// The wider question — every installed package nothing declares, dependency closure and all —
/// is `undeclared`, and `purge-undeclared` is what acts on it (II.11, `Q31`). One word per
/// question: while both wore this one, the two answers differed by a factor of four and the
/// number printed here was not the number the delete command would act on.
pub async fn handle_unmanaged(adopter: &crate::app::Adopter) -> Result<()> {
    let found = adopter.discover().await?;

    if found.adopt.is_empty() {
        println!("Nothing to adopt: Shall already manages everything you chose to install.");
    } else {
        println!(
            "{} package(s) `shall adopt` would take:\n",
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
        crate::app::adopt::print_left_alone(&found.skipped);
    }
    Ok(())
}

/// `check` (II.8): parse everything the active profiles reach and report errors, changing
/// nothing. Resolution is where every parse/validation error surfaces — a bad line, an
/// unknown option, a `use` cycle — so a clean resolve IS a clean parse; this just says so,
/// and prints the counts a reader wants before running `sync`.
/// `shall check` — the one command that looks (U9, 7i).
///
/// With no section it runs every question and prints a line each: the verdict, and the command
/// that acts on it. With a section it prints that section's detail. It never changes anything;
/// `shall heal` is what repairs.
pub async fn handle_check(app: &App, section: Option<&str>, out: Output) -> Result<()> {
    use crate::app::check::Section;

    let Some(name) = section else {
        return check_summary(app, out).await;
    };
    let Some(section) = Section::parse(name) else {
        anyhow::bail!(
            "`{}` is not a section of `check`. Sections: {}.",
            name,
            Section::vocabulary()
        );
    };
    match section {
        Section::Config => check_config(&app.config, &app.registry).await,
        Section::Drift => handle_status(app, out).await,
        Section::Unmanaged => handle_unmanaged(&app.adopter().await).await,
        Section::Absent => handle_absent(&app.config, &app.registry).await,
        Section::Conflicts => handle_conflicts(&app.config, &app.registry, out).await,
        Section::Health => check_health(app, out).await,
        Section::Security => handle_audit(&app.config, &app.registry, &app.state, out).await,
        Section::Approvals => check_approvals(&app.config, out).await,
        Section::Adapters => check_adapters(&app.config, out).await,
    }
}

/// How many extension surfaces this machine has written and Shall is not using.
///
/// `Absent` is not one of them: not extending Shall is the ordinary case, and a check that
/// reported it as work would be a check every machine fails on its first run.
fn adapters_not_in_use(config: &Config) -> usize {
    crate::app::adapters::survey(&config.layout())
        .iter()
        .filter(|e| e.standing.is_wrong())
        .count()
}

/// `check adapters` — the extension files a reader could not use.
///
/// **The exit code is the whole point.** A malformed `adapters/*.toml` warns and is skipped
/// mid-`sync` (ruled: a typo in an optional file must not stop you installing a package), and a
/// warning inside a sync is a warning nobody sees twice. This is where the same fact is a
/// non-zero exit — free to be loud, because looking changes nothing.
pub async fn check_adapters(config: &Config, out: Output) -> Result<()> {
    crate::verbs::setup::handle_adapters(config, None, out).await?;
    if adapters_not_in_use(config) == 0 {
        return Ok(());
    }
    // U21's exit 2, as every other section uses it: a read-only command that looked and found
    // work. The listing is already on stdout.
    Err(crate::core::Error::Differences(String::new()).into())
}

/// `check approvals` — the event hooks that will not run because they are unapproved (II.12).
///
/// Only event hooks. `exec:`, the `vars` provider and package hooks block a sync loudly, so a
/// user meets those the moment they run `sync`; this is for the ones nobody meets until the
/// machine drifts and the hook that should have told them does nothing.
///
/// An unapproved *adapter* warns and skips like a hook does — the sentence here claimed
/// otherwise — and it has its own section, `check adapters`, because it fails the same way.
pub async fn check_approvals(config: &Config, out: Output) -> Result<()> {
    let hooks = crate::app::events::EventHooks::load(config);
    let unapproved = hooks.unapproved();

    if out.is_json() {
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
        "{} event hook(s) are unapproved and will NOT run until you `shall lock`:",
        unapproved.len()
    );
    for h in &unapproved {
        println!("  {} at {}", h.event, h.origin);
    }
    // A read-only command that found work exits 2 (U21), like every other `check` section.
    Err(crate::core::Error::Differences(String::new()).into())
}

/// Every section's verdict, one line each. The summary is deliberately cheap to read: a reader
/// wants to know whether anything needs them, and if so which command to run.
/// Every backend's own health probe, run concurrently, in registry order, **with Q2's promotion
/// already applied**.
///
/// `check_health` is a real probe for several backends and there are ~55 of them, so asking
/// them one at a time cost the sum of every manager's answer — and the `check` rollup and the
/// `check health` detail view each did their own serial pass, so a machine paid it twice. They
/// share this one now.
///
/// **Sharing the probe is not sharing the verdict, and the difference is a shipped bug.** The
/// first cure taught the rollup to count `Critical` and left the `Absent`→`Critical` promotion
/// inside the detail view, so the rollup still could not see the failures that only exist after
/// it: `check` printed `Nothing needs you` and exited 0 while `check health`, on the same machine
/// in the same second, reported `8 critical`. The promotion lives here now because this is where
/// the two views actually meet, and a second copy of it in a caller is how it came back the
/// first time.
async fn probe_all_health(app: &App) -> Vec<(String, crate::core::HealthReport)> {
    use crate::core::HealthStatus;
    use futures::stream::StreamExt;
    let backends = &app.backends().await;
    let config = &app.config;
    // **Every backend this build knows, installed or not** — the one question where that is
    // right. A manager that is absent is a report; a manager that is absent *and* named by
    // `priority` is a failure, and neither is visible from the set Shall may use.
    let mut reports: Vec<(String, crate::core::HealthReport)> =
        futures::stream::iter(backends.registered())
            .map(|b| async move {
                let report = match b.core().check_health().await {
                    Ok(r) => r,
                    Err(e) => crate::core::HealthReport {
                        status: HealthStatus::Critical,
                        message: Some(format!("health probe errored: {}", e)),
                    },
                };
                (b.name().to_string(), report)
            })
            .buffered(config.max_parallel.max(1))
            .collect()
            .await;

    // Absent means "not installed, and nothing asked for it" — so a manager listed in
    // `priority` is not absent, it is broken. The user named it; Shall cannot use it. That
    // second half is what keeps Q2 from being a way to hide real failures: the state depends
    // on whether the machine was asked for the manager, not only on whether it is there.
    //
    // Unwrapped to empty on purpose, and this is the one place that is right: a `priority`
    // that will not resolve means Shall was told to use nothing it can name, so no absent
    // manager can be *promoted* to broken — and the machine can still be reported on. The
    // config section reports the unreadable file itself.
    let wanted: std::collections::HashSet<String> =
        backends.names().unwrap_or_default().into_iter().collect();
    for (name, report) in reports.iter_mut() {
        // A set, not a scan: this ran `wanted.iter().any(...)` once per backend, inside the
        // loop over every backend.
        if report.status == HealthStatus::Absent && wanted.contains(name) {
            report.status = HealthStatus::Critical;
            report.message = Some(format!(
                "{} — and `priority` lists it, so Shall was told to use it",
                report.message.as_deref().unwrap_or("it cannot run")
            ));
        }
    }

    // **The second promotion, and it lives here for the same reason the first one does.** A
    // backend that says it is healthy and cannot answer its cheapest real question is lying,
    // whatever the reason: `psresource` claimed `[READY]` for months on the strength of
    // PowerShell existing, and every operation then died on a cmdlet that was never there. A
    // probe can only be as good as the question it asks, and this asks the backend to do its
    // job instead.
    //
    // It used to run in the `check health` caller alone, which is how the divergence this
    // function exists to close came back one promotion later: `check` reported `24 ready, 1
    // cannot run` while `check health`, on the same machine in the same second, reported 7 —
    // six backends that answer `check_health` and cannot list. The rollup was not counting
    // differently, it was counting a verdict nobody had finished forming. **Any promotion a
    // caller applies after this returns is a second copy of the verdict, and both times that
    // has happened the two views have disagreed in public.**
    //
    // It costs one `list` per healthy backend. That is the price of the sentence "N ready"
    // being true, and `check` already pays far more than this elsewhere in the same run.
    let healthy: Vec<String> = reports
        .iter()
        .filter(|(_, r)| r.status == HealthStatus::Ok)
        .map(|(n, _)| n.clone())
        .collect();
    let probed: Vec<(String, Option<String>)> = futures::stream::iter(healthy)
        .map(|name| {
            let backend = backends.get(&name);
            async move {
                let Some(q) = backend.as_ref().and_then(|b| b.as_queryable()) else {
                    return (name, None); // nothing to ask; not a claim it failed
                };
                // Bounded, because `check` is a read-only command and a wedged manager must
                // not hold the whole report open. 60s, and the number is evidence rather than
                // taste: `list` measured 2-7s per backend on this machine, and an earlier 20s
                // cap with eight in flight timed out scoop and winget — which take 1.2s each
                // on their own. A limit tight enough to fail on contention manufactures the
                // defect it claims to find.
                let answer =
                    tokio::time::timeout(std::time::Duration::from_secs(60), q.list_installed())
                        .await;
                let complaint = match answer {
                    Ok(Ok(_)) => None,
                    Ok(Err(e)) => Some(format!("says it is ready but cannot list: {}", e)),
                    Err(_) => Some("says it is ready but `list` did not answer in 60s".into()),
                };
                (name, complaint)
            }
        })
        .buffer_unordered(config.max_parallel.max(1))
        .collect()
        .await;

    // An index rather than a scan per complaint: the outer loop is over backends and so was
    // the inner one.
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

    // **The third promotion, and it was still in the caller when the first two were moved.** A
    // backend can pass every probe above, answer `list`, and install nothing, because the setup
    // it needs was never done (Q11): `opam` reports READY with no switch and then fails every
    // install with `No switch is currently set`. Degraded rather than Critical because reads
    // genuinely work and the fix is one command, which the message carries.
    //
    // It demotes `Ok`, not `Absent`, so it moves the *ready* count rather than the *critical*
    // one — which is why the tally test did not catch it while it sat in `check health` alone,
    // and why it was left behind twice. The rollup was reporting those backends as ready in the
    // same breath as the detail view called them degraded. Same divergence, different column.
    //
    // Manager-level rows only. `asdf`'s prerequisite is a plugin per declared tool, which is a
    // question about a line rather than about the machine, and `check health` has no lines.
    let rows = app.prereqs().rows();
    let os = std::env::consts::OS;
    for (name, report) in reports.iter_mut() {
        if report.status != HealthStatus::Ok {
            continue;
        }
        for row in crate::model::prereq::for_manager(&rows, name, os) {
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

    reports
}

pub async fn check_summary(app: &App, out: Output) -> Result<()> {
    use crate::app::check::{Finding, Section};

    // The unmanaged section crawls every manager, so this run asks all of them whatever
    // happens; asking them together is what keeps the ones only that section wants from
    // waiting out the drift plan first (`App::warm_installed`).
    app.inventory().await.warm_installed().await;

    let mut findings: Vec<Finding> = Vec::new();

    // config — does everything the active profiles reach resolve?
    let resolver =
        crate::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let state = match resolver.resolve_model().await {
        Ok(state) => {
            // **Resources counted beside packages, because a config can declare none of the
            // one and plenty of the other.** A dotfiles-only manifest read `0 package(s)
            // declared` — true, and the sentence a user checks to see that their file was
            // understood at all. The drift row below has reported `place`/`undo` since round 3
            // and `unverifiable` since `F12`, so the *work* was visible; what was not was that
            // anything had been declared to do it to.
            let packages = state.total_present();
            let resources = state.extras.len();
            findings.push(
                Finding::ok(
                    Section::Config,
                    match resources {
                        0 => format!("{} package(s) declared", packages),
                        n => format!("{} package(s) and {} resource(s) declared", packages, n),
                    },
                )
                .counting([("declared", packages), ("resources", resources)]),
            );
            Some(state)
        }
        Err(e) => {
            // A config that does not resolve makes every section below it meaningless, so say
            // so plainly and stop rather than reporting "0 drift" from a model that failed.
            findings.push(Finding::attention(
                Section::Config,
                format!("does not resolve — {}", e),
                "shall check config",
            ));
            None
        }
    };

    if let Some(state) = state.as_ref() {
        // drift — what a sync would change.
        let hosts = app.resolver().await.host_backends().await;
        let changes = {
            let guard = app.state.lock().await;
            crate::app::sync::planner::ChangePlanner::new(app.registry.clone(), &guard, &app.config)
                .plan(&state.packages, PlanScope::Whole(hosts))
                .await
        };
        // N-2: the model is packages *and* resources. Asking only the package planner is how
        // `check` came to report that the machine matched while a declared `link:` was not on
        // disk — and again after one Shall had placed was deleted behind its back.
        let resources = app.extras().changes(state).await;
        match (changes, resources) {
            // **One row, built by appending what is true.**
            //
            // These used to be three alternative arms elected by a `match`, and the skip arm
            // came first — so a machine with one skipped declaration and any amount of real
            // pending work reported the skip and nothing else. A declared `link:` missing from
            // disk vanished from the human line *and* from the JSON, because `place`/`undo` had
            // no key in `counts` and lived only inside the summary sentence that arm replaced.
            //
            // A summary assembled by appending cannot lose a fact by gaining one. That is the
            // rule this arm now enforces, and the reason every quantity the prose can mention
            // has a key beside it.
            (Ok(c), Ok(r)) => {
                // Zeroes, spelled out, on every arm. A consumer that has to treat "the key is
                // absent" and "the count is nought" as the same thing will one day be handed a
                // real absence and call the machine converged. `place` and `undo` are here
                // because a number that exists only inside an English sentence is not
                // machine-readable however the document is encoded.
                use crate::app::sync::planner::SkipKind;
                let of_kind = |k: SkipKind| c.skipped.iter().filter(|s| s.kind == k).count();
                let counts = [
                    ("install", c.total_install()),
                    ("remove", c.total_remove()),
                    ("place", r.place.len()),
                    ("undo", r.undo.len()),
                    ("skipped", c.skipped.len()),
                    // The two kinds separately, because they are opposite facts and a consumer
                    // summing them is reading one number over two questions.
                    ("skipped_removals", of_kind(SkipKind::RemovalDeclined)),
                    ("skipped_installs", of_kind(SkipKind::InstallSkipped)),
                    ("unverifiable", r.unverifiable.len()),
                ];

                let mut clauses: Vec<String> = Vec::new();
                if !c.is_empty() || !r.is_empty() {
                    clauses.push(format!(
                        "{} to install, {} to remove, {}",
                        c.total_install(),
                        c.total_remove(),
                        r.summary()
                    ));
                }
                // A skip is drift the planner declined to act on, so it belongs here and not in
                // a clean bill of health: `check` reported `the machine matches your files`
                // about a machine holding a managed, undeclared, protected package (AU1).
                //
                // One clause per *kind*, for the same reason: a declined removal and a
                // declaration this machine cannot act on are opposites, and this row used to
                // call both of them "installed and declared nowhere".
                for (kind, rows) in crate::app::sync::planner::Skipped::by_kind(&c.skipped) {
                    clauses.push(format!(
                        "{}: {}",
                        kind.heading(rows.len()),
                        rows.iter()
                            .map(|s| format!("{} ({})", s.key, s.reason))
                            .collect::<Vec<_>>()
                            .join("; ")
                    ));
                }
                // **`ok` is the word that made this invisible.** This clause has always been
                // scrupulously honest — Shall says it cannot read these back, and names them —
                // and it was filed under the one marker that means "nothing to see", which is
                // also the marker that decides the exit code. A dotfile that no program could
                // open printed as a green row and exited 0, repeatedly (B0b).
                //
                // Absence and unavailability are different answers and only one of them is
                // knowable: that rule is the reason this codebase distinguishes them at all,
                // and reporting the unknowable one under the marker for the good answer throws
                // the distinction away at the last step.
                if !r.unverifiable.is_empty() {
                    let lead = if clauses.is_empty() {
                        "the packages and every resource Shall can read back match your files; "
                    } else {
                        ""
                    };
                    clauses.push(format!(
                        "{}{} resource(s) it cannot read back ({})",
                        lead,
                        r.unverifiable.len(),
                        r.unverifiable.join(", ")
                    ));
                }

                let finding = if clauses.is_empty() {
                    Finding::ok(Section::Drift, "the machine matches your files")
                } else {
                    // `sync` is the command for pending work; `shall protected` only when the
                    // *only* thing to report is a decision `sync` will not act on, because
                    // sending a user to the guard to explain an install they are waiting for
                    // answers a question they did not ask.
                    let advice = if c.is_empty() && r.is_empty() && !c.skipped.is_empty() {
                        "shall protected"
                    } else {
                        "shall sync"
                    };
                    Finding::attention(Section::Drift, clauses.join("; "), advice)
                };
                findings.push(finding.counting(counts));
            }
            (Err(e), _) | (_, Err(e)) => findings.push(Finding::attention(
                Section::Drift,
                format!("could not be planned — {}", e),
                "shall check drift",
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
        let specs: Vec<crate::core::PackageSpec> =
            state.packages.values().flatten().cloned().collect();
        let conflicts = crate::app::conflicts::detect_conflicts(&specs);
        findings.push(
            if conflicts.is_empty() {
                Finding::ok(Section::Conflicts, "none")
            } else {
                Finding::attention(
                    Section::Conflicts,
                    format!("{} package(s) declared two ways", conflicts.len()),
                    "shall check conflicts",
                )
            }
            .counting([("conflicts", conflicts.len())]),
        );
    }

    // unmanaged — what adopt would take.
    match app.adopter().await.discover().await {
        Ok(found) if found.adopt.is_empty() => findings.push(
            Finding::ok(Section::Unmanaged, "everything you chose is managed")
                .counting([("unmanaged", 0)]),
        ),
        Ok(found) => findings.push(
            Finding::attention(
                Section::Unmanaged,
                format!("{} package(s) `shall adopt` would take", found.adopt.len()),
                "shall adopt",
            )
            .counting([("unmanaged", found.adopt.len())]),
        ),
        Err(e) => findings.push(Finding::attention(
            Section::Unmanaged,
            format!("could not be crawled — {}", e),
            "shall check unmanaged",
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
    // Concurrent: `check_health` is a real probe for several backends — `psresource` asks
    // PowerShell about its cmdlets, a `generic` backend probes its binary — and there are ~55
    // of them with nothing to say to one another.
    //
    // Counted by `doctor_tally`, the same function `check health` counts with. Two tallies over
    // one probe is the shape that produced the divergence in the first place: this arm used to
    // carry its own `match`, whose `Absent => {}` discarded exactly the reports the detail view
    // had promoted to `Critical`.
    let (ok, degraded, critical, _absent) = doctor_tally(&probe_all_health(app).await);
    findings.push(
        if critical > 0 {
            Finding::attention(
                Section::Health,
                format!("{} ready, {} cannot run", ok, critical),
                "shall check health",
            )
        } else if degraded > 0 {
            Finding::attention(
                Section::Health,
                format!("{} ready, {} degraded", ok, degraded),
                "shall check health",
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
    match crate::app::insight::audit(&app.config, &app.registry, &app.state).await {
        Ok(report) if report.findings.is_empty() => findings.push(
            Finding::ok(Section::Security, "no known advisories").counting([("advisories", 0)]),
        ),
        Ok(report) => findings.push(
            Finding::attention(
                Section::Security,
                format!("{} package(s) with advisories", report.findings.len()),
                "shall check security",
            )
            .counting([("advisories", report.findings.len())]),
        ),
        // The advisory database is a network call: not reaching it is a gap in the report,
        // never a clean bill of health.
        Err(e) => findings.push(Finding::attention(
            Section::Security,
            format!("could not be checked — {}", e),
            "shall check security",
        )),
    }

    // approvals — event hooks that are unapproved and so will silently not run (II.12).
    let unapproved = crate::app::events::EventHooks::load(&app.config)
        .unapproved()
        .len();
    findings.push(
        if unapproved == 0 {
            Finding::ok(Section::Approvals, "every event hook will run")
        } else {
            Finding::attention(
                Section::Approvals,
                format!("{} event hook(s) will not run until approved", unapproved),
                "shall lock",
            )
        }
        .counting([("unapproved", unapproved)]),
    );

    // adapters — extension files that are written and inert. The readers warn and carry on, so
    // this is the surface where that fact is not a line in a log nobody re-reads.
    let inert = adapters_not_in_use(&app.config);
    findings.push(
        if inert == 0 {
            Finding::ok(Section::Adapters, "nothing written that Shall cannot use")
        } else {
            Finding::attention(
                Section::Adapters,
                format!("{} extension file(s) written but not in use", inert),
                "shall adapters",
            )
        }
        .counting([("not_in_use", inert)]),
    );

    if out.is_json() {
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
    Err(crate::core::Error::Differences(String::new()).into())
}

/// The `config` section: does every file the active profiles reach parse and resolve?
pub async fn check_config(config: &Config, registry: &Arc<BackendRegistry>) -> Result<()> {
    let resolver =
        crate::app::sync::resolver::StateResolver::new(config, registry.clone(), false).await;
    let state = resolver.resolve_model().await?;
    // `check` claims to parse everything the active profiles reach, and a `schedule:` line is
    // only validated where it is provisioned — so a missing `cron`, or a `run` a timer may not
    // run, surfaced at sync time on a file `check` had already called clean.
    for (name, opts, origin) in state.schedules() {
        crate::model::schedule::schedule_config(
            name,
            opts,
            origin,
            &config.guard.never_unattended,
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

    // `preferences.toml` is the other half of "does this configuration hold up", and nothing was
    // reading it here. A `[lock] freeze` that will not parse is deliberately not fatal — it
    // narrows a default, and a typo in it must not stop a machine approving a script — so the
    // only way a user learns of it is a warning on a `lock` run they may not make for weeks.
    // This is the command whose job is to find that before then.
    if let Err(e) = crate::core::lock_kind::LockSelection::parse(
        &config.lock.freeze.join(","),
        &config.lock.except,
    ) {
        println!(
            "\n`[lock]` in preferences.toml does not parse, so a bare `shall lock` freezes \
             everything:\n  {}",
            e
        );
    }

    // `H8`: the shipped steps this machine can name, because a catalogue you cannot list is one
    // you read the source for. Only the ones actually usable here — the row's OS matches and its
    // tool is on `PATH` — so this answers *what can I write*, not *what exists somewhere*. A
    // machine with none of them says nothing rather than printing an empty heading.
    let steps: Vec<crate::model::step::Step> = crate::model::step::rows_here()
        .into_iter()
        .filter(|s| crate::core::launch::program_exists(&s.detect))
        .collect();
    if !steps.is_empty() {
        println!("\n{} upgrade step(s) available here:", steps.len());
        for step in &steps {
            println!("  exec:step/{:<12} {}", step.name, step.what);
        }
        println!("  (no `shall lock` needed — these ship with Shall.)");
    }

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
        let referenced = referenced_variable_names(&config.config_root());
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
pub fn referenced_variable_names(
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
            refs.extend(crate::model::vars::referenced_names(&body));
        }
    }
    refs
}

/// `shall eval` — the resolved configuration, as JSON (U17).
///
/// Deliberately *only* a resolution: no lock is taken (it is in `READ_ONLY_COMMANDS`), no
/// backend is asked what is installed, nothing is written. It answers what the configuration
/// says, which is the half of `plan`'s question that does not depend on the machine — and the
/// half a script can act on.
pub async fn handle_eval(config: &Config, registry: &Arc<BackendRegistry>) -> Result<()> {
    let resolver =
        crate::app::sync::resolver::StateResolver::new(config, registry.clone(), false).await;
    let state = resolver.resolve_model().await?;
    let doc = crate::app::eval::Evaluation::of(&state, &config.config_root());
    print!("{}", doc.render()?);
    Ok(())
}

pub async fn handle_vars(config: &Config, registry: &Arc<BackendRegistry>) -> Result<()> {
    let resolver =
        crate::app::sync::resolver::StateResolver::new(config, registry.clone(), false).await;
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
        crate::model::vars_provider::Kind::LineFile => "line file",
        crate::model::vars_provider::Kind::External => "external program",
        crate::model::vars_provider::Kind::Embedded => "embedded script",
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

/// An origin as `shall vars`/`why` show it: the filename and, when it is a real line rather than
/// a whole-provider attribution, the line number — `vars:6`, or `vars.shall` for a script.
pub fn short_origin(origin: &crate::config::grammar::Origin) -> String {
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

pub async fn handle_absent(config: &Config, registry: &Arc<BackendRegistry>) -> Result<()> {
    let resolver =
        crate::app::sync::resolver::StateResolver::new(config, registry.clone(), false).await;
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
        let source = spec.options.one("__source").unwrap_or("?");
        println!("{:<15} {:<25} {}", spec.backend, spec.name, source);
    }
    Ok(())
}

pub async fn handle_conflicts(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    out: Output,
) -> Result<()> {
    use crate::app::conflicts::{detect_conflicts, ConflictKind};

    // Resolve the full desired state (all manifests/modules/groups), flatten to specs.
    let resolver =
        crate::app::sync::resolver::StateResolver::new(config, registry.clone(), false).await;
    let desired = resolver.resolve_desired_state().await?;
    let specs: Vec<crate::core::PackageSpec> = desired.into_values().flatten().collect();
    let conflicts = detect_conflicts(&specs);

    if out.is_json() {
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
pub fn status_label(s: crate::core::HealthStatus) -> &'static str {
    use crate::core::HealthStatus::*;
    match s {
        Ok => "OK",
        Degraded => "WARN",
        Critical => "FAIL",
        Absent => "absent",
    }
}

/// The status label, colored for a terminal (green/yellow/red) and plain otherwise / under
/// NO_COLOR. Centralizing color here keeps the doctor output readable without a color crate.
pub fn status_label_colored(s: crate::core::HealthStatus) -> String {
    use crate::core::HealthStatus::*;
    use crate::utils::style::{color_enabled, paint, DIM, GREEN, RED, YELLOW};
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
pub fn doctor_tally(
    reports: &[(String, crate::core::HealthReport)],
) -> (usize, usize, usize, usize) {
    use crate::core::HealthStatus::*;
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
pub async fn check_health(app: &App, out: Output) -> Result<()> {
    use crate::core::{HealthReport, HealthStatus};

    // ---- Per-backend health, via each backend's own probe (not a shallow is_available). ----
    // See `probe_all_health` for why it is concurrent.
    // Q2's promotion is applied by `probe_all_health` itself, so this view and the `check`
    // rollup read one verdict rather than two.
    // Not `mut`, and that is the point: this view no longer adjusts the verdict it was handed.
    let reports: Vec<(String, HealthReport)> = probe_all_health(app).await;

    // All three promotions — Q2's absent-but-wanted, the `list`-actually-works probe, and Q11's
    // prerequisite check — are inside `probe_all_health` now, so this view and the `check`
    // rollup read one finished verdict. Each of them was a copy here once, and each time the
    // two views disagreed in public about the same machine.

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
                Some(format!("missing: {} (run `shall heal`)", dir.display())),
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
                Some("none yet (run `shall lock` to pin versions)".into()),
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
            let recorded: serde_json::Map<String, serde_json::Value> =
                match tokio::fs::read_to_string(&lock_path).await {
                    Ok(data) => serde_json::from_str::<serde_json::Value>(&data)
                        .ok()
                        .and_then(|v| v.get("locks").and_then(|l| l.as_object()).cloned())
                        .unwrap_or_default(),
                    Err(_) => serde_json::Map::new(),
                };
            let locked_keys: std::collections::HashSet<String> = recorded.keys().cloned().collect();
            let missing = managed.difference(&locked_keys).count();
            let stale = locked_keys.difference(&managed).count();

            // **Which packages have moved off the version that was recorded** — the job a
            // lockfile does on *every* manager, because it needs only a version the manager can
            // report and not one it can accept (II.53). It is asked here rather than left to the
            // planner because the planner answers "what would a sync change", and a sync changes
            // nothing on a manager that cannot be told which version to install: brew, pacman,
            // snap and the rest would silently have no version drift at all.
            let current =
                crate::verbs::plan::scan_installed_versions(&app.state, &app.registry).await;
            let moved: Vec<String> = recorded
                .iter()
                .filter_map(|(key, was)| {
                    let now = current.get(key)?;
                    (now != was).then(|| {
                        format!(
                            "{} {} -> {}",
                            key,
                            was.as_str().unwrap_or("?"),
                            now.as_str().unwrap_or("?")
                        )
                    })
                })
                .collect();

            if missing == 0 && stale == 0 && moved.is_empty() {
                system.push(("lockfile".into(), HealthStatus::Ok, None));
            } else {
                let mut parts = Vec::new();
                if missing > 0 || stale > 0 {
                    parts.push(format!("{} unpinned / {} stale", missing, stale));
                }
                if !moved.is_empty() {
                    // Named, not counted. "3 moved" sends the reader to diff two files by hand,
                    // and the whole point of recording a version on a manager that cannot replay
                    // one is that the record is the only place the movement is visible.
                    parts.push(format!("moved: {}", moved.join(", ")));
                }
                system.push((
                    "lockfile".into(),
                    HealthStatus::Degraded,
                    Some(format!(
                        "drifted: {} (run `shall lock`, or `shall heal`)",
                        parts.join("; ")
                    )),
                ));
            }
        }
    }

    // Git is not a dependency (X.5): its absence is reported, not treated as a fault. What is
    // unavailable without it is exactly the history-and-rollback set, and `doctor` is where
    // K8 says the standing notice lives — not on `sync`, which runs unattended.
    {
        let git = app.vcs().manager();
        if !crate::core::GitManager::git_available() {
            system.push((
                "git".into(),
                HealthStatus::Degraded,
                Some(
                    "not installed. Shall runs without it; generations, `rollback` and `diff` \
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
                     `shall git init` here turns it on."
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
    if out.is_json() {
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
    // `shall check health | grep '^\[READY\]'` enumerates every usable backend on this host. Without
    // this, a healthy `doctor` printed nothing about which package managers actually work.
    //
    // **A backend that has never met its manager says so, here, on its own line.** 62 backends
    // ship and a substantial minority have never completed a real install → list →
    // binary-on-PATH → remove in any harness — which is not a testing gap, it is a claim the
    // program makes and nothing has ever checked. Until now a user could not tell those apart
    // from the ones with a lifecycle behind them: same list, same word, same colour.
    //
    // `[READY]` still means "this manager is here and answers", which is unchanged and is what
    // the greppable roster promises. The suffix is a separate fact about *Shall's* evidence, not
    // about the machine, and it is phrased so `grep '^\[READY\]'` still enumerates every usable
    // backend.
    for (name, r) in &reports {
        if r.status == HealthStatus::Ok {
            match crate::backends::proving::unproven_reason(name) {
                None => println!("[READY] {}", name),
                Some(_) => println!("[READY] {} (unproven — no harness has run it)", name),
            }
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
        println!("\nSome checks are CRITICAL. Install the missing tools, or run `shall heal`.");
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
    use crate::core::{HealthReport, HealthStatus};

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
        use crate::core::missing_program;

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
