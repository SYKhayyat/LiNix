use crate::config::config::ScheduleConfig;
use crate::core::{CommandExecutor, Error, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Delete a unit file Shall generated. A file that is already gone is the wanted end state;
/// any other failure leaves a schedule armed that Shall is about to report as removed.
fn remove_generated(path: &Path) -> Result<()> {
    crate::utils::file::force_remove(path).map_err(|e| {
        Error::Io(format!(
            "{e}. The schedule may still be armed; remove the file by hand and re-run \
             `shall sync`."
        ))
    })
}

/// What a scheduler holds for one task — and, from [`TaskProvisioner::rendered`], what Shall
/// would put there for a declaration.
///
/// `spec` is only ever compared against another `spec` from the **same** provisioner. A systemd
/// unit file and a Task Scheduler trigger have nothing to say to each other, and the value of
/// the comparison is that each provisioner gets to make it in its own terms: systemd and launchd
/// compare the whole generated file, so every option they can express is covered without anyone
/// remembering to add it here, and Task Scheduler — which keeps no file — compares a canonical
/// form built from the trigger XML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provisioned {
    /// The scheduler's whole record of the task, in that scheduler's own spelling.
    pub spec: String,
    /// Whether it fires. A task that exists and is disabled is not the schedule that was
    /// declared, and nothing that only asks "does it exist" can tell the difference.
    pub armed: bool,
}

/// The answer to *what does this machine hold for this schedule*.
///
/// **`Unreadable` is not `Absent`.** A read that failed reported as "not there" would re-write
/// the schedule on every sync for ever and keep `check` permanently red on something nobody can
/// see — the rule V.188 states for `setting:`, which applies here for the same reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    /// The scheduler answered, and this is what it holds.
    Holds(Provisioned),
    /// The scheduler answered, and it holds nothing under that name.
    Absent,
    /// The scheduler could not be asked.
    Unreadable,
}

/// How the machine stands against one `schedule:` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    /// The scheduler holds exactly what the declaration says.
    Matches,
    /// Nothing is provisioned under that name.
    Missing,
    /// Something is provisioned and it is not this. The string says what changed, for the line
    /// `sync` prints before it re-provisions.
    Differs(&'static str),
    /// It could not be asked, so nothing is claimed either way.
    Unknown,
}

impl Standing {
    /// The three-valued answer `apply::extras::in_effect` speaks: `Some(true)` in effect,
    /// `Some(false)` work to do, `None` unverifiable.
    pub fn in_effect(&self) -> Option<bool> {
        match self {
            Standing::Matches => Some(true),
            Standing::Missing | Standing::Differs(_) => Some(false),
            Standing::Unknown => None,
        }
    }
}

/// The separator between the two systemd units in one `spec`. A `#` line is a comment in unit
/// syntax, so a spec that ever did reach a file would still parse.
const UNIT_SEPARATOR: &str = "\n#--- shall: timer ---\n";

#[async_trait]
pub trait TaskProvisioner: Send + Sync {
    async fn add_task(
        &self,
        executor: &CommandExecutor,
        config: &ScheduleConfig,
        shall_path: &Path,
    ) -> Result<()>;
    async fn remove_task(&self, executor: &CommandExecutor, name: &str) -> Result<()>;
    async fn is_task_active(&self, executor: &CommandExecutor, name: &str) -> bool;

    /// Refuse, by name, an option this scheduler cannot express.
    ///
    /// The alternative is accepting the option and dropping it, which is the same failure as a
    /// cron silently widened into DAILY: the declaration says one thing, the machine does
    /// another, and both report success. `jitter` dropped on 500 machines is 500 machines
    /// hitting the mirror on the same second.
    fn refuse_unsupported(&self, config: &ScheduleConfig) -> std::result::Result<(), String>;

    /// What Shall would provision for `config`, in this scheduler's terms.
    fn rendered(
        &self,
        config: &ScheduleConfig,
        shall_bin: &Path,
    ) -> std::result::Result<Provisioned, String>;

    /// What this scheduler holds for `name` right now.
    async fn read_task(&self, executor: &CommandExecutor, name: &str) -> Reading;
}

pub struct SchedulerManager {
    provisioner: Box<dyn TaskProvisioner>,
    shall_bin_path: PathBuf,
}

impl SchedulerManager {
    pub fn new() -> Result<Self> {
        debug!("Detecting system-native task runner.");

        let shall_bin_path = std::env::current_exe()
            .map_err(|e| Error::Io(format!("Failed to locate current Shall binary: {}", e)))?;

        let provisioner: Box<dyn TaskProvisioner> = if cfg!(target_os = "linux") {
            Box::new(LinuxSystemdProvisioner)
        } else if cfg!(target_os = "macos") {
            Box::new(MacLaunchdProvisioner)
        } else if cfg!(target_os = "windows") {
            Box::new(WindowsTaskProvisioner)
        } else {
            return Err(Error::UnsupportedPlatform(
                "Native scheduling is not supported on this OS variant.".into(),
            ));
        };

        Ok(Self {
            provisioner,
            shall_bin_path,
        })
    }

    /// Register `cfg` with the OS scheduler (systemd/launchd/Task Scheduler) — the declarative
    /// path (S21). Unlike [`add_schedule`], it does NOT write to `preferences.toml`: a `schedule:`
    /// declared in the model lives in the `schedules` file, and `sync` provisions it from
    /// there on every run. Idempotent by nature — `add_task` re-registers the same task.
    pub async fn provision(&self, executor: &CommandExecutor, cfg: &ScheduleConfig) -> Result<()> {
        Self::validate_cron(&cfg.name, &cfg.cron)?;
        // Before anything is written, and for every provisioner, because an option accepted and
        // dropped is a declaration the machine does not keep.
        self.provisioner
            .refuse_unsupported(cfg)
            .map_err(|e| Error::Refused(format!("`schedule:{}`: {}", cfg.name, e)))?;
        self.provisioner
            .add_task(executor, cfg, &self.shall_bin_path)
            .await
    }

    /// Remove a task from the OS scheduler by name, without touching preferences.toml — the undo
    /// side of [`provision`], used when a `schedule:` line is deleted (S20 drift).
    /// `reaped` is proof the removal guard was consulted — see
    /// [`Reaped`](crate::app::sync::guard::Reaped).
    pub async fn deprovision(
        &self,
        executor: &CommandExecutor,
        name: &str,
        _reaped: crate::app::sync::guard::Reaped,
    ) -> Result<()> {
        self.provisioner.remove_task(executor, name).await
    }

    /// How this machine stands against one declaration — the read-back half of `schedule:`.
    ///
    /// Provisioning has always been idempotent, so the machine converged whatever the reporting
    /// said; what it could not do was *say* it had changed anything. `@cron=` and `@run=` are
    /// not in the ledger key, so an edited schedule is the same key, is found in the applied
    /// ledger, and was reported as nothing to do while `sync` rewrote it underneath — `J2`'s
    /// defect on a different kind.
    ///
    /// **The key is deliberately not widened to carry them.** A `setting:`'s scope makes two
    /// different subjects, so `J2`'s fix worked there; a schedule's name *is* its identity at the
    /// OS scheduler, so `schedule:nightly@cron=old` and `schedule:nightly@cron=new` are one cron
    /// entry — `reconcile` would deprovision by name the entry the apply phase had just written,
    /// and editing a schedule would silently delete it. Reading the machine answers the question
    /// the key cannot.
    pub async fn standing(&self, executor: &CommandExecutor, cfg: &ScheduleConfig) -> Standing {
        // A declaration this scheduler will refuse is not in effect and never will be. Reported
        // as work rather than as unverifiable, so `check` is red about it and the refusal that
        // follows names the option.
        if self.provisioner.refuse_unsupported(cfg).is_err() {
            return Standing::Missing;
        }
        let Ok(want) = self.provisioner.rendered(cfg, &self.shall_bin_path) else {
            return Standing::Missing;
        };
        match self.provisioner.read_task(executor, &cfg.name).await {
            Reading::Unreadable => Standing::Unknown,
            Reading::Absent => Standing::Missing,
            Reading::Holds(got) => {
                if got.spec != want.spec {
                    Standing::Differs("what it runs, or when it runs, is not what is declared")
                } else if got.armed != want.armed {
                    Standing::Differs(if want.armed {
                        "it is registered but will not fire"
                    } else {
                        "it is registered and armed, and the declaration says it should not fire"
                    })
                } else {
                    Standing::Matches
                }
            }
        }
    }

    /// Reject an invalid cron before it reaches the OS scheduler. One implementation, in the
    /// model, so a bad cron is the same error whether it came from a file or a flag.
    fn validate_cron(name: &str, cron: &str) -> Result<()> {
        crate::model::schedule::validate_cron(cron).map_err(|e| {
            Error::Validation(format!(
                "Invalid cron syntax for task '{}': {}. Rejection issued.",
                name, e
            ))
        })
    }
}

/// Is this schedule meant to fire? An option nobody wrote leaves the schedule armed, which is
/// what every schedule did before the option existed.
fn armed(config: &ScheduleConfig) -> bool {
    config.enabled.unwrap_or(true)
}

struct LinuxSystemdProvisioner;

impl LinuxSystemdProvisioner {
    fn unit_stem(name: &str) -> String {
        format!("shall-{}", name)
    }

