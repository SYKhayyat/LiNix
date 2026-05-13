use anyhow::{Context, Result};
use clap::Parser;
use linix::app::{
    App, SyncEngine, Runner, TuiPreview, Migrator, 
    Teleporter, UndoManager, ProfileManager, GhostShell
};
use linix::cli::{Cli, Commands, RepoCommand};
use linix::core::transaction::GraphAction;
use linix::core::{PackageSpec, Transaction};
use linix::config::parser::{add_package_to_local, remove_package_from_local};
use tracing::{info, warn, error, debug};
use tracing_subscriber::EnvFilter;
use std::env;
use std::collections::HashMap;
use petgraph::stable_graph::StableDiGraph;

#[tokio::main]
async fn main() -> Result<()> {
    // --- POINT 6: HIGH-PERFORMANCE RUST SHIM HIJACK ---
    // Detects if LiNix is being invoked as a symlink/hardlink for a specific tool.
    let args_raw: Vec<String> = env::args().collect();
    let bin_path = env::current_exe().ok();
    let current_bin_name = bin_path
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "linix".to_string());

    // If the name is not 'linix', we are in Shim Proxy Mode.
    if current_bin_name != "linix" && !current_bin_name.starts_with("linix") {
        let config = linix::config::Config::from_file(
            &dirs::config_dir().unwrap_or_default().join("linix").join("config.toml")
        ).unwrap_or_default();
        let app = App::new(config).await?;
        let runner = Runner::new(&app);
        // Point 6/16: Execute the tool environment with zero-cost overhead
        return runner.exec_shim(&current_bin_name, &args_raw[1..].to_vec()).await;
    }

    // --- STANDARD CLI MODE ---

    // 1. Initialize Logging and Telemetry
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    // 2. Parse CLI Arguments
    let cli = Cli::parse();

    // 3. Load Configuration
    let config_path = cli.config.clone().unwrap_or_else(|| {
        dirs::config_dir().unwrap_or_default().join("linix").join("config.toml")
    });

    let mut config = linix::config::Config::from_file(&config_path)
        .context("CRITICAL: Failed to load LiNix configuration file.")?;
    
    // Merge CLI overrides
    config.merge_cli_overrides(
        Some(cli.dry_run), 
        Some(cli.yes), 
        cli.backend.clone(), 
        None, 
        cli.groups_dir.clone(), 
        Some(cli.verbose)
    );

    // 4. Initialize Application Kernel (Dependency Injection)
    let app = App::new(config).await?;

    match &cli.command {
        // --- CORE SYNC (DAG-BASED + TUI) ---
        // Points: 1, 2.2, 4, 8, 9, 10, 11, 13, 15
        Commands::Sync { locked: _ } => {
            let engine = SyncEngine::new(
                &app.config, app.registry.clone(), app.executor.clone(), 
                app.metrics.clone(), app.progress.clone(), 
                app.hooks.clone(), app.snapshot_manager.clone(),
                app.journal.clone()
            );

            // Point 8: Crash Recovery via WAL
            if app.journal.lock().await.needs_recovery() {
                warn!("LiNix detected an incomplete transaction. Recovering system integrity...");
                engine.heal().await?;
            }

            // Phase 2.2: Planning logic
            let state_guard = app.state.lock().await;
            let resolver = linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone());
            let planner = linix::app::sync::planner::ChangePlanner::new(app.registry.clone(), &state_guard);
            
            let desired = resolver.resolve_desired_state().await?;
            let mut changes = planner.plan(&desired).await?;
            drop(state_guard); // Release state lock for UI interaction

            if changes.is_empty() {
                info!("Success: System matches declarative configuration.");
                return Ok(());
            }

            // Point 13: Interactive TUI Preview
            if !app.config.yes {
                let mut preview = TuiPreview::new(&changes);
                if !preview.run()? {
                    info!("Sync cancelled by user.");
                    return Ok(());
                }
                // Apply TUI filters (skipping nodes)
                for idx in preview.disabled_nodes {
                    changes.graph.remove_node(idx);
                }
            }

            // Point 12: Atomic Snapshot before execution
            let _snapshot = app.snapshot_manager.auto_snapshot("pre_sync").await?;

            engine.sync().await?;
        }

        // --- IMPERATIVE INSTALLATION (Auto-Commit) ---
        // Points: 2, 10, 14
        Commands::Install { packages } => {
            let mut state_guard = app.state.lock().await;
            for pkg_str in packages {
                // Point 10: Priority Probing handles bare names
                let resolved_specs = app.resolve_spec(pkg_str).await?;
                for spec in resolved_specs {
                    let backend = app.registry.get(&spec.backend)
                        .context(format!("Backend '{}' required for '{}' not found.", spec.backend, spec.name))?;
                    
                    if let Some(installer) = backend.as_installable() {
                        info!("Installing {} via {}...", spec.name, spec.backend);
                        installer.install(&[spec.clone()], true).await?;
                        
                        // Record in binary state
                        state_guard.add(&spec.backend, &spec.name, None, spec.options.clone());
                        
                        // Point 2: Declarative Reflection (Auto-Commit)
                        if let Err(e) = add_package_to_local(&app.config.groups_dir, pkg_str) {
                            warn!("Auto-Commit failed for {}: {}", spec.name, e);
                        }
                    }
                }
            }
            state_guard.save()?;
        }

        // --- IMPERATIVE REMOVAL (Auto-Commit) ---
        // Points: 2, 14
        Commands::Remove { packages } => {
            let mut state_guard = app.state.lock().await;
            for pkg_name in packages {
                let mut found = false;
                for backend in app.registry.available() {
                    if let Some(queryable) = backend.as_queryable() {
                        if queryable.info(pkg_name).await?.is_some() {
                            if let Some(installer) = backend.as_installable() {
                                info!("Removing {} from {}...", pkg_name, backend.name());
                                installer.remove(&[pkg_name.clone()], true).await?;
                                
                                // Record removal and Ghost Tracking (Point 14)
                                state_guard.remove(backend.name(), pkg_name);
                                
                                // Point 2: Remove from local manifest
                                let _ = remove_package_from_local(&app.config.groups_dir, pkg_name);
                                found = true;
                                break;
                            }
                        }
                    }
                }
                if !found { warn!("Target '{}' is not currently installed.", pkg_name); }
            }
            state_guard.save()?;
        }

        // --- MIGRATION (Point 3) ---
        Commands::Migrate => {
            let migrator = app.migrator();
            migrator.migrate().await?;
        }

        // --- TELEPORTATION (Point 5) ---
        Commands::Teleport { package, to } => {
            let teleporter = app.teleporter();
            teleporter.teleport(package, to).await?;
        }

        // --- GHOST SHELL (Point 19 / 20) ---
        Commands::Shell { packages } => {
            let shell = app.shell();
            if packages.is_empty() {
                shell.auto_shell().await?; // Point 20: Project-local
            } else {
                shell.enter(packages).await?; // Point 19: Ephemeral
            }
        }

        // --- SNAPSHOT GALLERY / UNDO (Point 12) ---
        Commands::Undo => {
            let undo = app.undo_manager();
            undo.run_interactive().await?;
        }

        // --- IDENTITY SWITCHER (Point 18) ---
        Commands::Profile { name } => {
            let pm = app.profile_manager();
            pm.switch(name).await?;
        }

        // --- SOURCE REPOSITORIES (Point 7) ---
        Commands::Repo(args) => {
            match &args.command {
                RepoCommand::Add { name, url, backend } => {
                    let b_name = backend.as_deref().unwrap_or("apt");
                    let b = app.registry.get(b_name).context("Backend not found")?;
                    let manager = b.as_repo_manager().context("Backend does not support repositories.")?;
                    manager.add_repo(name, url, true).await?;
                    info!("Successfully added repository: {}", name);
                }
                RepoCommand::Remove { name, backend } => {
                    let b_name = backend.as_deref().unwrap_or("apt");
                    let b = app.registry.get(b_name).context("Backend not found")?;
                    let manager = b.as_repo_manager().context("Backend does not support repositories.")?;
                    manager.remove_repo(name, true).await?;
                    info!("Successfully removed repository: {}", name);
                }
                RepoCommand::List { backend } => {
                    let b_name = backend.as_deref().unwrap_or("apt");
                    let b = app.registry.get(b_name).context("Backend not found")?;
                    let manager = b.as_repo_manager().context("Backend does not support repositories.")?;
                    let repos = manager.list_repos().await?;
                    println!("{:<20} {}", "NAME", "URL");
                    println!("{:-<60}", "");
                    for (n, u) in repos { println!("{:<20} {}", n, u); }
                }
            }
        }

        // --- UTILITY COMMANDS ---
        Commands::Update => app.update().await?,
        Commands::Upgrade => app.upgrade().await?,
        Commands::Orphans => app.clean_orphans().await?,
        
        Commands::Unmanaged => {
            let pkgs = app.get_unmanaged_packages().await?;
            println!("{:<15} {:<30} {}", "BACKEND", "UNMANAGED PACKAGE", "VERSION");
            println!("{:-<70}", "");
            for p in pkgs { 
                println!("{:<15} {:<30} {}", p.backend, p.name, p.version.unwrap_or_else(|| "N/A".into())); 
            }
        }

        Commands::List { backend } => {
            let pkgs = app.list(backend.as_deref()).await?;
            println!("{:<15} {:<30} {}", "BACKEND", "PACKAGE", "VERSION");
            println!("{:-<70}", "");
            for p in pkgs { 
                println!("{:<15} {:<30} {}", p.backend, p.name, p.version.unwrap_or_else(|| "unknown".into())); 
            }
        }

        Commands::Info { package } => {
            if let Some(p) = app.get_info(package).await? {
                println!("Package:     {}", p.name);
                println!("Backend:     {}", p.backend);
                println!("Version:     {}", p.version.unwrap_or_else(|| "N/A".into()));
                for (k, v) in p.properties {
                    println!("{:<12} {}", format!("{}:", k), v);
                }
            } else {
                error!("Package '{}' not found in any available backend.", package);
            }
        }

        Commands::Search { query } => {
            let results = app.search(query).await?;
            println!("{:<15} {:<40} {}", "BACKEND", "PACKAGE", "VERSION");
            println!("{:-<80}", "");
            for p in results {
                println!("{:<15} {:<40} {}", p.backend, p.name, p.version.unwrap_or_default());
            }
        }

        Commands::Doctor => {
            info!("Performing health check...");
            for backend in app.registry.all() {
                let status = if backend.is_available() { "READY" } else { "OFFLINE" };
                println!("[{}] {:<15}", status, backend.name());
            }
        }

        Commands::Shim { binary, source } => {
            app.create_shim(binary, source).await?;
        }

        Commands::Heal => {
            let engine = SyncEngine::new(
                &app.config, app.registry.clone(), app.executor.clone(), 
                app.metrics.clone(), app.progress.clone(), 
                app.hooks.clone(), app.snapshot_manager.clone(),
                app.journal.clone()
            );
            engine.heal().await?;
        }
        
        Commands::Run { packages, command } => {
            let runner = Runner::new(&app);
            // Splitting command for shell compatibility
            let (cmd_bin, args) = if let Some((c, a)) = linix::utils::command::split_command(command) {
                (c, a)
            } else {
                (command.clone(), vec![])
            };
            runner.run(packages, &cmd_bin, &args).await?;
        }

        _ => println!("This command is currently in the staging phase."),
    }

    Ok(())
}