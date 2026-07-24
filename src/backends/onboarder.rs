// The "onboarder": let users teach LiNix a new CLI package manager entirely from
// config, with no source changes. A built-in backend is just a `ManagerConfig` (the
// argv templates) plus an `OutputParser` (Rust code). The onboarder makes BOTH data:
// the argv come straight from TOML, and the parser is a declarative `ParserSpec` (JSON /
// columns / regex / lines) interpreted at runtime by `ConfiguredParser`.
//
// Definitions live in `custom_backends.toml` in the CONFIG REPO, beside `priority` and
// `schedules` (7a/U1) — never in the machine-local settings directory. A definition that
// cannot travel is a repo that fails on every machine but the one where somebody once
// hand-wrote the file, which contradicts the model's central claim.
//
//     [[backend]]
//     name = "firewall"                   # the prefix a line is written with
//     binary = "ufw"                      # the program actually run (defaults to `name`)
//     install_args = ["allow"]
//     remove_args  = ["delete", "allow"]
//     list_args    = ["status", "numbered"]
//     search_args  = ["-Ss"]
//     needs_root   = false
//     [backend.parser]
//     format = "columns"                  # "name version" per line
//     name_col = 0
//     version_col = 1
//
// Custom backends are registered LAST and never override a built-in (collisions are
// skipped with a warning), so a stray config can't hijack `apt` or `brew`.
//
// **The file is argv a shared repo can execute, so it is II.12's supply-chain surface and
// goes through the hook ledger** — the same approval a hook needs, not a second mechanism.
// An unapproved or changed file registers nothing and says so; `linix lock` approves it.

use crate::backends::generic::{
    GenericBackendCore, GenericInstallable, GenericQueryable, GenericSearchable, GenericUpgradable,
    ManagerConfig, ManualListing, VersionPin,
};
use crate::backends::BackendRegistry;
use crate::core::{BackendCapabilities, CommandExecutor, Package};
use crate::parsers::utils::sanitize;
use crate::parsers::OutputParser;
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tracing::{error, warn};


fn default_name_key() -> String {
    "name".to_string()
}
fn default_name_group() -> usize {
    1
}

/// A data-driven description of how to turn a backend's stdout into packages.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "format", rename_all = "lowercase")]
pub enum ParserSpec {
    /// One package name per non-empty line; no version.
    Lines {
        #[serde(default)]
        skip_prefixes: Vec<String>,
    },
    /// Whitespace- (or `delimiter`-) separated columns.
    Columns {
        #[serde(default)]
        name_col: usize,
        version_col: Option<usize>,
        /// Number of leading lines to drop (e.g. a table header).
        #[serde(default)]
        skip_header: usize,
        /// Split on this exact string instead of runs of whitespace.
        delimiter: Option<String>,
        #[serde(default)]
        skip_prefixes: Vec<String>,
    },
    /// JSON: an array of objects (or, at `array_path`, a nested one). If the target node
    /// is an object rather than an array, its keys are taken as package names.
    Json {
        /// Dot path to the array/object, e.g. "results.packages". Empty = document root.
        array_path: Option<String>,
        #[serde(default = "default_name_key")]
        name_key: String,
        version_key: Option<String>,
    },
    /// A regex applied per line; capture groups supply the name and optional version.
    Regex {
        pattern: String,
        #[serde(default = "default_name_group")]
        name_group: usize,
        version_group: Option<usize>,
    },
}

impl Default for ParserSpec {
    fn default() -> Self {
        ParserSpec::Lines {
            skip_prefixes: Vec::new(),
        }
    }
}

