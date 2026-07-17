// Cross-backend "insight" commands that are only possible because LiNix sits above
// every ecosystem at once:
//
//   * `audit` — one security scan across every managed package (apt, npm, pip, cargo,
//               gem, …) via the OSV.dev vulnerability database.
//   * `sbom`  — a single CycloneDX software bill of materials spanning all backends.
//   * `why`   — provenance (which manifest/module/imperative action pulled a package in)
//               plus cross-package reverse dependencies.

use crate::app::App;
use crate::core::{Error, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
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
async fn resolve_managed(app: &App) -> Vec<ResolvedPkg> {
    let managed: Vec<(String, String, Option<String>)> = {
        let state = app.state.lock().await;
        state
            .packages
            .iter()
            .map(|p| (p.backend.clone(), p.name.clone(), p.version.clone()))
            .collect()
    };

    let mut out = Vec::with_capacity(managed.len());
    for (backend, name, recorded) in managed {
        let version = match app
            .registry
            .get(&backend)
            .and_then(|b| b.as_queryable().cloned())
        {
            Some(q) => match q.info(&name).await {
                Ok(Some(p)) => p.version.or(recorded),
                _ => recorded,
            },
            None => recorded,
        };
        out.push(ResolvedPkg {
            backend,
            name,
            version,
        });
    }
    out
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

/// Map a LiNix backend to its OSV.dev ecosystem identifier for vulnerability queries.
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
                "properties": [{ "name": "linix:backend", "value": p.backend }],
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
        "metadata": { "tools": [{ "vendor": "LiNix", "name": "linix" }] },
        "components": components,
    })
}

