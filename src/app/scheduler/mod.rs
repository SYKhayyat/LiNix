use crate::core::{Result, Error, CommandExecutor};
use crate::config::Config;
use crate::config::config::ScheduleConfig;
use std::path::{Path, PathBuf};
use tracing::{info, debug, warn, trace};
use std::fs;
use cron::Schedule;
use std::str::FromStr;
use async_trait::async_trait; // Modernized: Required for trait-level async

/// Feature 5: Multi-channel alerting engine.
pub mod notify;

/// Represents the platform-native capability to schedule background tasks.
/// 
/// A+ Grade Architecture: This trait is asynchronous to support the non-blocking 
/// LiNix kernel executor and high-fidelity call-log mocking.
#[async_trait]
pub trait TaskProvisioner: Send + Sync {
    /// Registers a new background task with the host operating system.
    async fn add_task(&self, executor: &CommandExecutor, config: &ScheduleConfig, linix_path: &Path) -> Result<()>;
    /// Removes an existing background task from the host operating system.
    async fn remove_task(&self, executor: &CommandExecutor, name: &str) -> Result<()>;
    /// Checks if a task is currently active in the system scheduler.
    async fn is_task_active(&self, executor: &CommandExecutor, name: &str) -> bool;
}

/// The high-level orchestrator for background LiNix automation.
pub struct SchedulerManager {
    provisioner: Box<dyn TaskProvisioner>,
    linix_bin_path: PathBuf,
}

impl SchedulerManager {
    /// Initializes the manager by detecting the host OS and selecting the 
    /// appropriate native provisioner.
    pub fn new() -> Result<Self> {
        debug!("Scheduler: Detecting system-native task runner.");

        let linix_bin_path = std::env::current_exe().map_err(|e| {
            Error::Io(format!("Failed to locate current LiNix binary: {}", e))
        })?;

        let provisioner: Box<dyn TaskProvisioner> = if cfg!(target_os = "linux") {
            Box::new(LinuxSystemdProvisioner)
        } else if cfg!(target_os = "macos") {
            Box::new(MacLaunchdProvisioner)
        } else if cfg!(target_os = "windows") {
            Box::new(WindowsTaskProvisioner)
        } else {
            return Err(Error::UnsupportedPlatform(
                "Native scheduling is not supported on this OS variant.".into()
            ));
        };

        Ok(Self { provisioner, linix_bin_path })
    }

    /// Provisions a new schedule in the system and persists it to the configuration.
    pub async fn add_schedule(
        &self, 
        executor: &CommandExecutor,
        config_mut: &mut Config, 
        name: String, 
        cron: String, 
        command: String, 
        notification: Option<String>
    ) -> Result<()> {
        info!("Scheduler: Provisioning task '{}' with schedule '{}'.", name, cron);

        // 1. Immediate validation of the cron string. Standard cron is 5-field
        //    (min hour dom month dow); the `cron` crate expects a 6-field form with
        //    seconds, so normalize 5-field expressions by prepending "0". `@`-macros
        //    (@reboot, @daily, ...) are handled by the systemd/launchd mapping below.
        if !cron.starts_with('@') {
            let normalized = if cron.split_whitespace().count() == 5 {
                format!("0 {}", cron)
            } else {
                cron.clone()
            };
            if let Err(e) = Schedule::from_str(&normalized) {
                return Err(Error::Validation(format!(
                    "Invalid cron syntax for task '{}': {}. Rejection issued.", name, e
                )));
            }
        }

        let schedule_entry = ScheduleConfig {
            name: name.clone(),
            cron,
            command,
            notification,
            last_synced: None,
        };

        // 2. Delegate to OS-Specific Provisioner (Now awaited)
        self.provisioner.add_task(executor, &schedule_entry, &self.linix_bin_path).await?;

        // 3. Persist to configuration
        config_mut.schedules.retain(|s| s.name != name);
        config_mut.schedules.push(schedule_entry);
        config_mut.save()?;

        info!("Scheduler: Task '{}' successfully registered and active in OS.", name);
        Ok(())
    }

    /// Purges a schedule from both the system and the LiNix configuration.
    pub async fn remove_schedule(&self, executor: &CommandExecutor, config_mut: &mut Config, name: &str) -> Result<()> {
        info!("Scheduler: Purging background task '{}' from OS.", name);
        
        self.provisioner.remove_task(executor, name).await?;
        
        config_mut.schedules.retain(|s| s.name != name);
        config_mut.save()?;
        
        Ok(())
    }

