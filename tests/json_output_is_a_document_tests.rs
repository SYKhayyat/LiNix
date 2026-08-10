//! `--json` puts a document on stdout and nothing else — including on the paths nobody previews.
//!
//! `linix fleet` reads a remote `linix check --json` over SSH, so the whole command rests on
//! that output being parseable by a machine that is not standing there to read around a stray
//! sentence. Both of the verbs below broke that promise, and both broke it on the branch a
//! *healthy* machine takes:
//!
//! - **`sync --dry-run --json` on a converged machine printed no document at all.** The report
//!   was emitted inside the dry-run block, and the "nothing to do" exit returns above it. So the
//!   answer to "is this machine already in sync?" — the question a fleet asks most — was the
//!   words `already up to date`, in English, where JSON was expected.
//! - **`check --json` could be preceded by a plain-text note.** `Adopter::discover`, which the
//!   `unmanaged` section calls, printed `Note: your modules did not resolve …` to stdout. On a
//!   machine whose config is broken — the one least able to spare a working report — the
//!   document became unparseable.
//!
//! Neither is exotic and neither had a test, because a `--json` flag gets exercised on the busy
//! path where there is obviously something to print. **The empty case is the one nobody looks
//! at, and it is the one a converged fleet is made of.**
//!
//! Driven end to end against the real binary in a disposable config and data directory, because
//! the defect is in what reaches a file descriptor and no unit test of a formatter can see that.

use crate::harness::Fixture;

impl Fixture {
    fn module(&self, contents: &str) {
        let m = self.root.join("config").join("modules").join("starter.txt");
        std::fs::create_dir_all(m.parent().unwrap()).unwrap();
        std::fs::write(&m, contents).unwrap();
    }
}

/// Parse, or fail saying what arrived instead — the message is the finding.
fn parsed(what: &str, stdout: &str) -> serde_json::Value {
    serde_json::from_str(stdout).unwrap_or_else(|e| {
        panic!(
            "`{what}` did not put a JSON document on stdout ({e}).\n\
             A machine reads this over SSH and cannot read around a sentence.\n\
             stdout was:\n{}",
            if stdout.trim().is_empty() {
                "<empty>".to_string()
            } else {
                stdout.to_string()
            }
        )
    })
}

#[test]
fn sync_dry_run_json_answers_a_converged_machine_with_a_document() {
    let f = Fixture::new("json-doc-sync-converged");
    // Nothing declared: the plan is empty, which is the branch that returns above the report.
    f.module("");

    let (stdout, stderr, _) = f.run_split(&["--dry-run", "sync", "--json"]);
    let doc = parsed("sync --dry-run --json", &stdout);

    assert!(
        doc.get("install").is_some_and(|v| v.is_array()),
        "the plan document must carry its install list even when the list is empty: {doc}"
    );
    assert_eq!(
        doc["install"].as_array().map(Vec::len),
        Some(0),
        "nothing is declared, so nothing installs"
    );
    assert!(
        !stdout.contains("already up to date"),
        "the English answer is still on stdout beside the document:\n{stdout}"
    );
    // The control: this is not passing because the command silently did nothing at all.
    assert!(
        !stdout.trim().is_empty(),
        "stdout was empty, so `parsed` above proved nothing. stderr:\n{stderr}"
    );
}

/// The busy path too, so the test above is known to be about the *empty* case rather than about
/// `--json` never working.
#[test]
fn sync_dry_run_json_answers_a_drifted_machine_with_a_document() {
    let f = Fixture::new("json-doc-sync-drifted");
    f.module("github:sharkdp/hexyl\n");

    let (stdout, _, _) = f.run_split(&["--dry-run", "sync", "--json"]);
    let doc = parsed("sync --dry-run --json", &stdout);

    assert_eq!(
        doc["install"].as_array().map(Vec::len),
        Some(1),
        "one declared package, nothing installed: one install in the plan. Document was:\n{doc}"
    );
}

