//! Eight ways to extend Shall, and one place that knows there are eight.
//!
//! Owner request, 2026-08-09: a plugin system, *"almost like how in Lisp you can add your own"*.
//! The surfaces were already there and already worked — `[[backend]]` teaches a package manager,
//! `[[snapshot]]` teaches a rollback provider, and every one goes through II.12's ledger. What
//! was missing is that **nothing in the program knew the list**. Eight paths on `Layout`, eight
//! readers, eight `warn!("ignoring adapters/x.toml: …")` lines, and no way to ask what this
//! machine has extended.
//!
//! A list that lives only in a person's head grows a ninth entry silently, and the ninth is
//! invisible to `shall adapters`, absent from the docs, and reported by a bespoke warning. These
//! gates make each of those three a test failure instead.

use shall::app::adapters::{self, Standing, SURFACES};

fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `src/**/*.rs`.
fn sources() -> Vec<std::path::PathBuf> {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(&root().join("src"), &mut out);
    out.sort();
    out
}

/// **Every `adapters/<name>.toml` the source names is a declared surface.**
///
/// A ninth reader is one `layout.adapter_file("thing")` away, and nothing about adding it would
/// fail: `shall adapters` would list eight, the docs would list eight, and the ninth would work
/// perfectly and be invisible. This is the gate that makes the table the definition rather than
/// a copy of one.
#[test]
fn every_adapter_surface_is_in_the_table() {
    let declared: Vec<&str> = SURFACES.iter().map(|s| s.name).collect();
    let mut named: Vec<(String, String)> = Vec::new();

    for path in sources() {
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        for line in body.lines() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            // `adapter_file("x")` is the one way to name a surface's file, which is why the
            // firewall's inline `adapters_dir().join("firewall.toml")` had to go first.
            let mut rest = line;
            while let Some(i) = rest.find("adapter_file(\"") {
                rest = &rest[i + 14..];
                let Some(end) = rest.find('"') else { break };
                named.push((
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into(),
                    rest[..end].to_string(),
                ));
                rest = &rest[end..];
            }
        }
    }

    assert!(
        named.len() >= SURFACES.len(),
        "found only {} `adapter_file(\"…\")` call(s); the scan has stopped matching the source",
        named.len()
    );

    let strangers: Vec<String> = named
        .iter()
        .filter(|(_, s)| !declared.contains(&s.as_str()) && s != "{surface}")
        .map(|(f, s)| format!("{f} opens adapters/{s}.toml"))
        .collect();
    assert!(
        strangers.is_empty(),
        "these read an extension surface that `app::adapters::SURFACES` does not declare, so \
         `shall adapters` cannot see it and the docs do not name it:\n  {}",
        strangers.join("\n  ")
    );
}

