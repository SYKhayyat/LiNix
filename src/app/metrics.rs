use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::info;
use chrono::{DateTime, Utc};

/// Represents the performance data of a single package operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationMetrics {
    pub name: String,
    pub backend: String,
    pub started_at: DateTime<Utc>,
    pub duration_ms: u64,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Default)]
struct MetricsInner {
    /// List of all operations executed during the session.
    operations: Vec<OperationMetrics>,
    packages_installed: u64,
    packages_removed: u64,
    errors: Vec<(String, String)>,
    start_time: Option<Instant>,
}

/// A thread-safe metrics collector for high-performance parallel execution.
/// Follows the Monitor pattern to protect shared state across worker threads.
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
                errors: Vec::new(),
                start_time: Some(Instant::now()),
            })),
        }
    }

    /// Records a completed operation from a DAG node.
    pub fn record_operation(
        &self, 
        name: &str, 
        backend: &str, 
        start_time: DateTime<Utc>, 
        success: bool, 
        error: Option<String>
    ) {
        let mut inner = self.inner.lock().unwrap();
        let duration = Utc::now().signed_duration_since(start_time).num_milliseconds();
        
        inner.operations.push(OperationMetrics {
            name: name.to_string(),
            backend: backend.to_string(),
            started_at: start_time,
            duration_ms: duration as u64,
            success,
            error,
        });
    }

    pub fn record_install(&self, count: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.packages_installed += count;
    }

    pub fn record_remove(&self, count: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.packages_removed += count;
    }

    pub fn record_error(&self, context: &str, message: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.errors.push((context.to_string(), message.to_string()));
    }

    /// Generates a summary report for the user.
    pub fn print_summary(&self) {
        let inner = self.inner.lock().unwrap();
        let total_duration = inner.start_time.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);

        println!("\n=== Transaction Summary ===");
        println!("Status:       {}", if inner.errors.is_empty() { "SUCCESS" } else { "DEGRADED" });
        println!("Time:         {:.2}s", total_duration);
        println!("Installs:     {}", inner.packages_installed);
        println!("Removals:     {}", inner.packages_removed);

        if inner.verbose_needed() {
            println!("\nParallel Task Breakdown:");
            for op in &inner.operations {
                let status_icon = if op.success { "✓" } else { "✗" };
                println!("  {} [{:<8}] {:<20} ({}ms)", status_icon, op.backend, op.name, op.duration_ms);
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

impl MetricsInner {
    fn verbose_needed(&self) -> bool {
        self.operations.len() > 0
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}