impl ParserSpec {
    pub fn parse(&self, output: &str, backend: &str) -> Vec<Package> {
        match self {
            ParserSpec::Lines { skip_prefixes } => sanitize(output)
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !starts_with_any(l, skip_prefixes))
                .map(|l| Package::new(l, backend))
                .collect(),

            ParserSpec::Columns {
                name_col,
                version_col,
                skip_header,
                delimiter,
                skip_prefixes,
            } => sanitize(output)
                .lines()
                .skip(*skip_header)
                .filter_map(|line| {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || starts_with_any(trimmed, skip_prefixes) {
                        return None;
                    }
                    let cols: Vec<&str> = match delimiter {
                        Some(d) if !d.is_empty() => line.split(d.as_str()).map(str::trim).collect(),
                        _ => line.split_whitespace().collect(),
                    };
                    let name = cols.get(*name_col)?.trim();
                    if name.is_empty() {
                        return None;
                    }
                    match version_col
                        .and_then(|i| cols.get(i))
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                    {
                        Some(v) => Some(Package::with_version(name, v, backend)),
                        None => Some(Package::new(name, backend)),
                    }
                })
                .collect(),

            ParserSpec::Json {
                array_path,
                name_key,
                version_key,
            } => {
                let json: Value = serde_json::from_str(&sanitize(output)).unwrap_or_default();
                let node = match array_path {
                    Some(p) if !p.is_empty() => match navigate(&json, p) {
                        Some(n) => n,
                        None => return vec![],
                    },
                    _ => &json,
                };
                if let Some(arr) = node.as_array() {
                    arr.iter()
                        .filter_map(|item| json_package(item, name_key, version_key, backend))
                        .collect()
                } else if let Some(obj) = node.as_object() {
                    // Object shape: keys are the package names.
                    obj.keys().map(|k| Package::new(k, backend)).collect()
                } else {
                    vec![]
                }
            }

            ParserSpec::Regex {
                pattern,
                name_group,
                version_group,
            } => {
                let re = match Regex::new(pattern) {
                    Ok(re) => re,
                    Err(e) => {
                        warn!("Custom backend '{}': invalid regex: {}", backend, e);
                        return vec![];
                    }
                };
                sanitize(output)
                    .lines()
                    .filter_map(|line| {
                        let caps = re.captures(line)?;
                        let name = caps.get(*name_group)?.as_str().trim();
                        if name.is_empty() {
                            return None;
                        }
                        match version_group
                            .and_then(|g| caps.get(g))
                            .map(|m| m.as_str().trim())
                            .filter(|s| !s.is_empty())
                        {
                            Some(v) => Some(Package::with_version(name, v, backend)),
                            None => Some(Package::new(name, backend)),
                        }
                    })
                    .collect()
            }
        }
    }
}

fn starts_with_any(line: &str, prefixes: &[String]) -> bool {
    prefixes
        .iter()
        .any(|p| !p.is_empty() && line.starts_with(p))
}

/// Walks a dot-separated path (`a.b.c`) through a JSON document.
fn navigate<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = root;
    for seg in path.split('.').filter(|s| !s.is_empty()) {
        cur = cur.get(seg)?;
    }
    Some(cur)
}

fn json_package(
    item: &Value,
    name_key: &str,
    version_key: &Option<String>,
    backend: &str,
) -> Option<Package> {
    let name = item.get(name_key)?.as_str()?;
    if name.is_empty() {
        return None;
    }
    let version = version_key
        .as_deref()
        .and_then(|k| item.get(k))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    Some(match version {
        Some(v) => Package::with_version(name, v, backend),
        None => Package::new(name, backend),
    })
}

/// The `OutputParser` used by every onboarded backend: it delegates installed/search
/// parsing to two [`ParserSpec`]s.
pub struct ConfiguredParser {
    pub backend: String,
    pub installed: ParserSpec,
    pub search: ParserSpec,
}

impl OutputParser for ConfiguredParser {
    fn parse_installed(&self, output: &str) -> Vec<Package> {
        self.installed.parse(output, &self.backend)
    }
    fn parse_search(&self, output: &str) -> Vec<Package> {
        self.search.parse(output, &self.backend)
    }
}


/// A user's version-pin choice, mirrored for `serde` (the runtime [`VersionPin`] is not
/// `Deserialize`).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum VersionPinDef {
    /// A single token, e.g. `{name}=={version}`.
    Inline { template: String },
    /// The bare name followed by flag args, e.g. `["--version", "{version}"]`.
    Flag { args: Vec<String> },
}

