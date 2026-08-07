//! The `service:` backend — one spelling across every init system (U36).
//!
//! **Rows, not Rust.** The init systems LiNix drives — systemd, OpenRC, SysVinit, launchd,
//! Windows `sc` — are rows in `init_providers.toml`, parsed by the same approved loader a user's
//! own `adapters/init.toml` row goes through. s6, dinit, runit, GNU Shepherd and every appliance
//! init were unreachable while this was a closed `enum`; now they are six lines of TOML. The
//! shipped five register first and a user row never shadows one, exactly as `custom_backends.toml`
//! and the firewall adapters do.

use crate::core::adapter::{self, AdapterRow, Detected};
use crate::core::{
    BackendCore, CommandExecutor, Installable, MetadataProvider, Package, PackageSpec, Queryable,
    Result,
};
use async_trait::async_trait;
use serde::Deserialize;
use std::borrow::Cow;
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    Enable,
    Disable,
    Start,
    Stop,
    Restart,
}

/// One init system, wholly as data: how to tell it is here, and the argv for each action.
///
/// Every action is a *sequence* of commands, because some inits have no native restart and
/// express it as stop-then-start (launchd, Windows `sc`). An action a provider cannot express is
/// an empty list, reported by name, never a silent success.
#[derive(Debug, Clone, Deserialize)]
pub struct InitProvider {
    pub name: String,
    /// The command whose presence means this init drives the host.
    pub detect: String,
    /// Restrict to one OS (`std::env::consts::OS`). Absent means any.
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub enable: Vec<Vec<String>>,
    #[serde(default)]
    pub disable: Vec<Vec<String>>,
    #[serde(default)]
    pub start: Vec<Vec<String>>,
    #[serde(default)]
    pub stop: Vec<Vec<String>>,
    /// A native restart, if the init has one. Empty means "stop then start", derived from the
    /// two required actions so a niche init need not spell it out.
    #[serde(default)]
    pub restart: Vec<Vec<String>>,
    /// How to list running services, for drift. Absent means this init cannot report them, which
    /// is a stated limit, not a claim that nothing runs.
    #[serde(default)]
    pub list: Vec<String>,
    /// A regex whose first capture group is the service name on each `list` line.
    #[serde(default)]
    pub list_pattern: Option<String>,
    /// Header lines to skip before parsing `list` output (launchd prints one).
    #[serde(default)]
    pub list_skip_lines: usize,
    /// A suffix to strip off each listed name (systemd's `.service`).
    #[serde(default)]
    pub list_strip_suffix: Option<String>,
    /// How to read one service's status, for `info`. Optional.
    #[serde(default)]
    pub status: Vec<String>,
    /// How to list the services this machine starts on its own — `adopt --enabled-only`.
    ///
    /// One command, not one per service: reading a start type per name is 150 process spawns
    /// on the host this was measured on. A provider that cannot answer in one command leaves
    /// this empty, and `--enabled-only` says so by name rather than falling back to everything.
    #[serde(default)]
    pub list_enabled: Vec<String>,
    /// A regex whose first capture group is the service name on each `list_enabled` line.
    /// Falls back to [`list_pattern`](Self::list_pattern) when absent.
    #[serde(default)]
    pub list_enabled_pattern: Option<String>,
    /// Exit codes `start` returns when the service is *already running*.
    ///
    /// Being in the state the declaration asks for is what convergence means, so it is a
    /// success and not a failure. Per action rather than per provider: `sc` answers 1056
    /// (`ERROR_SERVICE_ALREADY_RUNNING`) to a start and 1062 (`ERROR_SERVICE_NOT_ACTIVE`) to a
    /// stop, and each of those is a genuine failure on the other verb.
    #[serde(default)]
    pub start_benign_exits: Vec<i32>,
    /// Exit codes `stop` returns when the service is *already stopped*. See
    /// [`start_benign_exits`](Self::start_benign_exits).
    #[serde(default)]
    pub stop_benign_exits: Vec<i32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InitProviderFile {
    #[serde(default)]
    pub init: Vec<InitProvider>,
}

const BUILTIN: &str = include_str!("init_providers.toml");

impl InitProvider {
    fn fill(cmd: &[String], name: &str) -> Vec<String> {
        adapter::fill(cmd, &[("{name}", name)])
    }

