// Cross-backend "insight" commands that are only possible because Shall sits above
// every ecosystem at once:
//
//   * `audit` — one security scan across every managed package (apt, npm, pip, cargo,
//               gem, …) via the OSV.dev vulnerability database.
//   * `sbom`  — a single CycloneDX software bill of materials spanning all backends.
//   * `why`   — provenance (which manifest/module/imperative action pulled a package in)
//               plus cross-package reverse dependencies.

use crate::backends::BackendRegistry;
use crate::config::Config;
use crate::core::LockFile;
use crate::core::{Error, Output, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::debug;

/// A managed package with its best-known concrete version resolved from the live backend.
#[derive(Debug, Clone)]
pub struct ResolvedPkg {
    pub backend: String,
    pub name: String,
    pub version: Option<String>,
}

/// Resolve every managed package to its live installed version (falling back to the
/// version recorded in the state registry). Shared by `audit` and `sbom`.
async fn resolve_managed(
    state: &tokio::sync::Mutex<crate::core::StateRegistry>,
    registry: &crate::backends::BackendRegistry,
    max_parallel: usize,
) -> Vec<ResolvedPkg> {
    // **One implementation, in `export`.** This was a byte-for-byte second copy of that loop,
    // and both were serial — so fixing the fan-out in one would have left the other at 1.0×.
    crate::app::export::managed_pkgs(state, registry, max_parallel)
        .await
        .into_iter()
        .map(|(backend, name, version)| ResolvedPkg {
            backend,
            name,
            version,
        })
        .collect()
}

/// Returns None for backends with no standardized purl type — those must be omitted from
/// SBOM output rather than guessed at.
fn purl_type(backend: &str) -> Option<&'static str> {
    Some(match backend {
        "cargo" => "cargo",
        "npm" | "pnpm" | "yarn" | "bun" => "npm",
        "pip" | "pipx" => "pypi",
        "gem" => "gem",
        "go" => "golang",
        "composer" => "composer",
        "pub" => "pub",
        "mix" => "hex",
        "cabal" | "stack" => "hackage",
        "luarocks" => "luarocks",
        "apt" => "deb",
        "brew" => "brew",
        "nix" => "nix",
        _ => return None,
    })
}

/// Build a purl string for a package, if its backend maps to a known type.
fn purl(backend: &str, name: &str, version: Option<&str>) -> Option<String> {
    let ty = purl_type(backend)?;
    Some(match version {
        Some(v) if !v.is_empty() => format!("pkg:{}/{}@{}", ty, name, v),
        _ => format!("pkg:{}/{}", ty, name),
    })
}

/// Map a Shall backend to its OSV.dev ecosystem identifier for vulnerability queries.
/// Returns None for backends OSV does not cover (so we skip them honestly).
fn osv_ecosystem(backend: &str) -> Option<&'static str> {
    Some(match backend {
        "cargo" => "crates.io",
        "npm" | "pnpm" | "yarn" | "bun" => "npm",
        "pip" | "pipx" => "PyPI",
        "gem" => "RubyGems",
        "go" => "Go",
        "composer" => "Packagist",
        "pub" => "Pub",
        "mix" => "Hex",
        "apt" => "Debian",
        _ => return None,
    })
}

/// True for a concrete, queryable version (not empty / floating).
fn is_concrete(v: &str) -> bool {
    !v.is_empty() && v != "latest" && v != "*" && v != "unknown"
}

/// Build a CycloneDX 1.5 document from resolved packages. Pure — unit tested.
fn build_cyclonedx(pkgs: &[ResolvedPkg]) -> Value {
    let components: Vec<Value> = pkgs
        .iter()
        .map(|p| {
            let mut c = json!({
                "type": "application",
                "name": p.name,
                "properties": [{ "name": "shall:backend", "value": p.backend }],
            });
            if let Some(v) = &p.version {
                if !v.is_empty() {
                    c["version"] = json!(v);
                }
            }
            if let Some(pu) = purl(&p.backend, &p.name, p.version.as_deref()) {
                c["purl"] = json!(pu);
            }
            c
        })
        .collect();

    json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "version": 1,
        "metadata": { "tools": [{ "vendor": "Shall", "name": "shall" }] },
        "components": components,
    })
}

/// Emit a CycloneDX SBOM of every managed package, across all backends, as pretty JSON.
pub async fn sbom(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    state: &tokio::sync::Mutex<crate::core::StateRegistry>,
) -> Result<String> {
    let pkgs = resolve_managed(state, registry, config.max_parallel).await;
    let doc = build_cyclonedx(&pkgs);
    serde_json::to_string_pretty(&doc).map_err(|e| Error::Json(e.to_string()))
}

