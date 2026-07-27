use crate::verbs::prelude::*;

/// `teleport PKG BACKEND` — move a declared package to another manager, then sync (II.8).
pub(crate) async fn handle_teleport(app: &App, package: &str, backend: &str) -> Result<()> {
    if app.registry.get(backend).is_none() {
        anyhow::bail!(
            "`{}` is not a package manager on this machine. `linix check` lists the ones that are.",
            backend
        );
    }

    // A dry run says where the line would move without touching a file or the machine.
    if app.config.dry_run {
        println!(
            "[DRY-RUN] would move `{}` to `{}:{}` and sync.",
            package, backend, package
        );
        return Ok(());
    }

    let edits = app.retarget(package, backend).await?;
    if edits.is_empty() {
        anyhow::bail!(
            "`{}` is not declared in any active file, so there is no line to move. \
             To add it from `{}`, run `linix install {}:{}`.",
            package,
            backend,
            backend,
            package
        );
    }

    // The line now names the new manager; sync installs it there and removes the old copy as
    // drift — the same convergence every other edit-then-sync command relies on.
    handle_sync(app, false, false, false).await
}

pub(crate) async fn handle_install(
    app: &App,
    packages: &[String],
    json: bool,
    temp: Option<&str>,
    into: Option<&str>,
) -> Result<()> {
    // P1: this command IS a shortcut for editing a file and syncing. So the edit comes
    // first and convergence follows — S15. Backwards, every refusal on the write (nothing
    // active, several profiles active, an unwritable file) landed after the package was
    // already installed: on the machine, in no file, and drift by the next sync.
    let mut lines: Vec<String> = Vec::with_capacity(packages.len());
    for pkg_str in packages {
        lines.push(match temp {
            // II.16: a lease is a dated line. `--temp 2h` is a fine thing to type and an
            // impossible thing to store, so the duration is resolved against `now` here and
            // the file gets the moment it runs out (V.38). Nothing sweeps it up later —
            // the line simply stops counting, and sync removes what nothing declares.
            Some(dur) => {
                let at = linix::model::dated::absolute_after(chrono::Utc::now(), dur)
                    .with_context(|| {
                        format!(
                            "Invalid --temp duration '{}'. Use forms like 2h, 30m, 7d.",
                            dur
                        )
                    })?;
                format!("{}@expires={}", pkg_str.trim(), at)
            }
            None => pkg_str.trim().to_string(),
        });
    }

    // Dry-run answers "what would this do" without touching your files or the machine.
    if app.config.dry_run {
        let mut planned = Vec::new();
        for line in &lines {
            for spec in app.resolve_spec(line).await? {
                planned.push(serde_json::json!({
                    "action": "install", "backend": spec.backend, "name": spec.name,
                    "temporary": temp.is_some(),
                }));
            }
        }
        if json {
            println!("{}", serde_json::to_string_pretty(&planned)?);
        } else {
            println!("[DRY-RUN] would install {} package spec(s):", planned.len());
            for p in &planned {
                println!(
                    "  + {}:{}",
                    p["backend"].as_str().unwrap_or(""),
                    p["name"].as_str().unwrap_or("")
                );
            }
        }
        return Ok(());
    }

    for line in &lines {
        app.declare(line, into, linix::model::Landing::Imperative)
            .await?;
    }

    // And now the ordinary declarative pipeline makes it true — which is also what puts an
    // imperative install behind the guard for the first time (II.10).
    let synced = handle_sync(app, false, false, json).await;

    // A name no backend claims can never be satisfied by retrying, so leaving it in the file
    // wedges every later command that parses the model — one typo, and `status` is broken
    // until someone hand-edits a file. Withdraw it. Only this cause: a sync that failed for
    // any other reason (the network, a lock, a hook) leaves the line alone, because you did
    // mean it and retrying is the right move.
    if let Err(e) = &synced {
        if let Some(linix::core::Error::Unresolvable { name, .. }) = e.downcast_ref() {
            if app.undeclare(name).await.is_ok_and(|es| !es.is_empty()) {
                warn!(
                    "`{}` was taken back out of your files — nothing can install it.",
                    name
                );
            }
        }
    }
    synced
}

