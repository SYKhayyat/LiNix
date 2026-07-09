use anyhow::{Context, Result};
use clap::Parser;
use linix::app::generation::GenerationStore;
use linix::app::sync::planner::{ScopedFilter as PlannerScope, ScopedFilter};
use linix::app::{ui::TuiPreview, App};
use linix::cli::{
    Cli, Commands, ConfigCommand, GenerationCommand, LeaseCommand, ModuleCommand, ProfileCommand,
    RepoCommand, ScheduleCommand, SnapshotCommand,
};
use linix::config::parser::{add_package_to_local, remove_package_from_local};
use std::collections::HashMap;
use std::env;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    // A closed output pipe (e.g. `linix search x | head`) makes `println!` fail with
    // EPIPE, which under `panic = "abort"` becomes a core dump ("Aborted"). Intercept
    // that one panic and exit quietly — the wanted output was already delivered. This
    // leaves SIGPIPE ignored for sockets, so network writes are unaffected.
    let default_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let is_broken_pipe = info.to_string().contains("Broken pipe")
            || info
                .payload()
                .downcast_ref::<String>()
                .is_some_and(|s| s.contains("Broken pipe"))
            || info
                .payload()
                .downcast_ref::<&str>()
                .is_some_and(|s| s.contains("Broken pipe"));
        if is_broken_pipe {
            std::process::exit(0);
        }
        default_panic_hook(info);
    }));

    // 1. Logging Initialization
    // Logs go to STDERR so that stdout carries only machine-readable payloads. Otherwise
    // `INFO` lines are interleaved with `--json` output on stdout, corrupting it for any
    // consumer (`linix search --json | jq`).
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // 2. High-Performance Shim Hijack
    if let Some(res) = attempt_shim_hijack().await? {
        return res;
    }

    // 3. CLI & Config Bootstrap
    let cli = Cli::parse();
    let config = load_and_merge_config(&cli).await?;
    linix::backends::node_registry::set_http_timeout(config.network_timeout_secs);

    // 4. Kernel Initialization
    let app = App::new(config).await?;

    // 5. Command Dispatcher (Modular A+ Routing)
    match &cli.command {
        Commands::Sync { locked, json } => handle_sync(&app, *locked, *json).await,
        Commands::Upgrade {
            profile,
            module,
            group,
            json,
            canary,
            test,
        } => handle_upgrade(&app, profile, module, group, *json, *canary, test).await,
        Commands::Install { packages, json } => handle_install(&app, packages, *json).await,
        Commands::Remove { packages, json } => handle_remove(&app, packages, *json).await,
        Commands::Shell { packages } => handle_shell(&app, packages).await,
        Commands::Module(args) => handle_module(&app, &args.command).await,
        Commands::Lease(args) => handle_lease(&app, &args.command).await,
        Commands::Schedule(args) => handle_schedule(&app, &args.command).await,
        Commands::Snapshot(args) => handle_snapshot(&app, &args.command).await,
        Commands::Generation(args) => handle_generation(&app, &args.command).await,
        Commands::Rollback { id, package } => {
            handle_rollback(&app, id, package.as_deref()).await
        }
        Commands::Repo(args) => handle_repo(&app, &args.command).await,
        Commands::Search { query, json } => handle_search(&app, query, *json).await,
        Commands::List { backend, json } => handle_list(&app, backend.as_deref(), *json).await,
        Commands::Info { package } => handle_info(&app, package).await,
        Commands::Clean => handle_clean(&app).await,
        Commands::Heal => handle_heal(&app).await,
        Commands::Doctor => handle_doctor(&app).await,
        Commands::Migrate => handle_migrate(&app).await,
        Commands::Undo => handle_undo(&app).await,
        Commands::Activate { profiles } => handle_activate(&app, profiles).await,
        Commands::Deactivate { profiles } => handle_deactivate(&app, profiles).await,
        Commands::Profile(args) => handle_profile(&app, &args.command).await,
        Commands::Run { packages, command } => handle_run(&app, packages, command).await,
        Commands::Orphans => handle_orphans(&app).await,
        Commands::Status { json } => handle_status(&app, *json).await,
        Commands::Prune { json } => handle_prune(&app, *json).await,
        Commands::Lock => handle_lock(&app).await,
        Commands::Update => handle_update(&app).await,
        Commands::Unmanaged => handle_unmanaged(&app).await,
        Commands::Teleport { package, to } => handle_teleport(&app, package, to).await,
        Commands::Shim { binary, source } => handle_shim(&app, binary, source).await,
        Commands::Config(args) => handle_config(&app, &args.command).await,
        Commands::Init { force } => handle_init(&app, *force).await,
        Commands::Audit { json } => handle_audit(&app, *json).await,
        Commands::Sbom => handle_sbom(&app).await,
        Commands::Why { package } => handle_why(&app, package).await,
        Commands::Bisect { test, yes } => linix::app::bisect::bisect(&app, test, *yes)
            .await
            .map_err(|e| e.into()),
        Commands::Clone { host, dry_run } => linix::app::fleet::clone(&app, host, *dry_run)
            .await
            .map_err(|e| e.into()),
        Commands::Fleet(args) => linix::app::fleet::fleet(&app, &args.hosts, args.sync)
            .await
            .map_err(|e| e.into()),
        Commands::Policy => handle_policy(&app).await,
        Commands::Completions { shell } => {
            let mut cmd = <Cli as clap::CommandFactory>::command();
            linix::cli::generate_completions(*shell, &mut cmd);
            Ok(())
        }
    }
}

