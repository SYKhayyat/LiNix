// The "onboarder": let users teach LiNix a new CLI package manager entirely from
// config, with no source changes. A built-in backend is just a `ManagerConfig` (the
// argv templates) plus an `OutputParser` (Rust code). The onboarder makes BOTH data:
// the argv come straight from TOML, and the parser is a declarative `ParserSpec` (JSON /
// columns / regex / lines) interpreted at runtime by `ConfiguredParser`.
//
// Definitions live in `adapters/backends.toml` in the CONFIG REPO (7a/U1, U10) — never in the
// machine-local settings directory. A definition that cannot travel is a repo that fails on
// every machine but the one where somebody once hand-wrote the file, which contradicts the
// model's central claim. Its siblings in that folder are `settings.toml` (how to drive a
// settings store) and `bootstrap.toml` (how to obtain a manager).
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
    GenericBackendCore, GenericEnumerable, GenericInstallable, GenericQueryable, GenericRepoManager,
    GenericSearchable, GenericUpgradable,
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

/// One `[[backend]]` entry in `adapters/backends.toml`.
#[derive(Debug, Clone, Deserialize, Default)]
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

    // --- U2: the fields that make a custom backend a first-class peer of a built-in. ---
    // Every one is optional, and absent means *this backend cannot answer that* — never *the
    // answer is none*. A backend that cannot list its catalogue is not one whose catalogue is
    // empty; a `re:` against it is refused, not expanded to nothing. That distinction is the
    // whole point: "not configured" and "none" are different answers, and conflating them is
    // how a custom backend silently under-reports.
    /// Config-destroying removal (Debian's `purge`). Absent ⇒ `--purge` on this backend is
    /// refused rather than quietly doing an ordinary removal.
    pub purge_args: Option<Vec<String>>,
    /// Args that report the packages the OS treats as essential, for the removal guard.
    pub essential_args: Option<Vec<String>>,
    /// Args that print every installable name, one per line — what `re:` expands against.
    pub enumerate_args: Option<Vec<String>>,
    /// Binary for `enumerate_args`, when the catalogue lives in a separate program.
    pub enumerate_binary: Option<String>,
    /// Binary for the LIST commands, when the query tool is a separate program.
    pub list_binary: Option<String>,
    /// Binary for `search_args`, when search runs a different program.
    pub search_binary: Option<String>,
    /// Adding, removing and listing repositories (`repo:` lines).
    pub repo_add_args: Option<Vec<String>>,
    pub repo_remove_args: Option<Vec<String>>,
    pub repo_list_args: Option<Vec<String>>,
    /// Querying a package's dependencies (reverse-dependency reports, `why`).
    pub depends_args: Option<Vec<String>>,
    /// A dry run of the manager's own orphan verb, so `sync` can remove what it *would*
    /// remove. Absent ⇒ this backend cannot say, and a removal it cannot enumerate it does
    /// not make.
    pub orphan_dry_run: Option<OrphanDryRunDef>,
    /// How this backend reports the *manually* installed set, so `adopt` takes what the user
    /// chose and not the dependency graph. Absent ⇒ adoption skips this backend (the safe
    /// default that every custom backend had before U2).
    pub manual: Option<ManualListingDef>,
}

/// TOML mirror of [`crate::backends::generic::OrphanDryRun`].
#[derive(Debug, Clone, Deserialize)]
pub struct OrphanDryRunDef {
    pub binary: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// The line prefix that marks a would-be-removed package in the dry-run output
    /// (`apt-get autoremove --dry-run` prints `Remv libfoo …`).
    pub removes_line_prefix: String,
}

impl From<OrphanDryRunDef> for crate::backends::generic::OrphanDryRun {
    fn from(d: OrphanDryRunDef) -> Self {
        crate::backends::generic::OrphanDryRun {
            binary: d.binary,
            args: d.args,
            removes_line_prefix: d.removes_line_prefix,
        }
    }
}

