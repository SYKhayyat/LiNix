use crate::core::{
    BackendCore, CommandExecutor, Installable, MetadataProvider, Package, PackageSpec, Queryable,
    Result, Searchable, Upgradable,
};
use crate::parsers::utils::sanitize;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::{debug, info, warn};

pub struct SnapBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl SnapBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "snap".to_string(),
        }
    }
}

#[async_trait]
impl BackendCore for SnapBackendCore {
    fn name(&self) -> &str {
        "snap"
    }

    fn is_available(&self) -> bool {
        self.executor.command_exists_sync("snap")
    }

    fn needs_root(&self) -> bool {
        // Snap operations almost always require administrative privileges.
        true
    }
}

#[async_trait]
impl MetadataProvider for SnapBackendCore {
    async fn get_dependencies(&self, name: &str) -> Result<Vec<String>> {
        let mut args = vec!["info".to_string()];
        crate::core::argv::push_names(&mut args, "snap", [name]);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = self.executor.run_output("snap", &arg_refs, false).await?;
        let mut deps = Vec::new();

        // Snap dependencies (base snaps like core22, or content interface connections)
        // are usually identified in the 'notes' or 'connections' but snaps are
        // largely self-contained. We look for 'base:' which is the most common requirement.
        for line in output.lines() {
            if let Some(base) = line.strip_prefix("base:") {
                deps.push(base.trim().to_string());
            }
        }

        Ok(deps)
    }
}

pub struct SnapInstallable {
    pub core: Arc<SnapBackendCore>,
}

fn install_args(spec: &PackageSpec) -> Vec<String> {
    let mut args = vec!["install".to_string()];

    if spec.options.get("classic") == Some(&"true".to_string()) {
        args.push("--classic".into());
    }

    if let Some(channel) = spec.options.get("channel") {
        args.push("--channel".into());
        args.push(channel.clone());
    }

    crate::core::argv::push_names(&mut args, "snap", [&spec.name]);
    args
}

/// Snap risk levels, most stable first. Moving toward a *more* stable level is a downgrade —
/// `edge -> stable` gives you an older build (D13).
const RISK_ORDER: &[&str] = &["stable", "candidate", "beta", "edge"];

fn risk_rank(channel: &str) -> Option<usize> {
    let risk = crate::backends::artifact::capability::channel_risk(channel);
    RISK_ORDER.iter().position(|r| *r == risk)
}

/// Whether moving from `from` to `to` is a downgrade (a more-stable, less-recent channel). An
/// unknown risk on either side is not called a downgrade — better to under-warn than to cry
/// downgrade on a channel we do not understand.
fn is_channel_downgrade(from: &str, to: &str) -> bool {
    match (risk_rank(from), risk_rank(to)) {
        (Some(f), Some(t)) => t < f,
        _ => false,
    }
}

impl SnapInstallable {
    async fn is_installed(&self, name: &str) -> bool {
        match self
            .core
            .executor
            .run_output("snap", &["list", name], false)
            .await
        {
            Ok(out) => out
                .lines()
                .skip(1)
                .any(|l| l.split_whitespace().next() == Some(name)),
            Err(_) => false,
        }
    }

    /// The channel `name` is currently following, read from `snap info`'s `tracking:` line.
    async fn current_channel(&self, name: &str) -> Option<String> {
        let out = self
            .core
            .executor
            .run_output("snap", &["info", name], false)
            .await
            .ok()?;
        out.lines()
            .find_map(|l| l.strip_prefix("tracking:"))
            .map(|v| crate::backends::artifact::capability::channel_risk(v).to_string())
    }
}

#[async_trait]
impl Installable for SnapInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        for spec in specs {
            // D13: `snap install` refuses an already-installed snap, so a `@channel` change has
            // to go through `snap refresh --channel=`. Decide by whether the snap is present.
            let want_channel = spec.options.get("channel");
            let already = self.is_installed(&spec.name).await;

