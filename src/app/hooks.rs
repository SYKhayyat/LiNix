use crate::config::Config;
use crate::core::{Error, Result};
use mlua::{Lua, MultiValue};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info};

pub struct LuaHooks {
    lua: Arc<Mutex<Lua>>,
    hooks: HashMap<String, HashMap<String, String>>,
}

impl LuaHooks {
    pub fn new(config: &Config) -> Result<Self> {
        let lua = Lua::new();
        Self::setup_sandbox(&lua)?;
        Ok(Self {
            lua: Arc::new(Mutex::new(lua)),
            hooks: config.hooks.clone(),
        })
    }

    fn setup_sandbox(lua: &Lua) -> Result<()> {
        lua.scope(|scope| {
            let safe_print = scope.create_function(|_, args: MultiValue| {
                let output: Vec<String> = args.iter().map(|v| format!("{:?}", v)).collect();
                println!("[Lua] {}", output.join(" "));
                Ok(())
            })?;
            lua.globals().set("print", safe_print)?;
            Ok(())
        })?;

        lua.load(
            r#"
            local original_os = os
            os = {
                execute = function(cmd)
                    print("Executing: " .. cmd)
                    return original_os.execute(cmd)
                end,
                getenv = original_os.getenv,
                date = original_os.date,
                time = original_os.time,
                clock = original_os.clock,
                difftime = original_os.difftime,
            }
            io = nil
            loadfile = nil
            dofile = nil
            load = nil
            rawget = nil
            rawset = nil
            rawequal = nil
            collectgarbage = nil
            getmetatable = nil
            setmetatable = nil
            debug = nil
        "#,
        )
        .exec()
        .map_err(|e| Error::LuaScript(format!("Failed to set up sandbox: {}", e)))?;

        Ok(())
    }

    pub async fn run_hook(&self, hook_name: &str, package: &str) -> Result<()> {
        let script = match self.hooks.get(hook_name) {
            Some(package_hooks) => match package_hooks.get(package) {
                Some(script) => script.clone(),
                None => return Ok(()),
            },
            None => return Ok(()),
        };

        info!("Running {} hook for {}", hook_name, package);
        debug!("Hook script: {}", script);

        let lua = self.lua.lock().await;
        lua.globals().set("PACKAGE", package).map_err(|e| Error::LuaScript(e.to_string()))?;
        lua.globals().set("HOOK_NAME", hook_name).map_err(|e| Error::LuaScript(e.to_string()))?;
        lua.load(&script).exec().map_err(|e| {
            Error::LuaScript(format!("Hook {} for {} failed: {}", hook_name, package, e))
        })?;

        Ok(())
    }

    pub async fn run_hook_file(&self, hook_name: &str, package: &str, script_path: &std::path::Path) -> Result<()> {
        if !script_path.exists() {
            return Ok(());
        }
        let script = std::fs::read_to_string(script_path)?;
        info!("Running {} hook for {} from {:?}", hook_name, package, script_path);
        let lua = self.lua.lock().await;
        lua.globals().set("PACKAGE", package).map_err(|e| Error::LuaScript(e.to_string()))?;
        lua.globals().set("HOOK_NAME", hook_name).map_err(|e| Error::LuaScript(e.to_string()))?;
        lua.globals().set("SCRIPT_PATH", script_path.to_string_lossy().to_string())
            .map_err(|e| Error::LuaScript(e.to_string()))?;
        lua.load(&script).exec().map_err(|e| {
            Error::LuaScript(format!("Hook {} for {} failed: {}", hook_name, package, e))
        })?;
        Ok(())
    }

    pub async fn run_before_sync(&self) -> Result<()> {
        self.run_hook("before_sync", "*").await
    }

    pub async fn run_after_sync(&self) -> Result<()> {
        self.run_hook("after_sync", "*").await
    }

    pub fn has_hook(&self, hook_name: &str, package: &str) -> bool {
        self.hooks
            .get(hook_name)
            .map(|h| h.contains_key(package))
            .unwrap_or(false)
    }

    pub fn packages_with_hook(&self, hook_name: &str) -> Vec<String> {
        self.hooks
            .get(hook_name)
            .map(|h| h.keys().cloned().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_config() -> Config {
        let mut hooks = HashMap::new();
        let mut before_install = HashMap::new();
        before_install.insert(
            "test-package".to_string(),
            r#"print("Installing test-package")"#.to_string(),
        );
        hooks.insert("before_install".to_string(), before_install);
        Config {
            hooks,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_hooks_creation() {
        let config = create_test_config();
        let hooks = LuaHooks::new(&config).unwrap();
        assert!(hooks.has_hook("before_install", "test-package"));
        assert!(!hooks.has_hook("before_install", "other-package"));
    }

    #[tokio::test]
    async fn test_hook_execution() {
        let config = create_test_config();
        let hooks = LuaHooks::new(&config).unwrap();
        let result = hooks.run_hook("before_install", "test-package").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_missing_hook() {
        let config = create_test_config();
        let hooks = LuaHooks::new(&config).unwrap();
        let result = hooks.run_hook("before_install", "nonexistent").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_before_sync_hook() {
        let mut config = create_test_config();
        let mut before_sync_map = HashMap::new();
        before_sync_map.insert("*".to_string(), r#"print("Before sync")"#.to_string());
        config.hooks.insert("before_sync".to_string(), before_sync_map);
        let hooks = LuaHooks::new(&config).unwrap();
        let result = hooks.run_before_sync().await;
        assert!(result.is_ok());
    }
}