const OSV_BATCH_URL: &str = "https://api.osv.dev/v1/querybatch";
const OSV_VULN_URL: &str = "https://api.osv.dev/v1/vulns";
/// Cap on per-vulnerability detail lookups, to bound network work on very drifty systems.
const MAX_DETAIL_LOOKUPS: usize = 100;

#[derive(Debug, Clone)]
pub struct Finding {
    pub backend: String,
    pub name: String,
    pub version: Option<String>,
    pub id: String,
    pub summary: Option<String>,
    pub fixed: Option<String>,
}

#[derive(Debug, Default)]
pub struct AuditReport {
    pub findings: Vec<Finding>,
    /// Number of packages actually queried (had a known ecosystem + concrete version).
    pub scanned: usize,
    /// Number of packages skipped (no OSV ecosystem mapping or no concrete version).
    pub skipped: usize,
}

/// Build an OSV `querybatch` body from resolved packages. Returns the JSON body and a
/// map from each query's position back to the source package index. Pure — unit tested.
fn build_querybatch(pkgs: &[ResolvedPkg]) -> (Value, Vec<usize>) {
    let mut queries = Vec::new();
    let mut index_map = Vec::new();
    for (i, p) in pkgs.iter().enumerate() {
        let (Some(eco), Some(ver)) = (osv_ecosystem(&p.backend), p.version.as_deref()) else {
            continue;
        };
        if !is_concrete(ver) {
            continue;
        }
        queries.push(json!({
            "version": ver,
            "package": { "name": p.name, "ecosystem": eco },
        }));
        index_map.push(i);
    }
    (json!({ "queries": queries }), index_map)
}

