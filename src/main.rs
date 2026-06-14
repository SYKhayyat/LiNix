use anyhow::{Context, Result};
use clap::Parser;
use linix::app::{App, SyncEngine, ui::TuiPreview};
use linix::cli::{
    Cli, Commands, ModuleCommand, SnapshotCommand, 
    LeaseCommand, ScheduleCommand, RepoCommand
};
use linix::app::sync::planner::{ScopedFilter as PlannerScope, ScopedFilter};
use linix::config::parser::{add_package_to_local, remove_package_from_local};
use tracing::{info, warn}; 
use tracing_subscriber::EnvFilter;
use std::env;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Initialize Logging context
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    // 2. High-Performance Shim Hijack
    if let Some(res) = attempt_shim_hijack().await? {
        return res;
    }

    // 3. Load CLI and Configuration
    let cli = Cli::parse();
    // Modernized: config is immutable here as merging occurs inside the loader
    let config = load_and_merge_config(&cli).await?;

    // 4. Initialize Kernel
    let app = App::new(config).await?;

    // 5. Command Routing (A+ Modular Dispatcher)
    match &cli.command {
        Commands::Sync { locked, json } => handle_sync(&app, *locked, *json).await,
        Commands::Upgrade { profile, module, group, json } => handle_upgrade(&app, profile, module, group, *json).await,
        Commands::Install { packages, json } => handle_install(&app, packages, *json).await,
        Commands::Remove { packages, json } => handle_remove(&app, packages, *json).await,
        Commands::Shell { packages } => handle_shell(&app, packages).await,
        Commands::Module(args) => handle_module(&app, &args.command).await,
        Commands::Lease(args) => handle_lease(&app, &args.command).await,
        Commands::Schedule(args) => handle_schedule(&app, &args.command).await,
        Commands::Snapshot(args) => handle_snapshot(&app, &args.command).await,
        Commands::Repo(args) => handle_repo(&app, &args.command).await,
        Commands::Search { query } => handle_search(&app, query).await,
        Commands::List { backend, json } => handle_list(&app, backend.as_deref(), *json).await,
        Commands::Info { package } => handle_info(&app, package).await,
        Commands::Clean => handle_clean(&app).await,
        Commands::Heal => handle_heal(&app).await,
        Commands::Doctor => handle_doctor(&app).await,
        Commands::Migrate => handle_migrate(&app).await,
        Commands::Undo => handle_undo(&app).await,
        Commands::Profile { name } => handle_profile(&app, name).await,
        Commands::Run { packages, command } => handle_run(&app, packages, command).await,
        Commands::Orphans => handle_clean(&app).await,
        _ => { warn!("Requested CLI variant is not implemented in this LiNix version."); Ok(()) }
    }
}

// ============================================================================
// MODULAR COMMAND HANDLERS
// ============================================================================

async fn handle_sync(app: &App, locked: bool, json: bool) -> Result<()> {
    let engine = create_sync_engine(app).await;
    if app.journal.lock().await.needs_recovery() {
        warn!("LiNix: Transaction journal indicates a previous crash. Healing system...");
        engine.heal().await?;
    }

    let resolver = linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), locked).await;
    let desired = resolver.resolve_desired_state().await?;
    
    let mut changes = {
        let state_guard = app.state.lock().await;
        let planner = linix::app::sync::planner::ChangePlanner::new(app.registry.clone(), &state_guard, &app.config);
        planner.plan(&desired, PlannerScope::None).await?
    };

    if changes.is_empty() {
        info!("LiNix: System is consistent with declarative configuration.");
        return Ok(());
    }

    if json && app.config.dry_run {
        println!("{}", serde_json::to_string_pretty(&changes.generate_report())?);
        return Ok(());
    }

    if !app.config.yes && !json {
        let mut preview = TuiPreview::new(&changes, HashMap::new());
        if !preview.run()? { return Ok(()); }
        changes = preview.get_filtered_changes();
    }

    engine.sync(changes).await?;
    perform_maintenance(app).await
}

async fn handle_repo(app: &App, cmd: &RepoCommand) -> Result<()> {
    let backend_name = match cmd {
        RepoCommand::Add { backend, .. } => backend.clone(),
        RepoCommand::Remove { backend, .. } => backend.clone(),
        RepoCommand::List { backend } => backend.clone(),
    }.unwrap_or_else(|| app.config.default_backend.clone().unwrap_or_else(|| "apt".into()));

    let b_cap = app.registry.get(&backend_name).context("Backend offline")?;
    let manager = b_cap.as_repo_manager().context("Backend lacks repo management capability")?;

    match cmd {
        RepoCommand::Add { name, url, .. } => {
            info!("Repo: Adding {} to {}...", name, backend_name);
            manager.add_repo(name, url, b_cap.needs_root()).await?;
        }
        RepoCommand::Remove { name, .. } => {
            info!("Repo: Removing {} from {}...", name, backend_name);
            manager.remove_repo(name, b_cap.needs_root()).await?;
        }
        RepoCommand::List { .. } => {
            let repos = manager.list_repos().await?;
            for (id, url) in repos { println!("{:<20} {}", id, url); }
        }
    }
    Ok(())
}

