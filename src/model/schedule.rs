//! Turning a `schedule:` line into a provisionable task (S21).
//!
//! A `schedule:` line names a job and carries its options — when to run and what to run:
//!
//! ```text
//! schedule:nightly-tidy { cron = 0 2 * * *; run = clean; notify = desktop }
//! ```
//!
//! The resolver collects these (from the `schedules` file only — II.2), and this module maps
//! one to the [`ScheduleConfig`] the existing `SchedulerManager` provisions onto systemd /
//! launchd / Task Scheduler. It is pure: no I/O, no provisioning — just the line-to-config
//! translation and the validation that a job a machine will run on a timer is fully specified.

use crate::config::config::ScheduleConfig;
use crate::config::grammar::{GrammarError, Options, Origin, Result};

/// The keys a `schedule:` line understands. `cron` and `run` are required — a timed job with
/// no schedule or no command is not a job — and `notify` is optional.
const KNOWN_KEYS: &[&str] = &["cron", "run", "notify"];

/// Build a [`ScheduleConfig`] from a `schedule:<name>` line's options, or an error that names
/// the file and line and says exactly what is missing or unrecognized.
pub fn schedule_config(name: &str, options: &Options, origin: &Origin) -> Result<ScheduleConfig> {
    // Unknown keys are an error, not ignored: a typo'd `crron =` would otherwise leave the job
    // with no schedule and no complaint, which is the class of silent failure II.2 refuses.
    for key in options.keys() {
        if !KNOWN_KEYS.contains(&key) {
            return Err(GrammarError::new(
                origin.clone(),
                format!("`schedule:{}` has an unknown option `{}`", name, key),
            )
            .with_hint("a schedule takes `cron`, `run`, and optional `notify`."));
        }
    }

    let cron = required(name, options, "cron", origin)?;
    let command = required(name, options, "run", origin)?;
    let notification = options.one("notify").map(str::to_string);

    Ok(ScheduleConfig {
        name: name.to_string(),
        cron,
        command,
        notification,
        last_synced: None,
    })
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

    #[test]
    fn a_complete_line_maps_to_a_schedule_config() {
        let o = opts(&[("cron", "0 2 * * *"), ("run", "clean"), ("notify", "desktop")]);
        let cfg = schedule_config("nightly-tidy", &o, &origin()).unwrap();
        assert_eq!(cfg.name, "nightly-tidy");
        assert_eq!(cfg.cron, "0 2 * * *");
        assert_eq!(cfg.command, "clean");
        assert_eq!(cfg.notification.as_deref(), Some("desktop"));
        assert!(cfg.last_synced.is_none());
    }

    #[test]
    fn notify_is_optional() {
        let o = opts(&[("cron", "0 2 * * *"), ("run", "clean")]);
        let cfg = schedule_config("t", &o, &origin()).unwrap();
        assert!(cfg.notification.is_none());
    }

    #[test]
    fn a_missing_cron_is_an_error_that_names_the_line() {
        let o = opts(&[("run", "clean")]);
        let err = schedule_config("t", &o, &origin()).unwrap_err();
        assert!(err.what.contains("missing `cron`"), "{}", err);
        assert!(err.to_string().contains("schedules:3"), "{}", err);
    }

    #[test]
    fn a_missing_run_is_an_error() {
        let o = opts(&[("cron", "0 2 * * *")]);
        let err = schedule_config("t", &o, &origin()).unwrap_err();
        assert!(err.what.contains("missing `run`"), "{}", err);
    }

    #[test]
    fn an_empty_cron_value_is_treated_as_missing() {
        let o = opts(&[("cron", "  "), ("run", "clean")]);
        assert!(schedule_config("t", &o, &origin()).is_err());
    }

    #[test]
    fn an_unknown_key_is_refused_not_ignored() {
        // A typo like `crron =` must not leave the job silently unscheduled.
        let o = opts(&[("crron", "0 2 * * *"), ("run", "clean")]);
        let err = schedule_config("t", &o, &origin()).unwrap_err();
        assert!(err.what.contains("unknown option `crron`"), "{}", err);
    }
}
