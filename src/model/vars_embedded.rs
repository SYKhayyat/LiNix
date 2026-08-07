//! The embedded `vars.linix` provider (Part IX): a script LiNix runs in-process to produce
//! `name → value` pairs, in a language it ships so a fleet resolves identically with nothing to
//! install. Rhai is the engine, behind the neutral `vars.linix` extension so it can be replaced
//! without renaming anyone's files.
//!
//! **The engine is not sandboxed, and that is the ruling.** A stock Rhai `Engine` has no file,
//! shell, clock or network access; `core::rhai_stdlib` puts all four back, always on, because
//! II.6b decided `vars.linix` is *"trusted the same as a hook — a script in your own repo"* and
//! gave it every power an external `vars.py` already had. A `#rhai` hook builds its engine from
//! that same function, so the two are one language and cannot drift apart.
//!
//! **What makes that safe is the ledger, not the engine.** The file is hashed into `locks/` and
//! goes through II.12: first sight asks, a changed hash stops, and under `-y` or with no
//! terminal an unapproved provider is a refusal rather than a skipped prompt. That matters more
//! here than for a hook, because this file resolves at **step 0** of II.7 — before a plan
//! exists — so `check`, `plan` and even `plan --dry-run` have already run it by the time they
//! print anything. *"I only previewed it"* is not a state in which this script has not run.

use crate::config::grammar::{GrammarError, Origin, Result};
use crate::config::parser::HostFacts;
use crate::model::vars::{Value, VarOrigins, Vars};
use rhai::{Dynamic, Scope};
use std::path::Path;

/// Run `vars.linix` and turn the map it evaluates to into resolved variables.
///
/// The script is handed the machine's detected facts as the constants `OS`, `ARCH`, `HOST` and
/// `FAMILY`, and must end in a map: `#{ role: "travel", cores: 8 }`. The map's values are the
/// four types (string, number, boolean, list); a map value, or a script that does not end in a
/// map, is an error naming the file.
pub fn resolve(path: &Path, facts: &HostFacts) -> Result<Vars> {
    resolve_with_origins(path, facts).map(|(v, _)| v)
}

/// [`resolve`], plus where each variable came from. A script has no lines to attribute, so every
/// name points at the script file itself (W11/W12).
pub fn resolve_with_origins(path: &Path, facts: &HostFacts) -> Result<(Vars, VarOrigins)> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("vars.linix")
        .to_string();
    let origin = Origin::new(name.clone(), 0);

    let code = std::fs::read_to_string(path).map_err(|e| {
        GrammarError::new(origin.clone(), format!("could not read `{}`: {}", name, e))
    })?;

    let engine = crate::core::rhai_stdlib::engine("vars");

    let mut scope = Scope::new();
    scope.push_constant("OS", facts.os.clone());
    scope.push_constant("ARCH", facts.arch.clone());
    scope.push_constant("HOST", facts.host.clone());
    scope.push_constant("FAMILY", facts.family.clone());

    let map = engine
        .eval_with_scope::<rhai::Map>(&mut scope, &code)
        .map_err(|e| {
            GrammarError::new(origin.clone(), format!("`{}` did not run: {}", name, e)).with_hint(
                "the script must end in a map of name → value, e.g. `#{ role: \"travel\" }`.",
            )
        })?;

    let mut vars = Vars::new();
    let mut origins = VarOrigins::new();
    for (key, value) in map {
        let key = key.to_string();
        valid_name(&key, &origin)?;
        origins.insert(key.clone(), origin.clone());
        vars.insert(key, dynamic_to_value(value, &origin)?);
    }
    Ok((vars, origins))
}

/// Rhai's types map onto ours; a map or a `()` is refused, because a variable is a scalar or a
/// list and a comparison would not know what to do with either.
fn dynamic_to_value(d: Dynamic, origin: &Origin) -> Result<Value> {
    if d.is_bool() {
        return Ok(Value::Bool(d.as_bool().unwrap_or(false)));
    }
    if d.is_int() {
        return Ok(Value::Num(d.as_int().unwrap_or(0) as f64));
    }
    if d.is_float() {
        return Ok(Value::Num(d.as_float().unwrap_or(0.0)));
    }
    if d.is_string() {
        return Ok(Value::Str(d.into_string().unwrap_or_default()));
    }
    if d.is_array() {
        let items = d.into_array().unwrap_or_default();
        return Ok(Value::List(
            items
                .into_iter()
                .map(|e| dynamic_to_value(e, origin))
                .collect::<Result<Vec<_>>>()?,
        ));
    }
    Err(GrammarError::new(
        origin.clone(),
        format!(
            "a variable cannot be a {}; it is a string, number, boolean, or list",
            d.type_name()
        ),
    ))
}

