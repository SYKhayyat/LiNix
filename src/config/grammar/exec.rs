//! What an `exec:` or a `generate:` line may say.
//!
//! The two verb statements, kept together and kept out of the parser. Every other keyword in
//! this grammar is a noun whose validation is a table of option keys; these two are the pair
//! that carry real rules — which verb a step belongs to, how many times its content may run,
//! how many packages must have moved first — and those rules are what grew `statement.rs` past
//! the size at which one file is one subject. `model::step::refusal` left for the same reason
//! and is called from here.

use super::error::{GrammarError, Origin, Result};
use super::options::Options;
use super::statement::{validate_extra_options, OptionKind};

/// `runs` caps how many times a distinct script content may run — `1` (the default) is
/// run-once-per-content; `always` opts out (see `model::exec`). `undo` is deliberately absent:
/// what a removal means is U3, still open, so no key promises it.
/// `undo` is what removing the line runs (U3). Optional, because a script has no inverse and
/// inventing one would be Shall claiming to undo something it cannot: without it, removing an
/// `exec:` drops the record and nothing else, and `plan` says so in those words.
/// `on` names the verb this step belongs to — `sync` (the default), `upgrade`, or `both`.
///
/// **Per step, never inherited (`H6`).** `upgrade` ran no declared steps at all, so a firmware
/// or `rustup` line correctly written and correctly approved was never run by the verb a user
/// reaches for weekly. Widening `upgrade` to run every `exec:` would have made a verb that has
/// never executed user scripts start executing every script in every existing manifest — and
/// the approval gate answers *what* may run, not *which verb* may run it, so somebody who
/// approved a script for `sync` did not thereby approve it for `upgrade`. Writing it on the
/// line makes the widening one step's, and a manifest that says nothing keeps today's meaning.
pub const EXEC_OPTION_KEYS: &[&str] = &["runs", "undo", "on", "after"];

/// Which verbs an `exec:` line belongs to.
///
/// A closed set rather than a comma-separated list, because options are *themselves* separated
/// by commas: `@on=sync,upgrade` parses as `on=sync` plus a second option named `upgrade`,
/// which is `F3`'s boundary confusion invited in by the value grammar. Three names spell the
/// three cases with nothing to disambiguate.
///
/// **The list lives with the type that means it.** This was three strings here and a `Verb`
/// enum in `model::exec`, which is a value the parser accepts and nothing understands the
/// moment the two drift.
pub use crate::model::exec::Verb as ExecVerb;
/// Empty, and stated as a table rather than as a special case in the validator: "what may
/// `generate:` carry" is then answered in the same place as it is for every other kind. It runs
/// every resolution to compute the current answer, so there is no `@runs` ceiling to set.
pub const GENERATE_OPTION_KEYS: &[&str] = &[];

/// `generate:` runs a command and reads declarations from its stdout (U33). It takes no options:
/// unlike `exec:`, it runs every resolution to compute the current answer, so there is no `@runs`
/// ceiling — a ceiling would freeze a stale set. The gate that matters is `allow_generators`
/// (off by default), enforced where the command is actually run, not in the grammar.
pub(super) fn validate_generate(origin: &Origin, name: &str, options: &Options) -> Result<()> {
    if name.trim().is_empty() {
        return Err(
            GrammarError::new(origin.clone(), "`generate:` names no command").with_hint(
                "write `generate:./bin/pick.sh` — a command whose stdout is declarations.",
            ),
        );
    }
    validate_extra_options(
        origin,
        OptionKind::Generate,
        name,
        options,
        Some(
            "a generator runs every resolution to compute the current set, so there is no \
             `@runs` ceiling to set.",
        ),
    )
}

