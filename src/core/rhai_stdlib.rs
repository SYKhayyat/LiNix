//! The one Rhai Shall ships — the engine and the standard library on it.
//!
//! Two places run Rhai: `vars.shall` (Part IX) and a `#rhai` hook (II.12). They are the same
//! trust — II.6b ruled `vars.shall` is *"trusted the same as a hook — a script in your own
//! repo"* — so they get the same language. They used to get different ones: `vars` had the
//! clock, a shell, files, the environment and the network, and a hook had `print`. That was
//! never a security posture (a hook two lines away in the same config can open `#!` and run
//! anything), it was a gap, and it made the shipped example config's `exec("systemctl enable
//! docker")` a call to a function no engine here had ever registered.
//!
//! **What makes this safe is the ledger, not the engine.** Every script either engine runs is
//! hashed into `locks/` and gated by II.12: first sight asks, a changed hash stops, and `-y`
//! cannot skip it. Withholding a shell from one arm stopped nobody and hid that.

use chrono::{Datelike, Timelike};
use rhai::{Dynamic, Engine, EvalAltResult};

/// A runaway script must not hang every `plan` and `sync`. Ten million operations is far more
/// than any variable computation or hook needs and far less than a wedged infinite loop reaches.
/// It counts Rhai operations, not seconds — a hook whose `sh()` runs for ten minutes is one.
const MAX_OPERATIONS: u64 = 10_000_000;

/// The ceiling on a `http_get`. `vars` resolves once per invocation (IX.6) and a hook fires once
/// per package, so this bounds how long one command can wait on an unresponsive host.
const HTTP_TIMEOUT_SECS: u64 = 30;

/// An engine with the operation cap, `print`, and the standard library. `tag` is what a script's
/// `print` is labelled with in the log — `[vars]` or `[hook]` — and is the only thing that
/// differs between the two callers.
pub fn engine(tag: &'static str) -> Engine {
    let mut engine = Engine::new();
    engine.set_max_operations(MAX_OPERATIONS);
    engine.register_fn("print", move |msg: &str| {
        tracing::info!("[{}] {}", tag, msg)
    });
    register_stdlib(&mut engine);
    engine
}

/// The standard library IX.6 rules a script may have — the clock, a shell, read-only files, the
/// environment, the network — always on, because it is a script you committed to your own repo.
///
/// The rule for which fail loud: a lookup that *asks a question* (`sh_ok`, `path_exists`,
/// `has_env`, `env`) returns a value, so a "no" is an answer; an action that *fetches something*
/// (`sh`, `read_file`, `http_get`, `parse_json`) throws on failure, because a fetch that quietly
/// returned nothing would resolve a variable to the wrong value — or let a hook report success
/// over a command that never ran — with no sign it had failed.
fn register_stdlib(engine: &mut Engine) {
    // --- the clock (local time — "is it the weekend here") ---
    engine.register_fn("now", || chrono::Utc::now().timestamp());
    engine.register_fn("today", || {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    });
    engine.register_fn("weekday", || chrono::Local::now().format("%A").to_string());
    engine.register_fn("hour", || chrono::Local::now().hour() as i64);
    engine.register_fn("year", || chrono::Local::now().year() as i64);
    engine.register_fn("month", || chrono::Local::now().month() as i64);
    engine.register_fn("day", || chrono::Local::now().day() as i64);

    // --- the shell ---
    engine.register_fn(
        "sh",
        |cmd: &str| -> std::result::Result<String, Box<EvalAltResult>> {
            // Bounded, for the reason `MAX_OPERATIONS` names and cannot reach: the operation
            // cap counts Rhai operations, not seconds, so a `sh()` that waits for ten minutes
            // costs one. `vars.shall` resolves before any manager is asked, so that wait is a
            // wait on every command Shall has.
            let out = crate::core::blocking::command_output_bounded(
                &mut shell_command(cmd),
                &format!("`sh(\"{}\")`", cmd),
            )
            .map_err(|e| rt_err(format!("sh: could not run `{}`: {}", cmd, e)))?;
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                return Err(rt_err(format!(
                    "sh: `{}` exited with {}: {}",
                    cmd,
                    out.status,
                    stderr.trim()
                )));
            }
            Ok(crate::utils::text::sanitize(&String::from_utf8_lossy(
                &out.stdout,
            )))
        },
    );
    engine.register_fn("sh_ok", |cmd: &str| {
        // A timeout is not a "yes" — `unwrap_or(false)` is already the answer this shape gives
        // to a command that could not be run, and a command that never finished is one of those.
        crate::core::blocking::command_output_bounded(
            &mut shell_command(cmd),
            &format!("`sh_ok(\"{}\")`", cmd),
        )
        .map(|o| o.status.success())
        .unwrap_or(false)
    });

    // --- read-only filesystem ---
    engine.register_fn(
        "read_file",
        |path: &str| -> std::result::Result<String, Box<EvalAltResult>> {
            let resolved = crate::core::Validator::validate_path_sync(std::path::Path::new(path))
                .map_err(|e| rt_err(format!("read_file: {}: {}", path, e)))?;
            std::fs::read_to_string(&resolved)
                .map_err(|e| rt_err(format!("read_file: {}: {}", path, e)))
        },
    );
    engine.register_fn("path_exists", |path: &str| {
        std::path::Path::new(path).exists()
    });

    // --- the environment (W7's escape hatch: SHALL_ROLE=work when hostname cannot say) ---
    engine.register_fn("env", |name: &str| std::env::var(name).unwrap_or_default());
    engine.register_fn("env", |name: &str, fallback: &str| {
        std::env::var(name).unwrap_or_else(|_| fallback.to_string())
    });
    engine.register_fn("has_env", |name: &str| std::env::var_os(name).is_some());

    // --- the network ---
    engine.register_fn(
        "http_get",
        |url: &str| -> std::result::Result<String, Box<EvalAltResult>> {
            http_get(url).map_err(rt_err)
        },
    );

    // --- JSON, so an http_get body can be navigated for one value ---
    engine.register_fn(
        "parse_json",
        |text: &str| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
            let v: serde_json::Value =
                serde_json::from_str(text).map_err(|e| rt_err(format!("parse_json: {}", e)))?;
            Ok(json_to_dynamic(&v))
        },
    );
}

