use crate::verbs::prelude::*;
use linix::model::Edit;

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

    let mut edits: Vec<Edit> = Vec::with_capacity(lines.len());
    for line in &lines {
        edits.push(
            app.declare(line, into, linix::model::Landing::Imperative)
                .await?,
        );
    }

    // And now the ordinary declarative pipeline makes it true — which is also what puts an
    // imperative install behind the guard for the first time (II.10).
    let synced = handle_sync(app, false, false, json).await;

    if let Err(e) = &synced {
        withdraw_what_can_never_succeed(app, e, &edits).await;
    }
    synced
}

/// A line this command just wrote that can never be satisfied, whatever happens next.
///
/// Two causes and no others. `Unresolvable` is a name no backend claims, and it carries that
/// name. `CommandFailed` marked [`Retryability::Permanent`] is the same fact by a different
/// road: a real manager ran, answered, and will answer identically every time — a typo behind
/// a real prefix (`scoop:definitely-not-real`) arrives here and not as `Unresolvable`, because
/// the *backend* resolved perfectly well.
///
/// Read off `CommandFailed` and never off [`Error::retryability`], which also calls a refusal,
/// a cancelled prompt and an unparseable config `Permanent`. Those are permanent in the retry
/// sense and none of them means the name was wrong; deleting a line because someone answered
/// "no" to a prompt would be a worse bug than the wedge this exists to prevent.
fn permanently_failed_message(e: &anyhow::Error) -> Option<&str> {
    match e.downcast_ref::<linix::core::Error>() {
        Some(linix::core::Error::CommandFailed {
            message,
            retry: linix::core::Retryability::Permanent,
        }) => Some(message),
        _ => None,
    }
}

/// The name a failed sync says can never be installed, if it says one.
fn unresolvable_name(e: &anyhow::Error) -> Option<&str> {
    match e.downcast_ref::<linix::core::Error>() {
        Some(linix::core::Error::Unresolvable { name, .. }) => Some(name.as_str()),
        _ => None,
    }
}

