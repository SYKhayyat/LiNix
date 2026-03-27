use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::sync::Arc;
use std::time::Duration;

/// Trait for progress reporting
pub trait ProgressReporter: Send + Sync {
    fn start(&self, total: u64, message: &str) -> Box<dyn ProgressHandle>;
    fn spinner(&self, message: &str) -> Box<dyn ProgressHandle>;
    fn println(&self, message: &str);
}

/// Handle for an individual progress operation
pub trait ProgressHandle: Send + Sync {
    fn set_position(&self, pos: u64);
    fn inc(&self, delta: u64);
    fn set_message(&self, message: &str);
    fn finish(&self);
    fn finish_with_message(&self, message: &str);
}

/// Console progress reporter using indicatif
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
    fn start(&self, total: u64, message: &str) -> Box<dyn ProgressHandle> {
        if !self.enabled {
            return Box::new(SilentHandle);
        }

        let pb = self.multi.add(ProgressBar::new(total));
        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}",
                )
                .unwrap()
                .progress_chars("#>-"),
        );
        pb.set_message(message.to_string());
        pb.enable_steady_tick(Duration::from_millis(100));

        Box::new(IndicatifHandle { pb })
    }

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

    fn println(&self, message: &str) {
        if self.enabled {
            self.multi.println(message).ok();
        } else {
            println!("{}", message);
        }
    }
}

/// Indicatif-based progress handle
struct IndicatifHandle {
    pb: ProgressBar,
}

impl ProgressHandle for IndicatifHandle {
    fn set_position(&self, pos: u64) {
        self.pb.set_position(pos);
    }

    fn inc(&self, delta: u64) {
        self.pb.inc(delta);
    }

    fn set_message(&self, message: &str) {
        self.pb.set_message(message.to_string());
    }

    fn finish(&self) {
        self.pb.finish_and_clear();
    }

    fn finish_with_message(&self, message: &str) {
        self.pb.finish_with_message(message.to_string());
    }
}

/// Silent progress reporter (no output)
pub struct SilentProgress;

impl ProgressReporter for SilentProgress {
    fn start(&self, _total: u64, _message: &str) -> Box<dyn ProgressHandle> {
        Box::new(SilentHandle)
    }

    fn spinner(&self, _message: &str) -> Box<dyn ProgressHandle> {
        Box::new(SilentHandle)
    }

    fn println(&self, _message: &str) {}
}

/// Silent progress handle
struct SilentHandle;

impl ProgressHandle for SilentHandle {
    fn set_position(&self, _pos: u64) {}
    fn inc(&self, _delta: u64) {}
    fn set_message(&self, _message: &str) {}
    fn finish(&self) {}
    fn finish_with_message(&self, _message: &str) {}
}

/// Create a progress reporter based on configuration
pub fn create_progress_reporter(enabled: bool) -> Box<dyn ProgressReporter> {
    if enabled {
        Box::new(ConsoleProgress::new(true))
    } else {
        Box::new(SilentProgress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_silent_progress() {
        let reporter = SilentProgress;
        let handle = reporter.start(100, "test");
        handle.inc(50);
        handle.finish();
    }

    #[test]
    fn test_console_progress_disabled() {
        let reporter = ConsoleProgress::new(false);
        let handle = reporter.start(100, "test");
        handle.inc(50);
        handle.finish();
    }
}
