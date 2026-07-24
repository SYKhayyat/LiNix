//! Health checks, and what a failing one means (XIII.5, U7).
//!
//! **Two scopes, one revert path (U7, ruled 2026-07-24).** `@health=` on a line answers *did
//! this upgrade break this*; a `health` list in `preferences.toml` catches what a package
//! cannot see — the boot, the network, the thing two packages away. They are not alternatives,
//! and the machine does not care which kind noticed: a failure restores the snapshot either way.
//!
//! **The precondition is the feature.** A health check that cannot revert is a health check
//! that tells you the machine is broken and leaves it broken. So the absence of a snapshot
//! provider is decided *before* anything is installed, not discovered afterwards when the
//! reverting would have mattered.
//!
//! Pure: parsing a probe, and deciding what a set of results means. Running a command and
//! restoring a snapshot belong to the caller.

/// One thing to check after a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// `port:8080` — succeeds when a TCP connection to localhost opens.
    Port(u16),
    /// `cmd:systemctl is-active nginx`, or a bare command. Succeeds on exit 0.
    Command(String),
}

impl Probe {
    /// Parse a probe as written on a line or in `preferences.toml`.
    ///
    /// A bare string is a command, because that is what people write. `port:` is the one
    /// prefix, because "is something listening" is the check nobody can spell portably as a
    /// command — `nc`, `ss` and `Test-NetConnection` are three answers to one question.
    pub fn parse(written: &str) -> Option<Probe> {
        let written = written.trim();
        if written.is_empty() {
            return None;
        }
        if let Some(port) = written.strip_prefix("port:") {
            // A malformed port is not silently a command named "port:donkey": the line says
            // port, so it is a port or it is an error the caller reports.
            return port.trim().parse().ok().map(Probe::Port);
        }
        let cmd = written.strip_prefix("cmd:").unwrap_or(written).trim();
        if cmd.is_empty() {
            return None;
        }
        Some(Probe::Command(cmd.to_string()))
    }
}

impl std::fmt::Display for Probe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Probe::Port(p) => write!(f, "port:{}", p),
            Probe::Command(c) => f.write_str(c),
        }
    }
}

/// A probe and what it belongs to, so a failure can name the thing that broke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// `apt:nginx` for a line's check, or `preferences.toml` for a machine-wide one.
    pub subject: String,
    pub probe: Probe,
}

/// What to do once the probes have run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Everything passed, or nothing was declared.
    Healthy,
    /// Something failed and there is a snapshot to go back to.
    Revert { failed: Vec<String>, snapshot: String },
    /// Something failed and there is nothing to revert to. The machine is left as it is —
    /// LiNix will not pretend it fixed anything — and the failure is loud.
    FailedWithoutRevert { failed: Vec<String> },
}

impl Outcome {
    /// Decide from the failures and the snapshot taken before the change.
    pub fn of(failed: Vec<String>, snapshot: Option<&str>) -> Outcome {
        if failed.is_empty() {
            return Outcome::Healthy;
        }
        match snapshot {
            Some(id) => Outcome::Revert {
                failed,
                snapshot: id.to_string(),
            },
            None => Outcome::FailedWithoutRevert { failed },
        }
    }
}

/// Whether to refuse before the change, and what to say.
///
/// The whole pre-flight decision, so it can be tested without a machine that happens to lack a
/// snapshot provider — the condition it exists for is the one hardest to reproduce on the
/// developer's own box, which is exactly why it must not live only in the wiring.
///
/// A dry run never refuses: it changes nothing, so there is nothing it might fail to revert.
pub fn refusal_if_unrevertable(
    checks: &[Check],
    has_provider: bool,
    dry_run: bool,
) -> Option<String> {
    if dry_run || has_provider || checks.is_empty() {
        return None;
    }
    Some(cannot_revert_refusal(checks))
}

/// The refusal issued *before* a change, when health checks are declared and nothing could
/// revert them.
///
/// 7f's exit condition in those words: it fails loudly and says it cannot revert, **before it
/// starts**. Finding this out afterwards is finding it out too late — the upgrade has already
/// happened and the check can only confirm the damage.
pub fn cannot_revert_refusal(checks: &[Check]) -> String {
    let mut named: Vec<&str> = checks.iter().map(|c| c.subject.as_str()).collect();
    named.sort_unstable();
    named.dedup();
    format!(
        "refusing to start: {} health check(s) are declared ({}) and this machine has no \
         snapshot provider, so a failing check could not revert anything.\n  \
         A health check that cannot revert reports the breakage and leaves it in place, which \
         is worse than not checking — you would have been told the machine is broken and given \
         no way back.\n  \
         Either set up a snapshot provider (btrfs, timeshift, or Windows System Restore), or \
         remove the health checks.",
        checks.len(),
        named.join(", ")
    )
}

/// What a revert says, in 7f's words.
pub fn reverted_message(failed: &[String], snapshot: &str) -> String {
    format!(
        "health check failed ({}) — restoring the snapshot taken before this change ({}).",
        failed.join(", "),
        snapshot
    )
}

