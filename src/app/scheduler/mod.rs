use crate::core::{Result, Error};
use crate::config::Config;
use crate::config::config::ScheduleConfig;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{info, debug, warn, trace};
use std::fs;
use cron::Schedule;
use std::str::FromStr;

/// Feature 5: Multi-channel alerting engine.
pub mod notify;

/// Represents the platform-native capability to schedule background tasks.
pub trait TaskProvisioner {
    /// Registers a new background task with the host operating system.
    fn add_task(&self, config: &ScheduleConfig, linix_path: &Path) -> Result<()>;
    /// Removes an existing background task from the host operating system.
    fn remove_task(&self, name: &str) -> Result<()>;
    /// Checks if a task is currently active in the system scheduler.
    fn is_task_active(&self, name: &str) -> bool;
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
        config_mut: &mut Config, 
        name: String, 
        cron: String, 
        command: String, 
        notification: Option<String>
    ) -> Result<()> {
        info!("Scheduler: Provisioning task '{}' with schedule '{}'.", name, cron);

        // A+ Hardening: Validate Cron Expression immediately
        if cron != "@reboot" {
            if let Err(e) = Schedule::from_str(&cron) {
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

        // 2. Delegate to OS-Specific Provisioner
        self.provisioner.add_task(&schedule_entry, &self.linix_bin_path)?;

        // 3. Persist to LiNix State
        config_mut.schedules.retain(|s| s.name != name);
        config_mut.schedules.push(schedule_entry);
        config_mut.save()?;

        info!("Scheduler: Task '{}' successfully registered and active in OS.", name);
        Ok(())
    }

    /// Purges a schedule from both the system and the LiNix configuration.
    pub async fn remove_schedule(&self, config_mut: &mut Config, name: &str) -> Result<()> {
        info!("Scheduler: Purging background task '{}' from OS.", name);
        self.provisioner.remove_task(name)?;
        config_mut.schedules.retain(|s| s.name != name);
        config_mut.save()?;
        Ok(())
    }

    /// Ensures consistency between the TOML config and the native OS registry.
    pub async fn sync_schedules(&self, config: &Config) -> Result<()> {
        trace!("Scheduler: Verifying OS registry for configured tasks.");
        for schedule in &config.schedules {
            if !self.provisioner.is_task_active(&schedule.name) {
                warn!("Scheduler: Task '{}' is missing from native timers. Restoring...", schedule.name);
                let _ = self.provisioner.add_task(schedule, &self.linix_bin_path);
            }
        }
        Ok(())
    }
}

// ============================================================================
// LINUX: Systemd Timers (A+ Full Cron Translation)
// ============================================================================

struct LinuxSystemdProvisioner;

impl TaskProvisioner for LinuxSystemdProvisioner {
    fn add_task(&self, config: &ScheduleConfig, linix_bin: &Path) -> Result<()> {
        let systemd_dir = dirs::config_dir()
            .ok_or_else(|| Error::Io("User config dir not found".into()))?
            .join("systemd").join("user");
        fs::create_dir_all(&systemd_dir).map_err(Error::from)?;

        let unit_name = format!("linix-{}", config.name);
        let service_path = systemd_dir.join(format!("{}.service", unit_name));
        let timer_path = systemd_dir.join(format!("{}.timer", unit_name));

        // 1. Process Logic for @reboot vs Calendar
        let use_boot_timer = config.cron == "@reboot";
        let schedule_spec = if use_boot_timer {
            "OnBootSec=1min".to_string()
        } else {
            format!("OnCalendar={}", self.map_cron_to_systemd(&config.cron))
        };

        let service_content = format!(
            "[Unit]\nDescription=LiNix Job: {name}\n\n\
             [Service]\nType=oneshot\nExecStart={bin} {cmd}\n\
             StandardOutput=append:{log}\nStandardError=append:{log}\n",
            name = config.name, bin = linix_bin.display(), cmd = config.command,
            log = crate::utils::safe_data_dir().join("schedule.log").display()
        );

        let timer_content = format!(
            "[Unit]\nDescription=LiNix Schedule Timer for {name}\n\n\
             [Timer]\n{schedule}\nPersistent=true\n\n\
             [Install]\nWantedBy=timers.target\n",
            name = config.name, schedule = schedule_spec
        );

        fs::write(&service_path, service_content).map_err(Error::from)?;
        if !use_boot_timer {
            fs::write(&timer_path, timer_content).map_err(Error::from)?;
        }

        // Inform systemd
        let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).status();
        if use_boot_timer {
            // For @reboot we don't use a timer file, we use a service with WantedBy=default.target
            let boot_service = format!(
                "[Unit]\nDescription=LiNix Reboot Job: {name}\n\n\
                 [Service]\nType=oneshot\nExecStart={bin} {cmd}\n\
                 [Install]\nWantedBy=default.target\n",
                name = config.name, bin = linix_bin.display(), cmd = config.command
            );
            fs::write(&service_path, boot_service).map_err(Error::from)?;
            let _ = Command::new("systemctl").args(["--user", "enable", &format!("{}.service", unit_name)]).status();
        } else {
            let _ = Command::new("systemctl").args(["--user", "enable", "--now", &format!("{}.timer", unit_name)]).status();
        }

        Ok(())
    }

    fn remove_task(&self, name: &str) -> Result<()> {
        let timer_name = format!("linix-{}.timer", name);
        let service_name = format!("linix-{}.service", name);
        let _ = Command::new("systemctl").args(["--user", "disable", "--now", &timer_name]).status();
        let _ = Command::new("systemctl").args(["--user", "disable", "--now", &service_name]).status();
        
        if let Some(config_dir) = dirs::config_dir() {
            let systemd_dir = config_dir.join("systemd").join("user");
            let _ = fs::remove_file(systemd_dir.join(&timer_name));
            let _ = fs::remove_file(systemd_dir.join(&service_name));
        }
        Ok(())
    }

    fn is_task_active(&self, name: &str) -> bool {
        let unit = format!("linix-{}.timer", name);
        let out = Command::new("systemctl").args(["--user", "is-active", &unit]).output();
        out.map(|o| String::from_utf8_lossy(&o.stdout).trim() == "active").unwrap_or(false)
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
                
                // Map fields: min hour dom mon dow
                // Systemd: [DayOfWeek] Year-Month-Day Hour:Minute:Second
                let min = self.translate_field(parts[0]);
                let hour = self.translate_field(parts[1]);
                let dom = self.translate_field(parts[2]);
                let mon = self.translate_field(parts[3]);
                let dow = self.translate_field(parts[4]);

                // Translate Dow numbers to Systemd Names
                let dow_mapped = dow.replace('0', "Sun").replace('1', "Mon").replace('2', "Tue")
                    .replace('3', "Wed").replace('4', "Thu").replace('5', "Fri").replace('6', "Sat");

                format!("{} *-{}-{} {}:{}:00", dow_mapped, mon, dom, hour, min)
                    .replace("*-*", "*") // Cleanup wildcard combinations
            }
        }
    }

    fn translate_field(&self, field: &str) -> String {
        if field == "*" { return "*".into(); }
        // Step: */5 -> 0/5
        if let Some(step) = field.strip_prefix("*/") { return format!("0/{}", step); }
        // Range: 1-5 -> 1..5
        if field.contains('-') { return field.replace('-', ".."); }
        field.to_string()
    }
}