/// Extract the per-query vulnerability IDs from an OSV `querybatch` response, aligned to
/// the request order. Pure — unit tested.
fn parse_querybatch_ids(v: &Value) -> Vec<Vec<String>> {
    v.get("results")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .map(|res| {
                    res.get("vulns")
                        .and_then(|vs| vs.as_array())
                        .map(|vs| {
                            vs.iter()
                                .filter_map(|x| {
                                    x.get("id").and_then(|i| i.as_str()).map(String::from)
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Pull the first "fixed" version out of an OSV vulnerability detail record. Pure.
fn extract_fixed(detail: &Value) -> Option<String> {
    let affected = detail.get("affected")?.as_array()?;
    for a in affected {
        let Some(ranges) = a.get("ranges").and_then(|r| r.as_array()) else {
            continue;
        };
        for r in ranges {
            let Some(events) = r.get("events").and_then(|e| e.as_array()) else {
                continue;
            };
            for e in events {
                if let Some(f) = e.get("fixed").and_then(|x| x.as_str()) {
                    return Some(f.to_string());
                }
            }
        }
    }
    None
}

/// A short human summary from an OSV vulnerability detail record. Pure.
fn summarize(detail: &Value) -> Option<String> {
    if let Some(s) = detail.get("summary").and_then(|s| s.as_str()) {
        return Some(s.to_string());
    }
    detail
        .get("details")
        .and_then(|s| s.as_str())
        .map(|s| s.chars().take(120).collect::<String>())
}

/// Scan every managed package against OSV.dev and report known-vulnerable ones.
pub async fn audit(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    state: &tokio::sync::Mutex<crate::core::StateRegistry>,
) -> Result<AuditReport> {
    let pkgs = resolve_managed(state, registry, config.max_parallel).await;
    let (body, index_map) = build_querybatch(&pkgs);

    let mut report = AuditReport {
        findings: Vec::new(),
        scanned: index_map.len(),
        skipped: pkgs.len() - index_map.len(),
    };
    if index_map.is_empty() {
        return Ok(report);
    }

    // Honour the configured value (F1); the pool raises a literal 0 to 1s, because reqwest
    // reads a zero-second timeout as "fail instantly" rather than "no timeout".
    let client = crate::core::http::api("shall-audit", config.network_timeout_secs)
        .map_err(|e| Error::Http(e.to_string()))?;

    let resp = client.post(OSV_BATCH_URL).json(&body).send().await?;
    if !resp.status().is_success() {
        return Err(Error::Http(format!(
            "OSV querybatch returned HTTP {}",
            resp.status()
        )));
    }
    let val: Value = resp.json().await?;
    let per_query_ids = parse_querybatch_ids(&val);

    // Which advisories need a detail lookup: distinct ids, in the order they were first seen,
    // capped. Deduping *before* fetching is what makes the cap mean "distinct advisories"
    // rather than "advisory mentions".
    let mut wanted: Vec<String> = Vec::new();
    let mut seen: HashMap<String, ()> = HashMap::new();
    for ids in per_query_ids.iter() {
        for id in ids {
            if seen.insert(id.clone(), ()).is_none() && wanted.len() < MAX_DETAIL_LOOKUPS {
                wanted.push(id.clone());
            }
        }
    }

    // Fetched at once, over one pooled connection. This was a nested serial loop of network
    // GETs: a scan across a few hundred managed packages with a handful of advisories each was
    // minutes of pure round-trip latency, and — before the shared client — a full TLS handshake
    // for every one of them.
    let detail_cache: HashMap<String, Value> = {
        use futures::stream::StreamExt;
        futures::stream::iter(wanted)
            .map(|id| {
                let client = client.clone();
                async move {
                    let url = format!("{}/{}", OSV_VULN_URL, id);
                    match client.get(url).send().await {
                        Ok(r) if r.status().is_success() => match r.json::<Value>().await {
                            Ok(d) => Some((id, d)),
                            Err(e) => {
                                debug!("audit: failed to parse detail for {}: {}", id, e);
                                None
                            }
                        },
                        _ => None,
                    }
                }
            })
            .buffer_unordered(config.network_parallel.max(1))
            .filter_map(|r| async move { r })
            .collect()
            .await
    };

    for (qi, ids) in per_query_ids.iter().enumerate() {
        let Some(&pkg_idx) = index_map.get(qi) else {
            continue;
        };
        let pkg = &pkgs[pkg_idx];
        for id in ids {
            let (summary, fixed) = match detail_cache.get(id) {
                Some(d) => (summarize(d), extract_fixed(d)),
                None => (None, None),
            };
            report.findings.push(Finding {
                backend: pkg.backend.clone(),
                name: pkg.name.clone(),
                version: pkg.version.clone(),
                id: id.clone(),
                summary,
                fixed,
            });
        }
    }

    Ok(report)
}

/// Render an audit report to stdout (human or JSON).
pub fn print_audit(report: &AuditReport, out: Output) -> Result<()> {
    if out.is_json() {
        let arr: Vec<Value> = report
            .findings
            .iter()
            .map(|f| {
                json!({
                    "backend": f.backend,
                    "name": f.name,
                    "version": f.version,
                    "id": f.id,
                    "summary": f.summary,
                    "fixed": f.fixed,
                })
            })
            .collect();
        let out = json!({
            "scanned": report.scanned,
            "skipped": report.skipped,
            "vulnerable": arr,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&out).map_err(|e| Error::Json(e.to_string()))?
        );
        return Ok(());
    }

    if report.findings.is_empty() {
        println!(
            "No known vulnerabilities found across {} scanned package(s). ({} skipped — no OSV coverage or no pinned version.)",
            report.scanned, report.skipped
        );
        return Ok(());
    }

    println!(
        "Found {} known-vulnerable package(s) (scanned {}, skipped {}):\n",
        report.findings.len(),
        report.scanned,
        report.skipped
    );
    for f in &report.findings {
        let ver = f.version.as_deref().unwrap_or("?");
        println!("  {}:{} {}", f.backend, f.name, ver);
        if let Some(s) = &f.summary {
            println!("      {} [{}]", s, f.id);
        } else {
            println!("      advisory {}", f.id);
        }
        match &f.fixed {
            Some(fx) => println!("      fix: upgrade to {}", fx),
            None => println!("      fix: see https://osv.dev/{}", f.id),
        }
    }
    println!("\nReview and run `shall upgrade` (or pin fixed versions) to remediate.");
    Ok(())
}

/// Where a package is declared, straight from the resolver (II.7).
///
/// **Asks the model rather than re-reading the files.** `why` answers the one question the
/// model exists to answer — *where is this declared?* — so a second implementation here is a
/// second answer, and it is the one a user reaches for precisely when they already distrust
/// the state. This used to crawl `groups_dir/*.txt` and `modules_dir/*.module.txt` with its
/// own `backend:name` parser: three things II.1 deleted. It could not see a II.1 module at
/// all, never opened `profiles/` or `active`, and said so confidently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// `modules/dev.txt:3` — the file and line, for a human to open.
    pub at: String,
    /// `module:dev`, `profile:Work` — what it belongs to.
    pub scopes: Vec<String>,
    /// The `re:` pattern that matched this name, when no line names it directly. Without it
    /// `why` sends the reader to a file and a line that does not contain the package (II.15).
    pub from_regex: Option<String>,
    /// A dated line that has stopped counting still sits in the file (II.16).
    pub lapsed: bool,
}

impl Declaration {
    /// The sentence `why` prints.
    pub fn describe(&self) -> String {
        let mut out = match &self.from_regex {
            Some(p) => format!("matched by `re:{}` at {}", p, self.at),
            None => format!("at {}", self.at),
        };
        if !self.scopes.is_empty() {
            out.push_str(&format!(" ({})", self.scopes.join(", ")));
        }
        if self.lapsed {
            out.push_str(" — expired, so it no longer counts");
        }
        out
    }
}

/// One `when` condition that had to hold for a package to be declared, with the current
/// value of every variable it tests.
///
/// Two hops in one sentence: the condition is in a file the user can open, the value is not
/// — it was resolved from `vars` — and W11 exists because reading the first without the
/// second explains nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gating {
    pub predicate: String,
    /// `active:4` — where the block is written.
    pub at: String,
    /// Each `$name` in the predicate: its value now, and where that value was set.
    pub variables: Vec<(String, String, String)>,
}

impl Gating {
    pub fn describe(&self) -> String {
        let mut out = format!("`when {}` at {}", self.predicate, self.at);
        for (name, value, origin) in &self.variables {
            out.push_str(&format!(" — ${} is {}, set at {}", name, value, origin));
        }
        out
    }
}

/// What the resolver knows about `backend:name`: where it is declared, and — on a backend
/// that picks between artifacts — which `formats` order it will pick with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Declared {
    pub declarations: Vec<Declaration>,
    /// The variable conditions that admitted it, outermost first. Empty when no `when` that
    /// tests a variable stands between the file and this machine.
    pub gating: Vec<Gating>,
    /// `None` on a backend whose ecosystem publishes one artifact, where there is nothing
    /// to choose and a `formats` line would be noise.
    pub formats: Option<String>,
}

/// The order that applied and which of the three levels set it, as one sentence.
///
/// The absent tag is the built-in default: `to_spec` writes the tag only when a level above
/// it answered.
fn format_choice(backend: &str, options: &crate::config::grammar::Options) -> Option<String> {
    if !crate::backends::capability::selects_artifacts(backend) {
        return None;
    }
    let read = crate::backends::artifact::ArtifactOptions::read(options).ok()?;
    let order = read.resolved_formats(&crate::backends::artifact::default_formats());
    let from = match options.one("__formats_from") {
        Some("line") => "set on the line".to_string(),
        Some(_) => format!("set by `{}` in `priority`", backend),
        None => "the built-in default for this machine".to_string(),
    };
    Some(format!("{} — {}", order, from))
}

/// Read the resolver's `__gated_by` tag back into sentences, filling in each variable's
/// value from this run's resolution.
///
/// A tag whose shape does not parse is shown as written rather than dropped: a `why` that
/// silently loses the reason a package is here is the failure it exists to prevent.
fn gating_of(
    options: &crate::config::grammar::Options,
    vars: &crate::model::vars::Vars,
    origins: &crate::model::vars::VarOrigins,
) -> Vec<Gating> {
    // The tag is a list and arrives as one. It was a `;`-joined string that this split back
    // apart, which meant a gate predicate containing a semicolon became two gates.
    let gates = options.all("__gated_by");
    if gates.is_empty() {
        return Vec::new();
    }
    gates
        .iter()
        .map(|entry| entry.as_str())
        .map(
            |entry| match entry.parse::<crate::config::grammar::Gate>() {
                Ok(gate) => Gating {
                    variables: crate::model::vars::referenced_names(&gate.predicate)
                        .into_iter()
                        .map(|n| {
                            let value = vars
                                .get(&n)
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "undefined".into());
                            let at = origins
                                .get(&n)
                                .map(|o| o.to_string())
                                .unwrap_or_else(|| "an unknown file".into());
                            (n, value, at)
                        })
                        .collect(),
                    predicate: gate.predicate,
                    at: gate.origin.to_string(),
                },
                Err(()) => Gating {
                    predicate: entry.to_string(),
                    at: "an unreadable origin".to_string(),
                    variables: Vec::new(),
                },
            },
        )
        .collect()
}

/// The configuration, resolved once, for every question `why` asks of it.
///
/// **`declarations_of` used to resolve it itself, inside the loop over matches** — so a name two
/// backends carry resolved the whole model twice, and every `vars` file with it. `why` is a read
/// command whose entire job is to answer from the configuration; resolving it per match is the
/// one cost in it that scales with an answer's length rather than with the question's.
struct ResolvedConfig {
    state: crate::model::DesiredState,
    vars: crate::model::vars::Vars,
    var_origins: crate::model::vars::VarOrigins,
}

impl ResolvedConfig {
    /// An error is returned, never swallowed into "declared nowhere": a `why` that cannot read
    /// your files must say so, or it reports a broken config as an absent declaration.
    async fn read(config: &Config, registry: &Arc<BackendRegistry>) -> Result<Self> {
        let resolver =
            crate::app::sync::resolver::StateResolver::new(config, registry.clone(), false).await;
        let state = resolver.resolve_model().await?;
        let (vars, var_origins) = resolver.resolve_vars_with_origins().await?;
        Ok(Self {
            state,
            vars,
            var_origins,
        })
    }
}

/// Ask the resolved configuration where `backend:name` is declared.
fn declarations_of(config: &ResolvedConfig, backend: &str, name: &str) -> Declared {
    let ResolvedConfig {
        state,
        vars,
        var_origins,
    } = config;

    let lapsed_keys: Vec<&str> = state.lapsed.iter().map(|(k, _)| k.as_str()).collect();
    let key = format!("{}:{}", backend, name);

    let mut out = Declared::default();
    for spec in state.packages.values().flatten() {
        if spec.backend != backend || spec.name != name {
            continue;
        }
        if out.formats.is_none() {
            out.formats = format_choice(backend, &spec.options);
        }
        if out.gating.is_empty() {
            out.gating = gating_of(&spec.options, vars, var_origins);
        }
        out.declarations.push(Declaration {
            at: spec
                .options
                .one("__source")
                .map(str::to_string)
                .unwrap_or_else(|| "an unknown file".to_string()),
            scopes: spec.options.all("__scopes").to_vec(),
            from_regex: spec.options.one("__from_regex").map(str::to_string),
            lapsed: lapsed_keys.contains(&key.as_str()),
        });
    }
    out
}

/// Explain why a package is present: how it entered management, and what depends on it.
/// With [`Output::Json`], emit the same provenance as a machine-readable array instead of text.
/// The artifact `why` should explain (D14): the installed file for a download backend, and the
/// rule that chose it, read from `locks/<backend>.toml`.
///
/// `None` for a backend that does not select artifacts, or a package with no lock yet. When a
/// declaration installed several files (`@asset=all`), the first is shown — they were all
/// chosen by the same rule, which is the thing being explained.
fn artifact_selection(config: &Config, backend: &str, name: &str) -> Option<(String, String)> {
    if !crate::backends::capability::selects_artifacts(backend) {
        return None;
    }
    let path = config.layout().lock_file(backend);
    let ledger = crate::core::artifact_lock::ArtifactLedger::load(&path).ok()?;
    let lock = ledger.locked(name).first()?;
    let reason = lock.selected_by.clone()?;
    Some((lock.asset.clone(), reason))
}

/// The commit that first declared `name`, or `None` when git cannot say (XIII.19).
///
/// **Nothing is written to support this.** The config repo is a git repo, every sync commits,
/// and the introducing commit is already recorded — so `why` asks git at the moment someone
/// wants to know. A store filled in at sync time would be a second copy of git's answer, free
/// to disagree with it, and a repo cloned or rebased would take the copy out of date silently.
///
/// Never an error: a config repo that is not under git, or a package declared only since the
/// last commit, simply has no answer, and `why` is still worth reading without it.
async fn introduced_in_git(
    config: &Config,
    executor: &crate::core::CommandExecutor,
    name: &str,
) -> Option<crate::model::introduced::Introduced> {
    use crate::model::introduced;

    let root = config.config_root();
    if !root.join(".git").exists() {
        return None;
    }
    // Limited to the files that can hold a declaration, so a mention in a README or a commit
    // message is never mistaken for the line that introduced the package.
    let args = introduced::argv(name, &["modules", "profiles", "active"]);
    let mut argv: Vec<String> = vec!["-C".into(), root.to_string_lossy().into_owned()];
    argv.extend(args);
    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();

    let out = executor.run_output("git", &refs, false).await.ok()?;
    introduced::introduced_in(&out)
}

/// How a package got into the registry — a different question from where it is declared.
///
/// The arms must match what the writers actually store: `declare.rs` writes `hook:<manager>`
/// and never a bare `hook`, so a reader matching the bare word answers every hooked package
/// with the fallback and nothing says so.
fn provenance(source: &str) -> String {
    match source {
        "imperative" => "installed by `shall install`".to_string(),
        "adopt" => "adopted from this machine".to_string(),
        s => match s.strip_prefix("hook:") {
            Some(manager) => format!(
                "installed behind Shall's back with {}, and caught by the hook",
                manager
            ),
            None => format!("recorded by {}", s),
        },
    }
}

pub async fn why(
    config: &Config,
    registry: &Arc<BackendRegistry>,
    state: &tokio::sync::Mutex<crate::core::StateRegistry>,
    executor: &crate::core::CommandExecutor,
    query: &str,
    out: Output,
) -> Result<()> {
    // Snapshot the state we need, then release the lock before doing async backend queries.
    #[allow(clippy::type_complexity)]
    let (matches, all_managed): (
        Vec<(String, String, Option<String>, String, Option<u64>)>,
        Vec<(String, String)>,
    ) = {
        let state = state.lock().await;
        let matches = state
            .managed()
            .filter(|p| p.name == query || format!("{}:{}", p.backend, p.name) == query)
            .map(|p| {
                (
                    p.backend.clone(),
                    p.name.clone(),
                    p.version.clone(),
                    p.source.clone(),
                    p.expires_at,
                )
            })
            .collect();
        let all = state
            .managed()
            .map(|p| (p.backend.clone(), p.name.clone()))
            .collect();
        (matches, all)
    };

    if matches.is_empty() {
        if out.is_json() {
            println!("{}", serde_json::json!({ "query": query, "matches": [] }));
        } else {
            println!("'{}' is not under Shall management.", query);
        }
        return Ok(());
    }

    let mut json_matches: Vec<serde_json::Value> = Vec::new();
    // Once, before the loop. Two backends carrying one name is an ordinary answer and used to
    // cost two full resolutions of every file you own.
    let resolved = ResolvedConfig::read(config, registry).await?;

    for (backend, name, version, source, expires) in matches {
        // Where your files declare it, from the resolver — the same answer `sync` acts on.
        let found = declarations_of(&resolved, &backend, &name);
        let formats = found.formats.clone();
        let declarations: Vec<String> = found.declarations.iter().map(|d| d.describe()).collect();
        let gating: Vec<String> = found.gating.iter().map(|g| g.describe()).collect();

        let prov = provenance(&source);

        // Declared nowhere and still managed IS the answer, not a gap: it is drift, and the
        // next sync removes it. Saying "declared nowhere" without saying what that means is
        // how a true sentence still misleads.
        let declarations: Vec<String> = if declarations.is_empty() {
            vec!["in no active file — the next `sync` will remove it".to_string()]
        } else {
            declarations
        };

        let lease = expires.map(|exp| {
            match chrono::DateTime::<chrono::Utc>::from_timestamp(exp as i64, 0) {
                Some(dt) => dt.to_rfc2822(),
                None => format!("{}", exp),
            }
        });

        // Reverse dependencies: which other managed packages in the same backend list this
        // one as a native dependency.
        // One subprocess per managed package in the same backend, so they run at once rather
        // than end to end. Ordered, so `why` prints the same list every time.
        let mut dependents = Vec::new();
        if let Some(mp) = registry
            .get(&backend)
            .and_then(|b| b.as_metadata_provider().cloned())
        {
            use futures::stream::StreamExt;
            dependents = futures::stream::iter(
                all_managed
                    .iter()
                    .filter(|(qb, qn)| qb == &backend && qn != &name)
                    .map(|(_, qn)| qn.clone()),
            )
            .map(|qn| {
                let mp = mp.clone();
                let name = name.clone();
                async move {
                    match mp.get_dependencies(&qn).await {
                        Ok(deps) if deps.iter().any(|d| d == &name) => Some(qn),
                        _ => None,
                    }
                }
            })
            .buffered(config.max_parallel.max(1))
            .filter_map(|r| async move { r })
            .collect()
            .await;
        }

        // XIII.19: when this declaration first appeared, asked of git rather than of a store
        // Shall writes at sync time. The config repo is a git repo and every sync commits, so
        // the fact already exists — and a copy of it could only ever disagree.
        let introduced = introduced_in_git(config, executor, &name).await;

        // D14: for an artifact backend, which rule chose the installed file — read from the
        // lock, so `why` answers "why this `.tar.gz` and not the `.deb`" without a network
        // re-selection. `(asset, reason)`.
        let selected = artifact_selection(config, &backend, &name);

        if out.is_json() {
            json_matches.push(serde_json::json!({
                "backend": backend,
                "name": name,
                "version": version,
                "why": prov,
                "declared_in": declarations,
                "gated_by": gating,
                "formats": formats,
                "selected": selected.as_ref().map(|(asset, reason)| serde_json::json!({
                    "asset": asset,
                    "by": reason,
                })),
                "lease_expires": lease,
                "required_by": dependents,
                "introduced": introduced.as_ref().map(|i| serde_json::json!({
                    "commit": i.commit,
                    "date": i.date,
                    "subject": i.subject,
                })),
            }));
        } else {
            let ver = version.map(|v| format!(" @ {}", v)).unwrap_or_default();
            println!("{}:{}{}", backend, name, ver);
            println!("  why:         {}", prov);
            if let Some(i) = &introduced {
                println!("  added:       {}", i.summary());
            }
            for (i, d) in declarations.iter().enumerate() {
                let label = if i == 0 { "declared:" } else { "" };
                println!("  {:<12} {}", label, d);
            }
            for (i, g) in gating.iter().enumerate() {
                let label = if i == 0 { "because:" } else { "" };
                println!("  {:<12} {}", label, g);
            }
            if let Some(f) = &formats {
                println!("  formats:     {}", f);
            }
            if let Some((asset, reason)) = &selected {
                println!("  selected:    {} — chosen by {}", asset, reason);
            }
            if let Some(l) = &lease {
                println!("  lease:       temporary — expires {}", l);
            }
            if dependents.is_empty() {
                println!("  required by: nothing else you manage (safe to remove)");
            } else {
                println!("  required by: {}", dependents.join(", "));
            }
        }
    }

    if out.is_json() {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "query": query, "matches": json_matches
            }))?
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every string a writer in the tree stores in `ManagedPackage::source`, against the
    /// sentence `why` gives back. One entry per writer, so a new writer with no arm here is
    /// visible as a bare "recorded by" rather than as nothing at all.
    #[test]
    fn why_names_each_writer_by_the_string_that_writer_stores() {
        // `app/shell/mod.rs`, `app/leases.rs`, `app/snapshot_restore.rs`.
        assert_eq!(provenance("imperative"), "installed by `shall install`");
        // `app/adopt.rs`.
        assert_eq!(provenance("adopt"), "adopted from this machine");
        // `verbs/declare.rs` — `hook:<manager>`, one arm per manager it can be. Matching a
        // bare `hook` made this unreachable for every one of them.
        for manager in ["choco", "scoop", "apt", "brew"] {
            let said = provenance(&format!("hook:{manager}"));
            assert!(
                said.contains("caught by the hook") && said.contains(manager),
                "hook:{manager} answered {said:?}"
            );
        }
        // `verbs/declare.rs` local-file arm, `app/diagnostics.rs`, `verbs/plan.rs`,
        // `app/sync/mod.rs` — no dedicated sentence, and the fallback names them.
        for other in ["local-file", "diagnostics", "plan", "sync"] {
            assert_eq!(provenance(other), format!("recorded by {other}"));
        }
        // A declared package: `model/resolve.rs` stamps `file:line`, which is the answer
        // `why` exists to give.
        assert_eq!(
            provenance("modules/dev.txt:14"),
            "recorded by modules/dev.txt:14"
        );
    }

    fn p(backend: &str, name: &str, version: Option<&str>) -> ResolvedPkg {
        ResolvedPkg {
            backend: backend.into(),
            name: name.into(),
            version: version.map(String::from),
        }
    }

    fn opts(pairs: &[(&str, &str)]) -> crate::config::grammar::Options {
        let mut o = crate::config::grammar::Options::default();
        for (k, v) in pairs {
            for part in v.split(';') {
                o.insert(*k, part);
            }
        }
        o
    }

    #[test]
    fn a_backend_that_installs_one_artifact_gets_no_formats_line() {
        assert_eq!(format_choice("apt", &opts(&[])), None);
        assert_eq!(format_choice("cargo", &opts(&[("formats", "deb")])), None);
    }

    #[test]
    fn the_line_says_so_when_the_declaration_carried_formats() {
        let c = format_choice(
            "github",
            &opts(&[("formats", "deb;tarball"), ("__formats_from", "line")]),
        )
        .unwrap();
        assert_eq!(c, "deb, tarball — set on the line");
    }

    #[test]
    fn the_priority_file_says_so_when_the_backend_body_won() {
        let c = format_choice(
            "github",
            &opts(&[
                ("formats", "appimage;binary"),
                ("__formats_from", "priority (github)"),
            ]),
        )
        .unwrap();
        assert_eq!(c, "appimage, binary — set by `github` in `priority`");
    }

    #[test]
    fn an_absent_tag_is_the_built_in_default_and_prints_this_machines_order() {
        let c = format_choice("github", &opts(&[])).unwrap();
        let expected = crate::backends::artifact::default_formats();
        assert_eq!(
            c,
            format!("{} — the built-in default for this machine", expected)
        );
    }

    #[test]
    fn purl_and_ecosystem_maps() {
        assert_eq!(purl_type("cargo"), Some("cargo"));
        assert_eq!(purl_type("pnpm"), Some("npm"));
        assert_eq!(purl_type("pipx"), Some("pypi"));
        assert_eq!(purl_type("winget"), None);
        assert_eq!(osv_ecosystem("pip"), Some("PyPI"));
        assert_eq!(osv_ecosystem("cargo"), Some("crates.io"));
        assert_eq!(osv_ecosystem("snap"), None);
        assert_eq!(
            purl("cargo", "ripgrep", Some("13.0.0")).as_deref(),
            Some("pkg:cargo/ripgrep@13.0.0")
        );
    }

    #[test]
    fn cyclonedx_has_expected_shape() {
        let pkgs = vec![
            p("cargo", "ripgrep", Some("13.0.0")),
            p("winget", "Foo", None),
        ];
        let doc = build_cyclonedx(&pkgs);
        assert_eq!(doc["bomFormat"], "CycloneDX");
        assert_eq!(doc["specVersion"], "1.5");
        let comps = doc["components"].as_array().unwrap();
        assert_eq!(comps.len(), 2);
        assert_eq!(comps[0]["name"], "ripgrep");
        assert_eq!(comps[0]["version"], "13.0.0");
        assert_eq!(comps[0]["purl"], "pkg:cargo/ripgrep@13.0.0");
        // winget has no purl mapping and no version → those keys absent.
        assert!(comps[1].get("purl").is_none());
        assert!(comps[1].get("version").is_none());
        assert_eq!(comps[1]["properties"][0]["value"], "winget");
    }

    #[test]
    fn querybatch_skips_uncovered_and_floating() {
        let pkgs = vec![
            p("cargo", "ripgrep", Some("13.0.0")), // covered + concrete -> included
            p("cargo", "exa", Some("latest")),     // floating -> skipped
            p("winget", "Foo", Some("1.0")),       // no ecosystem -> skipped
            p("pip", "requests", None),            // no version -> skipped
        ];
        let (body, map) = build_querybatch(&pkgs);
        assert_eq!(map, vec![0]);
        let queries = body["queries"].as_array().unwrap();
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0]["package"]["ecosystem"], "crates.io");
        assert_eq!(queries[0]["version"], "13.0.0");
    }

    #[test]
    fn parses_querybatch_ids_aligned() {
        let resp = json!({
            "results": [
                { "vulns": [ {"id": "RUSTSEC-1"}, {"id": "GHSA-2"} ] },
                {},
                { "vulns": [] }
            ]
        });
        let ids = parse_querybatch_ids(&resp);
        assert_eq!(
            ids,
            vec![
                vec!["RUSTSEC-1".to_string(), "GHSA-2".to_string()],
                vec![],
                vec![]
            ]
        );
    }

    #[test]
    fn a_declaration_reads_as_a_place_you_can_open() {
        // `why` answers "where is this declared?", so the answer is a file and a line — not
        // a label like "manifest: local.txt" that names a file the model no longer reads.
        let d = Declaration {
            at: "modules/dev.txt:3".into(),
            scopes: vec!["module:dev".into(), "profile:Work".into()],
            lapsed: false,
            from_regex: None,
        };
        assert_eq!(
            d.describe(),
            "at modules/dev.txt:3 (module:dev, profile:Work)"
        );
    }

    #[test]
    fn a_lapsed_declaration_says_it_stopped_counting() {
        // II.16: an expired line lingers in your file. `why` must not report it as the
        // reason a package is present when it has stopped being that reason.
        let d = Declaration {
            at: "modules/imperative.txt:2".into(),
            scopes: vec!["module:imperative".into()],
            lapsed: true,
            from_regex: None,
        };
        assert!(d.describe().contains("expired, so it no longer counts"));
    }

    #[test]
    fn a_declaration_with_no_scope_still_names_its_file() {
        // A line in a profile belongs to no module, and an imperative spec to neither.
        let d = Declaration {
            at: "profiles/Work:5".into(),
            scopes: vec![],
            lapsed: false,
            from_regex: None,
        };
        assert_eq!(d.describe(), "at profiles/Work:5");
    }

    #[test]
    fn a_package_a_pattern_matched_names_the_pattern() {
        // II.15: no line says `fonts-cantarell`. Sending the reader to `modules/dev.txt:3`
        // without saying why would send them to a line that does not mention the package.
        let d = Declaration {
            at: "modules/dev.txt:3".into(),
            scopes: vec!["module:dev".into()],
            lapsed: false,
            from_regex: Some("^fonts-".into()),
        };
        assert_eq!(
            d.describe(),
            "matched by `re:^fonts-` at modules/dev.txt:3 (module:dev)"
        );
    }

    #[test]
    fn extracts_fixed_version_and_summary() {
        let detail = json!({
            "summary": "Memory corruption in foo",
            "affected": [
                { "ranges": [ { "type": "SEMVER", "events": [ {"introduced": "0"}, {"fixed": "1.2.3"} ] } ] }
            ]
        });
        assert_eq!(extract_fixed(&detail).as_deref(), Some("1.2.3"));
        assert_eq!(
            summarize(&detail).as_deref(),
            Some("Memory corruption in foo")
        );
    }
}
