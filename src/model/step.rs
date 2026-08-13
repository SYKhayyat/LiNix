//! The upgrade steps Shall ships with (`H8`) — a name a user writes instead of a script.
//!
//! `H6` gave `exec:` an `@on=` so a declared step could be run by `upgrade`. It left the step
//! itself the user's to write, which is the difference between *possible* and *convenient* — and
//! convenience is the whole of the competing tool's claim. This is the catalogue: `exec:step/
//! rustup` means what `exec:./bin/rustup-up.sh` would have meant, without the file.
//!
//! **A row, never a shipped script, and the two are different in kind.** A `.sh` in a release is
//! code travelling to machines, which is precisely the question `II.12`'s approval gate exists to
//! ask. A row says *how a known tool is upgraded* — a fact about `rustup`, not code about this
//! user — and it is compiled into the binary the way `builtin_backends.toml` and
//! `firewall_adapters.toml` are. Those tables settled the approval question already:
//! *"this file is compiled into the binary, so there is no II.12 question to ask about it."* A
//! script the user writes still needs `shall lock`. That asymmetry is the point rather than an
//! inconsistency: you approve what you wrote, and you already approved the binary by installing
//! it.

use serde::Deserialize;

use crate::core::adapter::{AdapterRow, Detected};

/// The prefix that says a name is catalogued rather than a path on disk.
///
/// **A reserved first segment, not a guess.** `exec:` has always taken a path, and a bare
/// `exec:rustup` would be a file called `rustup` in the config repo *and* a catalogue name, with
/// nothing in the line to say which. Resolution orders that "try one, fall back to the other"
/// are how a typo becomes a different program: this way `step/` means the catalogue, everything
/// else means a file, and neither can shadow the other.
pub const STEP_PREFIX: &str = "step/";

/// The catalogue name a line refers to, or `None` if the line names a script.
pub fn named(script: &str) -> Option<&str> {
    script.strip_prefix(STEP_PREFIX)
}

/// One catalogued step.
#[derive(Debug, Clone, Deserialize)]
pub struct Step {
    pub name: String,
    /// The one OS this step exists on. Absent means every OS.
    #[serde(default)]
    pub os: Option<String>,
    /// The command whose presence on `PATH` means this machine runs this thing.
    pub detect: String,
    /// The command to run, argv-style — no shell, so nothing here is parsed as one.
    pub run: Vec<String>,
    /// Which verb runs it, as `H6`'s `@on=` spells it. The row's default; a line may override.
    pub on: String,
    /// The `@runs=` ceiling this step wants. `always` for anything whose job is to be re-run.
    pub runs: String,
    /// One line a human reads when they are choosing, and when a name is refused.
    pub what: String,
}

impl AdapterRow for Step {
    const WHAT: &'static str = "upgrade step";

    fn name(&self) -> &str {
        &self.name
    }

    fn only_on(&self) -> Option<&str> {
        self.os.as_deref()
    }

    fn why_unusable(&self) -> Option<&'static str> {
        if self.run.is_empty() {
            return Some("it has no `run` command, so there is nothing for it to do");
        }
        if self.detect.trim().is_empty() {
            return Some("it has no `detect`, so nothing could say whether this machine runs it");
        }
        None
    }
}

impl Detected for Step {
    fn detect_command(&self) -> &str {
        &self.detect
    }
}

/// The shipped table, uncompiled. Public so a test can assert against the bytes rather than
/// against a copy of them.
pub const CATALOGUE: &str = include_str!("upgrade_steps.toml");

#[derive(Deserialize)]
struct Catalogue {
    #[serde(default)]
    step: Vec<Step>,
}

/// Every shipped row, in file order, with the unusable ones dropped loudly.
///
/// Panics on a malformed table for the same reason `builtin_rows` does: the file is compiled
/// into this binary, so a parse failure is a build that should not have shipped rather than a
/// machine's problem, and a test in this module reads it.
pub fn rows() -> Vec<Step> {
    let parsed: Catalogue = toml::from_str(CATALOGUE)
        .expect("upgrade_steps.toml is compiled in and parsed by a test in this module");
    crate::core::adapter::usable(parsed.step)
}

/// The rows this OS has, which is what a user can name here.
pub fn rows_here() -> Vec<Step> {
    rows().into_iter().filter(|s| s.applies_here()).collect()
}

/// The step this name refers to, on this machine.
///
/// Looked up among the rows this OS runs, so a Linux-only step is *unknown* on Windows rather
/// than known-and-broken — the refusal then lists what a user can actually write.
pub fn find(name: &str) -> Option<Step> {
    rows_here().into_iter().find(|s| s.name == name)
}

/// Every name this machine offers, for the sentence a refused name prints.
pub fn names_here() -> Vec<String> {
    rows_here().into_iter().map(|s| s.name).collect()
}

/// What a run of this step actually executes — the row's argv, split for the executor.
///
/// Argv rather than a command line, so nothing here is parsed by a shell. A step is data, and
/// data that reaches a shell stops being data.
pub fn launch(step: &Step) -> Option<(String, Vec<String>)> {
    crate::core::adapter::program_and_args(step.run.clone())
}

