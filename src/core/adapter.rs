//! One mechanism for every adapter table (K17).
//!
//! **K17's ruling was applied seven times and implemented seven times.** "Adapters are a table,
//! and the built-ins are rows in it" is the reason `setting:` reaches a store nobody wrote an
//! arm for — and firewalls, init systems, snapshot providers, bootstrap commands, prereq setup
//! steps and secret providers each went on to make the same move separately. Seven row types is
//! correct: a firewall's argv is `allow`/`deny`, an init's is `start`/`stop`, a settings store's
//! is `read`/`write`/`reset`, and folding those into one schema would be a struct with twenty
//! optional fields. Seven copies of the *machinery around* them is not.
//!
//! What was written seven times: the `os` filter, the floor a row has to clear to be acted on,
//! the "shipped rows then the user's" merge, the "which row describes this machine" search, and
//! `{placeholder}` substitution. Four of the five had already drifted:
//!
//! - **`[[secret]]` had no `os` field at all** while every one of its six siblings did, so the
//!   one table whose rows hand a command a plaintext secret was the one that could not be
//!   restricted to the platform it was written for.
//! - **Three tables refused a duplicate name and three did not**, so whether a second row
//!   claiming a name was reported or silently kept depended on which table it was in.
//! - **The `os` question had two spellings** — `applies_to_this_os()` reading
//!   `std::env::consts::OS` and `applies_here(os)` taking it as a parameter — which is two
//!   answers to one question and no way to test the first.
//! - **The floor was seven near-copies**, agreeing on the first check and diverging after it.
//!
//! A row now says what it is (`name`, `only_on`, `why_unusable`) and this module answers
//! everything asked *about* rows. Adding an eighth table means implementing three methods, not
//! copying a loader.

/// A row in an adapter table: something Shall drives, which a user can add to without waiting
/// for a release.
pub trait AdapterRow {
    /// What this table is called in the sentence a dropped row prints — "firewall adapter",
    /// "init adapter". Read by [`usable`] and [`merge`], which is why it belongs to the type
    /// rather than to each call.
    const WHAT: &'static str;

    /// Why a row with an empty key is dropped.
    ///
    /// Overridable because the tables spell their key differently — most call it `name`, and
    /// `[[bootstrap]]`/`[[prereq]]` call it `manager`. A message naming the wrong field sends
    /// a reader to a line that is not the problem.
    const NAMELESS: &'static str = "it has no `name`";

    /// What this row is known by, and what a duplicate claims.
    fn name(&self) -> &str;

    /// The single OS this row is restricted to (`std::env::consts::OS`). `None` means any,
    /// which is the right default: a row that names no platform was written for all of them.
    fn only_on(&self) -> Option<&str> {
        None
    }

    /// Why Shall will not act on this row, beyond the empty-key floor every table shares.
    ///
    /// The answer is a sentence a user reads, not a bool, because a row that is dropped
    /// without a reason is a row whose author cannot fix it.
    fn why_unusable(&self) -> Option<&'static str> {
        None
    }

    /// Whether this row applies to a machine running `os`.
    ///
    /// Takes the OS rather than reading it, so the filter itself is testable on every
    /// platform — four of the five copies of this read `std::env::consts::OS` directly, which
    /// meant the Windows arm of every table was only ever exercised on Windows.
    fn applies_to(&self, os: &str) -> bool {
        match self.only_on() {
            Some(want) => want.eq_ignore_ascii_case(os),
            None => true,
        }
    }

    /// Whether this row applies to the machine Shall is running on.
    fn applies_here(&self) -> bool {
        self.applies_to(std::env::consts::OS)
    }

    /// Why Shall will not act on this row, or `None`.
    fn unusable(&self) -> Option<&'static str> {
        if self.name().trim().is_empty() {
            return Some(Self::NAMELESS);
        }
        self.why_unusable()
    }
}

/// A row whose table picks one to drive by looking at the machine.
pub trait Detected: AdapterRow {
    /// The command whose presence on `PATH` means this machine runs this thing.
    fn detect_command(&self) -> &str;

    /// A path whose existence means the thing is not merely INSTALLED but **running**.
    ///
    /// **`command_exists` answers the wrong question for anything with a daemon,** and the
    /// ubuntu container measured it: `systemctl` is on `PATH` in every Debian image, systemd is
    /// not PID 1 in any of them, so `detect_init` picked systemd and every `service:` line
    /// failed with *"System has not been booted with systemd as init system (PID 1). Can't
    /// operate. Failed to connect to bus: Host is down"* — on a host that also had `service` and
    /// `update-rc.d` sitting there, ready to do the job.
    ///
    /// That is not a container quirk. It is every machine where a daemon's client is installed
    /// and its daemon is not running: a chroot, WSL1, a Debian box on sysvinit with
    /// systemd-shim, a host whose firewalld is stopped.
    ///
    /// `None` means the command's presence IS the whole test, which is right for a row that
    /// drives something with no daemon behind it. A row that names a file must name one the
    /// tool's own documentation vouches for — `/run/systemd/system` is what `sd_booted(3)`
    /// checks, and guessing a path here would replace a wrong answer with an unfalsifiable one.
    fn detect_file(&self) -> Option<&str> {
        None
    }
}

