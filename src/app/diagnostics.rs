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
    /// `pattern`, compiled — once, when the rule is loaded.
    ///
    /// `Regex::new` builds and optimises an automaton; doing it inside the match loop
    /// recompiled **every** rule on **every** `diagnose()` call, and `diagnose` is called on
    /// every failed command. A pattern that will not compile is `None` and is skipped, which
    /// is what the old `if let Ok(re)` did anyway.
    #[serde(skip)]
    compiled: Option<Regex>,
}

impl DiagnosticRule {
    fn compile(&mut self) {
        self.compiled = Regex::new(&self.pattern).ok();
    }

    fn matches(&self, text: &str) -> bool {
        self.compiled.as_ref().is_some_and(|re| re.is_match(text))
    }
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
                    if let Ok(mut db) = serde_json::from_str::<Self>(&content) {
                        debug!("Loaded {} patterns from {:?}", db.rules.len(), db_path);
                        db.rules.iter_mut().for_each(DiagnosticRule::compile);
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

            compiled: None,
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

            compiled: None,
        });

        let mut zlib = HashMap::new();
        zlib.insert("apt".into(), "zlib1g-dev".into());
        zlib.insert("brew".into(), "zlib".into());
        rules.push(DiagnosticRule {
            pattern: r"zlib\.h|cannot find -lz".into(),
            suggestions: zlib,
            description: "Missing zlib compression development files".into(),

            compiled: None,
        });

        let mut db = Self { rules };
        db.rules.iter_mut().for_each(DiagnosticRule::compile);
        db
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
            if rule.matches(stderr) {
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
        suggestions
    }

    #[allow(clippy::too_many_arguments)]
    #[instrument(skip(self, registry, state, config, journal))]
    pub async fn handle_failure(
        &self,
        stderr: &str,
        current_backend: &str,
        registry: Arc<BackendRegistry>,
        state: Arc<Mutex<StateRegistry>>,
        config: &Config,
        journal: Arc<Mutex<crate::core::Journal>>,
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
            self.remediate(&suggestions, registry, state, config, journal)
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
                self.remediate(&suggestions, registry, state, config, journal)
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
        journal: Arc<Mutex<crate::core::Journal>>,
    ) -> Result<()> {
        let resolver = StateResolver::new(config, registry.clone(), false).await;

        for suggestion in suggestions {
            info!("installing {} to fix the failure", suggestion);
            let spec = resolver.parse_and_probe_spec(suggestion).await?;

            if let Some(b_cap) = registry.get(&spec.backend) {
                if let Some(installer) = b_cap.as_installable() {
                    // Dead code that installs software is the worst kind — it looks
                    // maintained and is never exercised — so it is held to the same rule as
                    // the live paths rather than exempted for being unreachable.
                    match crate::core::journalled(
                        &journal,
                        vec![crate::core::JournalAction::Install(spec.clone())],
                        installer.install(std::slice::from_ref(&spec), b_cap.sudo_for_write()),
                    )
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
                                    "diagnostics",
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
            if rule.matches(stderr) {
                return Some(rule.description.clone());
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
