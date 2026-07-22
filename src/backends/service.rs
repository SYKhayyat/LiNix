use crate::core::{
    BackendCore, CommandExecutor, Installable, MetadataProvider, Package, PackageSpec, Queryable,
    Result,
};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    Enable,
    Disable,
    Start,
    Stop,
    Restart,
}

/// The init / service-management system driving the host. This is what lets the one
/// `service` backend speak systemd, OpenRC, SysVinit, launchd and Windows `sc` alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitSystem {
    Systemd,
    OpenRc,
    SysVinit,
    Launchd,
    WindowsSc,
    Unknown,
}

/// Only systemd takes the unit as a trailing positional; every other init here puts the name
/// between two positionals, which leaves no place a terminator could go.
fn systemctl(verb: &str, name: &str) -> Vec<(String, Vec<String>)> {
    let mut args = vec![verb.to_string()];
    crate::core::argv::push_names(&mut args, "systemctl", [name]);
    vec![("systemctl".to_string(), args)]
}

/// Pure: the ordered list of `(program, args)` commands that realize `action` for a service
/// named `name` under a given init system. Empty when the platform can't express the action.
/// Kept free of I/O so every mapping is unit-testable.
pub fn plan_service(
    init: InitSystem,
    action: ServiceAction,
    name: &str,
) -> Vec<(String, Vec<String>)> {
    let s = name.to_string();
    let one = |prog: &str, args: &[&str]| {
        vec![(
            prog.to_string(),
            args.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
        )]
    };
    match (init, action) {
        (InitSystem::Systemd, ServiceAction::Enable) => systemctl("enable", name),
        (InitSystem::Systemd, ServiceAction::Disable) => systemctl("disable", name),
        (InitSystem::Systemd, ServiceAction::Start) => systemctl("start", name),
        (InitSystem::Systemd, ServiceAction::Stop) => systemctl("stop", name),
        (InitSystem::Systemd, ServiceAction::Restart) => systemctl("restart", name),

        (InitSystem::OpenRc, ServiceAction::Enable) => one("rc-update", &["add", &s, "default"]),
        (InitSystem::OpenRc, ServiceAction::Disable) => one("rc-update", &["del", &s, "default"]),
        (InitSystem::OpenRc, ServiceAction::Start) => one("rc-service", &[&s, "start"]),
        (InitSystem::OpenRc, ServiceAction::Stop) => one("rc-service", &[&s, "stop"]),
        (InitSystem::OpenRc, ServiceAction::Restart) => one("rc-service", &[&s, "restart"]),

        (InitSystem::SysVinit, ServiceAction::Enable) => one("update-rc.d", &[&s, "enable"]),
        (InitSystem::SysVinit, ServiceAction::Disable) => one("update-rc.d", &[&s, "disable"]),
        (InitSystem::SysVinit, ServiceAction::Start) => one("service", &[&s, "start"]),
        (InitSystem::SysVinit, ServiceAction::Stop) => one("service", &[&s, "stop"]),
        (InitSystem::SysVinit, ServiceAction::Restart) => one("service", &[&s, "restart"]),

        (InitSystem::Launchd, ServiceAction::Enable) => one("launchctl", &["load", "-w", &s]),
        (InitSystem::Launchd, ServiceAction::Disable) => one("launchctl", &["unload", "-w", &s]),
        (InitSystem::Launchd, ServiceAction::Start) => one("launchctl", &["start", &s]),
        (InitSystem::Launchd, ServiceAction::Stop) => one("launchctl", &["stop", &s]),
        (InitSystem::Launchd, ServiceAction::Restart) => {
            vec![
                ("launchctl".into(), vec!["stop".into(), s.clone()]),
                ("launchctl".into(), vec!["start".into(), s]),
            ]
        }

        (InitSystem::WindowsSc, ServiceAction::Enable) => {
            one("sc", &["config", &s, "start=", "auto"])
        }
        (InitSystem::WindowsSc, ServiceAction::Disable) => {
            one("sc", &["config", &s, "start=", "disabled"])
        }
        (InitSystem::WindowsSc, ServiceAction::Start) => one("sc", &["start", &s]),
        (InitSystem::WindowsSc, ServiceAction::Stop) => one("sc", &["stop", &s]),
        (InitSystem::WindowsSc, ServiceAction::Restart) => {
            vec![
                ("sc".into(), vec!["stop".into(), s.clone()]),
                ("sc".into(), vec!["start".into(), s]),
            ]
        }

        (InitSystem::Unknown, _) => Vec::new(),
    }
}

