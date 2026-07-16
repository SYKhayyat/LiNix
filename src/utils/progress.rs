use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::sync::Arc;
use std::time::Duration;

pub trait ProgressReporter: Send + Sync {
    fn start(&self, total: u64, message: &str) -> Box<dyn ProgressHandle>;
    fn spinner(&self, message: &str) -> Box<dyn ProgressHandle>;
    /// Prints without interfering with active progress bars; a bare `println!` races the
    /// bar's own redraws and corrupts the display.
    fn println(&self, message: &str);
}

pub trait ProgressHandle: Send + Sync {
    fn set_position(&self, pos: u64);
    fn inc(&self, delta: u64);
    fn set_message(&self, message: &str);
    fn finish(&self);
    fn finish_with_message(&self, message: &str);
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
            let _ = self.multi.println(message);
        } else {
            println!("{}", message);
        }
    }
}

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

pub struct SilentProgress;

impl ProgressReporter for SilentProgress {
    fn start(&self, _: u64, _: &str) -> Box<dyn ProgressHandle> {
        Box::new(SilentHandle)
    }
    fn spinner(&self, _: &str) -> Box<dyn ProgressHandle> {
        Box::new(SilentHandle)
    }
    fn println(&self, _: &str) {}
}

struct SilentHandle;

impl ProgressHandle for SilentHandle {
    fn set_position(&self, _: u64) {}
    fn inc(&self, _: u64) {}
    fn set_message(&self, _: &str) {}
    fn finish(&self) {}
    fn finish_with_message(&self, _: &str) {}
}

pub fn create_progress_reporter(enabled: bool) -> Arc<dyn ProgressReporter> {
    if enabled {
        Arc::new(ConsoleProgress::new(true))
    } else {
        Arc::new(SilentProgress)
    }
}
