use clap::Parser;
use linix::app::App;
use linix::cli::{Cli, Commands, Shell};
use linix::config::Config;
use linix::core::Package;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
        )
        .init();

    // Parse CLI arguments
    let cli = Cli::parse();

    // Determine config path
    let config_path = cli.config.clone().unwrap_or_else(|| {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("linix")
            .join("config.toml")
    });

    // Load configuration
    let mut config = Config::from_file(&config_path)?;

    // Merge CLI overrides
    if cli.dry_run {
        config.dry_run = true;
    }
    if cli.yes {
        config.yes = true;
    }
    if cli.verbose {
        config.verbose = true;
    }
    if let Some(ref backend) = cli.backend {
        config.enabled_backends = vec![backend.clone()];
    }
    if let Some(ref groups_dir) = cli.groups_dir {
        config.groups_dir = groups_dir.clone();
    }
    if cli.remove_bloatware {
        config.remove_bloatware = true;
    }
    config.show_progress = cli.progress;

    // Handle completions command first (doesn't need app initialization)
    if let Commands::Completions { shell } = &cli.command {
        generate_completions(*shell);
        return Ok(());
    }

    // Create app instance
    let mut app = App::new(config).await?;

    // Execute command
    match cli.command {
        Commands::Sync => {
            app.sync().await?;
        }
        Commands::Clean => {
            app.clean().await?;
        }
        Commands::Unmanaged => {
            let unmanaged = app.unmanaged().await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&unmanaged)?);
            } else if unmanaged.is_empty() {
                println!("No unmanaged packages found.");
            } else {
                println!("Unmanaged packages:");
                for (backend, packages) in unmanaged {
                    for pkg in packages {
                        println!("  [{}] {}", backend, pkg);
                    }
                }
            }
        }
        Commands::Orphans => {
            app.orphans().await?;
        }
        Commands::Search { query } => {
            let results = app.search(&query).await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&results)?);
            } else if results.is_empty() {
                println!("No packages found matching '{}'", query);
            } else {
                println!("Search results for '{}':", query);
                for pkg in results {
                    print!("  {} [{}]", pkg.name, pkg.backend);
                    if let Some(version) = &pkg.version {
                        print!(" v{}", version);
                    }
                    println!();
                    if let Some(desc) = &pkg.description {
                        println!("    {}", desc);
                    }
                }
            }
        }
        Commands::Update => {
            println!("Updating package databases...");
            for backend in app.registry.available() {
                println!("Updating {}...", backend.name());
                if let Err(e) = backend.update(true).await {
                    eprintln!("Failed to update {}: {}", backend.name(), e);
                }
            }
            println!("Done.");
        }
        Commands::Upgrade => {
            println!("Upgrading packages...");
            for backend in app.registry.available() {
                println!("Upgrading {} packages...", backend.name());
                if let Err(e) = backend.upgrade(true).await {
                    eprintln!("Failed to upgrade {}: {}", backend.name(), e);
                }
            }
            println!("Done.");
        }
        Commands::List { backend: filter_backend } => {
            let mut all_packages = Vec::new();
            
            let backends = if let Some(ref name) = filter_backend {
                if let Some(b) = app.registry.get(name) {
                    vec![b]
                } else {
                    eprintln!("Backend '{}' not found", name);
                    return Ok(());
                }
            } else {
                app.registry.available()
            };

            for backend in backends {
                match backend.list_installed().await {
                    Ok(pkgs) => all_packages.extend(pkgs),
                    Err(e) => eprintln!("Failed to list {}: {}", backend.name(), e),
                }
            }

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&all_packages)?);
            } else if all_packages.is_empty() {
                println!("No packages installed.");
            } else {
                println!("Installed packages ({}):", all_packages.len());
                for pkg in all_packages {
                    println!("  {} [{}]", pkg.display_name(), pkg.backend);
                }
            }
        }
        Commands::Info { package } => {
            let mut found = false;
            for backend in app.registry.available() {
                if let Ok(Some(pkg)) = backend.info(&package).await {
                    found = true;
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&pkg)?);
                    } else {
                        print_package_info(&pkg);
                    }
                    break;
                }
            }
            if !found {
                eprintln!("Package '{}' not found", package);
            }
        }
        Commands::Install { packages } => {
            if packages.is_empty() {
                eprintln!("No packages specified");
                return Ok(());
            }

            if let Some(ref backend_name) = cli.backend {
                if let Some(backend) = app.registry.get(backend_name) {
                    println!("Installing via {}...", backend_name);
                    backend.install(&packages, true).await?;
                    println!("Done.");
                } else {
                    eprintln!("Backend '{}' not found", backend_name);
                }
            } else {
                // Auto-detect backend or use default
                let default_backend = detect_default_backend(&app);
                if let Some(backend) = app.registry.get(&default_backend) {
                    println!("Installing via {}...", default_backend);
                    backend.install(&packages, true).await?;
                    println!("Done.");
                } else {
                    eprintln!("No suitable backend found");
                }
            }
        }
        Commands::Remove { packages } => {
            if packages.is_empty() {
                eprintln!("No packages specified");
                return Ok(());
            }

            if let Some(ref backend_name) = cli.backend {
                if let Some(backend) = app.registry.get(backend_name) {
                    println!("Removing via {}...", backend_name);
                    backend.remove(&packages, true).await?;
                    println!("Done.");
                } else {
                    eprintln!("Backend '{}' not found", backend_name);
                }
            } else {
                let default_backend = detect_default_backend(&app);
                if let Some(backend) = app.registry.get(&default_backend) {
                    println!("Removing via {}...", default_backend);
                    backend.remove(&packages, true).await?;
                    println!("Done.");
                } else {
                    eprintln!("No suitable backend found");
                }
            }
        }
        Commands::Backends => {
            let available = app.available_backends();
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&available)?);
            } else {
                println!("Available backends ({}):", available.len());
                for name in available {
                    println!("  - {}", name);
                }
            }
        }
        Commands::Completions { .. } => {
            // Already handled above
        }
    }

    // Print metrics if verbose
    if cli.verbose {
        app.metrics.print_summary();
    }

    Ok(())
}