/// What a failure with nothing to revert to says.
pub fn not_reverted_message(failed: &[String]) -> String {
    format!(
        "health check failed ({}) and there is no snapshot to restore — the machine is left as \
         the change made it.",
        failed.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_string_is_a_command() {
        assert_eq!(
            Probe::parse("systemctl is-active nginx"),
            Some(Probe::Command("systemctl is-active nginx".into()))
        );
        assert_eq!(
            Probe::parse("cmd:true"),
            Some(Probe::Command("true".into()))
        );
    }

    #[test]
    fn a_port_probe_parses_to_a_port() {
        assert_eq!(Probe::parse("port:8080"), Some(Probe::Port(8080)));
        assert_eq!(Probe::parse("port: 443 "), Some(Probe::Port(443)));
    }

    /// A malformed port must not become a command named `port:donkey`. The line says port, so
    /// it is a port or it is nothing — silently running it as a shell command would be the
    /// worst reading of a typo.
    #[test]
    fn a_malformed_port_is_not_quietly_a_command() {
        assert_eq!(Probe::parse("port:donkey"), None);
        assert_eq!(Probe::parse("port:99999"), None);
        assert_eq!(Probe::parse("port:"), None);
    }

    #[test]
    fn an_empty_probe_is_nothing() {
        assert_eq!(Probe::parse(""), None);
        assert_eq!(Probe::parse("   "), None);
        assert_eq!(Probe::parse("cmd:  "), None);
    }

    #[test]
    fn a_probe_round_trips_through_its_display() {
        for written in ["port:8080", "systemctl is-active nginx"] {
            let p = Probe::parse(written).unwrap();
            assert_eq!(p.to_string(), written);
        }
    }

    #[test]
    fn nothing_failing_is_healthy() {
        assert_eq!(Outcome::of(vec![], Some("snap-1")), Outcome::Healthy);
        assert_eq!(Outcome::of(vec![], None), Outcome::Healthy);
    }

    /// U7's ruling: one revert path. The outcome does not record which *kind* of check failed,
    /// because the machine does not care — a broken boot and a broken nginx both mean go back.
    #[test]
    fn a_failure_with_a_snapshot_reverts() {
        let out = Outcome::of(vec!["apt:nginx".into()], Some("snap-1"));
        assert_eq!(
            out,
            Outcome::Revert {
                failed: vec!["apt:nginx".into()],
                snapshot: "snap-1".into()
            }
        );
    }

    /// Without a snapshot the failure is loud and nothing is pretended. This is the state the
    /// pre-flight refusal exists to prevent ever reaching.
    #[test]
    fn a_failure_without_a_snapshot_does_not_pretend_to_revert() {
        let out = Outcome::of(vec!["boot".into()], None);
        assert!(matches!(out, Outcome::FailedWithoutRevert { .. }));
    }

    /// The refusal must say the two things a reader needs: that it will not start, and why the
    /// missing snapshot provider is what stopped it.
    #[test]
    fn the_refusal_names_the_checks_and_the_reason() {
        let checks = vec![
            Check {
                subject: "apt:nginx".into(),
                probe: Probe::Port(80),
            },
            Check {
                subject: "preferences.toml".into(),
                probe: Probe::Command("true".into()),
            },
        ];
        let msg = cannot_revert_refusal(&checks);
        assert!(msg.contains("refusing to start"), "{}", msg);
        assert!(msg.contains("apt:nginx"), "{}", msg);
        assert!(msg.contains("preferences.toml"), "{}", msg);
        assert!(msg.contains("no snapshot provider"), "{}", msg);
    }

    /// One subject with several checks is named once, not once per check.
    #[test]
    fn the_refusal_does_not_repeat_a_subject() {
        let checks = vec![
            Check {
                subject: "apt:nginx".into(),
                probe: Probe::Port(80),
            },
            Check {
                subject: "apt:nginx".into(),
                probe: Probe::Port(443),
            },
        ];
        let msg = cannot_revert_refusal(&checks);
        assert_eq!(msg.matches("apt:nginx").count(), 1, "{}", msg);
        // ...but the count is of checks, not subjects: two things are being checked.
        assert!(msg.contains("2 health check(s)"), "{}", msg);
    }

    fn one_check() -> Vec<Check> {
        vec![Check {
            subject: "apt:nginx".into(),
            probe: Probe::Port(80),
        }]
    }

    /// 7f's exit condition: with no snapshot provider, a declared health check stops the run
    /// **before it starts**.
    #[test]
    fn declared_checks_with_no_way_back_refuse_before_starting() {
        let msg = refusal_if_unrevertable(&one_check(), false, false)
            .expect("this must refuse — there is no way back");
        assert!(msg.contains("refusing to start"), "{}", msg);
    }

    #[test]
    fn a_provider_makes_the_same_checks_fine() {
        assert_eq!(refusal_if_unrevertable(&one_check(), true, false), None);
    }

    /// No checks declared, no opinion — the refusal must not fire on machines that never asked
    /// for health checks, which is most of them.
    #[test]
    fn no_checks_means_no_refusal_either_way() {
        assert_eq!(refusal_if_unrevertable(&[], false, false), None);
        assert_eq!(refusal_if_unrevertable(&[], true, false), None);
    }

    /// A dry run changes nothing, so there is nothing it might fail to revert. Refusing a
    /// preview would make `--dry-run` unusable on exactly the machines that most need to see
    /// what a sync would do before committing to it.
    #[test]
    fn a_dry_run_never_refuses() {
        assert_eq!(refusal_if_unrevertable(&one_check(), false, true), None);
    }

    #[test]
    fn the_revert_message_says_it_is_restoring_the_snapshot() {
        let msg = reverted_message(&["apt:nginx".to_string()], "snap-1");
        assert!(msg.contains("health check failed"), "{}", msg);
        assert!(msg.contains("restoring the snapshot"), "{}", msg);
        assert!(msg.contains("snap-1"), "{}", msg);
    }
}
