pub mod archive;
pub mod command;
pub mod file;
pub mod progress;
pub mod retry;
pub mod style;

use std::path::PathBuf;

pub use archive::{extract_archive, is_archive};
pub use file::{
    atomic_write, bin_destination, deploy_executable, ensure_dir, read_lines_filtered,
    remove_deployed_path, strip_archive_suffixes,
};
pub use progress::{create_progress_reporter, ProgressHandle, ProgressReporter};
pub use retry::{retry, retry_default, RetryConfig};

pub fn safe_data_dir() -> PathBuf {
    // `LINIX_DATA_DIR` overrides the OS data dir outright (used as-is, no `linix` suffix). This
    // lets a test harness or CI run against a throwaway, isolated state registry so it never
    // touches — or accumulates in — the user's real global state, and so a system-global
    // `prune` only ever sees the packages that run installed.
    if let Some(dir) = std::env::var_os("LINIX_DATA_DIR") {
        return PathBuf::from(dir);
    }
    dirs::data_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("linix")
}

pub fn safe_config_dir() -> PathBuf {
    // `LINIX_CONFIG_DIR` overrides the OS config dir outright (see `safe_data_dir`).
    if let Some(dir) = std::env::var_os("LINIX_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    dirs::config_dir()
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(".config")
        })
        .join("linix")
}

/// Backends that install a toolchain (mise, cargo) must call this before running hooks
/// that invoke it: the freshly installed binary is not on the PATH this process inherited,
/// and hooks are spawned from it.
pub fn refresh_path(new_path: std::path::PathBuf) {
    if let Some(path) = std::env::var_os("PATH") {
        let mut paths = std::env::split_paths(&path).collect::<Vec<_>>();
        if !paths.contains(&new_path) {
            paths.insert(0, new_path);
            if let Ok(merged) = std::env::join_paths(paths) {
                std::env::set_var("PATH", merged);
            }
        }
    }
}
