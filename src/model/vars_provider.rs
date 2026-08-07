//! Choosing a `vars` provider, and running the external-executable one (Part IX).
//!
//! A provider produces `name → value` pairs; the file provider does it by resolving a line file,
//! this module does it by running a program the user wrote. Several provider files may sit in a
//! repo at once — `vars`, `vars.py`, `vars.linix` — and exactly one is active per machine, named
//! by `[vars] source` in `preferences.toml`. Two present and none chosen is a loud error, never a
//! guess about which one wins.

use crate::config::grammar::{GrammarError, Origin, Result};
use crate::config::parser::HostFacts;
use crate::model::vars::{Value, VarOrigins, Vars};
use std::path::{Path, PathBuf};

/// Which of the three providers a filename names. The kind is the filename, not a config key, so
/// what a file *is* is visible in the repo (`vars.py` is obviously a program).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `vars` — the built-in line file, resolved by [`crate::model::vars::resolve`].
    LineFile,
    /// `vars.linix` — the embedded script, run in-process.
    Embedded,
    /// `vars.<ext>` — an external executable, run by this module.
    External,
}

/// The active provider file and what kind it is.
#[derive(Debug, Clone)]
pub struct Selected {
    pub path: PathBuf,
    pub kind: Kind,
}

/// The kind a candidate filename names, or `None` if it is not a provider file at all.
fn kind_of(filename: &str) -> Option<Kind> {
    match filename {
        "vars" => Some(Kind::LineFile),
        "vars.linix" => Some(Kind::Embedded),
        _ if filename.starts_with("vars.") => Some(Kind::External),
        _ => None,
    }
}

/// Every provider file present in the repo, by filename. Directories are ignored — a `vars.d/`
/// directory is not a provider until W6 says it is.
fn discover(config_root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(config_root) else {
        return Vec::new();
    };
    let mut found: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| kind_of(name).is_some())
        .collect();
    found.sort();
    found
}

/// Pick the active provider. `source` is `[vars] source` from `preferences.toml`.
///
/// - A named source that is not present is an error: you asked for a provider that isn't there.
/// - No source, one provider file: use it.
/// - No source, several provider files: refuse, and list them — choosing silently would make the
///   resolved state depend on directory order.
/// - No provider file at all: `None`, which means no variables (not an error).
pub fn select(config_root: &Path, source: &Option<String>) -> Result<Option<Selected>> {
    let origin = || Origin::new("preferences.toml", 0);

    if let Some(name) = source {
        let Some(kind) = kind_of(name) else {
            return Err(GrammarError::new(
                origin(),
                format!(
                    "`[vars] source = \"{}\"` is not a variable provider name",
                    name
                ),
            )
            .with_hint(
                "name the built-in `vars`, an embedded `vars.linix`, or an external `vars.<ext>`.",
            ));
        };
        let path = config_root.join(name);
        if !path.is_file() {
            return Err(GrammarError::new(
                origin(),
                format!(
                    "`[vars] source = \"{}\"` names a file that is not in the repo",
                    name
                ),
            )
            .with_hint("create the file, or point `source` at a provider that exists."));
        }
        return Ok(Some(Selected { path, kind }));
    }

    let found = discover(config_root);
    match found.as_slice() {
        [] => Ok(None),
        [only] => Ok(Some(Selected {
            path: config_root.join(only),
            kind: kind_of(only).expect("discover only returns provider files"),
        })),
        many => Err(GrammarError::new(
            origin(),
            format!(
                "more than one variable provider is present: {}",
                many.join(", ")
            ),
        )
        .with_hint("set `[vars] source` in preferences.toml to choose which one is active.")),
    }
}

/// Run an external provider and parse its output into resolved variables.
///
/// The program is handed the machine's detected facts as `LINIX_OS`/`LINIX_ARCH`/`LINIX_HOST`/
/// `LINIX_FAMILY`, so it decides per machine without re-detecting them. Its stdout is a JSON
/// object of `name → value`, or `name = value` lines. A non-zero exit is an error carrying the
/// program's own stderr — a provider that fails must not silently resolve to nothing.
pub fn run_external(path: &Path, facts: &HostFacts) -> Result<Vars> {
    run_external_with_origins(path, facts).map(|(v, _)| v)
}

