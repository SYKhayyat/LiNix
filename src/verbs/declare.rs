use crate::verbs::prelude::*;
use crate::verbs::sync::{handle_sync, SyncMode};

pub async fn handle_repo(app: &App, cmd: &RepoCommand) -> Result<()> {
    let explicit = match cmd {
        RepoCommand::Add { backend, .. } => backend.clone(),
        RepoCommand::Remove { backend, .. } => backend.clone(),
        RepoCommand::List { backend } => backend.clone(),
    };
    // No explicit `--backend`: fall back to the first backend in the `priority` file (this
    // host's default manager), or `apt` if the file names nothing.
    let b_name = match explicit {
        Some(b) => b,
        None => app
            .priority_backends()
            .await
            .into_iter()
            .next()
            .unwrap_or_else(|| "apt".into()),
    };

    // Q9: `repo` takes its backend positionally, so a typo landed on `Backend not found` —
    // true, but it named neither the file to edit nor the spelling to check, and the same
    // question has one good answer already written for `install`.
    app.require_known_backend(Some(&b_name))?;
    let b = app.registry.get(&b_name).context("Backend not found")?;
    let mgr = b
        .as_repo_manager()
        .context("Backend does not support repository management.")?;

    match cmd {
        RepoCommand::Add { name, url, .. } => {
            info!("Repo: Adding {} to {}...", name, b_name);
            mgr.add_repo(name, url, b.sudo_for_write()).await?;
        }
        RepoCommand::Remove { name, .. } => {
            // The imperative twin of the `repo:` teardown in `app/apply/extras.rs`. Guarding
            // the declarative path and not this one is how a guard comes to cover a resource
            // on Tuesday and not on Wednesday, depending on which command the user reached
            // for — the twin-branch shape `spec/history.md` records as S6.
            let reaped = crate::app::sync::guard::enforce_extras(
                &app.config,
                &app.registry,
                &[("repo".to_string(), format!("{}:{}", b_name, name))],
                &app.reaping,
                crate::app::sync::guard::GuardScope::Remove,
            )
            .await?;
            info!("Repo: Removing {} from {}...", name, b_name);
            mgr.remove_repo(name, b.sudo_for_write(), reaped).await?;
        }
        RepoCommand::List { .. } => {
            let repos = mgr.list_repos().await?;
            println!("{:<20} SOURCE", "NAME");
            for (n, u) in repos {
                println!("{:<20} {}", n, u);
            }
        }
    }
    Ok(())
}

/// Destroying a file you wrote is a plain refusal plus `--force`, like every other tool
/// (II.8). It has nothing to do with packages, so no removal setting reaches it — one prompt
/// standing for two unrelated questions is how it came to mean neither (E12).
pub fn refuse_overwrite(path: &std::path::Path, name: &str, force: bool) -> Result<()> {
    if force || !path.exists() {
        return Ok(());
    }
    anyhow::bail!(
        "module `{}` already exists at {}.\n  \
         Pass --force to overwrite it, or pick another name.",
        name,
        path.display()
    )
}

pub fn module_name(name: &str) -> Result<crate::model::ModuleName> {
    crate::model::ModuleName::new(name).map_err(|e| anyhow::anyhow!(e))
}

