//! What the write-ahead log covers, and what it deliberately does not.
//!
//! `readme.md`'s safety section says *"a write-ahead log records every mutation before it
//! runs"*. Until this landed, `JournalAction` had two variants — `Install` and `Remove` — and
//! all nine `apply/` modules contained zero references to the journal. Every non-package
//! mutation a sync makes happened outside the log.
//!
//! Most of that asymmetry is right and stays. A `service:`, a `setting:`, a `firewall:` port,
//! a placed `link:` are read-then-write converges from a declaration: killed halfway, the next
//! sync reads the machine, sees the line unmet and finishes the job — a better recovery than
//! replaying a log, because it also corrects drift the log never saw. Journalling those would
//! be durability theatre.
//!
//! One thing a sync does is not that. `exec:` runs code and `@undo=` runs an arbitrary shell
//! command: nothing records how far either got, their authors never promised they were safe to
//! run twice, and there is no declared end state to converge towards. Those two are now logged.
//!
//! The first test proves the *write-ahead* half rather than merely the *logged* half, and it
//! uses the only witness that can tell the difference: **the script reads the journal while it
//! is running.** An entry written after the interpreter returns would leave it nothing to find.

use linix::core::hook_lock::{exec_id, hash_script, HookLedger};
use linix::core::journal::{ActionStatus, JournalAction, JournalEntry};
use linix::core::LockFile;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let f = Self { root };
        let (out, code) = f.run(&["init"]);
        assert_eq!(code, 0, "the fixture's own `init` failed:\n{out}");
        f
    }

    fn cfg(&self) -> PathBuf {
        self.root.join("config")
    }

    fn data(&self) -> PathBuf {
        self.root.join("data")
    }

    fn journal(&self) -> PathBuf {
        self.data().join("journal.jsonl")
    }

    /// Declare one `exec:` line, write the script it names, and approve it — II.12 refuses an
    /// unapproved script, and this test is about what happens when one runs.
    fn declare_exec(&self, body: &str) -> String {
        let name = format!("setup{}", linix::model::script::SCRIPT_SUFFIX);
        std::fs::write(self.cfg().join(&name), body).unwrap();
        let rel = format!("./{}", name);
        std::fs::write(
            self.cfg().join("modules").join("tools.txt"),
            format!("exec:{}\n", rel),
        )
        .unwrap();
        std::fs::write(self.cfg().join("profiles").join("Main"), "use tools\n").unwrap();

        let locks = self.cfg().join("locks");
        let path = HookLedger::path_in(&locks);
        let mut ledger = HookLedger::load(&path).unwrap();
        ledger.approve(&exec_id(&rel), &hash_script(body));
        ledger.save(&path).unwrap();
        rel
    }

    /// Put the on-disk state a kill leaves behind into the log: one entry that started and
    /// never reached an outcome. `record_start` writes exactly this and nothing else runs, so
    /// hand-writing it is the same file a killed process leaves — which the first test proves
    /// by having a live script read it.
    fn leave_interrupted(&self, action: JournalAction) {
        let entry = JournalEntry {
            id: "exec:half-done:deadbeef".to_string(),
            action,
            status: ActionStatus::InProgress,
            started_at_unix: chrono::Utc::now().timestamp(),
            finished_at_unix: None,
            error: None,
        };
        std::fs::create_dir_all(self.data()).unwrap();
        std::fs::write(
            self.journal(),
            format!("{}\n", serde_json::to_string(&entry).unwrap()),
        )
        .unwrap();
    }

    fn run(&self, args: &[&str]) -> (String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_linix"))
            .args(args)
            .current_dir(&self.root)
            .env("LINIX_CONFIG_DIR", self.cfg())
            .env("LINIX_DATA_DIR", self.data())
            .env("HOME", &self.root)
            .env("USERPROFILE", &self.root)
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
}