/// [`run_external`], plus where each variable came from. A `name = value` line carries its line
/// number; a JSON object has no lines, so every name points at the provider file (W11/W12).
pub fn run_external_with_origins(path: &Path, facts: &HostFacts) -> Result<(Vars, VarOrigins)> {
    let name = file_name(path);
    let origin = Origin::new(name.clone(), 0);
    let mut argv = interpreter_argv(path);
    argv.push(path.as_os_str().to_owned());

    let mut cmd = std::process::Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .stdin(std::process::Stdio::null())
        .env("LINIX_OS", &facts.os)
        .env("LINIX_ARCH", &facts.arch)
        .env("LINIX_HOST", &facts.host)
        .env("LINIX_FAMILY", &facts.family);

    // An external provider runs on every resolution, before any manager is asked — so a slow
    // one looks like LiNix being slow to start until the breakdown names it.
    let timing = crate::core::timing::begin();
    let output = cmd.output();
    crate::core::timing::end(timing, "vars provider", std::slice::from_ref(&name));

    let output = output.map_err(|e| {
        GrammarError::new(
            origin.clone(),
            format!("could not run the `{}` provider: {}", name, e),
        )
        .with_hint("an external provider needs its interpreter installed and on PATH.")
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        let tail = if detail.is_empty() {
            String::new()
        } else {
            format!(": {}", detail)
        };
        return Err(GrammarError::new(
            origin,
            format!(
                "the `{}` provider exited with {}{}",
                name, output.status, tail
            ),
        ));
    }

    let stdout = crate::utils::text::sanitize(&String::from_utf8_lossy(&output.stdout));
    parse_output(&stdout, &origin)
}

/// The just-the-filename an error should name.
fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("vars")
        .to_string()
}

/// The conventional interpreter for a provider's extension — `.py` is Python, `.js` is Node — so
/// a user writes `vars.py` and it runs without a shebang or a chmod, which is what IX.6 means by
/// "an executable run by LiNix". An unknown extension is run directly, trusting the OS.
///
/// The *name* here is a Unix spelling, and finding this machine's copy of it is
/// `model::script`'s question, not a second one: `.py` used to mean literally `python` on Windows
/// and literally `python3` everywhere else, so a Windows box with only `python3` and a Linux box
/// with only `python` each had a `vars.py` that could not run (Y17). A name that resolves to
/// nothing is left as written, so the failure is still the provider's own "could not run" with
/// the interpreter named in it.
fn interpreter_argv(path: &Path) -> Vec<std::ffi::OsString> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let words: &[&str] = match ext.as_str() {
        "sh" | "bash" => &["sh"],
        "py" => &["python3"],
        "js" | "mjs" | "cjs" => &["node"],
        "rb" => &["ruby"],
        "pl" => &["perl"],
        "ps1" => {
            if cfg!(windows) {
                &["powershell", "-File"]
            } else {
                &["pwsh", "-File"]
            }
        }
        "cmd" | "bat" => &["cmd", "/c"],
        _ => &[],
    };
    let mut argv = words
        .iter()
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
    if let Some(first) = argv.first_mut() {
        if let Some(found) = first
            .to_str()
            .and_then(crate::model::script::interpreter_named)
        {
            *first = std::ffi::OsString::from(found);
        }
    }
    argv
}

/// Parse a provider's stdout: a JSON object, or `name = value` lines. JSON carries its own types;
/// a `name = value` line is read with the same literal rules a `vars` line uses. Returns the
/// origin of each variable alongside it (W11/W12).
fn parse_output(stdout: &str, origin: &Origin) -> Result<(Vars, VarOrigins)> {
    let text = stdout.trim();
    if text.is_empty() {
        return Ok((Vars::new(), VarOrigins::new()));
    }
    if text.starts_with('{') {
        return parse_json_object(text, origin);
    }
    parse_pairs(stdout, origin)
}

