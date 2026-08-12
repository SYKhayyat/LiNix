//! **`App` wires a run together. It must not also *be* the run.**
//!
//! It was both: twelve public fields, forty-nine methods and 139 call sites taking `&App`, so no
//! handler declared what it needed and none could be exercised without building the whole world
//! — the registry's forty-eight backends, a state file, a WAL, a snapshot-provider probe — to
//! test a function that reads one boolean out of `Config`.
//!
//! Nothing the compiler owns can see that. `&App` type-checks everywhere, an unused field is
//! never unused because something else uses it, and a method that reaches eleven collaborators
//! to use two is indistinguishable from one that needs eleven. So the two rules that keep it
//! dissolved are checked here, textually, or they are not checked at all.
//!
//! **Rule 1 — the root has no logic.** Every method on `App` outside its constructors is a
//! factory: it hands back a narrower type and decides nothing. A method that branches is a
//! method that belongs on the facet it branches about, where a test can reach it with two
//! fields instead of twelve.
//!
//! **Rule 2 — only the dispatch layer names the root.** `src/verbs/` and `main.rs` turn argv
//! into a command and are entitled to the whole context; everything under `src/app/` is a facet
//! of that context and must take what it uses. A facet that takes `&App` has inverted the
//! layering — it is reaching back for the container it is a part of, which is how the first
//! twelve of those 139 call sites happened.

use std::path::PathBuf;

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn rust_files(root: &PathBuf, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The three that build an `App` rather than hand out a view of one. Construction reads a
/// config, picks an executor and joins four futures — it is allowed to decide things, because
/// deciding them is the whole job.
const CONSTRUCTORS: &[&str] = &[
    "new",
    "new_with_executor_and_state_path",
    "reconfigured",
    "machinery",
];

/// `impl App { … }`, split into `(name, body)` per method.
fn app_methods(src: &str) -> Vec<(String, String)> {
    let start = src
        .find("\nimpl App {")
        .expect("src/app/context.rs no longer has an `impl App` block — has it been renamed?");
    let lines: Vec<&str> = src[start + 1..].lines().collect();

    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        // The impl block ends at the first unindented `}`.
        if i > 0 && line == "}" {
            break;
        }
        let trimmed = line.trim_start();
        let is_fn = line.starts_with("    ")
            && !line.starts_with("     ")
            && (trimmed.starts_with("pub fn ")
                || trimmed.starts_with("pub async fn ")
                || trimmed.starts_with("fn ")
                || trimmed.starts_with("async fn "));
        if !is_fn {
            i += 1;
            continue;
        }
        let name = trimmed
            .rsplit_once("fn ")
            .map(|(_, rest)| rest)
            .and_then(|rest| rest.split(['(', '<']).next())
            .unwrap_or("")
            .to_string();
        // Signatures wrap; the body starts at the first line ending in `{`.
        let mut j = i;
        while j < lines.len() && !lines[j].trim_end().ends_with('{') {
            j += 1;
        }
        let mut depth = 0usize;
        let mut body = String::new();
        let mut k = j;
        while k < lines.len() {
            depth += lines[k].matches('{').count();
            depth = depth.saturating_sub(lines[k].matches('}').count());
            if k > j {
                body.push_str(lines[k]);
                body.push('\n');
            }
            if depth == 0 {
                break;
            }
            k += 1;
        }
        out.push((name, body));
        i = k + 1;
    }
    out
}

#[test]
fn every_method_on_the_composition_root_is_a_factory() {
    let src = std::fs::read_to_string(repo("src/app/context.rs")).expect("context.rs");
    let methods = app_methods(&src);
    assert!(
        methods.len() > 10,
        "the parser found only {} methods on `App` — it has stopped reading the file, not \
         the file stopped having methods",
        methods.len()
    );

    let mut deciding = Vec::new();
    for (name, body) in &methods {
        if CONSTRUCTORS.contains(&name.as_str()) {
            continue;
        }
        // A factory hands back a narrower type. Anything that chooses between two answers is
        // doing the facet's work at the root, where every collaborator is in reach.
        for keyword in [" if ", "if ", "match ", "for ", "while ", " else "] {
            if body
                .lines()
                .map(str::trim_start)
                .filter(|l| !l.starts_with("//"))
                .any(|l| l.starts_with(keyword.trim_start()) || l.contains(keyword))
            {
                deciding.push(format!("{name} (branches on `{}`)", keyword.trim()));
                break;
            }
        }
    }
    assert!(
        deciding.is_empty(),
        "`App` is the composition root: it wires a run and hands out narrow views of it.\n\
         These methods decide something instead, which means they belong on the facet they \
         decide about:\n  {}\n\
         Move the body to a type that holds only what it reads — see `app/inventory.rs`, \
         `app/declarations.rs` and the nine facets under `app/apply/`.",
        deciding.join("\n  ")
    );
}

/// Whether this line declares a parameter of type `&App`.
///
/// The whole name, not a prefix of one: `core: &AppImageBackendCore` starts with `&App` and is
/// a backend's own type, which the first draft of this test reported as a layering violation.
fn names_the_app(code: &str) -> bool {
    for prefix in [": &App", ": &crate::app::App"] {
        let mut rest = code;
        while let Some(at) = rest.find(prefix) {
            let after = &rest[at + prefix.len()..];
            if !after.starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == ':') {
                return true;
            }
            rest = &rest[at + prefix.len()..];
        }
    }
    false
}

#[test]
fn only_the_dispatch_layer_takes_the_whole_app() {
    let mut files = Vec::new();
    rust_files(&repo("src"), &mut files);

    let mut offenders = Vec::new();
    for path in files {
        let rel = path
            .strip_prefix(repo(""))
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        // `verbs/` and `main.rs` ARE the dispatch layer: turning one argv into one command is
        // the job that legitimately holds the whole context.
        if rel.contains("/verbs/") || rel.ends_with("src/main.rs") || rel.ends_with("context.rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap_or_default();
        for (n, line) in src.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if names_the_app(code) {
                offenders.push(format!("{rel}:{}", n + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these are facets of `App`, so they must take what they use rather than the container \
         they are part of:\n  {}\n\
         Take `&Config`, `&Arc<BackendRegistry>`, a `&StateResolver<'_>` or one of the facet \
         types — whichever the body actually reads.",
        offenders.join("\n  ")
    );
}
