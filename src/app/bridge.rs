use crate::core::{Result, Error, PackageSpec};
use crate::App;
use regex::Regex;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, debug, warn, error};
use dialoguer::{theme::ColorfulTheme, Confirm, Select};

/// Represents a mapping between a specific error pattern and the system packages 
/// required to fix it across different package managers.
struct BridgeMapping {
    /// Regex pattern to match in stderr (e.g., "openssl/ssl.h: No such file")
    pattern: Regex,
    /// Map of Backend Name -> Package Name (e.g., "apt" -> "libssl-dev")
    suggestions: HashMap<String, String>,
    /// Human-readable explanation of the failure
    description: String,
}

/// The Knowledge Base for Semantic Dependency Bridging (Point 16).
/// FIX #18: Added auto-install functionality for missing dependencies.
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
        
        mappings.push(BridgeMapping {
            pattern: Regex::new(r"openssl/ssl\.h|libssl|cannot find -lssl|crypto|SSL_library_init").unwrap(),
            suggestions: ssl,
            description: "Missing OpenSSL development headers".into(),
        });

        // 2. Common Build Tooling (C/C++ Essentials)
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

        // 4. FFI / Header failures for dynamic languages
        let mut ffi = HashMap::new();
        ffi.insert("apt".into(), "libffi-dev".into());
        ffi.insert("dnf".into(), "libffi-devel".into());
        ffi.insert("pacman".into(), "libffi".into());
        ffi.insert("brew".into(), "libffi".into());
        
        mappings.push(BridgeMapping {
            pattern: Regex::new(r"ffi\.h|foreign function interface|cffi|_ffi").unwrap(),
            suggestions: ffi,
            description: "Missing Foreign Function Interface (FFI) headers".into(),
        });

        // 5. Sqlite3
        let mut sqlite = HashMap::new();
        sqlite.insert("apt".into(), "libsqlite3-dev".into());
        sqlite.insert("dnf".into(), "sqlite-devel".into());
        sqlite.insert("pacman".into(), "sqlite".into());
        sqlite.insert("brew".into(), "sqlite3".into());
        
        mappings.push(BridgeMapping {
            pattern: Regex::new(r"sqlite3\.h|cannot find -lsqlite3|SQLite3\.h").unwrap(),
            suggestions: sqlite,
            description: "Missing SQLite development files".into(),
        });

        // 6. CMake build system
        let mut cmake = HashMap::new();
        cmake.insert("apt".into(), "cmake".into());
        cmake.insert("dnf".into(), "cmake".into());
        cmake.insert("pacman".into(), "cmake".into());
        cmake.insert("brew".into(), "cmake".into());
        
        mappings.push(BridgeMapping {
            pattern: Regex::new(r"CMake Error|CMakeLists\.txt|cmake: command not found").unwrap(),
            suggestions: cmake,
            description: "Missing CMake build system".into(),
        });

        // 7. pkg-config
        let mut pkgconfig = HashMap::new();
        pkgconfig.insert("apt".into(), "pkg-config".into());
        pkgconfig.insert("dnf".into(), "pkgconfig".into());
        pkgconfig.insert("pacman".into(), "pkg-config".into());
        pkgconfig.insert("brew".into(), "pkg-config".into());
        
        mappings.push(BridgeMapping {
            pattern: Regex::new(r"pkg-config: command not found|PKG_CONFIG_PATH").unwrap(),
            suggestions: pkgconfig,
            description: "Missing pkg-config tool".into(),
        });

        // 8. Python development headers
        let mut python = HashMap::new();
        python.insert("apt".into(), "python3-dev".into());
        python.insert("dnf".into(), "python3-devel".into());
        python.insert("pacman".into(), "python".into());
        python.insert("brew".into(), "python3".into());
        
        mappings.push(BridgeMapping {
            pattern: Regex::new(r"Python\.h|python3\.h|cannot find -lpython|pyconfig\.h").unwrap(),
            suggestions: python,
            description: "Missing Python development headers".into(),
        });

        Self { mappings }
    }
    
    /// Finds the matching mapping for a given error message.
    fn find_mapping(&self, stderr: &str) -> Option<&BridgeMapping> {
        for mapping in &self.mappings {
            if mapping.pattern.is_match(stderr) {
                return Some(mapping);
            }
        }
        None
    }
    
    /// Gets the suggested package for a specific backend.
    fn get_suggestion(&self, mapping: &BridgeMapping, backend: &str) -> Option<String> {
        mapping.suggestions.get(backend).cloned()
    }
}

/// The Dependency Bridge orchestrator.
/// Intercepts failures from language backends and attempts to provide 
/// actionable intelligence to the user.
/// FIX #18: Added auto-install functionality.
pub struct DependencyBridge {
    db: BridgeDb,
}

impl DependencyBridge {
    pub fn new() -> Self {
        Self { db: BridgeDb::new() }
    }