/// Pure: translate the declarative `enabled` / `status` options on a spec into the ordered
/// list of actions to apply. When neither is given, default to "enable + start" (the common
/// intent of listing a service in a manifest). `status=restarted` maps to Restart.
pub fn actions_for(enabled: Option<&str>, status: Option<&str>) -> Vec<ServiceAction> {
    let mut acts = Vec::new();
    match enabled {
        Some(v) if v == "true" || v == "yes" || v == "1" => acts.push(ServiceAction::Enable),
        Some(_) => acts.push(ServiceAction::Disable),
        None => {}
    }
    match status {
        Some("running") | Some("started") | Some("start") => acts.push(ServiceAction::Start),
        Some("stopped") | Some("stop") => acts.push(ServiceAction::Stop),
        Some("restarted") | Some("restart") => acts.push(ServiceAction::Restart),
        Some(_) | None => {}
    }
    if enabled.is_none() && status.is_none() {
        acts.push(ServiceAction::Enable);
        acts.push(ServiceAction::Start);
    }
    acts
}


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

    /// Detect the host's init/service system, probing for the right tool on Linux where
    /// several coexist across distros.
    pub fn detect_init(&self) -> InitSystem {
        if cfg!(target_os = "windows") {
            InitSystem::WindowsSc
        } else if cfg!(target_os = "macos") {
            InitSystem::Launchd
        } else if cfg!(target_os = "linux") {
            if self.executor.command_exists_sync("systemctl") {
                InitSystem::Systemd
            } else if self.executor.command_exists_sync("rc-service") {
                InitSystem::OpenRc
            } else if self.executor.command_exists_sync("service") {
                InitSystem::SysVinit
            } else {
                InitSystem::Unknown
            }
        } else {
            InitSystem::Unknown
        }
    }

    /// Run the concrete commands for one action, propagating the first failure.
    async fn apply(&self, action: ServiceAction, name: &str, sudo: bool) -> Result<()> {
        let init = self.detect_init();
        for (prog, args) in plan_service(init, action, name) {
            let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            self.executor.run(&prog, &arg_refs, sudo).await?;
        }
        Ok(())
    }

    /// Best-effort variant used on removal, where a service that is already gone must not
    /// abort the teardown.
    async fn apply_lenient(&self, action: ServiceAction, name: &str, sudo: bool) {
        let init = self.detect_init();
        for (prog, args) in plan_service(init, action, name) {
            let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            let _ = self.executor.run(&prog, &arg_refs, sudo).await;
        }
    }
}