/// Emit a CycloneDX SBOM of every managed package, across all backends, as pretty JSON.
pub async fn sbom(app: &App) -> Result<String> {
    let pkgs = resolve_managed(app).await;
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
pub async fn audit(app: &App) -> Result<AuditReport> {
    let pkgs = resolve_managed(app).await;
    let (body, index_map) = build_querybatch(&pkgs);

    let mut report = AuditReport {
        findings: Vec::new(),
        scanned: index_map.len(),
        skipped: pkgs.len() - index_map.len(),
    };
    if index_map.is_empty() {
        return Ok(report);
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(app.config.network_timeout_secs.max(10)))
        .user_agent("linix-audit")
        .build()
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

    // Cache vuln-id -> detail so we don't refetch shared advisories, and cap total lookups.
    let mut detail_cache: HashMap<String, Value> = HashMap::new();
    let mut lookups = 0usize;

    for (qi, ids) in per_query_ids.iter().enumerate() {
        let Some(&pkg_idx) = index_map.get(qi) else {
            continue;
        };
        let pkg = &pkgs[pkg_idx];
        for id in ids {
            let detail = if let Some(d) = detail_cache.get(id) {
                Some(d.clone())
            } else if lookups < MAX_DETAIL_LOOKUPS {
                lookups += 1;
                match client.get(format!("{}/{}", OSV_VULN_URL, id)).send().await {
                    Ok(r) if r.status().is_success() => match r.json::<Value>().await {
                        Ok(d) => {
                            detail_cache.insert(id.clone(), d.clone());
                            Some(d)
                        }
                        Err(e) => {
                            debug!("audit: failed to parse detail for {}: {}", id, e);
                            None
                        }
                    },
                    _ => None,
                }
            } else {
                None
            };

            let (summary, fixed) = match &detail {
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
pub fn print_audit(report: &AuditReport, as_json: bool) -> Result<()> {
    if as_json {
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
    println!("\nReview and run `linix upgrade` (or pin fixed versions) to remediate.");
    Ok(())
}


/// Turn the structured `__source` provenance tag recorded at install time into a friendly,
/// specific explanation. `;`-joined tags mean a package has more than one origin.
pub fn interpret_source(src: &str) -> String {
    let one = |s: &str| -> String {
        let s = s.trim();
        if let Some(m) = s.strip_prefix("module:") {
            format!("pulled in by module `{}` (@module:{})", m, m)
        } else if let Some(m) = s.strip_prefix("profile:") {
            format!("required by profile `{}`", m)
        } else if s == "imperative" {
            "installed imperatively via `linix install`".to_string()
        } else if s.is_empty() {
            "origin unknown (installed before provenance tracking)".to_string()
        } else {
            let base = s.rsplit(['/', '\\']).next().unwrap_or(s);
            format!("declared in manifest `{}`", base)
        }
    };
    src.split(';')
        .filter(|s| !s.trim().is_empty())
        .map(one)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Pure: does a raw manifest line declare the given package? Matches the bare name or a
/// `backend:name` prefix, ignoring `@options` and leading exclusion markers. Unit tested.
pub fn line_declares(line: &str, backend: &str, name: &str) -> bool {
    let l = line.trim();
    if l.is_empty() || l.starts_with('#') || l.starts_with('-') {
        return false;
    }
    // Structural directives never *declare* a leaf package by name.
    if l.starts_with("@module:") || l.starts_with("when ") || l == "end" {
        return false;
    }
    let head = l.split('@').next().unwrap_or(l).trim();
    match head.split_once(':') {
        Some((b, n)) => b == backend && n == name,
        None => head == name,
    }
}

/// Scan every manifest (.txt) and module (.module.txt) under the config dirs for lines that
/// declare this package, returning human labels like "module: dev" or "manifest: local.txt".
async fn scan_declarations(app: &App, backend: &str, name: &str) -> Vec<String> {
    let mut hits = Vec::new();

    if let Ok(mut entries) = tokio::fs::read_dir(&app.config.groups_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let fname = entry.file_name().to_string_lossy().into_owned();
            if !fname.ends_with(".txt") {
                continue;
            }
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                if content.lines().any(|l| line_declares(l, backend, name)) {
                    hits.push(format!("manifest: {}", fname));
                }
            }
        }
    }

    // Modules directory.
    let modules_dir = app.config.modules_dir.clone();
    if let Ok(mut entries) = tokio::fs::read_dir(&modules_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let fname = entry.file_name().to_string_lossy().into_owned();
            let Some(mod_name) = fname.strip_suffix(".module.txt") else {
                continue;
            };
            if let Ok(content) = tokio::fs::read_to_string(entry.path()).await {
                if content.lines().any(|l| line_declares(l, backend, name)) {
                    hits.push(format!("module: {}", mod_name));
                }
            }
        }
    }

    hits.sort();
    hits.dedup();
    hits
}

/// Explain why a package is present: how it entered management, and what depends on it.
/// With `as_json`, emit the same provenance as a machine-readable array instead of text.
pub async fn why(app: &App, query: &str, as_json: bool) -> Result<()> {
    // Snapshot the state we need, then release the lock before doing async backend queries.
    #[allow(clippy::type_complexity)]
    let (matches, all_managed): (
        Vec<(String, String, Option<String>, Option<String>, Option<u64>)>,
        Vec<(String, String)>,
    ) = {
        let state = app.state.lock().await;
        let matches = state
            .packages
            .iter()
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
            .packages
            .iter()
            .map(|p| (p.backend.clone(), p.name.clone()))
            .collect();
        (matches, all)
    };

    if matches.is_empty() {
        if as_json {
            println!("{}", serde_json::json!({ "query": query, "matches": [] }));
        } else {
            println!("'{}' is not under LiNix management.", query);
        }
        return Ok(());
    }

    let mut json_matches: Vec<serde_json::Value> = Vec::new();

    for (backend, name, version, source, expires) in matches {
        // Provenance from the recorded source tag, interpreted into a specific sentence.
        let prov = interpret_source(source.as_deref().unwrap_or(""));

        // Live scan of all manifests/modules/groups — surfaces every place the package is
        // declared, including profiles/modules that the single recorded tag doesn't capture.
        let declarations = scan_declarations(app, &backend, &name).await;

        let lease = expires.map(|exp| {
            match chrono::DateTime::<chrono::Utc>::from_timestamp(exp as i64, 0) {
                Some(dt) => dt.to_rfc2822(),
                None => format!("{}", exp),
            }
        });

        // Reverse dependencies: which other managed packages in the same backend list this
        // one as a native dependency.
        let mut dependents = Vec::new();
        if let Some(b) = app.registry.get(&backend) {
            if let Some(mp) = b.as_metadata_provider() {
                for (qb, qn) in &all_managed {
                    if qb != &backend || qn == &name {
                        continue;
                    }
                    if let Ok(deps) = mp.get_dependencies(qn).await {
                        if deps.iter().any(|d| d == &name) {
                            dependents.push(qn.clone());
                        }
                    }
                }
            }
        }

        if as_json {
            json_matches.push(serde_json::json!({
                "backend": backend,
                "name": name,
                "version": version,
                "why": prov,
                "declared_in": declarations,
                "lease_expires": lease,
                "required_by": dependents,
            }));
        } else {
            let ver = version.map(|v| format!(" @ {}", v)).unwrap_or_default();
            println!("{}:{}{}", backend, name, ver);
            println!("  why:         {}", prov);
            if !declarations.is_empty() {
                println!("  declared in: {}", declarations.join(", "));
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

    if as_json {
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

    fn p(backend: &str, name: &str, version: Option<&str>) -> ResolvedPkg {
        ResolvedPkg {
            backend: backend.into(),
            name: name.into(),
            version: version.map(String::from),
        }
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
    fn interpret_source_maps_structured_tags() {
        assert_eq!(
            interpret_source("module:dev"),
            "pulled in by module `dev` (@module:dev)"
        );
        assert_eq!(
            interpret_source("imperative"),
            "installed imperatively via `linix install`"
        );
        assert_eq!(
            interpret_source("/home/u/.config/linix/groups/local.txt"),
            "declared in manifest `local.txt`"
        );
        // combined
        assert_eq!(
            interpret_source("module:dev;imperative"),
            "pulled in by module `dev` (@module:dev); installed imperatively via `linix install`"
        );
    }

    #[test]
    fn line_declares_matches_name_and_backend() {
        assert!(line_declares("apt:htop", "apt", "htop"));
        assert!(line_declares("htop", "apt", "htop"));
        assert!(line_declares("apt:htop@version=1.0", "apt", "htop"));
        // wrong backend
        assert!(!line_declares("brew:htop", "apt", "htop"));
        // exclusions, comments, directives never declare
        assert!(!line_declares("-apt:htop", "apt", "htop"));
        assert!(!line_declares("# apt:htop", "apt", "htop"));
        assert!(!line_declares("@module:htop", "apt", "htop"));
        assert!(!line_declares("when os == linux", "apt", "htop"));
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
