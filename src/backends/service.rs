use crate::core::{
    BackendCore, CommandExecutor, Installable, MetadataProvider, Package, PackageSpec, Queryable,
    Result,
};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;

/// Core backend implementation for system services across platforms.
pub struct ServiceBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl ServiceBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "service".to_string(),
        }
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

#[async_trait]
impl BackendCore for ServiceBackendCore {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        if let Some(cmd) = self.get_init_command() {
            self.executor.command_exists_sync(cmd)
        } else {
            false
        }
    }

    fn needs_root(&self) -> bool {
        // System service management requires root/administrative privileges.
        true
    }
}

#[async_trait]
impl MetadataProvider for ServiceBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        // Services handle their own unit dependencies; LiNix manages state only.
        Ok(vec![])
    }
}

pub struct ServiceInstallable {
    pub core: Arc<ServiceBackendCore>,
}

#[async_trait]
impl Installable for ServiceInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        for spec in specs {
            let status = spec
                .options
                .get("status")
                .map(|s| s.as_str())
                .unwrap_or("running");
            let enabled = spec
                .options
                .get("enabled")
                .map(|s| s == "true")
                .unwrap_or(true);

            #[cfg(target_os = "linux")]
            {
                let action = if enabled { "enable" } else { "disable" };
                self.core
                    .executor
                    .run("systemctl", &[action, &spec.name], sudo)
                    .await?;

                let state_cmd = if status == "running" { "start" } else { "stop" };
                self.core
                    .executor
                    .run("systemctl", &[state_cmd, &spec.name], sudo)
                    .await?;
            }

            #[cfg(target_os = "macos")]
            {
                let cmd = if status == "running" {
                    "load"
                } else {
                    "unload"
                };
                self.core
                    .executor
                    .run("launchctl", &[cmd, "-w", &spec.name], sudo)
                    .await?;
            }

            #[cfg(target_os = "windows")]
            {
                let start_type = if enabled { "auto" } else { "disabled" };
                self.core
                    .executor
                    .run("sc", &["config", &spec.name, "start=", start_type], sudo)
                    .await?;

                let state_cmd = if status == "running" { "start" } else { "stop" };
                self.core
                    .executor
                    .run("sc", &[state_cmd, &spec.name], sudo)
                    .await?;
            }

            info!(
                "Service {}: Set to {} (enabled={})",
                spec.name, status, enabled
            );
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        for name in names {
            #[cfg(target_os = "linux")]
            {
                let _ = self
                    .core
                    .executor
                    .run("systemctl", &["stop", name], sudo)
                    .await;
                let _ = self
                    .core
                    .executor
                    .run("systemctl", &["disable", name], sudo)
                    .await;
            }
            #[cfg(target_os = "macos")]
            {
                let _ = self
                    .core
                    .executor
                    .run("launchctl", &["unload", "-w", name], sudo)
                    .await;
            }
            #[cfg(target_os = "windows")]
            {
                let _ = self.core.executor.run("sc", &["stop", name], sudo).await;
                let _ = self
                    .core
                    .executor
                    .run("sc", &["config", name, "start=", "disabled"], sudo)
                    .await;
            }
        }
        Ok(())
    }
}

pub struct ServiceQueryable {
    pub core: Arc<ServiceBackendCore>,
}

#[async_trait]
impl Queryable for ServiceQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let mut pkgs = Vec::new();

        #[cfg(target_os = "linux")]
        {
            let out = self
                .core
                .executor
                .run_output(
                    "systemctl",
                    &[
                        "list-units",
                        "--type=service",
                        "--state=running",
                        "--no-legend",
                    ],
                    false,
                )
                .await?;
            for line in out.lines() {
                if let Some(name) = line.split_whitespace().next() {
                    pkgs.push(Package::new(name.trim_end_matches(".service"), "service"));
                }
            }
        }

        #[cfg(target_os = "macos")]
        {
            let out = self
                .core
                .executor
                .run_output("launchctl", &["list"], false)
                .await?;
            for line in out.lines().skip(1) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(label) = parts.get(2) {
                    pkgs.push(Package::new(*label, "service"));
                }
            }
        }

        #[cfg(target_os = "windows")]
        {
            let out = self
                .core
                .executor
                .run_output(
                    "sc",
                    &["query", "type=", "service", "state=", "active"],
                    false,
                )
                .await?;
            for line in out.lines() {
                if let Some(v) = line.strip_prefix("SERVICE_NAME: ") {
                    pkgs.push(Package::new(v.trim(), "service"));
                }
            }
        }

        Ok(pkgs)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    /// FIX: Hardening Phase 5.2. No longer greedy.
    /// Verifies existence before returning metadata.
    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let installed = self.list_installed().await?;
        if let Some(mut pkg) = installed.into_iter().find(|p| p.name == name) {
            self.fill_platform_metadata(&mut pkg).await?;
            return Ok(Some(pkg));
        }
        Ok(None)
    }
}

impl ServiceQueryable {
    #[allow(unused_variables)] // `p` is used only on linux/windows metadata paths
    async fn fill_platform_metadata(&self, p: &mut Package) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            if let Ok(out) = self
                .core
                .executor
                .run_output("systemctl", &["status", &p.name], false)
                .await
            {
                p.properties.insert("status_raw".into(), out);
            }
        }

        #[cfg(target_os = "windows")]
        {
            if let Ok(out) = self
                .core
                .executor
                .run_output("sc", &["qc", &p.name], false)
                .await
            {
                p.properties.insert("config_raw".into(), out);
            }
        }

        Ok(())
    }
}

/// Build and register the system-service backend.
pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let core = Arc::new(ServiceBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(ServiceInstallable { core: core.clone() }))
            .with_queryable(Arc::new(ServiceQueryable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}
