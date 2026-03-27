use crate::config::Config;
use crate::core::{Error, Result};
use mlua::{Lua, MultiValue};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info};

/// Lua hooks manager for pre/post install scripts
pub struct LuaHooks {
    lua: Arc<Mutex<Lua>>,
    hooks: HashMap<String, HashMap<String, String>>,
}

impl LuaHooks {
    /// Create a new hooks manager
    pub fn new(config: &Config) -> Result<Self> {
        let lua = Lua::new();

        // Set up sandbox environment
        Self::setup_sandbox(&lua)?;

        Ok(Self {
            lua: Arc::new(Mutex::new(lua)),
            hooks: config.hooks.clone(),
        })
    }

    /// Set up a sandboxed Lua environment
    fn setup_sandbox(lua: &Lua) -> Result<()> {
        lua.scope(|scope| {
            // Create safe print function
            let safe_print = scope.create_function(|_, args: MultiValue| {
                let output: Vec<String> = args.iter().map(|v| format!("{:?}", v)).collect();
                println!("[Lua] {}", output.join(" "));
                Ok(())
            })?;

            lua.globals().set("print", safe_print)?;
            Ok(())
        })?;

        // Add safe os.execute wrapper
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

            -- Remove dangerous functions
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

    /// Run a hook for a specific package
    pub async fn run_hook(&self, hook_name: &str, package: &str) -> Result<()> {
        // Check if hook exists for this package
        let script = match self.hooks.get(hook_name) {
            Some(package_hooks) => match package_hooks.get(package) {
                Some(script) => script.clone(),
                None => return Ok(()), // No hook for this package
            },
            None => return Ok(()), // No hooks of this type
        };

        info!("Running {} hook for {}", hook_name, package);
        debug!("Hook script: {}", script);

        let lua = self.lua.lock().await;

        // Set package name as global
        lua.globals()
            .set("PACKAGE", package)
            .map_err(|e| Error::LuaScript(e.to_string()))?;

        lua.globals()
            .set("HOOK_NAME", hook_name)
            .map_err(|e| Error::LuaScript(e.to_string()))?;

        // Execute script
        lua.load(&script).exec().map_err(|e| {
            Error::LuaScript(format!("Hook {} for {} failed: {}", hook_name, package, e))
        })?;

        Ok(())
    }

    /// Run a hook script from a file
    pub async fn run_hook_file(
        &self,
        hook_name: &str,
        package: &str,
        script_path: &std::path::Path,
    ) -> Result<()> {
        if !script_path.exists() {
            return Ok(());
        }

        let script = std::fs::read_to_string(script_path)?;

        info!(
            "Running {} hook for {} from {:?}",
            hook_name, package, script_path
        );

        let lua = self.lua.lock().await;

        lua.globals()
            .set("PACKAGE", package)
            .map_err(|e| Error::LuaScript(e.to_string()))?;

        lua.globals()
            .set("HOOK_NAME", hook_name)
            .map_err(|e| Error::LuaScript(e.to_string()))?;

        lua.globals()
            .set("SCRIPT_PATH", script_path.to_string_lossy().to_string())
            .map_err(|e| Error::LuaScript(e.to_string()))?;

        lua.load(&script).exec().map_err(|e| {
            Error::LuaScript(format!("Hook {} for {} failed: {}", hook_name, package, e))
        })?;

        Ok(())
    }

    /// Check if a hook exists for a package
    pub fn has_hook(&self, hook_name: &str, package: &str) -> bool {
        self.hooks
            .get(hook_name)
            .map(|h| h.contains_key(package))
            .unwrap_or(false)
    }

    /// Get all packages with hooks of a specific type
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

        // Should succeed (no-op) for missing hooks
        let result = hooks.run_hook("before_install", "nonexistent").await;
        assert!(result.is_ok());
    }
}
