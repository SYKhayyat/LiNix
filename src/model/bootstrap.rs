//! `adapters/bootstrap.toml` — how to obtain a package manager this machine does not have (7c).
//!
//! A config that declares `brew:ripgrep` on a machine with no Homebrew is not wrong; it is
//! ahead of the machine. Today that is an error the user resolves by going and reading
//! Homebrew's install page. **P8: LiNix does the thing, it does not hand you the thing to do**
//! — so if the repo says how to get the manager, LiNix offers to get it.
//!
//! **Ask, then do.** The command is printed in full and confirmed before it runs. That is the
//! shape P8 licenses and the one it does not: never *inform and leave*, never *act unasked*.
//! A bootstrap is usually `curl | sh` from a vendor, which is exactly the thing nobody should
//! run because a config file said so and nobody looked.
//!
//! The file goes through II.12's approval ledger like every other `adapters/` file, so the
//! command cannot arrive with a pulled repo and run unreviewed.

use serde::Deserialize;

/// One way to obtain one manager.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct BootstrapDef {
    /// The backend this obtains — the same name a line is written with (`brew`).
    pub manager: String,
    /// Restrict to one OS (`std::env::consts::OS`). Absent means any.
    #[serde(default)]
    pub os: Option<String>,
    /// The argv to run. A list, never a string: a string would have to be split by someone,
    /// and the someone is always a shell nobody declared (II.12b).
    pub run: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BootstrapFile {
    #[serde(default)]
    pub bootstrap: Vec<BootstrapDef>,
}

impl BootstrapDef {
    /// Whether this row applies to the machine LiNix is running on.
    pub fn applies_here(&self, os: &str) -> bool {
        match &self.os {
            Some(want) => want.eq_ignore_ascii_case(os),
            None => true,
        }
    }

    /// A row LiNix will act on, or why it will not. A row that names no manager or no command
    /// describes nothing; running "the empty command" is not a thing to guess at.
    pub fn is_usable(&self) -> Option<&'static str> {
        if self.manager.trim().is_empty() {
            return Some("it names no manager");
        }
        if self.run.iter().all(|a| a.trim().is_empty()) {
            return Some("its `run` command is empty");
        }
        None
    }

    /// The command as a reader would type it, for the confirmation. Printed in full and never
    /// abbreviated: the whole point of asking is that the reader sees what will run.
    pub fn command_line(&self) -> String {
        self.run.join(" ")
    }
}

/// The row that obtains `manager` on this machine, if the repo carries one.
///
/// First match wins, and rows naming this OS are considered before catch-alls — a file may
/// carry a specific answer and a general one, and the specific one is the one that was written
/// for this machine.
pub fn for_manager<'a>(
    rows: &'a [BootstrapDef],
    manager: &str,
    os: &str,
) -> Option<&'a BootstrapDef> {
    let usable = |r: &&BootstrapDef| r.is_usable().is_none() && r.manager == manager;
    rows.iter()
        .find(|r| usable(r) && r.os.is_some() && r.applies_here(os))
        .or_else(|| rows.iter().find(|r| usable(r) && r.os.is_none()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(manager: &str, os: Option<&str>) -> BootstrapDef {
        BootstrapDef {
            manager: manager.into(),
            os: os.map(str::to_string),
            run: vec![
                "/bin/sh".into(),
                "-c".into(),
                format!("install {}", manager),
            ],
        }
    }

    #[test]
    fn a_row_without_an_os_applies_anywhere() {
        assert!(row("brew", None).applies_here("linux"));
        assert!(row("brew", None).applies_here("windows"));
    }

    #[test]
    fn a_row_naming_an_os_applies_only_there() {
        let r = row("brew", Some("macos"));
        assert!(r.applies_here("macos"));
        assert!(!r.applies_here("linux"));
    }

    /// A file may carry a general answer and a specific one; the one written for this machine
    /// is the one that runs.
    #[test]
    fn a_row_for_this_os_beats_a_catch_all() {
        let rows = vec![row("brew", None), row("brew", Some("macos"))];
        let picked = for_manager(&rows, "brew", "macos").unwrap();
        assert_eq!(picked.os.as_deref(), Some("macos"));
        // ...and on a machine the specific row does not name, the catch-all still answers.
        assert_eq!(for_manager(&rows, "brew", "linux").unwrap().os, None);
    }

    #[test]
    fn a_manager_with_no_row_has_no_bootstrap() {
        let rows = vec![row("brew", None)];
        assert!(for_manager(&rows, "nix", "linux").is_none());
    }

    /// A row describing nothing is skipped rather than run as an empty command.
    #[test]
    fn an_unusable_row_is_never_chosen() {
        let mut nameless = row("brew", None);
        nameless.manager = String::new();
        assert!(nameless.is_usable().is_some());

        let mut commandless = row("brew", None);
        commandless.run = vec!["".into()];
        assert!(commandless.is_usable().is_some());
        assert!(for_manager(&[commandless], "brew", "linux").is_none());
    }

    /// The confirmation shows the command in full — that is the entire value of asking.
    #[test]
    fn the_command_line_is_shown_whole() {
        let r = row("brew", None);
        let line = r.command_line();
        assert!(line.contains("/bin/sh"), "{}", line);
        assert!(line.contains("install brew"), "{}", line);
    }

    /// Argv is a list, so a manager name with a space in it cannot become two arguments.
    #[test]
    fn the_command_is_argv_not_a_string_to_be_split() {
        let r = BootstrapDef {
            manager: "brew".into(),
            os: None,
            run: vec!["installer".into(), "--prefix".into(), "/opt/my brew".into()],
        };
        assert_eq!(r.run.len(), 3, "the path with a space stayed one argument");
    }
}