/// Why this `exec:` name is not a step Shall ships, or `None` if it is one (or is a path).
///
/// **Here rather than in the grammar, and the file-size gate is what said so.** The refusal was
/// twenty-five lines inside `validate_exec`, which pushed `statement.rs` past its recorded
/// ceiling — and the gate's own words are that the exemption "is not a licence to keep adding".
/// It was the wrong home anyway: the grammar's job is the shape of a line, and *which names
/// exist* is a fact about this catalogue. The parser asks; this answers.
///
/// Returns the sentence and its hint, so the caller builds its own error type and this module
/// needs no dependency on the grammar's.
pub fn refusal(name: &str) -> Option<(String, String)> {
    let step = named(name)?;
    if find(step).is_some() {
        return None;
    }
    let known = names_here();
    let hint = match known.is_empty() {
        // A machine that offers none is not a machine with a broken catalogue — every shipped
        // row names an OS or a tool that is not here — so the hint sends them to the thing that
        // does work rather than implying something is wrong.
        true => "this machine offers none — every shipped step names an OS or a tool that is \
                 not here. Write the script yourself and approve it with `shall lock`."
            .to_string(),
        // "ships for this OS", not "available here": a step whose tool is absent is still
        // declarable — it is skipped at run time, which is what makes one config work across a
        // laptop and a server. `check config` lists the narrower set and says so in those words.
        false => format!(
            "the steps Shall ships for this OS are: {}. A script of your own is `exec:` with a \
             path, and needs `shall lock`.",
            known.join(", ")
        ),
    };
    Some((format!("`exec:{}` names no step Shall ships", name), hint))
}

/// What the ledger keys this step by.
///
/// The argv, not a file's bytes, because there is no file — and hashing the argv gives the same
/// property the content hash gives a script: a step whose command changes in a later release is
/// a different step, and `@runs=` counts it separately instead of reading the old row's count.
pub fn fingerprint(step: &Step) -> String {
    crate::core::hook_lock::hash_script(&step.run.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped table parses, and every row can be driven.
    ///
    /// This is what makes `rows()`'s `expect` honest: it panics on a malformed file, and this is
    /// the test that would have caught the malformation before it shipped.
    #[test]
    fn the_shipped_catalogue_parses_and_every_row_is_usable() {
        let parsed: Catalogue = toml::from_str(CATALOGUE).expect("the shipped table parses");
        assert!(!parsed.step.is_empty(), "the catalogue ships no steps");
        for step in &parsed.step {
            assert_eq!(
                step.unusable(),
                None,
                "shipped step `{}` would be dropped at runtime",
                step.name
            );
            assert!(
                !step.what.trim().is_empty(),
                "`{}` says nothing about what it does, and a name a user chooses from has to",
                step.name
            );
            assert!(
                crate::model::exec::Verb::VALUES.contains(&step.on.as_str()),
                "`{}` has `on = {:?}`, which the grammar would refuse on a line",
                step.name,
                step.on
            );
        }
    }

    /// A name is catalogued or it is a path, and nothing is both.
    #[test]
    fn only_the_reserved_prefix_names_a_step() {
        assert_eq!(named("step/rustup"), Some("rustup"));
        assert_eq!(named("./bin/rustup.sh"), None);
        assert_eq!(
            named("rustup"),
            None,
            "a bare name is a file, as it always was"
        );
        assert_eq!(
            named("bin/step/rustup"),
            None,
            "the prefix is the whole first segment"
        );
    }

    /// The OS filter decides what a machine can name, and it is asserted from both sides so the
    /// Windows arm is exercised on Linux and back.
    #[test]
    fn a_step_belongs_to_the_platforms_its_row_names() {
        let rows = rows();
        let fwupd = rows
            .iter()
            .find(|s| s.name == "fwupd")
            .expect("fwupd is shipped");
        assert!(fwupd.applies_to("linux"));
        assert!(!fwupd.applies_to("windows"));
        assert!(!fwupd.applies_to("macos"));

        let rustup = rows
            .iter()
            .find(|s| s.name == "rustup")
            .expect("rustup is shipped");
        for os in ["linux", "windows", "macos"] {
            assert!(rustup.applies_to(os), "rustup should exist on {os}");
        }
    }

    /// The argv reaches the executor as a program and its arguments, never as one string.
    #[test]
    fn a_step_runs_as_argv_and_not_through_a_shell() {
        let rustup = find("rustup").or_else(|| rows().into_iter().find(|s| s.name == "rustup"));
        let rustup = rustup.expect("rustup ships on every platform");
        let (program, args) = launch(&rustup).expect("a usable row splits");
        assert_eq!(program, "rustup");
        assert_eq!(args, ["update"]);
    }

    /// Two steps with the same command would share a ledger row and one would report the
    /// other's run count.
    #[test]
    fn every_shipped_step_has_its_own_fingerprint() {
        let rows = rows();
        let mut seen = std::collections::HashMap::new();
        for step in &rows {
            if let Some(other) = seen.insert(fingerprint(step), step.name.clone()) {
                panic!("`{}` and `{}` hash to one ledger row", step.name, other);
            }
        }
    }
}