pub async fn handle_module(app: &App, cmd: &ModuleCommand) -> Result<()> {
    let layout = app.config.layout();
    match cmd {
        ModuleCommand::List => {
            // **The folder decides** (II.3): `modules/*.txt`, so a README.md in there costs
            // nothing. It used to list `*.module.txt`, a suffix II.1 does not have — so this
            // listed nothing on a real repo.
            let vocab = app.vocabulary().await?;
            let loader = crate::model::modules::ModuleLoader::new(&layout, &vocab);
            let names = loader.available();
            if names.is_empty() {
                println!(
                    "No modules yet. `shall module create <name>`, or `shall install` writes \
                     one for you."
                );
            }
            for n in names {
                println!("{}", n);
            }
        }
        ModuleCommand::Show { name } => {
            let path = layout.module_file(&module_name(name)?);
            let body = tokio::fs::read_to_string(&path)
                .await
                .with_context(|| format!("no module `{}` — looked in {}", name, path.display()))?;
            println!("{}", body);
        }
        ModuleCommand::Create { name, force } => {
            let path = layout.module_file(&module_name(name)?);
            refuse_overwrite(&path, name, *force)?;
            let body = format!(
                "# Module: {}\n\
                 #\n\
                 # A list of what this module holds, one per line:\n\
                 #\n\
                 #   apt:curl\n\
                 #   ripgrep            (no backend named — Shall asks each one in\n\
                 #                       `priority` order, then locks the answer)\n\
                 #   use base           (bring in another module)\n\
                 #   absent:apt:nano    (this must NOT exist)\n\
                 #\n\
                 # Nothing here happens until a profile reaches it: `use {}`.\n",
                name, name
            );
            let verb =
                crate::verbs::write_unless_previewing(app, &path, &body, "Created", "would create")
                    .await?;
            println!("{} {}", verb, path.display());
            println!(
                "  Add it to a profile with `use {}` — nothing reads a module no profile names.",
                name
            );
        }
        ModuleCommand::Add {
            source,
            name,
            force,
        } => {
            use crate::app::module_registry;
            let (url, default_name) = module_registry::resolve_module_source(source)?;
            let final_name = name.clone().unwrap_or(default_name);
            let path = layout.module_file(&module_name(&final_name)?);
            refuse_overwrite(&path, &final_name, *force)?;

            // Honour the configured value (F1); the pool raises a literal 0 to 1s, which
            // reqwest would otherwise read as an instant-fail timeout rather than "no timeout".
            let client = crate::core::http::api("shall-module", app.config.network_timeout_secs)?;
            info!("Fetching module from {}", url);
            let resp = client.get(&url).send().await?;
            if !resp.status().is_success() {
                anyhow::bail!("fetching {} returned HTTP {}", url, resp.status());
            }
            let body = resp.text().await?;
            if module_registry::looks_like_html(&body) {
                anyhow::bail!(
                    "response from {} looks like an HTML page, not a Shall module — check the source",
                    url
                );
            }

            let verb =
                crate::verbs::write_unless_previewing(app, &path, &body, "Added", "would add")
                    .await?;
            let count = module_registry::count_entries(&body);
            println!(
                "{} module `{}` ({} entries) from {}\n  saved to {}\n  \
                 Use it with `use {}` in a profile — nothing reads a module no profile names.",
                verb,
                final_name,
                count,
                url,
                path.display(),
                final_name
            );
        }
    }
    Ok(())
}

/// Apply a service spec (`service:<name>@<opts>`) through the install path.
pub async fn service_apply(app: &App, name: &str, opts: &str) -> Result<()> {
    let spec_str = if opts.is_empty() {
        format!("service:{}", name)
    } else {
        format!("service:{}@{}", name, opts)
    };
    let resolved = app.resolve_spec(&spec_str).await?;
    for spec in resolved {
        let b = app
            .registry
            .get(&spec.backend)
            .context("service backend unavailable on this host")?;
        if let Some(inst) = b.as_installable() {
            inst.install(std::slice::from_ref(&spec), b.sudo_for_write())
                .await?;
        }
    }
    Ok(())
}

