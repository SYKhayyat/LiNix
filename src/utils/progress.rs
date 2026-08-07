//! One spinner, around the one wait that is long enough to look like a hang.
//!
//! This was two traits with eleven methods between them, of which **two were called**:
//! `spinner()` and `finish()`, at `app/sync/mod.rs`, around
//! `Transaction::execute_with_telemetry`. `start()` (the determinate bar), `println()`,
//! `set_position()`, `inc()`, `set_message()` and `finish_with_message()` had no caller
//! anywhere in `src/` or `tests/` — a progress-bar API for a program that shows one spinner.
//!
//! The two shapes stay. `SilentProgress` is what `--no-progress` and `show_progress = false`
//! select, and it is a separate implementation rather than an `if` at the call site so the
//! setting cannot be honoured in one place and forgotten in another.

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::sync::Arc;
use std::time::Duration;

pub trait ProgressReporter: Send + Sync {
    fn spinner(&self, message: &str) -> Box<dyn ProgressHandle>;
}

pub trait ProgressHandle: Send + Sync {
    /// Clears the spinner. It leaves no line behind deliberately — the work it was covering
    /// prints its own summary, and a finished spinner above that summary is noise.
    fn finish(&self);
}

pub struct ConsoleProgress {
    multi: Arc<MultiProgress>,
    enabled: bool,
}

impl ConsoleProgress {
    pub fn new(enabled: bool) -> Self {
        Self {
            multi: Arc::new(MultiProgress::new()),
            enabled,
        }
    }
}

impl ProgressReporter for ConsoleProgress {
    fn spinner(&self, message: &str) -> Box<dyn ProgressHandle> {
        if !self.enabled {
            return Box::new(SilentHandle);
        }

        let pb = self.multi.add(ProgressBar::new_spinner());
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );
        pb.set_message(message.to_string());
        pb.enable_steady_tick(Duration::from_millis(100));

        Box::new(IndicatifHandle { pb })
    }
}

struct IndicatifHandle {
    pb: ProgressBar,
}

impl ProgressHandle for IndicatifHandle {
    fn finish(&self) {
        self.pb.finish_and_clear();
    }
}

pub struct SilentProgress;

impl ProgressReporter for SilentProgress {
    fn spinner(&self, _: &str) -> Box<dyn ProgressHandle> {
        Box::new(SilentHandle)
    }
}

struct SilentHandle;

impl ProgressHandle for SilentHandle {
    fn finish(&self) {}
}

pub fn create_progress_reporter(enabled: bool) -> Arc<dyn ProgressReporter> {
    if enabled {
        Arc::new(ConsoleProgress::new(true))
    } else {
        Arc::new(SilentProgress)
    }
}
