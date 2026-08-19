//! **A `Reader` subcommand leaves the config root byte-identical.**
//!
//! `LockScope::Reader` is documented, in `cli/args.rs`, as *"Reads the machine, the config or a
//! remote, and **writes neither**."* One of them wrote.
//!
//! Regex expansion happens during model resolution, which every command performs, and
//! `RegexLock::save` had **no gate on it at all** — while its sibling 330 lines later, the bare
//! lock, carries three: *"Written only when it changed: … And only by a run that acts: a preview
//! that froze the backend it guessed at made the real install afterwards use that guess."* So a
//! `shall check` over a manifest with a `^fonts-` line wrote `locks/regex.toml` for real, and
//! did it **under no lock whatsoever**, because a `Reader` never takes the data lock. Two of
//! those racing a `sync` are two processes rewriting one TOML file whole — last-one-wins, an
//! expansion silently gone. That is exactly the hazard `core::datalock` exists to prevent, in
//! the commands the lock deliberately exempts.
//!
//! **Why this is a sibling of `dry_run_every_verb` and not a case in it.**
//! `a_preview_leaves_the_config_byte_identical` is almost this assertion — it snapshots, runs
//! `--dry-run <verb>`, snapshots again, and requires no change. And it is *structurally unable*
//! to cover a `Reader`, because of the thing that makes it a good test: it guards against
//! vacuity by running the same verb **without** `--dry-run` on a fresh fixture and requiring
//! that control to change something.
//!
//! > *"… THE CONTROL DID NOTHING. Without a run that changes the config, the dry-run assertion
//! > below cannot fail and proves nothing. Fix the fixture."*
//!
//! That control admits only commands that mutate. Its fifteen cases are every one a `Writer`,
//! and `check`, `list`, `plan`, `diff`, `why` and `info` cannot be added without failing it —
//! by definition they change nothing. So the tree had a well-built gate for *"a preview must not
//! write"* and none at all for *"a reader must not write"*, and R3 is the second with `--dry-run`
//! nowhere in the picture.
//!
//! **This file therefore brings its own non-vacuity control**, and it has to be a different one:
//! a planted write the harness must see. `the_snapshot_can_see_a_write_at_all` runs a `Writer`
//! over the same fixture and requires the diff to name what changed. Without that, a snapshot
//! comparison that walked the wrong directory would report "nothing changed" for every reader
//! and pass with nothing behind it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Every `Reader`, as `cli/args.rs` classifies them, with argv that actually resolves the model.
///
/// **Driven with arguments, not bare.** A bare `shall why` prints usage and exits without
/// resolving anything, and a reader that never reached the resolver cannot write the file this
/// gate is about — which would make the pass meaningless. Where a subcommand needs a subject,
/// it is given one that exists in the fixture.
const READERS: &[&[&str]] = &[
    &["check"],
    &["list"],
    &["plan"],
    &["diff"],
    &["why", "ripgrep"],
    &["info", "ripgrep"],
    &["export"],
    &["vars"],
    &["protected"],
    &["policy"],
    &["adapters"],
    &["path"],
    &["sbom"],
];

/// A manifest whose regex line is what made R3 reachable.
///
/// `re:` expansion is the write. A fixture without one exercises the reader and not the bug, so
/// the line is the fixture's whole point — and `the_fixture_manifest_still_carries_a_regex_line`
/// below refuses to let it quietly stop being one.
const MANIFEST: &str = "\
# The line this gate exists for: expanding it is what wrote `locks/regex.toml`.
apt:re:^fonts-
ripgrep
";