/// The rows Shall will act on, in the order given, each dropped row saying why.
///
/// **Dropped loudly, never half-trusted.** A row Shall cannot drive is not a row it drives
/// badly: a settings store it can write but not read would be rewritten on every sync, and an
/// init it can start but not stop would half-apply a `service:` line. The reason is printed
/// because the person who can fix the row is reading.
pub fn usable<R: AdapterRow>(rows: impl IntoIterator<Item = R>) -> Vec<R> {
    rows.into_iter()
        .filter(|row| match row.unusable() {
            Some(why) => {
                tracing::warn!("ignoring the `{}` {}: {}.", row.name(), R::WHAT, why);
                false
            }
            None => true,
        })
        .collect()
}

/// The rows in force, with the first claim on a name winning.
///
/// **Chain the shipped rows first and a user's row can never shadow one Shall ships** — the
/// `custom_backends.toml` rule (K17/U1), applied to every table that has built-ins. Both
/// halves go through this one function on purpose: an adapter mechanism the built-ins bypass
/// is one nobody has tested.
///
/// Not every table wants this. `[[bootstrap]]` and `[[prereq]]` are keyed by *manager* and
/// carry several rows for one — `asdf` needs a plugin per declared tool — so they use
/// [`usable`] and pick with their own rule. Silently keeping a duplicate is what three of the
/// seven did.
pub fn merge<R: AdapterRow>(rows: impl IntoIterator<Item = R>) -> Vec<R> {
    let mut out: Vec<R> = Vec::new();
    for row in usable(rows) {
        if out
            .iter()
            .any(|k| k.name().eq_ignore_ascii_case(row.name()))
        {
            tracing::warn!("ignoring a second {} named `{}`.", R::WHAT, row.name());
            continue;
        }
        out.push(row);
    }
    out
}

/// The first row that describes this machine: it applies to this OS, its command is on `PATH`,
/// and — where the row names one — its liveness file is there.
///
/// Both predicates are injected rather than probed here so the choice is testable without a
/// machine that has ufw on it, and without one that has booted systemd.
pub fn first_present<'a, R: Detected>(
    rows: &'a [R],
    present: &dyn Fn(&str) -> bool,
    exists: &dyn Fn(&str) -> bool,
) -> Option<&'a R> {
    first_present_on(rows, std::env::consts::OS, present, exists)
}

/// [`first_present`] against a named OS, so the choice is testable on every platform.
///
/// The same move [`AdapterRow::applies_to`] makes, and for the same reason this module's header
/// records: a search that reads `std::env::consts::OS` itself can only ever be exercised for the
/// platform the test runs on — which is how the Windows arm of five tables went unexercised
/// everywhere but Windows. The systemd-installed-but-not-booted case is a *Linux* question, and
/// this repository's suite runs on Windows.
pub fn first_present_on<'a, R: Detected>(
    rows: &'a [R],
    os: &str,
    present: &dyn Fn(&str) -> bool,
    exists: &dyn Fn(&str) -> bool,
) -> Option<&'a R> {
    rows.iter().find(|r| {
        r.applies_to(os)
            && present(r.detect_command())
            // A row with no liveness file is unchanged: its command being there is the answer.
            && r.detect_file().is_none_or(exists)
    })
}

/// Fill an argv template's `{placeholder}`s, left to right.
///
/// Left to right and not simultaneously, because that is what all five hand-written copies
/// did: a chain of `.replace()`. Substituting a value that itself looks like a placeholder is
/// therefore possible and is unchanged behaviour — a table that cared would have to say so.
///
/// A placeholder a row does not use costs nothing, which is why callers pass the whole
/// vocabulary rather than branching: `{policy}` is filled with `""` on the rows that open a
/// port, and those rows do not mention it.
pub fn fill(args: &[String], subs: &[(&str, &str)]) -> Vec<String> {
    args.iter()
        .map(|a| {
            subs.iter()
                .fold(a.clone(), |acc, (key, value)| acc.replace(key, value))
        })
        .collect()
}