// ============================================================================
// COMMAND HANDLERS
// ============================================================================

async fn handle_sync(app: &App, locked: bool, json: bool) -> Result<()> {
    let engine = app.sync_engine().await;
    if app.journal.lock().await.needs_recovery() {
        warn!("LiNix: Transaction journal indicates previous crash. Healing system integrity...");
        engine.heal().await?;
    }

    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), locked)
            .await;
    let desired = resolver.resolve_desired_state().await?;
    enforce_policy(app, &desired).await?;

    let mut changes = {
        let state_guard = app.state.lock().await;
        // Drift removal during `sync` is opt-in (config `prune_on_sync`, default false).
        // Otherwise `sync` only installs/upgrades; `linix prune` removes drift.
        let planner = linix::app::sync::planner::ChangePlanner::new(
            app.registry.clone(),
            &state_guard,
            &app.config,
        )
        .with_prune(app.config.prune_on_sync);
        planner.plan(&desired, PlannerScope::None).await?
    };

    if changes.is_empty() {
        info!("Success: System matches declarative manifests.");
        return Ok(());
    }

    if !json {
        print_flight_plan(app, &changes);
    }

    // Dry-run is preview-only: never prompt, never mutate. (JSON dry-run emits the report.)
    if app.config.dry_run {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&changes.generate_report())?
            );
        }
        return Ok(());
    }

    // Interactive confirmation — but only with a real terminal. A non-interactive caller
    // (pipe/CI/script) must pass --yes (or --json); otherwise we neither hang on a TUI that
    // can't receive input nor silently apply unconfirmed changes.
    if !app.config.yes && !json {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            anyhow::bail!(
                "Refusing to apply changes without confirmation in a non-interactive shell. \
                 Re-run with --yes to proceed, or --dry-run to preview."
            );
        }
        let mut preview = TuiPreview::new(&changes, HashMap::new());
        if !preview.run()? {
            return Ok(());
        }
        changes = preview.get_filtered_changes();
    }

    engine.sync(changes).await?;
    perform_maintenance(app).await
}

async fn handle_upgrade(
    app: &App,
    profile: &Option<String>,
    module: &Option<String>,
    group: &Option<String>,
    json: bool,
    canary: bool,
    test: &Option<String>,
) -> Result<()> {
    let scope = if let Some(p) = profile {
        ScopedFilter::Profile(p.clone())
    } else if let Some(m) = module {
        ScopedFilter::Module(m.clone())
    } else if let Some(g) = group {
        ScopedFilter::Group(g.clone())
    } else {
        ScopedFilter::None
    };

    if canary {
        return handle_canary(app, scope, test).await;
    }

    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let desired = resolver.resolve_desired_state().await?;
    enforce_policy(app, &desired).await?;

    let changes = {
        let state_guard = app.state.lock().await;
        let planner = linix::app::sync::planner::ChangePlanner::new(
            app.registry.clone(),
            &state_guard,
            &app.config,
        );
        planner.plan(&desired, scope).await?
    };

    if json && app.config.dry_run {
        println!(
            "{}",
            serde_json::to_string_pretty(&changes.generate_report())?
        );
        return Ok(());
    }

    if !json && !changes.is_empty() {
        print_flight_plan(app, &changes);
    }

    // Extra guard before an upgrade that would also remove (drift) packages.
    if app.config.confirm_destructive
        && !app.config.yes
        && !app.config.dry_run
        && changes.total_remove() > 0
    {
        let proceed = dialoguer::Confirm::new()
            .with_prompt(format!(
                "Upgrade will also REMOVE {} package(s). Proceed?",
                changes.total_remove()
            ))
            .default(false)
            .interact()
            .unwrap_or(false);
        if !proceed {
            info!("Upgrade aborted by user.");
            return Ok(());
        }
    }

    if !changes.is_empty() {
        app.sync_engine().await.sync(changes).await?;
        perform_maintenance(app).await?;
    }
    Ok(())
}

async fn handle_install(app: &App, packages: &[String], json: bool) -> Result<()> {
    if json && app.config.dry_run {
        let mut planned = Vec::new();
        for pkg_str in packages {
            for spec in app.resolve_spec(pkg_str).await? {
                planned.push(serde_json::json!({
                    "action": "install", "backend": spec.backend, "name": spec.name,
                }));
            }
        }
        println!("{}", serde_json::to_string_pretty(&planned)?);
        return Ok(());
    }
    for pkg_str in packages {
        let resolved = app.resolve_spec(pkg_str).await?;
        for spec in resolved {
            let b = app.registry.get(&spec.backend).context("Backend offline")?;
            if let Some(inst) = b.as_installable() {
                info!("LiNix: Installing {} via {}...", spec.name, spec.backend);
                inst.install(std::slice::from_ref(&spec), b.sudo_for_write())
                    .await?;
                // Tag as "imperative" so `protect_imperative` can shield it from drift
                // pruning even if it never lands in (or is later removed from) a manifest.
                app.state.lock().await.add(
                    &spec.backend,
                    &spec.name,
                    None,
                    spec.options.clone(),
                    Some("imperative".into()),
                    false,
                );
                let _ = add_package_to_local(&app.config.groups_dir, pkg_str).await;
            }
        }
    }
    app.state.lock().await.save()?;
    perform_maintenance(app).await
}

