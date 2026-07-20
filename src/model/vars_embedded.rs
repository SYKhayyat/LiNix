//! The embedded `vars.linix` provider (Part IX): a script LiNix runs in-process to produce
//! `name → value` pairs, in a language it ships so a fleet resolves identically with nothing to
//! install. Rhai is the engine, behind the neutral `vars.linix` extension so it can be replaced
//! without renaming anyone's files.
//!
//! The engine is pure by construction: a stock Rhai `Engine` has no file, shell, clock or network
//! access at all. The host powers IX.6 rules the provider *may* have — reading the clock, running
//! a shell, reaching the network — are a separate, owner-decided standard library, registered on
//! top of this; until that is decided, the only inputs a script has are the detected facts below.

use crate::config::grammar::{GrammarError, Origin, Result};
use crate::config::parser::HostFacts;
use crate::model::vars::{Value, Vars};
use rhai::{Dynamic, Engine, Scope};
use std::path::Path;

/// A runaway script must not hang every `plan` and `sync`. Ten million operations is far more
/// than any variable computation needs and far less than a wedged infinite loop reaches.
const MAX_OPERATIONS: u64 = 10_000_000;

/// Run `vars.linix` and turn the map it evaluates to into resolved variables.
///
/// The script is handed the machine's detected facts as the constants `OS`, `ARCH`, `HOST` and
/// `FAMILY`, and must end in a map: `#{ role: "travel", cores: 8 }`. The map's values are the
/// four types (string, number, boolean, list); a map value, or a script that does not end in a
/// map, is an error naming the file.
pub fn resolve(path: &Path, facts: &HostFacts) -> Result<Vars> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("vars.linix")
        .to_string();
    let origin = Origin::new(name.clone(), 0);

    let code = std::fs::read_to_string(path).map_err(|e| {
        GrammarError::new(origin.clone(), format!("could not read `{}`: {}", name, e))
    })?;

    let mut engine = Engine::new();
    engine.set_max_operations(MAX_OPERATIONS);
    engine.register_fn("print", |msg: &str| tracing::info!("[vars] {}", msg));

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
    for (key, value) in map {
        let key = key.to_string();
        valid_name(&key, &origin)?;
        vars.insert(key, dynamic_to_value(value, &origin)?);
    }
    Ok(vars)
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
        let base = std::env::var("TMP")
            .or_else(|_| std::env::var("TMPDIR"))
            .unwrap_or_else(|_| ".".to_string());
        let path = std::path::PathBuf::from(base).join(format!(
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
        assert_eq!(vars["tags"], Value::List(vec![Value::Str("a".into()), Value::Str("b".into())]));
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
        let vars = run(
            r#"
            let role = if HOST == "laptop" { "travel" } else { "desktop" };
            #{ role: role }
        "#,
        )
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
    fn the_engine_has_no_file_or_shell_access() {
        // A stock Rhai engine defines no such functions, so a script that tries is a run error,
        // not a silent success. This is what makes the provider pure until the stdlib is decided.
        assert!(run(r#"#{ x: open_file("/etc/passwd") }"#).is_err());
    }
}