    /// The ordered list of concrete commands that realize `action` for `name`, each paired with
    /// the action it carries out. Empty when this init cannot express the action, so the caller
    /// reports "cannot" rather than reporting done.
    ///
    /// A derived restart is a `Stop` followed by a `Start`, and each half must be labelled as
    /// itself: the two halves have opposite ideas of which exit code means "already there".
    pub fn plan(&self, action: ServiceAction, name: &str) -> Vec<(ServiceAction, Vec<String>)> {
        let seq = match action {
            ServiceAction::Enable => &self.enable,
            ServiceAction::Disable => &self.disable,
            ServiceAction::Start => &self.start,
            ServiceAction::Stop => &self.stop,
            ServiceAction::Restart => {
                if self.restart.is_empty() {
                    // Derived stop-then-start for an init with no native restart verb.
                    let mut out: Vec<(ServiceAction, Vec<String>)> = Vec::new();
                    out.extend(
                        self.stop
                            .iter()
                            .map(|c| (ServiceAction::Stop, Self::fill(c, name))),
                    );
                    out.extend(
                        self.start
                            .iter()
                            .map(|c| (ServiceAction::Start, Self::fill(c, name))),
                    );
                    return out;
                }
                &self.restart
            }
        };
        seq.iter().map(|c| (action, Self::fill(c, name))).collect()
    }

    /// The exit codes that mean "already in the state `action` asks for" — success for a
    /// converger. Empty for every action whose init reports that case as exit 0.
    fn benign_exits(&self, action: ServiceAction) -> &[i32] {
        match action {
            ServiceAction::Start => &self.start_benign_exits,
            ServiceAction::Stop => &self.stop_benign_exits,
            // A native restart is not "already there" under any code: it asks for a transition.
            ServiceAction::Restart | ServiceAction::Enable | ServiceAction::Disable => &[],
        }
    }

    /// The running services this init reports, for drift. A line that does not match the pattern
    /// is skipped rather than guessed at — a header or a chain must not become a phantom service.
    fn parse_list(&self, output: &str) -> Vec<Package> {
        self.parse_with(output, self.list_pattern.as_deref())
    }

    /// The same reader over `list_enabled`, which usually differs from `list` only in the
    /// shape of one column.
    fn parse_enabled(&self, output: &str) -> Vec<Package> {
        self.parse_with(
            output,
            self.list_enabled_pattern
                .as_deref()
                .or(self.list_pattern.as_deref()),
        )
    }

    fn parse_with(&self, output: &str, pattern: Option<&str>) -> Vec<Package> {
        let Some(pattern) = pattern else {
            return Vec::new();
        };
        let Ok(re) = crate::utils::regex_cache::compiled(pattern) else {
            warn!(
                "the `{}` init adapter's list_pattern is not a regex",
                self.name
            );
            return Vec::new();
        };
        let mut out = Vec::new();
        for line in output.lines().skip(self.list_skip_lines) {
            let Some(caps) = re.captures(line) else {
                continue;
            };
            let Some(m) = caps.get(1) else { continue };
            let mut name = m.as_str().to_string();
            if let Some(suffix) = &self.list_strip_suffix {
                name = name.trim_end_matches(suffix.as_str()).to_string();
            }
            out.push(Package::new(&name, "service"));
        }
        out
    }
}

impl AdapterRow for InitProvider {
    const WHAT: &'static str = "init adapter";

    fn name(&self) -> &str {
        &self.name
    }

    fn only_on(&self) -> Option<&str> {
        self.os.as_deref()
    }

    /// Start and stop are the floor: an init that cannot do both is half a provider, and a
    /// `service:` line on it would half-apply (U36).
    fn why_unusable(&self) -> Option<&'static str> {
        if self.detect.trim().is_empty() {
            return Some("it has no `detect` command");
        }
        if self.start.is_empty() || self.stop.is_empty() {
            return Some("it cannot both start and stop a service");
        }
        None
    }
}

impl Detected for InitProvider {
    fn detect_command(&self) -> &str {
        &self.detect
    }
}

/// Every init adapter this machine knows: the shipped rows, then the user's. A user row that
/// repeats a shipped name is skipped, so a stray file cannot redefine systemd.
pub fn providers(user_rows: Vec<InitProvider>) -> Vec<InitProvider> {
    let shipped: InitProviderFile =
        toml::from_str(BUILTIN).expect("the shipped init_providers.toml must parse");
    adapter::merge(shipped.init.into_iter().chain(user_rows))
}

/// Translate the declarative `enabled` / `status` options on a spec into the ordered list of
/// actions to apply. When neither is given, default to "enable + start" (the common intent of
/// listing a service in a manifest). `status=restarted` maps to Restart.
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
    providers: Vec<InitProvider>,
}