/// A script that reports its own execution and then appends whatever the journal holds at that
/// moment.
///
/// The `RAN` line is written unconditionally and first, so *"the script never ran"* and *"the
/// script ran and the journal was empty"* are two different failures. Folded together — by
/// copying the journal and treating a missing copy as proof of nothing — the second would have
/// been reported as the first, and the second is the finding.
fn script_that_reads_the_journal(journal: &Path, to: &Path) -> String {
    #[cfg(windows)]
    {
        format!(
            "Set-Content -LiteralPath '{to}' -Value 'RAN'\n\
             Get-Content -LiteralPath '{journal}' -ErrorAction SilentlyContinue | \
             Add-Content -LiteralPath '{to}'\n\
             exit 0\n",
            to = to.display(),
            journal = journal.display()
        )
    }
    #[cfg(not(windows))]
    {
        format!(
            "echo RAN > '{to}'\n\
             cat '{journal}' >> '{to}' 2>/dev/null || true\n\
             exit 0\n",
            to = to.display(),
            journal = journal.display()
        )
    }
}

/// **Write-ahead, not write-behind.** The script under test reads the journal while it is
/// running: if the entry were recorded after the interpreter returned, the copy it takes would
/// not contain it — which is exactly what a kill mid-script sees, and exactly what the machine
/// looked like before this landed.
#[test]
fn the_entry_is_on_disk_before_the_script_starts() {
    let f = Fixture::new("wal-write-ahead");
    let seen = f.root.join("journal-as-the-script-saw-it.jsonl");
    let rel = f.declare_exec(&script_that_reads_the_journal(&f.journal(), &seen));

    let (out, code) = f.run(&["sync", "-y"]);
    assert_eq!(code, 0, "{out}");

    // The instrument, self-tested, in two steps: first that the script ran at all, and only
    // then what it saw. An empty journal and an unexecuted script are different failures.
    let observed = std::fs::read_to_string(&seen).unwrap_or_else(|e| {
        panic!("the script did not run, so this proves nothing about the log ({e}):\n{out}")
    });
    assert!(
        observed.contains("RAN"),
        "the script wrote its report but not the marker it writes first — the instrument is \
         broken, not the log:\n{observed}"
    );

    assert!(
        observed.contains("\"Exec\""),
        "the script ran and the journal held no `Exec` entry at that moment. A record written \
         after the interpreter returns is not a write-ahead record — the case it exists for is \
         the one where `after` never comes. The journal, as the script saw it:\n{observed}"
    );
    assert!(
        observed.contains(&rel),
        "an `Exec` entry was on disk but did not name the script that was running. `heal` \
         reports by name, and a name it cannot print is a report nobody can act on:\n{observed}"
    );
    assert!(
        observed.contains("InProgress"),
        "the entry was on disk already resolved, so it was not describing work in flight:\n\
         {observed}"
    );

    // And it does not stay in flight: the script succeeded, so nothing is left to report.
    let after = std::fs::read_to_string(f.journal()).unwrap_or_default();
    let (heal, code) = f.run(&["heal"]);
    assert_eq!(code, 0, "{heal}");
    assert!(
        !heal.contains("interrupted"),
        "a script that completed was reported as interrupted. An entry left open keeps \
         `needs_recovery` true for ever and re-reports itself in front of every sync — the \
         208-second shape. Journal after the run:\n{after}\n\nheal said:\n{heal}"
    );
}

