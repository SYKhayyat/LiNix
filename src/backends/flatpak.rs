use crate::core::{
    BackendCore, CommandExecutor, Installable, MetadataProvider, Package, PackageSpec, Queryable,
    Result, Searchable, Upgradable,
};
use crate::utils::text::sanitize;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info};

pub struct FlatpakBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
    /// Backend-specific settings like default scope (user vs system).
    pub settings: HashMap<String, String>,
}

impl FlatpakBackendCore {
    pub fn new(executor: CommandExecutor, settings: HashMap<String, String>) -> Self {
        Self {
            executor,
            name: "flatpak".to_string(),
            settings,
        }
    }

    pub fn scope_args(&self) -> Vec<&str> {
        if self
            .settings
            .get("user")
            .map(|v| v == "true")
            .unwrap_or(false)
        {
            vec!["--user"]
        } else {
            vec!["--system"]
        }
    }
}

#[async_trait]
impl BackendCore for FlatpakBackendCore {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        // No per-backend cache: the executor memoises every PATH lookup now, which dedupes
        // across the backends that probe the same program too. One backend having its own
        // `OnceCell` while the other forty-four re-probed is exactly the "two of everything"
        // this repo removes.
        self.executor.command_exists_sync("flatpak")
    }
    fn probes(&self) -> Vec<String> {
        vec!["flatpak".into()]
    }

    fn needs_root(&self) -> bool {
        // If the 'user' setting is true, Flatpak does not need root privileges.
        !self
            .settings
            .get("user")
            .map(|v| v == "true")
            .unwrap_or(false)
    }
}

#[async_trait]
impl MetadataProvider for FlatpakBackendCore {
    async fn get_dependencies(&self, name: &str) -> Result<Vec<String>> {
        let mut final_args: Vec<String> =
            self.scope_args().into_iter().map(str::to_string).collect();
        final_args.extend(["info".to_string(), "--show-metadata".to_string()]);
        crate::core::argv::push_names(&mut final_args, "flatpak", [name]);
        let arg_refs: Vec<&str> = final_args.iter().map(String::as_str).collect();

        // Flatpak metadata contains a [Extension] or [Runtime] section.
        // We look for 'runtime=' which is the primary transitive dependency.
        let output = self
            .executor
            .run_output("flatpak", &arg_refs, false)
            .await?;
        let mut deps = Vec::new();

        for line in output.lines() {
            if let Some(runtime) = line.strip_prefix("runtime=") {
                deps.push(runtime.trim().to_string());
            }
        }

        Ok(deps)
    }
}

pub struct FlatpakInstallable {
    pub core: Arc<FlatpakBackendCore>,
}

/// A flatpak ref is `name/arch/branch`. The arch slot stays empty so flatpak keeps choosing it
/// from the machine; writing `name/branch` would be read as an architecture, not a branch.
fn install_ref(spec: &PackageSpec) -> String {
    match spec.options.get("channel") {
        Some(channel) => format!("{}//{}", spec.name, channel),
        None => spec.name.clone(),
    }
}

fn install_argv(scope: &[&str], specs: &[PackageSpec]) -> Vec<String> {
    let mut args: Vec<String> = scope.iter().map(|s| s.to_string()).collect();
    args.extend([
        "install".to_string(),
        "-y".to_string(),
        "--noninteractive".to_string(),
    ]);
    let names: Vec<String> = specs.iter().map(install_ref).collect();
    crate::core::argv::push_names(&mut args, "flatpak", names);
    args
}

#[async_trait]
impl Installable for FlatpakInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        if specs.is_empty() {
            return Ok(());
        }

        let args = install_argv(&self.core.scope_args(), specs);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();

        info!("Flatpak: Installing {} package(s)...", specs.len());
        self.core
            .executor
            .run_exclusive("flatpak", "flatpak", &arg_refs, sudo)
            .await?;
        Ok(())
    }

    async fn remove(&self, names: &[String], sudo: bool) -> Result<()> {
        if names.is_empty() {
            return Ok(());
        }

        let mut args: Vec<String> = self
            .core
            .scope_args()
            .into_iter()
            .map(str::to_string)
            .collect();
        args.extend([
            "uninstall".to_string(),
            "-y".to_string(),
            "--noninteractive".to_string(),
        ]);
        crate::core::argv::push_names(&mut args, "flatpak", names);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();

        info!("Flatpak: Removing {} package(s)...", names.len());
        self.core
            .executor
            .run_exclusive("flatpak", "flatpak", &arg_refs, sudo)
            .await?;
        Ok(())
    }
}

pub struct FlatpakQueryable {
    pub core: Arc<FlatpakBackendCore>,
}

#[async_trait]
impl Queryable for FlatpakQueryable {
    fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
        (self.core.executor.installed_listings(), &self.core.name)
    }

    async fn fetch_installed(&self) -> Result<Vec<Package>> {
        let out = self
            .core
            .executor
            .run_output(
                "flatpak",
                &["list", "--app", "--columns=application,version"],
                false,
            )
            .await?;
        let mut packages = Vec::new();

        for line in sanitize(&out).lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                packages.push(Package::with_version(parts[0], parts[1], "flatpak"));
            } else if !line.is_empty() {
                packages.push(Package::new(line.trim(), "flatpak"));
            }
        }
        Ok(packages)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.list_installed().await?;
        Ok(all.into_iter().find(|p| p.name == name))
    }
}

pub struct FlatpakSearchable {
    pub core: Arc<FlatpakBackendCore>,
}