impl ServiceBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self::with_providers(executor, providers(Vec::new()))
    }

    pub fn with_providers(executor: CommandExecutor, providers: Vec<InitProvider>) -> Self {
        Self {
            executor,
            name: "service".to_string(),
            providers,
        }
    }

    /// The init driving this host: the first adapter that applies to this OS and whose `detect`
    /// command is present. Built-ins are considered before user rows, so a niche row only wins
    /// where no built-in matched.
    pub fn detect_init(&self) -> Option<&InitProvider> {
        adapter::first_present(&self.providers, &|c| self.executor.command_exists_sync(c))
    }

    /// The executor to run one action's command on: this backend's, unless the action has exit
    /// codes that mean "already in that state", which the executor is the only place that can
    /// forgive.
    fn executor_for(&self, init: &InitProvider, action: ServiceAction) -> Cow<'_, CommandExecutor> {
        let benign = init.benign_exits(action);
        if benign.is_empty() {
            return Cow::Borrowed(&self.executor);
        }
        Cow::Owned(
            self.executor
                .duplicate()
                .with_exit_policy(crate::core::ExitPolicy {
                    benign_exits: benign.to_vec(),
                    ..Default::default()
                }),
        )
    }

    /// Run the concrete commands for one action, propagating the first failure.
    async fn apply(&self, action: ServiceAction, name: &str, sudo: bool) -> Result<()> {
        let Some(init) = self.detect_init() else {
            return Ok(());
        };
        for (step, cmd) in init.plan(action, name) {
            let (prog, args) = cmd.split_first().expect("an init command is never empty");
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.executor_for(init, step)
                .run(prog, &arg_refs, sudo)
                .await?;
        }
        Ok(())
    }

    /// Best-effort variant used on removal, where a service that is already gone must not abort
    /// the teardown.
    async fn apply_lenient(&self, action: ServiceAction, name: &str, sudo: bool) {
        let Some(init) = self.detect_init() else {
            return;
        };
        for (step, cmd) in init.plan(action, name) {
            let Some((prog, args)) = cmd.split_first() else {
                continue;
            };
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let _ = self
                .executor_for(init, step)
                .run(prog, &arg_refs, sudo)
                .await;
        }
    }
}