impl From<VersionPinDef> for VersionPin {
    fn from(d: VersionPinDef) -> Self {
        match d {
            VersionPinDef::Inline { template } => VersionPin::Inline(template),
            VersionPinDef::Flag { args } => VersionPin::Flag(args),
        }
    }
}

/// One `[[backend]]` entry in `custom_backends.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct CustomBackendDef {
    /// The prefix a line is written with — `firewall:22/tcp`.
    pub name: String,
    /// The program actually run. Absent means the name is the command, which is what every
    /// definition said before XIII.12 split the two.
    pub binary: Option<String>,
    #[serde(default)]
    pub install_args: Vec<String>,
    #[serde(default)]
    pub remove_args: Vec<String>,
    #[serde(default)]
    pub list_args: Vec<String>,
    #[serde(default)]
    pub search_args: Vec<String>,
    #[serde(default)]
    pub upgrade_args: Vec<String>,
    pub update_args: Option<Vec<String>>,
    #[serde(default)]
    pub needs_root: bool,
    #[serde(default)]
    pub is_exclusive: bool,
    pub version_pin: Option<VersionPinDef>,
    /// How to parse `list` output (defaults to one name per line).
    pub parser: Option<ParserSpec>,
    /// How to parse `search` output (defaults to the same as `parser`).
    pub search_parser: Option<ParserSpec>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct CustomBackendsFile {
    #[serde(default)]
    backend: Vec<CustomBackendDef>,
}

/// True for a program name LiNix will run for a custom backend.
///
/// A path is refused rather than resolved: whether `binary = "/opt/vendor/thing"` is allowed
/// is **U16**, still open, and a definition naming an absolute path is one that works on one
/// machine — the exact property 7a moved this file to fix. Refusing keeps the question open;
/// running it would answer it in code.
fn is_valid_binary(binary: &str) -> bool {
    !binary.is_empty()
        && !binary.chars().any(|c| c.is_whitespace())
        && !binary.contains(['/', '\\'])
}

/// True for a syntactically valid backend id: non-empty, no whitespace or path
/// separators (it becomes both a HashMap key and an executed command name).
fn is_valid_backend_name(name: &str) -> bool {
    !name.is_empty()
        && !name.chars().any(|c| c.is_whitespace())
        && !name.contains(['/', '\\'])
        // A comma separates the managers in a chain and a colon separates the prefix from
        // the name, so a backend containing either could never be written on a line.
        && !name.contains([',', ':'])
        && !crate::config::grammar::RESERVED_BACKEND_NAMES.contains(&name)
}


/// Loads and registers the config repo's custom backends. Never fails the program: a missing
/// file is normal, and a malformed or unapproved one is reported and skipped so the built-in
/// backends still come up — including `linix lock`, which is how an unapproved file is fixed.
pub fn load_default_custom_backends(
    reg: &mut BackendRegistry,
    exec: &CommandExecutor,
    cfg: &crate::config::Config,
) {
    let layout = cfg.layout();
    load_custom_backends_from(reg, exec, &layout.custom_backends_file(), &layout.locks_dir());
}

/// Reads `path`, checks it against the hook ledger in `locks_dir`, parses it, and registers
/// each valid backend. Returns the number of backends registered.
pub fn load_custom_backends_from(
    reg: &mut BackendRegistry,
    exec: &CommandExecutor,
    path: &Path,
    locks_dir: &Path,
) -> usize {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return 0,
        Err(e) => {
            warn!("Could not read custom backends file {:?}: {}", path, e);
            return 0;
        }
    };

    // II.12, before the definitions become runnable argv: a shared repo that can define a
    // backend can run commands on every machine that clones it, which is the hook question
    // with a different file name. The check is here rather than at the sync gate because a
    // registered backend is reachable from `search` and `list` too, which no sync guards.
    if let Some(refusal) = unapproved(&content, locks_dir) {
        error!("{}", refusal);
        return 0;
    }

    let parsed: CustomBackendsFile = match toml::from_str(&content) {
        Ok(p) => p,
        Err(e) => {
            warn!("Ignoring malformed custom_backends.toml: {}", e);
            return 0;
        }
    };
    register_custom_backends(reg, exec, parsed.backend)
}

