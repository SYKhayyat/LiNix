use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Represents the performance data of a single package operation.
/// Hardened for Phase 2.5: Includes retry counts and bandwidth telemetry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationMetrics {
    pub name: String,
    pub backend: String,
    pub started_at_unix: i64,
    pub duration_ms: u64,
    pub success: bool,
    pub error: Option<String>,
    /// Number of times the operation was retried before succeeding or giving up.
    pub retry_count: u32,
    /// Number of bytes downloaded during this specific operation.
    pub bytes_downloaded: u64,
}

#[derive(Default)]
struct MetricsInner {
    /// List of all operations executed during the session.
    operations: Vec<OperationMetrics>,
    packages_installed: u64,
    packages_removed: u64,
    total_bytes_downloaded: u64,
    errors: Vec<(String, String)>,
    start_time: Option<Instant>,
}

/// A thread-safe metrics collector for high-performance parallel execution.
/// Fulfills Phase 2.5: Comprehensive system telemetry.
#[derive(Clone)]
pub struct MetricsCollector {
    inner: Arc<Mutex<MetricsInner>>,
}

impl MetricsCollector {
    /// Initializes a new collector with a monotonic start clock.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(MetricsInner {
                operations: Vec::new(),
                packages_installed: 0,
                packages_removed: 0,
                total_bytes_downloaded: 0,
                errors: Vec::new(),
                start_time: Some(Instant::now()),
            })),
        }
    }

    /// Records a completed operation from a DAG node.
    #[allow(clippy::too_many_arguments)]
    pub fn record_operation(
        &self,
        name: &str,
        backend: &str,
        start_time: DateTime<Utc>,
        success: bool,
        error: Option<String>,
        retry_count: u32,
        bytes_downloaded: u64,
    ) {
        let mut inner = self.inner.lock().expect("Metrics lock poisoned");
        let duration = Utc::now()
            .signed_duration_since(start_time)
            .num_milliseconds();

        inner.total_bytes_downloaded += bytes_downloaded;

        inner.operations.push(OperationMetrics {
            name: name.to_string(),
            backend: backend.to_string(),
            started_at_unix: start_time.timestamp(),
            duration_ms: duration.max(0) as u64,
            success,
            error,
            retry_count,
            bytes_downloaded,
        });
    }

    pub fn record_install(&self, count: u64) {
        let mut inner = self.inner.lock().expect("Metrics lock poisoned");
        inner.packages_installed += count;
    }

    pub fn record_remove(&self, count: u64) {
        let mut inner = self.inner.lock().expect("Metrics lock poisoned");
        inner.packages_removed += count;
    }

    pub fn record_error(&self, context: &str, message: &str) {
        let mut inner = self.inner.lock().expect("Metrics lock poisoned");
        inner
            .errors
            .push((context.to_string(), message.to_string()));
    }

    /// Generates a summary report for the user.
    pub fn print_summary(&self) {
        let inner = self.inner.lock().expect("Metrics lock poisoned");
        let total_duration = inner
            .start_time
            .map(|s| s.elapsed().as_secs_f64())
            .unwrap_or(0.0);

        println!("\n=== Transaction Summary ===");
        println!(
            "Status:       {}",
            if inner.errors.is_empty() {
                "SUCCESS"
            } else {
                "DEGRADED"
            }
        );
        println!("Time:         {:.2}s", total_duration);
        println!("Installs:     {}", inner.packages_installed);
        println!("Removals:     {}", inner.packages_removed);

        if inner.total_bytes_downloaded > 0 {
            let mb = inner.total_bytes_downloaded as f64 / 1024.0 / 1024.0;
            println!("Downloaded:   {:.2} MB", mb);
        }

        if !inner.operations.is_empty() {
            println!("\nParallel Task Breakdown:");
            for op in &inner.operations {
                let status_icon = if op.success { "✓" } else { "✗" };
                let retry_text = if op.retry_count > 0 {
                    format!(" (Retries: {})", op.retry_count)
                } else {
                    "".to_string()
                };

                println!(
                    "  {} [{:<8}] {:<20} ({}ms){}",
                    status_icon, op.backend, op.name, op.duration_ms, retry_text
                );
            }
        }

        if !inner.errors.is_empty() {
            println!("\nErrors Encountered:");
            for (ctx, err) in &inner.errors {
                println!("  - [{}]: {}", ctx, err);
            }
        }
        println!("===========================\n");
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}
