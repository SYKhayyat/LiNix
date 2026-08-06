//! **Nothing LiNix installs may come from asking a manager what a package depends on.**
//!
//! The rule this gates is II.19's, and the bug it gates is the one the repo kept fixing one
//! backend at a time. The planner asked each backend for a declared package's dependencies and
//! added every returned name as an install node of its own. That node then:
//!
//! - was written into `registry.json` as a package LiNix manages — so `apt:nginx` on one line
//!   took ownership of nginx's dependencies, and a managed package nothing declares is drift,
//!   which the next `sync` removes;
//! - wired a graph edge, and an edge splits a manager's wave into two command lines, so the
//!   one case where LiNix knew two declared packages were related was the one case it refused
//!   to put on a single `apt install`;
//! - cost a subprocess per declared package, and another per discovered dependency, before any
//!   install started.
//!
//! It was found and fixed **per backend**, more than once. Every `ManagerConfig` in
//! `registry.rs` sets `depends_args: None` — 17 literals, zero `Some`, including the shared
//! `base_config` the rest build from; zypper's carries a comment saying a re-derived closure
//! "adds nodes the planner then tries to install by name"; apt's carried a test asserting apt
//! returns an empty set, whose comment said it "guards against the expansion being silently
//! re-enabled". Seven hand-written backends — brew, dnf, flatpak, pacman, snap, vscode, xbps —
//! answered the query for real the whole time, and no gate covered them, because each gate was
//! drawn around the backend that was under review when it was written.
//!
//! So this one is drawn around the property instead: **no file that plans reads a
//! `MetadataProvider`.** That holds for the seven, for every data row, and for the backend
//! nobody has written yet.
//!
//! `MetadataProvider` itself is alive and wanted — `linix info <name>` prints dependencies
//! and `linix why` searches them for reverse dependencies. Reporting them is the feature.
//! Planning from them is the bug.

use std::path::{Path, PathBuf};

/// The files allowed to ask, and what they do with the answer. **An allowlist, not a list of
/// planning paths**: the seven backends survived five fixes because each fix named the places
/// that were wrong at the time, and the eighth caller would have been in none of those lists.
/// Adding a call anywhere in `src/` fails this until it is written down here.
const REPORTING: &[(&str, &str)] = &[
    (
        "src/app/insight.rs",
        "`linix why` — reverse dependencies among packages already managed. Reads them; \
         installs nothing.",
    ),
    (
        "src/verbs/packages.rs",
        "`linix info <name>` — prints a `Dependencies:` line. Reads them; installs nothing.",
    ),
];

fn asks_for_dependencies(line: &str) -> bool {
    let code = match line.split_once("//") {
        Some((before, _)) => before,
        None => line,
    };
    // A backend *answering* is not a backend being asked, and neither is the accessor that
    // hands the answerer out. 23 modules implement `get_dependencies`; the question this asks
    // is who calls one.
    if code.contains("fn get_dependencies") || code.contains("fn as_metadata_provider") {
        return false;
    }
    code.contains("get_dependencies(") || code.contains("as_metadata_provider(")
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            sources(&p, out);
        } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
            out.push(p);
        }
    }
}

/// Every `(file, line number, line)` in `src/` that asks a backend for dependencies.
///
/// The backends' own `impl MetadataProvider` blocks are not call sites and do not appear: an
/// answer nobody asks for changes nothing. Scanning stops at `#[cfg(test)]`, because the fake
/// backend the planner's own tests use to prove it does *not* ask has to be allowed to answer,
/// or the instrument fails on itself.
fn dependency_queries() -> Vec<(String, usize, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    sources(&root.join("src"), &mut files);
    files.sort();

    let mut found = Vec::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        let rel = f
            .strip_prefix(root)
            .unwrap_or(&f)
            .to_string_lossy()
            .replace('\\', "/");
        for (i, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("#[cfg(test)]") {
                break;
            }
            if asks_for_dependencies(line) {
                found.push((rel.clone(), i + 1, line.trim().to_string()));
            }
        }
    }
    found
}

#[test]
fn nothing_that_plans_asks_a_manager_what_a_package_depends_on() {
    let allowed: Vec<&str> = REPORTING.iter().map(|(f, _)| *f).collect();
    let problems: Vec<String> = dependency_queries()
        .into_iter()
        .filter(|(file, _, _)| !allowed.contains(&file.as_str()))
        .map(|(file, line, text)| format!("  {}:{}  {}", file, line, text))
        .collect();

    assert!(
        problems.is_empty(),
        "a file outside the reporting allowlist is asking a backend for dependencies:\n{}\n\n\
         Whatever a manager installs alongside what you asked for is that manager's business, \
         and it does it at install time whether or not LiNix asks first. A name that comes back \
         from here becomes an install node, which becomes a row in `registry.json` that nothing \
         declares — and a managed package nothing declares is what `sync` removes (II.7). \
         Report dependencies; never plan from them. If this call really only reports, add the \
         file to REPORTING in this test with the sentence saying so.",
        problems.join("\n")
    );
}

/// The oracle: the scan must be able to see a query that is really there, or the emptiness
/// above proves nothing. The allowlisted files legitimately ask, so they are the control — and
/// if one ever stops asking, this fails and says so rather than going quiet over a stale entry.
#[test]
fn the_scan_can_actually_see_a_dependency_query() {
    let found = dependency_queries();
    for (file, why) in REPORTING {
        assert!(
            found.iter().any(|(f, _, _)| f == file),
            "{} is allowlisted for {} and no longer asks for dependencies — either the feature \
             went away, in which case drop the entry, or the scan is broken and the assertion \
             above is passing for the wrong reason",
            file,
            why
        );
    }

    assert!(asks_for_dependencies(
        "                    let deps = provider.get_dependencies(&spec.name).await.ok()?;"
    ));
    assert!(asks_for_dependencies(
        "            let provider = registry.get(&spec.backend)?.as_metadata_provider()?.clone();"
    ));

    // …and the controls, or the two above would pass for a scan that returns true always.
    assert!(!asks_for_dependencies(
        "    let deps = spec.requires.clone();"
    ));
    assert!(!asks_for_dependencies(
        "        // get_dependencies( is named here on purpose"
    ));
    assert!(
        !asks_for_dependencies(
            "    async fn get_dependencies(&self, name: &str) -> Result<Vec<String>> {"
        ),
        "a backend declaring the method is not a backend being asked; 23 modules declare it"
    );
}