/// `uninstall PKG… [--temp]` — remove the line from every active module, sync (II.8).
///
/// P1, like `install`: the file edit IS the command, and convergence carries it out. So the
/// removal goes through the guard, the plan and the counts, exactly as any other removal
/// does — rather than reaching for the backend directly and asking the guard afterwards.
pub(crate) async fn handle_uninstall(
    app: &App,
    packages: &[String],
    json: bool,
    temp: Option<&Option<String>>,
) -> Result<()> {
    // Bare `--temp` restores when a `linix shell` session ends. That is the ephemeral shell's
    // business and it is outside the model by design (II.8), so it never touches a file.
    if let Some(None) = temp {
        let has_session = app.state.lock().await.active_session_id.is_some();
        if !has_session {
            anyhow::bail!(
                "Bare `--temp` restores on shell exit, but no `linix shell` session is \
                 active. Give a duration (e.g. --temp=2h) to schedule a timed restore."
            );
        }
        return suspend_for_session(app, packages).await;
    }

    let vocab = app.vocabulary().await?;
    let layout = app.config.layout();
    let facts = linix::config::parser::HostFacts::current();

    let mut never_declared: Vec<&str> = Vec::new();

    for pkg in packages {
        // II.8: a `--temp` uninstall of something undeclared has nothing to come back to.
        if let Some(Some(dur)) = temp {
            let declared = !linix::model::active_module_files(&layout, &vocab, &facts).is_empty()
                && app.declares(pkg).await?;
            if !declared {
                anyhow::bail!(
                    "{} isn't declared, so there's nothing for it to come back to. \
                     Did you mean a plain uninstall?",
                    pkg
                );
            }

            // II.16/V.37: "take the game away until the weekend". An `absent:` line with a
            // date beats the module that wants it (II.7 rule 6) until the date passes —
            // then the module wins again and it comes back. No timer, no sweep: the same
            // dated-line machinery `install --temp` uses, pointed the other way.
            let at = linix::model::dated::absolute_after(chrono::Utc::now(), dur).with_context(
                || {
                    format!(
                        "Invalid --temp duration '{}'. Use forms like 2h, 30m, 7d.",
                        dur
                    )
                },
            )?;
            let spec = app
                .resolve_spec(pkg)
                .await?
                .into_iter()
                .next()
                .with_context(|| format!("no package `{}` in any backend you use", pkg))?;
            app.declare(
                &format!("absent:{}:{}@until={}", spec.backend, spec.name, at),
                None,
                linix::model::Landing::Imperative,
            )
            .await?;
            continue;
        }

        // A line you can see deleted, while an identical line waits in a module you forgot
        // about, is a package that returns the next time you switch profiles (II.8).
        for module in linix::model::inactive_declarations(&layout, &vocab, &facts, pkg) {
            warn!(
                "{} is still declared in module `{}`, which isn't active. It will come back \
                 if a profile you activate uses it.",
                pkg, module
            );
        }

        let edits = app.undeclare(pkg).await?;
        if edits.is_empty() {
            warn!("{} is not declared in any active file.", pkg);
            never_declared.push(pkg.as_str());
        }
    }

    // And the ordinary pipeline removes it: the package is now drift, and removing drift is
    // what sync is (V.34).
    handle_sync(app, false, false, json).await?;

    // The sync runs first: the names that *were* declared are still owed their removal.
    // But a removal that removed nothing is not a removal, and a warning is the one thing
    // a script driving this cannot see.
    if !never_declared.is_empty() {
        anyhow::bail!(
            "nothing was uninstalled: {} not declared in any active file.",
            match never_declared.as_slice() {
                [one] => format!("`{}` is", one),
                many => format!("`{}` are", many.join("`, `")),
            }
        );
    }
    Ok(())
}