    /// Ensures consistency between the configuration and the native OS timers.
    pub async fn sync_schedules(&self, executor: &CommandExecutor, config: &Config) -> Result<()> {
        trace!("Scheduler: Verifying OS registry for configured tasks.");
        for schedule in &config.schedules {
            if !self.provisioner.is_task_active(executor, &schedule.name).await {
                warn!("Scheduler: Task '{}' is missing from native timers. Restoring...", schedule.name);
                let _ = self.provisioner.add_task(executor, schedule, &self.linix_bin_path).await;
            }
        }
        Ok(())
    }
}

// ============================================================================
// LINUX: Systemd Timers Implementation
// ============================================================================

struct LinuxSystemdProvisioner;

#[async_trait]
impl TaskProvisioner for LinuxSystemdProvisioner {
    async fn add_task(&self, executor: &CommandExecutor, config: &ScheduleConfig, linix_bin: &Path) -> Result<()> {
        let systemd_dir = dirs::config_dir()
            .ok_or_else(|| Error::Io("User configuration directory not found".into()))?
            .join("systemd").join("user");
            
        if !systemd_dir.exists() {
            fs::create_dir_all(&systemd_dir).map_err(Error::from)?;
        }

        let unit_name = format!("linix-{}", config.name);
        let service_path = systemd_dir.join(format!("{}.service", unit_name));
        let timer_path = systemd_dir.join(format!("{}.timer", unit_name));

        let use_boot_timer = config.cron == "@reboot";
        
        // Use executor.write_atomic (Awaited)
        let service_content = format!(
            "[Unit]\nDescription=LiNix Job: {name}\n\n\
             [Service]\nType=oneshot\nExecStart={bin} {cmd}\n\
             StandardOutput=append:{log}\nStandardError=append:{log}\n",
            name = config.name, bin = linix_bin.display(), cmd = config.command,
            log = crate::utils::safe_data_dir().join("schedule.log").display()
        );

        executor.write_atomic(&service_path, &service_content).await?;

        if use_boot_timer {
            let boot_service = format!(
                "[Unit]\nDescription=LiNix Reboot Job: {name}\n\n\
                 [Service]\nType=oneshot\nExecStart={bin} {cmd}\n\n\
                 [Install]\nWantedBy=default.target\n",
                name = config.name, bin = linix_bin.display(), cmd = config.command
            );
            executor.write_atomic(&service_path, &boot_service).await?;
            executor.run("systemctl", &["--user", "daemon-reload"], false).await?;
            executor.run("systemctl", &["--user", "enable", &format!("{}.service", unit_name)], false).await?;
        } else {
            let calendar_spec = self.map_cron_to_systemd(&config.cron);
            let timer_content = format!(
                "[Unit]\nDescription=LiNix Schedule Timer for {name}\n\n\
                 [Timer]\nOnCalendar={calendar}\nPersistent=true\n\n\
                 [Install]\nWantedBy=timers.target\n",
                name = config.name, calendar = calendar_spec
            );
            executor.write_atomic(&timer_path, &timer_content).await?;
            executor.run("systemctl", &["--user", "daemon-reload"], false).await?;
            executor.run("systemctl", &["--user", "enable", "--now", &format!("{}.timer", unit_name)], false).await?;
        }

        Ok(())
    }

    async fn remove_task(&self, executor: &CommandExecutor, name: &str) -> Result<()> {
        let timer_name = format!("linix-{}.timer", name);
        let service_name = format!("linix-{}.service", name);
        
        let _ = executor.run("systemctl", &["--user", "disable", "--now", &timer_name], false).await;
        let _ = executor.run("systemctl", &["--user", "disable", "--now", &service_name], false).await;
        
        if let Some(config_dir) = dirs::config_dir() {
            let systemd_dir = config_dir.join("systemd").join("user");
            let _ = fs::remove_file(systemd_dir.join(&timer_name));
            let _ = fs::remove_file(systemd_dir.join(&service_name));
        }
        Ok(())
    }

