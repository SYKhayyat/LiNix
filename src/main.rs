use anyhow::{Context, Result};
use clap::Parser;
use linix::app::{
    App, SyncEngine, Runner, ui::TuiPreview, 
};
use linix::cli::{Cli, Commands, RepoCommand};
use linix::config::parser::{add_package_to_local, remove_package_from_local};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use std::env;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<()> {
    // 1. HIGH-PERFORMANCE RUST SHIM HIJACK
    // Detect if we are being called via a symlink or a renamed binary (shim mode)
    let args_raw: Vec<String> = env::args().collect();
    let bin_path = env::current_exe().ok();
    let current_bin_name = bin_path
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "linix".to_string());

    if current_bin_name != "linix" && !current_bin_name.starts_with("linix") {
        // We are in shim mode. Load config and delegate to Runner.
        // Config::from_file is sync; wrap in spawn_blocking for async integrity.
        let config_path = dirs::config_dir().unwrap_or_default().join("linix").join("config.toml");
        let config = tokio::task::spawn_blocking(move || {
            linix::config::Config::from_file(&config_path)
        }).await.map_err(|e| anyhow::anyhow!(e))?.unwrap_or_default();

        let app = App::new(config).await?;
        let runner = Runner::new(&app);
        return runner.exec_shim(&current_bin_name, &args_raw[1..].to_vec()).await.map_err(|e| e.into());
    }

    // 2. STANDARD CLI MODE
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let cli = Cli::parse();
    let config_path = cli.config.clone().unwrap_or_else(|| {
        dirs::config_dir().unwrap_or_default().join("linix").join("config.toml")
    });

    // Load config with blocking isolation
    let mut config = tokio::task::spawn_blocking(move || {
        linix::config::Config::from_file(&config_path)
    }).await.map_err(|e| anyhow::anyhow!(e))?
      .context("CRITICAL: Failed to load LiNix configuration file.")?;
    
    config.merge_cli_overrides(
        Some(cli.dry_run), Some(cli.yes), cli.backend.clone(), 
        None, cli.groups_dir.clone(), Some(cli.verbose)
    );

    let app = App::new(config).await?;

    match &cli.command {
        Commands::Sync { locked: _ } => {
            // Phase 3.2: SyncEngine::new is now async
            let engine = SyncEngine::new(
                &app.config, 
                app.registry.clone(), 
                app.executor.duplicate(), 
                app.metrics.clone(), 
                app.progress.clone(), 
                app.hooks.clone(), 
                app.snapshot_manager.clone(),
                app.journal.clone()
            ).await;

            if app.journal.lock().await.needs_recovery() {
                warn!("LiNix detected an incomplete transaction. Recovering system integrity...");
                engine.heal().await?;
            }

            let resolver = linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone());
            let desired = resolver.resolve_desired_state().await?;
            
            let mut changes = {
                let state_guard = app.state.lock().await;
                let planner = linix::app::sync::planner::ChangePlanner::new(app.registry.clone(), &state_guard, &app.config);
                planner.plan(&desired).await?
            };

            if changes.is_empty() {
                info!("Success: System matches declarative configuration.");
                return Ok(());
            }

            if !app.config.yes {
                // TUI runs on main thread; it blocks, but since we are at a decision point, this is expected.
                let mut preview = TuiPreview::new(&changes, HashMap::new());
                if !preview.run()? {
                    info!("Sync cancelled by user.");
                    return Ok(());
                }
                changes = preview.get_filtered_changes();
            }

            engine.sync(changes).await.map_err(|e| e.into())
        }

        Commands::Install { packages } => {
            for pkg_str in packages {
                let resolved_specs = app.resolve_spec(pkg_str).await?;
                for spec in resolved_specs {
                    let backend_cap = app.registry.get(&spec.backend)
                        .context(format!("Backend '{}' not found.", spec.backend))?;
                    
                    if let Some(installer) = backend_cap.as_installable() {
                        info!("Installing {} via {}...", spec.name, spec.backend);
                        // Phase 2.2: Respect backend root requirements
                        let sudo = backend_cap.needs_root();
                        installer.install(&[spec.clone()], sudo).await?;
                        
                        let mut state_guard = app.state.lock().await;
                        state_guard.add_simple(&spec.backend, &spec.name, None);
                        
                        // Phase 3.2: add_package_to_local is now async
                        if let Err(e) = add_package_to_local(&app.config.groups_dir, pkg_str).await {
                            warn!("Auto-Commit failed for {}: {}", spec.name, e);
                        }
                    }
                }
            }
            // Persistence is blocking; isolate
            let state_final = app.state.lock().await.clone();
            tokio::task::spawn_blocking(move || state_final.save()).await??;
            Ok(())
        }

        Commands::Remove { packages } => {
            for pkg_name in packages {
                let mut found = false;
                for backend in app.registry.available() {
                    if let Some(queryable) = backend.as_queryable() {
                        if queryable.info(pkg_name).await?.is_some() {
                            if let Some(installer) = backend.as_installable() {
                                info!("Removing {} from {}...", pkg_name, backend.name());
                                // Phase 2.2: Respect backend root requirements
                                let sudo = backend.needs_root();
                                installer.remove(&[pkg_name.clone()], sudo).await?;
                                
                                let mut state_guard = app.state.lock().await;
                                state_guard.remove(backend.name(), pkg_name);
                                
                                // Phase 3.2: remove_package_from_local is now async
                                let _ = remove_package_from_local(&app.config.groups_dir, pkg_name).await;
                                found = true;
                                break;
                            }
                        }
                    }
                }
                if !found { warn!("Target '{}' is not currently installed.", pkg_name); }
            }
            let state_final = app.state.lock().await.clone();
            tokio::task::spawn_blocking(move || state_final.save()).await??;
            Ok(())
        }

        Commands::Migrate => app.migrator().migrate().await.map_err(|e| e.into()),

        Commands::Teleport { package, to } => app.teleporter().teleport(package, to).await.map_err(|e| e.into()),

        Commands::Shell { packages } => {
            let shell = app.shell();
            if packages.is_empty() {
                shell.auto_shell().await.map_err(|e| e.into())
            } else {
                shell.enter(packages).await.map_err(|e| e.into())
            }
        }

        Commands::Undo => app.undo_manager().run_interactive().await.map_err(|e| e.into()),

        Commands::Profile { name } => app.profile_manager().switch(name).await.map_err(|e| e.into()),

        Commands::Repo(args) => {
            match &args.command {
                RepoCommand::Add { name, url, backend } => {
                    let b_name = backend.as_deref().unwrap_or("apt");
                    let b = app.registry.get(b_name).context("Backend not found")?;
                    let manager = b.as_repo_manager().context("Backend does not support repositories.")?;
                    // Phase 2.2: Sudo precision
                    let sudo = b.needs_root();
                    manager.add_repo(name, url, sudo).await?;
                    info!("Successfully added repository: {}", name);
                    Ok(())
                }
                RepoCommand::Remove { name, backend } => {
                    let b_name = backend.as_deref().unwrap_or("apt");
                    let b = app.registry.get(b_name).context("Backend not found")?;
                    let manager = b.as_repo_manager().context("Backend does not support repositories.")?;
                    let sudo = b.needs_root();
                    manager.remove_repo(name, sudo).await?;
                    info!("Successfully removed repository: {}", name);
                    Ok(())
                }
                RepoCommand::List { backend } => {
                    let b_name = backend.as_deref().unwrap_or("apt");
                    let b = app.registry.get(b_name).context("Backend not found")?;
                    let manager = b.as_repo_manager().context("Backend does not support repositories.")?;
                    let repos = manager.list_repos().await?;
                    println!("{:<20} {}", "NAME", "URL");
                    for (n, u) in repos { println!("{:<20} {}", n, u); }
                    Ok(())
                }
            }
        }

        Commands::Update => app.update().await.map_err(|e| e.into()),
        Commands::Upgrade => app.upgrade().await.map_err(|e| e.into()),
        Commands::Orphans => app.clean_orphans().await.map_err(|e| e.into()),
        Commands::Clean => app.clean_orphans().await.map_err(|e| e.into()),
        Commands::Heal => {
            let engine = SyncEngine::new(
                &app.config, app.registry.clone(), app.executor.duplicate(), 
                app.metrics.clone(), app.progress.clone(), 
                app.hooks.clone(), app.snapshot_manager.clone(),
                app.journal.clone()
            ).await;
            engine.heal().await.map_err(|e| e.into())
        }
        Commands::Run { packages, command } => {
            let runner = Runner::new(&app);
            let (cmd_bin, args) = if let Some((c, a)) = linix::utils::command::split_command(command) {
                (c, a)
            } else {
                (command.clone(), vec![])
            };
            runner.run(packages, &cmd_bin, &args).await.map_err(|e| e.into())
        }
        Commands::Unmanaged => {
            let pkgs = app.get_unmanaged_packages().await?;
            for p in pkgs { println!("{:<15} {:<30}", p.backend, p.name); }
            Ok(())
        }
        Commands::List { backend } => {
            let pkgs = app.list(backend.as_deref()).await?;
            for p in pkgs { println!("{:<15} {:<30}", p.backend, p.name); }
            Ok(())
        }
        Commands::Info { package } => {
            if let Some(p) = app.get_info(package).await? {
                println!("Package:     {}\nBackend:     {}", p.name, p.backend);
            }
            Ok(())
        }
        Commands::Search { query } => {
            let results = app.search(query).await?;
            for p in results { println!("{:<15} {:<40}", p.backend, p.name); }
            Ok(())
        }
        Commands::Doctor => {
            for backend in app.registry.all() {
                let status = if backend.is_available() { "READY" } else { "OFFLINE" };
                let root_req = if backend.needs_root() { "ROOT" } else { "USER" };
                println!("[{}] [{}] {:<15}", status, root_req, backend.name());
            }
            Ok(())
        }
        Commands::Shim { binary, source } => app.create_shim(binary, source).await.map_err(|e| e.into()),
    }
}