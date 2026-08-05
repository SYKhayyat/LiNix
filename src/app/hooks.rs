use crate::config::Config;
use crate::core::hook_lock::{hash_script, hook_id, refusal, HookLedger};
use crate::core::LockFile;
use crate::core::{Error, Result};
use mlua::Lua;
use rhai::{Engine, Scope};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use tempfile::NamedTempFile;
use tokio::process::Command;
use tracing::{debug, info};

pub struct LuaHooks {
    rhai_engine: Engine,
    pub hooks: HashMap<String, HashMap<String, String>>,
    /// The repo's `locks/` directory, where the hook approval ledger lives (II.12).
    locks_dir: PathBuf,
}

impl LuaHooks {
    pub fn new(config: &Config) -> Result<Self> {
        let mut rhai_engine = Engine::new();
        Self::setup_rhai_sandbox(&mut rhai_engine);

        Ok(Self {
            rhai_engine,
            hooks: config.hooks.clone(),
            locks_dir: config.config_root().join("locks"),
        })
    }

    /// The supply-chain gate (II.12): before a sync runs any hook, every configured hook must
    /// be approved at its current hash. A new or changed script stops the sync — `-y` cannot
    /// skip this, and only `linix lock` approves. Called with `?` from a place that propagates,
    /// because a swallowed refusal here is no refusal at all.
    ///
    /// Reports every unapproved hook at once, not just the first: a reader fixing their locks
    /// wants the whole list, not a one-at-a-time drip.
    pub fn verify_all_approved(&self) -> Result<()> {
        if self.hooks.is_empty() {
            return Ok(());
        }
        let ledger = HookLedger::load(&HookLedger::path_in(&self.locks_dir))?;
        let mut refusals = Vec::new();
        for (id, script) in self.each_hook() {
            let verdict = ledger.verdict(&id, &hash_script(&script));
            if !verdict.is_approved() {
                refusals.push(refusal(&id, "config", &verdict));
            }
        }
        if refusals.is_empty() {
            return Ok(());
        }
        Err(Error::Refused(format!(
            "refusing to sync: {} hook(s) are not approved (II.12).\n\n{}",
            refusals.len(),
            refusals.join("\n\n")
        )))
    }

    /// Approve every configured hook at its current hash — what `linix lock` does for hooks.
    /// Returns how many approvals were written. This is the only path that writes an approval,
    /// so approval stays a deliberate act.
    pub fn approve_all_hooks(&self) -> Result<usize> {
        let path = HookLedger::path_in(&self.locks_dir);
        let mut ledger = HookLedger::load(&path)?;
        let mut count = 0;
        for (id, script) in self.each_hook() {
            ledger.approve(&id, &hash_script(&script));
            count += 1;
        }
        ledger.save(&path)?;
        Ok(count)
    }

    /// Every hook as `(hook_id, script)`. One place builds the identity so enforcement and
    /// approval can never key the ledger differently.
    fn each_hook(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (hook_name, by_package) in &self.hooks {
            for (package, script) in by_package {
                out.push((hook_id(hook_name, package), script.clone()));
            }
        }
        out
    }

    async fn run_external_polyglot(&self, code: &str, hook: &str, pkg: &str) -> Result<()> {
        debug!("Hooks: Launching Polyglot Bridge for {}/{}", hook, pkg);

        let code_owned = code.to_string();

        let tmp_script = tokio::task::spawn_blocking(move || -> Result<NamedTempFile> {
            let mut tmp = NamedTempFile::new().map_err(Error::from)?;
            tmp.write_all(code_owned.as_bytes()).map_err(Error::from)?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(tmp.path())
                    .map_err(Error::from)?
                    .permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(tmp.path(), perms).map_err(Error::from)?;
            }
            Ok(tmp)
        })
        .await
        .map_err(|e| Error::Other(e.to_string()))??;

        let mut cmd = Command::new(tmp_script.path());
        cmd.env("LINIX_PKG_NAME", pkg)
            .env("LINIX_HOOK_TYPE", hook)
            .env("LINIX_OS", std::env::consts::OS)
            .env("LINIX_ARCH", std::env::consts::ARCH);

