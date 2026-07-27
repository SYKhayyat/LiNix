use crate::app::sync::resolver::StateResolver;
use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::core::{Error, Result, StateRegistry};
use dialoguer::{theme::ColorfulTheme, Confirm};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, instrument, trace, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticRule {
    pub pattern: String,
    /// backend id -> the package that fixes this failure on that backend.
    pub suggestions: HashMap<String, String>,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiagnosticDb {
    pub rules: Vec<DiagnosticRule>,
}

impl DiagnosticDb {
    pub async fn load(config: &Config) -> Self {
        let db_path = config.config_root().join("diagnostics.json");

        if tokio::fs::try_exists(&db_path).await.unwrap_or(false) {
            match tokio::fs::read_to_string(&db_path).await {
                Ok(content) => {
                    if let Ok(db) = serde_json::from_str::<Self>(&content) {
                        debug!("Loaded {} patterns from {:?}", db.rules.len(), db_path);
                        return db;
                    }
                }
                Err(e) => warn!("Failed to read Knowledge Base: {}", e),
            }
        }

        trace!("Falling back to internal seed rules.");
        Self::seed()
    }

    fn seed() -> Self {
        let mut rules = Vec::new();

        let mut ssl = HashMap::new();
        ssl.insert("apt".into(), "libssl-dev".into());
        ssl.insert("dnf".into(), "openssl-devel".into());
        ssl.insert("pacman".into(), "openssl".into());
        ssl.insert("brew".into(), "openssl@3".into());
        rules.push(DiagnosticRule {
            pattern: r"openssl/ssl\.h|libssl|cannot find -lssl|SSL_library_init".into(),
            suggestions: ssl,
            description: "Missing OpenSSL development headers".into(),
        });

        let mut build = HashMap::new();
        build.insert("apt".into(), "build-essential".into());
        build.insert("dnf".into(), "development-tools".into());
        build.insert("pacman".into(), "base-devel".into());
        build.insert("brew".into(), "gcc".into());
        rules.push(DiagnosticRule {
            pattern: r"cc1plus|stdio\.h|stdlib\.h|g\+\+ not found|make: command not found".into(),
            suggestions: build,
            description: "Missing C/C++ compiler or standard build orchestration tools".into(),
        });

        let mut zlib = HashMap::new();
        zlib.insert("apt".into(), "zlib1g-dev".into());
        zlib.insert("brew".into(), "zlib".into());
        rules.push(DiagnosticRule {
            pattern: r"zlib\.h|cannot find -lz".into(),
            suggestions: zlib,
            description: "Missing zlib compression development files".into(),
        });

        Self { rules }
    }
}

pub struct FailureDiagnosticEngine {
    db: DiagnosticDb,
}

impl FailureDiagnosticEngine {
    pub async fn init(config: &Config) -> Self {
        Self {
            db: DiagnosticDb::load(config).await,
        }
    }

    pub fn diagnose(&self, stderr: &str, current_backend: &str) -> Vec<String> {
        let mut suggestions = Vec::new();
        for rule in &self.db.rules {
            if let Ok(re) = Regex::new(&rule.pattern) {
                if re.is_match(stderr) {
                    info!("Match found: {}", rule.description);
                    if let Some(pkg) = rule.suggestions.get(current_backend) {
                        suggestions.push(format!("{}:{}", current_backend, pkg));
                    } else {
                        for (b, p) in &rule.suggestions {
                            suggestions.push(format!("{}:{}", b, p));
                        }
                    }
                    break;
                }
            }
        }
        suggestions
    }

    #[instrument(skip(self, registry, state, config))]
    pub async fn handle_failure(
        &self,
        stderr: &str,
        current_backend: &str,
        registry: Arc<BackendRegistry>,
        state: Arc<Mutex<StateRegistry>>,
        config: &Config,
        auto_install: bool,
    ) -> Result<()> {
        let suggestions = self.diagnose(stderr, current_backend);
        if suggestions.is_empty() {
            return Ok(());
        }

        println!("\nmissing dependency.");
        println!(
            "Identified Issue: {}",
            self.get_description(stderr)
                .unwrap_or_else(|| "Conflict".into())
        );

        println!("\nRemediation Suggestion:");
        for s in &suggestions {
            println!("  - install {}", s);
        }

        if auto_install {
            self.remediate(&suggestions, registry, state, config)
                .await?;
        } else {
            use std::io::IsTerminal;
            // This prompt installs software. Without the check, a scripted run got dialoguer's
            // `IO error: not a terminal` — safe, but it names neither what stopped nor the
            // flag that gets past it, which is the whole of the difference.
            if !std::io::stdin().is_terminal() {
                return Err(Error::Refused(
                    "Refusing to install remediation packages without confirmation in a \
                     non-interactive shell. Re-run with --yes to proceed."
                        .to_string(),
                ));
            }
            let res = tokio::task::spawn_blocking(move || {
                Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt("Would you like to execute remediation now?")
                    .default(false)
                    .interact()
            })
            .await
            .map_err(|e| Error::Other(format!("Join error: {}", e)))??;

            if res {
                self.remediate(&suggestions, registry, state, config)
                    .await?;
            }
        }
        Ok(())
    }

    async fn remediate(
        &self,
        suggestions: &[String],
        registry: Arc<BackendRegistry>,
        state: Arc<Mutex<StateRegistry>>,
        config: &Config,
    ) -> Result<()> {
        let resolver = StateResolver::new(config, registry.clone(), false).await;

        for suggestion in suggestions {
            info!("installing {} to fix the failure", suggestion);
            let spec = resolver.parse_and_probe_spec(suggestion).await?;

            if let Some(b_cap) = registry.get(&spec.backend) {
                if let Some(installer) = b_cap.as_installable() {
                    match installer
                        .install(std::slice::from_ref(&spec), b_cap.sudo_for_write())
                        .await
                    {
                        Ok(_) => {
                            // The save runs on a blocking thread and so needs an owned,
                            // 'static value: clone under the lock and drop it before the
                            // spawn, rather than holding the guard across an await.
                            let state_snapshot = {
                                let mut state_guard = state.lock().await;
                                state_guard.add(
                                    &spec.backend,
                                    &spec.name,
                                    None,
                                    HashMap::new(),
                                    Some("diagnostics".into()),
                                    false,
                                );
                                state_guard.clone()
                            };

                            tokio::task::spawn_blocking(move || state_snapshot.save())
                                .await
                                .map_err(|e| Error::Other(format!("Task panic: {}", e)))??;
                        }
                        Err(e) => {
                            warn!("Remediation FAILED for {}: {}", suggestion, e)
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn get_description(&self, stderr: &str) -> Option<String> {
        for rule in &self.db.rules {
            if let Ok(re) = Regex::new(&rule.pattern) {
                if re.is_match(stderr) {
                    return Some(rule.description.clone());
                }
            }
        }
        None
    }

    pub fn print_suggestions(&self, stderr: &str, current_backend: &str) {
        let suggestions = self.diagnose(stderr, current_backend);
        if !suggestions.is_empty() {
            println!(
                "\nmissing dependency: {} — try: linix install {}",
                suggestions.join(", "),
                suggestions.join(" ")
            );
        }
    }
}