async fn handle_remove(app: &App, packages: &[String], json: bool) -> Result<()> {
    if json && app.config.dry_run {
        let mut planned = Vec::new();
        for pkg_name in packages {
            for b in app.registry.available() {
                if let Some(q) = b.as_queryable() {
                    if q.info(pkg_name).await?.is_some() {
                        planned.push(serde_json::json!({
                            "action": "remove", "backend": b.name(), "name": pkg_name,
                        }));
                        break;
                    }
                }
            }
        }
        println!("{}", serde_json::to_string_pretty(&planned)?);
        return Ok(());
    }
    // Optional extra guard before destructive removals.
    if app.config.confirm_destructive && !app.config.yes && !app.config.dry_run {
        let proceed = dialoguer::Confirm::new()
            .with_prompt(format!("Remove {} package(s)?", packages.len()))
            .default(false)
            .interact()
            .unwrap_or(false);
        if !proceed {
            info!("Remove aborted by user.");
            return Ok(());
        }
    }
    for pkg_str in packages {
        // Parse the same `backend:name[@opts]` syntax `install` accepts. A recognized
        // backend prefix scopes the removal to that backend; otherwise every backend is
        // searched. The *bare* name is what backends query/remove by — passing the full
        // "backend:name" string here was the bug that made `remove backend:pkg` a no-op.
        let (scoped_backend, bare_name) =
            linix::config::parser::split_removal_target(pkg_str, |b| app.registry.get(b).is_some());

        let mut removed = false;
        for b in app.registry.available() {
            if scoped_backend.as_deref().is_some_and(|sb| sb != b.name()) {
                continue;
            }
            if let Some(inst) = b.as_installable() {
                if let Some(q) = b.as_queryable() {
                    if q.info(&bare_name).await?.is_some() {
                        info!("LiNix: Purging {} from {}...", bare_name, b.name());
                        inst.remove(std::slice::from_ref(&bare_name), b.sudo_for_write())
                            .await?;
                        app.state.lock().await.remove(b.name(), &bare_name);
                        let _ = remove_package_from_local(&app.config.groups_dir, pkg_str).await;
                        removed = true;
                        break;
                    }
                }
            }
        }
        if !removed {
            warn!("LiNix: '{}' is not installed under any managed backend.", pkg_str);
        }
    }
    app.state.lock().await.save()?;
    perform_maintenance(app).await
}