/// Recovery cannot finish a half-run script, and must not pretend to. What it owes is the
/// account nobody was given: which script, and what the next sync will now do about it.
///
/// **It must not replay it.** A package is finished by installing it again; a script that got
/// half way has no recorded progress, so re-running it re-runs the half that already ran.
#[test]
fn an_interrupted_script_is_reported_by_name_and_never_replayed() {
    let f = Fixture::new("wal-interrupted-script");
    // The script exists and is declared, so a `heal` that decided to replay one would have
    // something to run — the finding is not that it cannot, it is that it must not.
    let marker = f.root.join("it-ran-again");
    f.declare_exec(&script_that_reads_the_journal(&f.journal(), &marker));

    f.leave_interrupted(JournalAction::Exec {
        script: "./deploy.sh".to_string(),
        hash: "abc123def456789".to_string(),
    });

    let (heal, code) = f.run(&["heal"]);
    assert_eq!(code, 0, "{heal}");

    assert!(
        heal.contains("deploy.sh"),
        "recovery said nothing about an interrupted script. Before this, a machine killed \
         mid-`exec:` came back silent and ran the script again from the top on the next \
         sync:\n{heal}"
    );
    assert!(
        heal.contains("run it again from the top"),
        "the report named the script but not what happens next, which is the half a user can \
         act on:\n{heal}"
    );
    assert!(
        !heal.contains("recovered 1"),
        "recovery counted a script it did not repair as an operation it recovered. Nothing on \
         the machine changed:\n{heal}"
    );
    assert!(
        !marker.exists(),
        "recovery re-ran the script. A half-run script has no recorded progress, so replaying \
         it repeats the half that already ran."
    );

    // Reported is resolved: a second `heal` has nothing left to say. An entry that can never
    // be recovered but stays `InProgress` keeps `needs_recovery` true for ever, and that ran a
    // full recovery in front of every sync — 208 seconds of one `watch --once` (Q33).
    let (again, code) = f.run(&["heal"]);
    assert_eq!(code, 0, "{again}");
    assert!(
        !again.contains("deploy.sh"),
        "the same interruption was reported twice, so it will be reported for ever:\n{again}"
    );
}

/// The other half of the ruling: the converges are deliberately NOT logged, and that is a
/// decision rather than an omission.
///
/// A `service:`/`setting:`/`firewall:`/`link:` line describes an end state, so the next sync
/// recomputes it. This pins the shape so that a later change adding a variant per phase — the
/// review's original proposal — has to argue with the reason rather than slip past it.
#[test]
fn a_converge_from_a_declaration_is_not_logged() {
    let replayable = |a: JournalAction| a.is_replayable();

    assert!(
        replayable(JournalAction::Remove {
            name: "jq".into(),
            backend: "apt".into()
        }),
        "a package is finished by re-running it: reaching a state twice is reaching it once"
    );
    assert!(!replayable(JournalAction::Exec {
        script: "./s.sh".into(),
        hash: "h".into()
    }));
    assert!(!replayable(JournalAction::ExecUndo {
        script: "./s.sh".into(),
        command: "rm -rf /tmp/x".into()
    }));

    // And the log has no vocabulary for a converge — there is deliberately no variant to
    // record one with. If this list ever grows a `Service`, `Setting`, `Firewall` or
    // `Schedule`, the reasoning in `JournalAction`'s header is what has to change first.
    let logged = ["Install", "Remove", "Exec", "ExecUndo"];
    let source =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/journal.rs"))
            .expect("the journal's source is readable");
    let body = source
        .split_once("pub enum JournalAction {")
        .expect("JournalAction's shape changed")
        .1
        .split_once("\n}")
        .expect("unterminated enum")
        .0;
    let found: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//") && !l.starts_with('/'))
        .filter_map(|l| l.split(['(', ' ', '{', ',']).next())
        .filter(|w| w.chars().next().is_some_and(char::is_uppercase))
        .collect();

    // **Asserted as an equality, not as a membership test.** A loop that only fires on an
    // unexpected variant passes just as quietly when the parse above finds nothing at all —
    // a reformat, a renamed enum, a doc comment shaped differently — and a gate that cannot
    // fail is this repository's signature defect. Equality fails in both directions: a new
    // variant, and an instrument that stopped reading.
    assert_eq!(
        found, logged,
        "the log's variants are not the four this rule allows. If the parse found nothing or \
         something unrecognisable, fix the instrument. If a variant is genuinely new: a \
         resource converged from a declaration is recomputed by the next sync, so its log \
         entry is durability theatre — read `JournalAction`'s header before adding it, and \
         update this list only if the argument there has actually changed."
    );
}