#[async_trait]
impl BackendCore for ServiceBackendCore {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        self.detect_init().is_some()
    }

    /// Every init program this OS could have. Any one of them is enough, which is why the
    /// message names them all rather than claiming which is missing.
    fn probes(&self) -> Vec<String> {
        self.providers
            .iter()
            .filter(|p| p.applies_here())
            .map(|p| p.detect.clone())
            .collect()
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
                "Service {}: applied {:?} (init={})",
                spec.name,
                actions,
                self.core
                    .detect_init()
                    .map(|p| p.name.as_str())
                    .unwrap_or("none"),
            );
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool, _reaped: crate::app::sync::guard::Reaped) -> Result<()> {
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
    fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
        (self.core.executor.installed_listings(), &self.core.name)
    }

    async fn fetch_installed(&self) -> Result<Vec<Package>> {
        let Some(init) = self.core.detect_init() else {
            return Ok(Vec::new());
        };
        if init.list.is_empty() {
            return Ok(Vec::new());
        }
        let (prog, args) = init.list.split_first().expect("list is non-empty here");
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self
            .core
            .executor
            .run_output(prog, &arg_refs, false)
            .await?;
        Ok(init.parse_list(&out))
    }

    /// Every running service. Not "the ones you chose" — no init records that — but a service
    /// that is running is a state a person can read off the machine and decide about, which is
    /// what `adopt` offers. `adoption_options` is what keeps the offer to that claim.
    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    /// **A bare `adopt` does not take services** (owner ruling, 2026-08-05 — `Q39`).
    ///
    /// `manual_source` below has always said why, in its own words: *"no init records which you
    /// chose."* Running is a fact about the machine, not evidence of intent — on the host this
    /// was measured on, `adopt` wrote 150 service lines out of 161 declarations, and two of
    /// them were trigger-start services Windows had already stopped again twenty minutes later.
    /// `linix adopt service` still takes them.
    fn adopted_unasked(&self) -> bool {
        false
    }

    /// The services this machine starts on its own — the closest thing an init keeps to a
    /// record of a decision, as opposed to a record of right now.
    async fn list_manual_enabled(&self) -> Result<Option<Vec<Package>>> {
        let Some(init) = self.core.detect_init() else {
            return Ok(None);
        };
        if init.list_enabled.is_empty() {
            return Ok(None);
        }
        let (prog, args) = init
            .list_enabled
            .split_first()
            .expect("list_enabled is non-empty here");
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self
            .core
            .executor
            .run_output(prog, &arg_refs, false)
            .await?;
        Ok(Some(init.parse_enabled(&out)))
    }

    fn manual_source(&self) -> String {
        match self.core.detect_init() {
            Some(init) => format!(
                "every service {} reports as running (no init records which you chose)",
                init.name
            ),
            None => "nothing — no init system was detected".to_string(),
        }
    }

    fn adoption_options(&self) -> Vec<(String, String)> {
        vec![("status".to_string(), "running".to_string())]
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
        let Some(init) = self.core.detect_init() else {
            return Ok(());
        };
        if init.status.is_empty() {
            return Ok(());
        }
        let cmd = InitProvider::fill(&init.status, &p.name);
        let (prog, args) = cmd.split_first().expect("status is non-empty here");
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        if let Ok(out) = self.core.executor.run_output(prog, &arg_refs, false).await {
            p.properties.insert("status_raw".into(), out);
        }
        Ok(())
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    cfg: &crate::config::Config,
) {
    // The user's own init rows, through the approval every adapter file goes through (U36/II.12).
    let layout = cfg.layout();
    let user_rows = crate::backends::onboarder::read_approved_definitions(
        &layout.adapter_init_file(),
        &layout.locks_dir(),
    )
    .and_then(|body| match toml::from_str::<InitProviderFile>(&body) {
        Ok(f) => Some(f.init),
        Err(e) => {
            warn!("ignoring adapters/init.toml: {}", e);
            None
        }
    })
    .unwrap_or_default();

    let core = Arc::new(ServiceBackendCore::with_providers(
        exec.duplicate(),
        providers(user_rows),
    ));
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

    fn shipped(name: &str) -> InitProvider {
        providers(vec![])
            .into_iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("{} must ship", name))
    }

    #[test]
    fn the_shipped_table_parses_and_carries_the_five_inits() {
        let names: Vec<String> = providers(vec![]).into_iter().map(|p| p.name).collect();
        for want in ["systemd", "openrc", "sysvinit", "launchd", "windows-sc"] {
            assert!(names.iter().any(|n| n == want), "{:?}", names);
        }
    }

    #[test]
    fn systemd_maps_each_action_and_ends_its_options_before_the_unit() {
        let sd = shipped("systemd");
        for (action, verb) in [
            (ServiceAction::Enable, "enable"),
            (ServiceAction::Disable, "disable"),
            (ServiceAction::Start, "start"),
            (ServiceAction::Stop, "stop"),
            (ServiceAction::Restart, "restart"),
        ] {
            assert_eq!(
                sd.plan(action, "nginx"),
                vec![(
                    action,
                    vec![
                        "systemctl".to_string(),
                        "--no-pager".into(),
                        verb.into(),
                        "--".into(),
                        "nginx".into()
                    ]
                )]
            );
        }
    }

    /// A pager waits for a keypress no captured child receives. Every systemctl row has to
    /// carry the suppression, not only the two that print a screenful — `list` and `status`
    /// are where it was seen, and the rest are the same command deciding the same way.
    #[test]
    fn every_systemd_row_suppresses_the_pager() {
        let sd = shipped("systemd");
        for action in [
            ServiceAction::Enable,
            ServiceAction::Disable,
            ServiceAction::Start,
            ServiceAction::Stop,
            ServiceAction::Restart,
        ] {
            for (_, cmd) in sd.plan(action, "nginx") {
                assert!(cmd.iter().any(|a| a == "--no-pager"), "{:?} can page", cmd);
            }
        }
        assert!(sd.list.iter().any(|a| a == "--no-pager"), "{:?}", sd.list);
        assert!(
            sd.status.iter().any(|a| a == "--no-pager"),
            "{:?}",
            sd.status
        );
    }

    /// These inits take the service between two positionals, so there is nowhere a `--` could go
    /// — and each of them would read it as the service name.
    #[test]
    fn the_other_inits_deliberately_emit_no_terminator() {
        for name in ["openrc", "sysvinit", "launchd", "windows-sc"] {
            let p = shipped(name);
            for action in [
                ServiceAction::Enable,
                ServiceAction::Disable,
                ServiceAction::Start,
                ServiceAction::Stop,
                ServiceAction::Restart,
            ] {
                for (_, cmd) in p.plan(action, "nginx") {
                    assert!(
                        !cmd.iter().any(|a| a == "--"),
                        "{}/{:?} emitted a terminator",
                        name,
                        action,
                    );
                }
            }
        }
    }

    #[test]
    fn openrc_uses_rc_update_and_rc_service() {
        let p = shipped("openrc");
        assert_eq!(
            p.plan(ServiceAction::Enable, "sshd"),
            vec![(
                ServiceAction::Enable,
                vec![
                    "rc-update".to_string(),
                    "add".into(),
                    "sshd".into(),
                    "default".into()
                ]
            )]
        );
        assert_eq!(
            p.plan(ServiceAction::Start, "sshd"),
            vec![(
                ServiceAction::Start,
                vec!["rc-service".to_string(), "sshd".into(), "start".into()]
            )]
        );
    }

    #[test]
    fn windows_restart_is_stop_then_start() {
        let cmds = shipped("windows-sc").plan(ServiceAction::Restart, "W32Time");
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].1[1], "stop");
        assert_eq!(cmds[1].1[1], "start");
    }

    #[test]
    fn launchd_restart_is_stop_then_start() {
        let cmds = shipped("launchd").plan(ServiceAction::Restart, "com.foo");
        assert_eq!(cmds.len(), 2);
    }

    /// A user row with no native restart still restarts — stop then start, derived from the two
    /// required actions, so a niche init need not spell restart out.
    #[test]
    fn a_row_without_restart_derives_stop_then_start() {
        let toml = r#"
[[init]]
name = "dinit"
detect = "dinitctl"
enable = [["dinitctl", "enable", "{name}"]]
disable = [["dinitctl", "disable", "{name}"]]
start = [["dinitctl", "start", "{name}"]]
stop = [["dinitctl", "stop", "{name}"]]
"#;
        let file: InitProviderFile = toml::from_str(toml).unwrap();
        let all = providers(file.init);
        let dinit = all.iter().find(|p| p.name == "dinit").expect("dinit loads");
        let cmds = dinit.plan(ServiceAction::Restart, "web");
        assert_eq!(
            cmds,
            vec![
                (
                    ServiceAction::Stop,
                    vec!["dinitctl".to_string(), "stop".into(), "web".into()]
                ),
                (
                    ServiceAction::Start,
                    vec!["dinitctl".to_string(), "start".into(), "web".into()]
                ),
            ]
        );
    }

    /// Already being in the state the line asks for is what convergence *means*, and `sc` says
    /// so with an exit code: 1056 to a start of a running service, 1062 to a stop of a stopped
    /// one. Per verb, because each is an ordinary failure on the other — a stop that came back
    /// "already running" did not stop anything.
    #[test]
    fn windows_forgives_already_in_that_state_once_per_verb() {
        let sc = shipped("windows-sc");
        assert_eq!(sc.benign_exits(ServiceAction::Start), [1056]);
        assert_eq!(sc.benign_exits(ServiceAction::Stop), [1062]);
        for action in [
            ServiceAction::Enable,
            ServiceAction::Disable,
            ServiceAction::Restart,
        ] {
            assert!(
                sc.benign_exits(action).is_empty(),
                "{:?} asks for a transition, so no code means it already happened",
                action
            );
        }
    }

    /// The other four shipped inits report "already in that state" as exit 0, so they declare
    /// nothing here. Pinned rather than left blank: a row that grows a code silently, or loses
    /// one, is the same invisible change either way.
    #[test]
    fn the_inits_that_answer_zero_declare_no_benign_codes() {
        for name in ["systemd", "openrc", "sysvinit", "launchd"] {
            let p = shipped(name);
            assert!(
                p.start_benign_exits.is_empty() && p.stop_benign_exits.is_empty(),
                "{} declares benign exits — if that is real it needs a measurement beside it",
                name
            );
        }
    }

    /// A derived restart's two halves carry their own verbs, so each forgives its own code. A
    /// single per-provider list could not tell them apart, and would let a failed stop through.
    #[test]
    fn a_derived_restart_labels_each_half_with_its_own_verb() {
        let sc = shipped("windows-sc");
        let cmds = sc.plan(ServiceAction::Restart, "W32Time");
        assert_eq!(cmds[0].0, ServiceAction::Stop);
        assert_eq!(cmds[1].0, ServiceAction::Start);
        assert_eq!(sc.benign_exits(cmds[0].0), [1062]);
        assert_eq!(sc.benign_exits(cmds[1].0), [1056]);
    }

    /// A code in the table that never reaches the executor running the command is the gap that
    /// left sixteen registrars with no policy at all — so this drives a real command through
    /// `install` and reads the outcome, rather than asserting on the table it just read.
    ///
    /// One row, one exit code, two verbs: `start` declares it benign and `stop` does not, so a
    /// pass here cannot come from the command, the code, or the platform.
    #[tokio::test]
    async fn the_benign_code_reaches_the_verb_that_declared_it_and_no_other() {
        #[cfg(windows)]
        let toml = r#"
[[init]]
name = "probe"
detect = "cmd"
start = [["cmd", "/C", "exit 7"]]
stop  = [["cmd", "/C", "exit 7"]]
start_benign_exits = [7]
"#;
        #[cfg(not(windows))]
        let toml = r#"
[[init]]
name = "probe"
detect = "sh"
start = [["sh", "-c", "exit 7"]]
stop  = [["sh", "-c", "exit 7"]]
start_benign_exits = [7]
"#;
        let file: InitProviderFile = toml::from_str(toml).unwrap();
        let core = Arc::new(ServiceBackendCore::with_providers(
            CommandExecutor::new(false, false),
            file.init,
        ));
        let inst = ServiceInstallable { core };
        let spec = |status: &str| PackageSpec {
            name: "irrelevant".to_string(),
            backend: "service".to_string(),
            options: std::collections::HashMap::from([("status".to_string(), status.to_string())]),
            requires: Vec::new(),
            present: true,
        };

        inst.install(&[spec("running")], false)
            .await
            .expect("7 is declared benign for start: already running is the declared state");
        let err = inst
            .install(&[spec("stopped")], false)
            .await
            .expect_err("stop never declared 7 benign, so it is an ordinary failure");
        assert!(err.to_string().contains('7'), "{}", err);
    }

    /// A row that cannot both start and stop is refused rather than half-loaded (U36).
    #[test]
    fn a_row_missing_start_or_stop_is_refused() {
        let toml = r#"
[[init]]
name = "broken"
detect = "brokenctl"
start = [["brokenctl", "up", "{name}"]]
"#;
        let file: InitProviderFile = toml::from_str(toml).unwrap();
        assert!(!providers(file.init).iter().any(|p| p.name == "broken"));
    }

    /// A user row never shadows a shipped init.
    #[test]
    fn a_user_row_cannot_redefine_a_builtin() {
        let toml = r#"
[[init]]
name = "systemd"
detect = "systemctl"
start = [["evil"]]
stop = [["evil"]]
"#;
        let file: InitProviderFile = toml::from_str(toml).unwrap();
        let all = providers(file.init);
        let sd = all.iter().find(|p| p.name == "systemd").unwrap();
        assert_eq!(sd.start[0][0], "systemctl", "the built-in systemd must win");
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
        assert_eq!(
            actions_for(None, Some("running")),
            vec![ServiceAction::Start]
        );
        assert_eq!(
            actions_for(None, Some("stopped")),
            vec![ServiceAction::Stop]
        );
        assert_eq!(actions_for(Some("true"), None), vec![ServiceAction::Enable]);
        assert_eq!(
            actions_for(Some("false"), None),
            vec![ServiceAction::Disable]
        );
        assert_eq!(
            actions_for(Some("true"), Some("running")),
            vec![ServiceAction::Enable, ServiceAction::Start]
        );
        assert_eq!(
            actions_for(None, Some("restarted")),
            vec![ServiceAction::Restart]
        );
    }

    #[test]
    fn systemd_listing_strips_the_service_suffix() {
        let out = "nginx.service loaded active running Nginx\n\
                   sshd.service  loaded active running OpenSSH\n";
        let pkgs = shipped("systemd").parse_list(out);
        let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["nginx", "sshd"]);
    }

    #[test]
    fn windows_listing_reads_the_service_name_field() {
        let out = "SERVICE_NAME: W32Time\n        DISPLAY_NAME: Windows Time\n\
                   SERVICE_NAME: Spooler\n";
        let pkgs = shipped("windows-sc").parse_list(out);
        let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["W32Time", "Spooler"]);
    }

    #[test]
    fn launchd_listing_skips_its_header() {
        let out = "PID\tStatus\tLabel\n\
                   123\t0\tcom.apple.foo\n\
                   -\t0\tcom.apple.bar\n";
        let pkgs = shipped("launchd").parse_list(out);
        let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["com.apple.foo", "com.apple.bar"]);
    }
}
