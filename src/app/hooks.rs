use crate::config::Config;
use crate::core::{Error, Result};
use mlua::Lua;
use rhai::{Engine, Scope};
use regex::Regex;
use std::collections::HashMap;
use tokio::process::Command;
use std::io::Write;
use tempfile::NamedTempFile;
use tracing::{debug, info};

/// Manages the execution of scripting hooks for package lifecycle events.
/// Supports Lua, Rhai, and any language with a system shebang.
/// 
/// Hardened for Phase 3.3: Correctly handles mlua thread-safety by initializing 
/// the interpreter within the execution context, ensuring the closure is Send.
pub struct LuaHooks {
    rhai_engine: Engine,
    pub hooks: HashMap<String, HashMap<String, String>>,
}

impl LuaHooks {
    pub fn new(config: &Config) -> Result<Self> {
        let mut rhai_engine = Engine::new();
        Self::setup_rhai_sandbox(&mut rhai_engine);
        
        Ok(Self {
            rhai_engine,
            hooks: config.hooks.clone(),
        })
    }

    /// The Polyglot Bridge: Detects shebangs and executes via system interpreter.
    async fn run_external_polyglot(&self, code: &str, hook: &str, pkg: &str) -> Result<()> {
        debug!("Hooks: Launching Polyglot Bridge for {}/{}", hook, pkg);

        let code_owned = code.to_string();
        
        let tmp_script = tokio::task::spawn_blocking(move || -> Result<NamedTempFile> {
            let mut tmp = NamedTempFile::new().map_err(Error::from)?;
            tmp.write_all(code_owned.as_bytes()).map_err(Error::from)?;
            
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(tmp.path()).map_err(Error::from)?.permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(tmp.path(), perms).map_err(Error::from)?;
            }
            Ok(tmp)
        }).await.map_err(|e| Error::Other(e.to_string()))??;

        let mut cmd = Command::new(tmp_script.path());
        cmd.env("LINIX_PKG_NAME", pkg)
           .env("LINIX_HOOK_TYPE", hook)
           .env("LINIX_OS", std::env::consts::OS)
           .env("LINIX_ARCH", std::env::consts::ARCH);

        let status = cmd.status().await.map_err(|e| Error::Other(format!("Polyglot execution failed: {}", e)))?;

        if !status.success() {
            return Err(Error::LuaScript(format!("External hook failed with exit code: {:?}", status.code())));
        }

        Ok(())
    }

    /// Executes a specific hook trigger for a package.
    pub async fn run_hook(&self, hook_name: &str, package_name: &str) -> Result<()> {
        let script = if let Some(category) = self.hooks.get(hook_name) {
            category.get(package_name).or_else(|| category.get("*"))
        } else {
            None
        };

        if let Some(code) = script {
            let trimmed = code.trim_start();
            if trimmed.starts_with("#!") {
                self.run_external_polyglot(code, hook_name, package_name).await?;
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

        self.rhai_engine.run_with_scope(&mut scope, code)
            .map_err(|e| Error::LuaScript(format!("Rhai execution error: {}", e)))?;
        Ok(())
    }

    /// Runs a Lua hook within a fresh interpreter instance inside a blocking task.
    /// This ensures thread-safety since mlua::Lua is !Send.
    async fn run_lua(&self, code: &str, hook: &str, pkg: &str) -> Result<()> {
        let code_owned = code.to_string();
        let hook_owned = hook.to_string();
        let pkg_owned = pkg.to_string();

        tokio::task::spawn_blocking(move || {
            let lua = Lua::new();
            Self::setup_lua_sandbox(&lua)?;
            
            lua.globals().set("PKG_NAME", pkg_owned).map_err(Error::from)?;
            lua.globals().set("HOOK_TYPE", hook_owned).map_err(Error::from)?;

            lua.load(&code_owned).exec().map_err(Error::from)?;
            Ok::<(), Error>(())
        }).await.map_err(|e| Error::Other(e.to_string()))??;

        Ok(())
    }

    fn setup_lua_sandbox(lua: &Lua) -> Result<()> {
        let globals = lua.globals();
        globals.set("OS", std::env::consts::OS).map_err(Error::from)?;
        globals.set("ARCH", std::env::consts::ARCH).map_err(Error::from)?;
        
        let print_proxy = lua.create_function(|_, args: mlua::MultiValue| {
            let output: Vec<String> = args.iter().map(|v| format!("{:?}", v)).collect();
            info!("[Lua] {}", output.join(" "));
            Ok(())
        }).map_err(Error::from)?;
        
        globals.set("print", print_proxy).map_err(Error::from)?;
        Ok(())
    }

    fn setup_rhai_sandbox(engine: &mut Engine) {
        engine.register_fn("print", |msg: &str| info!("[Rhai] {}", msg));
    }

    /// Renders a template using a localized Lua instance.
    pub fn render_template(&self, template: &str) -> String {
        let lua = Lua::new();
        if let Err(e) = Self::setup_lua_sandbox(&lua) {
            debug!("Template: Failed to setup Lua sandbox: {}", e);
            return template.to_string();
        }

        let re = Regex::new(r"\{\{(.*?)\}\}").unwrap();
        re.replace_all(template, |caps: &regex::Captures| {
            let code = &caps[1];
            match lua.load(code).eval::<String>() {
                Ok(val) => val,
                Err(_) => caps[0].to_string() 
            }
        }).to_string()
    }

    pub async fn run_before_sync(&self) -> Result<()> { self.run_hook("before_sync", "*").await }
    pub async fn run_after_sync(&self) -> Result<()> { self.run_hook("after_sync", "*").await }
}