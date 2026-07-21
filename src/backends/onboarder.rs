// The "onboarder": let users teach LiNix a new CLI package manager entirely from
// config, with no source changes. A built-in backend is just a `ManagerConfig` (the
// argv templates) plus an `OutputParser` (Rust code). The onboarder makes BOTH data:
// the argv come straight from TOML, and the parser is a declarative `ParserSpec` (JSON /
// columns / regex / lines) interpreted at runtime by `ConfiguredParser`.
//
// Definitions live in `~/.config/linix/custom_backends.toml`:
//
//     [[backend]]
//     name = "paru"                       # also the invoked binary
//     install_args = ["-S", "--noconfirm"]
//     remove_args  = ["-R", "--noconfirm"]
//     list_args    = ["-Qm"]
//     search_args  = ["-Ss"]
//     needs_root   = false
//     [backend.parser]
//     format = "columns"                  # "name version" per line
//     name_col = 0
//     version_col = 1
//
// Custom backends are registered LAST and never override a built-in (collisions are
// skipped with a warning), so a stray config can't hijack `apt` or `brew`.

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
use tracing::warn;


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
    /// Backend id AND the binary invoked (they must match for custom backends).
    pub name: String,
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
    pub orphan_args: Option<Vec<String>>,
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

/// True for a syntactically valid backend id: non-empty, no whitespace or path
/// separators (it becomes both a HashMap key and an executed command name).
fn is_valid_backend_name(name: &str) -> bool {
    !name.is_empty() && !name.chars().any(|c| c.is_whitespace()) && !name.contains(['/', '\\'])
}


/// Loads and registers custom backends from the default config path, if the file
/// exists. Never fails the program: a missing file is normal, and a malformed one is
/// logged and skipped so the built-in backends still come up.
pub fn load_default_custom_backends(reg: &mut BackendRegistry, exec: &CommandExecutor) {
    let path = crate::utils::safe_config_dir().join("custom_backends.toml");
    load_custom_backends_from(reg, exec, &path);
}

/// Reads `path`, parses it, and registers each valid backend. Returns the number of
/// backends registered (useful for tests/telemetry).
pub fn load_custom_backends_from(
    reg: &mut BackendRegistry,
    exec: &CommandExecutor,
    path: &Path,
) -> usize {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return 0,
        Err(e) => {
            warn!("Could not read custom backends file {:?}: {}", path, e);
            return 0;
        }
    };
    let parsed: CustomBackendsFile = match toml::from_str(&content) {
        Ok(p) => p,
        Err(e) => {
            warn!("Ignoring malformed custom_backends.toml: {}", e);
            return 0;
        }
    };
    register_custom_backends(reg, exec, parsed.backend)
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
        orphan_args: def.orphan_args,
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
            install_args: vec!["-S".into()],
            remove_args: vec!["-R".into()],
            list_args: vec!["-Qm".into()],
            search_args: vec![],
            upgrade_args: vec![],
            update_args: None,
            orphan_args: None,
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
}