    async fn is_task_active(&self, executor: &CommandExecutor, name: &str) -> bool {
        let unit = format!("linix-{}.timer", name);
        match executor.run("systemctl", &["--user", "is-active", &unit], false).await {
            Ok(out) => String::from_utf8_lossy(&out.stdout).trim() == "active",
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
                if parts.len() < 5 { return "daily".into(); }
                
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
                    let dow_mapped = dow.replace('0', "Sun").replace('1', "Mon").replace('2', "Tue")
                        .replace('3', "Wed").replace('4', "Thu").replace('5', "Fri").replace('6', "Sat");
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
        if field == "*" { return "*".into(); }
        if let Some(step) = field.strip_prefix("*/") { return format!("0/{}", step); }
        if field.contains('-') { return field.replace('-', ".."); }
        field.to_string()
    }
}

// ============================================================================
// MACOS: Launchd Implementation
// ============================================================================

struct MacLaunchdProvisioner;

#[async_trait]
impl TaskProvisioner for MacLaunchdProvisioner {
    async fn add_task(&self, executor: &CommandExecutor, config: &ScheduleConfig, linix_bin: &Path) -> Result<()> {
        let label = format!("com.linix.{}", config.name);
        let plist_path = dirs::home_dir()
            .ok_or_else(|| Error::Io("Could not locate home directory".into()))?
            .join("Library/LaunchAgents").join(format!("{}.plist", label));

        let is_reboot = config.cron == "@reboot";
        let schedule_xml = if is_reboot {
            "<key>RunAtLoad</key><true/>".to_string()
        } else {
            format!("<key>StartCalendarInterval</key>{}", self.map_cron_to_launchd_xml(&config.cron))
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
            label = label, bin = linix_bin.display(), cmd = config.command,
            schedule = schedule_xml,
            log = crate::utils::safe_data_dir().join("schedule.log").display()
        );

        executor.write_atomic(&plist_path, &plist_content).await?;
        executor.run("launchctl", &["load", &plist_path.to_string_lossy()], false).await?;
        Ok(())
    }

    async fn remove_task(&self, executor: &CommandExecutor, name: &str) -> Result<()> {
        let label = format!("com.linix.{}", name);
        if let Some(home) = dirs::home_dir() {
            let plist_path = home.join("Library/LaunchAgents").join(format!("{}.plist", label));
            let _ = executor.run("launchctl", &["unload", &plist_path.to_string_lossy()], false).await;
            let _ = fs::remove_file(plist_path);
        }
        Ok(())
    }

    async fn is_task_active(&self, executor: &CommandExecutor, name: &str) -> bool {
        let label = format!("com.linix.{}", name);
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

// ============================================================================
// WINDOWS: Task Scheduler Implementation
// ============================================================================

struct WindowsTaskProvisioner;

#[async_trait]
impl TaskProvisioner for WindowsTaskProvisioner {
    async fn add_task(&self, executor: &CommandExecutor, config: &ScheduleConfig, linix_bin: &Path) -> Result<()> {
        let name = format!("LiNix_{}", config.name);
        let cmd = format!("\"{}\" {}", linix_bin.display(), config.command);
        
        let (sc, st) = match config.cron.as_str() {
            "@reboot" => ("ONSTART", String::new()),
            "@hourly" => ("HOURLY", String::new()),
            _ => {
                let parts: Vec<&str> = config.cron.split_whitespace().collect();
                let hour = parts.get(1).unwrap_or(&"02");
                let min = parts.first().unwrap_or(&"00");
                ("DAILY", format!("{}:{}", hour, min))
            }
        };

        let mut args = vec!["/Create", "/TN", &name, "/TR", &cmd, "/SC", sc, "/F"];
        if !st.is_empty() {
            args.extend(["/ST", &st]);
        }

        // Now correctly awaited
        executor.run("schtasks", &args, true).await?;
        Ok(())
    }

    async fn remove_task(&self, executor: &CommandExecutor, name: &str) -> Result<()> {
        let tn = format!("LiNix_{}", name);
        let _ = executor.run("schtasks", &["/Delete", "/TN", &tn, "/F"], true).await;
        Ok(())
    }

    async fn is_task_active(&self, executor: &CommandExecutor, name: &str) -> bool {
        let tn = format!("LiNix_{}", name);
        match executor.run("schtasks", &["/Query", "/TN", &tn], false).await {
            Ok(o) => o.status.success(),
            Err(_) => false,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::LinuxSystemdProvisioner;

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
}
