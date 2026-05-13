pub mod archive;
pub mod command;
pub mod file;
pub mod progress;
pub mod retry;

// Centralize critical utility re-exports
pub use archive::{extract_archive, is_archive};
pub use file::{atomic_write, ensure_dir, read_lines_filtered};
pub use progress::{create_progress_reporter, ProgressReporter, ProgressHandle};
pub use retry::{retry, retry_default, RetryConfig};

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