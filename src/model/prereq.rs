//! The setup a manager needs before it can install anything (Q10, Q11, Q13).
//!
//! Three managers in the `tools` image failed every install, and none of them was a LiNix
//! defect: `mix` had no Hex, `asdf` had no plugin for the tool it was asked for, `opam` had no
//! switch. Each fails with the manager's own message, which is accurate and which the user is
//! left to act on by hand — while LiNix, which knows the command, watches.
//!
//! **Ask, then do** (owner ruling, 2026-07-29), the same shape [`bootstrap`](super::bootstrap)
//! uses for a manager that is missing entirely: print what is absent and the exact command,
//! confirm it, run it. `--yes` is the flag that forces it, because a run that already answered
//! "yes, apply the plan" has answered this too, and inventing a second yes-flag would be one
//! more thing to pass. A non-interactive run without it says what it would have asked and
//! leaves the machine alone.
//!
//! **Not folded into `[[bootstrap]]`**, which answers a different question — "this manager is
//! not installed" — with a different probe, and whose file schema is ruled under 7c/U10. Two
//! row types with one behaviour is the price of not changing a ruled contract; the *asking*
//! lives in one place ([`crate::app::Prereqs`]) rather than in each.

use serde::Deserialize;

/// One thing one manager needs before it works.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PrereqDef {
    /// The backend this belongs to — the name a line is written with (`mix`).
    pub manager: String,
    /// Restrict to one OS (`std::env::consts::OS`). Absent means any.
    #[serde(default)]
    pub os: Option<String>,
    /// What is absent, in the words a user would use. Printed above the command, because
    /// "`mix` needs Hex" is a reason to say yes and `mix local.hex --force` on its own is not.
    pub missing: String,
    /// The command that answers "is it already there".
    pub probe: Vec<String>,
    /// When set, the probe's *output* is the answer and its exit code is not: one line of it,
    /// trimmed, must equal this. `asdf plugin list` exits 0 and prints `No plugins installed`,
    /// so an exit code would report every missing plugin as present. Line-exact rather than a
    /// substring: `jq` must not be answered by a plugin called `jqx`.
    #[serde(default)]
    pub probe_output: Option<String>,
    /// The argv that provides it. A list, never a string — a string has to be split by a shell
    /// nobody declared (II.12b).
    pub run: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PrereqFile {
    #[serde(default)]
    pub prereq: Vec<PrereqDef>,
}

impl PrereqDef {
    /// Whether this row applies to the machine LiNix is running on.
    pub fn applies_here(&self, os: &str) -> bool {
        match &self.os {
            Some(want) => want.eq_ignore_ascii_case(os),
            None => true,
        }
    }

    /// A row LiNix will act on, or why it will not. Nothing here is guessed at: a row with no
    /// probe would have to assume the prerequisite is missing and act unasked-for on every
    /// sync, and a row with no command describes a problem and no answer.
    pub fn is_usable(&self) -> Option<&'static str> {
        if self.manager.trim().is_empty() {
            return Some("it names no manager");
        }
        if self.probe.iter().all(|a| a.trim().is_empty()) {
            return Some("its `probe` is empty, so nothing could tell whether it is needed");
        }
        if self.run.iter().all(|a| a.trim().is_empty()) {
            return Some("its `run` command is empty");
        }
        None
    }

    /// Whether this row is about one declared package rather than the manager as a whole.
    ///
    /// Derived from the argv rather than declared beside it: asdf's plugin *is* the package
    /// name, and a `per_package = true` that disagreed with a `run` containing no `{name}`
    /// would be a row that installs the same thing for every line.
    pub fn is_per_package(&self) -> bool {
        let mentions = |v: &[String]| v.iter().any(|a| a.contains("{name}"));
        mentions(&self.run)
            || mentions(&self.probe)
            || self
                .probe_output
                .as_deref()
                .is_some_and(|o| o.contains("{name}"))
    }

    fn fill(args: &[String], name: &str) -> Vec<String> {
        args.iter().map(|a| a.replace("{name}", name)).collect()
    }

    /// The probe for one declared package (`name` is ignored by a row that does not mention it).
    pub fn probe_command(&self, name: &str) -> Vec<String> {
        Self::fill(&self.probe, name)
    }

    /// The command that provides it.
    pub fn run_command(&self, name: &str) -> Vec<String> {
        Self::fill(&self.run, name)
    }

    /// What the probe's output must contain, when the output is the answer.
    pub fn expected_output(&self, name: &str) -> Option<String> {
        self.probe_output
            .as_ref()
            .map(|o| o.replace("{name}", name))
    }

    /// The sentence naming what is absent.
    pub fn missing_line(&self, name: &str) -> String {
        self.missing.replace("{name}", name)
    }

    /// The command as a reader would type it, for the confirmation. Printed in full and never
    /// abbreviated: the whole point of asking is that the reader sees what will run.
    pub fn command_line(&self, name: &str) -> String {
        self.run_command(name).join(" ")
    }

    /// Is this what the probe's output says?
    ///
    /// A line of the output, trimmed, equal to `want` — never a substring of the whole, which
    /// would let one plugin's name answer for another's.
    pub fn output_satisfies(out: &str, want: &str) -> bool {
        out.lines().any(|l| l.trim() == want)
    }
}