pub(super) fn validate_exec(origin: &Origin, name: &str, options: &Options) -> Result<()> {
    if name.trim().is_empty() {
        return Err(GrammarError::new(origin.clone(), "`exec:` names no script")
            .with_hint("write `exec:./bin/setup.sh` — a path to a script the config carries."));
    }
    validate_extra_options(
        origin,
        OptionKind::Exec,
        name,
        options,
        Some(
            "an exec takes `runs` (a positive number, or `always`), `undo` (a command to run \
             when the line is removed), `on` (`sync`, `upgrade` or `both`), and `after` (how \n             many packages must have moved before it runs).",
        ),
    )?;
    if let Some(on) = options.one("on") {
        let on = on.trim();
        if !ExecVerb::VALUES.contains(&on) {
            return Err(GrammarError::new(
                origin.clone(),
                format!("`exec:{}` has an invalid `on={}`", name, on),
            )
            .with_hint(
                "`on` is `sync` (the default), `upgrade`, or `both`. It says which verb runs \
                 this step; a step `upgrade` should run has to say so, because approving a \
                 script is not the same as approving every verb to run it.",
            ));
        }
    }
    // Which names exist is a fact about the catalogue, so the catalogue answers it — see
    // `model::step::refusal`. Refused here rather than at run time, where "cannot read the
    // script at <config>/step/rustupp" would send a reader looking for a file they never meant
    // to write.
    if let Some((what, hint)) = crate::model::step::refusal(name) {
        return Err(GrammarError::new(origin.clone(), what).with_hint(hint));
    }
    // `@after=N` — run this step only once the run it belongs to has actually moved N
    // packages. Zero is refused rather than read as "always": a threshold of nothing is the
    // absence of a threshold, so writing it means the author expected it to mean something else.
    if let Some(after) = options.one("after") {
        let after = after.trim();
        match after.parse::<usize>() {
            Ok(0) => {
                return Err(GrammarError::new(
                    origin.clone(),
                    format!("`exec:{}` has `after=0`", name),
                )
                .with_hint(
                    "`after` is how many packages must have moved before this step runs, so \n                     `after=0` is the same as leaving it out. Delete it, or write \n                     the number you meant.",
                ));
            }
            Ok(_) => {}
            Err(_) => {
                return Err(GrammarError::new(
                    origin.clone(),
                    format!("`exec:{}` has a non-numeric `after={}`", name, after),
                )
                .with_hint(
                    "`after` is a count of packages, so it is a whole number: `@after=5` runs \n                     the step only on a run that moved at least five.",
                ));
            }
        }
    }
    if let Some(runs) = options.one("runs") {
        let runs = runs.trim();
        if runs != "always" && runs.parse::<u32>().map(|n| n == 0).unwrap_or(true) {
            return Err(GrammarError::new(
                origin.clone(),
                format!("`exec:{}` has an invalid `runs={}`", name, runs),
            )
            .with_hint(
                "`runs` is a positive number (the ceiling on how many times this \
                        content runs) or `always` to run every sync.",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod exec_tests {
    use super::super::statement::{parse, validate, ResourceKind, Statement};
    use super::*;

    fn o() -> Origin {
        Origin::new("modules/dev.txt", 7)
    }
    fn known(name: &str) -> bool {
        matches!(name, "apt" | "cargo")
    }
    fn p(line: &str) -> Result<Statement> {
        parse(&o(), line, &known)
    }
    /// Parse and validate, the way a real file is read — `parse` alone does not check options.
    fn pv(line: &str) -> Result<Statement> {
        let s = p(line)?;
        validate(&o(), &s)?;
        Ok(s)
    }

    #[test]
    fn an_exec_names_a_script() {
        let Statement::Exec(script, opts) = pv("exec:./bin/enroll-tpm.sh").unwrap() else {
            panic!("not an exec");
        };
        assert_eq!(script, "./bin/enroll-tpm.sh");
        assert!(opts.one("runs").is_none(), "no ceiling means the default");
    }

    #[test]
    fn a_generate_names_a_command() {
        let Statement::Generate(cmd, _) = pv("generate:./bin/pick.sh").unwrap() else {
            panic!("not a generate");
        };
        assert_eq!(cmd, "./bin/pick.sh");
        assert_eq!(
            pv("generate:./bin/pick.sh").unwrap().kind(),
            Some(ResourceKind::Generate)
        );
    }

    /// The keyword and the type are one fact, so they cannot drift apart.
    ///
    /// `ALL` is hand-written, which is the one thing about this type that a compiler does not
    /// check — so a variant added without being listed there would silently stop being parseable
    /// from a ledger key. That is caught below by parsing every keyword the grammar actually
    /// produces, rather than by trusting the list.
    #[test]
    fn every_resource_kind_round_trips_through_its_keyword() {
        let mut seen = std::collections::HashSet::new();
        for k in ResourceKind::ALL {
            assert_eq!(
                k.as_str().parse::<ResourceKind>(),
                Ok(*k),
                "`{k}` does not parse back to itself"
            );
            assert_eq!(k.to_string(), k.as_str());
            assert!(seen.insert(k.as_str()), "two kinds answer `{k}`");
        }
        assert!(
            "apt".parse::<ResourceKind>().is_err(),
            "a backend is not a kind"
        );
        assert!("".parse::<ResourceKind>().is_err());
    }

    /// **Every statement that has a keyword reports a kind in `ALL`, and its key opens with
    /// that kind.** `ALL` is the one hand-maintained part of the type, and the ledger's keys
    /// are parsed back through it — so a variant added without being listed would produce rows
    /// nothing could dispatch on.
    ///
    /// The `key`-opens-with-`kind` half is what `subject()` and `split_key` both assume, in two
    /// files, neither of which says so.
    #[test]
    fn every_statement_with_a_keyword_reports_a_listed_kind() {
        let opt = Options::default;
        let statements = [
            Statement::Repo {
                backend: "apt".into(),
                spec: "ppa:x/y".into(),
            },
            Statement::Shim("rg".into(), opt()),
            Statement::Schedule("nightly".into(), opt()),
            Statement::Service("nginx".into(), opt()),
            Statement::Link("./vimrc".into(), opt()),
            Statement::Setting("dark".into(), opt()),
            Statement::Exec("./bin/x.sh".into(), opt()),
            Statement::Generate("./bin/pick.sh".into(), opt()),
            Statement::Dotfiles("./tree".into(), opt()),
            Statement::Firewall("22/tcp".into(), opt()),
        ];
        let mut kinds = std::collections::HashSet::new();
        for stmt in &statements {
            let kind = stmt
                .kind()
                .unwrap_or_else(|| panic!("{stmt:?} reported no kind"));
            assert!(
                ResourceKind::ALL.contains(&kind),
                "{stmt:?} reports `{kind}`, which is not in ResourceKind::ALL"
            );
            assert!(
                stmt.key().starts_with(kind.as_str()),
                "`{}` does not open with its own kind `{kind}`",
                stmt.key()
            );
            assert!(stmt.subject().is_some(), "{stmt:?} has no subject");
            kinds.insert(kind);
        }
        assert_eq!(
            kinds.len(),
            ResourceKind::ALL.len(),
            "a kind has no statement here"
        );
    }

    #[test]
    fn a_generate_takes_no_options() {
        // It runs every resolution, so there is no `@runs` ceiling to set.
        let err = pv("generate:./pick.sh@runs=3").unwrap_err();
        assert!(err.what.contains("takes none"), "{}", err);
    }

    #[test]
    fn a_generate_with_no_command_is_an_error() {
        assert!(pv("generate:").is_err());
    }

    #[test]
    fn an_exec_takes_a_runs_ceiling() {
        let Statement::Exec(_, opts) = pv("exec:./setup.sh@runs=3").unwrap() else {
            panic!("not an exec");
        };
        assert_eq!(opts.one("runs"), Some("3"));
        let Statement::Exec(_, opts) = pv("exec:./tick.sh@runs=always").unwrap() else {
            panic!("not an exec");
        };
        assert_eq!(opts.one("runs"), Some("always"));
    }

    /// A path with punctuation is a path, not set math — the same rule that keeps a Windows
    /// `link:` target from being read as a difference.
    #[test]
    fn a_windows_path_is_a_script_not_an_expression() {
        let Statement::Exec(script, _) = pv(r"exec:C:\Users\me\bin\setup.ps1").unwrap() else {
            panic!("not an exec");
        };
        assert_eq!(script, r"C:\Users\me\bin\setup.ps1");
    }

    #[test]
    fn an_exec_that_names_nothing_is_refused() {
        assert!(p("exec:").is_err());
    }

    /// The refusal is the one every other kind gives — `exec:` used to phrase this itself,
    /// which is why its arm in the option table was unreachable. What must survive the sharing
    /// is the *hint*: "runs, undo" lists the spellings and explains neither, so exec keeps its
    /// own sentence about what they mean.
    #[test]
    fn an_unknown_exec_option_is_refused_and_names_the_real_one() {
        let err = pv("exec:./s.sh@run=2").unwrap_err();
        assert!(err.what.contains("`@run`"), "{}", err);
        assert!(err.what.contains("exec:"), "{}", err);
        assert!(err.to_string().contains("`runs`"), "{}", err);
        assert!(err.to_string().contains("`undo`"), "{}", err);
    }

    /// `runs=0` would mean "never runs", which is what deleting the line means. A ceiling that
    /// silently disables the statement is the kind of quiet no-op II.2 refuses.
    #[test]
    fn a_zero_or_garbage_ceiling_is_refused() {
        for bad in [
            "exec:./s.sh@runs=0",
            "exec:./s.sh@runs=lots",
            "exec:./s.sh@runs=-1",
        ] {
            assert!(pv(bad).is_err(), "{} was accepted", bad);
        }
    }

    /// `on` names a verb, and only the three the program has (`H6`).
    ///
    /// A fourth word must be refused here rather than read leniently downstream: `Verb::claims`
    /// falls back to `sync` for anything it does not know, which is the safe direction, and a
    /// grammar that let `@on=upgrde` through would silently give a user the opposite of what
    /// they wrote. `on=sync,upgrade` is refused for the same reason a value cannot carry a
    /// comma — that is the option separator, so it parses as a second option named `upgrade`.
    #[test]
    fn on_names_one_of_the_three_verbs_and_nothing_else() {
        for good in [
            "exec:./s.sh@on=sync",
            "exec:./s.sh@on=upgrade",
            "exec:./s.sh@on=both",
            "exec:./s.sh@runs=always,on=upgrade",
        ] {
            assert!(pv(good).is_ok(), "{} was refused", good);
        }
        for bad in [
            "exec:./s.sh@on=",
            "exec:./s.sh@on=always",
            "exec:./s.sh@on=Upgrade",
            "exec:./s.sh@on=sync,upgrade",
        ] {
            assert!(pv(bad).is_err(), "{} was accepted", bad);
        }
    }

    /// `exec:` is a verb: it must never be keyed into the extras teardown ledger, or a script
    /// whose `when` went false would be "undone" (XIII.3's flapping bug).
    #[test]
    fn an_exec_is_not_an_extra_with_a_teardown_key() {
        let stmt = pv("exec:./bin/enroll-tpm.sh").unwrap();
        assert_eq!(crate::core::extra_key(&stmt), None);
    }
}
