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

struct LinuxSystemdProvisioner;

#[async_trait]
impl TaskProvisioner for LinuxSystemdProvisioner {
    async fn add_task(
        &self,
        executor: &CommandExecutor,
        config: &ScheduleConfig,
        shall_bin: &Path,
    ) -> Result<()> {
        let systemd_dir = dirs::config_dir()
            .ok_or_else(|| Error::Io("User configuration directory not found".into()))?
            .join("systemd")
            .join("user");

        crate::utils::file::ensure_dir(&systemd_dir)?;

        let unit_name = format!("shall-{}", config.name);
        let service_path = systemd_dir.join(format!("{}.service", unit_name));
        let timer_path = systemd_dir.join(format!("{}.timer", unit_name));

        let use_boot_timer = config.cron == "@reboot";

        let service_content = format!(
            "[Unit]\nDescription=Shall Job: {name}\n\n\
             [Service]\nType=oneshot\nExecStart={bin} {cmd}\n\
             StandardOutput=append:{log}\nStandardError=append:{log}\n",
            name = config.name,
            bin = shall_bin.display(),
            cmd = config.command,
            log = crate::utils::safe_data_dir().join("schedule.log").display()
        );

        executor
            .write_atomic(&service_path, &service_content)
            .await?;

        if use_boot_timer {
            let boot_service = format!(
                "[Unit]\nDescription=Shall Reboot Job: {name}\n\n\
                 [Service]\nType=oneshot\nExecStart={bin} {cmd}\n\n\
                 [Install]\nWantedBy=default.target\n",
                name = config.name,
                bin = shall_bin.display(),
                cmd = config.command
            );
            executor.write_atomic(&service_path, &boot_service).await?;
            executor
                .run(
                    "systemctl",
                    &["--no-pager", "--user", "daemon-reload"],
                    false,
                )
                .await?;
            executor
                .run(
                    "systemctl",
                    &[
                        "--no-pager",
                        "--user",
                        "enable",
                        &format!("{}.service", unit_name),
                    ],
                    false,
                )
                .await?;
        } else {
            let calendar_spec = self.map_cron_to_systemd(&config.cron);
            let timer_content = format!(
                "[Unit]\nDescription=Shall Schedule Timer for {name}\n\n\
                 [Timer]\nOnCalendar={calendar}\nPersistent=true\n\n\
                 [Install]\nWantedBy=timers.target\n",
                name = config.name,
                calendar = calendar_spec
            );
            executor.write_atomic(&timer_path, &timer_content).await?;
            executor
                .run(
                    "systemctl",
                    &["--no-pager", "--user", "daemon-reload"],
                    false,
                )
                .await?;
            executor
                .run(
                    "systemctl",
                    &[
                        "--no-pager",
                        "--user",
                        "enable",
                        "--now",
                        &format!("{}.timer", unit_name),
                    ],
                    false,
                )
                .await?;
        }

        Ok(())
    }

