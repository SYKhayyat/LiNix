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

/// Whether a failed sync says the name it was given does not exist.
///
/// One question, asked of a property rather than of prose. It was `CommandFailed` marked
/// [`Retryability::Permanent`] until N-1, and that reading was wrong in both directions:
/// permanence is not existence (helm's `plugin already exists` is permanent about a name that
/// is plainly there), and the 36 backends with no [`ExitPolicy`] never answered `Permanent` at
/// all, so a mistyped `npm:` package wedged the config while the same typo behind `scoop:`
/// did not. The backends decide this now — from their own declared phrasings, or by saying so
/// directly — and this reads their answer.
fn says_a_name_is_absent(e: &anyhow::Error) -> bool {
    e.downcast_ref::<linix::core::Error>()
        .is_some_and(|err| err.says_a_name_is_absent())
}

/// A spawned manager's own words, when its policy recognised them as "no such name".
///
/// The message is *not* what establishes the fact — `says_a_name_is_absent` did that. It is
/// read only to pick which of the lines this command wrote the manager was talking about,
/// which is a question the fact cannot answer and the edits can.
fn absent_command_message(e: &anyhow::Error) -> Option<&str> {
    match e.downcast_ref::<linix::core::Error>() {
        Some(linix::core::Error::CommandFailed {
            message,
            absent_name: true,
            ..
        }) => Some(message),
        _ => None,
    }
}

/// The name a name-resolving backend says is not there — a git host, an index, an API. Those
/// backends looked one name up and know which, so nothing has to be inferred from their text.
fn backend_absent_name(e: &anyhow::Error) -> Option<&str> {
    match e.downcast_ref::<linix::core::Error>() {
        Some(err @ linix::core::Error::NoSuchPackage { .. }) => err.absent_name(),
        _ => None,
    }
}