pub async fn handle_service(app: &App, cmd: &ServiceCommand) -> Result<()> {
    // Enable/disable/start/stop/restart mutate the system and (enable/disable) the manifest.
    // Honor --dry-run by describing the action without touching either. Status/List are
    // read-only and always run.
    if app.config.dry_run {
        let action = match cmd {
            ServiceCommand::Enable { name } => Some(("enable + start", name)),
            ServiceCommand::Disable { name } => Some(("disable + stop", name)),
            ServiceCommand::Start { name } => Some(("start", name)),
            ServiceCommand::Stop { name } => Some(("stop", name)),
            ServiceCommand::Restart { name } => Some(("restart", name)),
            ServiceCommand::Status { .. } | ServiceCommand::List => None,
        };
        if let Some((what, name)) = action {
            crate::would_print!("would {} service '{}'.", what, name);
            return Ok(());
        }
    }
    match cmd {
        ServiceCommand::Enable { name } => {
            // **The write comes first, and this used to be the other way round.** `S15` and the
            // comment at `packages.rs:46` state the rule for exactly this shape: *"Backwards,
            // every refusal on the write (nothing active, several profiles active, an unwritable
            // file) landed after the package was already installed: on the machine, in no file,
            // and drift by the next sync."* A service is the same sentence with a different noun
            // — enabled on the box, declared nowhere, and turned off again by the next `sync`.
            app.declare(
                &format!("service:{}@enabled=true", name),
                None,
                crate::model::Landing::Imperative,
            )
            .await?;
            service_apply(app, name, "enabled=true,status=running").await?;
            println!("Service '{}' enabled and started.", name);
        }
        ServiceCommand::Disable { name } => {
            // Same order for the same reason, mirrored: the declaration is what `sync` reads, so
            // it goes first. An `undeclare` that failed after the service was already stopped
            // would leave a machine whose state no file explains, and the next `sync` would
            // start it again.
            app.undeclare(&format!("service:{}", name)).await?;
            service_apply(app, name, "enabled=false,status=stopped").await?;
            println!("Service '{}' disabled and stopped.", name);
        }
        ServiceCommand::Start { name } => {
            service_apply(app, name, "status=running").await?;
            println!("Service '{}' started.", name);
        }
        ServiceCommand::Stop { name } => {
            service_apply(app, name, "status=stopped").await?;
            println!("Service '{}' stopped.", name);
        }
        ServiceCommand::Restart { name } => {
            service_apply(app, name, "status=restarted").await?;
            println!("Service '{}' restarted.", name);
        }
        ServiceCommand::Status { name } => {
            let b = app
                .registry
                .get("service")
                .context("service backend unavailable on this host")?;
            match b.as_queryable() {
                Some(q) => match q.info(name).await? {
                    Some(pkg) => {
                        println!("{}: running", name);
                        if let Some(raw) = pkg.properties.get("status_raw") {
                            println!("{}", raw.trim());
                        }
                    }
                    None => println!("{}: not running (or unknown to this init system)", name),
                },
                None => println!("service status is not queryable on this platform"),
            }
        }
        ServiceCommand::List => {
            let b = app
                .registry
                .get("service")
                .context("service backend unavailable on this host")?;
            match b.as_queryable() {
                Some(q) => {
                    let svcs = q.list_installed().await?;
                    if svcs.is_empty() {
                        println!("No running services reported.");
                    } else {
                        println!("Running services ({}):", svcs.len());
                        for s in svcs {
                            println!("  {}", s.name);
                        }
                    }
                }
                None => println!("service listing is not available on this platform"),
            }
        }
    }
    Ok(())
}

