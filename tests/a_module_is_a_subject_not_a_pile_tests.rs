//! **Two gates on the shape of the tree, because neither is visible to any tool this repo runs.**
//!
//! `cargo` has no opinion about a 4,000-line file or a struct with twelve public fields, and
//! clippy has none either — so both grew until a review counted them. What the count found:
//! four files over 3,000 lines, `verbs/history.rs` holding seven verbs of which none was
//! history, and `App` with 12 public fields, 49 methods and 137 call sites taking `&App`, so
//! that no handler declared what it needed and none could be tested against a narrow fake.
//!
//! Neither number is interesting on its own. What makes them worth a gate is what they cost:
//!
//! - **A file organised by size has no answer to "what else is like this?"** `registry.rs` held
//!   apt beside winget beside pip, in the order somebody wrote them, so the question *which
//!   managers need root and can take the OS with them* had no location — you scrolled.
//! - **A god object hides its dependencies, and a hidden dependency cannot be gated.** While
//!   `registry` was reachable from anywhere holding an `&App`, any code could fan out to every
//!   backend on the machine and walk past the file that says which ones Shall may use. That is
//!   not an aesthetic complaint: it is why `priority` was decorative outside resolution.
//!
//! Both lists below are exemption tables, and they are audited the way this repo audits every
//! other one — a row carries the reason it is a row, and the gate fails when the world and the
//! table disagree in *either* direction, so a row that has been earned back gets deleted.

use std::path::{Path, PathBuf};

/// Nothing under `src/` may exceed this without saying why.
///
/// Not a law of nature — it is roughly where a file stops being readable in one sitting, and
/// every file over it that has been split turned out to hold two subjects.
const LINES: usize = 3_000;

/// The files still over the line, with what each is waiting for.
///
/// **A row here is a promise, not a permission.** Both were measured at 2026-08-12, both are a
/// single genuine subject that is merely large, and both have a shape for the split already —
/// which is exactly why neither was done in a hurry alongside a security fix.
const TOO_BIG_FOR_NOW: &[(&str, usize, &str)] = &[
    (
        "src/config/grammar/statement.rs",
        3_900,
        "One subject — what a line of a manifest means — and three layers of it: the parser, the \
         per-kind validators, and the option-key tables. The split is parse/validate/keys, and it \
         wants doing on a day when the grammar is not also being changed: `B2` moved the option \
         lexer and `B5` moved the name rule in the same week.",
    ),
    (
        "src/backends/generic.rs",
        3_700,
        "`ManagerConfig` and the eight capability impls that read it. The split is one module per \
         capability (installable / queryable / searchable / upgradable), which is a bigger change \
         than it looks: the impls share `GenericBackendCore` and the argv-building helpers, and \
         getting the visibility wrong turns a private helper into public API.",
    ),
];

fn rust_files(root: &Path, out: &mut Vec<PathBuf>) {
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

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn relative(p: &Path) -> String {
    p.strip_prefix(env!("CARGO_MANIFEST_DIR"))
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

#[test]
fn no_file_grows_past_readable_without_a_written_reason() {
    let mut files = Vec::new();
    rust_files(&repo("src"), &mut files);
    assert!(files.len() > 50, "the scan is not walking the tree");

    let mut over: Vec<(String, usize)> = Vec::new();
    for path in &files {
        let Ok(body) = std::fs::read_to_string(path) else {
            continue;
        };
        let n = body.lines().count();
        if n > LINES {
            over.push((relative(path), n));
        }
    }
    over.sort();

    for (file, n) in &over {
        let Some((_, ceiling, _)) = TOO_BIG_FOR_NOW.iter().find(|(f, ..)| f == file) else {
            panic!(
                "{file} is {n} lines and is not on the exemption list.\n\
                 A file this size is two subjects wearing one name — `registry.rs` held apt \
                 beside winget beside pip, so \"what else is like apt\" had no answer but \
                 scrolling. Split it, or add it to TOO_BIG_FOR_NOW with the shape the split \
                 will take."
            );
        };
        assert!(
            n <= ceiling,
            "{file} is {n} lines against a recorded ceiling of {ceiling}. The exemption was for \
             a file that was already large; it is not a licence to keep adding."
        );
    }

    // The other direction, which is what stops an exemption table becoming a graveyard: a file
    // that has been split, or renamed, or deleted, must leave.
    for (file, _, _) in TOO_BIG_FOR_NOW {
        assert!(
            over.iter().any(|(f, _)| f == file),
            "{file} is on the too-big list and is not too big any more (or is gone). Delete the \
             row — an exemption nobody can retire is an exemption nobody re-reads."
        );
    }
}

/// **`App`'s public surface, capped where it stands.**
///
/// Twelve public fields is what let every caller reach past whatever narrow thing it needed, and
/// the private thirteenth — `backends` — is the one that had to be unreachable for the priority
/// gate to mean anything. This does not un-write the god object; it stops it growing while the
/// pieces are taken off it one at a time, and it makes the next field a decision somebody makes
/// on purpose rather than a line somebody adds.
#[test]
fn the_app_god_object_does_not_grow_while_it_is_being_taken_apart() {
    let body = std::fs::read_to_string(repo("src/app/context.rs")).expect("context.rs is readable");
    let struct_body = body
        .split_once("pub struct App {")
        .expect("`App` moved or was renamed")
        .1
        .split_once("\n}")
        .expect("`App` has no end")
        .0;

    let public: Vec<&str> = struct_body
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("pub ") && l.contains(':'))
        .collect();
    let private: Vec<&str> = struct_body
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("pub ") && !l.starts_with("//") && l.contains(':'))
        .collect();

    assert!(
        public.len() <= 12,
        "`App` has {} public fields, up from 12. Every one is a dependency no handler has to \
         declare and no fake has to satisfy — which is why 137 call sites take `&App` and none \
         of them says what it needs. Before adding another, ask whether the thing that wants it \
         could take that field alone.\n  {}",
        public.len(),
        public.join("\n  ")
    );

    // The direction of travel, asserted so it is a direction and not a hope: at least one field
    // is private, and it is the one the backend gate depends on.
    assert!(
        !private.is_empty(),
        "`App` has no private fields. `backends` is private on purpose — while `registry` is \
         reachable, any code can fan out past `priority`, and the gate in \
         `priority_gates_every_fan_out_tests` is only as good as that field being unreachable."
    );
    assert!(
        struct_body.contains("backends:") && !struct_body.contains("pub backends:"),
        "`App::backends` became public. It is the accessor that cannot be bypassed; a public \
         field is a bypass."
    );
}
