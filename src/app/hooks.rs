//! The three hook dialects (owner ruling, 2026-07-20: all three stay), chosen by a script's
//! first line — a shebang runs it as a process, `#rhai` runs it in-process, anything else is
//! Lua.
//!
//! **They are one feature in three notations, so they get the same things.** All three are
//! handed the same four facts ([`LuaHooks::hook_facts`]), and the `#rhai` arm builds its engine
//! from `core::rhai_stdlib` — the same one `vars.linix` uses, because II.6b already ruled that
//! file *"trusted the same as a hook"* and a hook may not have less than the thing defined by
//! reference to it. It previously had `print` and nothing else, which made the shipped example
//! config's `exec("systemctl enable docker")` a call into an empty room.
//!
//! **None of the three is sandboxed, and none of them ever was.** `Lua::new` loads `os.execute`;
//! a shebang is a process; Rhai now has `sh`. The gate is II.12's ledger — every script hashed,
//! an unapproved or changed one stops the sync, and `-y` cannot skip it.

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

/// Which language a hook is written in. One place decides, and it hands back the body to run
/// with it — because the marker that chose the dialect is not always part of the script.
enum Dialect {
    /// A shebang: written to a file and executed. The shebang line stays — it is what tells the
    /// kernel which interpreter to use, so removing it would break the thing it selected.
    Process,
    Rhai,
    Lua,
}

impl Dialect {
    /// The dialect, and the script to hand the engine that runs it.
    ///
    /// **`#rhai` is not Rhai.** `#` is a reserved symbol there, so a marker left in place is a
    /// syntax error on line 1 and the script never runs — which is what every `#rhai` hook got.
    /// The marker's line is blanked rather than deleted so that a runtime error still names the
    /// line the author wrote.
    fn of(code: &str) -> (Dialect, String) {
        let lead = code.len() - code.trim_start().len();
        let trimmed = &code[lead..];

        if trimmed.starts_with("#!") {
            return (Dialect::Process, code.to_string());
        }
        if trimmed.starts_with("#rhai") {
            let rest_of_script = trimmed.find('\n').map(|i| &trimmed[i..]).unwrap_or("");
            return (Dialect::Rhai, format!("{}{}", &code[..lead], rest_of_script));
        }
        (Dialect::Lua, code.to_string())
    }
}

pub struct LuaHooks {
    rhai_engine: Engine,
    pub hooks: HashMap<String, HashMap<String, String>>,
    /// The repo's `locks/` directory, where the hook approval ledger lives (II.12).
    locks_dir: PathBuf,
}

