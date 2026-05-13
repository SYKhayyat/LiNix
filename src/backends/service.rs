use crate::core::{
    Backend, CommandExecutor, Installable, Package, PackageSpec, 
    Queryable, Result
};
use async_trait::async_trait;
use std::collections::HashMap;
use tracing::{info, warn, debug};

/// Manages system services across Linux (systemd), macOS (launchctl), and Windows (sc.exe).
/// In the LiNix declarative model, a 'service' is treated as a package type where 
/// 'install' ensures the service is enabled/running, and 'remove' ensures it is stopped/disabled.
pub struct ServiceManager {
    executor: CommandExecutor,
}

impl ServiceManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self { executor }
    }

    /// Determines the native service management command for the current platform.
    fn get_init_command(&self) -> Option<&'static str> {
        if cfg!(target_os = "linux") {
            Some("systemctl")
        } else if cfg!(target_os = "macos") {
            Some("launchctl")
        } else if cfg!(target_os = "windows") {
            Some("sc")
        } else {
            None
        }
    }
}

impl Backend for ServiceManager {
    fn name(&self) -> &str { "service" }

    fn is_available(&self) -> bool {
        if let Some(cmd) = self.get_init_command() {
            self.executor.command_exists_sync(cmd)
        } else {
            false
        }
    }

    fn as_installable(&self) -> Option<&dyn Installable> { Some(self) }
    fn as_queryable(&self) -> Option<&dyn Queryable> { Some(self) }
}

#[async_trait]
impl Installable for ServiceManager {
    /// Aligns the system service state with the desired PackageSpec.
    /// Supports options: 
    /// - @status=running|stopped
    /// - @enabled=true|false
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        for spec in specs {
            let status = spec.options.get("status").map(|s| s.as_str()).unwrap_or("running");
            let enabled = spec.options.get("enabled").map(|s| s == "true").unwrap_or(true);

            #[cfg(target_os = "linux")]
            {
                // Manage boot persistence (enabled/disabled)
                let action = if enabled { "enable" } else { "disable" };
                self.executor.run("systemctl", &[action, &spec.name], sudo).await?;

                // Manage current runtime state (started/stopped)
                let state_cmd = if status == "running" { "start" } else { "stop" };
                self.executor.run("systemctl", &[state_cmd, &spec.name], sudo).await?;
            }

            #[cfg(target_os = "macos")]
            {
                // launchctl uses load/unload -w to manage persistence and state simultaneously.
                let cmd = if status == "running" { "load" } else { "unload" };
                // Assumption: spec.name refers to a valid agent/daemon label or plist path.
                self.executor.run("launchctl", &[cmd, "-w", &spec.name], sudo).await?;
            }

            #[cfg(target_os = "windows")]
            {
                // sc.exe config manages the start type (auto/disabled)
                let start_type = if enabled { "auto" } else { "disabled" };
                self.executor.run("sc", &["config", &spec.name, "start=", start_type], sudo).await?;

                // sc.exe start/stop manages the immediate runtime state
                let state_cmd = if status == "running" { "start" } else { "stop" };
                self.executor.run("sc", &[state_cmd, &spec.name], sudo).await?;
            }

            info!("Service {}: Set to {} (enabled={})", spec.name, status, enabled);
        }
        Ok(())
    }

    /// Ensures services are stopped and disabled on the host.
    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        for name in names {
            info!("Ensuring service is stopped and disabled: {}", name);
            #[cfg(target_os = "linux")]
            {
                let _ = self.executor.run("systemctl", &["stop", name], sudo).await;
                let _ = self.executor.run("systemctl", &["disable", name], sudo).await;
            }
            #[cfg(target_os = "macos")]
            {
                let _ = self.executor.run("launchctl", &["unload", "-w", name], sudo).await;
            }
            #[cfg(target_os = "windows")]
            {
                let _ = self.executor.run("sc", &["stop", name], sudo).await;
                let _ = self.executor.run("sc", &["config", name, "start=", "disabled"], sudo).await;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Queryable for ServiceManager {
    /// Discovers all services currently in an active/running state on the host.
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let mut pkgs = Vec::new();

        #[cfg(target_os = "linux")]
        {
            let out = self.executor.run_output("systemctl", &["list-units", "--type=service", "--state=running", "--no-legend"], false).await?;
            for line in out.lines() {
                if let Some(name) = line.split_whitespace().next() {
                    pkgs.push(Package::new(name.trim_end_matches(".service"), "service"));
                }
            }
        }

        #[cfg(target_os = "macos")]
        {
            // launchctl list output format: "PID Status Label"
            let out = self.executor.run_output("launchctl", &["list"], false).await?;
            for line in out.lines().skip(1) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(label) = parts.get(2) {
                    pkgs.push(Package::new(*label, "service"));
                }
            }
        }

        #[cfg(target_os = "windows")]
        {
            // sc query type= service state= active
            let out = self.executor.run_output("sc", &["query", "type=", "service", "state=", "active"], false).await?;
            for line in out.lines() {
                if let Some(v) = line.strip_prefix("SERVICE_NAME: ") {
                    pkgs.push(Package::new(v.trim(), "service"));
                }
            }
        }

        Ok(pkgs)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        // Active services are treated as managed state candidates.
        self.list_installed().await
    }

    /// Fetches platform-specific status details for a specific service.
    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let mut p = Package::new(name, "service");

        #[cfg(target_os = "linux")]
        {
            if let Ok(out) = self.executor.run_output("systemctl", &["status", name], false).await {
                p.properties.insert("status_raw".into(), out);
            }
        }

        #[cfg(target_os = "windows")]
        {
            if let Ok(out) = self.executor.run_output("sc", &["qc", name], false).await {
                p.properties.insert("config_raw".into(), out);
            }
        }

        Ok(Some(p))
    }
}