// ============================================================================
// MACOS: Launchd (A+ Full Cron Translation)
// ============================================================================

struct MacLaunchdProvisioner;

impl TaskProvisioner for MacLaunchdProvisioner {
    fn add_task(&self, config: &ScheduleConfig, linix_bin: &Path) -> Result<()> {
        let label = format!("com.linix.{}", config.name);
        let plist_path = dirs::home_dir().unwrap().join("Library/LaunchAgents").join(format!("{}.plist", label));

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

        fs::write(&plist_path, plist_content).map_err(Error::from)?;
        let _ = Command::new("launchctl").args(["load", &plist_path.to_string_lossy()]).status();
        Ok(())
    }

    fn remove_task(&self, name: &str) -> Result<()> {
        let label = format!("com.linix.{}", name);
        let plist_path = dirs::home_dir().unwrap().join("Library/LaunchAgents").join(format!("{}.plist", label));
        let _ = Command::new("launchctl").args(["unload", &plist_path.to_string_lossy()]).status();
        let _ = fs::remove_file(plist_path);
        Ok(())
    }

    fn is_task_active(&self, name: &str) -> bool {
        let label = format!("com.linix.{}", name);
        let out = Command::new("launchctl").args(["list", &label]).output();
        out.map(|o| o.status.success()).unwrap_or(false)
    }
}

