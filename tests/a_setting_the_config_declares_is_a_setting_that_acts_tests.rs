//! A knob the configuration offers must be read by something.
//!
//! `SandboxSettings::require_bwrap` is declared, documented — *"On Linux, if true, Shall will
//! fail if 'bwrap' is missing"* — defaulted, serialised, and **read nowhere**. Setting it does
//! nothing at all.
//!
//! Its Windows twin is wired. `windows_require_sandbox` is checked at `sandbox.rs` and
//! produces a refusal in exactly the words its doc promises. The Linux one — the one that
//! governs the default host for this program, and the platform where `bwrap` is the only real
//! mechanism — was never connected.
//!
//! **What it costs is not a dead field.** `require_bwrap` exists to prevent precisely the
//! failure recorded as F10 in `docs/GRADE-2026-08-13.md`: `@sandbox` requested, no `bwrap` on
//! the host, `fallback_allowed` defaulting to `true`, and the command running unconfined with
//! nothing said above `debug!`. A Linux administrator who reads the configuration reference,
//! decides that is unacceptable on their fleet, and writes `require_bwrap = true` has taken the
//! documented step to prevent it — and gets the identical unconfined execution. The knob whose
//! whole purpose is to close the hole is the one that does not exist.
//!
//! **Scope: the family, not the field.** The scan below reads every `pub` field of every struct
//! in `src/config/config.rs` and asks whether the name appears anywhere outside `src/config/`.
//! On this tree that is 76 fields and exactly one answer, so the instrument is not
//! over-reporting — it is naming the single case. Any future setting that is offered and not
//! wired fails here on the day it is added.
//!
//! This repository already gates dead `pub fn`s (`a_pub_fn_nobody_calls_is_dead_tests.rs`). It
//! does not gate dead settings, which is the same question asked of the surface a *user* touches
//! rather than the one a caller touches.
//!
//! **What the scan cannot say.** A name that appears outside `src/config/` is read *somewhere*;
//! it is not proof the reading is correct. This is the cheap half, and the cheap half already
//! found the one that matters.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Every `pub` field name declared in the configuration schema.
fn declared_settings() -> Vec<String> {
    let src = std::fs::read_to_string(repo_root().join("src/config/config.rs"))
        .expect("src/config/config.rs should be readable");
    let mut out = Vec::new();
    // **`#[serde(skip)]` is not a setting.** A field serde never reads cannot be written in
    // `preferences.toml`, is not documented as a knob, and has no user to disappoint — it is a
    // derived cache that happens to live on the same struct as the settings it is derived from
    // (`GuardSettings::matchers` holds the protection lists pre-lowered). Counting one as a
    // setting reports "declared, documented and defaulted, and nothing reads it" about
    // something that is none of those three, which is a checker crying wolf — and a checker
    // that cries wolf gets switched off.
    let mut skipped = false;
    for line in src.lines() {
        let t = line.trim();
        if t.starts_with("#[serde(skip)]") {
            skipped = true;
            continue;
        }
        let Some(rest) = t.strip_prefix("pub ") else {
            // Doc comments and other attributes sit between the `#[serde(skip)]` and the field,
            // so the flag survives them and is cleared only by the field it belongs to.
            continue;
        };
        if skipped {
            skipped = false;
            continue;
        }
        let Some((name, _)) = rest.split_once(':') else {
            continue;
        };
        // Fields only: `pub struct`, `pub fn`, `pub enum` have no colon in that position, and a
        // field name is lowercase with underscores.
        if !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            out.push(name.to_string());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Every `.rs` under `src/` that is not part of the configuration schema itself.
///
/// `src/config/` is excluded because a setting is always named where it is declared, defaulted
/// and deserialised — the question is whether anything downstream ever asks for it.
fn everything_outside_the_schema() -> String {
    let root = repo_root().join("src");
    let config_dir = root.join("config");
    let mut text = String::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        if dir == config_dir {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                if let Ok(s) = std::fs::read_to_string(&p) {
                    text.push_str(&s);
                    text.push('\n');
                }
            }
        }
    }
    text
}

fn names_it(text: &str, field: &str) -> bool {
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(at) = text[from..].find(field) {
        let start = from + at;
        let end = start + field.len();
        let before_ok = start == 0 || !is_word(bytes[start - 1]);
        let after_ok = end >= bytes.len() || !is_word(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The self-test: the scanner reads the schema and finds settings that are genuinely wired.
///
/// A source scan that silently matched nothing, or read no fields, would pass forever. These
/// three are wired through three different subsystems, and one of them is the twin of the
/// setting the gate below reports.
#[test]
fn the_scanner_reads_the_schema_and_recognises_a_wired_setting() {
    let settings = declared_settings();
    assert!(
        settings.len() > 40,
        "only {} setting(s) came out of the schema; the reader has lost the file: {settings:?}",
        settings.len()
    );
    let body = everything_outside_the_schema();
    for wired in [
        "fallback_allowed",
        "max_parallel",
        "windows_require_sandbox",
    ] {
        assert!(
            settings.iter().any(|s| s == wired),
            "`{wired}` is declared in the schema and the reader did not find it"
        );
        assert!(
            names_it(&body, wired),
            "`{wired}` is read outside `src/config/` and the scan cannot see it — the \
             instrument under-reports, so its silence would mean nothing"
        );
    }
}

/// Every setting the configuration offers is read by something.
#[test]
fn every_setting_the_config_declares_is_read_somewhere() {
    let body = everything_outside_the_schema();
    let dead: Vec<String> = declared_settings()
        .into_iter()
        .filter(|f| !names_it(&body, f))
        .collect();

    assert!(
        dead.is_empty(),
        "{} setting(s) are declared, documented and defaulted, and nothing outside \
         `src/config/` ever reads them: {:?}\n\n\
         A setting that does nothing is worse than a missing one: the user who sets it has \
         taken the documented step and believes the matter is closed. `require_bwrap` promises \
         to fail when `bwrap` is absent, and its Windows twin `windows_require_sandbox` keeps \
         that promise at `sandbox.rs:303`.",
        dead.len(),
        dead
    );
}