    fn user_unit_dir() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("systemd").join("user"))
    }

    /// The `.service` unit, for both shapes a schedule can take.
    ///
    /// One renderer, because there were two and they disagreed: the boot shape was written by
    /// overwriting the file the ordinary shape had just written, and the replacement dropped
    /// `StandardOutput=`/`StandardError=` — so an `@reboot` job's output went nowhere while
    /// every other job's was appended to `schedule.log`.
    fn service_unit(config: &ScheduleConfig, shall_bin: &Path) -> String {
        let log = crate::utils::safe_data_dir().join("schedule.log");
        let mut unit = format!(
            "[Unit]\nDescription=Shall {kind}: {name}\n\n\
             [Service]\nType=oneshot\nExecStart={bin} {cmd}\n\
             StandardOutput=append:{log}\nStandardError=append:{log}\n",
            kind = if config.cron == "@reboot" {
                "Reboot Job"
            } else {
                "Job"
            },
            name = config.name,
            bin = shall_bin.display(),
            cmd = config.command,
            log = log.display(),
        );
        if config.cron == "@reboot" {
            unit.push_str("\n[Install]\nWantedBy=default.target\n");
        }
        unit
    }

    /// The `.timer` unit. `@reboot` has none — it is an `[Install]` on the service.
    fn timer_unit(&self, config: &ScheduleConfig) -> String {
        let mut unit = format!(
            "[Unit]\nDescription=Shall Schedule Timer for {name}\n\n\
             [Timer]\nOnCalendar={calendar}\nPersistent={persistent}\n",
            name = config.name,
            calendar = self.map_cron_to_systemd(&config.cron),
            // Undeclared is `true`, which is what the timer always said before the option
            // existed, so adding it moved nobody's schedule.
            persistent = config.persistent.unwrap_or(true),
        );
        if let Some(seconds) = config.jitter {
            unit.push_str(&format!("RandomizedDelaySec={}\n", seconds));
        }
        unit.push_str("\n[Install]\nWantedBy=timers.target\n");
        unit
    }

    async fn systemctl_says(
        &self,
        executor: &CommandExecutor,
        verb: &str,
        unit: &str,
        expect: &str,
    ) -> bool {
        match executor
            .run("systemctl", &["--no-pager", "--user", verb, unit], false)
            .await
        {
            Ok(out) => {
                crate::utils::text::sanitize(&String::from_utf8_lossy(&out.stdout)) == expect
            }
            Err(_) => false,
        }
    }

    fn map_cron_to_systemd(&self, cron: &str) -> String {
        match cron {
            "@hourly" => "hourly".into(),
            "@daily" => "daily".into(),
            "@weekly" => "weekly".into(),
            "@monthly" => "monthly".into(),
            "@yearly" | "@annually" => "yearly".into(),
            other => {
                let parts: Vec<&str> = other.split_whitespace().collect();
                if parts.len() < 5 {
                    return "daily".into();
                }

                // systemd OnCalendar = [DOW ]YYYY-MM-DD HH:MM:SS with `*` wildcards and
                // zero-padded time. Standard cron order is min hour dom month dow.
                let min = self.pad2(&self.translate_field(parts[0]));
                let hour = self.pad2(&self.translate_field(parts[1]));
                let dom = self.translate_field(parts[2]);
                let mon = self.translate_field(parts[3]);
                let dow = self.translate_field(parts[4]);

                let date = format!("*-{}-{}", mon, dom);
                let time = format!("{}:{}:00", hour, min);

                if dow == "*" {
                    format!("{} {}", date, time)
                } else {
                    let dow_mapped = dow
                        .replace('0', "Sun")
                        .replace('1', "Mon")
                        .replace('2', "Tue")
                        .replace('3', "Wed")
                        .replace('4', "Thu")
                        .replace('5', "Fri")
                        .replace('6', "Sat");
                    format!("{} {} {}", dow_mapped, date, time)
                }
            }
        }
    }

    /// Zero-pad a single-digit numeric field to two digits (systemd time); leave
    /// wildcards / ranges / step expressions untouched.
    fn pad2(&self, s: &str) -> String {
        if s.len() == 1 && s.chars().all(|c| c.is_ascii_digit()) {
            format!("0{}", s)
        } else {
            s.to_string()
        }
    }

    fn translate_field(&self, field: &str) -> String {
        if field == "*" {
            return "*".into();
        }
        if let Some(step) = field.strip_prefix("*/") {
            return format!("0/{}", step);
        }
        if field.contains('-') {
            return field.replace('-', "..");
        }
        field.to_string()
    }
}

#[async_trait]
impl TaskProvisioner for LinuxSystemdProvisioner {
    fn refuse_unsupported(&self, config: &ScheduleConfig) -> std::result::Result<(), String> {
        // A user timer runs as the user who owns the session, and there is no `--user` unit that
        // can raise that. Making one would mean writing into /etc/systemd/system as root, which
        // is a different schedule with a different lifetime and a different removal.
        if config.elevated == Some(true) {
            return Err(
                "`elevated = true` needs a system unit, and Shall's schedules are user \
                        timers — a `--user` unit runs as you and cannot raise itself. Run the \
                        privileged part from the command the schedule invokes."
                    .into(),
            );
        }
        // `jitter` on a `@reboot` job has nowhere to live: the delay is a `[Timer]` setting and
        // a boot job is an `[Install]` on the service.
        if config.jitter.is_some() && config.cron == "@reboot" {
            return Err(
                "`jitter` is a timer setting and `@reboot` has no timer — it is an \
                        install on the service. Delay the work inside the command instead."
                    .into(),
            );
        }
        if config.persistent.is_some() && config.cron == "@reboot" {
            return Err(
                "`persistent` asks a timer to catch up a missed firing, and `@reboot` \
                        has no timer — it fires when the machine comes up, which is the same \
                        thing."
                    .into(),
            );
        }
        Ok(())
    }

    fn rendered(
        &self,
        config: &ScheduleConfig,
        shall_bin: &Path,
    ) -> std::result::Result<Provisioned, String> {
        let mut spec = Self::service_unit(config, shall_bin);
        if config.cron != "@reboot" {
            spec.push_str(UNIT_SEPARATOR);
            spec.push_str(&self.timer_unit(config));
        }
        Ok(Provisioned {
            spec,
            armed: armed(config),
        })
    }

    async fn read_task(&self, executor: &CommandExecutor, name: &str) -> Reading {
        let Some(dir) = Self::user_unit_dir() else {
            return Reading::Unreadable;
        };
        let stem = Self::unit_stem(name);
        let mut spec = match std::fs::read_to_string(dir.join(format!("{}.service", stem))) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Reading::Absent,
            // A unit directory this account cannot read is not an empty one.
            Err(_) => return Reading::Unreadable,
        };
        match std::fs::read_to_string(dir.join(format!("{}.timer", stem))) {
            Ok(text) => {
                spec.push_str(UNIT_SEPARATOR);
                spec.push_str(&text);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Reading::Unreadable,
        }
        Reading::Holds(Provisioned {
            spec,
            armed: self.is_task_active(executor, name).await,
        })
    }

    async fn add_task(
        &self,
        executor: &CommandExecutor,
        config: &ScheduleConfig,
        shall_bin: &Path,
    ) -> Result<()> {
        let systemd_dir = Self::user_unit_dir()
            .ok_or_else(|| Error::Io("User configuration directory not found".into()))?;

        crate::utils::file::ensure_dir(&systemd_dir)?;

        let stem = Self::unit_stem(&config.name);
        let service_unit = format!("{}.service", stem);
        let timer_unit = format!("{}.timer", stem);
        let boot_job = config.cron == "@reboot";
        let arm = armed(config);

        executor
            .write_atomic(
                &systemd_dir.join(&service_unit),
                &Self::service_unit(config, shall_bin),
            )
            .await?;
        if !boot_job {
            executor
                .write_atomic(&systemd_dir.join(&timer_unit), &self.timer_unit(config))
                .await?;
        }

        executor
            .run(
                "systemctl",
                &["--no-pager", "--user", "daemon-reload"],
                false,
            )
            .await?;

        // The unit that carries the schedule: the timer, or the service itself for a boot job.
        let carrier = if boot_job { &service_unit } else { &timer_unit };
        if arm {
            let mut args = vec!["--no-pager", "--user", "enable"];
            // `--now` starts a timer; there is nothing to start about a boot job, and asking
            // systemd to start a oneshot service here would run the command immediately.
            if !boot_job {
                args.push("--now");
            }
            args.push(carrier);
            executor.run("systemctl", &args, false).await?;
        } else {
            // Declared and deliberately silent. `disable --now` rather than "do not enable",
            // because the previous sync may have armed it and a declaration that changed has to
            // reach the machine.
            let _ = executor
                .run(
                    "systemctl",
                    &["--no-pager", "--user", "disable", "--now", carrier],
                    false,
                )
                .await;
            if self.is_task_active(executor, &config.name).await {
                return Err(Error::Io(format!(
                    "`schedule:{}` is declared `enabled = false` and its systemd unit is still \
                     active. Check `systemctl --user status {}`.",
                    config.name, carrier
                )));
            }
        }

        Ok(())
    }

    async fn remove_task(&self, executor: &CommandExecutor, name: &str) -> Result<()> {
        let stem = Self::unit_stem(name);
        let timer_name = format!("{}.timer", stem);
        let service_name = format!("{}.service", stem);

        let _ = executor
            .run(
                "systemctl",
                &["--no-pager", "--user", "disable", "--now", &timer_name],
                false,
            )
            .await;
        let _ = executor
            .run(
                "systemctl",
                &["--no-pager", "--user", "disable", "--now", &service_name],
                false,
            )
            .await;

        if let Some(systemd_dir) = Self::user_unit_dir() {
            remove_generated(&systemd_dir.join(&timer_name))?;
            remove_generated(&systemd_dir.join(&service_name))?;
        }
        // `disable` is allowed to fail — a unit that was never enabled reports failure — so
        // the end state is what gets asserted. A timer still running after this is a schedule
        // Shall would otherwise report as removed while it keeps firing.
        if self.is_task_active(executor, name).await {
            return Err(Error::Io(format!(
                "the systemd timer for `{}` is still active after removal. Check \
                 `systemctl --user status {}`.",
                name, timer_name
            )));
        }
        Ok(())
    }

    /// Does this schedule still fire?
    ///
    /// **Both shapes, because only one was asked about.** A timer job is `is-active` on the
    /// timer; a `@reboot` job has no timer at all, so the question is whether the service is
    /// enabled — and asking only the first meant the end-state assertion in `remove_task` was
    /// vacuous for every boot job, which is precisely the case where a surviving unit runs the
    /// command again on the next boot.
    async fn is_task_active(&self, executor: &CommandExecutor, name: &str) -> bool {
        let stem = Self::unit_stem(name);
        self.systemctl_says(executor, "is-active", &format!("{}.timer", stem), "active")
            .await
            || self
                .systemctl_says(
                    executor,
                    "is-enabled",
                    &format!("{}.service", stem),
                    "enabled",
                )
                .await
    }
}