impl LuaHooks {
    pub fn new(config: &Config) -> Result<Self> {
        Ok(Self {
            rhai_engine: crate::core::rhai_stdlib::engine("hook"),
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
        for (name, value) in Self::hook_facts(hook, pkg) {
            cmd.env(format!("LINIX_{}", name), value);
        }

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
            match Dialect::of(code) {
                (Dialect::Process, body) => {
                    self.run_external_polyglot(&body, hook_name, package_name)
                        .await?
                }
                (Dialect::Rhai, body) => self.run_rhai(&body, hook_name, package_name)?,
                (Dialect::Lua, body) => self.run_lua(&body, hook_name, package_name).await?,
            }
        }

        Ok(())
    }

    fn run_rhai(&self, code: &str, hook: &str, pkg: &str) -> Result<()> {
        let mut scope = Scope::new();
        for (name, value) in Self::hook_facts(hook, pkg) {
            scope.push_constant(name, value);
        }

        self.rhai_engine
            .run_with_scope(&mut scope, code)
            .map_err(|e| Error::LuaScript(format!("Rhai execution error: {}", e)))?;
        Ok(())
    }

    /// What every hook knows about why it is running, whatever dialect it is written in. One
    /// list, because three dialects that each decide for themselves is how the Rhai arm ended up
    /// unable to ask which OS it was on while the Lua and `#!` arms both could.
    fn hook_facts(hook: &str, pkg: &str) -> [(&'static str, String); 4] {
        [
            ("PKG_NAME", pkg.to_string()),
            ("HOOK_TYPE", hook.to_string()),
            ("OS", std::env::consts::OS.to_string()),
            ("ARCH", std::env::consts::ARCH.to_string()),
        ]
    }

    /// The Lua interpreter must be constructed INSIDE the blocking closure: `mlua::Lua` is
    /// !Send, so holding one across this boundary will not compile.
    async fn run_lua(&self, code: &str, hook: &str, pkg: &str) -> Result<()> {
        let code_owned = code.to_string();
        let hook_owned = hook.to_string();
        let pkg_owned = pkg.to_string();

        tokio::task::spawn_blocking(move || {
            let lua = Lua::new();
            Self::register_lua_host_functions(&lua)?;

            for (name, value) in Self::hook_facts(&hook_owned, &pkg_owned) {
                lua.globals().set(name, value).map_err(Error::from)?;
            }

            lua.load(&code_owned).exec().map_err(Error::from)?;
            Ok::<(), Error>(())
        })
        .await
        .map_err(|e| Error::Other(e.to_string()))??;

        Ok(())
    }

    /// A Lua hook's `print` goes to the log rather than to a stdout the sync is already using.
    /// It was called `setup_lua_sandbox`, which claimed a boundary this does not build: `Lua::new`
    /// loads the standard library, so `os.execute` and `io` are live in every Lua hook. What gates
    /// a hook is II.12's ledger, not the interpreter.
    fn register_lua_host_functions(lua: &Lua) -> Result<()> {
        let globals = lua.globals();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `LuaHooks` over one `after_install` script for `pkg`. No ledger involved: approval is
    /// `verify_all_approved`'s job and is tested where it lives.
    fn hooks_with(script: &str) -> LuaHooks {
        let mut config = Config::default();
        config.hooks.insert(
            "after_install".to_string(),
            HashMap::from([("pkg".to_string(), script.to_string())]),
        );
        LuaHooks::new(&config).expect("hooks")
    }

    /// A command that succeeds, and one that fails, in whichever shell `sh` reaches for.
    fn commands() -> (&'static str, &'static str) {
        if cfg!(windows) {
            ("cmd /c exit 0", "cmd /c exit 3")
        } else {
            ("true", "exit 3")
        }
    }

    #[tokio::test]
    async fn a_rhai_hook_can_reach_the_shell() {
        // The bug: this arm had `print` and nothing else, so the shipped example config's
        // `exec("systemctl enable docker")` — and any `sh` — was a call to a function no engine
        // in this binary had ever registered.
        let (ok, _) = commands();
        let hooks = hooks_with(&format!("#rhai\nlet ran = sh_ok(\"{}\");", ok));
        hooks.run_hook("after_install", "pkg").await.unwrap();
    }

    #[tokio::test]
    async fn a_rhai_hook_reaches_the_whole_standard_library_vars_has() {
        // Not just the shell: II.6b's "trusted the same as a hook" is a two-way statement, so
        // every provider `vars.linix` has, a hook has. One test per family, not one per member.
        let hooks = hooks_with(
            r#"#rhai
            if weekday() == "" { throw "no clock" }
            if !has_env("PATH") { throw "no environment" }
            if path_exists("/definitely/not/here") { throw "no filesystem" }
            if parse_json(`{"a": 1}`).a != 1 { throw "no json" }
            "#,
        );
        hooks.run_hook("after_install", "pkg").await.unwrap();
    }

    #[tokio::test]
    async fn a_failing_command_in_a_rhai_hook_fails_the_hook() {
        // `sh` throws where `sh_ok` answers. A hook that swallowed a failed command would report
        // an install as configured when it was not.
        let (_, bad) = commands();
        let hooks = hooks_with(&format!("#rhai\nsh(\"{}\");", bad));
        let err = hooks.run_hook("after_install", "pkg").await.unwrap_err();
        assert!(err.to_string().contains("sh:"), "{}", err);
    }

    #[test]
    fn every_dialect_is_handed_the_same_four_facts() {
        let facts = LuaHooks::hook_facts("after_install", "ripgrep");
        let names: Vec<&str> = facts.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, ["PKG_NAME", "HOOK_TYPE", "OS", "ARCH"]);
        assert_eq!(facts[0].1, "ripgrep");
        assert_eq!(facts[1].1, "after_install");
        for (name, value) in &facts {
            assert!(!value.is_empty(), "{} is empty", name);
        }
    }

    #[tokio::test]
    async fn a_rhai_hook_knows_all_four() {
        // The sibling nobody reported: Lua and `#!` were handed OS and ARCH, Rhai was not, so a
        // cross-platform hook could not branch in one of the three dialects.
        let hooks = hooks_with(
            r#"#rhai
            if PKG_NAME != "pkg" { throw "no PKG_NAME" }
            if HOOK_TYPE != "after_install" { throw "no HOOK_TYPE" }
            if OS == "" { throw "no OS" }
            if ARCH == "" { throw "no ARCH" }
            "#,
        );
        hooks.run_hook("after_install", "pkg").await.unwrap();
    }

    #[tokio::test]
    async fn a_lua_hook_knows_all_four() {
        let hooks = hooks_with(
            r#"
            assert(PKG_NAME == "pkg", "no PKG_NAME")
            assert(HOOK_TYPE == "after_install", "no HOOK_TYPE")
            assert(OS ~= nil and OS ~= "", "no OS")
            assert(ARCH ~= nil and ARCH ~= "", "no ARCH")
            "#,
        );
        hooks.run_hook("after_install", "pkg").await.unwrap();
    }

    #[test]
    fn the_rhai_marker_is_not_handed_to_the_engine_but_the_shebang_is() {
        // `#rhai` is a LiNix marker; `#!` is the script's own first instruction. Confusing the
        // two either way breaks a dialect: a kept `#rhai` is a syntax error on line 1, and a
        // stripped `#!` leaves the kernel with no interpreter to run.
        let (dialect, body) = Dialect::of("#rhai\nlet x = 1;\n");
        assert!(matches!(dialect, Dialect::Rhai));
        assert!(!body.contains("#rhai"), "the marker survived: {:?}", body);

        let (dialect, body) = Dialect::of("#!/bin/sh\necho hi\n");
        assert!(matches!(dialect, Dialect::Process));
        assert!(body.starts_with("#!/bin/sh"), "the shebang was stripped");

        let (dialect, body) = Dialect::of("print('hi')");
        assert!(matches!(dialect, Dialect::Lua));
        assert_eq!(body, "print('hi')");
    }

    #[test]
    fn stripping_the_marker_does_not_move_the_lines_under_it() {
        // An error that names the wrong line is how a one-line offset survives review.
        let (_, body) = Dialect::of("#rhai\nlet a = 1;\nlet b = 2;\n");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines[0], "", "line 1 should be blank, not gone");
        assert_eq!(lines[1], "let a = 1;");
        assert_eq!(lines[2], "let b = 2;");
    }

    #[tokio::test]
    async fn a_rhai_hook_indented_in_a_toml_block_still_runs() {
        // TOML multi-line strings routinely arrive with a leading newline and indentation, which
        // is exactly how the example config writes them.
        let (ok, _) = commands();
        let hooks = hooks_with(&format!("\n  #rhai\n  let ran = sh_ok(\"{}\");\n", ok));
        hooks.run_hook("after_install", "pkg").await.unwrap();
    }

    #[tokio::test]
    async fn the_dialect_is_chosen_by_the_first_line() {
        // `#rhai` is Rhai and everything else is Lua, so a script written in one dialect must not
        // silently parse as the other. `..` concatenates in Lua and is a range in Rhai.
        let lua = hooks_with(r#"local s = "a" .. "b""#);
        lua.run_hook("after_install", "pkg").await.unwrap();

        let rhai = hooks_with("#rhai\nlet s = \"a\" + \"b\";");
        rhai.run_hook("after_install", "pkg").await.unwrap();
    }

    #[test]
    fn the_shipped_example_config_calls_functions_that_exist() {
        // The bug reached a user through `examples/preferences.toml`, which documented
        // `exec(...)`. A doc that names a function nothing registers is the same defect one
        // layer out, so the example is pinned to the shell's real name.
        let example = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/preferences.toml"),
        )
        .expect("examples/preferences.toml");
        assert!(
            !example.contains("exec("),
            "the example config calls `exec(`, which no engine registers; the shell is `sh(`"
        );
        assert!(
            example.contains("sh(\"systemctl enable docker\")"),
            "the example's Rhai hook should demonstrate the real shell function"
        );
    }
}