/// Take back the lines that can never be installed, and give the ones deliberately kept a way
/// out.
///
/// Every later command parses the model, so one line nothing can satisfy breaks `sync`,
/// `upgrade` and every install after it. Withdrawing the impossible ones is half the cure; the
/// other half is that a line kept on purpose — because the network dropped, or a lock was
/// held, and retrying is right — now names the file it is in and the command that removes it.
/// A wedge with an exit is not a wedge.
async fn withdraw_what_can_never_succeed(app: &App, e: &anyhow::Error, edits: &[Edit]) {
    let mut withdrawn: Vec<&Edit> = Vec::new();

    if let Some(name) = unresolvable_name(e) {
        if app.undeclare(name).await.is_ok_and(|es| !es.is_empty()) {
            warn!(
                "`{}` was taken back out of your files — nothing can install it.",
                name
            );
            withdrawn.extend(edits.iter().filter(|ed| ed.line.contains(name)));
        }
    } else if let Some(message) = permanently_failed_message(e) {
        // Which of the lines just written did the manager refuse? Managers name the package
        // they could not install, so the ones this run wrote and this message names are the
        // ones to take back. A line the message does not name is left alone and told about
        // below: withdrawing a declaration on a guess is the one outcome worse than keeping it.
        for edit in edits {
            let Ok(specs) = app.resolve_spec(&edit.line).await else {
                continue;
            };
            if specs.iter().any(|s| message.contains(&s.name))
                && app
                    .undeclare(&edit.line)
                    .await
                    .is_ok_and(|es| !es.is_empty())
            {
                warn!(
                    "`{}` was taken back out of {} — `{}` cannot install it, and trying again \
                     would fail the same way.",
                    edit.line,
                    edit.file.display(),
                    specs
                        .first()
                        .map(|s| s.backend.as_str())
                        .unwrap_or("that manager")
                );
                withdrawn.push(edit);
            }
        }
    }

    // A refusal is kept — deliberately, and for the reason above: LiNix said no to *this line
    // as written*, not to the name, and the refusal already says what to change (`@allow_http`,
    // `@sha256=`, a `@target=` outside the repo). Editing the line is the fix, so deleting it
    // would throw away the thing the user has to edit.
    //
    // But it must not be described as a transient failure. "`sync` will try it again" over a
    // plain-HTTP refusal promises a retry that fails identically forever, which is the sentence
    // E1 was about, said about a different cause.
    let refused = e
        .downcast_ref::<linix::core::Error>()
        .is_some_and(|err| matches!(err, linix::core::Error::Refused(_)));

    for edit in edits {
        if withdrawn.iter().any(|w| w.line == edit.line) {
            continue;
        }
        if refused {
            warn!(
                "`{}` is still declared in {} — it is kept because the line is the thing to \
                 edit, not the thing to delete. Change it as the refusal above says, or run \
                 `linix unmanage {}`. Re-running `sync` unchanged will refuse identically.",
                edit.line,
                edit.file.display(),
                edit.line
            );
        } else {
            warn!(
                "`{}` is still declared in {}, so `sync` will try it again. If you did not \
                 mean it, run `linix unmanage {}`.",
                edit.line,
                edit.file.display(),
                edit.line
            );
        }
    }
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

    // Dry-run answers "what would this do" without touching your files or the machine. The
    // answer is built here and not by the sync below, because the sync reads the files — and
    // in a preview the line is still in them, so a sync-shaped report says "remove 0" about
    // the very package the command names.
    if app.config.dry_run {
        return preview_uninstall(app, packages, json, temp).await;
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

/// What `uninstall` would do, without doing any of it.
///
/// The editor is in [`Writes::Planned`](linix::model::Writes) for the whole run, so the calls
/// below report their edits and write nothing — the same code path a real uninstall takes,
/// which is what keeps the preview and the act from drifting apart.
async fn preview_uninstall(
    app: &App,
    packages: &[String],
    json: bool,
    temp: Option<&Option<String>>,
) -> Result<()> {
    let mut planned = Vec::new();

    for pkg in packages {
        if let Some(Some(dur)) = temp {
            let at = linix::model::dated::absolute_after(chrono::Utc::now(), dur).with_context(
                || {
                    format!(
                        "Invalid --temp duration '{}'. Use forms like 2h, 30m, 7d.",
                        dur
                    )
                },
            )?;
            planned.push(serde_json::json!({
                "action": "suspend", "package": pkg, "until": at.to_string(),
            }));
            continue;
        }
        for edit in app.undeclare(pkg).await? {
            planned.push(serde_json::json!({
                "action": "undeclare",
                "package": pkg,
                "line": edit.line,
                "file": edit.file.display().to_string(),
            }));
        }
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&planned)?);
        return Ok(());
    }

    if planned.is_empty() {
        println!(
            "[DRY-RUN] nothing to uninstall — {} not declared in any active file.",
            match packages {
                [one] => format!("`{}` is", one),
                many => format!("`{}` are", many.join("`, `")),
            }
        );
        return Ok(());
    }

    println!("[DRY-RUN] would make {} change(s):", planned.len());
    for p in &planned {
        match p["action"].as_str() {
            Some("suspend") => println!(
                "  ~ suspend {} until {}",
                p["package"].as_str().unwrap_or(""),
                p["until"].as_str().unwrap_or("")
            ),
            _ => println!(
                "  - {}  (from {}, then removed by the sync that follows)",
                p["line"].as_str().unwrap_or(""),
                p["file"].as_str().unwrap_or("")
            ),
        }
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
    // A name nothing claims is a typo, and a typo that prints zero rows and exits 0 reads as
    // "that manager is empty" (Q9).
    app.require_known_backend(backend)?;
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
        // `info` reports on what is INSTALLED. "not found in any available backend" reads as
        // "no such package", which is a different and usually false claim — `linix search
        // ripgrep` finds it on crates.io while `info cargo:ripgrep` says this. Say which
        // question was asked, and name the command that answers the other one.
        println!(
            "'{}' is not installed on this machine, so there is nothing to describe.\n  \
             `linix search {}` looks for it in the managers you use.",
            package,
            package.rsplit(':').next().unwrap_or(package)
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use linix::core::{Error, Retryability};

    fn boxed(e: Error) -> anyhow::Error {
        anyhow::Error::new(e)
    }

    /// The reported bug: a typo behind a real backend prefix. `scoop` resolves, so this is
    /// never `Unresolvable`; scoop's own policy calls it permanent, and until that was read
    /// the line stayed in `modules/imperative.txt` and wedged every later command.
    #[test]
    fn a_permanent_command_failure_is_withdrawn() {
        let e = boxed(Error::CommandFailed {
            message: "`scoop` failed: Couldn't find manifest for 'definitely-not-real'."
                .to_string(),
            retry: Retryability::Permanent,
        });
        assert_eq!(
            permanently_failed_message(&e),
            Some("`scoop` failed: Couldn't find manifest for 'definitely-not-real'.")
        );
    }

    /// The half that must not regress. A dropped network or a held lock means you did mean
    /// it, and the line stays so a retry works.
    #[test]
    fn a_transient_or_unclassified_failure_keeps_the_line() {
        for retry in [Retryability::Transient, Retryability::Unknown] {
            let e = boxed(Error::CommandFailed {
                message: "`apt` failed: Could not get lock /var/lib/dpkg/lock".to_string(),
                retry,
            });
            assert_eq!(
                permanently_failed_message(&e),
                None,
                "withdrew on {retry:?}"
            );
        }
    }

    /// Every other variant that `Error::retryability()` also calls `Permanent`. None of them
    /// says the name was wrong, and withdrawing on any of them would delete a declaration the
    /// user still means — the reason this reads `CommandFailed` rather than `retryability()`.
    #[test]
    fn no_other_permanent_error_withdraws_a_line() {
        let others = [
            Error::Refused("the guard said no".into()),
            Error::Cancelled,
            Error::Config("modules/web.txt:3: bad line".into()),
            Error::Validation("nope".into()),
            Error::Permission("need root".into()),
            Error::BackendNotFound("nosuch".into()),
            Error::PackageNotFound("nosuch".into()),
            Error::Unsupported("purge".into()),
            Error::UnsupportedPlatform("aix".into()),
            Error::Differences("2 changes".into()),
            Error::LuaScript("boom".into()),
            Error::Toml("bad".into()),
            Error::Json("bad".into()),
        ];
        for e in others {
            let label = format!("{e:?}");
            assert_eq!(
                Retryability::Permanent,
                e.retryability(),
                "{label} is not the case this test guards"
            );
            assert_eq!(
                permanently_failed_message(&boxed(e)),
                None,
                "{label} would have withdrawn a line the user still means"
            );
        }
    }

    /// The existing path, kept working: a bare name nothing claims carries its own name.
    #[test]
    fn an_unresolvable_name_is_still_recognised_and_carries_itself() {
        let e = boxed(Error::Unresolvable {
            name: "linix-no-such-pkg-zzz".into(),
            message: "no backend claims it".into(),
        });
        assert_eq!(unresolvable_name(&e), Some("linix-no-such-pkg-zzz"));
        assert_eq!(permanently_failed_message(&e), None);
    }

    /// Both readers must survive the `.context()` wrapping every caller adds, or the fix
    /// works in a unit test and never once in the program.
    #[test]
    fn both_readers_see_through_a_context_chain() {
        let e = boxed(Error::CommandFailed {
            message: "`scoop` failed: Couldn't find manifest for 'nope'.".into(),
            retry: Retryability::Permanent,
        })
        .context("while syncing")
        .context("while installing");
        assert!(permanently_failed_message(&e).is_some());

        let u = boxed(Error::Unresolvable {
            name: "zzz".into(),
            message: "m".into(),
        })
        .context("while syncing");
        assert_eq!(unresolvable_name(&u), Some("zzz"));
    }
}