struct MacLaunchdProvisioner;

impl MacLaunchdProvisioner {
    fn label(name: &str) -> String {
        format!("com.shall.{}", name)
    }

    fn plist_path(name: &str) -> Option<PathBuf> {
        Some(
            dirs::home_dir()?
                .join("Library/LaunchAgents")
                .join(format!("{}.plist", Self::label(name))),
        )
    }

    fn plist(&self, config: &ScheduleConfig, shall_bin: &Path) -> String {
        let label = Self::label(&config.name);
        let schedule_xml = if config.cron == "@reboot" {
            "<key>RunAtLoad</key><true/>".to_string()
        } else {
            format!(
                "<key>StartCalendarInterval</key>{}",
                self.map_cron_to_launchd_xml(&config.cron)
            )
        };
        // launchd's own switch for a job that is present and must not fire. Written into the
        // file rather than left to `launchctl unload`, so a reboot does not quietly arm it.
        let disabled = if armed(config) {
            ""
        } else {
            "<key>Disabled</key><true/>\n"
        };

        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
            <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
            <plist version=\"1.0\">\n<dict>\n\
            <key>Label</key><string>{label}</string>\n\
            <key>ProgramArguments</key>\n<array>\n\
            <string>{bin}</string><string>{cmd}</string>\n</array>\n\
            {schedule}\n\
            {disabled}\
            <key>StandardOutPath</key><string>{log}</string>\n\
            <key>StandardErrorPath</key><string>{log}</string>\n\
            </dict>\n</plist>",
            label = label,
            bin = shall_bin.display(),
            cmd = config.command,
            schedule = schedule_xml,
            disabled = disabled,
            log = crate::utils::safe_data_dir().join("schedule.log").display()
        )
    }

    fn map_cron_to_launchd_xml(&self, cron: &str) -> String {
        let parts: Vec<&str> = cron.split_whitespace().collect();
        let (m, h, dom, mon, dow) = match cron {
            "@hourly" => ("0", "*", "*", "*", "*"),
            "@daily" => ("0", "0", "*", "*", "*"),
            "@weekly" => ("0", "0", "*", "*", "1"),
            "@monthly" => ("0", "0", "1", "*", "*"),
            _ if parts.len() >= 5 => (parts[0], parts[1], parts[2], parts[3], parts[4]),
            _ => ("0", "2", "*", "*", "*"),
        };

        let mut xml = String::from("<dict>");
        let keys = ["Minute", "Hour", "Day", "Month", "Weekday"];
        let vals = [m, h, dom, mon, dow];

        for (i, &val) in vals.iter().enumerate() {
            if val != "*" {
                let first_val = val.split([',', '-', '/']).next().unwrap_or("0");
                if let Ok(num) = first_val.parse::<u32>() {
                    xml.push_str(&format!("<key>{}</key><integer>{}</integer>", keys[i], num));
                }
            }
        }
        xml.push_str("</dict>");
        xml
    }
}

#[async_trait]
impl TaskProvisioner for MacLaunchdProvisioner {
    fn refuse_unsupported(&self, config: &ScheduleConfig) -> std::result::Result<(), String> {
        if config.jitter.is_some() {
            return Err(
                "`jitter` has no launchd equivalent — a `StartCalendarInterval` fires on \
                        the minute it names and nothing spreads it. Randomise inside the command \
                        the schedule invokes, or drop the option."
                    .into(),
            );
        }
        // `@reboot` is `RunAtLoad`, which fires when the agent loads and has no calendar to
        // miss — the same reason systemd refuses it on a boot job, said in launchd's terms.
        if config.persistent.is_some() && config.cron == "@reboot" {
            return Err(
                "`persistent` asks a calendar job to catch up a firing it slept through, \
                        and `@reboot` is `RunAtLoad` — it fires when the agent loads, which is \
                        the same thing."
                    .into(),
            );
        }
        // launchd runs a missed calendar job when the machine wakes, and offers no switch for
        // it. `true` is therefore what happens; `false` is a promise nothing here can keep.
        if config.persistent == Some(false) {
            return Err(
                "`persistent = false` has no launchd equivalent — a calendar job whose \
                        time passed while the machine was asleep runs on wake, and launchd \
                        offers no way to decline that."
                    .into(),
            );
        }
        if config.elevated == Some(true) {
            return Err(
                "`elevated = true` needs a LaunchDaemon, and Shall's schedules are \
                        LaunchAgents — an agent runs as you and cannot raise itself. Run the \
                        privileged part from the command the schedule invokes."
                    .into(),
            );
        }
        Ok(())
    }

    fn rendered(
        &self,
        config: &ScheduleConfig,
        shall_bin: &Path,
    ) -> std::result::Result<Provisioned, String> {
        Ok(Provisioned {
            spec: self.plist(config, shall_bin),
            armed: armed(config),
        })
    }

    async fn read_task(&self, executor: &CommandExecutor, name: &str) -> Reading {
        let Some(path) = Self::plist_path(name) else {
            return Reading::Unreadable;
        };
        match std::fs::read_to_string(&path) {
            Ok(spec) => Reading::Holds(Provisioned {
                spec,
                armed: self.is_task_active(executor, name).await,
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Reading::Absent,
            Err(_) => Reading::Unreadable,
        }
    }

    async fn add_task(
        &self,
        executor: &CommandExecutor,
        config: &ScheduleConfig,
        shall_bin: &Path,
    ) -> Result<()> {
        let plist_path = Self::plist_path(&config.name)
            .ok_or_else(|| Error::Io("Could not locate home directory".into()))?;

        executor
            .write_atomic(&plist_path, &self.plist(config, shall_bin))
            .await?;

        if armed(config) {
            executor
                .run("launchctl", &["load", &plist_path.to_string_lossy()], false)
                .await?;
            return Ok(());
        }

        // Declared silent: the `Disabled` key keeps it that way across a reboot, and an unload
        // takes it out of this session. An agent that was never loaded reports failure, so the
        // end state is what gets asserted rather than the exit code.
        let _ = executor
            .run(
                "launchctl",
                &["unload", &plist_path.to_string_lossy()],
                false,
            )
            .await;
        if self.is_task_active(executor, &config.name).await {
            return Err(Error::Io(format!(
                "`schedule:{}` is declared `enabled = false` and its launchd agent is still \
                 loaded. Check `launchctl list {}`.",
                config.name,
                Self::label(&config.name)
            )));
        }
        Ok(())
    }

    async fn remove_task(&self, executor: &CommandExecutor, name: &str) -> Result<()> {
        if let Some(plist_path) = Self::plist_path(name) {
            let _ = executor
                .run(
                    "launchctl",
                    &["unload", &plist_path.to_string_lossy()],
                    false,
                )
                .await;
            remove_generated(&plist_path)?;
        }
        // `unload` is allowed to fail (an agent that was never loaded reports failure); a job
        // still listed after this one is a schedule that keeps firing.
        if self.is_task_active(executor, name).await {
            return Err(Error::Io(format!(
                "the launchd agent for `{}` is still loaded after removal. Check \
                 `launchctl list {}`.",
                name,
                Self::label(name)
            )));
        }
        Ok(())
    }

    async fn is_task_active(&self, executor: &CommandExecutor, name: &str) -> bool {
        match executor
            .run("launchctl", &["list", &Self::label(name)], false)
            .await
        {
            Ok(o) => o.status.success(),
            Err(_) => false,
        }
    }
}

/// The five cron fields, with `@`-shorthands already expanded into them.
///
/// One expansion, shared. The shorthand table was written out once per provisioner, and the
/// Windows one simply did not have it — so `@daily` reached `split_whitespace()` as a single
/// field and came out the other side as the start time `02:@daily`. A table each is a table
/// that can be missing.
struct CronFields<'a> {
    minute: &'a str,
    hour: &'a str,
    dom: &'a str,
    month: &'a str,
    dow: &'a str,
}

/// `@weekly` is Monday here, not Sunday, because that is what the systemd and launchd mappings
/// have always done (`OnCalendar=weekly` is Mon 00:00). Matching vixie-cron instead would move
/// existing users' schedules by a day on two platforms to fix a third.
fn parse_cron(cron: &str) -> Option<CronFields<'_>> {
    let f = |minute, hour, dom, month, dow| {
        Some(CronFields {
            minute,
            hour,
            dom,
            month,
            dow,
        })
    };
    match cron.trim() {
        "@hourly" => f("0", "*", "*", "*", "*"),
        "@daily" | "@midnight" => f("0", "0", "*", "*", "*"),
        "@weekly" => f("0", "0", "*", "*", "1"),
        "@monthly" => f("0", "0", "1", "*", "*"),
        "@yearly" | "@annually" => f("0", "0", "1", "1", "*"),
        other => {
            let p: Vec<&str> = other.split_whitespace().collect();
            if p.len() < 5 {
                return None;
            }
            f(p[0], p[1], p[2], p[3], p[4])
        }
    }
}

/// `HH:mm`, which is the only start time Task Scheduler accepts.
///
/// The whole reported defect was `format!("{}:{}", hour, min)` on the raw cron fields: `0 3 * * *`
/// became `3:0` and `schtasks` answered `ERROR: Invalid starttime value.` A time is two digits
/// and two digits, always.
fn schtasks_time(hour: &str, minute: &str) -> Option<String> {
    let h: u8 = hour.parse().ok()?;
    let m: u8 = minute.parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    Some(format!("{:02}:{:02}", h, m))
}

