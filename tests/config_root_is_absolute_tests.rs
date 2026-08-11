//! Every door that answers "where is the repo" refuses a relative path (AU2).
//!
//! `shall --config-dir ./sandbox init` read `preferences.toml` from the sandbox and `modules/`
//! from the real repo — because `main.rs` honours the raw flag while `Config::config_root()`
//! discards any path that is not absolute and falls back to `safe_config_dir()`, which re-reads
//! `$SHALL_CONFIG_DIR`. So the documented precedence inverted (the flag lost to the environment
//! variable it "outranks") and nothing said a word.
//!
//! `shall path --set ./cfg` had refused a relative path since it was written, with a message
//! that explains exactly why one is wrong. **Three of the four doors did not.** This asserts all
//! four, so the next one that opens is one this file already covers.

use shall::config::settings::{resolve_root, Settings};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The relative shapes a person actually types. `sandbox` is the dangerous one: it looks like a
/// name rather than a path, which is how it passes review.
const RELATIVE: &[&str] = &["./sandbox", "sandbox", "../sibling", "cfg/nested"];

fn absolute(tail: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(format!("C:/{}", tail))
    } else {
        PathBuf::from(format!("/{}", tail))
    }
}

#[test]
fn the_flag_refuses_a_relative_path() {
    for relative in RELATIVE {
        let err = resolve_root(Some(Path::new(relative)), &Settings::default())
            .expect_err(&format!("`--config-dir {}` was accepted", relative));
        let msg = err.to_string();
        assert!(
            msg.contains("absolute") && msg.contains(relative),
            "the refusal must name the path and say what is wrong with it, got: {}",
            msg
        );
        assert!(
            msg.contains("--config-dir"),
            "the refusal must name the door it came through, got: {}",
            msg
        );
    }
}

#[test]
fn the_flag_accepts_an_absolute_path() {
    let root = absolute("srv/shall");
    let resolved = resolve_root(Some(&root), &Settings::default()).expect("absolute is fine");
    assert_eq!(resolved.path, root);
}

#[test]
fn the_settings_file_refuses_a_relative_path() {
    let dir = tempfile::TempDir::new().unwrap();
    let file = dir.path().join("shall.settings.toml");
    std::fs::write(&file, "config_root = \"./sandbox\"\n").unwrap();
    let err = Settings::load_from(&file).expect_err("a relative stored root was accepted");
    assert!(err.to_string().contains("absolute"), "{}", err);
}

/// The environment variable and the stored path, in one test because the variable is
/// process-global and two tests racing over it would be a flake.
#[test]
fn the_environment_variable_refuses_a_relative_path_and_the_stored_one_still_wins_when_it_is_absent(
) {
    let stored = Settings {
        config_root: Some(absolute("stored/shall")),
    };

    std::env::set_var("SHALL_CONFIG_DIR", "./sandbox");
    let err = resolve_root(None, &stored).expect_err("a relative $SHALL_CONFIG_DIR was accepted");
    let msg = err.to_string();
    assert!(msg.contains("absolute"), "{}", msg);
    assert!(
        msg.contains("SHALL_CONFIG_DIR"),
        "the refusal must name the door it came through, got: {}",
        msg
    );

    // Control: the same call with the variable gone must succeed, or the assertion above
    // would pass for a resolver that refuses everything.
    std::env::remove_var("SHALL_CONFIG_DIR");
    let resolved = resolve_root(None, &stored).expect("an absolute stored root is fine");
    assert_eq!(resolved.path, absolute("stored/shall"));
}

/// The end of AU2's reproduction, through the real binary: the flag must not be silently
/// discarded in favour of a directory the user did not name.
#[test]
fn the_binary_refuses_rather_than_scaffolding_somewhere_else() {
    let dir = tempfile::TempDir::new().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_shall"))
        .args(["--config-dir", "./sandbox", "init"])
        .current_dir(dir.path())
        .env("SHALL_DATA_DIR", dir.path().join("data"))
        .env_remove("SHALL_CONFIG_DIR")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("the binary should run");

    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_ne!(
        out.status.code(),
        Some(0),
        "`--config-dir ./sandbox init` succeeded. It scaffolded SOMEWHERE, and not where it \
         was told: {}",
        text
    );
    assert!(
        text.contains("absolute"),
        "the refusal must explain itself, got: {}",
        text
    );
    assert!(
        !dir.path().join("sandbox").exists(),
        "it refused and scaffolded anyway"
    );
}

/// `--data-dir` is the flag AU4 adds, and it answers the same question about a different
/// directory. A second door with the same defect is the shape this repo keeps finding.
#[test]
fn the_data_dir_flag_refuses_a_relative_path() {
    let dir = tempfile::TempDir::new().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_shall"))
        .args(["--data-dir", "./state", "path"])
        .current_dir(dir.path())
        .env_remove("SHALL_CONFIG_DIR")
        .env_remove("SHALL_DATA_DIR")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("the binary should run");

    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_ne!(out.status.code(), Some(0), "accepted: {}", text);
    assert!(
        text.contains("absolute") && text.contains("--data-dir"),
        "got: {}",
        text
    );
}

/// The four doors, enumerated by the compiler rather than by me.
///
/// Every test above names one source. A fifth source added later would be covered by none of
/// them and by no assertion that could notice — which is the shape of AU2 itself, where three
/// doors of four had been reviewed as though they were one. This `match` is exhaustive over
/// `RootSource`, so adding a variant **stops this file compiling** until someone says how the
/// new door refuses a relative path.
#[test]
fn every_source_of_the_root_is_accounted_for() {
    use shall::config::settings::RootSource;

    for source in [
        RootSource::Flag,
        RootSource::Environment,
        RootSource::SettingsFile,
        RootSource::Default,
    ] {
        let covered_by = match source {
            RootSource::Flag => "the_flag_refuses_a_relative_path",
            RootSource::Environment => {
                "the_environment_variable_refuses_a_relative_path_and_the_stored_one_still_wins_when_it_is_absent"
            }
            RootSource::SettingsFile => "the_settings_file_refuses_a_relative_path",
            // Nothing to refuse: `safe_config_dir()` builds it from the platform dir, which is
            // absolute by construction. Asserted rather than assumed, because "absolute by
            // construction" is the kind of sentence this whole file exists because of.
            RootSource::Default => {
                assert!(
                    resolve_root(None, &Settings::default())
                        .map(|r| r.path.is_absolute())
                        .unwrap_or(false)
                        || std::env::var_os("SHALL_CONFIG_DIR").is_some(),
                    "the built-in default is not an absolute path"
                );
                "this test"
            }
        };
        assert!(!covered_by.is_empty());
    }
}
