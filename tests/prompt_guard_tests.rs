//! No prompt without a refusal — enumerated from the source, not from a list.
//!
//! A prompt with nothing on the other end is not a safe default. In a CI job, a cron run, a
//! pipe or a container, `dialoguer` reads EOF and returns a bare `IO error: not a terminal` —
//! a message that names no command, no file and no way forward, attached to a run that was
//! about to change the machine. Ten of the eleven prompt sites refuse properly; the eleventh
//! was `snapshot_restore`'s gallery, on a *restore* path, where nobody is watching by
//! definition.
//!
//! The reason this is a source scan rather than eleven behavioural tests: the defect is a
//! missing line, and a list of known prompts is exactly what the previous audit had — it
//! enumerated the prompts someone remembered. A new `.interact()` added tomorrow joins this
//! test automatically, and that is the only property that stops the class from coming back.

use std::path::{Path, PathBuf};

/// Every `src/**/*.rs` file, so a prompt cannot hide in a module nobody listed.
fn sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
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
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    out.sort();
    out
}

/// How far back a guard may sit and still be guarding this prompt. Generous on purpose: the
/// question is whether the refusal exists at all, and a false pass here is cheaper than a
/// test nobody can keep green.
const LOOKBACK: usize = 60;

#[test]
fn every_interactive_prompt_refuses_when_there_is_no_terminal() {
    let mut unguarded: Vec<String> = Vec::new();
    let mut found = 0usize;

    for path in sources() {
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        let lines: Vec<&str> = body.lines().collect();

        for (i, line) in lines.iter().enumerate() {
            // `dialoguer`'s four blocking entry points. A prompt is whatever waits for a human.
            let prompts = line.contains(".interact()")
                || line.contains(".interact_opt()")
                || line.contains(".interact_text()")
                || line.contains(".interact_on(");
            if !prompts {
                continue;
            }
            found += 1;

            let from = i.saturating_sub(LOOKBACK);
            let guarded = lines[from..=i].iter().any(|l| l.contains("is_terminal"));
            if !guarded {
                unguarded.push(format!(
                    "{}:{}  {}",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    i + 1,
                    line.trim()
                ));
            }
        }
    }

    // Without this the test passes on a tree where the scan matched nothing at all — the
    // shape of check this file exists to replace.
    assert!(
        found >= 8,
        "the prompt scan found only {found} prompts; it has stopped matching the code it audits"
    );

    assert!(
        unguarded.is_empty(),
        "these prompts will hang or die with `IO error: not a terminal` instead of refusing \
         with something a user can act on:\n  {}\n\nThe pattern to copy is in \
         src/verbs/cleanup.rs: check `std::io::stdin().is_terminal()` and return \
         `Error::Refused` naming the flag that proceeds without asking.",
        unguarded.join("\n  ")
    );
}