/// Sunday-first day names, the order Task Scheduler's `<DaysOfWeek>` lists them in.
const DAY_NAMES: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];
/// The `<DaysOfWeek>` element names, in the same order.
const DAY_ELEMENTS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTH_NAMES: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];
/// The `<Months>` element names, in the same order.
const MONTH_ELEMENTS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// cron's day-of-week (0/7 = Sunday) as the day names `/D` takes, comma-separated.
fn schtasks_days(dow: &str) -> Option<String> {
    let name = |n: u8| DAY_NAMES.get((n % 7) as usize).copied();

    let mut out: Vec<&str> = Vec::new();
    for part in dow.split(',') {
        // `/D` takes a list and no ranges, so a range is expanded rather than passed through.
        if let Some((a, b)) = part.split_once('-') {
            let (a, b): (u8, u8) = (a.parse().ok()?, b.parse().ok()?);
            if a > b || b > 7 {
                return None;
            }
            for n in a..=b {
                out.push(name(n)?);
            }
        } else {
            out.push(name(part.parse().ok()?)?);
        }
    }
    if out.is_empty() {
        return None;
    }
    out.dedup();
    Some(out.join(","))
}

/// The `/SC …` arguments that make Task Scheduler do what a cron line says.
///
/// Windows has no cron, so this is a translation and not every sentence has one. Where a cron
/// cannot be expressed it is **refused by name** rather than widened into the nearest thing
/// Task Scheduler can do: the defect this replaces silently turned `0 3 * * 1` into a DAILY
/// task, which ran seven times as often as it was declared to and reported success each time.
/// A schedule that fires when it should not is worse than one that refuses to be created.
fn map_cron_to_schtasks(cron: &str) -> std::result::Result<Vec<String>, String> {
    let s = |v: &str| v.to_string();
    let cannot = || {
        format!(
            "`{}` is a schedule Windows Task Scheduler cannot express. It understands a time of \
             day, a weekday, a day of the month, or a fixed interval — but not a combination of \
             an interval with a time window. Split it into separate schedules, or use a cron \
             this machine can keep.",
            cron
        )
    };

    if cron.trim() == "@reboot" {
        return Ok(vec![s("/SC"), s("ONSTART")]);
    }

    let c = parse_cron(cron).ok_or_else(|| {
        format!(
            "`{}` is not a cron expression: it needs five fields (minute hour day month weekday) \
             or one of @reboot, @hourly, @daily, @weekly, @monthly, @yearly.",
            cron
        )
    })?;

    // Sub-hourly. Task Scheduler's MINUTE/HOURLY intervals run around the clock, so they can
    // carry no other constraint — and pretending otherwise is how a schedule fires at times
    // nobody asked for.
    let unconstrained = c.dom == "*" && c.month == "*" && c.dow == "*";
    if c.minute == "*" || c.minute.starts_with("*/") {
        if !unconstrained || c.hour != "*" {
            return Err(cannot());
        }
        let mut args = vec![s("/SC"), s("MINUTE")];
        if let Some(step) = c.minute.strip_prefix("*/") {
            step.parse::<u16>().map_err(|_| cannot())?;
            args.extend([s("/MO"), s(step)]);
        }
        return Ok(args);
    }
    if c.hour == "*" || c.hour.starts_with("*/") {
        if !unconstrained {
            return Err(cannot());
        }
        let mut args = vec![s("/SC"), s("HOURLY")];
        if let Some(step) = c.hour.strip_prefix("*/") {
            step.parse::<u16>().map_err(|_| cannot())?;
            args.extend([s("/MO"), s(step)]);
        }
        // The first run of the hour, which is what the minute field means here.
        let st = schtasks_time("0", c.minute).ok_or_else(cannot)?;
        args.extend([s("/ST"), st]);
        return Ok(args);
    }

    let st = schtasks_time(c.hour, c.minute).ok_or_else(cannot)?;

    // A weekday beats a day of the month: `/SC WEEKLY` and `/SC MONTHLY` are exclusive, and a
    // cron naming both is the one shape with no Task Scheduler equivalent.
    if c.dow != "*" {
        if c.dom != "*" {
            return Err(cannot());
        }
        let days = schtasks_days(c.dow).ok_or_else(cannot)?;
        return Ok(vec![s("/SC"), s("WEEKLY"), s("/D"), days, s("/ST"), st]);
    }

    if c.dom != "*" || c.month != "*" {
        let mut args = vec![s("/SC"), s("MONTHLY")];
        if c.month != "*" {
            let n: usize = c.month.parse().map_err(|_| cannot())?;
            let name = MONTH_NAMES.get(n.wrapping_sub(1)).ok_or_else(cannot)?;
            args.extend([s("/M"), s(name)]);
        }
        // `/SC MONTHLY` with no `/D` is the 1st, which is also what a bare month means.
        let day = if c.dom == "*" { "1" } else { c.dom };
        day.parse::<u8>()
            .ok()
            .filter(|d| (1..=31).contains(d))
            .ok_or_else(cannot)?;
        args.extend([s("/D"), s(day), s("/ST"), st]);
        return Ok(args);
    }

    Ok(vec![s("/SC"), s("DAILY"), s("/ST"), st])
}

/// The value after `flag` in a `/SC …` argument list.
fn arg_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).map(String::as_str)
}

/// Day names in Sunday-first order, so a cron that lists them in another order compares equal
/// to the same set read back out of `<DaysOfWeek>`. `5,1` and `1,5` are one schedule.
fn sorted_days(list: &str) -> String {
    let mut days: Vec<&str> = DAY_NAMES
        .iter()
        .copied()
        .filter(|d| list.split(',').any(|part| part.trim() == *d))
        .collect();
    days.dedup();
    days.join(",")
}

/// When a task fires, in one spelling both sides of the comparison can produce.
///
/// Task Scheduler keeps no file to diff, so the declaration is canonicalised from the `/SC`
/// arguments Shall would pass and the machine's copy is canonicalised from the trigger XML it
/// hands back. Anything neither can express is not canonicalised into something close — the
/// reader returns `None` and the reading is *unreadable*, which is the V.188 answer: a shape
/// Shall does not understand must never be reported as drift, or it re-writes the task on every
/// sync for ever.
fn schtasks_when(args: &[String]) -> Option<String> {
    let sc = arg_after(args, "/SC")?;
    let every = arg_after(args, "/MO").unwrap_or("1");
    let at = arg_after(args, "/ST").unwrap_or("00:00");
    Some(match sc {
        "ONSTART" => "boot".to_string(),
        "MINUTE" => format!("every {}m", every),
        "HOURLY" => format!("every {}h from {}", every, at),
        "DAILY" => format!("daily at {}", at),
        "WEEKLY" => format!(
            "weekly on {} at {}",
            sorted_days(arg_after(args, "/D")?),
            at
        ),
        "MONTHLY" => {
            let day = arg_after(args, "/D").unwrap_or("1");
            match arg_after(args, "/M") {
                Some(month) => format!("monthly in {} on {} at {}", month, day, at),
                None => format!("monthly on {} at {}", day, at),
            }
        }
        _ => return None,
    })
}

/// The text between `<tag>` and `</tag>`, first occurrence, un-escaped for the five XML
/// entities Task Scheduler emits. Deliberately not a parser: the document is machine-written,
/// the elements wanted are leaves, and a dependency for four `find` calls is a dependency.
fn xml_text<'a>(doc: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = doc.find(&open)? + open.len();
    let end = doc[start..].find(&close)? + start;
    Some(doc[start..end].trim())
}

/// The whole `<tag> … </tag>` element, angle brackets included, so a nested lookup can be
/// scoped to it. `<Settings><Enabled>` and a trigger's own `<Enabled>` are different questions,
/// and a document-wide search for the first `<Enabled>` answers whichever comes first.
fn xml_element<'a>(doc: &'a str, tag: &str) -> Option<&'a str> {
    let close = format!("</{}>", tag);
    // The opening tag may carry attributes (`<BootTrigger id="Resume">`), so it is matched on
    // its prefix and the `>` is found after it.
    let open = format!("<{}", tag);
    let at = doc.find(&open)?;
    let after_name = doc[at + open.len()..].chars().next()?;
    if after_name != '>' && !after_name.is_whitespace() {
        return None;
    }
    let end = doc[at..].find(&close)? + at + close.len();
    Some(&doc[at..end])
}

/// How many trigger elements a `<Triggers>` block opens, whatever kind they are.
///
/// Any element whose name ends in `Trigger` counts — `<Triggers>` itself does not, and neither
/// does a closing tag. Shall writes exactly one.
fn trigger_count(triggers: &str) -> usize {
    triggers
        .match_indices('<')
        .filter(|(at, _)| {
            let rest = &triggers[at + 1..];
            !rest.starts_with('/')
                && rest
                    .split(['>', ' ', '/'])
                    .next()
                    .is_some_and(|name| name.ends_with("Trigger"))
        })
        .count()
}