/// The rows that apply to `manager` on this machine, in the order they should be offered.
///
/// A row naming this OS is considered before a catch-all, and the caller's own rows come
/// before the built-ins, so a user who disagrees with a shipped row replaces it by writing one
/// rather than by editing LiNix.
pub fn for_manager<'a>(rows: &'a [PrereqDef], manager: &str, os: &str) -> Vec<&'a PrereqDef> {
    rows.iter()
        .filter(|r| r.is_usable().is_none() && r.manager == manager && r.applies_here(os))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(manager: &str, run: &[&str]) -> PrereqDef {
        PrereqDef {
            manager: manager.into(),
            os: None,
            missing: "something".into(),
            probe: vec!["probe".into()],
            probe_output: None,
            run: run.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    /// The built-in rows are the answer to three measured failures, so they are parsed and
    /// checked here rather than trusted to be well-formed at run time.
    #[test]
    fn the_builtin_rows_are_usable() {
        let file: PrereqFile =
            toml::from_str(crate::app::apply::prereq::BUILTIN).expect("the built-in rows parse");
        assert!(
            file.prereq.len() >= 3,
            "mix, asdf and opam each measured needing one"
        );
        for r in &file.prereq {
            assert!(
                r.is_usable().is_none(),
                "{}: {:?}",
                r.manager,
                r.is_usable()
            );
            assert!(
                !r.missing.trim().is_empty(),
                "{} says nothing about what is missing",
                r.manager
            );
        }
    }

    /// asdf's plugin is the declared package; mix's Hex is not. The difference is read off the
    /// command, so a row cannot claim one and be written as the other.
    #[test]
    fn a_row_is_per_package_when_its_command_names_the_package() {
        let asdf = PrereqDef {
            run: vec!["asdf".into(), "plugin".into(), "add".into(), "{name}".into()],
            ..row("asdf", &[])
        };
        assert!(asdf.is_per_package());
        assert_eq!(
            asdf.run_command("jq"),
            vec!["asdf", "plugin", "add", "jq"],
            "the declared name is what reaches the command"
        );
        assert!(!row("mix", &["mix", "local.hex", "--force"]).is_per_package());
    }

    /// `asdf plugin list` exits 0 and prints `No plugins installed`, so the exit code says
    /// "yes" for every plugin that is missing. And a substring match would let `jqx` answer
    /// for `jq`.
    #[test]
    fn an_output_probe_is_line_exact() {
        assert!(PrereqDef::output_satisfies("nodejs\njq\n", "jq"));
        assert!(!PrereqDef::output_satisfies("No plugins installed\n", "jq"));
        assert!(!PrereqDef::output_satisfies("jqx\n", "jq"));
    }

    /// A row that could not tell whether it is needed would act on every sync.
    #[test]
    fn a_row_that_cannot_answer_is_refused() {
        let mut r = row("mix", &["mix", "local.hex"]);
        r.probe = vec!["".into()];
        assert!(r.is_usable().is_some());
        r.probe = vec!["mix".into(), "hex.info".into()];
        r.run = vec![];
        assert!(r.is_usable().is_some());
    }

    /// A user's row comes first, so disagreeing with a shipped one is writing a row rather
    /// than editing LiNix — and a row for another OS is not offered at all.
    #[test]
    fn rows_are_selected_by_manager_and_os() {
        let mut theirs = row("mix", &["mine"]);
        theirs.os = Some("linux".into());
        let mut elsewhere = row("mix", &["theirs"]);
        elsewhere.os = Some("macos".into());
        let rows = vec![theirs, elsewhere, row("mix", &["shipped"])];
        let picked = for_manager(&rows, "mix", "linux");
        assert_eq!(picked.len(), 2, "the macos row is not for this machine");
        assert_eq!(picked[0].run, vec!["mine".to_string()]);
        assert!(for_manager(&rows, "opam", "linux").is_empty());
    }
}