/// TOML mirror of [`crate::backends::generic::ManualListing`], in the two shapes a user can
/// actually describe: "everything installed was user-requested", or "a command reports it".
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManualListingDef {
    /// Every installed package was user-requested (this manager installs no dependencies of
    /// its own): the installed set *is* the manual set.
    AllInstalled,
    /// A command reports the explicit set.
    Command {
        binary: Option<String>,
        #[serde(default)]
        args: Vec<String>,
        /// `same_as_installed` (reuse the list parser) or `bare_names` (one name per line).
        #[serde(default)]
        format: ManualFormatDef,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ManualFormatDef {
    #[default]
    SameAsInstalled,
    BareNames,
}

impl From<ManualListingDef> for crate::backends::generic::ManualListing {
    fn from(d: ManualListingDef) -> Self {
        use crate::backends::generic::{ManualFormat, ManualListing};
        match d {
            ManualListingDef::AllInstalled => ManualListing::AllInstalled,
            ManualListingDef::Command {
                binary,
                args,
                format,
            } => ManualListing::Command {
                binary,
                args,
                format: match format {
                    ManualFormatDef::SameAsInstalled => ManualFormat::SameAsInstalled,
                    ManualFormatDef::BareNames => ManualFormat::BareNames,
                },
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
struct CustomBackendsFile {
    #[serde(default)]
    backend: Vec<CustomBackendDef>,
}

/// True for a program LiNix will run for a custom backend: a plain command name found on
/// `$PATH`, or a path to an executable.
///
/// **A path is allowed (U16, ruled 2026-07-24).** A prefix that runs `/opt/vendor/thing` is
/// more useful than one confined to `$PATH`, and the cost — a definition that only works on the
/// machine with that path — is caught where it lands: `check health` reports a custom backend
/// whose binary is missing as a named diagnosis, not an unknown-backend error three layers
/// away. Whitespace and emptiness are still refused, because those are a malformed value rather
/// than a path.
fn is_valid_binary(binary: &str) -> bool {
    !binary.trim().is_empty() && !binary.chars().any(|c| c.is_whitespace())
}

/// Expand a leading `~` in a `binary` path to the user's home directory.
///
/// `which::which` — the availability check — does not expand `~`, so a `binary = "~/bin/tool"`
/// would read as a literal `~` directory and never be found. Expanded here, once, at the seam
/// where the definition becomes a runnable command. A `~` anywhere but the start is left alone:
/// only a leading one is the home-directory shorthand.
fn expand_binary(binary: &str) -> String {
    if let Some(rest) = binary.strip_prefix("~/").or_else(|| binary.strip_prefix("~\\")) {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    binary.to_string()
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
    load_custom_backends_from(
        reg,
        exec,
        &layout.adapter_backends_file(),
        &layout.locks_dir(),
    );
}

/// Reads `path`, checks it against the hook ledger in `locks_dir`, parses it, and registers
/// each valid backend. Returns the number of backends registered.
pub fn load_custom_backends_from(
    reg: &mut BackendRegistry,
    exec: &CommandExecutor,
    path: &Path,
    locks_dir: &Path,
) -> usize {
    // II.12, before the definitions become runnable argv: a shared repo that can define a
    // backend can run commands on every machine that clones it, which is the hook question
    // with a different file name. The check is at load rather than at the sync gate because a
    // registered backend is reachable from `search` and `list` too, which no sync guards.
    let Some(content) = read_approved_definitions(path, locks_dir) else {
        return 0;
    };

    let parsed: CustomBackendsFile = match toml::from_str(&content) {
        Ok(p) => p,
        Err(e) => {
            warn!("Ignoring malformed adapters/backends.toml: {}", e);
            return 0;
        }
    };
    register_custom_backends(reg, exec, parsed.backend)
}

/// An `adapters/` file's contents, or `None` when there is none or it is not approved.
///
/// Every reader of every adapter file goes through here — backends, `setting:` stores (K17),
/// bootstrap (7c) — so there is one approval, one refusal message, and no way to add a fourth
/// kind of definition that quietly skips the check.
pub fn read_approved_definitions(path: &Path, locks_dir: &Path) -> Option<String> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            warn!("Could not read custom backends file {:?}: {}", path, e);
            return None;
        }
    };
    if let Some(refusal) = unapproved(path, &content, locks_dir) {
        error!("{}", refusal);
        return None;
    }
    Some(content)
}

/// The II.12 refusal for this file's current contents, or `None` when it is approved.
fn unapproved(path: &Path, content: &str, locks_dir: &Path) -> Option<String> {
    use crate::core::hook_lock::{adapter_id, hash_script, refusal, HookLedger};
    let ledger = match HookLedger::load(&HookLedger::path_in(locks_dir)) {
        Ok(l) => l,
        Err(e) => return Some(format!("could not read the approval ledger: {}", e)),
    };
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("backends.toml");
    let id = adapter_id(name);
    let verdict = ledger.verdict(&id, &hash_script(content));
    if verdict.is_approved() {
        return None;
    }
    Some(refusal(&id, "adapter definition", &verdict))
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
                    "Skipping custom backend '{}': `binary = \"{}\"` is empty or contains \
                     whitespace, so it is not a command LiNix can run.",
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
        binary: def.binary.as_deref().map(expand_binary),
        install_args: def.install_args,
        remove_args: def.remove_args,
        list_args: def.list_args,
        // U2: a definition may now say how it reports its manual set. Absent stays
        // `Unsupported` — the safe default — so `adopt` skips a backend that has not opted in,
        // rather than risk adopting its dependency graph.
        manual: def.manual.map(Into::into).unwrap_or(ManualListing::Unsupported),
        essential_args: def.essential_args,
        search_args: def.search_args,
        search_binary: def.search_binary,
        enumerate_args: def.enumerate_args,
        enumerate_binary: def.enumerate_binary,
        list_binary: def.list_binary,
        upgrade_args: def.upgrade_args,
        update_args: def.update_args,
        purge_args: def.purge_args,
        orphan_dry_run: def.orphan_dry_run.map(Into::into),
        repo_add_args: def.repo_add_args,
        repo_remove_args: def.repo_remove_args,
        repo_list_args: def.repo_list_args,
        depends_args: def.depends_args,
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
    // U2: the capabilities a built-in gets, now reachable from a definition. A backend is a
    // repo manager only if it said how to add a repo, and enumerable only if it said how to
    // list its catalogue — so `repo:` and `re:` against a backend that did not opt in are
    // still refused, not silently no-ops.
    if core.config.repo_add_args.is_some() {
        builder = builder.with_repo_manager(Arc::new(GenericRepoManager { core: core.clone() }));
    }
    if core.config.enumerate_args.is_some() {
        builder = builder.with_enumerable(Arc::new(GenericEnumerable { core: core.clone() }));
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
            install_args: vec!["-S".into()],
            remove_args: vec!["-R".into()],
            list_args: vec!["-Qm".into()],
            ..Default::default()
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

    /// U2: a custom backend gains a capability only when its definition provides the fields
    /// for it — and gains it when it does. Absent stays absent (the safe default), present
    /// makes it a first-class peer.
    #[test]
    fn a_custom_backend_is_a_first_class_peer_when_it_says_so() {
        let (_, exec) = mock_exec();

        // A bare definition: install/remove/list only, so no repo manager, not enumerable,
        // adoption skips it.
        let mut plain = BackendRegistry::new();
        register_custom_backends(&mut plain, &exec, vec![firewall_def()]);
        let caps = plain.get("firewall").unwrap();
        assert!(!caps.is_repo_manager(), "a bare def is not a repo manager");
        assert!(caps.as_enumerable().is_none(), "a bare def cannot list a catalogue");

        // The same backend, now told how to manage repos and list its catalogue.
        let mut full = BackendRegistry::new();
        let def = CustomBackendDef {
            repo_add_args: Some(vec!["repo".into(), "add".into()]),
            repo_remove_args: Some(vec!["repo".into(), "rm".into()]),
            repo_list_args: Some(vec!["repo".into(), "list".into()]),
            enumerate_args: Some(vec!["list".into(), "--all".into()]),
            depends_args: Some(vec!["deps".into()]),
            manual: Some(ManualListingDef::AllInstalled),
            ..firewall_def()
        };
        register_custom_backends(&mut full, &exec, vec![def]);
        let caps = full.get("firewall").unwrap();
        assert!(caps.is_repo_manager(), "a def with repo args IS a repo manager");
        assert!(caps.as_enumerable().is_some(), "a def with enumerate args CAN list its catalogue");
    }

    fn firewall_def() -> CustomBackendDef {
        CustomBackendDef {
            name: "firewall".into(),
            binary: Some("ufw".into()),
            install_args: vec!["allow".into()],
            remove_args: vec!["delete".into(), "allow".into()],
            list_args: vec!["status".into()],
            ..Default::default()
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

    /// U16 (ruled 2026-07-24): a `binary` naming a path is ALLOWED — `/opt/vendor/thing` is a
    /// more useful prefix than one confined to `$PATH`, and a missing one is caught by
    /// `check health`, not refused at load.
    #[test]
    fn a_binary_that_names_a_path_is_accepted() {
        let (_, exec) = mock_exec();
        let mut reg = BackendRegistry::new();
        for path in ["/opt/vendor/ufw", "..\\ufw", "~/bin/ufw"] {
            let mut r = BackendRegistry::new();
            let def = CustomBackendDef {
                binary: Some(path.into()),
                ..firewall_def()
            };
            assert_eq!(
                register_custom_backends(&mut r, &exec, vec![def]),
                1,
                "`{}` should be accepted as a binary",
                path
            );
            reg = r;
        }
        let _ = reg;
    }

    /// ...but an empty or whitespace-bearing `binary` is still a malformed value, not a path,
    /// and is refused.
    #[test]
    fn an_empty_or_spaced_binary_is_still_refused() {
        let (_, exec) = mock_exec();
        for bad in ["", "   ", "ufw x"] {
            let mut reg = BackendRegistry::new();
            let def = CustomBackendDef {
                binary: Some(bad.into()),
                ..firewall_def()
            };
            assert_eq!(
                register_custom_backends(&mut reg, &exec, vec![def]),
                0,
                "`{}` was accepted as a binary",
                bad
            );
        }
    }

    /// `~/bin/tool` expands to the home directory, because the availability check does not.
    #[test]
    fn a_leading_tilde_expands_to_home() {
        let expanded = expand_binary("~/bin/tool");
        assert!(!expanded.starts_with('~'), "{}", expanded);
        assert!(expanded.ends_with("bin/tool") || expanded.ends_with("bin\\tool"), "{}", expanded);
        // A bare command and a `~` that is not a leading path segment are left untouched.
        assert_eq!(expand_binary("ufw"), "ufw");
        assert_eq!(expand_binary("/opt/x~y"), "/opt/x~y");
    }

    fn write_repo(dir: &Path, body: &str) -> std::path::PathBuf {
        let file = dir.join("backends.toml");
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
        use crate::core::hook_lock::{adapter_id, hash_script, HookLedger};
        let tmp = tempfile::tempdir().unwrap();
        let (_, exec) = mock_exec();
        write_repo(tmp.path(), PARU_TOML);
        let locks = tmp.path().join("locks");

        // Unapproved: nothing registers, however valid the definition is.
        let mut reg = BackendRegistry::new();
        let n = load_custom_backends_from(
            &mut reg,
            &exec,
            &tmp.path().join("backends.toml"),
            &locks,
        );
        assert_eq!(n, 0, "an unapproved definition file registered a backend");
        assert!(reg.get("paru").is_none());

        // What `linix lock` writes.
        let mut ledger = HookLedger::new();
        ledger.approve(&adapter_id("backends.toml"), &hash_script(PARU_TOML));
        ledger.save(&HookLedger::path_in(&locks)).unwrap();

        let mut reg = BackendRegistry::new();
        let n = load_custom_backends_from(
            &mut reg,
            &exec,
            &tmp.path().join("backends.toml"),
            &locks,
        );
        assert_eq!(n, 1);
        assert!(reg.get("paru").is_some());
    }

    /// And the case the ledger exists for: approved once, then edited. An added `[[backend]]`
    /// is a new command the repo can run, so one identity covers the whole file.
    #[test]
    fn an_edited_definition_file_stops_registering_until_it_is_re_approved() {
        use crate::core::hook_lock::{adapter_id, hash_script, HookLedger};
        let tmp = tempfile::tempdir().unwrap();
        let (_, exec) = mock_exec();
        let locks = tmp.path().join("locks");
        let mut ledger = HookLedger::new();
        ledger.approve(&adapter_id("backends.toml"), &hash_script(PARU_TOML));
        ledger.save(&HookLedger::path_in(&locks)).unwrap();

        let edited = format!("{}\n[[backend]]\nname = \"yay\"\ninstall_args = [\"-S\"]\n", PARU_TOML);
        write_repo(tmp.path(), &edited);

        let mut reg = BackendRegistry::new();
        let n = load_custom_backends_from(
            &mut reg,
            &exec,
            &tmp.path().join("backends.toml"),
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
                &tmp.path().join("backends.toml"),
                &tmp.path().join("locks"),
            ),
            0
        );
    }
}

/// U10: three files, one folder, each approved on its own.
#[cfg(test)]
mod adapter_folder_tests {
    use super::*;
    use crate::core::hook_lock::{adapter_id, hash_script, HookLedger};

    const BACKENDS: &str = "[[backend]]\nname = \"paru\"\ninstall_args = [\"-S\"]\nlist_args = [\"-Qm\"]\n";
    const SETTINGS: &str = "[[setting_store]]\nname = \"kde\"\ndetect = \"kwriteconfig6\"\nread = [\"kreadconfig6\"]\nwrite = [\"kwriteconfig6\"]\nreset = [\"kwriteconfig6\"]\n";

    fn approve(locks: &Path, file: &str, body: &str) {
        let path = HookLedger::path_in(locks);
        let mut l = HookLedger::load(&path).unwrap();
        l.approve(&adapter_id(file), &hash_script(body));
        l.save(&path).unwrap();
    }

    /// Approving one adapter file must not approve its siblings. They carry different argv and
    /// an edit to one is not a review of the other.
    #[test]
    fn each_adapter_file_is_approved_on_its_own() {
        let tmp = tempfile::tempdir().unwrap();
        let adapters = tmp.path().join("adapters");
        std::fs::create_dir_all(&adapters).unwrap();
        std::fs::write(adapters.join("backends.toml"), BACKENDS).unwrap();
        std::fs::write(adapters.join("settings.toml"), SETTINGS).unwrap();
        let locks = tmp.path().join("locks");

        // Approve only the backends file.
        approve(&locks, "backends.toml", BACKENDS);

        assert!(
            read_approved_definitions(&adapters.join("backends.toml"), &locks).is_some(),
            "the approved file did not load"
        );
        assert!(
            read_approved_definitions(&adapters.join("settings.toml"), &locks).is_none(),
            "approving backends.toml also approved settings.toml"
        );

        // Approve the sibling too, and now both load.
        approve(&locks, "settings.toml", SETTINGS);
        assert!(read_approved_definitions(&adapters.join("settings.toml"), &locks).is_some());
    }

    /// The whole point of the folder move: the definition travels with the repo, so it is read
    /// from the config root and nowhere machine-local.
    #[test]
    fn the_adapters_folder_is_inside_the_config_repo() {
        let cfg = crate::config::Config {
            config_root: std::path::PathBuf::from(if cfg!(windows) {
                r"C:\repo"
            } else {
                "/repo"
            }),
            ..Default::default()
        };
        let layout = cfg.layout();
        for f in [
            layout.adapter_backends_file(),
            layout.adapter_settings_file(),
            layout.adapter_bootstrap_file(),
        ] {
            assert!(f.starts_with(cfg.config_root()), "{:?} escaped the repo", f);
            assert!(f.parent().unwrap().ends_with("adapters"), "{:?}", f);
        }
    }
}
