use crate::config::Config;
use crate::core::{Error, Result};
use mlua::Lua;
use rhai::{Engine, Scope};
use regex::Regex;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::process::Command;
use std::io::Write;
use tempfile::NamedTempFile;
use tracing::{debug, info, warn};

/// Manages the execution of scripting hooks for package lifecycle events.
/// Supports Lua, Rhai, and any language with a system shebang (The Polyglot Bridge).
pub struct LuaHooks {
    /// Mutex protected Lua state for !Send compatibility.
    lua: Arc<Mutex<Lua>>,
    /// High-performance Rust-native Rhai engine.
    rhai_engine: Engine,
    /// Map of Hook Category to (Package -> Script) mapping.
    pub hooks: HashMap<String, HashMap<String, String>>,
}

impl LuaHooks {
    pub fn new(config: &Config) -> Result<Self> {
        let lua = Lua::new();
        Self::setup_lua_sandbox(&lua)?;

        let mut rhai_engine = Engine::new();
        Self::setup_rhai_sandbox(&mut rhai_engine);
        
        Ok(Self {
            lua: Arc::new(Mutex::new(lua)),
            rhai_engine,
            hooks: config.hooks.clone(),
        })
    }

    /// Point 4: The Polyglot Bridge.
    /// Detects shebangs and executes via system interpreter.
    /// Injects context via LINIX_ environment variables.
    async fn run_external_polyglot(&self, code: &str, hook: &str, pkg: &str) -> Result<()> {
        debug!("Hooks: Detected shebang. Launching Polyglot Bridge for {}/{}", hook, pkg);

        // 1. Create a temporary executable file for the script
        let mut tmp_script = NamedTempFile::new().map_err(Error::Io)?;
        tmp_script.write_all(code.as_bytes()).map_err(Error::Io)?;
        
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(tmp_script.path())?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(tmp_script.path(), perms)?;
        }

        // 2. Prepare Command with Injected Context
        let mut cmd = Command::new(tmp_script.path());
        cmd.env("LINIX_PKG_NAME", pkg)
           .env("LINIX_HOOK_TYPE", hook)
           .env("LINIX_OS", std::env::consts::OS)
           .env("LINIX_ARCH", std::env::consts::ARCH)
           .env("LINIX_USER", std::env::var("USER").unwrap_or_else(|_| "unknown".to_string()));

        // 3. Execute and capture status
        let status = cmd.status().map_err(|e| Error::Other(format!("Polyglot execution failed: {}", e)))?;

        if !status.success() {
            return Err(Error::LuaScript(format!("External hook failed with exit code: {:?}", status.code())));
        }

        Ok(())
    }

    /// Executes a specific hook trigger for a package.
    /// Dispatch Logic:
    /// 1. If code starts with #!, use Polyglot Bridge.
    /// 2. If code starts with #rhai, use Rhai Engine.
    /// 3. Otherwise, use Lua Engine (Default).
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

    /// Logic for Rhai script execution.
    fn run_rhai(&self, code: &str, hook: &str, pkg: &str) -> Result<()> {
        debug!("Hooks: Executing Rhai hook '{}' for '{}'", hook, pkg);
        let mut scope = Scope::new();
        scope.push("PKG_NAME", pkg.to_string());
        scope.push("HOOK_TYPE", hook.to_string());

        self.rhai_engine.run_with_scope(&mut scope, code)
            .map_err(|e| Error::LuaScript(format!("Rhai execution error: {}", e)))?;
        Ok(())
    }

    /// Logic for Lua script execution.
    async fn run_lua(&self, code: &str, hook: &str, pkg: &str) -> Result<()> {
        debug!("Hooks: Executing Lua hook '{}' for '{}'", hook, pkg);
        let lua = self.lua.lock().map_err(|e| Error::Other(e.to_string()))?;
        
        lua.globals().set("PKG_NAME", pkg).map_err(|e| Error::LuaScript(e.to_string()))?;
        lua.globals().set("HOOK_TYPE", hook).map_err(|e| Error::LuaScript(e.to_string()))?;

        lua.load(code).exec().map_err(|e| Error::LuaScript(e.to_string()))?;
        Ok(())
    }

    fn setup_lua_sandbox(lua: &Lua) -> Result<()> {
        let globals = lua.globals();
        globals.set("OS", std::env::consts::OS).map_err(|e| Error::LuaScript(e.to_string()))?;
        globals.set("ARCH", std::env::consts::ARCH).map_err(|e| Error::LuaScript(e.to_string()))?;
        
        let print_proxy = lua.create_function(|_, args: mlua::MultiValue| {
            let output: Vec<String> = args.iter().map(|v| format!("{:?}", v).trim_matches('"').to_string()).collect();
            info!("[Lua] {}", output.join(" "));
            Ok(())
        })?;
        globals.set("print", print_proxy).map_err(|e| Error::LuaScript(e.to_string()))?;
        Ok(())
    }

    fn setup_rhai_sandbox(engine: &mut Engine) {
        engine.register_fn("print", |msg: &str| info!("[Rhai] {}", msg));
        engine.register_fn("exec", |cmd: &str| {
            let _ = Command::new("sh").arg("-c").arg(cmd).status();
        });
    }

    pub fn render_template(&self, template: &str) -> String {
        let lua = self.lua.lock().expect("Failed to acquire Lua lock");
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