fn parse_json_object(text: &str, origin: &Origin) -> Result<(Vars, VarOrigins)> {
    let json: serde_json::Value = serde_json::from_str(text).map_err(|e| {
        GrammarError::new(
            origin.clone(),
            format!("the provider's JSON did not parse: {}", e),
        )
    })?;
    let serde_json::Value::Object(map) = json else {
        return Err(GrammarError::new(
            origin.clone(),
            "the provider printed JSON that is not an object of name → value".to_string(),
        ));
    };
    let mut vars = Vars::new();
    let mut origins = VarOrigins::new();
    for (name, value) in map {
        check_name(&name, origin)?;
        origins.insert(name.clone(), origin.clone());
        vars.insert(name, json_to_value(&value, origin)?);
    }
    Ok((vars, origins))
}

/// JSON's types map straight onto ours; an object or null is refused, because a variable is a
/// scalar or a list and nothing about a comparison would know what to do with either.
fn json_to_value(value: &serde_json::Value, origin: &Origin) -> Result<Value> {
    match value {
        serde_json::Value::String(s) => Ok(Value::Str(s.clone())),
        serde_json::Value::Bool(b) => Ok(Value::Bool(*b)),
        serde_json::Value::Number(n) => n.as_f64().map(Value::Num).ok_or_else(|| {
            GrammarError::new(origin.clone(), format!("`{}` is not a finite number", n))
        }),
        serde_json::Value::Array(items) => Ok(Value::List(
            items
                .iter()
                .map(|v| json_to_value(v, origin))
                .collect::<Result<Vec<_>>>()?,
        )),
        serde_json::Value::Object(_) => Err(GrammarError::new(
            origin.clone(),
            "a variable cannot be a JSON object; it is a string, number, boolean, or list"
                .to_string(),
        )),
        serde_json::Value::Null => Err(GrammarError::new(
            origin.clone(),
            "a variable cannot be null; give it a value".to_string(),
        )),
    }
}

fn parse_pairs(stdout: &str, origin: &Origin) -> Result<(Vars, VarOrigins)> {
    let mut vars = Vars::new();
    let mut origins = VarOrigins::new();
    for (i, line) in stdout.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line_origin = Origin::new(file_name_of(origin), i + 1);
        let Some((name, value)) = line.split_once('=') else {
            return Err(GrammarError::new(
                line_origin,
                format!("`{}` is not a `name = value` line", line),
            ));
        };
        let name = name.trim().to_string();
        check_name(&name, origin)?;
        origins.insert(name.clone(), line_origin);
        vars.insert(name, Value::parse_literal(value.trim()));
    }
    Ok((vars, origins))
}

/// A variable name is an identifier, so it can be read back as `$name` — a name that starts with
/// a digit or holds a space could never be referenced and so cannot be a variable.
fn check_name(name: &str, origin: &Origin) -> Result<()> {
    let ok = !name.is_empty()
        && name.starts_with(|c: char| c.is_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(GrammarError::new(
            origin.clone(),
            format!("`{}` is not a valid variable name", name),
        )
        .with_hint("a name starts with a letter or `_` and holds only letters, digits and `_`."))
    }
}

