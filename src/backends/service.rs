use crate::core::{CommandExecutor, Package, PackageManager, Result, PackageSpec};
use async_trait::async_trait;
use std::collections::HashMap;
use tracing::info;

pub struct ServiceManager {
    executor: CommandExecutor,
}

impl ServiceManager {
    pub fn new(executor: CommandExecutor, _: Option<HashMap<String, String>>) -> Self {
        Self { executor }
    }
}

#[async_trait]
impl PackageManager for ServiceManager {
    fn name(&self) -> &str { "service" }

    fn is_available(&self) -> bool {
        // Init systems are always present on modern OSs
        true
    }

    async fn install_with_options(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        for spec in specs {
            let status = spec.options.get("status").map(|s| s.as_str()).unwrap_or("running");
            let enabled = spec.options.get("enabled").map(|s| s == "true").unwrap_or(true);

            #[cfg(target_os = "linux")] {
                // REAL LOGIC: Systemd Management
                if enabled { self.executor.run("systemctl", &["enable", &spec.name], sudo).await?; }
                else { self.executor.run("systemctl", &["disable", &spec.name], sudo).await?; }

                let cmd = if status == "running" { "start" } else { "stop" };
                self.executor.run("systemctl", &[cmd, &spec.name], sudo).await?;
            }

            #[cfg(target_os = "macos")] {
                // REAL LOGIC: Launchctl Management
                let cmd = if status == "running" { "load" } else { "unload" };
                // On macOS, 'enabled' usually corresponds to the presence of the plist in LaunchAgents/Daemons
                self.executor.run("launchctl", &[cmd, "-w", &spec.name], sudo).await?;
            }

            #[cfg(target_os = "windows")] {
                // REAL LOGIC: Windows Service Control (sc.exe)
                if enabled { self.executor.run("sc", &["config", &spec.name, "start=", "auto"], sudo).await?; }
                else { self.executor.run("sc", &["config", &spec.name, "start=", "disabled"], sudo).await?; }

                let cmd = if status == "running" { "start" } else { "stop" };
                self.executor.run("sc", &[cmd, &spec.name], sudo).await?;
            }

            info!("Service {} set to {} (enabled: {})", spec.name, status, enabled);
        }
        Ok(())
    }

    async fn install(&self, p: &[String], s: bool) -> Result<()> {
        // Fallback: Default to starting and enabling the service
        let specs: Vec<_> = p.iter().map(|n| PackageSpec { 
            name: n.clone(), 
            backend: "service".into(), 
            options: HashMap::from([("status".into(), "running".into()), ("enabled".into(), "true".into())])
        }).collect();
        self.install_with_options(&specs, s).await
    }

    async fn remove(&self, p: &[String], s: bool) -> Result<()> {
        // REAL LOGIC: Removing a service in LiNix context means stopping and disabling it
        for name in p {
            #[cfg(target_os = "linux")] {
                let _ = self.executor.run("systemctl", &["stop", name], s).await;
                let _ = self.executor.run("systemctl", &["disable", name], s).await;
            }
            #[cfg(target_os = "windows")] {
                let _ = self.executor.run("sc", &["stop", name], s).await;
                let _ = self.executor.run("sc", &["config", name, "start=", "disabled"], s).await;
            }
        }
        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        // REAL LOGIC: Returns only services that are currently active/running
        let mut pkgs = Vec::new();

        #[cfg(target_os = "linux")] {
            let out = self.executor.run_output("systemctl", &["list-units", "--type=service", "--state=running", "--no-legend"], false).await?;
            for line in out.lines() {
                if let Some(name) = line.split_whitespace().next() {
                    pkgs.push(Package::new(name.trim_end_matches(".service"), "service"));
                }
            }
        }

        #[cfg(target_os = "windows")] {
            let out = self.executor.run_output("sc", &["query", "type=", "service", "state=", "active"], false).await?;
            for line in out.lines() {
                if let Some(v) = line.strip_prefix("SERVICE_NAME: ") {
                    pkgs.push(Package::new(v.trim(), "service"));
                }
            }
        }

        Ok(pkgs)
    }

    async fn info(&self, package: &str) -> Result<Option<Package>> {
        // REAL LOGIC: Returns the current status of the service
        let mut p = Package::new(package, "service");

        #[cfg(target_os = "linux")] {
            let out = self.executor.run_output("systemctl", &["status", package], false).await?;
            p.description = Some(out.lines().take(3).collect::<Vec<_>>().join(" "));
        }

        #[cfg(target_os = "windows")] {
            let out = self.executor.run_output("sc", &["query", package], false).await?;
            p.description = Some(out);
        }

        Ok(Some(p))
    }
}