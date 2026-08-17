//! Turning a `schedule:` line into a provisionable task (S21).
//!
//! A `schedule:` line names a job and carries its options — when to run and what to run:
//!
//! ```text
//! schedule:nightly-tidy@cron=0 2 * * *,run=clean,notify=desktop
//!
//! schedule:nightly-tidy {
//!   cron = 0 2 * * *
//!   run = clean
//!   notify = desktop
//!   enabled = true       # provision it and leave it silent with `false`
//!   persistent = true    # run a firing the machine was switched off for
//!   jitter = 30m         # spread a fleet out around the scheduled moment
//!   elevated = false     # run with the highest privileges the account holds
//! }
//! ```
//!
//! The four options past `notify` are **not** universal, and this module does not pretend
//! otherwise: it parses and bounds them, and each provisioner in `app/scheduler` either
//! expresses the option or refuses it by name. Accepting an option and dropping it is the same
//! failure as a cron silently widened into DAILY — the declaration says one thing, the machine
//! does another, and both report success.
//!
//! The resolver collects these (from the `schedules` file only — II.2), and this module maps
//! one to the [`ScheduleConfig`] the existing `SchedulerManager` provisions onto systemd /
//! launchd / Task Scheduler. It is pure: no I/O, no provisioning — just the line-to-config
//! translation and the validation that a job a machine will run on a timer is fully specified.

use crate::config::config::ScheduleConfig;
use crate::config::grammar::{GrammarError, Options, Origin, Result};

/// The keys a `schedule:` line understands live with the rest of II.2's option tables, in the
/// grammar. `cron` and `run` are required — a timed job with no schedule or no command is not
/// a job — and `notify` is optional.
use crate::config::grammar::statement::SCHEDULE_OPTION_KEYS as KNOWN_KEYS;