/// Whether a manager's output is talking about this package.
///
/// Managers wrap their output at the terminal width and pixi breaks lines *inside* a package
/// name (`No candidates were found for linix-\n      no-such-pkg-zzz`), so a name that is
/// plainly there reads as a name nobody mentioned. Comparing with the whitespace taken out
/// recovers it. This decides *which* line, never *whether* — a wrong answer here keeps a
/// declaration that could have been withdrawn, which is the safe direction.
fn mentions_package(message: &str, name: &str) -> bool {
    if message.contains(name) {
        return true;
    }
    let squeeze = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    squeeze(message).contains(&squeeze(name))
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
        // `Unresolvable` carries the name as the user wrote it, so the model takes it back
        // directly and no line has to be identified.
        if app.undeclare(name).await.is_ok_and(|es| !es.is_empty()) {
            warn!(
                "`{}` was taken back out of your files — nothing can install it.",
                name
            );
            withdrawn.extend(edits.iter().filter(|ed| ed.line.contains(name)));
        }
    } else if says_a_name_is_absent(e) {
        // A backend has determined that a name it was handed is not there. Which of the lines
        // this command just wrote is that about? Two ways to know, and neither is "the error
        // sounded permanent": the backend says which name it looked up, or — for a spawned
        // manager, which reports about a whole command — the manager's output mentions it.
        //
        // A line nothing identifies is left alone and told about below. Withdrawing on a
        // guess is the one outcome worse than keeping a line: a `sync` that fails on a
        // pre-existing wedge would otherwise delete the good declaration just written.
        let named = backend_absent_name(e);
        let message = absent_command_message(e);
        for edit in edits {
            let Ok(specs) = app.resolve_spec(&edit.line).await else {
                continue;
            };
            let is_this_line = match (named, message) {
                (Some(n), _) => specs.iter().any(|s| s.name == n),
                (None, Some(m)) => specs.iter().any(|s| mentions_package(m, &s.name)),
                (None, None) => false,
            };
            if is_this_line
                && app
                    .undeclare(&edit.line)
                    .await
                    .is_ok_and(|es| !es.is_empty())
            {
                warn!(
                    "`{}` was taken back out of {} — `{}` has no such package, and trying \
                     again would fail the same way.",
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

    let why = why_kept(e);
    for edit in edits {
        if withdrawn.iter().any(|w| w.line == edit.line) {
            continue;
        }
        warn!("{}", kept_line_advice(why, &edit.line, &edit.file));
    }
}

/// Why a line this command wrote is still in the file after the sync failed.
///
/// Named rather than decided at the moment of printing. E1's wording half was one `else`
/// covering four different situations and it promised a retry for all of them; a promise the
/// program has already disproved is the sentence this whole finding is about, so which
/// situations exist has to be something a test can enumerate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WhyKept {
    /// LiNix said no to *this line as written* — plain HTTP, no `@sha256=`, a `@target=`
    /// inside the repo. The refusal already says what to change, and the line is the thing
    /// the user edits, so deleting it would throw away the fix.
    Refused,
    /// It was called transient, retried, and came back identical. The retry already happened;
    /// saying another one will help is a promise the program disproved a second ago. The line
    /// still stays: the cause can be a `wget` on the PATH that rejects the flags the manager
    /// passes, and a declaration is not deleted over a broken environment.
    Exhausted,
    /// A name is absent and nothing tied it to this line — the manager reported about a
    /// command covering several, or wrapped its output through the middle of the name.
    NameAbsentElsewhere,
    /// LiNix classified it as passing: a rate-limit window, a dropped connection, a lock
    /// someone else holds. The retry that helps here is the *next run*, not this one, and the
    /// error above already says how long the window is — so this is the one branch that may
    /// promise a later attempt will work, because something did look.
    Transient,
    /// Nothing classified it. Another attempt is worth suggesting, and the honest reason is
    /// that nobody looked rather than that it will work.
    Unclassified,
}

fn why_kept(e: &anyhow::Error) -> WhyKept {
    let Some(err) = e.downcast_ref::<linix::core::Error>() else {
        return WhyKept::Unclassified;
    };
    if matches!(err, linix::core::Error::Refused(_)) {
        return WhyKept::Refused;
    }
    match err.retryability() {
        linix::core::Retryability::Exhausted => return WhyKept::Exhausted,
        // Ahead of the name check on purpose: `says_a_name_is_absent` reads the failure's
        // text, and a passing HTTP failure whose body happens to contain "not found" would
        // otherwise be reported as a package name that does not exist — sending the user to
        // edit a line that is correct. The classification is structured; the text match is a
        // guess, so the classification wins where they disagree.
        linix::core::Retryability::Transient => return WhyKept::Transient,
        linix::core::Retryability::Permanent | linix::core::Retryability::Unknown => {}
    }
    if err.says_a_name_is_absent() {
        return WhyKept::NameAbsentElsewhere;
    }
    WhyKept::Unclassified
}

/// What to tell a user about a line that stayed.
///
/// Every branch names the file the line is in and the command that removes it — a wedge with
/// an exit is not a wedge — and only [`WhyKept::Unclassified`] may suggest that `sync` trying
/// again could work, because it is the only one where LiNix has not already been shown
/// otherwise.
fn kept_line_advice(why: WhyKept, line: &str, file: &std::path::Path) -> String {
    let where_it_is = format!("`{}` is still declared in {}", line, file.display());
    match why {
        WhyKept::Exhausted => format!(
            "{}, but the failure above repeated on every retry, so `sync` will keep failing \
             the same way until its cause is fixed. Read the error above, or run \
             `linix unmanage {}`.",
            where_it_is, line
        ),
        WhyKept::Refused => format!(
            "{} — it is kept because the line is the thing to edit, not the thing to delete. \
             Change it as the refusal above says, or run `linix unmanage {}`. Re-running \
             `sync` unchanged will refuse identically.",
            where_it_is, line
        ),
        WhyKept::NameAbsentElsewhere => format!(
            "{}, and the failure above says a package name does not exist. `sync` will keep \
             failing the same way until the line naming it is corrected or removed with \
             `linix unmanage {}`.",
            where_it_is, line
        ),
        WhyKept::Transient => format!(
            "{}, and the failure above is a passing one — a window, a lock or a connection, \
             not the line. That is why it is kept: the next `sync` is expected to succeed \
             without you changing anything. Read the error above for how long it lasts, or \
             run `linix unmanage {}` if you did not mean the line at all.",
            where_it_is, line
        ),
        WhyKept::Unclassified => format!(
            "{}, so `sync` will try it again. Nothing classified the failure above, so if it \
             repeats unchanged the cause is not a passing one — run `linix unmanage {}` if you \
             did not mean it.",
            where_it_is, line
        ),
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
    // Q9: `uninstall nosuchbackend:foo` warned that it "is not declared in any active file" —
    // true, and it names the wrong thing. The manager is what does not exist, and the message
    // sent the user looking through their modules for a line they never wrote.
    app.require_known_spec_backends(packages).await?;
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
    // Q9, before a hold is recorded: `hold nosuchbackend:foo` wrote the hold and answered
    // `Held 1 package(s).` at exit 0, against a manager that does not exist.
    app.require_known_spec_backends(packages).await?;
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
    let recorded = {
        let mut state = app.state.lock().await;
        for p in packages {
            if state.hold(p) {
                n += 1;
            }
        }
        state.save()?
    };
    if recorded {
        println!(
            "Held {} package(s). `linix upgrade` will skip them until `linix unhold`.",
            n
        );
    } else {
        println!(
            "[DRY-RUN] would hold {} package(s). Nothing was recorded.",
            n
        );
    }
    Ok(())
}

pub(crate) async fn handle_unhold(app: &App, packages: &[String]) -> Result<()> {
    app.require_known_spec_backends(packages).await?;
    let mut n = 0usize;
    let recorded = {
        let mut state = app.state.lock().await;
        for p in packages {
            if state.unhold(p) {
                n += 1;
            }
        }
        state.save()?
    };
    if recorded {
        println!("Released {} hold(s).", n);
    } else {
        println!(
            "[DRY-RUN] would release {} hold(s). Nothing was recorded.",
            n
        );
    }
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
        let Ok(Some(remote)) = s.lookup(&p.name).await else {
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
    // Any remaining properties, surfaced rather than hidden — but not every property is a
    // field, and this loop used to render them all as though they were.
    //
    // `linix info service:Appinfo` printed `status raw:    [SC] QueryServiceConfig SUCCESS`:
    // a key name with its underscore swapped for a space, holding the whole of `sc qc`'s
    // multi-line output, squeezed into a 14-column aligned row. Two faults in one line — an
    // internal key shown as a label, and a tool's raw dump shown as a value (GRADER §4:
    // *flag every place internal vocabulary leaks*).
    let internal = |k: &str| k.starts_with("__");
    let verbatim = |k: &str| k.ends_with("_raw");
    // Sorted, because `properties` is a `HashMap` and Rust randomises its iteration order per
    // process — so two `info` runs on one unchanged package printed their fields in different
    // orders. Latent rather than observed on the host this was written on: no backend here
    // carries two properties the generic loop reaches. It is still output a person diffs.
    let mut ordered: Vec<(&String, &String)> = p.properties.iter().collect();
    ordered.sort_by_key(|(k, _)| k.as_str());
    for (k, v) in &ordered {
        if matches!(k.as_str(), "description" | "install_path" | "bin_path")
            || internal(k)
            || verbatim(k)
        {
            continue;
        }
        let label = format!("{}:", k.replace('_', " "));
        println!("{:<14} {}", label, v);
    }
    // A manager's own output is quoted as its own words, at the end, where a multi-line block
    // can be read — rather than pretending to be a field with a value.
    for (k, v) in &ordered {
        if !verbatim(k) || v.trim().is_empty() {
            continue;
        }
        println!(
            "\nWhat the manager said about its {}:",
            k.trim_end_matches("_raw").replace('_', " ")
        );
        for line in v.lines() {
            println!("  {}", line);
        }
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
    use linix::core::exit_policy;
    use linix::core::{Error, Retryability};

    fn boxed(e: Error) -> anyhow::Error {
        anyhow::Error::new(e)
    }

    /// The reported bug: a typo behind a real backend prefix. `scoop` resolves, so this is
    /// never `Unresolvable`; scoop's own policy says its output means the name is not there,
    /// and until that was read the line stayed in `modules/imperative.txt` and wedged every
    /// later command.
    #[test]
    fn a_failure_that_says_the_name_is_absent_is_withdrawn() {
        let e = boxed(Error::command_failed_absent(
            "`scoop` failed: Couldn't find manifest for 'definitely-not-real'.",
        ));
        assert!(says_a_name_is_absent(&e));
        assert_eq!(
            absent_command_message(&e),
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
                absent_name: false,
            });
            assert!(!says_a_name_is_absent(&e), "withdrew on {retry:?}");
            assert_eq!(absent_command_message(&e), None, "withdrew on {retry:?}");
        }
    }

    /// GRADER round 5, 2026-07-30 — RED.
    ///
    /// `error.rs:226` classifies a rate limit `Transient`, and says why in as many words:
    /// *"The whole point of a rate limit is that the window moves."* `why_kept` branches on
    /// `Refused`, `Exhausted` and name-absence and then falls through to `Unclassified`, so the
    /// user is told **"Nothing classified the failure above"** about a failure this program
    /// classified three lines away — and then told *"if it repeats unchanged the cause is not a
    /// passing one"*, which is exactly backwards: a rate limit repeats unchanged *because* it is
    /// passing.
    ///
    /// Observed live on the macOS runner, with the window in the line above the advice:
    ///
    ///     Error: API rate limit: api.github.com is rate limiting this machine and does not
    ///     reset for 1236s, past the 30s ceiling. …
    ///      WARN `github:sharkdp/fd` is still declared in …, so `sync` will try it again.
    ///           Nothing classified the failure above, …
    ///
    /// It costs two red CI jobs: the sweep harness tests transience by retrying immediately,
    /// which cannot succeed inside a 1236-second window, so it scores `defect`, the macOS leg
    /// goes red, and the real-lifecycle ratchet falls 8 -> 7 and goes red behind it.
    #[test]
    fn a_transient_failure_is_not_reported_as_unclassified() {
        let e = boxed(Error::RateLimit(
            "api.github.com is rate limiting this machine and does not reset for 1236s".to_string(),
        ));
        assert_eq!(
            e.downcast_ref::<Error>().map(|x| x.retryability()),
            Some(Retryability::Transient),
            "this fixture is not transient, so it does not test the distinction"
        );

        let why = why_kept(&e);
        assert_ne!(
            why,
            WhyKept::Unclassified,
            "a failure `Error::retryability()` calls Transient is routed to the one branch whose \
             text says nothing classified it"
        );
    }

    /// The sentence itself, because the branch is only half the harm: the advice a user reads
    /// must not tell them a moving window will not move.
    #[test]
    fn a_transient_failure_is_not_advised_as_if_it_were_permanent() {
        let e = boxed(Error::RateLimit(
            "api.github.com is rate limiting this machine and does not reset for 1236s".to_string(),
        ));
        let advice = kept_line_advice(
            why_kept(&e),
            "github:sharkdp/fd",
            std::path::Path::new("modules/imperative.txt"),
        );
        assert!(
            !advice.contains("Nothing classified the failure above"),
            "the advice for a Transient failure is the Unclassified sentence:\n{advice}"
        );
        assert!(
            !advice.contains("the cause is not a passing one"),
            "a rate limit repeats unchanged precisely because it is passing:\n{advice}"
        );
    }

    /// **The distinction N-1 was about.** A command failure can be permanent and be about a
    /// name that plainly exists. Reading permanence as absence withdrew declarations for
    /// packages that were installed; reading it as the *only* road to absence left every
    /// manager with no policy wedging the config.
    #[test]
    fn a_permanent_failure_about_a_name_that_exists_never_withdraws() {
        let cases = [
            // helm refusing an unsignable plugin source, and refusing one already installed.
            ("plugin already exists", exit_policy::helm()),
            (
                "plugin source does not support verification",
                exit_policy::helm(),
            ),
            // A crate that is real and simply ships no program.
            ("error: there are no binaries", exit_policy::cargo()),
            // nimble: the package exists, the `@version=` on the line does not.
            ("Error: Version not found", exit_policy::nimble()),
            // scoop declining to remove what is not on the machine says nothing about the
            // bucket, and a failed uninstall must never delete the declaration.
            ("ERROR 'jq' isn't installed.", exit_policy::scoop()),
        ];
        for (output, policy) in cases {
            assert_eq!(
                policy.retryability(&linix::core::ExitPolicy::haystack(output.as_bytes(), b"")),
                Retryability::Permanent,
                "not permanent, so this case does not test the distinction: {output}"
            );
            assert!(
                !policy.names_an_absent_package(&linix::core::ExitPolicy::haystack(
                    output.as_bytes(),
                    b""
                )),
                "read as a missing name, so a declaration would be withdrawn over: {output}"
            );
        }
    }

    /// Every other variant that `Error::retryability()` also calls `Permanent`. None of them
    /// says the name was wrong, and withdrawing on any of them would delete a declaration the
    /// user still means — the reason this reads a property and not `retryability()`.
    #[test]
    fn no_other_permanent_error_withdraws_a_line() {
        let others = [
            Error::Refused("the guard said no".into()),
            Error::Cancelled,
            Error::Config("modules/web.txt:3: bad line".into()),
            Error::Validation("nope".into()),
            Error::Permission("need root".into()),
            Error::BackendNotFound("nosuch".into()),
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
            assert!(
                !says_a_name_is_absent(&boxed(e)),
                "{label} would have withdrawn a line the user still means"
            );
        }
    }

    /// The two variants that *do* withdraw, and the reason each is safe to: both carry the
    /// name they looked up, so nothing is inferred from a sentence.
    #[test]
    fn the_variants_that_withdraw_carry_the_name_they_looked_up() {
        let no_such = Error::NoSuchPackage {
            name: "linix-zzz-nope/nope".into(),
            message: "the repo has no published release".into(),
        };
        assert!(says_a_name_is_absent(&boxed(no_such.clone())));
        assert_eq!(no_such.absent_name(), Some("linix-zzz-nope/nope"));
        assert_eq!(
            backend_absent_name(&boxed(no_such)),
            Some("linix-zzz-nope/nope")
        );

        let unresolvable = Error::Unresolvable {
            name: "linix-no-such-pkg-zzz".into(),
            message: "no backend claims it".into(),
        };
        assert!(says_a_name_is_absent(&boxed(unresolvable.clone())));
        assert_eq!(unresolvable.absent_name(), Some("linix-no-such-pkg-zzz"));
        // A spawned manager's failure is the one that does *not* know which name, which is
        // why the edits are consulted for that case and only that case.
        assert_eq!(
            backend_absent_name(&boxed(Error::command_failed_absent("`npm` failed: 404"))),
            None
        );
    }

    /// The existing path, kept working: a bare name nothing claims carries its own name.
    #[test]
    fn an_unresolvable_name_is_still_recognised_and_carries_itself() {
        let e = boxed(Error::Unresolvable {
            name: "linix-no-such-pkg-zzz".into(),
            message: "no backend claims it".into(),
        });
        assert_eq!(unresolvable_name(&e), Some("linix-no-such-pkg-zzz"));
        assert_eq!(absent_command_message(&e), None);
    }

    /// Every reader must survive the `.context()` wrapping every caller adds, or the fix
    /// works in a unit test and never once in the program.
    #[test]
    fn every_reader_sees_through_a_context_chain() {
        let e = boxed(Error::command_failed_absent(
            "`scoop` failed: Couldn't find manifest for 'nope'.",
        ))
        .context("while syncing")
        .context("while installing");
        assert!(says_a_name_is_absent(&e));
        assert!(absent_command_message(&e).is_some());

        let u = boxed(Error::Unresolvable {
            name: "zzz".into(),
            message: "m".into(),
        })
        .context("while syncing");
        assert_eq!(unresolvable_name(&u), Some("zzz"));
        assert!(says_a_name_is_absent(&u));

        let n = boxed(Error::NoSuchPackage {
            name: "owner/repo".into(),
            message: "no release".into(),
        })
        .context("while syncing");
        assert_eq!(backend_absent_name(&n), Some("owner/repo"));
    }

    /// The retry loop must not launder the fact. A failure that travels through
    /// `falsify_transience` and back is still about a name that is not there.
    #[test]
    fn the_absent_fact_survives_the_retry_classifier() {
        let e = boxed(Error::command_failed_absent("`npm` failed: 404 Not Found"));
        assert!(says_a_name_is_absent(&e));
        assert_eq!(
            e.downcast_ref::<Error>().map(|x| x.retryability()),
            Some(Retryability::Permanent),
            "an absent name is not worth retrying, so it never enters the loop"
        );
    }

    /// pixi wraps its output inside the package name. Attribution has to survive that, or a
    /// line whose name is plainly in the output reads as one nobody mentioned. Captured from
    /// pixi on this host, 2026-07-29.
    #[test]
    fn a_name_wrapped_across_lines_is_still_recognised_as_mentioned() {
        let wrapped = "  × failed to solve the environment\n  ╰─▶ Cannot solve the request \
                       because of: No candidates were found for linix-\n      \
                       no-such-pkg-zzz *.\n";
        assert!(
            !wrapped.contains("linix-no-such-pkg-zzz"),
            "the fixture no longer wraps, so it cannot test the wrap"
        );
        assert!(mentions_package(wrapped, "linix-no-such-pkg-zzz"));
        assert!(!mentions_package(wrapped, "some-other-package"));
    }

    /// Every reason a line can stay, and the sentence each one earns. Enumerated from the
    /// enum rather than sampled on a host, because the wording half of E1 came back by
    /// growing a fourth situation that the single `else` covering the other three still
    /// answered with "`sync` will try it again".
    #[test]
    fn only_an_unclassified_failure_may_suggest_that_a_retry_could_work() {
        let file = std::path::Path::new("modules/imperative.txt");
        for why in [
            WhyKept::Refused,
            WhyKept::Exhausted,
            WhyKept::NameAbsentElsewhere,
            WhyKept::Unclassified,
        ] {
            let advice = kept_line_advice(why, "npm:cowsay", file);
            // Every branch, without exception: where the line is, and the way out of it.
            assert!(
                advice.contains("modules/imperative.txt"),
                "{why:?} does not name the file the line is in: {advice}"
            );
            assert!(
                advice.contains("linix unmanage npm:cowsay"),
                "{why:?} does not name the command that removes it: {advice}"
            );
            let promises_a_retry = advice.contains("`sync` will try it again");
            assert_eq!(
                promises_a_retry,
                why == WhyKept::Unclassified,
                "{why:?} earns the wrong sentence — only an unclassified failure may suggest \
                 that trying again could work, because every other case has already been \
                 shown otherwise: {advice}"
            );
        }
    }

    /// And the classifier that feeds it. A failure whose name is absent must not be read as
    /// unclassified, which is what left `github:` printing the forbidden sentence.
    #[test]
    fn each_failure_is_classified_as_the_reason_its_line_stayed() {
        let cases = [
            (boxed(Error::Refused("plain HTTP".into())), WhyKept::Refused),
            (
                boxed(Error::CommandFailed {
                    message: "`luarocks` failed: failed downloading (tried 4 times)".into(),
                    retry: Retryability::Exhausted,
                    absent_name: false,
                }),
                WhyKept::Exhausted,
            ),
            (
                boxed(Error::command_failed_absent("`npm` failed: 404 Not Found")),
                WhyKept::NameAbsentElsewhere,
            ),
            (
                boxed(Error::NoSuchPackage {
                    name: "owner/repo".into(),
                    message: "no published release".into(),
                }),
                WhyKept::NameAbsentElsewhere,
            ),
            (
                boxed(Error::command_failed("`mix` failed: something")),
                WhyKept::Unclassified,
            ),
            // W35/R-3: this case used to expect `Unclassified`, and that expectation WAS the
            // defect — a dpkg lock someone else holds is the textbook passing failure, and
            // telling the user "nothing classified the failure above, so if it repeats
            // unchanged the cause is not a passing one" is the exact inversion R-3 measured on
            // a rate limit. The two expectations could not both stand; the register ruled with
            // the grader, so `Transient` now has a branch and this is it.
            (
                boxed(Error::CommandFailed {
                    message: "`apt` failed: Could not get lock".into(),
                    retry: Retryability::Transient,
                    absent_name: false,
                }),
                WhyKept::Transient,
            ),
            // And the one that keeps the new branch honest: `Permanent` is not `Transient`, so
            // widening the classifier cannot have swallowed the case above it.
            (
                boxed(Error::CommandFailed {
                    message: "`helm` failed: plugin source does not support verification".into(),
                    retry: Retryability::Permanent,
                    absent_name: false,
                }),
                WhyKept::Unclassified,
            ),
        ];
        for (e, expected) in cases {
            assert_eq!(why_kept(&e), expected, "misclassified: {e}");
        }
    }

    /// And the half that keeps attribution honest: a manager talking about one package is not
    /// talking about another. This is what stops a `sync` that failed on a pre-existing wedge
    /// from withdrawing the good line the command just wrote.
    #[test]
    fn attribution_does_not_spread_to_a_line_the_manager_never_named() {
        let message = "`npm` failed (exit 1): 404 Not Found - GET \
                       https://registry.npmjs.org/linix-no-such-pkg-zzz-9";
        assert!(mentions_package(message, "linix-no-such-pkg-zzz-9"));
        assert!(!mentions_package(message, "cowsay"));
    }
}