    async fn remove_task(&self, executor: &CommandExecutor, name: &str) -> Result<()> {
        let timer_name = format!("shall-{}.timer", name);
        let service_name = format!("shall-{}.service", name);

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

        if let Some(config_dir) = dirs::config_dir() {
            let systemd_dir = config_dir.join("systemd").join("user");
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

    async fn is_task_active(&self, executor: &CommandExecutor, name: &str) -> bool {
        let unit = format!("shall-{}.timer", name);
        match executor
            .run(
                "systemctl",
                &["--no-pager", "--user", "is-active", &unit],
                false,
            )
            .await
        {
            Ok(out) => {
                crate::utils::text::sanitize(&String::from_utf8_lossy(&out.stdout)) == "active"
            }
            Err(_) => false,
        }
    }
}

impl LinuxSystemdProvisioner {
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

struct MacLaunchdProvisioner;

#[async_trait]
impl TaskProvisioner for MacLaunchdProvisioner {
    async fn add_task(
        &self,
        executor: &CommandExecutor,
        config: &ScheduleConfig,
        shall_bin: &Path,
    ) -> Result<()> {
        let label = format!("com.shall.{}", config.name);
        let plist_path = dirs::home_dir()
            .ok_or_else(|| Error::Io("Could not locate home directory".into()))?
            .join("Library/LaunchAgents")
            .join(format!("{}.plist", label));

        let is_reboot = config.cron == "@reboot";
        let schedule_xml = if is_reboot {
            "<key>RunAtLoad</key><true/>".to_string()
        } else {
            format!(
                "<key>StartCalendarInterval</key>{}",
                self.map_cron_to_launchd_xml(&config.cron)
            )
        };

        let plist_content = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
            <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
            <plist version=\"1.0\">\n<dict>\n\
            <key>Label</key><string>{label}</string>\n\
            <key>ProgramArguments</key>\n<array>\n\
            <string>{bin}</string><string>{cmd}</string>\n</array>\n\
            {schedule}\n\
            <key>StandardOutPath</key><string>{log}</string>\n\
            <key>StandardErrorPath</key><string>{log}</string>\n\
            </dict>\n</plist>",
            label = label, bin = shall_bin.display(), cmd = config.command,
            schedule = schedule_xml,
            log = crate::utils::safe_data_dir().join("schedule.log").display()
        );

        executor.write_atomic(&plist_path, &plist_content).await?;
        executor
            .run("launchctl", &["load", &plist_path.to_string_lossy()], false)
            .await?;
        Ok(())
    }

    async fn remove_task(&self, executor: &CommandExecutor, name: &str) -> Result<()> {
        let label = format!("com.shall.{}", name);
        if let Some(home) = dirs::home_dir() {
            let plist_path = home
                .join("Library/LaunchAgents")
                .join(format!("{}.plist", label));
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
                name, label
            )));
        }
        Ok(())
    }

    async fn is_task_active(&self, executor: &CommandExecutor, name: &str) -> bool {
        let label = format!("com.shall.{}", name);
        match executor.run("launchctl", &["list", &label], false).await {
            Ok(o) => o.status.success(),
            Err(_) => false,
        }
    }
}

impl MacLaunchdProvisioner {
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

/// cron's day-of-week (0/7 = Sunday) as the day names `/D` takes, comma-separated.
fn schtasks_days(dow: &str) -> Option<String> {
    const NAMES: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];
    let name = |n: u8| NAMES.get((n % 7) as usize).copied();

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
        const MONTHS: [&str; 12] = [
            "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
        ];
        let mut args = vec![s("/SC"), s("MONTHLY")];
        if c.month != "*" {
            let n: usize = c.month.parse().map_err(|_| cannot())?;
            let name = MONTHS.get(n.wrapping_sub(1)).ok_or_else(cannot)?;
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

struct WindowsTaskProvisioner;

#[async_trait]
impl TaskProvisioner for WindowsTaskProvisioner {
    async fn add_task(
        &self,
        executor: &CommandExecutor,
        config: &ScheduleConfig,
        shall_bin: &Path,
    ) -> Result<()> {
        let name = format!("Shall_{}", config.name);
        let cmd = format!("\"{}\" {}", shall_bin.display(), config.command);

        // Refused here, before anything is created: a schedule Task Scheduler cannot express
        // must not become the nearest one it can. `Refused` and not `Io` — Shall looked and
        // declined on purpose, which is exit code 3 (U21), and a script that retries on failure
        // must not retry this.
        let schedule = map_cron_to_schtasks(&config.cron)
            .map_err(|e| Error::Refused(format!("`schedule:{}`: {}", config.name, e)))?;

        let mut args: Vec<&str> = vec!["/Create", "/TN", &name, "/TR", &cmd, "/F"];
        args.extend(schedule.iter().map(String::as_str));

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
        Ok(())
    }

    async fn remove_task(&self, executor: &CommandExecutor, name: &str) -> Result<()> {
        let tn = format!("Shall_{}", name);
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
        let tn = format!("Shall_{}", name);
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
    use super::{map_cron_to_schtasks, remove_generated, LinuxSystemdProvisioner};

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
}
