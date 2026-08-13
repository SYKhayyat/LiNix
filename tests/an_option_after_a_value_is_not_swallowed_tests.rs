//! An option written after a valued option must not be absorbed into that value.
//!
//! `config/grammar/options.rs::spliced_option` refuses `@version=1.0.0 @hold` — an `@`
//! preceded by whitespace — and its own message names the failure exactly: *"`@hold` is
//! absorbed into the option before it rather than being an option at all."* The same
//! sentence describes `@version=1.0.0@hold`, which is not refused: the detector requires
//! `bytes[i - 1].is_ascii_whitespace()`, and there is no space before that `@`.
//!
//! So the spelling a user reaches for when chaining options — no space, the way the first
//! `@` attaches to the name — is the one spelling that parses, and it parses to something
//! else. `cargo:ripgrep@version=1.0.0@hold` is one package at version `1.0.0@hold` with no
//! hold on it, and nothing says so.
//!
//! Three of the losses are more than cosmetic. `hold` is what stops an upgrade moving a
//! pinned package; `sandbox` is what confines `shall run`; `system` decides whether a
//! package is written into the environment the OS owns. Each of them is silently absent,
//! which is the failure mode Part I's first principle exists to rule out.
//!
//! **Not the same bug as a comma.** `@version=1.0.0,hold` is the documented spelling and it
//! works. This is about the undocumented one that neither works nor fails.

use std::path::{Path, PathBuf};
use std::process::Command;

fn shall() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_shall"))
}

/// A config directory with one module holding exactly `line`.
fn config_with(name: &str, line: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("shall-swallow-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let status = Command::new(shall())
        .arg("init")
        .env("SHALL_CONFIG_DIR", &dir)
        .env("SHALL_DATA_DIR", dir.join("data"))
        .output()
        .expect("init");
    assert!(status.status.success(), "init failed");
    std::fs::write(dir.join("modules").join("starter.txt"), format!("{line}\n")).unwrap();
    dir
}

fn eval(dir: &Path) -> (String, bool) {
    let out = Command::new(shall())
        .arg("eval")
        .env("SHALL_CONFIG_DIR", dir)
        .env("SHALL_DATA_DIR", dir.join("data"))
        .output()
        .expect("eval");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (text, out.status.success())
}

/// The control, and it is the half that makes the rest a finding rather than a preference.
///
/// The space-separated spelling is refused, with a message that describes absorption. If
/// this ever stops being refused the grammar has been loosened rather than fixed, and the
/// test below would go green for the wrong reason.
#[test]
fn a_space_before_the_second_option_is_still_refused() {
    let dir = config_with("space", "cargo:ripgrep@version=1.0.0 @hold");
    let (text, ok) = eval(&dir);
    assert!(
        !ok,
        "`@version=1.0.0 @hold` was accepted; the control this file depends on is gone:\n{text}"
    );
    assert!(
        text.contains("runs two options together"),
        "refused, but not as an absorbed option — the control no longer covers the case:\n{text}"
    );
}

/// The same mistake without the space parses, and the option disappears.
#[test]
fn an_option_abutting_a_value_is_refused_rather_than_absorbed() {
    let dir = config_with("abut", "cargo:ripgrep@version=1.0.0@hold");
    let (text, ok) = eval(&dir);
    assert!(
        !ok,
        "`cargo:ripgrep@version=1.0.0@hold` was accepted. The line declares no hold and a \
         version of `1.0.0@hold`, and the user is told nothing:\n{text}"
    );
}

/// Every bare flag, so a fix that teaches the lexer one keyword is not mistaken for a fix.
///
/// These are the options that carry no `=`, which is what makes them absorbable: the value
/// grammar for `version` accepts an `@` (it has to — `npm:@angular/cli@version=17.3.0` is
/// Q23's ruling), so the flag lands inside the version and is indistinguishable from it.
#[test]
fn no_bare_flag_is_absorbed_into_a_version() {
    const FLAGS: &[&str] = &[
        "hold",
        "optional",
        "system",
        "download_only",
        "allow_http",
        "unverified",
        "classic",
        "allow_shrink",
        "sandbox",
        "shim",
    ];

    let mut swallowed = Vec::new();
    for flag in FLAGS {
        let dir = config_with(
            &format!("flag-{flag}"),
            &format!("cargo:ripgrep@version=1.0.0@{flag}"),
        );
        let (text, ok) = eval(&dir);
        if ok {
            let version = text
                .lines()
                .find(|l| l.contains("\"version\""))
                .unwrap_or("<no version field>")
                .trim()
                .to_string();
            swallowed.push(format!("@{flag} -> {version}"));
        }
    }

    assert!(
        swallowed.is_empty(),
        "{} of {} bare flags were absorbed into the version instead of being read as \
         options:\n  {}\n\nEach line declares the flag and does not get it. `hold` is the \
         one that stops an upgrade, `sandbox` is the one that confines `shall run`.",
        swallowed.len(),
        FLAGS.len(),
        swallowed.join("\n  ")
    );
}

/// The consequence, asked of the feature rather than of the parse.
///
/// `shall hold` is where a user checks whether the hold took. It says nothing is held, and
/// the manifest says otherwise — which is the whole defect in the two commands a person
/// would actually run.
#[test]
fn a_hold_written_after_a_version_reaches_the_hold_list() {
    let dir = config_with("holdlist", "cargo:ripgrep@version=1.0.0@hold");
    let (_, parsed) = eval(&dir);
    if !parsed {
        return; // Refused at the grammar, which is the other legitimate outcome.
    }
    let out = Command::new(shall())
        .arg("hold")
        .env("SHALL_CONFIG_DIR", &dir)
        .env("SHALL_DATA_DIR", dir.join("data"))
        .output()
        .expect("hold");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !text.contains("No packages are held"),
        "the manifest declares `@hold` and `shall hold` reports nothing held:\n{text}"
    );
}