            let args = if let (true, Some(channel)) = (already, want_channel) {
                if let Some(current) = self.current_channel(&spec.name).await {
                    if is_channel_downgrade(&current, channel) {
                        // A downgrade is removal-shaped; name it so it is not a silent swap in
                        // the plan the user is confirming.
                        warn!(
                            "Snap: {} is a channel downgrade ({} -> {}) — a less recent build",
                            spec.name, current, channel
                        );
                    }
                }
                let mut a = vec![
                    "refresh".to_string(),
                    "--channel".to_string(),
                    channel.clone(),
                ];
                crate::core::argv::push_names(&mut a, "snap", [spec.name.as_str()]);
                info!("Snap: Switching {} to channel {}...", spec.name, channel);
                a
            } else {
                info!("Snap: Installing {}...", spec.name);
                install_args(spec)
            };

            let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            self.core
                .executor
                .run_exclusive("snap", "snap", &arg_refs, sudo)
                .await?;
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        for name in names {
            info!("Snap: Removing {}...", name);
            let mut args = vec!["remove".to_string()];
            crate::core::argv::push_names(&mut args, "snap", [name]);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core
                .executor
                .run_exclusive("snap", "snap", &arg_refs, sudo)
                .await?;
        }
        Ok(())
    }
}

pub struct SnapQueryable {
    pub core: Arc<SnapBackendCore>,
}

#[async_trait]
impl Queryable for SnapQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        let output = self
            .core
            .executor
            .run_output("snap", &["list"], false)
            .await?;
        let mut packages = Vec::new();

        for line in sanitize(&output).lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let (Some(name), Some(version)) = (parts.first(), parts.get(1)) {
                packages.push(Package::with_version(name, version, "snap"));
            }
        }
        Ok(packages)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        let installed = self.list_installed().await?;
        let base_snaps = [
            "core",
            "core18",
            "core20",
            "core22",
            "snapd",
            "bare",
            "gtk-common-themes",
        ];
        Ok(installed
            .into_iter()
            .filter(|p| !base_snaps.contains(&p.name.as_str()))
            .collect())
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let mut args = vec!["info".to_string()];
        crate::core::argv::push_names(&mut args, "snap", [name]);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = self
            .core
            .executor
            .run_output("snap", &arg_refs, false)
            .await?;
        if output.is_empty() {
            return Ok(None);
        }

        let mut p = Package::new(name, "snap");
        for line in output.lines() {
            if let Some(v) = line.strip_prefix("summary:") {
                p.properties.insert("summary".into(), v.trim().to_string());
            }
            if let Some(v) = line.strip_prefix("installed:") {
                let ver = v.split_whitespace().next().unwrap_or(v);
                p.version = Some(ver.trim().to_string());
            }
            // D13: the channel this snap is following, so a `@channel` change is visible to the
            // planner. `snap info` prints `tracking:     latest/stable`.
            if let Some(v) = line.strip_prefix("tracking:") {
                let risk = crate::backends::artifact::capability::channel_risk(v.trim());
                p.properties.insert("channel".into(), risk.to_string());
            }
        }
        Ok(Some(p))
    }
}

pub struct SnapSearchable {
    pub core: Arc<SnapBackendCore>,
}

#[async_trait]
impl Searchable for SnapSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let mut args = vec!["find".to_string()];
        crate::core::argv::push_names(&mut args, "snap", [query]);
        let search_args: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = self
            .core
            .executor
            .search_output("snap", &search_args, false)
            .await?;
        Ok(parse_snap_find(&output))
    }
}

/// Parse `snap find <q>` => "Name  Version  Publisher  Notes  Summary" (header + rows).
fn parse_snap_find(output: &str) -> Vec<Package> {
    let mut results = Vec::new();
    for line in sanitize(output).lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        let Some(name) = parts.first() else { continue };
        let mut p = Package::new(*name, "snap");
        if let Some(version) = parts.get(1) {
            p.version = Some(version.to_string());
        }
        if let Some(publisher) = parts.get(2) {
            p.properties
                .insert("publisher".into(), publisher.to_string());
        }
        if parts.len() > 4 {
            p.properties.insert("summary".into(), parts[4..].join(" "));
        }
        results.push(p);
    }
    results
}