async fn handle_upgrade(app: &App, profile: &Option<String>, module: &Option<String>, group: &Option<String>, json: bool) -> Result<()> {
    let scope = if let Some(p) = profile { ScopedFilter::Profile(p.clone()) }
                else if let Some(m) = module { ScopedFilter::Module(m.clone()) }
                else if let Some(g) = group { ScopedFilter::Group(g.clone()) }
                else { ScopedFilter::None };

    let resolver = linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false).await;
    let desired = resolver.resolve_desired_state().await?;
    
    let changes = {
        let state_guard = app.state.lock().await;
        let planner = linix::app::sync::planner::ChangePlanner::new(app.registry.clone(), &state_guard, &app.config);
        planner.plan(&desired, scope).await?
    };

    if json && app.config.dry_run {
        println!("{}", serde_json::to_string_pretty(&changes.generate_report())?);
        return Ok(());
    }

    if !changes.is_empty() {
        create_sync_engine(app).await.sync(changes).await?;
        perform_maintenance(app).await?;
    }
    Ok(())
}

async fn handle_install(app: &App, packages: &[String], json: bool) -> Result<()> {
    if json && app.config.dry_run { return Ok(()); }
    for pkg_str in packages {
        let resolved = app.resolve_spec(pkg_str).await?;
        for spec in resolved {
            let b = app.registry.get(&spec.backend).context("Backend offline")?;
            if let Some(inst) = b.as_installable() {
                info!("LiNix: Installing {}...", spec.name);
                inst.install(&[spec.clone()], b.needs_root()).await?;
                app.state.lock().await.add(&spec.backend, &spec.name, None, spec.options.clone(), None, false);
                let _ = add_package_to_local(&app.config.groups_dir, pkg_str).await;
            }
        }
    }
    app.state.lock().await.save()?;
    perform_maintenance(app).await
}

async fn handle_remove(app: &App, packages: &[String], _json: bool) -> Result<()> {
    for pkg_name in packages {
        let mut removed = false;
        for b in app.registry.available() {
            if let Some(inst) = b.as_installable() {
                if let Some(q) = b.as_queryable() {
                    if q.info(pkg_name).await?.is_some() {
                        info!("LiNix: Purging {}...", pkg_name);
                        inst.remove(&[pkg_name.clone()], b.needs_root()).await?;
                        app.state.lock().await.remove(b.name(), pkg_name);
                        let _ = remove_package_from_local(&app.config.groups_dir, pkg_name).await;
                        removed = true; break;
                    }
                }
            }
        }
        if !removed { warn!("LiNix: '{}' is not under active management.", pkg_name); }
    }
    app.state.lock().await.save()?;
    perform_maintenance(app).await
}

async fn handle_run(app: &App, packages: &[String], command: &String) -> Result<()> {
    let parts: Vec<&str> = command.split_whitespace().collect();
    let bin = parts.first().context("Run: Empty command.")?;
    let args: Vec<String> = parts.iter().skip(1).map(|s| s.to_string()).collect();
    app.runner().run(packages, bin, &args).await.map_err(|e| e.into())
}

async fn handle_module(app: &App, cmd: &ModuleCommand) -> Result<()> {
    match cmd {
        ModuleCommand::List => {
            let mut entries = tokio::fs::read_dir(&app.config.modules_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.ends_with(".module.txt") { println!("{}", name.replace(".module.txt", "")); }
            }
        }
        ModuleCommand::Show { name } => {
            let path = app.config.modules_dir.join(format!("{}.module.txt", name));
            println!("{}", tokio::fs::read_to_string(path).await?);
        }
        ModuleCommand::Create { name } => {
            let path = app.config.modules_dir.join(format!("{}.module.txt", name));
            tokio::fs::write(&path, "# New LiNix Module\n").await?;
            info!("Module '{}' created successfully.", name);
        }
    }
    Ok(())
}

async fn handle_lease(app: &App, cmd: &LeaseCommand) -> Result<()> {
    match cmd {
        LeaseCommand::List => {
            let state = app.state.lock().await;
            for pkg in &state.packages {
                if let Some(exp) = pkg.expires_at {
                    let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(exp as i64, 0).unwrap();
                    println!("{:<20} {}", pkg.name, dt.to_rfc2822());
                }
            }
        }
        LeaseCommand::Set { package, duration } => {
            let (b, n) = package.split_once(':').context("Use format backend:package")?;
            app.state.lock().await.update_lease(b, n, duration)?;
            app.state.lock().await.save()?;
        }
    }
    Ok(())
}