fn file_name_of(origin: &Origin) -> String {
    origin
        .file
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("vars")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A per-test directory that deletes itself.
    ///
    /// It used to be `temp_dir()/linix-vars-test-<pid>-<n>`, created and never removed. PIDs
    /// are recycled, so a later run could land on a previous run's directory and find its
    /// files — and these tests are about *which provider files are present*, so inheriting two
    /// of them made "more than one variable provider is present" a failure that appeared and
    /// vanished depending on what the OS handed out as a process id.
    ///
    /// The caller binds the returned `TempDir`: dropping it deletes the directory, which is
    /// what stops the next run inheriting anything.
    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temp dir")
    }

    /// A `.py` provider finds whatever this machine calls Python.
    ///
    /// It used to name literally `python` on Windows and literally `python3` everywhere else, so
    /// a Windows box with only `python3` and a Linux box with only `python` each had a `vars.py`
    /// they could not run — the same one-spelling assumption that made `#!` hooks fail (Y17).
    #[test]
    fn a_py_provider_finds_this_machines_python_whatever_it_is_called() {
        let argv = interpreter_argv(Path::new("vars.py"));
        let first = argv.first().expect("an interpreter").to_string_lossy().into_owned();
        match crate::model::script::interpreter_named("python3") {
            Some(_) => assert!(
                Path::new(&first).is_file(),
                "python was found but the argv still says `{}`",
                first
            ),
            None => assert_eq!(first, "python3", "an unresolvable name is left as written"),
        }
    }

    /// Resolving the interpreter must not eat the flag beside it — `powershell` without `-File`
    /// reads the provider as an expression rather than running it as a file.
    #[test]
    fn resolving_the_interpreter_keeps_the_flags_that_follow_it() {
        for (file, flag) in [("vars.ps1", "-File"), ("vars.bat", "/c"), ("vars.cmd", "/c")] {
            let argv = interpreter_argv(Path::new(file));
            assert_eq!(argv.len(), 2, "{}: {:?}", file, argv);
            assert_eq!(argv[1].to_string_lossy(), flag, "{}", file);
        }
    }

    /// An extension with no convention is run directly, which is what IX.6 says.
    #[test]
    fn an_unknown_extension_names_no_interpreter() {
        assert!(interpreter_argv(Path::new("vars.wat")).is_empty());
        assert!(interpreter_argv(Path::new("vars")).is_empty());
    }

    fn facts() -> HostFacts {
        HostFacts {
            os: "linux".into(),
            arch: "x86_64".into(),
            host: "laptop".into(),
            family: "debian".into(),
            vars: Default::default(),
        }
    }

    #[test]
    fn no_provider_file_is_no_variables() {
        let tmp = tmp();
        let dir = tmp.path();
        assert!(select(dir, &None).unwrap().is_none());
    }

    #[test]
    fn a_sole_provider_is_selected_without_a_source_key() {
        let tmp = tmp();
        let dir = tmp.path();
        std::fs::write(dir.join("vars"), "role = desktop\n").unwrap();
        let s = select(dir, &None).unwrap().unwrap();
        assert_eq!(s.kind, Kind::LineFile);
    }

    #[test]
    fn two_providers_and_no_choice_is_a_loud_error() {
        let tmp = tmp();
        let dir = tmp.path();
        std::fs::write(dir.join("vars"), "role = desktop\n").unwrap();
        std::fs::write(dir.join("vars.py"), "print()\n").unwrap();
        let err = select(dir, &None).unwrap_err();
        assert!(err.what.contains("more than one"), "{}", err);
        assert!(err.what.contains("vars.py"), "{}", err);
    }

    #[test]
    fn a_named_source_picks_that_one_and_names_its_kind() {
        let tmp = tmp();
        let dir = tmp.path();
        std::fs::write(dir.join("vars"), "role = desktop\n").unwrap();
        std::fs::write(dir.join("vars.py"), "print()\n").unwrap();
        let s = select(dir, &Some("vars.py".into())).unwrap().unwrap();
        assert_eq!(s.kind, Kind::External);
        assert_eq!(s.path, dir.join("vars.py"));
    }

    #[test]
    fn a_named_source_that_is_absent_is_an_error() {
        let tmp = tmp();
        let dir = tmp.path();
        std::fs::write(dir.join("vars"), "role = desktop\n").unwrap();
        let err = select(dir, &Some("vars.py".into())).unwrap_err();
        assert!(err.what.contains("not in the repo"), "{}", err);
    }

    #[test]
    fn a_named_source_with_a_nonsense_name_is_an_error() {
        let tmp = tmp();
        let dir = tmp.path();
        let err = select(dir, &Some("something".into())).unwrap_err();
        assert!(err.what.contains("not a variable provider name"), "{}", err);
    }

    #[test]
    fn vars_linix_is_the_embedded_kind() {
        assert_eq!(kind_of("vars.linix"), Some(Kind::Embedded));
        assert_eq!(kind_of("vars"), Some(Kind::LineFile));
        assert_eq!(kind_of("vars.py"), Some(Kind::External));
        assert_eq!(kind_of("modules"), None);
    }

    #[test]
    fn json_object_output_carries_its_types() {
        let origin = Origin::new("vars.js", 0);
        let (vars, origins) = parse_output(
            r#"{"role": "travel", "cores": 8, "gpu": true, "tags": ["a", "b"]}"#,
            &origin,
        )
        .unwrap();
        assert_eq!(vars["role"], Value::Str("travel".into()));
        assert_eq!(vars["cores"], Value::Num(8.0));
        assert_eq!(vars["gpu"], Value::Bool(true));
        assert_eq!(
            vars["tags"],
            Value::List(vec![Value::Str("a".into()), Value::Str("b".into())])
        );
        // JSON has no lines, so a variable's origin is the provider file itself.
        assert_eq!(origins["role"].to_string(), "vars.js");
    }

    #[test]
    fn name_value_lines_infer_types_like_a_vars_line() {
        let origin = Origin::new("vars.sh", 0);
        let (vars, origins) = parse_output(
            "role = travel\ncores = 8\n# a comment\ngpu = true\n",
            &origin,
        )
        .unwrap();
        assert_eq!(vars["role"], Value::Str("travel".into()));
        assert_eq!(vars["cores"], Value::Num(8.0));
        assert_eq!(vars["gpu"], Value::Bool(true));
        // A pair line carries its line number, counting the comment line.
        assert_eq!(origins["gpu"].to_string(), "vars.sh:4");
    }

    #[test]
    fn a_json_object_value_is_refused() {
        let origin = Origin::new("vars.js", 0);
        let err = parse_output(r#"{"x": {"nested": 1}}"#, &origin).unwrap_err();
        assert!(err.what.contains("object"), "{}", err);
    }

    #[test]
    fn a_bad_variable_name_is_refused() {
        let origin = Origin::new("vars.js", 0);
        let err = parse_output(r#"{"1bad": "x"}"#, &origin).unwrap_err();
        assert!(err.what.contains("valid variable name"), "{}", err);
    }

    #[test]
    fn an_external_provider_runs_and_its_output_is_parsed() {
        // A provider written in whatever runs here without extra installs: a `.cmd` on Windows,
        // a `.sh` everywhere else. Both are guaranteed present (cmd / sh).
        let tmp = tmp();
        let dir = tmp.path();
        let (name, body) = if cfg!(windows) {
            (
                "vars.cmd",
                "@echo off\r\necho role=%LINIX_OS%\r\necho cores=8\r\n",
            )
        } else {
            ("vars.sh", "echo role=$LINIX_OS\necho cores=8\n")
        };
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        let vars = run_external(&path, &facts()).unwrap();
        assert_eq!(
            vars["role"],
            Value::Str("linux".into()),
            "the machine's facts reach the provider"
        );
        assert_eq!(vars["cores"], Value::Num(8.0));
    }

    #[test]
    fn a_provider_that_exits_nonzero_is_an_error_with_its_stderr() {
        let tmp = tmp();
        let dir = tmp.path();
        let (name, body) = if cfg!(windows) {
            ("vars.cmd", "@echo off\r\necho boom 1>&2\r\nexit /b 3\r\n")
        } else {
            ("vars.sh", "echo boom >&2\nexit 3\n")
        };
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        let err = run_external(&path, &facts()).unwrap_err();
        assert!(
            err.what.contains("boom"),
            "carries the provider's stderr: {}",
            err
        );
    }
}
