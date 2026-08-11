use crate::config::Config;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, trace, warn};

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
        ssl.insert("apt".to_string(), "libssl-dev".into());
        ssl.insert("dnf".to_string(), "openssl-devel".into());
        ssl.insert("pacman".to_string(), "openssl".into());
        ssl.insert("brew".to_string(), "openssl@3".into());
        rules.push(DiagnosticRule {
            pattern: r"openssl/ssl\.h|libssl|cannot find -lssl|SSL_library_init".into(),
            suggestions: ssl,
            description: "Missing OpenSSL development headers".into(),

            compiled: None,
        });

        let mut build = HashMap::new();
        build.insert("apt".to_string(), "build-essential".into());
        build.insert("dnf".to_string(), "development-tools".into());
        build.insert("pacman".to_string(), "base-devel".into());
        build.insert("brew".to_string(), "gcc".into());
        rules.push(DiagnosticRule {
            pattern: r"cc1plus|stdio\.h|stdlib\.h|g\+\+ not found|make: command not found".into(),
            suggestions: build,
            description: "Missing C/C++ compiler or standard build orchestration tools".into(),

            compiled: None,
        });

        let mut zlib = HashMap::new();
        zlib.insert("apt".to_string(), "zlib1g-dev".into());
        zlib.insert("brew".to_string(), "zlib".into());
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

    // `handle_failure` and `remediate` lived here: 115 lines that prompted with dialoguer and
    // then **installed packages**, writing them into the state registry with
    // `source: "diagnostics"`. Nothing called either of them, on any path, ever.
    //
    // Dead code that installs software is the worst kind — it looks maintained, it is held to
    // every rule the live paths are (`Y14` gave it a write-ahead record while it was already
    // unreachable), and it is exercised by nothing. `suggestions_for` above is the live half:
    // a failure's advice is *printed*, and what to do about it is the user's.

    // `get_description` went with them, and the compiler is what found it: it had exactly one
    // caller, and that caller was `handle_failure`. Dead code hides dead code.

    pub fn print_suggestions(&self, stderr: &str, current_backend: &str) {
        let suggestions = self.diagnose(stderr, current_backend);
        if !suggestions.is_empty() {
            println!(
                "\nmissing dependency: {} — try: shall install {}",
                suggestions.join(", "),
                suggestions.join(" ")
            );
        }
    }
}