/// The II.12 refusal for this file's current contents, or `None` when it is approved.
fn unapproved(content: &str, locks_dir: &Path) -> Option<String> {
    use crate::core::hook_lock::{backends_id, hash_script, refusal, HookLedger};
    let ledger = match HookLedger::load(&HookLedger::path_in(locks_dir)) {
        Ok(l) => l,
        Err(e) => return Some(format!("could not read the approval ledger: {}", e)),
    };
    let id = backends_id();
    let verdict = ledger.verdict(&id, &hash_script(content));
    if verdict.is_approved() {
        return None;
    }
    Some(refusal(&id, "custom backend definition", &verdict))
}

/// Registers a set of already-parsed definitions. Invalid names and collisions with an
/// existing (built-in or earlier custom) backend are skipped with a warning.
pub fn register_custom_backends(
    reg: &mut BackendRegistry,
    exec: &CommandExecutor,
    defs: Vec<CustomBackendDef>,
) -> usize {
    let mut count = 0;
    for def in defs {
        if !is_valid_backend_name(&def.name) {
            warn!("Skipping custom backend with invalid name: '{}'", def.name);
            continue;
        }
        if let Some(binary) = &def.binary {
            if !is_valid_binary(binary) {
                warn!(
                    "Skipping custom backend '{}': `binary = \"{}\"` is not a plain command \
                     name. A path is refused — a definition that names one only works on the \
                     machine that has it there, and this file travels with the repo.",
                    def.name, binary
                );
                continue;
            }
        }
        if reg.get(&def.name).is_some() {
            warn!(
                "Skipping custom backend '{}': a backend with that name already exists",
                def.name
            );
            continue;
        }
        reg.register(Arc::new(build_capabilities(def, exec)));
        count += 1;
    }
    count
}

