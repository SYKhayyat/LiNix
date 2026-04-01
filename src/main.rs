use anyhow::{Context, Result};
use clap::Parser;
use linix::app::{App, SyncEngine};
use linix::cli::{Cli, Commands, RepoCommand};
use linix::core::transaction::PackageOperation;
use linix::core::Transaction;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let cli = Cli::parse();
    let config_path = cli.config.clone().unwrap_or_else(|| {
        dirs::config_dir().unwrap_or_default().join("linix").join("config.toml")
    });

    let config = linix::config::Config::from_file(&config_path).context("Failed to load config")?;
    let mut app = App::new(config).await?;
    app.config.merge_cli_overrides(Some(cli.dry_run), Some(cli.yes), cli.backend, None, cli.groups_dir, Some(cli.verbose));

    match &cli.command {
        Commands::Sync { locked } => {
            let engine = SyncEngine::new(&app.config, app.registry.clone(), app.executor.clone(), 
                                        app.cache.clone(), app.metrics.clone(), app.progress.clone(), 
                                        app.hooks.clone())
                        .with_lockfile(*locked);
            let _ = engine.heal().await;
            engine.sync().await?;
        }

        Commands::Install { packages } => {
            let mut tx = Transaction::new();
            let mut registry = app.state.clone();
            
            for pkg in packages {
                let backend = if let Some(ref b) = cli.backend { b.clone() } 
                              else { app.search(pkg).await?.first().map(|p| p.backend.clone()).unwrap_or_else(|| "apt".into()) };
                
                let mgr = app.registry.get(&backend).context("Backend not found")?;
                tx.add(Box::new(PackageOperation { 
                    manager: mgr.clone(), 
                    packages: vec![pkg.clone()], 
                    is_install: true, 
                    sudo: true 
                }));
                registry.add(&backend, pkg);
            }
            
            tx.execute().await?;
            registry.save()?; // Persistence
        }

        Commands::Remove { packages } => {
            let mut tx = Transaction::new();
            let mut registry = app.state.clone();

            for pkg in packages {
                let mut found = false;
                for mgr in app.registry.available() {
                    let inst = mgr.list_installed().await?;
                    if inst.iter().any(|p| p.name == *pkg) {
                        tx.add(Box::new(PackageOperation { 
                            manager: mgr.clone(), 
                            packages: vec![pkg.clone()], 
                            is_install: false, 
                            sudo: true 
                        }));
                        registry.remove(mgr.name(), pkg);
                        found = true;
                        break;
                    }
                }
                if !found { tracing::warn!("Package '{}' not found in any managed backend.", pkg); }
            }
            tx.execute().await?;
            registry.save()?;
        }

        Commands::Repo(args) => {
            match &args.command {
                RepoCommand::Add { name, url, backend } => {
                    let b = backend.as_deref().unwrap_or("apt");
                    app.registry.get(b).context("Backend not found")?.add_repo(name, url, true).await?;
                    println!("Added repository: {}", name);
                }
                RepoCommand::Remove { name, backend } => {
                    let b = backend.as_deref().unwrap_or("apt");
                    app.registry.get(b).context("Backend not found")?.remove_repo(name, true).await?;
                    println!("Removed repository: {}", name);
                }
                RepoCommand::List { backend } => {
                    let b = backend.as_deref().unwrap_or("apt");
                    let mgr = app.registry.get(b).context("Backend not found")?;
                    for (n, u) in mgr.list_repos().await? { println!("{:<20} {}", n, u); }
                }
            }
        }

        Commands::Update => app.update().await?,
        Commands::Upgrade => app.upgrade().await?,
        Commands::List { backend } => {
            let pkgs = app.list(backend.as_deref()).await?;
            for p in pkgs { println!("{:<15} {:<30} {}", p.backend, p.name, p.version.unwrap_or_default()); }
        }
        Commands::Info { package } => {
            if let Some(p) = app.get_info(package).await? {
                println!("Name:        {}\nBackend:     {}\nVersion:     {}\nDescription: {}", 
                         p.name, p.backend, p.version.unwrap_or_default(), p.description.unwrap_or_default());
            }
        }
        Commands::Orphans => app.orphans().await?,
        Commands::Backends => {
            let engine = SyncEngine::new(&app.config, app.registry.clone(), app.executor.clone(), app.cache.clone(), app.metrics.clone(), app.progress.clone(), app.hooks.clone());
            println!("{}", engine.export_system().await?);
        }
        Commands::Doctor => {
            for m in app.registry.all() {
                let report = m.check_health().await?;
                println!("[{}] {:<15} {}", if matches!(report.status, linix::core::manager::HealthStatus::Ok) { "OK" } else { "ERR" }, m.name(), report.message.unwrap_or_default());
            }
        }
        _ => println!("Command complete."),
    }
    Ok(())
}