#[async_trait]
impl Searchable for FlatpakSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let mut args = vec!["search".to_string()];
        crate::core::argv::push_names(&mut args, "flatpak", [query]);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = self
            .core
            .executor
            .search_output("flatpak", &arg_refs, false)
            .await?;
        Ok(parse_flatpak_search(&output))
    }
}

/// Parse `flatpak search <q>` => TAB-separated columns:
/// Name \t Description \t Application ID \t Version \t Branch \t Remotes.
/// The Application ID is the installable identifier, so prefer it as the name.
fn parse_flatpak_search(output: &str) -> Vec<Package> {
    let mut results = Vec::new();
    for line in sanitize(output).lines() {
        if line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').map(|c| c.trim()).collect();
        let display_name = cols.first().copied().unwrap_or("").trim();
        let app_id = cols
            .get(2)
            .copied()
            .filter(|s| !s.is_empty())
            .unwrap_or(display_name);
        if app_id.is_empty() {
            continue;
        }
        let mut p = Package::new(app_id, "flatpak");
        if let Some(desc) = cols.get(1).filter(|s| !s.is_empty()) {
            p.properties.insert("description".into(), desc.to_string());
        }
        if let Some(ver) = cols.get(3).filter(|s| !s.is_empty()) {
            p.version = Some(ver.to_string());
        }
        results.push(p);
    }
    results
}

pub struct FlatpakUpgradable {
    pub core: Arc<FlatpakBackendCore>,
}

#[async_trait]
impl Upgradable for FlatpakUpgradable {
    async fn update(&self, sudo: bool) -> Result<()> {
        // Must pass -y --noninteractive (like install/upgrade/clean_orphans), otherwise an
        // automated run blocks on flatpak's interactive confirmation prompt.
        let mut args = self.core.scope_args();
        args.extend(["update", "-y", "--noninteractive"]);
        debug!("Flatpak: Refreshing remotes...");
        self.core
            .executor
            .run_exclusive("flatpak", "flatpak", &args, sudo)
            .await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        let mut args = self.core.scope_args();
        args.extend(["update", "-y", "--noninteractive"]);
        info!("Flatpak: Upgrading all applications...");
        self.core
            .executor
            .run_exclusive("flatpak", "flatpak", &args, sudo)
            .await?;
        Ok(())
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    cfg: &crate::config::Config,
) {
    let settings = cfg
        .backend_settings
        .get("flatpak")
        .cloned()
        .unwrap_or_default();
    let core = Arc::new(FlatpakBackendCore::new(exec.duplicate(), settings));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(FlatpakInstallable { core: core.clone() }))
            .with_queryable(Arc::new(FlatpakQueryable { core: core.clone() }))
            .with_searchable(Arc::new(FlatpakSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(FlatpakUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatpak_search_prefers_app_id() {
        // Name \t Description \t AppID \t Version \t Branch \t Remotes
        let out = "Blender\tFree 3D suite\torg.blender.Blender\t4.0\tstable\tflathub\n\
                   GIMP\tImage editor\torg.gimp.GIMP\t2.10\tstable\tflathub\n";
        let pkgs = parse_flatpak_search(out);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "org.blender.Blender");
        assert_eq!(pkgs[0].version.as_deref(), Some("4.0"));
        assert_eq!(
            pkgs[0].properties.get("description").map(String::as_str),
            Some("Free 3D suite")
        );
    }

    fn spec_with(name: &str, options: &[(&str, &str)]) -> PackageSpec {
        PackageSpec {
            name: name.to_string(),
            backend: "flatpak".to_string(),
            options: options
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn flatpak_channel_becomes_the_branch_of_the_ref() {
        let spec = spec_with("org.gimp.GIMP", &[("channel", "beta")]);
        assert_eq!(install_ref(&spec), "org.gimp.GIMP//beta");
    }

    #[test]
    fn flatpak_without_a_channel_installs_the_bare_name() {
        let spec = spec_with("org.gimp.GIMP", &[]);
        assert_eq!(install_ref(&spec), "org.gimp.GIMP");
    }

    #[test]
    fn flatpak_refs_come_after_the_terminator() {
        let argv = install_argv(
            &["--system"],
            &[spec_with("org.gimp.GIMP", &[]), spec_with("--user", &[])],
        );
        assert_eq!(
            argv,
            [
                "--system",
                "install",
                "-y",
                "--noninteractive",
                "--",
                "org.gimp.GIMP",
                "--user"
            ]
        );
    }

    #[tokio::test]
    async fn flatpaks_other_name_carrying_commands_terminate_too() {
        let vfs = Arc::new(dashmap::DashMap::new());
        let mock = Arc::new(crate::core::executor::MockExecutor::new(vfs.clone()));
        let exec = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(dashmap::DashMap::new()),
        );
        let core = Arc::new(FlatpakBackendCore::new(exec, HashMap::new()));

        FlatpakInstallable { core: core.clone() }
            .remove(&["org.gimp.GIMP".to_string()], false)
            .await
            .unwrap();
        FlatpakSearchable { core: core.clone() }
            .search("gimp")
            .await
            .unwrap();
        core.get_dependencies("org.gimp.GIMP").await.unwrap();

        assert_eq!(
            mock.get_calls().await,
            vec![
                "flatpak --system uninstall -y --noninteractive -- org.gimp.GIMP",
                "flatpak search -- gimp",
                "flatpak --system info --show-metadata -- org.gimp.GIMP",
            ]
        );
    }
}