/// Turns one definition into a fully-wired [`BackendCapabilities`] over the generic
/// backend machinery. Capabilities are attached only for the operations the definition
/// actually specifies (e.g. no `search_args` ⇒ not searchable).
fn build_capabilities(def: CustomBackendDef, exec: &CommandExecutor) -> BackendCapabilities {
    let has_install = !def.install_args.is_empty();
    let has_list = !def.list_args.is_empty();
    let has_search = !def.search_args.is_empty();
    let has_upgrade = !def.upgrade_args.is_empty();

    let parser = ConfiguredParser {
        backend: def.name.clone(),
        installed: def.parser.clone().unwrap_or_default(),
        search: def.search_parser.or(def.parser).unwrap_or_default(),
    };

    let config = ManagerConfig {
        name: def.name.clone(),
        binary: def.binary,
        install_args: def.install_args,
        remove_args: def.remove_args,
        list_args: def.list_args,
        // A user-defined backend describes an install/remove/list command set; nothing in
        // that definition says whether its lister reports dependencies too. Don't assume —
        // `adopt` skips custom backends rather than risk adopting a dependency graph.
        manual: ManualListing::Unsupported,
        essential_args: None,
        search_args: def.search_args,
        search_binary: None,
        // A custom backend describes one CLI; nothing in that description can promise a
        // complete catalogue, so `re:` does not apply to one.
        enumerate_args: None,
        enumerate_binary: None,
        list_binary: None,
        upgrade_args: def.upgrade_args,
        update_args: def.update_args,
        purge_args: None,
        // A definition that names an `autoremove` verb cannot say what that verb would
        // delete, and a removal LiNix cannot enumerate is one it does not make.
        orphan_dry_run: None,
        repo_add_args: None,
        repo_remove_args: None,
        repo_list_args: None,
        depends_args: None,
        version_pin: def.version_pin.map(Into::into),
        needs_root: def.needs_root,
        is_exclusive: def.is_exclusive,
        flag_map: HashMap::new(),
    };

    let core = Arc::new(GenericBackendCore {
        name: def.name,
        executor: exec.duplicate(),
        config,
        parser: Arc::new(parser),
    });

    let mut builder =
        BackendCapabilities::builder(core.clone()).with_metadata_provider(core.clone());
    if has_install {
        builder = builder.with_installable(Arc::new(GenericInstallable { core: core.clone() }));
    }
    if has_list {
        builder = builder.with_queryable(Arc::new(GenericQueryable { core: core.clone() }));
    }
    if has_search {
        builder = builder.with_searchable(Arc::new(GenericSearchable { core: core.clone() }));
    }
    if has_upgrade {
        builder = builder.with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }));
    }
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_the_prefix_grammar_already_spends_is_refused() {
        // `re` and `list` mean something in a `backend:name` prefix, so a backend answering
        // to one could never be reached by a line: `list:rg` would keep meaning the priority
        // file. Refusing the name is the only place that can be said out loud.
        assert!(!is_valid_backend_name("re"));
        assert!(!is_valid_backend_name("list"));
        // A comma splits a chain and a colon splits the prefix, so neither can be in a name.
        assert!(!is_valid_backend_name("apt,dnf"));
        assert!(!is_valid_backend_name("we:ird"));
        // A hyphen is fine, and has to be: `nix-env` and `apt-get` are real names, which is
        // why a chain is comma-separated.
        assert!(is_valid_backend_name("nix-env"));
    }

    #[test]
    fn columns_parser_extracts_name_and_version() {
        let spec = ParserSpec::Columns {
            name_col: 0,
            version_col: Some(1),
            skip_header: 0,
            delimiter: None,
            skip_prefixes: vec![],
        };
        let pkgs = spec.parse("ripgrep 13.0.0\nbat 0.24.0\n", "custom");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "ripgrep");
        assert_eq!(pkgs[0].version.as_deref(), Some("13.0.0"));
        assert_eq!(pkgs[0].backend, "custom");
    }

    #[test]
    fn columns_parser_skips_header_and_prefixes() {
        let spec = ParserSpec::Columns {
            name_col: 0,
            version_col: Some(1),
            skip_header: 1,
            delimiter: Some("|".to_string()),
            skip_prefixes: vec!["#".to_string()],
        };
        let pkgs = spec.parse("NAME|VER\ngit|2.40\n# comment|x\ncurl|8.1\n", "c");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "git");
        assert_eq!(pkgs[1].name, "curl");
    }

    #[test]
    fn lines_parser_one_name_per_line() {
        let spec = ParserSpec::Lines {
            skip_prefixes: vec!["==".to_string()],
        };
        let pkgs = spec.parse("foo\n== legend\nbar\n\n", "c");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "foo");
        assert_eq!(pkgs[1].name, "bar");
    }

    #[test]
    fn json_parser_array_of_objects_with_path() {
        let spec = ParserSpec::Json {
            array_path: Some("results".to_string()),
            name_key: "name".to_string(),
            version_key: Some("version".to_string()),
        };
        let out =
            r#"{"results":[{"name":"httpie","version":"3.2"},{"name":"jq","version":"1.7"}]}"#;
        let pkgs = spec.parse(out, "c");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "httpie");
        assert_eq!(pkgs[0].version.as_deref(), Some("3.2"));
    }

    #[test]
    fn json_parser_object_keys_as_names() {
        let spec = ParserSpec::Json {
            array_path: None,
            name_key: default_name_key(),
            version_key: None,
        };
        let pkgs = spec.parse(r#"{"numpy":[],"pandas":[]}"#, "c");
        assert_eq!(pkgs.len(), 2);
        assert!(pkgs.iter().any(|p| p.name == "numpy"));
    }

    #[test]
    fn regex_parser_named_captures() {
        let spec = ParserSpec::Regex {
            pattern: r"^(\S+)\s+v(\d[\d.]*)$".to_string(),
            name_group: 1,
            version_group: Some(2),
        };
        let pkgs = spec.parse("exa v0.10.1\nripgrep v13.0.0\n", "c");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[1].name, "ripgrep");
        assert_eq!(pkgs[1].version.as_deref(), Some("13.0.0"));
    }

    #[test]
    fn registers_valid_and_skips_collisions_and_bad_names() {
        let exec = CommandExecutor::new(true, false);
        let mut reg = BackendRegistry::new();

        let good = CustomBackendDef {
            name: "paru".into(),
            binary: None,
            install_args: vec!["-S".into()],
            remove_args: vec!["-R".into()],
            list_args: vec!["-Qm".into()],
            search_args: vec![],
            upgrade_args: vec![],
            update_args: None,
            needs_root: false,
            is_exclusive: false,
            version_pin: None,
            parser: None,
            search_parser: None,
        };
        let bad_name = CustomBackendDef {
            name: "bad name/x".into(),
            ..good.clone()
        };
        let collision = CustomBackendDef {
            name: "paru".into(),
            ..good.clone()
        };

        let n = register_custom_backends(&mut reg, &exec, vec![good, bad_name, collision]);
        assert_eq!(
            n, 1,
            "only the first valid, non-colliding backend registers"
        );

        let caps = reg.get("paru").expect("paru registered");
        assert!(caps.is_installable());
        assert!(caps.is_queryable());
        // no search_args ⇒ not searchable; no upgrade_args ⇒ not upgradable
        assert!(!caps.is_searchable());
        assert!(!caps.is_upgradable());
        assert!(caps.is_metadata_provider());
    }

    fn firewall_def() -> CustomBackendDef {
        CustomBackendDef {
            name: "firewall".into(),
            binary: Some("ufw".into()),
            install_args: vec!["allow".into()],
            remove_args: vec!["delete".into(), "allow".into()],
            list_args: vec!["status".into()],
            search_args: vec![],
            upgrade_args: vec![],
            update_args: None,
            needs_root: false,
            is_exclusive: false,
            version_pin: None,
            parser: None,
            search_parser: None,
        }
    }

    fn mock_exec() -> (Arc<crate::core::executor::MockExecutor>, CommandExecutor) {
        use dashmap::DashMap;
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(crate::core::executor::MockExecutor::new(vfs.clone()));
        let exec = CommandExecutor::with_layer(
            true,
            false,
            mock.clone(),
            vfs,
            Arc::new(DashMap::new()),
        );
        (mock, exec)
    }

    /// XIII.12: the prefix a line is written with and the program that runs are two facts.
    /// `firewall:22/tcp` runs `ufw`, and every verb has to agree about that — an install that
    /// ran `ufw` while the removal ran `firewall` would leave a rule nothing can take back.
    #[tokio::test]
    async fn a_name_that_differs_from_its_binary_runs_the_binary_on_every_verb() {
        let (mock, exec) = mock_exec();
        let mut reg = BackendRegistry::new();
        assert_eq!(register_custom_backends(&mut reg, &exec, vec![firewall_def()]), 1);
        let caps = reg.get("firewall").expect("firewall registered");

        caps.as_installable()
            .unwrap()
            .install(
                &[crate::core::PackageSpec {
                    name: "22/tcp".into(),
                    backend: "firewall".into(),
                    options: HashMap::new(),
                    requires: vec![],
                    present: true,
                }],
                false,
            )
            .await
            .unwrap();
        caps.as_queryable().unwrap().list_installed().await.unwrap();
        caps.as_installable()
            .unwrap()
            .remove(&["22/tcp".to_string()], false)
            .await
            .unwrap();

        let calls = mock.get_calls().await;
        assert_eq!(calls.len(), 3, "{:?}", calls);
        assert!(calls.iter().all(|c| c.starts_with("ufw ")), "{:?}", calls);
        assert!(calls.iter().any(|c| c.contains("allow 22/tcp")), "{:?}", calls);
        assert!(calls.iter().any(|c| c.contains("status")), "{:?}", calls);
        assert!(calls.iter().any(|c| c.contains("delete allow 22/tcp")), "{:?}", calls);
        // And the backend still answers to the name a line is written with.
        assert_eq!(caps.name(), "firewall");
    }

    /// U16 stays open: a `binary` naming a path is refused rather than resolved, because a
    /// definition that only works where `/opt/vendor/thing` exists is the machine-local
    /// problem 7a moved this file to fix.
    #[test]
    fn a_binary_that_names_a_path_is_refused() {
        let (_, exec) = mock_exec();
        let mut reg = BackendRegistry::new();
        for path in ["/opt/vendor/ufw", "..\\ufw", "ufw x"] {
            let def = CustomBackendDef {
                binary: Some(path.into()),
                ..firewall_def()
            };
            assert_eq!(
                register_custom_backends(&mut reg, &exec, vec![def]),
                0,
                "`{}` was accepted as a binary",
                path
            );
        }
    }

    fn write_repo(dir: &Path, body: &str) -> std::path::PathBuf {
        let file = dir.join("custom_backends.toml");
        std::fs::write(&file, body).unwrap();
        file
    }

    const PARU_TOML: &str = r#"