fn unescape_xml(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// `PT15M` / `PT6H` as the interval Shall spells `every 15m`.
///
/// Day-scale repetitions (`P1D`) deliberately have no reading: Shall writes a daily schedule as
/// a `ScheduleByDay` calendar trigger, never as a repetition, so a `P1D` under Shall's task name
/// is somebody else's task and describing it would be a guess.
fn iso_interval(spec: &str) -> Option<String> {
    let body = spec.strip_prefix('P')?;
    let time = body.strip_prefix('T')?;
    if let Some(m) = time.strip_suffix('M') {
        m.parse::<u32>().ok()?;
        return Some(format!("every {}m", m));
    }
    let h = time.strip_suffix('H')?;
    h.parse::<u32>().ok()?;
    Some(format!("every {}h", h))
}

/// `2010-12-16T03:00:00` as `03:00`.
fn boundary_time(boundary: &str) -> Option<String> {
    let time = boundary.split('T').nth(1)?;
    let (h, rest) = time.split_once(':')?;
    let m = rest.split(':').next()?;
    schtasks_time(h, m)
}

/// The canonical `when` for the single trigger in a task's XML, or `None` for any shape Shall
/// does not write and therefore cannot compare.
fn when_from_xml(doc: &str) -> Option<String> {
    let triggers = xml_element(doc, "Triggers")?;
    // Shall writes exactly one trigger. More than one means somebody else has been here, and
    // describing what they did is not something this reader can do honestly.
    //
    // **Counted by shape, not against a list of the kinds this reader understands.** A list
    // would count the calendar trigger, miss the `<EventTrigger>` beside it, and report the
    // task as matching a declaration that says nothing about the event — which is the reader
    // claiming to have read something it never saw.
    if trigger_count(triggers) != 1 {
        return None;
    }

    if triggers.contains("<BootTrigger") {
        return Some("boot".to_string());
    }

    let at = xml_text(triggers, "StartBoundary").and_then(boundary_time);

    // A repetition is the only place Task Scheduler can put a sub-daily interval, whichever
    // trigger element carries it.
    if let Some(repetition) = xml_element(triggers, "Repetition") {
        let interval = iso_interval(xml_text(repetition, "Interval")?)?;
        return Some(match interval.strip_suffix(|c: char| c == 'h') {
            Some(_) => format!("{} from {}", interval, at?),
            None => interval,
        });
    }

    if let Some(daily) = xml_element(triggers, "ScheduleByDay") {
        // Shall only ever writes a one-day interval; anything else is not its task.
        if xml_text(daily, "DaysInterval").unwrap_or("1") != "1" {
            return None;
        }
        return Some(format!("daily at {}", at?));
    }

    if let Some(weekly) = xml_element(triggers, "ScheduleByWeek") {
        if xml_text(weekly, "WeeksInterval").unwrap_or("1") != "1" {
            return None;
        }
        let days = xml_element(weekly, "DaysOfWeek")?;
        let named: Vec<&str> = DAY_ELEMENTS
            .iter()
            .enumerate()
            .filter(|(_, element)| {
                days.contains(&format!("<{} ", element))
                    || days.contains(&format!("<{}/", element))
                    || days.contains(&format!("<{}>", element))
            })
            .map(|(i, _)| DAY_NAMES[i])
            .collect();
        if named.is_empty() {
            return None;
        }
        return Some(format!("weekly on {} at {}", named.join(","), at?));
    }

    if let Some(monthly) = xml_element(triggers, "ScheduleByMonth") {
        let day = xml_text(xml_element(monthly, "DaysOfMonth")?, "Day")?.to_string();
        let month = xml_element(monthly, "Months").and_then(|months| {
            MONTH_ELEMENTS
                .iter()
                .enumerate()
                .find(|(_, element)| {
                    months.contains(&format!("<{} ", element))
                        || months.contains(&format!("<{}/", element))
                        || months.contains(&format!("<{}>", element))
                })
                .map(|(i, _)| MONTH_NAMES[i])
        });
        return Some(match month {
            Some(m) => format!("monthly in {} on {} at {}", m, day, at?),
            None => format!("monthly on {} at {}", day, at?),
        });
    }

    None
}

/// A whole `schtasks /Query /XML` document as the canonical record it describes, or `None` for a
/// document this reader cannot honestly claim to have read.
///
/// Pure, and separate from the query, so it can be asked about a **verbatim** document captured
/// from a real Task Scheduler rather than only about documents this file made up.
fn provisioned_from_xml(doc: &str) -> Option<Provisioned> {
    let exec = xml_element(doc, "Exec")?;
    let program = unescape_xml(xml_text(exec, "Command")?);
    let arguments = xml_text(exec, "Arguments")
        .map(unescape_xml)
        .unwrap_or_default();
    let command = format!("{} {}", program.trim().trim_matches('"'), arguments.trim());

    let when = when_from_xml(doc)?;
    let elevated = xml_element(doc, "Principals")
        .and_then(|p| xml_text(p, "RunLevel"))
        .is_some_and(|level| level == "HighestAvailable");
    // The task's own switch, which lives in `<Settings>`. A trigger carries an `<Enabled>` of its
    // own, so a document-wide search for the first one answers a different question. Absent
    // means armed — Task Scheduler omits the element on a task nobody has disabled, which is the
    // ordinary case and not a missing answer.
    let armed = xml_element(doc, "Settings")
        .and_then(|s| xml_text(s, "Enabled"))
        .map(|v| v == "true")
        .unwrap_or(true);

    Some(Provisioned {
        spec: schtasks_spec(&command, &when, elevated),
        armed,
    })
}

/// The canonical record of a Windows task: what it runs, when, and at which privilege.
fn schtasks_spec(command: &str, when: &str, elevated: bool) -> String {
    format!(
        "run: {}\nwhen: {}\nlevel: {}\n",
        command.trim(),
        when,
        if elevated {
            "HighestAvailable"
        } else {
            "LeastPrivilege"
        }
    )
}

/// Task Scheduler's `/XML` output, whichever width it came back in.
///
/// The document declares `encoding="UTF-16"` and arrives here as bytes; on this machine a piped
/// read gives one byte per character, and a redirect to a file gives two. Deciding from the
/// bytes rather than from the header covers both without a table of which Windows does which.
fn decode_xml(bytes: &[u8]) -> String {
    let utf16 =
        bytes.starts_with(&[0xFF, 0xFE]) || (bytes.len() >= 4 && bytes[1] == 0 && bytes[3] == 0);
    if !utf16 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let body = bytes.strip_prefix(&[0xFF, 0xFE][..]).unwrap_or(bytes);
    let units: Vec<u16> = body
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

struct WindowsTaskProvisioner;

impl WindowsTaskProvisioner {
    fn task_name(name: &str) -> String {
        format!("Shall_{}", name)
    }

    fn command_line(config: &ScheduleConfig, shall_bin: &Path) -> String {
        format!("{} {}", shall_bin.display(), config.command)
    }
}

#[async_trait]
impl TaskProvisioner for WindowsTaskProvisioner {
    fn refuse_unsupported(&self, config: &ScheduleConfig) -> std::result::Result<(), String> {
        // Both live in the task's XML and `schtasks` has no flag for either, so accepting them
        // would mean accepting an option and writing a task without it.
        if config.jitter.is_some() {
            return Err(
                "`jitter` is a Task Scheduler XML setting (`RandomDelay`) and `schtasks` \
                        has no flag for it, so Shall cannot put it on the task it creates. \
                        Randomise inside the command the schedule invokes, or drop the option."
                    .into(),
            );
        }
        if config.persistent.is_some() {
            return Err(
                "`persistent` is a Task Scheduler XML setting (`StartWhenAvailable`) and \
                        `schtasks` has no flag for it, so Shall can neither set it nor promise \
                        what it defaults to. Drop the option on this machine."
                    .into(),
            );
        }
        Ok(())
    }

    fn rendered(
        &self,
        config: &ScheduleConfig,
        shall_bin: &Path,
    ) -> std::result::Result<Provisioned, String> {
        let args = map_cron_to_schtasks(&config.cron)?;
        let when = schtasks_when(&args).ok_or_else(|| {
            format!(
                "`{}` maps to a Task Scheduler trigger Shall cannot describe.",
                config.cron
            )
        })?;
        Ok(Provisioned {
            spec: schtasks_spec(
                &Self::command_line(config, shall_bin),
                &when,
                config.elevated == Some(true),
            ),
            armed: armed(config),
        })
    }

    async fn read_task(&self, executor: &CommandExecutor, name: &str) -> Reading {
        let tn = Self::task_name(name);
        let queried = executor
            .run("schtasks", &["/Query", "/TN", &tn, "/XML"], false)
            .await;
        let doc = match queried {
            Ok(out) => decode_xml(&out.stdout),
            Err(_) => {
                // Two reasons a query fails and only one of them is an answer. Asking
                // `schtasks` a question it can always answer separates them, which the error
                // text cannot: `ERROR: The system cannot find the file specified.` is
                // translated on a non-English Windows and matching it would make the reading
                // depend on the machine's display language.
                return match executor
                    .run("schtasks", &["/Query", "/FO", "CSV", "/NH"], false)
                    .await
                {
                    Ok(_) => Reading::Absent,
                    Err(_) => Reading::Unreadable,
                };
            }
        };

        match provisioned_from_xml(&doc) {
            Some(held) => Reading::Holds(held),
            None => Reading::Unreadable,
        }
    }

    async fn add_task(
        &self,
        executor: &CommandExecutor,
        config: &ScheduleConfig,
        shall_bin: &Path,
    ) -> Result<()> {
        let name = Self::task_name(&config.name);
        // Quoted for `/TR`, which takes one string and splits it itself; the read-back compares
        // the unquoted form Task Scheduler stores.
        let cmd = format!("\"{}\" {}", shall_bin.display(), config.command);

        // Refused here, before anything is created: a schedule Task Scheduler cannot express
        // must not become the nearest one it can. `Refused` and not `Io` — Shall looked and
        // declined on purpose, which is exit code 3 (U21), and a script that retries on failure
        // must not retry this.
        let schedule = map_cron_to_schtasks(&config.cron)
            .map_err(|e| Error::Refused(format!("`schedule:{}`: {}", config.name, e)))?;

        let mut args: Vec<&str> = vec!["/Create", "/TN", &name, "/TR", &cmd, "/F"];
        args.extend(schedule.iter().map(String::as_str));
        if config.elevated == Some(true) {
            args.extend(["/RL", "HIGHEST"]);
        }

        // `ERROR: Access is denied.` is what Task Scheduler says when the shell is not
        // elevated, and on its own it names neither the cause nor the cure — it reads like a
        // permissions problem with the config. Registering a task needs an administrator here
        // whatever `/RU` says (measured), so say that instead of forwarding four words.
        executor.run("schtasks", &args, true).await.map_err(|e| {
            if e.to_string().to_lowercase().contains("access is denied") {
                Error::Permission(format!(
                    "creating the scheduled task `{}` needs an elevated shell — Windows Task \
                     Scheduler refuses to register one otherwise. Re-run `shall sync` from a \
                     terminal opened with \"Run as administrator\".",
                    name
                ))
            } else {
                e
            }
        })?;

        // `/Create` always produces an armed task, so the declaration that says otherwise is
        // applied after it rather than instead of it.
        if !armed(config) {
            executor
                .run("schtasks", &["/Change", "/TN", &name, "/DISABLE"], true)
                .await?;
        }
        Ok(())
    }

    async fn remove_task(&self, executor: &CommandExecutor, name: &str) -> Result<()> {
        let tn = Self::task_name(name);
        // `/Delete` on a task that does not exist exits non-zero, so the exit code cannot
        // tell "already gone" from "refused"; the end state can.
        let _ = executor
            .run("schtasks", &["/Delete", "/TN", &tn, "/F"], true)
            .await;
        if self.is_task_active(executor, name).await {
            return Err(Error::Io(format!(
                "the scheduled task `{}` still exists after removal. Check \
                 `schtasks /Query /TN {}`.",
                name, tn
            )));
        }
        Ok(())
    }

    async fn is_task_active(&self, executor: &CommandExecutor, name: &str) -> bool {
        let tn = Self::task_name(name);
        match executor
            .run("schtasks", &["/Query", "/TN", &tn], false)
            .await
        {
            Ok(o) => o.status.success(),
            Err(_) => false,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::config::ScheduleConfig;

    fn cfg(cron: &str) -> ScheduleConfig {
        ScheduleConfig {
            name: "nightly".into(),
            cron: cron.into(),
            command: "sync".into(),
            notification: None,
            enabled: None,
            persistent: None,
            jitter: None,
            elevated: None,
        }
    }

    #[test]
    fn a_generated_file_that_is_already_gone_is_the_wanted_end_state() {
        let dir = tempfile::tempdir().unwrap();
        assert!(remove_generated(&dir.path().join("shall-nightly.timer")).is_ok());
    }

    #[test]
    fn a_generated_file_that_exists_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let unit = dir.path().join("shall-nightly.timer");
        std::fs::write(&unit, "[Timer]\n").unwrap();
        remove_generated(&unit).unwrap();
        assert!(!unit.exists());
    }

    #[test]
    fn a_removal_that_cannot_happen_is_an_error_naming_the_file() {
        // The point is that the failure is reported at all: swallowing it left a timer armed
        // under a schedule Shall had just reported as removed. Making a path undeletable is
        // the platform-specific part; the assertion is not.
        //
        // It used to be a directory, back when the removal was `remove_file` and a directory
        // was therefore undeletable by it. `force_remove` deletes directories on purpose, so
        // that stand-in silently became a success — a test asserting an error over a call that
        // could no longer produce one.
        let dir = tempfile::tempdir().unwrap();
        let unit = dir.path().join("shall-nightly.timer");
        std::fs::write(&unit, b"[Timer]\n").unwrap();

        // Windows: an open handle with no sharing at all. `File::open` will not do — Rust's
        // default share mode includes `FILE_SHARE_DELETE`, so a plain open leaves the file
        // perfectly deletable, which is how the first attempt at this test passed nothing.
        #[cfg(windows)]
        let _held = {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&unit)
                .unwrap()
        };
        // Unix: a parent nobody may write. Root ignores the mode, so the check below can be
        // vacuous in a container running as root — Windows carries this one in CI.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        }

        let outcome = remove_generated(&unit);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
            if outcome.is_ok() {
                return; // running as root; the path was deletable after all
            }
        }
        let err = outcome.unwrap_err().to_string();
        assert!(
            err.contains("shall-nightly.timer"),
            "the error does not name the file: {}",
            err
        );
        assert!(
            err.contains("still be armed"),
            "the error does not say what is left behind: {}",
            err
        );
    }

    #[test]
    fn systemd_oncalendar_mapping() {
        let p = LinuxSystemdProvisioner;
        // every Monday 04:30 -> zero-padded time, full date wildcards, weekday name
        assert_eq!(p.map_cron_to_systemd("30 4 * * 1"), "Mon *-*-* 04:30:00");
        // daily midnight: no weekday constraint
        assert_eq!(p.map_cron_to_systemd("0 0 * * *"), "*-*-* 00:00:00");
        // @-macros pass through
        assert_eq!(p.map_cron_to_systemd("@daily"), "daily");
    }

    /// The reported defect: `0 3 * * *` produced `/ST 3:0`, and Task Scheduler answers
    /// `ERROR: Invalid starttime value.` It wants `HH:mm`, zero-padded. Measured against real
    /// `schtasks` on 2026-07-28: `/ST 3:0` is rejected at parse time, `/ST 03:00` is accepted
    /// and reaches the privilege check.
    #[test]
    fn schtasks_start_time_is_zero_padded() {
        assert_eq!(
            map_cron_to_schtasks("0 3 * * *").unwrap(),
            vec!["/SC", "DAILY", "/ST", "03:00"]
        );
    }

    /// The siblings. Every one of these went through the same two lines that dropped the
    /// padding, and each produced either a hard error or — worse — a schedule that fires more
    /// often than it was told to.
    #[test]
    fn schtasks_maps_every_shape_a_cron_can_take() {
        let cases: &[(&str, &[&str])] = &[
            // `@daily` split on whitespace to ONE field, so the minute came out as the literal
            // string "@daily" and the time was `02:@daily`.
            ("@daily", &["/SC", "DAILY", "/ST", "00:00"]),
            ("@midnight", &["/SC", "DAILY", "/ST", "00:00"]),
            ("@hourly", &["/SC", "HOURLY", "/ST", "00:00"]),
            ("@reboot", &["/SC", "ONSTART"]),
            ("@weekly", &["/SC", "WEEKLY", "/D", "MON", "/ST", "00:00"]),
            ("@monthly", &["/SC", "MONTHLY", "/D", "1", "/ST", "00:00"]),
            (
                "@yearly",
                &["/SC", "MONTHLY", "/M", "JAN", "/D", "1", "/ST", "00:00"],
            ),
            (
                "@annually",
                &["/SC", "MONTHLY", "/M", "JAN", "/D", "1", "/ST", "00:00"],
            ),
            // A step in the minute field became the time `*:*/15`.
            ("*/15 * * * *", &["/SC", "MINUTE", "/MO", "15"]),
            ("* * * * *", &["/SC", "MINUTE"]),
            // A step in the hour field became `*/6:0`.
            (
                "0 */6 * * *",
                &["/SC", "HOURLY", "/MO", "6", "/ST", "00:00"],
            ),
            ("0 * * * *", &["/SC", "HOURLY", "/ST", "00:00"]),
            // Day-of-week was ignored entirely: this ran EVERY day, seven times as often as
            // it was declared to, and reported success while doing it.
            ("0 3 * * 1", &["/SC", "WEEKLY", "/D", "MON", "/ST", "03:00"]),
            (
                "30 4 * * 0",
                &["/SC", "WEEKLY", "/D", "SUN", "/ST", "04:30"],
            ),
            (
                "0 9 * * 1-5",
                &["/SC", "WEEKLY", "/D", "MON,TUE,WED,THU,FRI", "/ST", "09:00"],
            ),
            (
                "0 9 * * 1,3",
                &["/SC", "WEEKLY", "/D", "MON,WED", "/ST", "09:00"],
            ),
            // Day-of-month was ignored too: monthly became daily.
            ("30 4 1 * *", &["/SC", "MONTHLY", "/D", "1", "/ST", "04:30"]),
        ];
        for (cron, want) in cases {
            assert_eq!(
                map_cron_to_schtasks(cron).unwrap_or_else(|e| panic!("{cron}: {e}")),
                *want,
                "wrong schtasks args for `{cron}`"
            );
        }
    }

    /// The property, not the cases: whatever a cron says, the time handed to Task Scheduler is
    /// always `HH:mm`. This is the assertion that would have caught the reported defect without
    /// anyone thinking of `0 3 * * *` in particular.
    #[test]
    fn schtasks_never_emits_a_time_task_scheduler_cannot_read() {
        for cron in [
            "0 3 * * *",
            "@daily",
            "@weekly",
            "@monthly",
            "@yearly",
            "@hourly",
            "5 9 * * *",
            "0 0 * * *",
            "59 23 * * *",
            "30 4 1 * *",
            "0 3 * * 1",
            "0 */6 * * *",
            "7 7 7 7 *",
        ] {
            let args = map_cron_to_schtasks(cron).unwrap_or_else(|e| panic!("{cron}: {e}"));
            if let Some(i) = args.iter().position(|a| a == "/ST") {
                let st = &args[i + 1];
                let (h, m) = st.split_once(':').unwrap_or_else(|| panic!("{cron}: {st}"));
                assert!(
                    h.len() == 2 && m.len() == 2,
                    "`{cron}` produced /ST {st}, which Task Scheduler rejects"
                );
                assert!(
                    h.chars().chain(m.chars()).all(|c| c.is_ascii_digit()),
                    "`{cron}` produced /ST {st}, which is not a time"
                );
                assert!(
                    h.parse::<u8>().unwrap() < 24 && m.parse::<u8>().unwrap() < 60,
                    "`{cron}` produced /ST {st}, which is not a real time"
                );
            }
        }
    }

    /// A cron Task Scheduler genuinely cannot express is refused by name — it does not quietly
    /// become DAILY. Running more often than declared is the failure mode this whole fix is
    /// about, and it must not survive as the error path.
    #[test]
    fn a_cron_windows_cannot_express_is_refused_rather_than_widened() {
        // Every 15 minutes, but only during hour 9. `/SC MINUTE /MO 15` runs all day.
        let err = map_cron_to_schtasks("*/15 9 * * *").unwrap_err();
        assert!(
            err.contains("*/15 9 * * *"),
            "does not quote the cron: {err}"
        );
        assert!(
            err.to_lowercase().contains("task scheduler"),
            "does not say which scheduler cannot do it: {err}"
        );
    }

    // ---- the read-back --------------------------------------------------------------------

    /// The whole point of the durable fix: what Shall would write and what it reads back are
    /// the same string, so an unedited schedule compares equal and an edited one does not.
    #[test]
    fn a_systemd_unit_written_and_read_back_is_the_same_spec() {
        let p = LinuxSystemdProvisioner;
        let bin = Path::new("/usr/local/bin/shall");
        let a = p.rendered(&cfg("0 2 * * *"), bin).unwrap();

        // Assembled the way `read_task` assembles it from the two files on disk.
        let mut from_disk = LinuxSystemdProvisioner::service_unit(&cfg("0 2 * * *"), bin);
        from_disk.push_str(UNIT_SEPARATOR);
        from_disk.push_str(&p.timer_unit(&cfg("0 2 * * *")));
        assert_eq!(a.spec, from_disk);

        // And an edit to either half moves it.
        let mut edited = cfg("0 3 * * *");
        edited.command = "sync".into();
        assert_ne!(p.rendered(&edited, bin).unwrap().spec, a.spec);
        let mut other_command = cfg("0 2 * * *");
        other_command.command = "clean".into();
        assert_ne!(p.rendered(&other_command, bin).unwrap().spec, a.spec);
    }

    /// A `@reboot` job keeps its log redirection. It did not: the boot shape was written by
    /// overwriting the file the ordinary shape had just written, and the replacement had no
    /// `StandardOutput=` at all — so the one kind of job nobody watches run wrote its output
    /// nowhere.
    #[test]
    fn a_boot_job_still_says_where_its_output_goes() {
        let unit = LinuxSystemdProvisioner::service_unit(&cfg("@reboot"), Path::new("/bin/shall"));
        assert!(unit.contains("StandardOutput=append:"), "{}", unit);
        assert!(unit.contains("StandardError=append:"), "{}", unit);
        assert!(unit.contains("WantedBy=default.target"), "{}", unit);
        // And it has no timer to be installed into.
        let p = LinuxSystemdProvisioner;
        assert!(!p
            .rendered(&cfg("@reboot"), Path::new("/bin/shall"))
            .unwrap()
            .spec
            .contains("OnCalendar="));
    }

    /// Every new option reaches the unit systemd reads, and an option nobody wrote leaves the
    /// timer exactly as it was before the option existed.
    #[test]
    fn the_timer_carries_the_options_that_are_declared_and_nothing_else() {
        let p = LinuxSystemdProvisioner;
        let plain = p.timer_unit(&cfg("0 2 * * *"));
        assert!(plain.contains("Persistent=true"), "{}", plain);
        assert!(!plain.contains("RandomizedDelaySec"), "{}", plain);

        let mut rich = cfg("0 2 * * *");
        rich.persistent = Some(false);
        rich.jitter = Some(1800);
        let unit = p.timer_unit(&rich);
        assert!(unit.contains("Persistent=false"), "{}", unit);
        assert!(unit.contains("RandomizedDelaySec=1800"), "{}", unit);
    }

    /// `enabled = false` is a declaration about the machine, not a comment: it changes what
    /// `rendered` claims, so a task somebody armed by hand reports as drift.
    #[test]
    fn a_schedule_declared_silent_is_rendered_disarmed_everywhere() {
        let mut silent = cfg("0 2 * * *");
        silent.enabled = Some(false);
        let bin = Path::new("/bin/shall");
        assert!(
            !LinuxSystemdProvisioner
                .rendered(&silent, bin)
                .unwrap()
                .armed
        );
        assert!(!MacLaunchdProvisioner.rendered(&silent, bin).unwrap().armed);
        assert!(!WindowsTaskProvisioner.rendered(&silent, bin).unwrap().armed);
        // launchd carries it in the file as well, so a reboot does not arm it behind us.
        assert!(MacLaunchdProvisioner
            .plist(&silent, bin)
            .contains("<key>Disabled</key><true/>"));
        assert!(!MacLaunchdProvisioner
            .plist(&cfg("0 2 * * *"), bin)
            .contains("Disabled"));
    }

    /// Every option, against every scheduler, stated as a table — because a matrix checked one
    /// cell at a time is a matrix with a hole in it. `Ok` means the scheduler expresses it;
    /// `Err` means it says so by name rather than accepting the option and dropping it.
    #[test]
    fn every_option_is_either_expressed_or_refused_by_name() {
        let with = |f: fn(&mut ScheduleConfig)| {
            let mut c = cfg("0 2 * * *");
            f(&mut c);
            c
        };
        let boot = |f: fn(&mut ScheduleConfig)| {
            let mut c = cfg("@reboot");
            f(&mut c);
            c
        };
        let cases: &[(&str, ScheduleConfig, bool, bool, bool)] = &[
            // option            declaration                                 systemd  launchd  windows
            (
                "enabled=false",
                with(|c| c.enabled = Some(false)),
                true,
                true,
                true,
            ),
            (
                "enabled=true",
                with(|c| c.enabled = Some(true)),
                true,
                true,
                true,
            ),
            (
                "persistent=true",
                with(|c| c.persistent = Some(true)),
                true,
                true,
                false,
            ),
            (
                "persistent=false",
                with(|c| c.persistent = Some(false)),
                true,
                false,
                false,
            ),
            ("jitter", with(|c| c.jitter = Some(900)), true, false, false),
            (
                "elevated=true",
                with(|c| c.elevated = Some(true)),
                false,
                false,
                true,
            ),
            (
                "elevated=false",
                with(|c| c.elevated = Some(false)),
                true,
                true,
                true,
            ),
            // A timer option on a job that has no timer.
            (
                "jitter@reboot",
                boot(|c| c.jitter = Some(900)),
                false,
                false,
                false,
            ),
            (
                "persistent@reboot",
                boot(|c| c.persistent = Some(true)),
                false,
                false,
                false,
            ),
        ];
        for (what, config, systemd, launchd, windows) in cases {
            let got = (
                LinuxSystemdProvisioner.refuse_unsupported(config).is_ok(),
                MacLaunchdProvisioner.refuse_unsupported(config).is_ok(),
                WindowsTaskProvisioner.refuse_unsupported(config).is_ok(),
            );
            assert_eq!(
                got,
                (*systemd, *launchd, *windows),
                "{} is expressed/refused differently than the table says",
                what
            );
        }
    }

    /// A refusal names the option, so the reader knows which line to change.
    #[test]
    fn a_refusal_names_the_option_it_is_about() {
        let mut c = cfg("0 2 * * *");
        c.jitter = Some(900);
        let err = MacLaunchdProvisioner.refuse_unsupported(&c).unwrap_err();
        assert!(err.contains("`jitter`"), "{}", err);
        let err = WindowsTaskProvisioner.refuse_unsupported(&c).unwrap_err();
        assert!(err.contains("`jitter`"), "{}", err);

        let mut c = cfg("0 2 * * *");
        c.elevated = Some(true);
        let err = LinuxSystemdProvisioner.refuse_unsupported(&c).unwrap_err();
        assert!(err.contains("`elevated = true`"), "{}", err);
    }

    // ---- the Windows canonical form -------------------------------------------------------

    /// Real Task Scheduler XML, captured from tasks on a Windows 11 machine on 2026-08-16 —
    /// the trigger shapes are quoted rather than imagined, which is the only reason this
    /// comparison can be trusted.
    fn task_xml(triggers: &str, settings_enabled: &str, run_level: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n\
             <Task version=\"1.2\">\n\
             <Triggers>{triggers}</Triggers>\n\
             <Principals><Principal id=\"Author\"><UserId>S-1-5-18</UserId>{run_level}\
             </Principal></Principals>\n\
             <Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>\
             <Enabled>{settings_enabled}</Enabled></Settings>\n\
             <Actions Context=\"Author\"><Exec>\
             <Command>C:\\Program Files\\shall\\shall.exe</Command><Arguments>sync</Arguments>\
             </Exec></Actions>\n</Task>"
        )
    }

    #[test]
    fn a_trigger_shall_wrote_reads_back_as_the_when_shall_rendered() {
        let cases: &[(&str, &str)] = &[
            (
                "<CalendarTrigger id=\"Trigger1\"><StartBoundary>2026-07-06T03:00:00\
                 </StartBoundary><ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay>\
                 </CalendarTrigger>",
                "daily at 03:00",
            ),
            (
                "<CalendarTrigger><StartBoundary>2010-12-16T09:00:00</StartBoundary>\
                 <ScheduleByWeek><WeeksInterval>1</WeeksInterval><DaysOfWeek><Monday />\
                 <Wednesday /></DaysOfWeek></ScheduleByWeek></CalendarTrigger>",
                "weekly on MON,WED at 09:00",
            ),
            (
                "<CalendarTrigger><StartBoundary>2010-12-16T04:30:00</StartBoundary>\
                 <ScheduleByMonth><DaysOfMonth><Day>1</Day></DaysOfMonth><Months><January />\
                 </Months></ScheduleByMonth></CalendarTrigger>",
                "monthly in JAN on 1 at 04:30",
            ),
            ("<BootTrigger><Delay>PT30S</Delay></BootTrigger>", "boot"),
            (
                "<TimeTrigger><StartBoundary>1992-05-01T00:00:00</StartBoundary>\
                 <Repetition><Interval>PT15M</Interval></Repetition></TimeTrigger>",
                "every 15m",
            ),
            (
                "<TimeTrigger><StartBoundary>1992-05-01T00:00:00</StartBoundary>\
                 <Repetition><Interval>PT6H</Interval></Repetition></TimeTrigger>",
                "every 6h from 00:00",
            ),
        ];
        for (triggers, want) in cases {
            let doc = task_xml(triggers, "true", "");
            assert_eq!(
                when_from_xml(&doc).as_deref(),
                Some(*want),
                "trigger read wrong: {}",
                triggers
            );
        }
    }

    /// The two halves meet: for every cron Task Scheduler can express, the `when` derived from
    /// the `/SC` arguments is one of the strings the XML reader produces. A canonical form only
    /// one side can write is not a comparison, it is a permanent mismatch.
    #[test]
    fn the_rendered_when_and_the_read_when_use_one_vocabulary() {
        let pairs: &[(&str, &str)] = &[
            ("0 3 * * *", "daily at 03:00"),
            ("@reboot", "boot"),
            ("0 9 * * 1,3", "weekly on MON,WED at 09:00"),
            ("30 4 1 * *", "monthly on 1 at 04:30"),
            ("@yearly", "monthly in JAN on 1 at 00:00"),
            ("*/15 * * * *", "every 15m"),
            ("0 */6 * * *", "every 6h from 00:00"),
        ];
        for (cron, want) in pairs {
            let args = map_cron_to_schtasks(cron).unwrap();
            assert_eq!(
                schtasks_when(&args).as_deref(),
                Some(*want),
                "`{}` canonicalises wrong",
                cron
            );
        }
    }

    /// A cron that names its weekdays out of order is the same schedule as one that names them
    /// in order — `<DaysOfWeek>` is a set and comes back Sunday-first, so the declaration is
    /// sorted the same way or every such schedule reports drift for ever.
    #[test]
    fn weekdays_compare_as_a_set_not_as_a_list() {
        let a = map_cron_to_schtasks("0 9 * * 5,1").unwrap();
        let b = map_cron_to_schtasks("0 9 * * 1,5").unwrap();
        assert_ne!(a, b, "the /D lists genuinely differ in order");
        assert_eq!(schtasks_when(&a), schtasks_when(&b));
    }

    /// A shape Shall does not write is **unreadable**, never drift. Reported as a mismatch it
    /// would rewrite the task on every sync for ever and keep `check` red on something the
    /// reader simply did not understand — V.188's rule, on a different store.
    #[test]
    fn a_trigger_shall_does_not_write_is_unreadable_rather_than_wrong() {
        for triggers in [
            // Somebody added a second trigger by hand.
            "<CalendarTrigger><StartBoundary>2026-07-06T03:00:00</StartBoundary><ScheduleByDay>\
             <DaysInterval>1</DaysInterval></ScheduleByDay></CalendarTrigger>\
             <LogonTrigger><Delay>PT5M</Delay></LogonTrigger>",
            // Every third day is not a schedule Shall can write.
            "<CalendarTrigger><StartBoundary>2026-07-06T03:00:00</StartBoundary><ScheduleByDay>\
             <DaysInterval>3</DaysInterval></ScheduleByDay></CalendarTrigger>",
            // An event trigger, which has no cron at all.
            "<EventTrigger><Subscription>x</Subscription></EventTrigger>",
            // A shape this reader understands, with a shape it does not standing beside it.
            // Counting only the kinds it knows would read the calendar trigger and report the
            // task as matching a declaration that says nothing about the event.
            "<CalendarTrigger><StartBoundary>2026-07-06T03:00:00</StartBoundary><ScheduleByDay>\
             <DaysInterval>1</DaysInterval></ScheduleByDay></CalendarTrigger>\
             <EventTrigger><Subscription>x</Subscription></EventTrigger>",
            // A day-scale repetition. Shall writes daily as a calendar trigger, never as a
            // repetition, so this is somebody else's task under Shall's name.
            "<TimeTrigger><StartBoundary>1992-05-01T01:00:00</StartBoundary>\
             <Repetition><Interval>P1D</Interval></Repetition></TimeTrigger>",
            // No triggers whatsoever.
            "",
        ] {
            let doc = task_xml(triggers, "true", "");
            assert_eq!(when_from_xml(&doc), None, "read as drift: {}", triggers);
        }
    }

    /// The element that opens the block is not one of the things inside it, and a closing tag is
    /// not an opening one. Both would make "exactly one trigger" mean nothing.
    #[test]
    fn the_trigger_count_counts_triggers_and_not_the_block_around_them() {
        assert_eq!(trigger_count("<Triggers></Triggers>"), 0);
        assert_eq!(trigger_count("<Triggers><BootTrigger /></Triggers>"), 1);
        assert_eq!(
            trigger_count("<Triggers><BootTrigger><Delay>PT1M</Delay></BootTrigger></Triggers>"),
            1
        );
        assert_eq!(
            trigger_count("<Triggers><CalendarTrigger id=\"a\" /><EventTrigger /></Triggers>"),
            2
        );
    }

    /// `<Enabled>` appears in two places and they mean different things. A reader that takes
    /// the first one in the document answers about a trigger and calls it the task.
    #[test]
    fn the_task_switch_is_read_from_settings_and_not_from_a_trigger() {
        let doc = task_xml(
            "<BootTrigger id=\"Resume\"><Enabled>false</Enabled></BootTrigger>",
            "true",
            "",
        );
        let settings = xml_element(&doc, "Settings").unwrap();
        assert_eq!(xml_text(settings, "Enabled"), Some("true"));
    }

    #[test]
    fn the_run_level_is_read_where_task_scheduler_writes_it() {
        let elevated = task_xml(
            "<BootTrigger />",
            "true",
            "<RunLevel>HighestAvailable</RunLevel>",
        );
        assert_eq!(
            xml_element(&elevated, "Principals").and_then(|p| xml_text(p, "RunLevel")),
            Some("HighestAvailable")
        );
        let plain = task_xml("<BootTrigger />", "true", "");
        assert_eq!(
            xml_element(&plain, "Principals").and_then(|p| xml_text(p, "RunLevel")),
            None
        );
    }

    /// The document declares UTF-16 and can arrive either width. Decoding by the header rather
    /// than by the bytes would make the reading depend on how the process was spawned.
    #[test]
    fn the_query_is_decoded_whichever_width_it_arrives_in() {
        let text = "<Task><Enabled>true</Enabled></Task>";
        assert_eq!(decode_xml(text.as_bytes()), text);

        let mut utf16 = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode_xml(&utf16), text);

        // And without the byte-order mark, which is how it arrives through a pipe.
        assert_eq!(decode_xml(&utf16[2..]), text);
    }

    #[test]
    fn the_windows_spec_names_what_runs_when_and_at_which_privilege() {
        let mut elevated = cfg("0 3 * * *");
        elevated.elevated = Some(true);
        let bin = Path::new("C:\\Program Files\\shall\\shall.exe");
        let plain = WindowsTaskProvisioner
            .rendered(&cfg("0 3 * * *"), bin)
            .unwrap();
        let raised = WindowsTaskProvisioner.rendered(&elevated, bin).unwrap();
        assert!(plain.spec.contains("LeastPrivilege"), "{}", plain.spec);
        assert!(raised.spec.contains("HighestAvailable"), "{}", raised.spec);
        assert!(plain.spec.contains("daily at 03:00"), "{}", plain.spec);
        assert!(
            plain
                .spec
                .contains("C:\\Program Files\\shall\\shall.exe sync"),
            "{}",
            plain.spec
        );
        assert_ne!(plain.spec, raised.spec);
    }

    /// The read side of the same task produces the same spec — which is the whole comparison,
    /// and the thing that was missing.
    #[test]
    fn a_task_shall_created_reads_back_equal_to_what_shall_rendered() {
        let doc = task_xml(
            "<CalendarTrigger><StartBoundary>2026-07-06T03:00:00</StartBoundary>\
             <ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay></CalendarTrigger>",
            "true",
            "",
        );
        let exec = xml_element(&doc, "Exec").unwrap();
        let command = format!(
            "{} {}",
            xml_text(exec, "Command").unwrap().trim_matches('"'),
            xml_text(exec, "Arguments").unwrap()
        );
        let read = schtasks_spec(&command, &when_from_xml(&doc).unwrap(), false);

        let bin = Path::new("C:\\Program Files\\shall\\shall.exe");
        let rendered = WindowsTaskProvisioner
            .rendered(&cfg("0 3 * * *"), bin)
            .unwrap();
        assert_eq!(read, rendered.spec);
    }

    /// A **verbatim** `schtasks /Query /XML` document, captured from this machine on
    /// 2026-08-16 and not edited. Every fixture above is one this file wrote, so every fixture
    /// above shares this file's assumptions; this one does not. It carries three things nothing
    /// hand-written would have thought to include — an `xmlns` on `<Task>`, an `id` attribute on
    /// the trigger, and **no `<Enabled>` in `<Settings>` at all**, which is what Task Scheduler
    /// emits for a task nobody has disabled.
    const REAL_TASK_XML: &str = r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <URI>\CorelUpdateHelperTaskCore</URI>
  </RegistrationInfo>
  <Principals>
    <Principal id="Author">
      <GroupId>S-1-5-32-545</GroupId>
    </Principal>
  </Principals>
  <Settings>
    <DisallowStartIfOnBatteries>true</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>true</StopIfGoingOnBatteries>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <StartWhenAvailable>true</StartWhenAvailable>
    <IdleSettings>
      <Duration>PT10M</Duration>
      <WaitTimeout>PT1H</WaitTimeout>
      <StopOnIdleEnd>true</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
  </Settings>
  <Triggers>
    <CalendarTrigger id="Trigger1">
      <StartBoundary>2026-07-06T13:56:53</StartBoundary>
      <ScheduleByDay>
        <DaysInterval>1</DaysInterval>
      </ScheduleByDay>
    </CalendarTrigger>
  </Triggers>
  <Actions Context="Author">
    <Exec>
      <Command>c:\Program Files (x86)\Corel\CUH\v2\CUH.exe</Command>
      <Arguments>/t</Arguments>
    </Exec>
  </Actions>
</Task>"#;

    #[test]
    fn a_real_task_scheduler_document_reads_the_way_the_fixtures_say_it_does() {
        let held = provisioned_from_xml(REAL_TASK_XML).expect("a real document must be readable");
        assert_eq!(
            held.spec,
            schtasks_spec(
                "c:\\Program Files (x86)\\Corel\\CUH\\v2\\CUH.exe /t",
                "daily at 13:56",
                false
            )
        );
        // No `<Enabled>` anywhere in `<Settings>`, and the task is not disabled — so absent has
        // to mean armed. Reading it as "not armed" would report drift on every ordinary task.
        assert!(held.armed);
        // And the nested `<IdleSettings>` did not swallow the `<Settings>` lookup.
        assert!(xml_element(REAL_TASK_XML, "Settings")
            .expect("settings")
            .contains("IdleSettings"));
    }

    #[test]
    fn standing_speaks_the_three_valued_answer_in_effect_wants() {
        assert_eq!(Standing::Matches.in_effect(), Some(true));
        assert_eq!(Standing::Missing.in_effect(), Some(false));
        assert_eq!(Standing::Differs("x").in_effect(), Some(false));
        assert_eq!(Standing::Unknown.in_effect(), None);
    }
}