/// **Every surface's file is reachable through the one accessor.**
///
/// `firewall:` read `adapters_dir().join("firewall.toml")` inline, which is why it was the one
/// surface with no `Layout` method — and a table derived from the methods would have had seven
/// rows and no way to notice the eighth.
#[test]
fn no_adapter_file_is_opened_by_hand() {
    let mut offenders: Vec<String> = Vec::new();
    let mut scanned = 0usize;
    for path in sources() {
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        scanned += 1;
        // `layout.rs` is where the join lives; that is what "one accessor" means.
        let is_the_accessor = path.ends_with("layout.rs");
        for (i, line) in body.lines().enumerate() {
            if is_the_accessor || line.trim_start().starts_with("//") {
                continue;
            }
            if line.contains("adapters_dir().join(") {
                offenders.push(format!(
                    "{}:{}  {}",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    i + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(scanned > 50, "the scan read only {scanned} files");
    assert!(
        offenders.is_empty(),
        "these join an adapters path by hand instead of `Layout::adapter_file`:\n  {}",
        offenders.join("\n  ")
    );
}

/// **No reader writes its own "ignoring adapters/…" sentence.**
///
/// Eight of them did, and a serde message alone (*"missing field `name` at line 4 column 1"*)
/// says which line and nothing else: not which of eight files, not what a row of that kind looks
/// like, not that the rest of the file is inert, not that a command exists which says all three.
#[test]
fn a_surface_that_cannot_be_used_is_reported_in_one_voice() {
    let mut offenders: Vec<String> = Vec::new();
    let mut callers = 0usize;
    for path in sources() {
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        for (i, line) in body.lines().enumerate() {
            if line.contains("cannot_use(") {
                callers += 1;
            }
            if line.trim_start().starts_with("//") {
                continue;
            }
            let hand_rolled = (line.contains("ignoring adapters/")
                || line.contains("Ignoring malformed adapters/"))
                && !line.contains("cannot_use");
            if hand_rolled {
                offenders.push(format!(
                    "{}:{}  {}",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    i + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these write their own refusal for an unusable adapter file:\n  {}\n\n\
         `app::adapters::cannot_use` names the file, what a row teaches, how a row opens, and \
         `shall adapters`.",
        offenders.join("\n  ")
    );
    // One caller per surface, plus the definition and its test.
    assert!(
        callers >= SURFACES.len(),
        "only {callers} references to `cannot_use`; the readers have drifted back to their own \
         wording"
    );
}

/// **The readme names all eight**, because a plugin surface nobody can find is not a plugin
/// surface. The docs were the other place the list lived only by hand.
#[test]
fn the_readme_names_every_surface_and_the_verb_that_lists_them() {
    let readme = std::fs::read_to_string(root().join("readme.md")).expect("readme.md");
    let missing: Vec<&str> = SURFACES
        .iter()
        .filter(|s| !readme.contains(&format!("adapters/{}.toml", s.name)))
        .map(|s| s.name)
        .collect();
    assert!(
        missing.is_empty(),
        "the readme does not name these extension surfaces: {missing:?}"
    );
    assert!(
        readme.contains("shall adapters"),
        "the readme never mentions the command that lists the surfaces"
    );
}

/// A machine with no `adapters/` directory reports eight absences and no problems — the case
/// almost every machine is in, and the one where a survey that panicked or reported trouble
/// would be worse than none.
#[test]
fn a_repo_with_no_adapters_directory_is_surveyed_without_complaint() {
    let dir = tempfile::TempDir::new().unwrap();
    let layout = shall::model::layout::Layout::new(dir.path(), dir.path().join("data"));
    let found = adapters::survey(&layout);
    assert_eq!(found.len(), SURFACES.len());
    assert!(found.iter().all(|e| e.standing == Standing::Absent));
    assert!(found.iter().all(|e| !e.standing.is_wrong()));
}

/// **The ruling of 2026-08-10, as an exit code.** A malformed `adapters/*.toml` warns and is
/// skipped; it does not refuse the run. The alternative was on the table and lost on where it
/// would fire — a `sync` on a working machine, where a typo in an optional extension file would
/// stop you installing a package.
///
/// Here rather than only in the container harness: there, the same claim can only be made as
/// "the command exited 0", which is a check a binary that does nothing also passes — the
/// mutation gate says so and is right. The container asserts the words a user reads; this
/// asserts the code a script branches on.
#[test]
fn a_malformed_adapter_does_not_refuse_a_sync() {
    let f = crate::harness::Fixture::new("adapters-malformed-degrades");
    f.write("priority", "cargo\n");
    f.write_module("cargo:ripgrep\n");
    f.write("adapters/backends.toml", "this is not toml at all\n");

    let (out, code) = f.run(&["sync", "--dry-run"]);
    assert_eq!(
        code, 0,
        "a malformed adapter file refused the run; the ruling is that it degrades:\n{out}"
    );

    // And the other half of the ruling: the fact is not lost, it moves to the command whose
    // exit code is free to be loud.
    let (out, code) = f.run(&["check", "adapters"]);
    assert_ne!(
        code, 0,
        "`check adapters` reported nothing wrong about a file nothing can read:\n{out}"
    );
    assert!(
        out.contains("malformed"),
        "the report has to say which way it is unusable:\n{out}"
    );
}