async fn handle_schedule(app: &App, cmd: &ScheduleCommand) -> Result<()> {
    match cmd {
        ScheduleCommand::Add { name, cron, command, notification } => {
            let mut cfg = (*app.config).clone();
            app.scheduler.add_schedule(&mut cfg, name.clone(), cron.clone(), command.clone(), notification.clone()).await?;
        }
        ScheduleCommand::List => {
            for s in &app.config.schedules { println!("{:<15} {:<15} {}", s.name, s.cron, s.command); }
        }
        ScheduleCommand::Remove { name } => {
            let mut cfg = (*app.config).clone();
            app.scheduler.remove_schedule(&mut cfg, name).await?;
        }
    }
    Ok(())
}

async fn handle_snapshot(app: &App, cmd: &SnapshotCommand) -> Result<()> {
    match cmd {
        SnapshotCommand::List => {
            for s in app.snapshot_manager.list_snapshots().await? { println!("{:<15} {}", s.backend, s.id); }
        }
        SnapshotCommand::Prune { force } => { app.prune_snapshots(*force).await?; }
    }
    Ok(())
}

async fn handle_shell(app: &App, packages: &[String]) -> Result<()> {
    app.shell().enter(packages).await.map_err(|e| e.into())
}

async fn handle_search(app: &App, query: &str) -> Result<()> {
    let res = app.search(query).await?;
    for p in res { println!("{:<15} {}", p.backend, p.name); }
    Ok(())
}

async fn handle_list(app: &App, backend: Option<&str>, json: bool) -> Result<()> {
    let list = app.list(backend).await?;
    if json { println!("{}", serde_json::to_string_pretty(&list)?); }
    else { for p in list { println!("{:<15} {}", p.backend, p.name); } }
    Ok(())
}

async fn handle_info(app: &App, package: &str) -> Result<()> {
    if let Some(p) = app.get_info(package).await? { println!("Package: {}\nBackend: {}", p.name, p.backend); }
    Ok(())
}

async fn handle_clean(app: &App) -> Result<()> {
    app.clean_orphans().await?;
    perform_maintenance(app).await
}

async fn handle_heal(app: &App) -> Result<()> {
    create_sync_engine(app).await.heal().await.map_err(|e| e.into())
}

async fn handle_doctor(app: &App) -> Result<()> {
    for b in app.registry.all() {
        let status = if b.is_available() { "READY" } else { "OFFLINE" };
        println!("[{}] {}", status, b.name());
    }
    Ok(())
}

async fn handle_migrate(app: &App) -> Result<()> {
    app.migrator().migrate().await.map_err(|e| e.into())
}

async fn handle_undo(app: &App) -> Result<()> {
    app.undo_manager().run_interactive().await.map_err(|e| e.into())
}

async fn handle_profile(app: &App, name: &str) -> Result<()> {
    app.profile_manager().switch(name).await.map_err(|e| e.into())
}

// ============================================================================
// KERNEL HELPERS
// ============================================================================

async fn attempt_shim_hijack() -> Result<Option<Result<()>>> {
    let current_name = env::current_exe().ok().and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned())).unwrap_or_else(|| "linix".to_string());
    if current_name != "linix" && !current_name.starts_with("linix") {
        let path = linix::utils::safe_config_dir().join("config.toml");
        let config = tokio::task::spawn_blocking(move || linix::config::Config::from_file(&path)).await??;
        let app = App::new(config).await?;
        return Ok(Some(app.runner().exec_shim(&current_name, &env::args().collect::<Vec<_>>()[1..]).await.map_err(|e| e.into())));
    }
    Ok(None)
}

async fn load_and_merge_config(cli: &Cli) -> Result<linix::config::Config> {
    let path = cli.config.clone().unwrap_or_else(|| linix::utils::safe_config_dir().join("config.toml"));
    let mut config = tokio::task::spawn_blocking(move || linix::config::Config::from_file(&path)).await??;
    config.merge_cli_overrides(Some(cli.dry_run), Some(cli.yes), cli.backend.clone(), None, cli.groups_dir.clone(), Some(cli.verbose));
    Ok(config)
}

async fn create_sync_engine(app: &App) -> SyncEngine<'_> {
    // Resolves E0061: Passing the 10th argument (diagnostics)
    SyncEngine::new(
        &app.config, 
        app.registry.clone(), 
        app.executor.duplicate(), 
        app.metrics.clone(), 
        app.progress.clone(), 
        app.hooks.clone(), 
        app.snapshot_manager.clone(), 
        app.journal.clone(), 
        app.state.clone(),
        app.diagnostics.clone() // Correctly provided
    ).await
}

async fn perform_maintenance(app: &App) -> Result<()> {
    let mut journal = app.journal.lock().await;
    let _ = journal.cleanup();
    if app.config.snapshots.auto_prune { app.prune_snapshots(false).await?; }
    Ok(())
}