        let status = cmd
            .status()
            .await
            .map_err(|e| Error::Other(format!("Polyglot execution failed: {}", e)))?;

        if !status.success() {
            return Err(Error::LuaScript(format!(
                "External hook failed with exit code: {:?}",
                status.code()
            )));
        }

        Ok(())
    }

    /// A package-specific hook shadows the `*` catch-all rather than running in addition
    /// to it — a script registered for one package silently disables the global one.
    pub async fn run_hook(&self, hook_name: &str, package_name: &str) -> Result<()> {
        let script = if let Some(category) = self.hooks.get(hook_name) {
            category.get(package_name).or_else(|| category.get("*"))
        } else {
            None
        };

        if let Some(code) = script {
            let trimmed = code.trim_start();
            if trimmed.starts_with("#!") {
                self.run_external_polyglot(code, hook_name, package_name)
                    .await?;
            } else if trimmed.starts_with("#rhai") {
                self.run_rhai(code, hook_name, package_name)?;
            } else {
                self.run_lua(code, hook_name, package_name).await?;
            }
        }

        Ok(())
    }

    fn run_rhai(&self, code: &str, hook: &str, pkg: &str) -> Result<()> {
        let mut scope = Scope::new();
        scope.push("PKG_NAME", pkg.to_string());
        scope.push("HOOK_TYPE", hook.to_string());

        self.rhai_engine
            .run_with_scope(&mut scope, code)
            .map_err(|e| Error::LuaScript(format!("Rhai execution error: {}", e)))?;
        Ok(())
    }

    /// The Lua interpreter must be constructed INSIDE the blocking closure: `mlua::Lua` is
    /// !Send, so holding one across this boundary will not compile.
    async fn run_lua(&self, code: &str, hook: &str, pkg: &str) -> Result<()> {
        let code_owned = code.to_string();
        let hook_owned = hook.to_string();
        let pkg_owned = pkg.to_string();

        tokio::task::spawn_blocking(move || {
            let lua = Lua::new();
            Self::setup_lua_sandbox(&lua)?;

            lua.globals()
                .set("PKG_NAME", pkg_owned)
                .map_err(Error::from)?;
            lua.globals()
                .set("HOOK_TYPE", hook_owned)
                .map_err(Error::from)?;

            lua.load(&code_owned).exec().map_err(Error::from)?;
            Ok::<(), Error>(())
        })
        .await
        .map_err(|e| Error::Other(e.to_string()))??;

        Ok(())
    }

    fn setup_lua_sandbox(lua: &Lua) -> Result<()> {
        let globals = lua.globals();
        globals
            .set("OS", std::env::consts::OS)
            .map_err(Error::from)?;
        globals
            .set("ARCH", std::env::consts::ARCH)
            .map_err(Error::from)?;

        let print_proxy = lua
            .create_function(|_, args: mlua::MultiValue| {
                let output: Vec<String> = args.iter().map(|v| format!("{:?}", v)).collect();
                info!("[Lua] {}", output.join(" "));
                Ok(())
            })
            .map_err(Error::from)?;

        globals.set("print", print_proxy).map_err(Error::from)?;
        Ok(())
    }

    fn setup_rhai_sandbox(engine: &mut Engine) {
        engine.register_fn("print", |msg: &str| info!("[Rhai] {}", msg));
    }

    // SEC7: `render_template` (arbitrary `{{ … }}` evaluated as Lua, with `os`/`io`/`os.execute`
    // left intact in the sandbox) is DELETED. It had no callers — the only `.render_template(`
    // in the tree is the link backend's Tera renderer — so it was dead code, but a loaded gun:
    // wire it to file content and it is ungated RCE. Tera is the live, safe templating path; the
    // gated Lua/Rhai/`#!` *hook* path (approved via the II.12 ledger) is a separate feature and
    // stays. NO-LEGACY: a dead code-exec path is removed, not kept "just in case".

    pub async fn run_before_sync(&self) -> Result<()> {
        self.run_hook("before_sync", "*").await
    }
    pub async fn run_after_sync(&self) -> Result<()> {
        self.run_hook("after_sync", "*").await
    }
}
