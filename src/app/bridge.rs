use crate::core::{Result, Error, StateRegistry};
use crate::backends::BackendRegistry;
use crate::config::Config;
use regex::Regex;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, debug, warn};
use dialoguer::{theme::ColorfulTheme, Confirm};

/// Represents a mapping between a specific error pattern and the system packages 
/// required to fix it across different package managers.
struct BridgeMapping {
    pattern: Regex,
    suggestions: HashMap<String, String>,
    description: String,
}

/// The Knowledge Base for Semantic Dependency Bridging (Point 16).
pub struct BridgeDb {
    mappings: Vec<BridgeMapping>,
}

impl BridgeDb {
    pub fn new() -> Self {
        let mut mappings = Vec::new();

        // 1. OpenSSL / Cryptography Failures
        let mut ssl = HashMap::new();
        ssl.insert("apt".into(), "libssl-dev".into());
        ssl.insert("dnf".into(), "openssl-devel".into());
        ssl.insert("pacman".into(), "openssl".into());
        ssl.insert("brew".into(), "openssl@3".into());
        ssl.insert("choco".into(), "openssl".into());
        
        mappings.push(BridgeMapping {
            pattern: Regex::new(r"openssl/ssl\.h|libssl|cannot find -lssl|crypto|SSL_library_init").unwrap(),
            suggestions: ssl,
            description: "Missing OpenSSL development headers".into(),
        });

        // 2. Common Build Tooling
        let mut build = HashMap::new();
        build.insert("apt".into(), "build-essential".into());
        build.insert("dnf".into(), "development-tools".into());
        build.insert("pacman".into(), "base-devel".into());
        build.insert("brew".into(), "gcc".into());
        
        mappings.push(BridgeMapping {
            pattern: Regex::new(r"cc1plus|stdio\.h|stdlib\.h|g\+\+ not found|clang: error|make: command not found").unwrap(),
            suggestions: build,
            description: "Missing C/C++ compiler or standard library headers".into(),
        });

        // 3. Zlib Compression
        let mut zlib = HashMap::new();
        zlib.insert("apt".into(), "zlib1g-dev".into());
        zlib.insert("dnf".into(), "zlib-devel".into());
        zlib.insert("pacman".into(), "zlib".into());
        zlib.insert("brew".into(), "zlib".into());
        
        mappings.push(BridgeMapping {
            pattern: Regex::new(r"zlib\.h|cannot find -lz|Zlib\.h").unwrap(),
            suggestions: zlib,
            description: "Missing zlib compression library".into(),
        });

        Self { mappings }
    }
    
    fn find_mapping(&self, stderr: &str) -> Option<&BridgeMapping> {
        for mapping in &self.mappings {
            if mapping.pattern.is_match(stderr) {
                return Some(mapping);
            }
        }
        None
    }
    
    fn get_suggestion(&self, mapping: &BridgeMapping, backend: &str) -> Option<String> {
        mapping.suggestions.get(backend).cloned()
    }
}

/// The Dependency Bridge orchestrator.
/// Hardened for Phase 4.1: Fully decoupled from the God Object (App).
pub struct DependencyBridge {
    db: BridgeDb,
}

impl DependencyBridge {
    pub fn new() -> Self {
        Self { db: BridgeDb::new() }
    }

    /// Analyzes stderr output and identifies missing system dependencies.
    pub fn diagnose_failure(&self, stderr: &str, current_backend: &str) -> Vec<String> {
        let mut suggestions = Vec::new();
        debug!("DependencyBridge: Analyzing error output...");

        if let Some(mapping) = self.db.find_mapping(stderr) {
            info!("DependencyBridge: Detected potential root cause: {}", mapping.description);
            
            if let Some(package) = self.db.get_suggestion(mapping, current_backend) {
                suggestions.push(format!("{}:{}", current_backend, package));
            } else {
                for (b, p) in &mapping.suggestions {
                    suggestions.push(format!("{}:{}", b, p));
                }
            }
        }
        suggestions
    }

    /// Primary decoupled failure handler.
    /// Fulfills Phase 5.1 cleanup: Removed dead reference to monolithic App.
    pub async fn handle_failure(
        &self, 
        stderr: &str, 
        current_backend: &str, 
        registry: Arc<BackendRegistry>,
        state: Arc<Mutex<StateRegistry>>,
        config: &Config,
        auto_install: bool
    ) -> Result<()> {
        let suggestions = self.diagnose_failure(stderr, current_backend);
        if suggestions.is_empty() { return Ok(()); }
        
        println!("\n💡 LiNix Insight: This build likely failed due to missing dependencies.");
        println!("Detected issue: {}", self.get_description(stderr).unwrap_or_else(|| "Unknown build failure".to_string()));
        
        println!("\nSuggested package(s) to install:");
        for s in &suggestions {
            println!("  - linix install {}", s);
        }
        
        if auto_install {
            self.auto_install_suggestions(&suggestions, registry, state, config).await?;
        } else {
            // Confirm::interact() is blocking; wrap in spawn_blocking
            let should_install = tokio::task::spawn_blocking(move || {
                Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt("Would you like to install these dependencies now?")
                    .default(false)
                    .interact()
            }).await.map_err(|e| Error::Other(e.to_string()))?
              .map_err(|e| Error::Other(e.to_string()))?;
            
            if should_install {
                self.auto_install_suggestions(&suggestions, registry, state, config).await?;
            }
        }
        Ok(())
    }
    
    async fn auto_install_suggestions(
        &self, 
        suggestions: &[String], 
        registry: Arc<BackendRegistry>,
        state: Arc<Mutex<StateRegistry>>,
        config: &Config
    ) -> Result<()> {
        let mut success_count = 0;
        
        // Use a resolver to parse the suggestion strings
        let resolver = crate::app::sync::resolver::StateResolver::new(config, registry.clone());

        for suggestion in suggestions {
            info!("DependencyBridge: Auto-installing {}", suggestion);
            
            let spec = resolver.parse_and_probe_spec(suggestion).await?;
            if let Some(backend_cap) = registry.get(&spec.backend) {
                if let Some(installer) = backend_cap.as_installable() {
                    let sudo = backend_cap.needs_root();
                    match installer.install(&[spec.clone()], sudo).await {
                        Ok(_) => {
                            success_count += 1;
                            let mut state_guard = state.lock().await;
                            state_guard.add_simple(&spec.backend, &spec.name, None);
                            
                            // StateRegistry::save is blocking
                            let state_clone = state_guard.clone();
                            tokio::task::spawn_blocking(move || {
                                state_clone.save()
                            }).await.map_err(|e| Error::Other(e.to_string()))??;
                        }
                        Err(e) => warn!("Failed to install {}: {}", suggestion, e),
                    }
                }
            }
        }
        println!("\n📦 Auto-install complete: {} installed", success_count);
        Ok(())
    }
    
    fn get_description(&self, stderr: &str) -> Option<String> {
        self.db.find_mapping(stderr).map(|m| m.description.clone())
    }

    pub fn print_suggestions(&self, stderr: &str, current_backend: &str) {
        let suggestions = self.diagnose_failure(stderr, current_backend);
        if !suggestions.is_empty() {
            println!("\n💡 LiNix Insight: This build likely failed due to missing system headers.");
            for s in suggestions {
                println!("  - linix install {}", s);
            }
            println!();
        }
    }
}

impl Default for DependencyBridge {
    fn default() -> Self {
        Self::new()
    }
}