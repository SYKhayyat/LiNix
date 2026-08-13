pub mod archive;
pub mod file;
pub mod progress;
pub mod regex_cache;
pub mod style;
pub mod text;

use std::path::PathBuf;

pub use archive::extract_archive;
pub use file::{
    bin_destination, deploy_executable, ensure_deployable, ensure_dir, read_lines_filtered,
    remove_deployed_path, strip_archive_suffixes,
};
pub use progress::{create_progress_reporter, ProgressHandle, ProgressReporter};

pub fn safe_data_dir() -> PathBuf {
    // `SHALL_DATA_DIR` overrides the OS data dir outright (used as-is, no `shall` suffix). This
    // lets a test harness or CI run against a throwaway, isolated state registry so it never
    // touches — or accumulates in — the user's real global state, and so a system-global
    // `prune` only ever sees the packages that run installed.
    if let Some(dir) = std::env::var_os("SHALL_DATA_DIR") {
        return PathBuf::from(dir);
    }
    dirs::data_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("shall")
}

pub fn safe_config_dir() -> PathBuf {
    // `SHALL_CONFIG_DIR` overrides the OS config dir outright (see `safe_data_dir`).
    if let Some(dir) = std::env::var_os("SHALL_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    dirs::config_dir()
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(".config")
        })
        .join("shall")
}

// `refresh_path` was here, with a doc comment asserting that "backends that install a toolchain
// (mise, cargo) must call this before running hooks that invoke it". **Nothing called it, and
// there is no `cargo.rs`, since-deleted** — so either the bug it describes is live and this
// never guarded it,
// or the requirement is fiction. Deleting it makes that an open question instead of a solved
// one: a helper nobody calls, documented as mandatory, reads as coverage from every angle
// except the only one that counts.
//
// The live PATH mechanism is `executor::forget_path_lookups`, which drops the memo after
// Shall installs a manager — a different problem, and one with callers.