/// Bare `--temp` inside an ephemeral shell: suspend now, restore when the session ends.
///
/// Outside the model on purpose (II.8) — a shell session is not a declaration, and writing
/// a file for something that ends when the shell does would leave the file behind.
pub(crate) async fn suspend_for_session(app: &App, packages: &[String]) -> Result<()> {
    for pkg_str in packages {
        let (scoped_backend, bare_name) =
            linix::config::parser::split_removal_target(pkg_str, |b| app.registry.get(b).is_some());

        let mut done = false;
        for b in app.registry.available() {
            if scoped_backend.as_deref().is_some_and(|sb| sb != b.name()) {
                continue;
            }
            let Some(inst) = b.as_installable() else {
                continue;
            };
            let (present, version) = match b.as_queryable() {
                Some(q) => match q.info(&bare_name).await? {
                    Some(p) => (true, p.version),
                    None => (false, None),
                },
                None => (scoped_backend.as_deref() == Some(b.name()), None),
            };
            if !present {
                continue;
            }

            // Every removal path calls the guard (II.10), this one included.
            linix::app::sync::guard::enforce(
                &app.config,
                &app.registry,
                &[(b.name().to_string(), bare_name.clone())],
                linix::app::sync::guard::GuardScope::Remove,
            )
            .await?;

            if app.config.dry_run {
                println!("[DRY-RUN] would suspend {}:{}", b.name(), bare_name);
                done = true;
                break;
            }

            inst.remove(std::slice::from_ref(&bare_name), b.sudo_for_write())
                .await?;
            app.state.lock().await.remove(b.name(), &bare_name);
            app.state
                .lock()
                .await
                .suspend(b.name(), &bare_name, version, None)?;
            println!(
                "{} suspended; it comes back when this shell exits.",
                bare_name
            );
            done = true;
            break;
        }
        if !done {
            warn!("'{}' is not installed under any backend you use.", pkg_str);
        }
    }
    app.state.lock().await.save()?;
    Ok(())
}

pub(crate) async fn handle_hold(app: &App, packages: &[String]) -> Result<()> {
    if packages.is_empty() {
        let state = app.state.lock().await;
        let held = state.list_held();
        if held.is_empty() {
            println!("No packages are held.");
        } else {
            println!("Held packages ({}):", held.len());
            for h in held {
                println!("  {}", h);
            }
        }
        return Ok(());
    }
    let mut n = 0usize;
    {
        let mut state = app.state.lock().await;
        for p in packages {
            if state.hold(p) {
                n += 1;
            }
        }
        state.save()?;
    }
    println!(
        "Held {} package(s). `linix upgrade` will skip them until `linix unhold`.",
        n
    );
    Ok(())
}

pub(crate) async fn handle_unhold(app: &App, packages: &[String]) -> Result<()> {
    let mut n = 0usize;
    {
        let mut state = app.state.lock().await;
        for p in packages {
            if state.unhold(p) {
                n += 1;
            }
        }
        state.save()?;
    }
    println!("Released {} hold(s).", n);
    Ok(())
}

/// Render a package as one aligned row: backend, name, version.
pub(crate) fn print_package_row(p: &linix::core::Package) {
    println!(
        "{:<12} {:<32} {}",
        p.backend,
        p.name,
        p.version.as_deref().unwrap_or("")
    );
}

pub(crate) async fn handle_search(
    app: &App,
    query: &str,
    json: bool,
    installed: bool,
) -> Result<()> {
    let mut results = app.search(query).await?;
    if installed {
        // Keep only results LiNix already manages, so `search --installed foo` answers
        // "which of my packages match" without a second command.
        let managed: std::collections::HashSet<(String, String)> = {
            let state = app.state.lock().await;
            state
                .packages
                .iter()
                .map(|p| (p.backend.clone(), p.name.clone()))
                .collect()
        };
        results.retain(|p| managed.contains(&(p.backend.clone(), p.name.clone())));
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        if results.is_empty() && installed {
            println!("No installed package matches '{}'.", query);
        }
        for p in &results {
            print_package_row(p);
        }
    }
    Ok(())
}