/// Split a filled template into the program and its arguments.
///
/// A row with an empty command is refused by [`AdapterRow::unusable`] before it can get here,
/// so this returns `None` rather than inventing a program name.
pub fn program_and_args(filled: Vec<String>) -> Option<(String, Vec<String>)> {
    let (program, rest) = filled.split_first()?;
    Some((program.clone(), rest.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Row {
        name: String,
        os: Option<String>,
        broken: Option<&'static str>,
        live: Option<String>,
    }

    impl AdapterRow for Row {
        const WHAT: &'static str = "test adapter";
        fn name(&self) -> &str {
            &self.name
        }
        fn only_on(&self) -> Option<&str> {
            self.os.as_deref()
        }
        fn why_unusable(&self) -> Option<&'static str> {
            self.broken
        }
    }

    impl Detected for Row {
        fn detect_command(&self) -> &str {
            &self.name
        }
        fn detect_file(&self) -> Option<&str> {
            self.live.as_deref()
        }
    }

    fn row(name: &str, os: Option<&str>) -> Row {
        Row {
            name: name.into(),
            os: os.map(str::to_string),
            broken: None,
            live: None,
        }
    }

    /// The question every table asked separately, asked once — and asked for a platform this
    /// test is not running on, which is what taking the OS as a parameter buys.
    #[test]
    fn a_row_naming_an_os_applies_only_there() {
        assert!(row("ufw", None).applies_to("linux"));
        assert!(row("ufw", None).applies_to("windows"));
        assert!(row("ufw", Some("linux")).applies_to("linux"));
        assert!(!row("ufw", Some("linux")).applies_to("windows"));
        // Case-insensitive: the field is user-written.
        assert!(row("ufw", Some("Linux")).applies_to("linux"));
    }

    #[test]
    fn the_floor_drops_a_nameless_row_and_says_which_field() {
        assert_eq!(row("", None).unusable(), Some("it has no `name`"));
        assert_eq!(row("   ", None).unusable(), Some("it has no `name`"));
        assert_eq!(row("ufw", None).unusable(), None);

        let broken = Row {
            name: "ufw".into(),
            os: None,
            broken: Some("it cannot both open and close a port"),
            live: None,
        };
        assert_eq!(
            broken.unusable(),
            Some("it cannot both open and close a port"),
            "a row's own reason must survive the shared floor"
        );
    }

    /// The rule three of the seven tables had and three did not.
    #[test]
    fn the_first_claim_on_a_name_wins_and_a_second_is_dropped() {
        let merged = merge(vec![
            row("ufw", None),
            row("firewalld", None),
            row("UFW", None),
        ]);
        let names: Vec<&str> = merged.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["ufw", "firewalld"],
            "a shipped row is chained first, so a user's row claiming its name is the one \
             dropped — and the comparison is case-insensitive or `UFW` would shadow `ufw`"
        );
    }

    #[test]
    fn an_unusable_row_never_reaches_the_merged_table() {
        let merged = merge(vec![
            Row {
                name: "broken".into(),
                os: None,
                broken: Some("it cannot list its rules"),
                live: None,
            },
            row("ufw", None),
        ]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].name, "ufw");
    }

    /// `usable` keeps duplicates on purpose — `[[prereq]]` carries several rows for one
    /// manager. Stated as a test so folding it into `merge` fails rather than quietly
    /// dropping a machine's second setup step.
    #[test]
    fn usable_keeps_two_rows_that_share_a_name() {
        let kept = usable(vec![row("asdf", None), row("asdf", None)]);
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn the_first_row_this_machine_has_is_the_one_chosen() {
        let rows = vec![row("ufw", None), row("firewalld", None)];
        assert_eq!(
            first_present(&rows, &|c| c == "firewalld", &|_| true).map(|r| r.name.as_str()),
            Some("firewalld"),
            "a row whose detect command is absent is skipped, not chosen and then failed on"
        );
        assert!(first_present(&rows, &|_| false, &|_| true).is_none());

        // The OS filter is part of the search, not a separate step a caller can forget.
        let elsewhere = vec![row("netsh", Some("definitely-not-this-os"))];
        assert!(
            first_present(&elsewhere, &|_| true, &|_| true).is_none(),
            "a row for another platform is not chosen even when its command is on PATH"
        );
    }

    #[test]
    fn placeholders_are_filled_left_to_right() {
        let template: Vec<String> = ["ufw", "allow", "{port}/{proto}"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            fill(&template, &[("{port}", "22"), ("{proto}", "tcp")]),
            vec!["ufw", "allow", "22/tcp"]
        );

        // A placeholder the row does not mention costs nothing — every caller passes the whole
        // vocabulary rather than branching per row.
        assert_eq!(
            fill(
                &template,
                &[("{policy}", "deny"), ("{port}", "22"), ("{proto}", "udp")]
            ),
            vec!["ufw", "allow", "22/udp"]
        );
    }

    #[test]
    fn a_filled_template_splits_into_a_program_and_its_arguments() {
        let filled: Vec<String> = ["reg", "query", "HKCU\\x"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (program, args) = program_and_args(filled).expect("a non-empty template splits");
        assert_eq!(program, "reg");
        assert_eq!(args, vec!["query", "HKCU\\x"]);
        assert!(
            program_and_args(Vec::new()).is_none(),
            "an empty command yields nothing rather than an invented program name"
        );
    }
}