    /// Analyzes the stderr from a failed command and returns suggested 
    /// package strings for the current system.
    pub fn diagnose_failure(&self, stderr: &str, current_backend: &str) -> Vec<String> {
        let mut suggestions = Vec::new();

        debug!("DependencyBridge: Analyzing error output for missing headers...");

        if let Some(mapping) = self.db.find_mapping(stderr) {
            info!("DependencyBridge: Detected potential root cause: {}", mapping.description);
            
            if let Some(package) = self.db.get_suggestion(mapping, current_backend) {
                suggestions.push(format!("{}:{}", current_backend, package));
            } else {
                for (b, p) in &mapping.suggestions {
                    debug!("DependencyBridge: Suggestion for other backend {}: {}", b, p);
                    suggestions.push(format!("{}:{}", b, p));
                }
            }
        }

        suggestions
    }

    /// FIX #18: High-level method to print suggestions and optionally auto-install.
    pub async fn handle_failure(&self, stderr: &str, current_backend: &str, app: &App, auto_install: bool) -> Result<()> {
        let suggestions = self.diagnose_failure(stderr, current_backend);
        
        if suggestions.is_empty() {
            return Ok(());
        }
        
        println!("\n💡 LiNix Insight: This build likely failed due to missing system dependencies.");
        println!("Detected issue: {}", self.get_description(stderr).unwrap_or("Unknown build failure"));
        println!("\nSuggested package(s) to install:");
        for s in &suggestions {
            println!("  - linix install {}", s);
        }
        
        if auto_install {
            println!("\n🔧 Auto-install is enabled. Attempting to install missing dependencies...");
            self.auto_install_suggestions(&suggestions, app).await?;
        } else {
            println!("\nTo automatically install these dependencies, run with --fix or set auto_install = true in config.");
            
            let should_install = Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt("Would you like to install these dependencies now?")
                .default(false)
                .interact()
                .map_err(|e| Error::Other(e.to_string()))?;
            
            if should_install {
                self.auto_install_suggestions(&suggestions, app).await?;
            }
        }
        
        Ok(())
    }
    
    /// FIX #18: Automatically install suggested packages.
    async fn auto_install_suggestions(&self, suggestions: &[String], app: &App) -> Result<()> {
        let mut success_count = 0;
        let mut fail_count = 0;
        
        for suggestion in suggestions {
            info!("DependencyBridge: Auto-installing {}", suggestion);
            
            let specs = match app.resolve_spec(suggestion).await {
                Ok(s) => s,
                Err(e) => {
                    warn!("Failed to resolve {}: {}", suggestion, e);
                    fail_count += 1;
                    continue;
                }
            };
            
            for spec in specs {
                if let Some(backend) = app.registry.get(&spec.backend) {
                    if let Some(installer) = backend.as_installable() {
                        match installer.install(&[spec.clone()], true).await {
                            Ok(_) => {
                                info!("Successfully installed {}", suggestion);
                                success_count += 1;
                                
                                // Record in state registry
                                let mut state = app.state.lock().await;
                                state.add(&spec.backend, &spec.name, None, spec.options.clone());
                                state.save()?;
                            }
                            Err(e) => {
                                warn!("Failed to install {}: {}", suggestion, e);
                                fail_count += 1;
                            }
                        }
                    }
                }
            }
        }
        
        println!("\n📦 Auto-install complete: {} installed, {} failed", success_count, fail_count);
        
        if fail_count > 0 && success_count == 0 {
            Err(Error::Other("Failed to install any dependencies".into()))
        } else {
            Ok(())
        }
    }
    
    /// Gets the description of the detected issue.
    fn get_description(&self, stderr: &str) -> Option<String> {
        self.db.find_mapping(stderr).map(|m| m.description.clone())
    }

    /// Prints suggestions to the console (legacy method).
    pub fn print_suggestions(&self, stderr: &str, current_backend: &str) {
        let suggestions = self.diagnose_failure(stderr, current_backend);
        if !suggestions.is_empty() {
            println!("\n💡 LiNix Insight: This build likely failed due to missing system headers.");
            println!("Try installing the following package(s) and running the command again:");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::core::CommandExecutor;

    #[test]
    fn test_diagnose_openssl_failure() {
        let bridge = DependencyBridge::new();
        let stderr = "fatal error: openssl/ssl.h: No such file or directory";
        let suggestions = bridge.diagnose_failure(stderr, "apt");
        assert!(!suggestions.is_empty());
        assert!(suggestions.contains(&"apt:libssl-dev".to_string()));
    }
    
    #[test]
    fn test_diagnose_cmake_failure() {
        let bridge = DependencyBridge::new();
        let stderr = "CMake Error: Could not find CMakeLists.txt";
        let suggestions = bridge.diagnose_failure(stderr, "dnf");
        assert!(suggestions.contains(&"dnf:cmake".to_string()));
    }
    
    #[test]
    fn test_diagnose_python_failure() {
        let bridge = DependencyBridge::new();
        let stderr = "fatal error: Python.h: No such file or directory";
        let suggestions = bridge.diagnose_failure(stderr, "apt");
        assert!(suggestions.contains(&"apt:python3-dev".to_string()));
    }
    
    #[test]
    fn test_no_match() {
        let bridge = DependencyBridge::new();
        let stderr = "Some random error message";
        let suggestions = bridge.diagnose_failure(stderr, "apt");
        assert!(suggestions.is_empty());
    }
    
    #[test]
    fn test_get_description() {
        let bridge = DependencyBridge::new();
        let stderr = "openssl/ssl.h: No such file";
        let desc = bridge.get_description(stderr);
        assert_eq!(desc, Some("Missing OpenSSL development headers".to_string()));
    }
}