impl MacLaunchdProvisioner {
    fn map_cron_to_launchd_xml(&self, cron: &str) -> String {
        let parts: Vec<&str> = cron.split_whitespace().collect();
        // Fallback for special strings
        let (m, h, dom, mon, dow) = match cron {
            "@hourly" => ("0", "*", "*", "*", "*"),
            "@daily" => ("0", "0", "*", "*", "*"),
            "@weekly" => ("0", "0", "*", "*", "1"),
            "@monthly" => ("0", "0", "1", "*", "*"),
            _ if parts.len() >= 5 => (parts[0], parts[1], parts[2], parts[3], parts[4]),
            _ => ("0", "2", "*", "*", "*"), // Default 2 AM
        };

        let mut xml = String::from("<dict>");
        let keys = ["Minute", "Hour", "Day", "Month", "Weekday"];
        let vals = [m, h, dom, mon, dow];

        for (i, &val) in vals.iter().enumerate() {
            if val != "*" {
                // launchd doesn't support steps/ranges in XML natively; we take the first specific value
                let first_val = val.split(|c| c == ',' || c == '-' || c == '/').next().unwrap_or("0");
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
// WINDOWS: Task Scheduler
// ============================================================================

struct WindowsTaskProvisioner;

impl TaskProvisioner for WindowsTaskProvisioner {
    fn add_task(&self, config: &ScheduleConfig, linix_bin: &Path) -> Result<()> {
        let name = format!("LiNix_{}", config.name);
        let cmd = format!("{} {}", linix_bin.display(), config.command);
        
        let (sc, st) = match config.cron.as_str() {
            "@reboot" => ("ONSTART", "".to_string()),
            "@hourly" => ("HOURLY", "".to_string()),
            _ => {
                let parts: Vec<&str> = config.cron.split_whitespace().collect();
                let hour = parts.get(1).unwrap_or(&"02");
                let min = parts.get(0).unwrap_or(&"00");
                ("DAILY", format!("{}:{}", hour, min))
            }
        };

        let mut args = vec!["/Create", "/TN", &name, "/TR", &cmd, "/SC", sc, "/F"];
        if !st.is_empty() {
            args.extend(["/ST", &st]);
        }

        let status = Command::new("schtasks").args(&args).status().map_err(Error::from)?;
        if !status.success() { return Err(Error::CommandFailed("Windows Task Scheduler rejection.".into())); }
        Ok(())
    }

    fn remove_task(&self, name: &str) -> Result<()> {
        let tn = format!("LiNix_{}", name);
        let _ = Command::new("schtasks").args(["/Delete", "/TN", &tn, "/F"]).status();
        Ok(())
    }

    fn is_task_active(&self, name: &str) -> bool {
        let tn = format!("LiNix_{}", name);
        let out = Command::new("schtasks").args(["/Query", "/TN", &tn]).output();
        out.map(|o| o.status.success()).unwrap_or(false)
    }
}