pub struct SnapUpgradable {
    pub core: Arc<SnapBackendCore>,
}

#[async_trait]
impl Upgradable for SnapUpgradable {
    async fn update(&self, sudo: bool) -> Result<()> {
        debug!("Snap: Refreshing all snaps...");
        self.core
            .executor
            .run_exclusive("snap", "snap", &["refresh"], sudo)
            .await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        self.update(sudo).await
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let core = Arc::new(SnapBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(SnapInstallable { core: core.clone() }))
            .with_queryable(Arc::new(SnapQueryable { core: core.clone() }))
            .with_searchable(Arc::new(SnapSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(SnapUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_find_parses_rows() {
        let out = "Name   Version  Publisher  Notes  Summary\n\
                   hello  2.10     canonical  -      GNU Hello prints a greeting\n\
                   code   1.85     vscode✓    classic Code editing redefined\n";
        let pkgs = parse_snap_find(out);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "hello");
        assert_eq!(pkgs[0].version.as_deref(), Some("2.10"));
        assert_eq!(
            pkgs[0].properties.get("publisher").map(String::as_str),
            Some("canonical")
        );
        assert!(pkgs[0]
            .properties
            .get("summary")
            .unwrap()
            .contains("greeting"));
    }

    fn spec_with(name: &str, options: &[(&str, &str)]) -> PackageSpec {
        PackageSpec {
            name: name.to_string(),
            backend: "snap".to_string(),
            options: options
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn snap_channel_reaches_the_command() {
        let args = install_args(&spec_with("code", &[("channel", "edge")]));
        assert_eq!(args, ["install", "--channel", "edge", "--", "code"]);
    }

    #[test]
    fn snap_without_a_channel_passes_no_channel_flag() {
        let args = install_args(&spec_with("code", &[]));
        assert_eq!(args, ["install", "--", "code"]);
    }

    #[test]
    fn moving_to_a_more_stable_channel_is_a_downgrade() {
        // D13: edge -> stable gives an older build, so the guard/plan should see it.
        assert!(is_channel_downgrade("edge", "stable"));
        assert!(is_channel_downgrade("latest/beta", "latest/candidate"));
        // stable -> edge is newer, not a downgrade.
        assert!(!is_channel_downgrade("stable", "edge"));
        // Same channel, whichever way it is spelled, is not a downgrade.
        assert!(!is_channel_downgrade("latest/stable", "stable"));
        // A channel we do not understand is never called a downgrade.
        assert!(!is_channel_downgrade("weird", "stable"));
    }

    #[tokio::test]
    async fn snaps_other_name_carrying_commands_terminate_too() {
        let vfs = Arc::new(dashmap::DashMap::new());
        let mock = Arc::new(crate::core::executor::MockExecutor::new(vfs.clone()));
        let exec = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(dashmap::DashMap::new()),
        );
        let core = Arc::new(SnapBackendCore::new(exec));

        SnapInstallable { core: core.clone() }
            .remove(&["code".to_string()], false)
            .await
            .unwrap();
        SnapQueryable { core: core.clone() }.info("code").await.ok();
        SnapSearchable { core: core.clone() }
            .search("code")
            .await
            .unwrap();
        core.get_dependencies("code").await.unwrap();

        assert_eq!(
            mock.get_calls().await,
            vec![
                "snap remove -- code",
                "snap info -- code",
                "snap find -- code",
                "snap info -- code",
            ]
        );
    }

    #[test]
    fn a_snap_named_like_a_flag_stays_a_name() {
        let args = install_args(&spec_with("--classic", &[]));
        assert_eq!(args, ["install", "--", "--classic"]);
    }
}