/// A Rhai runtime error, which each caller surfaces in its own vocabulary — a `GrammarError`
/// naming `vars.shall`, or a failed hook naming the package.
fn rt_err(msg: String) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorRuntime(
        msg.into(),
        rhai::Position::NONE,
    ))
}

/// The platform shell, so `sh("a | b")` behaves as the script author expects. The command is the
/// author's own text in their own committed file — the trust the ledger already gated.
fn shell_command(cmd: &str) -> std::process::Command {
    let mut c = if cfg!(windows) {
        std::process::Command::new("cmd")
    } else {
        std::process::Command::new("sh")
    };
    c.arg(if cfg!(windows) { "/c" } else { "-c" }).arg(cmd);
    // Both callers capture the output, so a command that stops to ask something would block on
    // a prompt the user never sees.
    c.stdin(std::process::Stdio::null());
    c
}

/// An HTTP GET from a synchronous Rhai builtin, over the process-wide connection pool.
///
/// Rhai has no async, so the request has to be driven to completion from this thread. The
/// previous shape spawned an OS thread per call and built a fresh `reqwest::blocking::Client`
/// inside it — a TLS root store parsed and a connection pool discarded for every `http()`
/// variable. Handing the future to the runtime that is already there costs neither.
///
/// Blocking this thread is safe because both callers run behind `spawn_blocking`: this is never
/// a runtime worker, so nothing is starved while the request is in flight.
fn http_get(url: &str) -> std::result::Result<String, String> {
    // **The seed URL is checked, not only the redirects.** The pooled client refuses a hop that
    // leaves HTTPS, so the one URL its policy never sees is the one it was handed. What comes
    // back here becomes a resolved variable, and a resolved variable decides which packages get
    // declared — the same "the value crossing the network decides what runs on the machine"
    // that `check_scheme` exists for. There is no `@allow_http` here to spell the exception
    // with, and inventing a silent default is what that option exists to prevent.
    crate::core::download::check_scheme(url, false, "an http_get from a script")
        .map_err(|e| e.to_string())?;
    let client =
        crate::core::http::api("shall-vars", HTTP_TIMEOUT_SECS).map_err(|e| e.to_string())?;
    let url = url.to_string();
    let fut = async move {
        let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("{} returned {}", url, resp.status()));
        }
        resp.text().await.map_err(|e| e.to_string())
    };

    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            handle.spawn(async move {
                let _ = tx.send(fut.await);
            });
            rx.recv()
                .map_err(|_| "http_get: the request task disappeared".to_string())?
        }
        // `shall eval` on a synchronous path, and the unit tests, have no runtime to borrow.
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?
            .block_on(fut),
    }
}

/// JSON into a Rhai value, so `parse_json(body).field` works. This produces a Rhai map for a JSON
/// object, which a script may navigate — the map is refused only if a `vars` script returns one
/// *as a variable*, not while the script is reaching into it.
fn json_to_dynamic(v: &serde_json::Value) -> Dynamic {
    match v {
        serde_json::Value::Null => Dynamic::UNIT,
        serde_json::Value::Bool(b) => (*b).into(),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map(Dynamic::from)
            .unwrap_or_else(|| n.as_f64().unwrap_or(0.0).into()),
        serde_json::Value::String(s) => s.clone().into(),
        serde_json::Value::Array(a) => a
            .iter()
            .map(json_to_dynamic)
            .collect::<rhai::Array>()
            .into(),
        serde_json::Value::Object(o) => {
            let mut map = rhai::Map::new();
            for (k, val) in o {
                map.insert(k.clone().into(), json_to_dynamic(val));
            }
            map.into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The URL a script hands in is checked, not only the ones it is redirected to.**
    ///
    /// The pooled client refuses a hop that leaves HTTPS, so the seed URL was the one place its
    /// policy never looked. What `http_get` returns becomes a resolved variable, and a resolved
    /// variable decides which packages get declared — the value crossing the network decides
    /// what runs on the machine, which is `check_scheme`'s whole argument.
    #[test]
    fn a_plain_http_seed_url_is_refused_before_the_request() {
        let err = http_get("http://internal/config").expect_err("plain http is not a seed URL");
        assert!(err.contains("plain HTTP"), "{err}");
    }

    /// The refusal is about the scheme and nothing else — an https URL gets past it and fails,
    /// if it fails, on the network rather than on the check.
    #[test]
    fn an_https_url_is_not_refused_by_the_scheme_check() {
        let err = http_get("https://host.invalid./nothing-here").unwrap_err();
        assert!(!err.contains("plain HTTP"), "{err}");
    }
}
