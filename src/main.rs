// src/main.rs
use clap::{Parser, CommandFactory};
use linix::app::App;
use linix::cli::{Cli, Commands, RepoCommand};
use linix::config::Config;
use tracing_subscriber::EnvFilter;
use dialoguer::{Select, theme::ColorfulTheme};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let cli = Cli::parse();
    let config_path = cli.config.clone().unwrap_or_else(|| {
        dirs::config_dir().unwrap_or_default().join("linix").join("config.toml")
    });

    let mut config = Config::from_file(&config_path)?;
    if cli.dry_run { config.dry_run = true; }
    if cli.yes { config.yes = true; }
    if let Some(ref b) = cli.backend { config.enabled_backends = vec![b.clone()]; }
    config.show_progress = cli.progress;

    if let Commands::Completions { shell } = &cli.command {
        let mut cmd = Cli::command();
        let generator: clap_complete::Shell = (*shell).into();
        clap_complete::generate(generator, &mut cmd, "linix", &mut std::io::stdout());
        return Ok(());
    }

    let app = App::new(config.clone()).await?;

    match &cli.command {
        Commands::Sync { locked } => {
            let engine = linix::app::SyncEngine::new(&app.config, &app.registry, &app.executor, &app.cache, &app.metrics, app.progress.as_ref(), &app.hooks)
                .with_lockfile(*locked);
            let _ = engine.heal().await;
            engine.sync().await?;
        }
        Commands::Heal => {
            let engine = linix::app::SyncEngine::new(&app.config, &app.registry, &app.executor, &app.cache, &app.metrics, app.progress.as_ref(), &app.hooks);
            engine.heal().await?;
        }
        Commands::Install { packages } => {
            for pkg in packages {
                let backend_name = if let Some(ref b) = cli.backend { b.clone() } else {
                    let results = app.search(pkg).await?;
                    let mut providers: Vec<String> = results.into_iter().map(|p| p.backend).collect();
                    providers.sort(); providers.dedup();
                    if providers.is_empty() { detect_default_backend(&app, &config) }
                    else if providers.len() == 1 || cli.yes { providers[0].clone() }
                    else {
                        let idx = Select::with_theme(&ColorfulTheme::default())
                            .with_prompt(format!("Choose manager for '{}':", pkg))
                            .items(&providers).default(0).interact()?;
                        providers[idx].clone()
                    }
                };
                if let Some(m) = app.registry.get(&backend_name) {
                    m.install(&[pkg.clone()], true).await?;
                    let _ = update_user_group_file(&app.config, &backend_name, &[pkg.clone()], true);
                }
            }
        }
        Commands::Repo(repo_args) => {
            let backends = app.registry.available();
            match &repo_args.command {
                RepoCommand::List { backend } => {
                    let targets = backend.as_ref().map(|b| vec![app.registry.get(b).unwrap()]).unwrap_or(backends);
                    for t in targets {
                        println!("Repositories for {}:", t.name());
                        if let Ok(repos) = t.list_repos().await {
                            for (n, u) in repos { println!("  {} -> {}", n, u); }
                        }
                    }
                }
                RepoCommand::Add { name, url, backend } => {
                    let targets = backend.as_ref().map(|b| vec![app.registry.get(b).unwrap()]).unwrap_or(backends);
                    for t in targets { let _ = t.add_repo(name, url, true).await; }
                }
                RepoCommand::Remove { name, backend } => {
                    let targets = backend.as_ref().map(|b| vec![app.registry.get(b).unwrap()]).unwrap_or(backends);
                    for t in targets { let _ = t.remove_repo(name, true).await; }
                }
            }
        }
        Commands::Doctor => {
            for m in app.registry.all() {
                let report = m.check_health().await?;
                let status = if matches!(report.status, linix::core::manager::HealthStatus::Ok) { "[OK]" } else { "[ERR]" };
                println!("{} {}", status, m.name());
            }
        }
        _ => {}
    }
    Ok(())
}

fn detect_default_backend(app: &App, config: &Config) -> String {
    if let Some(ref d) = config.default_backend {
        if app.registry.get(d).map(|m| m.is_available()).unwrap_or(false) { return d.clone(); }
    }
    ["apt", "dnf", "pacman", "brew"].into_iter()
        .find(|b| app.registry.get(b).map(|m| m.is_available()).unwrap_or(false))
        .unwrap_or("apt").to_string()
}

fn update_user_group_file(config: &Config, b: &str, pkgs: &[String], add: bool) -> anyhow::Result<()> {
    if config.dry_run { return Ok(()); }
    let path = linix::config::parser::get_user_group_file(&config.groups_dir);
    let mut current = if path.exists() { linix::config::parser::parse_group_file(&path)? } else { vec![] };
    let specs: Vec<String> = pkgs.iter().map(|p| format!("{}:{}", b, p)).collect();
    if add { for s in specs { if !current.contains(&s) { current.push(s); } } }
    else { current.retain(|s| !specs.contains(s)); }
    linix::config::parser::write_group_file(&path, &current)?;
    Ok(())
}