pub async fn handle_hooks(app: &App, cmd: &HooksCommand) -> Result<()> {
    use crate::app::pm_hooks;

    // Path to this very binary, so a hook can call back into `shall`.
    let shall_bin = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "shall".to_string());

    match cmd {
        HooksCommand::Install { managers } => {
            let specs = pm_hooks::hook_specs(&shall_bin);
            let mut wrote = 0usize;
            for spec in &specs {
                if !managers.is_empty() && !managers.iter().any(|m| m == spec.manager) {
                    continue;
                }
                // Only install hooks for managers actually present on this system.
                if app.registry.get(spec.manager).is_none()
                    && !managers.iter().any(|m| m == spec.manager)
                {
                    continue;
                }
                if let Some(parent) = spec.path.parent() {
                    if let Err(e) = tokio::fs::create_dir_all(parent).await {
                        warn!(
                            "hooks: cannot create {} ({}). Try with sudo.",
                            parent.display(),
                            e
                        );
                        continue;
                    }
                }
                match tokio::fs::write(&spec.path, &spec.content).await {
                    Ok(()) => {
                        // Make script-style hooks executable on Unix.
                        #[cfg(unix)]
                        if spec.content.starts_with("#!") {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = tokio::fs::set_permissions(
                                &spec.path,
                                std::fs::Permissions::from_mode(0o755),
                            )
                            .await;
                        }
                        println!("  installed  {:<8} {}", spec.manager, spec.path.display());
                        wrote += 1;
                    }
                    Err(e) => warn!(
                        "hooks: failed to write {} ({}). This usually needs root.",
                        spec.path.display(),
                        e
                    ),
                }
            }
            if wrote == 0 {
                println!(
                    "No hooks installed. Named managers may be absent, or writing needs sudo.\n\
                     Hookable managers: {}",
                    pm_hooks::hookable_manager_names().join(", ")
                );
            } else {
                println!(
                    "\nInstalled {wrote} hook file(s). Manual installs now record into Shall."
                );
            }
        }
        HooksCommand::Uninstall { managers } => {
            let specs = pm_hooks::hook_specs(&shall_bin);
            let mut removed = 0usize;
            for spec in &specs {
                if !managers.is_empty() && !managers.iter().any(|m| m == spec.manager) {
                    continue;
                }
                if tokio::fs::try_exists(&spec.path).await.unwrap_or(false) {
                    match tokio::fs::remove_file(&spec.path).await {
                        Ok(()) => {
                            println!("  removed    {:<8} {}", spec.manager, spec.path.display());
                            removed += 1;
                        }
                        Err(e) => warn!("hooks: failed to remove {} ({})", spec.path.display(), e),
                    }
                }
            }
            println!("Removed {removed} hook file(s).");
        }
        HooksCommand::Status => {
            let specs = pm_hooks::hook_specs(&shall_bin);
            println!("{:<10} {:<9} {:<9} PATH", "MANAGER", "PRESENT", "HOOKED");
            for spec in &specs {
                let present = app.registry.get(spec.manager).is_some();
                let hooked = tokio::fs::try_exists(&spec.path).await.unwrap_or(false);
                println!(
                    "{:<10} {:<9} {:<9} {}",
                    spec.manager,
                    if present { "yes" } else { "no" },
                    if hooked { "yes" } else { "no" },
                    spec.path.display()
                );
            }
        }
        HooksCommand::ShellInit { shell } => {
            print!("{}", pm_hooks::shell_wrappers(&shall_bin, shell));
        }
    }
    Ok(())
}

/// Shared recording path for a single hooked target. Repo installs become declarative
/// (recorded + appended to the active module); local-file installs are recorded imperatively
/// and kept OUT of the modules (not reproducible), so a sync never removes them as drift.
pub async fn record_hooked_target(
    app: &App,
    manager: &str,
    op: crate::app::pm_hooks::HookOp,
    target: &str,
) -> Result<()> {
    use crate::app::pm_hooks::{classify_install_target, local_file_stem, HookOp, InstallKind};

    match op {
        HookOp::Install => {
            let kind = classify_install_target(target);
            let (name, source, declarative) = match kind {
                InstallKind::Repo => (target.to_string(), format!("hook:{manager}"), true),
                InstallKind::LocalFile => {
                    (local_file_stem(target), "local-file".to_string(), false)
                }
            };
            app.state
                .lock()
                .await
                .add(manager, &name, None, Default::default(), &source, false);
            if declarative {
                app.declare(
                    &format!("{manager}:{name}"),
                    None,
                    crate::model::Landing::Hooks,
                )
                .await?;
            }
            info!(
                "hook: recorded install {}:{} ({})",
                manager,
                name,
                if declarative {
                    "managed"
                } else {
                    "imperative/local"
                }
            );
        }
        HookOp::Remove => {
            app.state.lock().await.remove(manager, target);
            app.undeclare(&format!("{manager}:{target}")).await?;
            info!("hook: recorded remove {}:{}", manager, target);
        }
    }
    Ok(())
}