/// Files a `Reader` may create, because they are its **answer** rather than Shall's state.
///
/// `cli/args.rs` already draws this line and says why: *"`plan --save` writes a plan FILE, not
/// state, so it is not a counter-example."* The distinction is not a loophole — it is the whole
/// reason `plan` is a `Reader` at all. A plan document is output, like a `--json` payload that
/// happens to land on disk; nothing later reads it as the truth about this machine, and a second
/// `plan` overwriting it loses nothing but a report.
///
/// The snapshot still walks the working directory, because `dry_run_every_verb` learned the hard
/// way that a config-only walk excuses exactly the writes that matter. What is excused here is
/// one named file with one sentence, not a directory.
const A_DOCUMENT_NOT_STATE: &[(&str, &str)] = &[(
    "shall-plan.json",
    "the plan `shall plan` was asked to produce. It is this command's answer, written where the \
     user is standing rather than into the config root or the data directory, and nothing reads \
     it back as the state of the machine — `apply` runs it only when a person names it.",
)];

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_shall")
}

fn run(dir: &Path, args: &[&str]) -> (String, i32) {
    let out = Command::new(bin())
        .args(args)
        .current_dir(dir)
        .env("SHALL_CONFIG_DIR", dir.join("config"))
        .env("SHALL_DATA_DIR", dir.join("data"))
        .stdin(std::process::Stdio::null())
        .output()
        .expect("the binary should run");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

/// Every file under the fixture, by relative path, with its bytes.
///
/// `data/` as well as `config/`, for the reason `dry_run_every_verb` gives about its own
/// snapshot: `registry.json` is the managed set, and a reader that records a package there has
/// armed the next `sync` to remove it.
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, out);
            } else if let Ok(bytes) = std::fs::read(&p) {
                let rel = p
                    .strip_prefix(base)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, bytes);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn diff(before: &BTreeMap<String, Vec<u8>>, after: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    let mut changed = Vec::new();
    for (path, bytes) in after {
        match before.get(path) {
            None => changed.push(format!("created {path}")),
            Some(old) if old != bytes => changed.push(format!("rewrote {path}")),
            Some(_) => {}
        }
    }
    for path in before.keys() {
        if !after.contains_key(path) {
            changed.push(format!("deleted {path}"));
        }
    }
    changed.sort();
    changed
}

fn fixture(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (out, code) = run(&dir, &["init"]);
    assert_eq!(code, 0, "the fixture's own `init` failed:\n{out}");

    // Overwrite whatever `init` wrote with the manifest this gate needs.
    let manifest = dir.join("config").join("starter");
    if manifest.exists() {
        std::fs::write(&manifest, MANIFEST).unwrap();
    } else {
        // `init`'s starter file has moved before. Find the one the config points at rather than
        // pinning a path that goes stale silently — a fixture whose manifest is not read is a
        // fixture that proves nothing.
        let modules = dir.join("config").join("modules");
        std::fs::create_dir_all(&modules).unwrap();
        std::fs::write(modules.join("starter"), MANIFEST).unwrap();
    }
    dir
}

#[test]
fn no_reader_subcommand_writes_anything() {
    let mut offenders: Vec<String> = Vec::new();

    for argv in READERS {
        let dir = fixture(&format!("reader_{}", argv.join("_")));
        let before = snapshot(&dir);
        let (out, _code) = run(&dir, argv);
        let changed: Vec<String> = diff(&before, &snapshot(&dir))
            .into_iter()
            .filter(|line| {
                !A_DOCUMENT_NOT_STATE
                    .iter()
                    .any(|(name, _)| line.ends_with(*name))
            })
            .collect();
        // The exit code is deliberately not asserted. A reader that finds drift exits 2 and one
        // that cannot reach a manager exits 1, and neither is this gate's business — what is
        // being asked is whether it *wrote*, which is a different question from whether it
        // liked what it read.
        if !changed.is_empty() {
            offenders.push(format!(
                "shall {} changed:\n      {}\n    it said:\n      {}",
                argv.join(" "),
                changed.join("\n      "),
                out.lines().take(4).collect::<Vec<_>>().join("\n      ")
            ));
        }
    }

    assert!(
        offenders.is_empty(),
        "`cli/args.rs` classifies these as `LockScope::Reader` — \"reads the machine, the config \
         or a remote, and writes neither\" — and they wrote:\n\n    {}\n\n\
         A `Reader` never takes the data lock, so a write from one is an unsynchronised \
         whole-file rewrite racing every other Shall on the machine. Gate the write on \
         `may_record_locks && !dry_run`, the way `resolver.rs` gates the bare lock, or move the \
         subcommand out of `Reader`.",
        offenders.join("\n\n    ")
    );
}