/// Build a [`ScheduleConfig`] from a `schedule:<name>` line's options, or an error that names
/// the file and line and says exactly what is missing or unrecognized.
pub fn schedule_config(
    name: &str,
    options: &Options,
    origin: &Origin,
    never_unattended: &[String],
) -> Result<ScheduleConfig> {
    // Unknown keys are an error, not ignored: a typo'd `crron =` would otherwise leave the job
    // with no schedule and no complaint, which is the class of silent failure II.2 refuses.
    for key in options.keys() {
        if !KNOWN_KEYS.contains(&key) {
            return Err(GrammarError::new(
                origin.clone(),
                format!("`schedule:{}` has an unknown option `{}`", name, key),
            )
            .with_hint(format!(
                "a schedule takes {}.",
                KNOWN_KEYS
                    .iter()
                    .map(|k| format!("`{}`", k))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
    }

    let cron = required(name, options, "cron", origin)?;
    validate_cron(&cron).map_err(|e| {
        GrammarError::new(
            origin.clone(),
            format!("`schedule:{}` has an invalid cron: {}", name, e),
        )
        .with_hint(
            "five fields (min hour dom month dow), e.g. `0 2 * * *`, or a macro like `@daily`.",
        )
    })?;
    let command = required(name, options, "run", origin)?;
    refuse_unattended(name, &command, origin, never_unattended)?;
    let notification = options.one("notify").map(str::to_string);

    Ok(ScheduleConfig {
        name: name.to_string(),
        cron,
        command,
        notification,
        enabled: boolean(name, options, "enabled", origin)?,
        persistent: boolean(name, options, "persistent", origin)?,
        jitter: seconds(name, options, "jitter", origin)?,
        elevated: boolean(name, options, "elevated", origin)?,
    })
}

/// A yes/no option, or an error naming the line.
///
/// **Anything that is not a spelling of true or false is refused**, rather than read as false
/// the way `service:`'s `enabled` reads it. A typo that silently means "no" on an option whose
/// job is to stop a job firing is the quiet failure this file's unknown-key check exists to
/// prevent, one level down.
fn boolean(name: &str, options: &Options, key: &str, origin: &Origin) -> Result<Option<bool>> {
    let Some(raw) = options.one(key) else {
        return Ok(None);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Ok(Some(true)),
        "false" | "no" | "off" | "0" => Ok(Some(false)),
        other => Err(GrammarError::new(
            origin.clone(),
            format!(
                "`schedule:{}` has `{} = {}`, which is not yes or no",
                name, key, other
            ),
        )
        .with_hint("write `true` or `false` (`yes`/`no`, `on`/`off` and `1`/`0` also read).")),
    }
}

/// The longest jitter that is still a jitter. A day of randomised delay on a daily timer means
/// the firing can land after the next one is due, which is not a spread — it is a schedule
/// nobody can predict.
const MAX_JITTER_SECONDS: u32 = 24 * 60 * 60;

/// A duration in seconds, written bare or with a `s`/`m`/`h` suffix.
fn seconds(name: &str, options: &Options, key: &str, origin: &Origin) -> Result<Option<u32>> {
    let Some(raw) = options.one(key) else {
        return Ok(None);
    };
    let raw = raw.trim();
    let bad = |what: &str| {
        Err(GrammarError::new(
            origin.clone(),
            format!(
                "`schedule:{}` has `{} = {}`, which {}",
                name, key, raw, what
            ),
        )
        .with_hint(format!(
            "write a duration: `900`, `15m`, `2h`. The longest this option takes is {}h.",
            MAX_JITTER_SECONDS / 3600
        )))
    };
    let (digits, multiplier) = match raw.chars().last() {
        Some('s') | Some('S') => (&raw[..raw.len() - 1], 1),
        Some('m') | Some('M') => (&raw[..raw.len() - 1], 60),
        Some('h') | Some('H') => (&raw[..raw.len() - 1], 3600),
        _ => (raw, 1),
    };
    let Ok(value) = digits.trim().parse::<u32>() else {
        return bad("is not a duration");
    };
    let Some(total) = value.checked_mul(multiplier) else {
        return bad("is longer than any schedule");
    };
    if total > MAX_JITTER_SECONDS {
        return bad("is longer than a day");
    }
    Ok(Some(total))
}

/// Refuse a `run` whose command is on this machine's `[guard] never_unattended` list (K13).
///
/// The list arrives as an argument rather than being read here so the rule stays one fact in
/// one place — `preferences.toml` — and so the check is testable without a config on disk.
fn refuse_unattended(
    name: &str,
    command: &str,
    origin: &Origin,
    never_unattended: &[String],
) -> Result<()> {
    let head = command.split_whitespace().next().unwrap_or("");
    if never_unattended.iter().any(|c| c == head) {
        return Err(GrammarError::new(
            origin.clone(),
            format!("`schedule:{}` may not run `{}`", name, head),
        )
        .with_hint(format!(
            "a schedule runs unattended, and this command removes software. `{}` is in \
             `[guard] never_unattended` in preferences.toml, which currently reads [{}]; take \
             it out of that list to permit it, or run the command yourself.",
            head,
            never_unattended.join(", ")
        )));
    }
    Ok(())
}

fn required(name: &str, options: &Options, key: &str, origin: &Origin) -> Result<String> {
    match options.one(key) {
        Some(v) if !v.trim().is_empty() => Ok(v.to_string()),
        _ => Err(GrammarError::new(
            origin.clone(),
            format!("`schedule:{}` is missing `{}`", name, key),
        )
        .with_hint(match key {
            "cron" => "add `cron = <expression>`, e.g. `cron = 0 2 * * *` for 2am daily.",
            "run" => "add `run = <command>`, e.g. `run = clean`.",
            _ => "a required option is missing.",
        })),
    }
}

/// Is this a cron expression the OS scheduler will accept?
///
/// Standard cron is 5-field (min hour dom month dow) and the `cron` crate wants 6 with
/// seconds, so a 5-field expression is normalized by prepending `0`. `@`-macros never reach
/// the parser — the systemd/launchd mapping handles those.
///
/// One implementation, called at parse time so the error can name the file and line, and
/// again by the provisioner for the config it is handed.
pub fn validate_cron(cron: &str) -> std::result::Result<(), String> {
    if cron.starts_with('@') {
        return Ok(());
    }
    let normalized = if cron.split_whitespace().count() == 5 {
        format!("0 {}", cron)
    } else {
        cron.to_string()
    };
    normalized
        .parse::<cron::Schedule>()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Write a `schedule:` block into the `schedules` file body.
///
/// `schedule add` is a shortcut for editing this file, the way `install` is a shortcut for
/// editing a module (P1): the file is the state, so the edit IS the command and `sync`
/// provisions it. There is no second store — a `[schedules]` table in preferences was one,
/// and the two could disagree about what this machine runs.
pub fn add_line(
    body: &str,
    name: &str,
    cron: &str,
    run: &str,
    notify: Option<&str>,
) -> std::result::Result<String, String> {
    if find_block(body, name).is_some() {
        return Err(format!(
            "`schedule:{}` is already in the `schedules` file. Remove it first, or edit the \
             file — two lines with one name is a schedule that means whichever the reader \
             saw last.",
            name
        ));
    }

    let mut out = body.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&format!("schedule:{} {{\n", name));
    out.push_str(&format!("  cron = {}\n", cron));
    out.push_str(&format!("  run  = {}\n", run));
    if let Some(n) = notify {
        out.push_str(&format!("  notify = {}\n", n));
    }
    out.push_str("}\n");
    Ok(out)
}

/// Take a `schedule:` line or block out of the body. `None` if no such name is in the file.
pub fn remove_line(body: &str, name: &str) -> Option<String> {
    let (start, end) = find_block(body, name)?;
    let kept: Vec<&str> = body
        .lines()
        .enumerate()
        .filter(|(i, _)| *i < start || *i > end)
        .map(|(_, l)| l)
        .collect();
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    Some(out)
}

/// The 0-based line range a `schedule:NAME` occupies — one line in short form, header
/// through `}` in block form.
fn find_block(body: &str, name: &str) -> Option<(usize, usize)> {
    let lines: Vec<&str> = body.lines().collect();
    let header = format!("schedule:{}", name);
    for (i, raw) in lines.iter().enumerate() {
        let text = match raw.find('#') {
            Some(c) => &raw[..c],
            None => raw,
        }
        .trim();
        // `schedule:nightly` must not match `schedule:nightly-tidy`, so the name has to end
        // where the header does or be followed by an option, a brace or space.
        let Some(rest) = text.strip_prefix(&header) else {
            continue;
        };
        if rest.starts_with(|c: char| c.is_alphanumeric() || c == '-' || c == '_') {
            continue;
        }
        if crate::config::grammar::block_header(rest).is_none() {
            return Some((i, i));
        }
        for (j, close) in lines.iter().enumerate().skip(i + 1) {
            if close.trim() == "}" {
                return Some((i, j));
            }
        }
        return Some((i, lines.len().saturating_sub(1)));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(pairs: &[(&str, &str)]) -> Options {
        let mut o = Options::default();
        for (k, v) in pairs {
            o.insert(*k, *v);
        }
        o
    }

    fn origin() -> Origin {
        Origin::new("schedules", 3)
    }

    /// What `[guard] never_unattended` ships with. Tests take it from the config default so a
    /// change to the shipped set cannot leave these asserting a list nobody has.
    fn shipped() -> Vec<String> {
        crate::config::Config::default().guard.never_unattended
    }

    #[test]
    fn cron_accepts_five_fields_six_fields_and_macros_and_refuses_garbage() {
        for good in ["30 4 * * 1", "0 30 4 * * 1", "@daily", "0 2 * * *"] {
            assert!(validate_cron(good).is_ok(), "{} was refused", good);
        }
        assert!(validate_cron("not a cron").is_err());
    }

    /// A bad cron is refused where the line is read, so the error can name the file and the
    /// line rather than surfacing when the OS scheduler is handed the job.
    #[test]
    fn a_bad_cron_is_refused_at_parse_time() {
        let o = opts(&[("cron", "not a cron"), ("run", "sync")]);
        let err = schedule_config("nightly", &o, &origin(), &shipped()).unwrap_err();
        assert!(err.to_string().contains("schedules:3"), "{}", err);
    }

    #[test]
    fn add_line_writes_a_block_and_refuses_a_duplicate_name() {
        let body = add_line("", "nightly", "0 2 * * *", "sync", None).unwrap();
        assert!(body.contains("schedule:nightly {"));
        assert!(body.contains("cron = 0 2 * * *"));
        assert!(add_line(&body, "nightly", "0 3 * * *", "clean", None).is_err());
    }

    #[test]
    fn remove_line_takes_the_whole_block_and_leaves_its_neighbours() {
        let body = "schedule:nightly {\n  cron = 0 2 * * *\n  run  = sync\n}\n\
                    schedule:weekly {\n  cron = @weekly\n  run  = clean\n}\n";
        let out = remove_line(body, "nightly").unwrap();
        assert!(!out.contains("nightly"));
        assert!(out.contains("schedule:weekly {"));
        assert!(out.contains("run  = clean"));
        assert!(remove_line(&out, "nosuch").is_none());
    }

    /// `schedule:nightly` must not take `schedule:nightly-tidy` with it.
    #[test]
    fn a_name_that_prefixes_another_is_not_a_match() {
        let body = "schedule:nightly-tidy {\n  cron = @daily\n  run  = clean\n}\n";
        assert!(remove_line(body, "nightly").is_none());
        assert!(remove_line(body, "nightly-tidy").is_some());
    }

    #[test]
    fn a_complete_line_maps_to_a_schedule_config() {
        let o = opts(&[
            ("cron", "0 2 * * *"),
            ("run", "clean"),
            ("notify", "desktop"),
        ]);
        let cfg = schedule_config("nightly-tidy", &o, &origin(), &shipped()).unwrap();
        assert_eq!(cfg.name, "nightly-tidy");
        assert_eq!(cfg.cron, "0 2 * * *");
        assert_eq!(cfg.command, "clean");
        assert_eq!(cfg.notification.as_deref(), Some("desktop"));
        // Nothing declared is nothing decided: every option a line does not mention arrives as
        // `None`, which is what stops a scheduler refusing an option nobody wrote.
        assert_eq!(cfg.enabled, None);
        assert_eq!(cfg.persistent, None);
        assert_eq!(cfg.jitter, None);
        assert_eq!(cfg.elevated, None);
    }

    #[test]
    fn every_spelling_of_yes_and_no_reads_and_anything_else_is_refused() {
        for (written, want) in [
            ("true", true),
            ("yes", true),
            ("on", true),
            ("1", true),
            ("TRUE", true),
            ("false", false),
            ("no", false),
            ("off", false),
            ("0", false),
        ] {
            // Every boolean option, not just the one that was thought of: three keys go through
            // the same function and a check on one of them says nothing about the others.
            for key in ["enabled", "persistent", "elevated"] {
                let o = opts(&[("cron", "0 2 * * *"), ("run", "clean"), (key, written)]);
                let cfg = schedule_config("t", &o, &origin(), &shipped()).unwrap();
                let got = match key {
                    "enabled" => cfg.enabled,
                    "persistent" => cfg.persistent,
                    _ => cfg.elevated,
                };
                assert_eq!(got, Some(want), "`{} = {}` read wrong", key, written);
            }
        }
        for key in ["enabled", "persistent", "elevated"] {
            let o = opts(&[("cron", "0 2 * * *"), ("run", "clean"), (key, "maybe")]);
            let err = schedule_config("t", &o, &origin(), &shipped()).unwrap_err();
            assert!(err.what.contains("not yes or no"), "{}", err);
            assert!(err.to_string().contains("schedules:3"), "{}", err);
        }
    }

    #[test]
    fn a_jitter_is_a_duration_and_a_bounded_one() {
        for (written, want) in [
            ("900", 900u32),
            ("15m", 900),
            ("2h", 7200),
            ("45s", 45),
            ("24h", 86400),
        ] {
            let o = opts(&[("cron", "0 2 * * *"), ("run", "clean"), ("jitter", written)]);
            let cfg = schedule_config("t", &o, &origin(), &shipped()).unwrap();
            assert_eq!(cfg.jitter, Some(want), "`jitter = {}` read wrong", written);
        }
        // Longer than a day, and an overflow that would wrap to a small number if it were
        // multiplied without asking.
        for written in ["25h", "2d", "soon", "-5", "4294967295h", ""] {
            let o = opts(&[("cron", "0 2 * * *"), ("run", "clean"), ("jitter", written)]);
            assert!(
                schedule_config("t", &o, &origin(), &shipped()).is_err(),
                "`jitter = {}` was accepted",
                written
            );
        }
    }

    #[test]
    fn notify_is_optional() {
        let o = opts(&[("cron", "0 2 * * *"), ("run", "clean")]);
        let cfg = schedule_config("t", &o, &origin(), &shipped()).unwrap();
        assert!(cfg.notification.is_none());
    }

    #[test]
    fn a_missing_cron_is_an_error_that_names_the_line() {
        let o = opts(&[("run", "clean")]);
        let err = schedule_config("t", &o, &origin(), &shipped()).unwrap_err();
        assert!(err.what.contains("missing `cron`"), "{}", err);
        assert!(err.to_string().contains("schedules:3"), "{}", err);
    }

    #[test]
    fn a_missing_run_is_an_error() {
        let o = opts(&[("cron", "0 2 * * *")]);
        let err = schedule_config("t", &o, &origin(), &shipped()).unwrap_err();
        assert!(err.what.contains("missing `run`"), "{}", err);
    }

    #[test]
    fn an_empty_cron_value_is_treated_as_missing() {
        let o = opts(&[("cron", "  "), ("run", "clean")]);
        assert!(schedule_config("t", &o, &origin(), &shipped()).is_err());
    }

    #[test]
    fn a_timer_may_not_run_rebuild_or_purge_undeclared_out_of_the_box() {
        // K13. Both shipped names, not just the one the ruling was asked about: a check that
        // covers `rebuild` alone is how `purge-undeclared` came to be refused by a constant
        // nobody could edit.
        for command in ["rebuild --all", "purge-undeclared"] {
            let o = opts(&[("cron", "0 2 * * *"), ("run", command)]);
            let err = schedule_config("nightly", &o, &origin(), &shipped()).unwrap_err();
            let head = command.split_whitespace().next().unwrap();
            assert!(
                err.what.contains(&format!("may not run `{}`", head)),
                "{}",
                err
            );
        }
    }

    /// The refusal names the list and its contents, so the way out is in the error rather than
    /// in the documentation.
    #[test]
    fn the_refusal_names_the_list_it_came_from() {
        let o = opts(&[("cron", "0 2 * * *"), ("run", "rebuild")]);
        let err = schedule_config("nightly", &o, &origin(), &shipped())
            .unwrap_err()
            .to_string();
        assert!(err.contains("never_unattended"), "{}", err);
        assert!(err.contains("rebuild, purge-undeclared"), "{}", err);
    }

    /// Taking a name out of the list is how a machine permits that command — the whole point
    /// of the ruling that replaced the constant.
    #[test]
    fn taking_a_name_out_of_the_list_permits_the_command() {
        let permitting = vec!["purge-undeclared".to_string()];
        let o = opts(&[("cron", "0 2 * * *"), ("run", "rebuild --all")]);
        assert!(schedule_config("nightly", &o, &origin(), &permitting).is_ok());

        // And the name still in the list is still refused, so the edit is per-command.
        let o = opts(&[("cron", "0 2 * * *"), ("run", "purge-undeclared")]);
        assert!(schedule_config("nightly", &o, &origin(), &permitting).is_err());
    }

    /// An empty list refuses nothing. Stated as a test because the alternative — a list whose
    /// emptiness silently restores the built-in pair — is the shape that makes a guard setting
    /// unable to mean what it says.
    #[test]
    fn an_empty_list_refuses_nothing() {
        let o = opts(&[("cron", "0 2 * * *"), ("run", "rebuild")]);
        assert!(schedule_config("nightly", &o, &origin(), &[]).is_ok());
    }

    #[test]
    fn the_refusal_reads_the_command_not_the_whole_line() {
        // `run = sync --rebuild-cache` is not `run = rebuild`.
        let o = opts(&[("cron", "0 2 * * *"), ("run", "sync --locked")]);
        assert!(schedule_config("t", &o, &origin(), &shipped()).is_ok());
    }

    #[test]
    fn an_unknown_key_is_refused_not_ignored() {
        // A typo like `crron =` must not leave the job silently unscheduled.
        let o = opts(&[("crron", "0 2 * * *"), ("run", "clean")]);
        let err = schedule_config("t", &o, &origin(), &shipped()).unwrap_err();
        assert!(err.what.contains("unknown option `crron`"), "{}", err);
    }
}