/// One outdated package: what's installed now vs the newest the backend offers.
#[derive(serde::Serialize)]
pub(crate) struct Outdated {
    backend: String,
    name: String,
    installed: String,
    latest: String,
}

/// Find managed packages whose backend reports a newer version than what's installed. Backends
/// without a `Searchable` capability (no "latest" source) are honestly skipped, not guessed at.
pub(crate) async fn compute_outdated(app: &App, list: &[linix::core::Package]) -> Vec<Outdated> {
    use version_compare::{compare, Cmp};
    let mut out = Vec::new();
    for p in list {
        let Some(cur) = p.version.as_deref() else {
            continue;
        };
        let Some(b) = app.registry.get(&p.backend) else {
            continue;
        };
        let Some(s) = b.as_searchable() else {
            continue;
        };
        let Ok(Some(remote)) = s.remote_info(&p.name).await else {
            continue;
        };
        let Some(latest) = remote.version.as_deref() else {
            continue;
        };
        // A newer remote version than installed → outdated. Unparseable versions compare
        // unequal safely and are simply not reported.
        if compare(latest, cur) == Ok(Cmp::Gt) {
            out.push(Outdated {
                backend: p.backend.clone(),
                name: p.name.clone(),
                installed: cur.to_string(),
                latest: latest.to_string(),
            });
        }
    }
    out
}

pub(crate) async fn handle_list(
    app: &App,
    backend: Option<&str>,
    json: bool,
    outdated: bool,
) -> Result<()> {
    let list = app.list(backend).await?;
    if outdated {
        let rows = compute_outdated(app, &list).await;
        if json {
            println!("{}", serde_json::to_string_pretty(&rows)?);
        } else if rows.is_empty() {
            println!("Everything is up to date (for backends that report a latest version).");
        } else {
            println!(
                "{:<12} {:<32} {:<18} LATEST",
                "BACKEND", "PACKAGE", "INSTALLED"
            );
            for r in &rows {
                println!(
                    "{:<12} {:<32} {:<18} {}",
                    r.backend, r.name, r.installed, r.latest
                );
            }
            println!("\nUpgrade all: `linix upgrade --all`  ·  one: `linix upgrade <name>`");
        }
        return Ok(());
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&list)?);
    } else {
        for p in &list {
            print_package_row(p);
        }
    }
    Ok(())
}

pub(crate) async fn handle_info(app: &App, package: &str) -> Result<()> {
    let Some(p) = app.get_info(package).await? else {
        println!("Package '{}' not found in any available backend.", package);
        return Ok(());
    };

    println!("{:<14} {}", "Package:", p.name);
    println!("{:<14} {}", "Backend:", p.backend);
    if let Some(v) = &p.version {
        println!("{:<14} {}", "Version:", v);
    }
    if let Some(d) = p.properties.get("description") {
        println!("{:<14} {}", "Description:", d);
    }
    if let Some(path) = p
        .properties
        .get("install_path")
        .or_else(|| p.properties.get("bin_path"))
    {
        println!("{:<14} {}", "Install path:", path);
    }
    // Any remaining properties, surfaced rather than hidden.
    for (k, v) in &p.properties {
        if matches!(k.as_str(), "description" | "install_path" | "bin_path") {
            continue;
        }
        let label = format!("{}:", k.replace('_', " "));
        println!("{:<14} {}", label, v);
    }
    // Dependencies via the backend's MetadataProvider, if it has one.
    if let Some(b) = app.registry.get(&p.backend) {
        if let Some(mp) = b.as_metadata_provider() {
            if let Ok(deps) = mp.get_dependencies(&p.name).await {
                if !deps.is_empty() {
                    println!("{:<14} {}", "Dependencies:", deps.join(", "));
                }
            }
        }
    }
    Ok(())
}
