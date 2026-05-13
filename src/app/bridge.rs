use crate::core::{Result, Error};
use regex::Regex;
use std::collections::HashMap;
use tracing::{info, debug};

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
            pattern: Regex::new(r"openssl/ssl\.h|libssl|cannot find -lssl|crypto").unwrap(),
            suggestions: ssl,
            description: "Missing OpenSSL development headers".into(),
        });

        // 2. Common Build Tooling (C/C++ Essentials)
        let mut build = HashMap::new();
        build.insert("apt".into(), "build-essential".into());
        build.insert("dnf".into(), "development-tools".into());
        build.insert("pacman".into(), "base-devel".into());
        
        mappings.push(BridgeMapping {
            pattern: Regex::new(r"cc1plus|stdio\.h|stdlib\.h|g\+\+ not found").unwrap(),
            suggestions: build,
            description: "Missing C++ compiler or standard library headers".into(),
        });

        // 3. Zlib Compression
        let mut zlib = HashMap::new();
        zlib.insert("apt".into(), "zlib1g-dev".into());
        zlib.insert("dnf".into(), "zlib-devel".into());
        zlib.insert("brew".into(), "zlib".into());
        
        mappings.push(BridgeMapping {
            pattern: Regex::new(r"zlib\.h|cannot find -lz").unwrap(),
            suggestions: zlib,
            description: "Missing zlib compression library".into(),
        });

        // 4. FFI / Header failures for dynamic languages
        let mut ffi = HashMap::new();
        ffi.insert("apt".into(), "libffi-dev".into());
        ffi.insert("dnf".into(), "libffi-devel".into());
        
        mappings.push(BridgeMapping {
            pattern: Regex::new(r"ffi\.h|foreign function interface").unwrap(),
            suggestions: ffi,
            description: "Missing Foreign Function Interface (FFI) headers".into(),
        });

        // 5. Sqlite3
        let mut sqlite = HashMap::new();
        sqlite.insert("apt".into(), "libsqlite3-dev".into());
        sqlite.insert("dnf".into(), "sqlite-devel".into());
        
        mappings.push(BridgeMapping {
            pattern: Regex::new(r"sqlite3\.h|cannot find -lsqlite3").unwrap(),
            suggestions: sqlite,
            description: "Missing SQLite development files".into(),
        });

        Self { mappings }
    }
}

/// The Dependency Bridge orchestrator.
/// Intercepts failures from language backends and attempts to provide 
/// actionable intelligence to the user.
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

        for mapping in &self.db.mappings {
            if mapping.pattern.is_match(stderr) {
                info!("DependencyBridge: Detected potential root cause: {}", mapping.description);
                
                // Attempt to find a suggestion for the user's current system backend
                if let Some(package) = mapping.suggestions.get(current_backend) {
                    suggestions.push(format!("{}:{}", current_backend, package));
                } else {
                    // Fallback: If current backend isn't mapped, suggest any available
                    for (b, p) in &mapping.suggestions {
                        debug!("DependencyBridge: Suggestion for other backend {}: {}", b, p);
                    }
                }
            }
        }

        suggestions
    }

    /// High-level method to print suggestions directly to the console.
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