#[async_trait]
impl BackendCore for ServiceBackendCore {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        self.detect_init() != InitSystem::Unknown
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
            let enabled = spec.options.get("enabled").map(|s| s.as_str());
            let status = spec.options.get("status").map(|s| s.as_str());
            let actions = actions_for(enabled, status);
            for action in &actions {
                self.core.apply(*action, &spec.name, sudo).await?;
            }
            info!(
                "Service {}: applied {:?} (init={:?})",
                spec.name,
                actions,
                self.core.detect_init()
            );
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        for name in names {
            // Stop then disable; never let a missing service abort the sweep.
            self.core
                .apply_lenient(ServiceAction::Stop, name, sudo)
                .await;
            self.core
                .apply_lenient(ServiceAction::Disable, name, sudo)
                .await;
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

        match self.core.detect_init() {
            InitSystem::Systemd => {
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
            InitSystem::OpenRc => {
                if let Ok(out) = self
                    .core
                    .executor
                    .run_output("rc-status", &["--servicelist"], false)
                    .await
                {
                    for line in out.lines() {
                        if let Some(name) = line.split_whitespace().next() {
                            pkgs.push(Package::new(name, "service"));
                        }
                    }
                }
            }
            InitSystem::Launchd => {
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
            InitSystem::WindowsSc => {
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
            InitSystem::SysVinit | InitSystem::Unknown => {}
        }

        Ok(pkgs)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

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
    async fn fill_platform_metadata(&self, p: &mut Package) -> Result<()> {
        let one = |prog: &str, args: &[&str]| {
            (
                prog.to_string(),
                args.iter().map(|a| a.to_string()).collect::<Vec<String>>(),
            )
        };
        let (prog, args): (String, Vec<String>) = match self.core.detect_init() {
            InitSystem::Systemd => {
                let (_, args) = systemctl("status", &p.name).remove(0);
                ("systemctl".to_string(), args)
            }
            InitSystem::OpenRc => one("rc-service", &[&p.name, "status"]),
            InitSystem::SysVinit => one("service", &[&p.name, "status"]),
            InitSystem::WindowsSc => one("sc", &["qc", &p.name]),
            InitSystem::Launchd | InitSystem::Unknown => return Ok(()),
        };
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        if let Ok(out) = self.core.executor.run_output(&prog, &args, false).await {
            p.properties.insert("status_raw".into(), out);
        }
        Ok(())
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemd_maps_each_action_and_ends_its_options_before_the_unit() {
        for (action, verb) in [
            (ServiceAction::Enable, "enable"),
            (ServiceAction::Disable, "disable"),
            (ServiceAction::Start, "start"),
            (ServiceAction::Stop, "stop"),
            (ServiceAction::Restart, "restart"),
        ] {
            assert_eq!(
                plan_service(InitSystem::Systemd, action, "nginx"),
                vec![(
                    "systemctl".to_string(),
                    vec![verb.into(), "--".into(), "nginx".into()]
                )]
            );
        }
    }

    /// These inits take the service between two positionals, so there is nowhere a `--`
    /// could go — and each of them would read it as the service name.
    #[test]
    fn the_other_inits_deliberately_emit_no_terminator() {
        for init in [
            InitSystem::OpenRc,
            InitSystem::SysVinit,
            InitSystem::Launchd,
            InitSystem::WindowsSc,
        ] {
            for action in [
                ServiceAction::Enable,
                ServiceAction::Disable,
                ServiceAction::Start,
                ServiceAction::Stop,
                ServiceAction::Restart,
            ] {
                for (prog, args) in plan_service(init, action, "nginx") {
                    assert!(
                        !args.iter().any(|a| a == "--"),
                        "{:?}/{:?} emitted a terminator to `{}`",
                        init,
                        action,
                        prog
                    );
                }
            }
        }
    }

    #[test]
    fn openrc_uses_rc_update_and_rc_service() {
        assert_eq!(
            plan_service(InitSystem::OpenRc, ServiceAction::Enable, "sshd"),
            vec![(
                "rc-update".to_string(),
                vec!["add".into(), "sshd".into(), "default".into()]
            )]
        );
        assert_eq!(
            plan_service(InitSystem::OpenRc, ServiceAction::Start, "sshd"),
            vec![(
                "rc-service".to_string(),
                vec!["sshd".into(), "start".into()]
            )]
        );
    }

    #[test]
    fn windows_restart_is_stop_then_start() {
        let cmds = plan_service(InitSystem::WindowsSc, ServiceAction::Restart, "W32Time");
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].1[0], "stop");
        assert_eq!(cmds[1].1[0], "start");
    }

    #[test]
    fn launchd_restart_is_stop_then_start() {
        let cmds = plan_service(InitSystem::Launchd, ServiceAction::Restart, "com.foo");
        assert_eq!(cmds.len(), 2);
    }

    #[test]
    fn unknown_init_plans_nothing() {
        assert!(plan_service(InitSystem::Unknown, ServiceAction::Start, "x").is_empty());
    }

    #[test]
    fn options_default_to_enable_and_start() {
        assert_eq!(
            actions_for(None, None),
            vec![ServiceAction::Enable, ServiceAction::Start]
        );
    }

    #[test]
    fn options_are_independent() {
        // status only -> no enable/disable touched
        assert_eq!(
            actions_for(None, Some("running")),
            vec![ServiceAction::Start]
        );
        assert_eq!(
            actions_for(None, Some("stopped")),
            vec![ServiceAction::Stop]
        );
        // enabled only -> no start/stop
        assert_eq!(actions_for(Some("true"), None), vec![ServiceAction::Enable]);
        assert_eq!(
            actions_for(Some("false"), None),
            vec![ServiceAction::Disable]
        );
        // both
        assert_eq!(
            actions_for(Some("true"), Some("running")),
            vec![ServiceAction::Enable, ServiceAction::Start]
        );
        assert_eq!(
            actions_for(None, Some("restarted")),
            vec![ServiceAction::Restart]
        );
    }
}
