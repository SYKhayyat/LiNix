// src/main.rs
use anyhow::{Context, Result};
use clap::{CommandFactory, Parser};
use dialoguer::{theme::ColorfulTheme, Select};
use linix::app::App;
use linix::cli::{Cli, Commands, RepoCommand};
use linix::config::Config;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Initialize Logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // 2. Parse CLI and Load Config
    let cli = Cli::parse();
    let config_path = cli.config.clone().unwrap_or_else(|| {
        dirs::config_dir()
            .unwrap_or_default()
            .join("linix")
            .join("config.toml")
    });

    let mut config = Config::from_file(&config_path).context("Failed to load config file")?;
    
    // Override config with CLI flags
    if cli.dry_run { config.dry_run = true; }
    if cli.yes { config.yes = true; }
    if let Some(ref b) = cli.backend { config.enabled_backends = vec![b.clone()]; }
    config.show_progress = cli.progress;

    // 3. Handle Shell Completions (Early Exit)
    if let Commands::Completions { shell } = &cli.command {
        let mut cmd = Cli::command();
        let gen: clap_complete::Shell = (*shell).into();
        clap_complete::generate(gen, &mut cmd, "linix", &mut std::io::stdout());
        return Ok(());
    }

    // 4. Initialize Core App Context
    let app = App::new(config).await?;

    // 5. Route Commands
    match &cli.command {
        Commands::Sync { locked } => {
            let engine = linix::app::SyncEngine::new(
                &app.config,
                app.registry.clone(),
                app.executor.clone(),
                app.cache.clone(),
                app.metrics.clone(),
                app.progress.clone(),
                app.hooks.clone(),
            )
            .with_lockfile(*locked);
            
            // MISSION CRITICAL: Automatically heal interrupted transactions
            let _ = engine.heal().await;
            engine.sync().await?;
        }

        Commands::Heal => {
            let engine = linix::app::SyncEngine::new(
                &app.config,
                app.registry.clone(),
                app.executor.clone(),
                app.cache.clone(),
                app.metrics.clone(),
                app.progress.clone(),
                app.hooks.clone(),
            );
            engine.heal().await?;
        }

        Commands::Clean => {
            let engine = linix::app::SyncEngine::new(
                &app.config,
                app.registry.clone(),
                app.executor.clone(),
                app.cache.clone(),
                app.metrics.clone(),
                app.progress.clone(),
                app.hooks.clone(),
            );
            engine.clean().await?;
        }

        Commands::Backends => {
            // FEATURE: Smart Export (Only manual installs, no junk)
            let engine = linix::app::SyncEngine::new(
                &app.config,
                app.registry.clone(),
                app.executor.clone(),
                app.cache.clone(),
                app.metrics.clone(),
                app.progress.clone(),
                app.hooks.clone(),
            );
            let exported = engine.export_system().await?;
            println!("{}", exported);
        }

        Commands::Run { packages, command } => {
            app.run_ephemeral(packages.clone(), command).await?;
        }

        Commands::Teleport { package, to } => {
            app.teleport(package, to).await?;
        }

        Commands::Shim { binary, source } => {
            app.create_shim(binary, source).await?;
        }

        Commands::Install { packages } => {
            for pkg in packages {
                let backend_name = if let Some(ref b) = cli.backend {
                    b.clone()
                } else {
                    let results = app.search(pkg).await?;
                    let mut providers: Vec<String> = results.into_iter().map(|p| p.backend).collect();
                    providers.sort();
                    providers.dedup();
                    
                    if providers.is_empty() {
                        detect_default_backend(&app, &app.config)
                    } else if providers.len() == 1 || cli.yes {
                        providers[0].clone()
                    } else {
                        let idx = Select::with_theme(&ColorfulTheme::default())
                            .with_prompt(format!("Choose manager for '{}':", pkg))
                            .items(&providers)
                            .default(0)
                            .interact()?;
                        providers[idx].clone()
                    }
                };
                let m = app.registry.get(&backend_name).context("Backend not found")?;
                m.install(&[pkg.clone()], true).await?;
                let _ = update_user_group_file(&app.config, &backend_name, &[pkg.clone()], true);
            }
        }

        Commands::Repo(repo_args) => {
            match &repo_args.command {
                RepoCommand::List { backend } => {
                    let target_name = backend.as_deref().unwrap_or("apt");
                    if let Some(mgr) = app.registry.get(target_name) {
                        let repos = mgr.list_repos().await?;
                        for (name, url) in repos { println!("{} -> {}", name, url); }
                    }
                }
                _ => { println!("Repository modification via CLI is currently disabled for safety. Use config.toml."); }
            }
        }

        Commands::Doctor => {
            for m in app.registry.all() {
                let report = m.check_health().await?;
                let status = if matches!(report.status, linix::core::manager::HealthStatus::Ok) {
                    "[OK]"
                } else {
                    "[ERR]"
                };
                println!("{} {}", status, m.name());
            }
        }

        _ => {
            println!("This command is available in the CLI but its internal logic is still being finalized.");
        }
    }

    Ok(())
}

fn detect_default_backend(app: &App, config: &Config) -> String {
    if let Some(ref d) = config.default_backend {
        if app.registry.get(d).map(|m| m.is_available()).unwrap_or(false) {
            return d.clone();
        }
    }
    // Fallback order
    ["apt", "pacman", "dnf", "winget", "brew"]
        .into_iter()
        .find(|b| {
            app.registry.get(b).map(|m| m.is_available()).unwrap_or(false)
        })
        .unwrap_or("apt")
        .to_string()
}

fn update_user_group_file(config: &Config, b: &str, pkgs: &[String], add: bool) -> Result<()> {
    if config.dry_run { return Ok(()); }
    let path = linix::config::parser::get_user_group_file(&config.groups_dir);
    let mut current = if path.exists() {
        linix::config::parser::parse_group_file(&path)?
    } else {
        vec![]
    };
    let specs: Vec<String> = pkgs.iter().map(|p| format!("{}:{}", b, p)).collect();
    if add {
        for s in specs {
            if !current.contains(&s) { current.push(s); }
        }
    } else {
        current.retain(|s| !specs.contains(s));
    }
    linix::config::parser::write_group_file(&path, &current)?;
    Ok(())
}