[[backend]]
name = "paru"
install_args = ["-S"]
remove_args = ["-R"]
list_args = ["-Qm"]
"#;

    /// 7a: the definition travels with the repo. A machine that has never seen this file
    /// registers the backend from it — after `linix lock`, because the file is argv the repo
    /// can run and that is II.12's question, not a new one.
    #[test]
    fn a_repo_definition_registers_once_it_is_approved() {
        use crate::core::hook_lock::{backends_id, hash_script, HookLedger};
        let tmp = tempfile::tempdir().unwrap();
        let (_, exec) = mock_exec();
        write_repo(tmp.path(), PARU_TOML);
        let locks = tmp.path().join("locks");

        // Unapproved: nothing registers, however valid the definition is.
        let mut reg = BackendRegistry::new();
        let n = load_custom_backends_from(
            &mut reg,
            &exec,
            &tmp.path().join("custom_backends.toml"),
            &locks,
        );
        assert_eq!(n, 0, "an unapproved definition file registered a backend");
        assert!(reg.get("paru").is_none());

        // What `linix lock` writes.
        let mut ledger = HookLedger::new();
        ledger.approve(&backends_id(), &hash_script(PARU_TOML));
        ledger.save(&HookLedger::path_in(&locks)).unwrap();

        let mut reg = BackendRegistry::new();
        let n = load_custom_backends_from(
            &mut reg,
            &exec,
            &tmp.path().join("custom_backends.toml"),
            &locks,
        );
        assert_eq!(n, 1);
        assert!(reg.get("paru").is_some());
    }

    /// And the case the ledger exists for: approved once, then edited. An added `[[backend]]`
    /// is a new command the repo can run, so one identity covers the whole file.
    #[test]
    fn an_edited_definition_file_stops_registering_until_it_is_re_approved() {
        use crate::core::hook_lock::{backends_id, hash_script, HookLedger};
        let tmp = tempfile::tempdir().unwrap();
        let (_, exec) = mock_exec();
        let locks = tmp.path().join("locks");
        let mut ledger = HookLedger::new();
        ledger.approve(&backends_id(), &hash_script(PARU_TOML));
        ledger.save(&HookLedger::path_in(&locks)).unwrap();

        let edited = format!("{}\n[[backend]]\nname = \"yay\"\ninstall_args = [\"-S\"]\n", PARU_TOML);
        write_repo(tmp.path(), &edited);

        let mut reg = BackendRegistry::new();
        let n = load_custom_backends_from(
            &mut reg,
            &exec,
            &tmp.path().join("custom_backends.toml"),
            &locks,
        );
        assert_eq!(n, 0, "an edited file kept running on the old approval");
        assert!(reg.get("paru").is_none(), "the unchanged half kept running too");
    }

    /// A missing file is the ordinary case, not a refusal: nothing is approved and nothing
    /// needs to be.
    #[test]
    fn no_definition_file_is_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, exec) = mock_exec();
        let mut reg = BackendRegistry::new();
        assert_eq!(
            load_custom_backends_from(
                &mut reg,
                &exec,
                &tmp.path().join("custom_backends.toml"),
                &tmp.path().join("locks"),
            ),
            0
        );
    }
}
