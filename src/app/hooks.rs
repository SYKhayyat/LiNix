// src/app/hooks.rs
use crate::config::Config;
use crate::core::{Error, Result};
use mlua::Lua;
use std::collections::HashMap;
use std::sync::{Arc, Mutex}; // Use std Mutex for !Send types

pub struct LuaHooks {
    // We use a standard Mutex here to ensure we can manually 
    // control thread access to the !Send Lua state.
    lua: Arc<Mutex<Lua>>,
    pub hooks: HashMap<String, HashMap<String, String>>,
}

// Safety: We guarantee that Lua is only accessed via the Mutex
// and we do not hold the lock across any .await points.
unsafe impl Send for LuaHooks {}
unsafe impl Sync for LuaHooks {}

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
        let globals = lua.globals();
        let safe_print = lua.create_function(|_, args: mlua::MultiValue| {
            let output: Vec<String> = args.iter().map(|v| format!("{:?}", v)).collect();
            println!("[Lua] {}", output.join(" "));
            Ok(())
        })?;
        globals.set("print", safe_print).map_err(|e| Error::LuaScript(e.to_string()))?;
        Ok(())
    }

    pub fn render_template(&self, template: &str) -> String {
    let lua = self.lua.lock().unwrap();
    let globals = lua.globals();
    
    // Inject system variables for the template to use
    let _ = globals.set("OS", std::env::consts::OS);
    let _ = globals.set("ARCH", std::env::consts::ARCH);
    let _ = globals.set("USER", std::env::var("USER").unwrap_or_default());

    // REAL LOGIC: If template contains {{...}}, treat it as a Lua expression
    let re = regex::Regex::new(r"\{\{(.*?)\}\}").unwrap();
    re.replace_all(template, |caps: &regex::Captures| {
        let code = &caps[1];
        lua.load(code).eval::<String>().unwrap_or_else(|_| caps[0].to_string())
    }).to_string()
}

    pub async fn run_hook(&self, hook_name: &str, package: &str) -> Result<()> {
        if let Some(h) = self.hooks.get(hook_name).and_then(|h| h.get(package)) {
            // We lock, run the script, and release the lock immediately.
            let lua = self.lua.lock().map_err(|e| Error::Other(e.to_string()))?;
            lua.load(h).exec().map_err(|e| Error::LuaScript(e.to_string()))?;
        }
        Ok(())
    }

    pub async fn run_before_sync(&self) -> Result<()> { self.run_hook("before_sync", "*").await }
    pub async fn run_after_sync(&self) -> Result<()> { self.run_hook("after_sync", "*").await }
}