async fn handle_repo(app: &App, cmd: &RepoCommand) -> Result<()> {
    let b_name = match cmd {
        RepoCommand::Add { backend, .. } => backend.clone(),
        RepoCommand::Remove { backend, .. } => backend.clone(),
        RepoCommand::List { backend } => backend.clone(),
    }
    .unwrap_or_else(|| {
        app.config
            .default_backend
            .clone()
            .unwrap_or_else(|| "apt".into())
    });

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
            info!("Repo: Removing {} from {}...", name, b_name);
            mgr.remove_repo(name, b.sudo_for_write()).await?;
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

async fn handle_module(app: &App, cmd: &ModuleCommand) -> Result<()> {
    match cmd {
        ModuleCommand::List => {
            let mut entries = tokio::fs::read_dir(&app.config.modules_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.ends_with(".module.txt") {
                    println!("{}", name.replace(".module.txt", ""));
                }
            }
        }
        ModuleCommand::Show { name } => {
            let path = app.config.modules_dir.join(format!("{}.module.txt", name));
            println!("{}", tokio::fs::read_to_string(path).await?);
        }
        ModuleCommand::Create { name } => {
            let path = app.config.modules_dir.join(format!("{}.module.txt", name));
            tokio::fs::write(&path, format!("# LiNix Module: {}\n", name)).await?;
            info!("Module '{}' created successfully.", name);
        }
    }
    Ok(())
}

async fn handle_lease(app: &App, cmd: &LeaseCommand) -> Result<()> {
    match cmd {
        LeaseCommand::List => {
            let state = app.state.lock().await;
            println!("{:<15} {:<20} {:<20}", "BACKEND", "PACKAGE", "EXPIRATION");
            for pkg in &state.packages {
                if let Some(exp) = pkg.expires_at {
                    // Guard against a corrupt/out-of-range timestamp instead of panicking.
                    match chrono::DateTime::<chrono::Utc>::from_timestamp(exp as i64, 0) {
                        Some(dt) => {
                            println!("{:<15} {:<20} {}", pkg.backend, pkg.name, dt.to_rfc2822())
                        }
                        None => println!(
                            "{:<15} {:<20} <invalid expiry: {}>",
                            pkg.backend, pkg.name, exp
                        ),
                    }
                }
            }
        }
        LeaseCommand::Set { package, duration } => {
            let (b, n) = package
                .split_once(':')
                .context("Input format: backend:package")?;
            app.state.lock().await.update_lease(b, n, duration)?;
            app.state.lock().await.save()?;
        }
    }
    Ok(())
}

async fn handle_schedule(app: &App, cmd: &ScheduleCommand) -> Result<()> {
    match cmd {
        ScheduleCommand::Add {
            name,
            cron,
            command,
            notification,
        } => {
            let mut cfg = (*app.config).clone();
            app.scheduler
                .add_schedule(
                    &app.executor,
                    &mut cfg,
                    name.clone(),
                    cron.clone(),
                    command.clone(),
                    notification.clone(),
                )
                .await?;
        }
        ScheduleCommand::List => {
            for s in &app.config.schedules {
                println!("{:<15} {:<15} {}", s.name, s.cron, s.command);
            }
        }
        ScheduleCommand::Remove { name } => {
            let mut cfg = (*app.config).clone();
            app.scheduler
                .remove_schedule(&app.executor, &mut cfg, name)
                .await?;
        }
    }
    Ok(())
}

async fn handle_snapshot(app: &App, cmd: &SnapshotCommand) -> Result<()> {
    match cmd {
        SnapshotCommand::List => {
            let list = app.snapshot_manager.list_snapshots().await?;
            for s in list {
                println!("{:<15} {}", s.backend, s.id);
            }
        }
        SnapshotCommand::Prune { force } => {
            app.prune_snapshots(*force).await?;
        }
    }
    Ok(())
}

/// Generation store anchored beside the (possibly redirected) state registry.
async fn generation_store(app: &App) -> GenerationStore {
    let dir = {
        let state = app.state.lock().await;
        state
            .path
            .parent()
            .map(|p| p.join("generations"))
            .unwrap_or_else(|| linix::utils::safe_data_dir().join("generations"))
    };
    GenerationStore::new(dir)
}

async fn rollback_to(
    app: &App,
    store: &GenerationStore,
    id: &str,
    package: Option<&str>,
) -> Result<()> {
    let generation = store.load(id).await?;
    let current = {
        let state = app.state.lock().await;
        state.packages.clone()
    };

    // Backend scope comes from the global `--backend` (via effective enabled backends);
    // package scope from `--package`. Neither ⇒ a full rollback.
    let eff = app.config.effective_enabled_backends();
    let backends: Option<&[String]> = if eff.is_empty() { None } else { Some(eff.as_slice()) };
    let full_scope = backends.is_none() && package.is_none();

    let changes = linix::app::generation::plan_rollback(&generation, &current, backends, package);
    if changes.is_empty() {
        println!("System already matches generation {id} for the requested scope.");
        return Ok(());
    }
    let (installs, removes) = (changes.total_install(), changes.total_remove());

    // A full rollback also restores the manifests, so the declarative state matches the
    // system and a later `sync` won't undo the rollback. A scoped rollback is a targeted,
    // imperative fix and leaves manifests untouched.
    if full_scope {
        linix::app::generation::write_manifests_with_backup(
            &generation.manifests,
            &app.config.groups_dir,
        )
        .await?;
    }

    println!(
        "Rolling back to generation {id}: {installs} install/downgrade, {removes} remove{}.",
        if full_scope {
            String::new()
        } else {
            " (scoped)".to_string()
        }
    );
    app.sync_engine().await.sync(changes).await?;
    println!("Rollback complete.");
    Ok(())
}

async fn handle_generation(app: &App, cmd: &GenerationCommand) -> Result<()> {
    let store = generation_store(app).await;
    match cmd {
        GenerationCommand::List => {
            let gens = store.list().await?;
            if gens.is_empty() {
                println!("No generations recorded yet. They are created after each `sync`.");
            }
            for g in gens {
                let pin = if g.pinned { " [pinned]" } else { "" };
                let label = if g.label.is_empty() {
                    String::new()
                } else {
                    format!(" {}", g.label)
                };
                println!(
                    "{}  {}  ({} pkgs){}{}",
                    g.id,
                    g.timestamp,
                    g.packages.len(),
                    label,
                    pin
                );
            }
        }
        GenerationCommand::Rollback { id, package } => {
            rollback_to(app, &store, id, package.as_deref()).await?
        }
        GenerationCommand::Pin { id } => {
            store.set_pinned(id, true).await?;
            println!("Pinned generation {id} (retention will keep it).");
        }
        GenerationCommand::Unpin { id } => {
            store.set_pinned(id, false).await?;
            println!("Unpinned generation {id}.");
        }
    }
    Ok(())
}

async fn handle_rollback(app: &App, id: &str, package: Option<&str>) -> Result<()> {
    let store = generation_store(app).await;
    rollback_to(app, &store, id, package).await
}

async fn handle_shell(app: &App, packages: &[String]) -> Result<()> {
    app.shell().enter(packages).await.map_err(|e| e.into())
}

async fn handle_run(app: &App, packages: &[String], command: &str) -> Result<()> {
    let parts: Vec<&str> = command.split_whitespace().collect();
    let bin = parts.first().unwrap_or(&"");
    let args: Vec<String> = parts.iter().skip(1).map(|s| s.to_string()).collect();
    app.runner()
        .run(packages, bin, &args)
        .await
        .map_err(|e| e.into())
}

async fn handle_migrate(app: &App) -> Result<()> {
    app.migrator().migrate().await.map_err(|e| e.into())
}
async fn handle_undo(app: &App) -> Result<()> {
    app.undo_manager()
        .run_interactive()
        .await
        .map_err(|e| e.into())
}
async fn handle_activate(app: &App, profiles: &[String]) -> Result<()> {
    app.profile_manager()
        .activate(profiles)
        .await
        .map_err(|e| e.into())
}

async fn handle_deactivate(app: &App, profiles: &[String]) -> Result<()> {
    app.profile_manager()
        .deactivate(profiles)
        .await
        .map_err(|e| e.into())
}

async fn handle_profile(app: &App, cmd: &ProfileCommand) -> Result<()> {
    let pm = app.profile_manager();
    match cmd {
        ProfileCommand::List => {
            let names = pm.list_profiles().await?;
            let active = pm.active_profiles().await?;
            if names.is_empty() {
                println!("No profiles defined. Create one with `linix profile create <name>`.");
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
            for pkg in pm.show(name).await? {
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
        ProfileCommand::Switch { name } => {
            pm.switch(name).await?;
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
async fn handle_clean(app: &App) -> Result<()> {
    app.clean_orphans().await?;
    perform_maintenance(app).await
}

/// Non-destructive: report packages the next unscoped sync would prune as drift,
/// without removing anything. `clean` performs the actual deep cleanup.
async fn handle_orphans(app: &App) -> Result<()> {
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let desired = resolver.resolve_desired_state().await?;
    let changes = {
        let state_guard = app.state.lock().await;
        let planner = linix::app::sync::planner::ChangePlanner::new(
            app.registry.clone(),
            &state_guard,
            &app.config,
        );
        planner.plan(&desired, PlannerScope::None).await?
    };
    let report = changes.generate_report();
    if report.remove.is_empty() {
        info!("Orphans: no orphaned/drifted packages detected.");
    } else {
        println!("{:<15} PACKAGE", "BACKEND");
        for entry in &report.remove {
            println!("{:<15} {}", entry.backend, entry.name);
        }
        info!(
            "Orphans: {} package(s) would be removed by `linix sync`/`linix clean`.",
            report.remove.len()
        );
    }
    Ok(())
}

/// Read-only reconciliation report: what `sync` would install, what drift `prune` would
/// remove, and what's installed-but-unmanaged. Changes nothing.
async fn handle_status(app: &App, json: bool) -> Result<()> {
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let desired = resolver.resolve_desired_state().await?;
    let changes = {
        let state_guard = app.state.lock().await;
        // prune=true so drift shows in the report regardless of `prune_on_sync`.
        let planner = linix::app::sync::planner::ChangePlanner::new(
            app.registry.clone(),
            &state_guard,
            &app.config,
        )
        .with_prune(true);
        planner.plan(&desired, PlannerScope::None).await?
    };
    let report = changes.generate_report();
    let unmanaged = app.get_unmanaged_packages().await.unwrap_or_default();

    if json {
        let out = serde_json::json!({
            "to_install": report.install,
            "to_remove": report.remove,
            "unmanaged": unmanaged.iter().map(|p| serde_json::json!({"backend": p.backend, "name": p.name})).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if report.install.is_empty() && report.remove.is_empty() && unmanaged.is_empty() {
        println!(
            "System matches your manifests; nothing to install, no drift, no unmanaged packages."
        );
        return Ok(());
    }
    if !report.install.is_empty() {
        println!("+ to install ({}):", report.install.len());
        for e in &report.install {
            println!(
                "    {}:{}{}",
                e.backend,
                e.name,
                e.version
                    .as_deref()
                    .map(|v| format!(" @ {}", v))
                    .unwrap_or_default()
            );
        }
    }
    if !report.remove.is_empty() {
        println!("- drift / `prune` would remove ({}):", report.remove.len());
        for e in &report.remove {
            println!("    {}:{}", e.backend, e.name);
        }
    }
    if !unmanaged.is_empty() {
        println!(
            "? unmanaged — installed but not in your manifests ({}):",
            unmanaged.len()
        );
        for p in &unmanaged {
            println!("    {}:{}", p.backend, p.name);
        }
    }
    Ok(())
}

/// Remove drift: managed packages no longer present in the manifests. Separate from
/// `sync` so removal is always an explicit action.
async fn handle_prune(app: &App, json: bool) -> Result<()> {
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let desired = resolver.resolve_desired_state().await?;
    let removals = {
        let state_guard = app.state.lock().await;
        let planner = linix::app::sync::planner::ChangePlanner::new(
            app.registry.clone(),
            &state_guard,
            &app.config,
        )
        .with_prune(true);
        let changes = planner.plan(&desired, PlannerScope::None).await?;
        changes.removals_only()
    };

    if removals.is_empty() {
        info!("Prune: no drift packages to remove.");
        return Ok(());
    }

    let report = removals.generate_report();
    if json && app.config.dry_run {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("Prune will remove {} package(s):", report.remove.len());
    for e in &report.remove {
        println!("    {}:{}", e.backend, e.name);
    }

    if !app.config.yes && !app.config.dry_run {
        let proceed = dialoguer::Confirm::new()
            .with_prompt("Proceed with removal?")
            .default(false)
            .interact()
            .unwrap_or(false);
        if !proceed {
            info!("Prune aborted by user.");
            return Ok(());
        }
    }

    app.sync_engine().await.sync(removals).await?;
    perform_maintenance(app).await
}

/// Write the currently-installed version of every managed package to locks.json so a
/// later `sync --locked` reproduces those exact versions (where the backend supports it).
async fn handle_lock(app: &App) -> Result<()> {
    let mut locks = serde_json::Map::new();
    {
        let state = app.state.lock().await;
        for pkg in &state.packages {
            // Prefer the live installed version from the backend; fall back to recorded state.
            let version = match app
                .registry
                .get(&pkg.backend)
                .and_then(|b| b.as_queryable().cloned())
            {
                Some(q) => match q.info(&pkg.name).await {
                    Ok(Some(p)) => p.version.or_else(|| pkg.version.clone()),
                    _ => pkg.version.clone(),
                },
                None => pkg.version.clone(),
            };
            if let Some(v) = version {
                if !v.is_empty() && v != "unknown" {
                    locks.insert(
                        format!("{}:{}", pkg.backend, pkg.name),
                        serde_json::Value::String(v),
                    );
                }
            }
        }
    }

    let count = locks.len();
    let doc = serde_json::json!({ "locks": locks });
    let path = app.config.groups_dir.join("locks.json");
    tokio::fs::create_dir_all(&app.config.groups_dir).await.ok();
    tokio::fs::write(&path, serde_json::to_string_pretty(&doc)?)
        .await
        .with_context(|| format!("Failed to write {}", path.display()))?;
    info!(
        "Lock: pinned {} package version(s) to {}",
        count,
        path.display()
    );
    Ok(())
}

async fn handle_update(app: &App) -> Result<()> {
    app.update().await.map_err(|e| e.into())
}

async fn handle_unmanaged(app: &App) -> Result<()> {
    let pkgs = app.get_unmanaged_packages().await?;
    if pkgs.is_empty() {
        info!("Unmanaged: every installed package is under LiNix management.");
    } else {
        println!("{:<15} PACKAGE", "BACKEND");
        for p in pkgs {
            println!("{:<15} {}", p.backend, p.name);
        }
    }
    Ok(())
}

async fn handle_teleport(app: &App, package: &str, to: &str) -> Result<()> {
    app.teleporter()
        .teleport(package, to)
        .await
        .map_err(|e| e.into())
}

async fn handle_shim(app: &App, binary: &str, source: &str) -> Result<()> {
    app.create_shim(binary, source).await.map_err(|e| e.into())
}

const CONFIG_TEMPLATE: &str = r#"# LiNix configuration file (config.toml)
# Every key is optional; omit a key to use its built-in default.

# Maximum number of packages installed/removed (and searched) in parallel.
max_parallel = 4

# Timeout (seconds) for outbound HTTP search requests (npm/PyPI/marketplace).
network_timeout_secs = 15

# Retention window for `nix-collect-garbage --delete-older-than` during cleanup.
nix_gc_age = "30d"

# Require confirmation before destructive (removal) operations unless `yes = true`.
confirm_destructive = false

# Seconds to cache backend query results.
cache_ttl = 300

# Remove packages found in the bloatware file during sync.
remove_bloatware = false

# Whether `sync` removes drift (packages no longer in your manifests). Default false:
# `sync` only installs/upgrades, and `linix prune` removes drift explicitly. Set true to
# fold pruning back into `sync`.
prune_on_sync = false

# Drift-removal scope for `prune`/`sync`. "managed" (default) only removes packages LiNix
# manages; "system" removes ANY installed package not in your manifests (except protected).
# prune_scope = "managed"

# Never let drift pruning remove packages you installed imperatively (`linix install ...`),
# even if they aren't in a manifest. Safe default: true.
protect_imperative = true

# Default SSH destinations for `linix fleet` when none are given on the command line.
# fleet_hosts = ["user@web-01", "user@web-02"]

# Packages that must never be removed (exact, case-insensitive match).
# protected_packages = ["sudo", "bash", "linix"]

# Force a single default backend (otherwise auto-detected by priority).
# default_backend = "apt"

# Per-backend settings. Example: install flatpaks into the user scope.
# [backend_settings.flatpak]
# user = "true"
"#;

async fn handle_config(app: &App, cmd: &ConfigCommand) -> Result<()> {
    let path = app.config.config_file.clone();
    match cmd {
        ConfigCommand::Path => {
            println!("{}", path.display());
        }
        ConfigCommand::Show => {
            let source = if path.exists() {
                format!("file: {}", path.display())
            } else {
                "built-in defaults".to_string()
            };
            println!("# source: {}", source);
            println!(
                "{}",
                toml::to_string_pretty(&*app.config).context("Failed to serialize config")?
            );
        }
        ConfigCommand::Init { force } => {
            if path.exists() && !force {
                warn!(
                    "Config already exists at {} (use --force to overwrite).",
                    path.display()
                );
                return Ok(());
            }
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
            tokio::fs::write(&path, CONFIG_TEMPLATE)
                .await
                .with_context(|| format!("Failed to write config to {}", path.display()))?;
            info!("Wrote commented default config to {}", path.display());
        }
    }
    Ok(())
}
async fn handle_heal(app: &App) -> Result<()> {
    app.sync_engine().await.heal().await.map_err(|e| e.into())
}

/// Health-gated upgrade: snapshot, upgrade, run the test, roll back automatically on
/// failure so a bad upgrade never leaves the machine broken.
async fn handle_canary(app: &App, scope: ScopedFilter, test: &Option<String>) -> Result<()> {
    let test = test
        .clone()
        .ok_or_else(|| anyhow::anyhow!("--canary requires --test <command> (the health check)"))?;
    if !app.snapshot_manager.has_provider() {
        return Err(anyhow::anyhow!(
            "--canary needs a snapshot provider (btrfs/zfs/timeshift/Windows Restore) to guarantee rollback; none is available"
        ));
    }

    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let desired = resolver.resolve_desired_state().await?;
    enforce_policy(app, &desired).await?;

    let changes = {
        let state_guard = app.state.lock().await;
        let planner = linix::app::sync::planner::ChangePlanner::new(
            app.registry.clone(),
            &state_guard,
            &app.config,
        );
        planner.plan(&desired, scope).await?
    };
    if changes.is_empty() {
        info!("Canary: nothing to upgrade.");
        return Ok(());
    }
    print_flight_plan(app, &changes);

    if app.config.dry_run {
        println!(
            "[dry-run] Would snapshot, upgrade, run `{}`, and roll back on failure.",
            test
        );
        return Ok(());
    }

    let snap = app
        .snapshot_manager
        .auto_snapshot("pre_canary")
        .await?
        .ok_or_else(|| anyhow::anyhow!("failed to create pre-canary snapshot"))?;
    info!("Canary: snapshot {} taken; applying upgrade...", snap.id);
    app.sync_engine().await.sync(changes).await?;

    info!("Canary: running health check: {}", test);
    if linix::app::bisect::run_test(&test).await {
        println!("Canary: health check passed — upgrade kept.");
        perform_maintenance(app).await
    } else {
        warn!(
            "Canary: health check FAILED — rolling back to snapshot {}...",
            snap.id
        );
        app.snapshot_manager.restore_snapshot(&snap.id).await?;
        println!(
            "Canary: rolled back to pre-upgrade snapshot {}. System left unchanged.",
            snap.id
        );
        Ok(())
    }
}

/// Enforce policy.toml (if present) against the desired state before any change. Spec rules
/// are checked purely; snapshot- and vulnerability-based rules use runtime state.
async fn enforce_policy(
    app: &App,
    desired: &HashMap<String, Vec<linix::core::PackageSpec>>,
) -> Result<()> {
    let path = app.config.groups_dir.join("policy.toml");
    let Some(policy) = linix::app::policy::Policy::load(&path).await? else {
        return Ok(());
    };
    if policy.is_empty() {
        return Ok(());
    }
    let mut violations = policy.check_specs(desired);
    if policy.require_snapshot && !app.snapshot_manager.has_provider() {
        violations.push(
            "policy requires a snapshot provider but none is available (require_snapshot)".into(),
        );
    }
    if policy.deny_vulnerable {
        match linix::app::insight::audit(app).await {
            Ok(report) => {
                for f in report.findings {
                    violations.push(format!(
                        "{}:{} — known vulnerability {} (deny_vulnerable)",
                        f.backend, f.name, f.id
                    ));
                }
            }
            Err(e) => warn!("Policy: vulnerability check skipped ({}).", e),
        }
    }
    if violations.is_empty() {
        return Ok(());
    }
    eprintln!("Blocked by policy ({} violation(s)):", violations.len());
    for v in &violations {
        eprintln!("  - {}", v);
    }
    Err(anyhow::anyhow!(
        "policy violations prevent this operation; nothing was changed"
    ))
}

/// A concise pre-flight summary of what a sync/upgrade is about to do. Real download-size
/// and time estimates are backend-specific and deliberately not faked.
fn print_flight_plan(app: &App, changes: &linix::app::sync::planner::SyncChanges) {
    let report = changes.generate_report();
    if report.install.is_empty() && report.remove.is_empty() {
        return;
    }
    let mut backends: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut needs_root = false;
    let mut service_ops = 0;
    for e in report.install.iter().chain(report.remove.iter()) {
        backends.insert(e.backend.clone());
        if let Some(b) = app.registry.get(&e.backend) {
            if b.needs_root() {
                needs_root = true;
            }
        }
        if e.backend == "service" {
            service_ops += 1;
        }
    }
    println!("Flight plan:");
    println!(
        "  install {}   remove {}   (total {} change(s))",
        report.install.len(),
        report.remove.len(),
        report.install.len() + report.remove.len()
    );
    println!(
        "  backends: {}",
        backends.into_iter().collect::<Vec<_>>().join(", ")
    );
    if needs_root {
        println!("  privileges: some operations require root/sudo");
    }
    if service_ops > 0 {
        println!(
            "  services: {} change(s) may restart running services",
            service_ops
        );
    }
}

/// `linix policy` — report whether the desired state complies with policy.toml.
async fn handle_policy(app: &App) -> Result<()> {
    let path = app.config.groups_dir.join("policy.toml");
    let Some(policy) = linix::app::policy::Policy::load(&path).await? else {
        println!("No policy.toml at {} — no rules in effect.", path.display());
        return Ok(());
    };
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let desired = resolver.resolve_desired_state().await?;
    let mut violations = policy.check_specs(&desired);
    if policy.require_snapshot && !app.snapshot_manager.has_provider() {
        violations.push("policy requires a snapshot provider but none is available".into());
    }
    if violations.is_empty() {
        println!("Policy check passed — the desired state is compliant.");
        if policy.deny_vulnerable {
            println!("(deny_vulnerable is also enforced at sync time via `linix audit`.)");
        }
    } else {
        println!("Policy violations ({}):", violations.len());
        for v in &violations {
            println!("  - {}", v);
        }
    }
    Ok(())
}

async fn handle_audit(app: &App, json: bool) -> Result<()> {
    let report = linix::app::insight::audit(app).await?;
    linix::app::insight::print_audit(&report, json).map_err(|e| e.into())
}

async fn handle_sbom(app: &App) -> Result<()> {
    println!("{}", linix::app::insight::sbom(app).await?);
    Ok(())
}

async fn handle_why(app: &App, package: &str) -> Result<()> {
    linix::app::insight::why(app, package)
        .await
        .map_err(|e| e.into())
}

/// Scaffold the on-disk layout LiNix expects and drop a starter manifest so a fresh
/// machine (or a freshly-cloned checkout) is immediately usable.
async fn handle_init(app: &App, force: bool) -> Result<()> {
    let cfg = &app.config;
    let dirs: [(&str, &std::path::Path); 6] = [
        ("groups", &cfg.groups_dir),
        ("modules", &cfg.modules_dir),
        ("tmp", &cfg.tmp_dir),
        ("github", &cfg.github_dir),
        ("web", &cfg.web_dir),
        ("appimages", &cfg.appimage_dir),
    ];
    println!("Scaffolding LiNix directories:");
    for (label, path) in dirs {
        tokio::fs::create_dir_all(path)
            .await
            .with_context(|| format!("Failed to create {} directory {}", label, path.display()))?;
        println!("  created  {:<10} {}", label, path.display());
    }

    let local = cfg.groups_dir.join("local.txt");
    if !local.exists() || force {
        let starter = "# LiNix manifest — one package per line. Examples:\n\
                       #   ripgrep                     (auto-detect backend by priority)\n\
                       #   cargo:exa                   (force a backend)\n\
                       #   npm:typescript@version=>5.0.0\n\
                       #   github:BurntSushi/ripgrep\n";
        tokio::fs::write(&local, starter)
            .await
            .with_context(|| format!("Failed to write starter manifest {}", local.display()))?;
        println!("  created  {:<10} {}", "manifest", local.display());
    } else {
        println!(
            "  kept     {:<10} {} (exists; use --force to reset)",
            "manifest",
            local.display()
        );
    }

    println!("\nReady. Edit {} then run `linix sync`.", local.display());
    println!("(Run `linix config init` to also write a commented config.toml.)");
    Ok(())
}
/// Render a package as one aligned row: backend, name, version.
fn print_package_row(p: &linix::core::Package) {
    println!(
        "{:<12} {:<32} {}",
        p.backend,
        p.name,
        p.version.as_deref().unwrap_or("")
    );
}

async fn handle_search(app: &App, query: &str, json: bool) -> Result<()> {
    let results = app.search(query).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        for p in &results {
            print_package_row(p);
        }
    }
    Ok(())
}

async fn handle_list(app: &App, backend: Option<&str>, json: bool) -> Result<()> {
    let list = app.list(backend).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&list)?);
    } else {
        for p in &list {
            print_package_row(p);
        }
    }
    Ok(())
}

async fn handle_info(app: &App, package: &str) -> Result<()> {
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
async fn handle_doctor(app: &App) -> Result<()> {
    for b in app.registry.all() {
        println!(
            "[{}] {}",
            if b.is_available() { "READY" } else { "OFFLINE" },
            b.name()
        );
    }
    Ok(())
}

// ============================================================================
// KERNEL HELPERS
// ============================================================================

async fn attempt_shim_hijack() -> Result<Option<Result<()>>> {
    let current_name = env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "linix".to_string());
    if current_name != "linix" && !current_name.starts_with("linix") {
        let config =
            linix::config::Config::from_file(&linix::utils::safe_config_dir().join("config.toml"))
                .unwrap_or_default();
        let app = App::new(config).await?;
        return Ok(Some(
            app.runner()
                .exec_shim(&current_name, &env::args().collect::<Vec<_>>()[1..])
                .await
                .map_err(|e| e.into()),
        ));
    }
    Ok(None)
}

async fn load_and_merge_config(cli: &Cli) -> Result<linix::config::Config> {
    let path = cli
        .config
        .clone()
        .unwrap_or_else(|| linix::utils::safe_config_dir().join("config.toml"));
    let mut config =
        tokio::task::spawn_blocking(move || linix::config::Config::from_file(&path)).await??;
    config.merge_cli_overrides(
        Some(cli.dry_run),
        Some(cli.yes),
        cli.backend.clone(),
        None,
        cli.groups_dir.clone(),
        Some(cli.verbose),
    );
    Ok(config)
}

async fn perform_maintenance(app: &App) -> Result<()> {
    app.journal.lock().await.cleanup()?;
    // Reclaim expired temporary installs so leases are enforced on every state-changing run.
    if let Err(e) = app.sweep_expired_leases().await {
        warn!("Maintenance: lease sweep failed: {}", e);
    }
    if app.config.snapshots.auto_prune {
        app.prune_snapshots(false).await?;
    }
    Ok(())
}