fn detect_default_backend(app: &App) -> String {
    let system_backends = ["apt", "dnf", "pacman", "zypper", "apk", "brew"];
    
    for backend in system_backends {
        if app.registry.get(backend).map(|m| m.is_available()).unwrap_or(false) {
            return backend.to_string();
        }
    }
    
    // Windows fallbacks
    #[cfg(target_os = "windows")]
    {
        let windows_backends = ["winget", "choco", "scoop"];
        for backend in windows_backends {
            if app.registry.get(backend).map(|m| m.is_available()).unwrap_or(false) {
                return backend.to_string();
            }
        }
    }
    
    "apt".to_string()
}

fn print_package_info(pkg: &Package) {
    println!("Name: {}", pkg.name);
    if let Some(version) = &pkg.version {
        println!("Version: {}", version);
    }
    println!("Backend: {}", pkg.backend);
    if let Some(desc) = &pkg.description {
        println!("Description: {}", desc);
    }
    if let Some(repo) = &pkg.repository {
        println!("Repository: {}", repo);
    }
    if let Some(size) = pkg.size {
        let size_str = if size > 1_000_000_000 {
            format!("{:.2} GB", size as f64 / 1_000_000_000.0)
        } else if size > 1_000_000 {
            format!("{:.2} MB", size as f64 / 1_000_000.0)
        } else if size > 1_000 {
            format!("{:.2} KB", size as f64 / 1_000.0)
        } else {
            format!("{} bytes", size)
        };
        println!("Size: {}", size_str);
    }
}

fn generate_completions(shell: Shell) {
    use clap::CommandFactory;
    use clap_complete::generate;
    
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    let clap_shell: clap_complete::Shell = shell.into();
    
    generate(clap_shell, &mut cmd, name, &mut std::io::stdout());
}