/// A variable name is an identifier, so it can be read back as `$name`.
fn valid_name(name: &str, origin: &Origin) -> Result<()> {
    let ok = !name.is_empty()
        && name.starts_with(|c: char| c.is_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(GrammarError::new(
            origin.clone(),
            format!("`{}` is not a valid variable name", name),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> HostFacts {
        HostFacts {
            os: "linux".into(),
            arch: "x86_64".into(),
            host: "laptop".into(),
            family: "debian".into(),
            vars: Default::default(),
        }
    }

    fn run(body: &str) -> Result<Vars> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        // `env::temp_dir()`, not TMP-or-TMPDIR-or-".": neither variable is set in a plain
        // Linux shell, so the fallback was the current directory — which is the repo, and
        // every `cargo test` left a pile of `linix-embedded-*.linix` in it.
        let path = std::env::temp_dir().join(format!(
            "linix-embedded-{}-{}.linix",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, body).unwrap();
        resolve(&path, &facts())
    }

    #[test]
    fn a_map_of_the_four_types_resolves() {
        let vars = run(r#"#{ role: "travel", cores: 8, ratio: 1.5, gpu: true, tags: ["a", "b"] }"#)
            .unwrap();
        assert_eq!(vars["role"], Value::Str("travel".into()));
        assert_eq!(vars["cores"], Value::Num(8.0));
        assert_eq!(vars["ratio"], Value::Num(1.5));
        assert_eq!(vars["gpu"], Value::Bool(true));
        assert_eq!(
            vars["tags"],
            Value::List(vec![Value::Str("a".into()), Value::Str("b".into())])
        );
    }

    #[test]
    fn the_script_can_read_the_detected_facts() {
        let vars = run(r#"#{ here: OS, machine: HOST }"#).unwrap();
        assert_eq!(vars["here"], Value::Str("linux".into()));
        assert_eq!(vars["machine"], Value::Str("laptop".into()));
    }

    #[test]
    fn the_script_can_compute() {
        // The whole point over the line file: real logic decides the value.
        let vars = run(r#"
            let role = if HOST == "laptop" { "travel" } else { "desktop" };
            #{ role: role }
        "#)
        .unwrap();
        assert_eq!(vars["role"], Value::Str("travel".into()));
    }

    #[test]
    fn a_script_that_does_not_end_in_a_map_is_an_error() {
        let err = run("42").unwrap_err();
        assert!(err.hint.as_deref().unwrap_or("").contains("map"), "{}", err);
    }

    #[test]
    fn a_map_valued_variable_is_refused() {
        let err = run(r#"#{ nested: #{ a: 1 } }"#).unwrap_err();
        assert!(err.what.contains("cannot be a"), "{}", err);
    }

    #[test]
    fn the_clock_is_available() {
        let vars = run(r#"#{ t: now(), wd: weekday() }"#).unwrap();
        assert!(matches!(vars["t"], Value::Num(n) if n > 1_600_000_000.0));
        assert!(matches!(&vars["wd"], Value::Str(s) if !s.is_empty()));
    }

    #[test]
    fn the_environment_is_readable_and_is_w7s_escape_hatch() {
        std::env::set_var("LINIX_TEST_ROLE", "work");
        let vars = run(
            r#"#{ role: env("LINIX_TEST_ROLE"), missing: env("LINIX_NOPE", "default"), present: has_env("LINIX_TEST_ROLE") }"#,
        )
        .unwrap();
        std::env::remove_var("LINIX_TEST_ROLE");
        assert_eq!(vars["role"], Value::Str("work".into()));
        assert_eq!(vars["missing"], Value::Str("default".into()));
        assert_eq!(vars["present"], Value::Bool(true));
    }

    #[test]
    fn a_file_can_be_read_and_probed() {
        let marker = std::env::temp_dir().join(format!("linix-marker-{}", std::process::id()));
        std::fs::write(&marker, "gpu").unwrap();
        let script = format!(
            r#"#{{ here: path_exists("{p}"), body: read_file("{p}") }}"#,
            p = marker.display().to_string().replace('\\', "\\\\")
        );
        let vars = run(&script).unwrap();
        assert_eq!(vars["here"], Value::Bool(true));
        assert_eq!(vars["body"], Value::Str("gpu".into()));
    }

    #[test]
    fn a_read_of_a_missing_file_throws_rather_than_resolving_to_nothing() {
        let err = run(r#"#{ x: read_file("/no/such/file/anywhere") }"#).unwrap_err();
        assert!(err.what.contains("read_file"), "{}", err);
    }

    #[test]
    fn the_shell_runs_and_a_check_variant_does_not_throw() {
        let (echo, ok_cmd, bad_cmd) = if cfg!(windows) {
            ("echo hi", "cmd /c exit 0", "cmd /c exit 1")
        } else {
            ("echo hi", "true", "false")
        };
        let script = format!(
            r#"#{{ out: sh("{}"), good: sh_ok("{}"), bad: sh_ok("{}") }}"#,
            echo, ok_cmd, bad_cmd
        );
        let vars = run(&script).unwrap();
        assert_eq!(vars["out"], Value::Str("hi".into()));
        assert_eq!(vars["good"], Value::Bool(true));
        assert_eq!(vars["bad"], Value::Bool(false));
    }

    #[test]
    fn a_failing_shell_command_throws() {
        let cmd = if cfg!(windows) {
            "cmd /c exit 2"
        } else {
            "exit 2"
        };
        let err = run(&format!(r#"#{{ x: sh("{}") }}"#, cmd)).unwrap_err();
        assert!(err.what.contains("sh:"), "{}", err);
    }

    #[test]
    fn json_can_be_parsed_and_navigated_for_a_scalar() {
        let vars = run(r#"#{ v: parse_json(`{"a": {"b": 42}}`).a.b }"#).unwrap();
        assert_eq!(vars["v"], Value::Num(42.0));
    }

    #[test]
    fn an_http_get_to_a_dead_address_throws() {
        // Port 1 refuses fast; no live network needed. Proves failure is loud, not empty.
        let err = run(r#"#{ x: http_get("http://127.0.0.1:1/") }"#).unwrap_err();
        assert!(
            err.what.contains("did not run") || err.what.contains("http"),
            "{}",
            err
        );
    }
}
