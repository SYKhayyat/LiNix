pub mod archive;
pub mod command;
pub mod file;
pub mod progress;
pub mod retry;
pub mod style;

use std::path::PathBuf;

// Centralize critical utility re-exports
pub use archive::{extract_archive, is_archive};
pub use file::{atomic_write, ensure_dir, read_lines_filtered};
pub use progress::{create_progress_reporter, ProgressHandle, ProgressReporter};
pub use retry::{retry, retry_default, RetryConfig};

/// Reliably locates the LiNix data directory across platforms.
/// Resolves unresolved import errors in teleport.rs and profile.rs.
pub fn safe_data_dir() -> PathBuf {
    // `LINIX_DATA_DIR` overrides the OS data dir outright (used as-is, no `linix` suffix). This
    // lets a test harness or CI run against a throwaway, isolated state registry so it never
    // touches — or accumulates in — the user's real global state, and so a system-global
    // `prune` only ever sees the packages that run installed.
    if let Some(dir) = std::env::var_os("LINIX_DATA_DIR") {
        return PathBuf::from(dir);
    }
    dirs::data_dir()
        .unwrap_or_else(|| {
            // Fallback to current directory if system data dir is unavailable
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        })
        .join("linix")
}

/// Reliably locates the LiNix configuration directory.
pub fn safe_config_dir() -> PathBuf {
    // `LINIX_CONFIG_DIR` overrides the OS config dir outright (see `safe_data_dir`).
    if let Some(dir) = std::env::var_os("LINIX_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    dirs::config_dir()
        .unwrap_or_else(|| {
            // Fallback to current directory/.config if system config dir is unavailable
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(".config")
        })
        .join("linix")
}

/// Injects a new directory into the current process's PATH environment variable.
/// Vital for backends that install toolchains (like Mise or Cargo) and need
/// immediate access to them in hooks.
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