pub async fn handle_hook_record(
    app: &App,
    manager: &str,
    op: &str,
    targets: &[String],
) -> Result<()> {
    let op = crate::app::pm_hooks::HookOp::parse(op)
        .ok_or_else(|| anyhow::anyhow!("hook-record: --op must be 'install' or 'remove'"))?;
    for target in targets {
        record_hooked_target(app, manager, op, target).await?;
    }
    app.state.lock().await.save()?;
    app.git_autocommit("shall: record hooked package change")
        .await;
    Ok(())
}

pub async fn handle_hook_reconcile(app: &App, manager: &str) -> Result<()> {
    // Additive reconcile: record packages the manager reports installed that Shall isn't yet
    // tracking. We never auto-remove here — a missing package could be a transient query
    // hiccup, and destructive action from a background hook would be a nasty surprise.
    let Some(backend) = app.registry.get(manager) else {
        warn!(
            "hook-reconcile: backend '{}' is not available; skipping.",
            manager
        );
        return Ok(());
    };
    let Some(queryable) = backend.as_queryable() else {
        return Ok(());
    };
    // Adopting nothing is the safe direction when a listing fails — this records what a
    // manager installed behind Shall's back, and inventing entries would be worse. But a hook
    // that silently records nothing looks exactly like a hook with nothing to record, and this
    // one runs unattended, where nobody is watching for the difference.
    let installed = match queryable.list_installed().await {
        Ok(pkgs) => pkgs,
        Err(e) => {
            warn!(
                "hook-reconcile: `{manager}` could not be listed, so nothing it installed was \
                 recorded as managed. It is not that there was nothing: {e}"
            );
            return Ok(());
        }
    };
    let mut newly = 0usize;
    {
        let mut state = app.state.lock().await;
        for pkg in &installed {
            if !state.is_managed(manager, &pkg.name) {
                state.add(
                    manager,
                    &pkg.name,
                    pkg.version.clone(),
                    Default::default(),
                    &format!("hook:{manager}"),
                    false,
                );
                newly += 1;
            }
        }
        state.save()?;
    }
    if newly > 0 {
        info!(
            "hook-reconcile: adopted {} new {}-installed package(s).",
            newly, manager
        );
        app.git_autocommit("shall: reconcile hooked manager").await;
    }
    Ok(())
}

pub async fn handle_hook_observe(
    app: &App,
    manager: Option<&str>,
    learn: bool,
    argv: &[String],
) -> Result<()> {
    use crate::app::pm_hooks::{detect_operation, extract_targets};

    let Some(op) = detect_operation(argv) else {
        // Not an install/remove command (e.g. `apt list`); nothing to record.
        return Ok(());
    };
    // Manager name: explicit, else inferred from argv[0] (the wrapped binary).
    let manager = manager
        .map(|m| m.to_string())
        .or_else(|| argv.first().cloned())
        .unwrap_or_else(|| "unknown".to_string());

    // For a brand-new manager we've never seen, suggest onboarding it properly.
    if learn && app.registry.get(&manager).is_none() {
        info!(
            "Auto-learn: observed unknown manager '{}'. Consider onboarding it with a TOML \
             definition so Shall knows its full command set.",
            manager
        );
    }

    let targets = extract_targets(argv);
    for target in &targets {
        record_hooked_target(app, &manager, op, target).await?;
    }
    if !targets.is_empty() {
        app.state.lock().await.save()?;
        app.git_autocommit("shall: observed manual package change")
            .await;
    }
    Ok(())
}