#[test]
fn check_json_is_a_document_even_when_the_config_does_not_resolve() {
    let f = Fixture::new("json-doc-check-broken-config");
    // A `use` of a module that does not exist: resolution fails, which is what makes
    // `Adopter::discover` reach for the note that used to go to stdout.
    f.module("use nothing-by-this-name\n");

    let (stdout, _, _) = f.run_split(&["check", "--json"]);
    let doc = parsed("check --json", &stdout);

    assert!(
        doc.is_array(),
        "`check --json` is an array of sections: {doc}"
    );
    assert!(
        !stdout.contains("Note:"),
        "the human note is on stdout, in front of the document:\n{stdout}"
    );
}

/// The counts `fleet` reads. They are the reason `check --json` is machine-readable at all: a
/// consumer that has to regex `"3 to install, 1 to remove"` has made the wording of a sentence
/// into an API, and the next person to improve the sentence breaks the fleet.
#[test]
fn check_json_carries_its_numbers_beside_its_sentences() {
    let f = Fixture::new("json-doc-check-counts");
    f.module("github:sharkdp/hexyl\n");

    let (stdout, _, _) = f.run_split(&["check", "--json"]);
    let doc = parsed("check --json", &stdout);
    let sections = doc.as_array().expect("an array of sections");

    let drift = sections
        .iter()
        .find(|s| s["section"] == "drift")
        .unwrap_or_else(|| panic!("no `drift` section — `fleet` reads that one: {doc}"));

    // Present, and present as a number, on a machine that has drifted.
    for key in ["install", "remove", "skipped"] {
        assert!(
            drift["counts"][key].is_u64(),
            "drift.counts.{key} is missing or is not a number: {drift}"
        );
    }
    assert_eq!(
        drift["counts"]["install"], 1,
        "one declared package, nothing installed: {drift}"
    );

    // And every section carries the object, even the ones with nothing to count — a consumer
    // that must tell "the key is absent" from "the count is nought" writes that branch wrong
    // once, and then reports an unread machine as a clean one.
    for s in sections {
        assert!(
            s["counts"].is_object(),
            "section `{}` has no `counts` object: {s}",
            s["section"]
        );
    }
}

/// **A verb is handed a reader, never the flag.**
///
/// Both defects above were written under a `!json` guard: the negation reads as "not
/// machine-readable", not as "there is a person here", and an early return written in the first
/// dialect happily jumped over the document. `--json` is now converted to
/// [`linix::core::Output`] once, in `main`'s dispatch, and every handler below it takes the
/// decision rather than the flag.
///
/// A source scan and not a behavioural test because the property is *absence* — the twenty-first
/// `json: bool` parameter is the one nobody would write a case for.
#[test]
fn a_verb_is_handed_a_reader_not_a_flag() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders: Vec<String> = Vec::new();
    let mut scanned = 0usize;
    let mut readers = 0usize;

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
    let mut files = Vec::new();
    walk(&src, &mut files);
    files.sort();

    for path in &files {
        let body = std::fs::read_to_string(path).unwrap_or_default();
        scanned += 1;
        for (i, line) in body.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            // Two files may say `json: bool`, and they are the two ends of the flag's life:
            // `args.rs` is where clap parses it, `output.rs` is the single conversion into a
            // reader. Anything between them is a handler that was handed the flag.
            let exempt = path.ends_with("args.rs") || path.ends_with("output.rs");
            if !exempt && (line.contains("json: bool") || line.contains("as_json: bool")) {
                offenders.push(format!(
                    "{}:{}  {}",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    i + 1,
                    line.trim()
                ));
            }
            if line.contains("out: Output") {
                readers += 1;
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these take the `--json` flag instead of the reader it decides:\n  {}\n\n`--json` \
         becomes `core::Output` once, in `main`'s dispatch. A handler asks `out.is_human()` \
         rather than negating a flag it should not have been given.",
        offenders.join("\n  ")
    );
    // Two floors, because either alone passes on a tree this scan has stopped reading.
    assert!(
        scanned > 50,
        "the scan read only {scanned} source files; it is not reading `src/`"
    );
    assert!(
        readers >= 15,
        "only {readers} signatures take an `Output`; the type has stopped being the way a verb \
         learns who is reading"
    );
}