/// **The control: a write the snapshot can see.**
///
/// The assertion above is a claim that nothing changed, and a claim that nothing changed is what
/// a broken harness also produces — a snapshot of the wrong directory, a fixture the binary
/// never read, a `run` whose environment sent the config somewhere else. So one `Writer` is run
/// over the same fixture, built the same way, and the diff has to name what it did.
///
/// `dry_run_every_verb` makes the same argument about its own control and states the cost of
/// not having one: *"without a run that changes the config, the assertion below cannot fail and
/// proves nothing."*
#[test]
fn the_snapshot_can_see_a_write_at_all() {
    let dir = fixture("reader_control");
    let before = snapshot(&dir);
    let (out, code) = run(&dir, &["module", "create", "planted"]);
    assert_eq!(code, 0, "the control's own command failed:\n{out}");
    let changed = diff(&before, &snapshot(&dir));
    assert!(
        !changed.is_empty(),
        "THE CONTROL DID NOTHING. `shall module create planted` changed no file the snapshot \
         can see, so `no_reader_subcommand_writes_anything` cannot fail and proves nothing. The \
         fixture, the environment or the walk is wrong — fix that before trusting the green \
         above it.\n{out}"
    );
}

/// **And the fixture still contains the line that makes the gate about R3.**
///
/// Without a `re:` line there is nothing to expand, so no reader would write `locks/regex.toml`
/// however broken the gate was, and the pass above would be about nothing. Pinned here so the
/// manifest cannot be simplified into vacuity by somebody who does not know what it is for.
/// **The exemption is audited like every other exemption table in this suite.**
///
/// One name, one sentence, and the sentence has to be long enough to be an argument rather than
/// an assertion — `an_exemption_table_is_audited_the_same_way` makes the same demand of the
/// others. And the excused file has to still be one a reader actually produces: a permission
/// granted to nothing still reads as one guarding something.
#[test]
fn the_only_excused_write_is_a_document_and_it_still_happens() {
    for (name, reason) in A_DOCUMENT_NOT_STATE {
        assert!(
            reason.len() >= 120,
            "{name}'s exemption is {} characters. Say why a file written by a command that \
             \"writes neither\" is not state — the answer has to survive somebody reading it \
             next year.",
            reason.len()
        );
    }

    let dir = fixture("reader_document");
    let before = snapshot(&dir);
    let (out, _) = run(&dir, &["plan"]);
    let changed = diff(&before, &snapshot(&dir));
    assert!(
        changed.iter().any(|l| l.ends_with("shall-plan.json")),
        "`shall plan` no longer writes `shall-plan.json`, so A_DOCUMENT_NOT_STATE excuses a \
         write that does not happen — and an exemption for nothing is how the next real one \
         gets waved through. Delete the entry.\n{out}"
    );
}

#[test]
fn the_fixture_manifest_still_carries_a_regex_line() {
    assert!(
        MANIFEST.contains(":re:"),
        "the fixture manifest no longer has a regex line, so regex expansion never runs and \
         `no_reader_subcommand_writes_anything` is asserting nothing about the write it exists \
         for"
    );

    // And the gate the fix put in is still in the tree. If `may_record_locks` stops guarding
    // the regex lock, this file is the thing that should say so.
    let resolver = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app/sync/resolver.rs"),
    )
    .expect("the resolver is where both locks are written");
    let gated = resolver
        .matches("lock_changed && self.may_record_locks && !self.config.dry_run")
        .count();
    assert_eq!(
        gated, 2,
        "the regex lock and the bare lock should each be written behind \
         `lock_changed && may_record_locks && !dry_run`, and {gated} site(s) are. The regex \
         lock had none of the three, which is the whole of R3."
    );
}