/// `shall schedule` — a shortcut for editing the `schedules` file, then converging.
///
/// The file is the state (II.6: being in the file means it's on), so `add` and `remove` write
/// it and `sync` provisions what changed. They do not talk to the OS scheduler directly: a
/// command that registered a timer the file did not describe would be a second store, and the
/// two would disagree about what this machine runs.
pub async fn handle_schedule(app: &App, cmd: &ScheduleCommand) -> Result<()> {
    use crate::model::schedule::{add_line, remove_line};

    let file = app.config.layout().schedules_file();
    let body = tokio::fs::read_to_string(&file).await.unwrap_or_default();
    let registry = app.registry.clone();
    let known = move |b: &str| registry.get(b).is_some();

    match cmd {
        ScheduleCommand::Add {
            name,
            cron,
            run,
            notify,
        } => {
            let updated = add_line(&body, name, cron, run, notify.as_deref())
                .map_err(|e| anyhow::anyhow!(e))?;
            // Parse what was just written before it is written: a bad cron or an unknown key
            // must be refused at the door, naming the line, not discovered at provision time.
            crate::config::grammar::parse_document(&file, &updated, &known)?;
            let verb =
                crate::verbs::write_unless_previewing(app, &file, &updated, "Added", "would add")
                    .await?;
            println!("{} `schedule:{}` to {}.", verb, name, file.display());
        }
        ScheduleCommand::Remove { name } => {
            let Some(updated) = remove_line(&body, name) else {
                println!("No `schedule:{}` in {}.", name, file.display());
                return Ok(());
            };
            let verb = crate::verbs::write_unless_previewing(
                app,
                &file,
                &updated,
                "Removed",
                "would remove",
            )
            .await?;
            println!("{} `schedule:{}` from {}.", verb, name, file.display());
        }
        ScheduleCommand::List => {
            let doc = crate::config::grammar::parse_document(&file, &body, &known)?;
            let facts = crate::config::parser::HostFacts::current();
            let mut listed = 0usize;
            for (stmt, origin) in doc.statements_for(&facts)? {
                if let crate::config::grammar::Statement::Schedule(name, opts) = stmt {
                    let cfg = crate::model::schedule::schedule_config(
                        &name,
                        &opts,
                        &origin,
                        &app.config.guard.never_unattended,
                    )?;
                    println!("{:<15} {:<15} {}", cfg.name, cfg.cron, cfg.command);
                    listed += 1;
                }
            }
            // An empty list printed as nothing at all reads as a command that failed. Name the
            // file too: a `when` block that did not fire is the other reason this is empty.
            if listed == 0 {
                println!(
                    "No schedules are in force. {} declares none that apply to this machine.",
                    file.display()
                );
            }
            return Ok(());
        }
    }

    handle_sync(app, SyncMode::default(), Output::Human).await
}

pub async fn handle_activate(app: &App, profiles: &[String], add: bool) -> Result<()> {
    app.profile_manager()
        .activate(profiles, add)
        .await
        .map_err(|e| e.into())
}

pub async fn handle_deactivate(app: &App, profiles: &[String]) -> Result<()> {
    app.profile_manager()
        .deactivate(profiles)
        .await
        .map_err(|e| e.into())
}

pub async fn handle_profile(app: &App, cmd: &ProfileCommand) -> Result<()> {
    let pm = app.profile_manager();
    match cmd {
        ProfileCommand::List => {
            let names = pm.list_profiles().await?;
            let active = pm.active_profiles().await?;
            if names.is_empty() {
                println!("No profiles defined. Create one with `shall profile create <name>`.");
            }
            for n in &names {
                let mark = if active.iter().any(|a| a == n) {
                    "\u{2605}"
                } else {
                    " "
                };
                println!("{} {}", mark, n);
            }
        }
        ProfileCommand::Show { name } => {
            let packages = pm.show(name).await?;
            // A profile that reaches nothing is the commonest thing a new user writes — a
            // `use` line pointing at an empty module — and printing nothing for it says
            // neither "empty" nor "no such profile".
            if packages.is_empty() {
                println!(
                    "`{}` declares no packages. A profile is `use <module>` lines and package \
                     lines; `shall module list` shows what there is to use.",
                    name
                );
            }
            for pkg in packages {
                println!("{}", pkg);
            }
        }
        ProfileCommand::Create { name } => {
            pm.create(name).await?;
            println!("Created profile '{}' at the profiles directory.", name);
        }
        ProfileCommand::Save { name } => {
            pm.save_current_as(name).await?;
        }
        ProfileCommand::Active => {
            let active = pm.active_profiles().await?;
            if active.is_empty() {
                println!("No profiles are currently active.");
            }
            for a in &active {
                println!("{}", a);
            }
        }
    }
    Ok(())
}
