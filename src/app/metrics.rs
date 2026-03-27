use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::info;

/// Metrics collector for tracking operations
#[derive(Clone)]
pub struct MetricsCollector {
    inner: Arc<Mutex<MetricsInner>>,
}

#[derive(Default)]
struct MetricsInner {
    operations: HashMap<String, OperationMetrics>,
    packages_installed: u64,
    packages_removed: u64,
    errors: Vec<ErrorRecord>,
    start_time: Option<Instant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationMetrics {
    pub name: String,
    pub start_time: Option<u64>,
    pub end_time: Option<u64>,
    pub duration_ms: Option<u64>,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorRecord {
    pub operation: String,
    pub message: String,
    pub timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MetricsReport {
    pub total_duration_ms: u64,
    pub packages_installed: u64,
    pub packages_removed: u64,
    pub operations: Vec<OperationMetrics>,
    pub errors: Vec<ErrorRecord>,
    pub success: bool,
}

impl MetricsCollector {
    /// Create a new metrics collector
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(MetricsInner {
                operations: HashMap::new(),
                packages_installed: 0,
                packages_removed: 0,
                errors: Vec::new(),
                start_time: Some(Instant::now()),
            })),
        }
    }

    /// Start tracking an operation
    pub fn start_operation(&self, name: &str) {
        let mut inner = self.inner.lock().unwrap();

        let now = inner
            .start_time
            .map(|s| s.elapsed().as_millis() as u64)
            .unwrap_or(0);

        inner.operations.insert(
            name.to_string(),
            OperationMetrics {
                name: name.to_string(),
                start_time: Some(now),
                end_time: None,
                duration_ms: None,
                success: false,
                error: None,
            },
        );

        info!("Started operation: {}", name);
    }

    /// End tracking an operation
    pub fn end_operation(&self, name: &str) {
        let mut inner = self.inner.lock().unwrap();

        let now = inner
            .start_time
            .map(|s| s.elapsed().as_millis() as u64)
            .unwrap_or(0);

        if let Some(op) = inner.operations.get_mut(name) {
            op.end_time = Some(now);
            op.duration_ms = op.start_time.map(|s| now.saturating_sub(s));
            op.success = true;

            info!(
                "Completed operation: {} ({} ms)",
                name,
                op.duration_ms.unwrap_or(0)
            );
        }
    }

    /// Record an operation failure
    pub fn fail_operation(&self, name: &str, error: &str) {
        let mut inner = self.inner.lock().unwrap();

        let now = inner
            .start_time
            .map(|s| s.elapsed().as_millis() as u64)
            .unwrap_or(0);

        if let Some(op) = inner.operations.get_mut(name) {
            op.end_time = Some(now);
            op.duration_ms = op.start_time.map(|s| now.saturating_sub(s));
            op.success = false;
            op.error = Some(error.to_string());
        }

        inner.errors.push(ErrorRecord {
            operation: name.to_string(),
            message: error.to_string(),
            timestamp: now,
        });
    }

    /// Record packages installed
    pub fn record_install(&self, count: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.packages_installed += count;
    }

    /// Record packages removed
    pub fn record_remove(&self, count: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.packages_removed += count;
    }

    /// Record an error
    pub fn record_error(&self, operation: &str, message: &str) {
        let mut inner = self.inner.lock().unwrap();

        let now = inner
            .start_time
            .map(|s| s.elapsed().as_millis() as u64)
            .unwrap_or(0);

        inner.errors.push(ErrorRecord {
            operation: operation.to_string(),
            message: message.to_string(),
            timestamp: now,
        });
    }

    /// Get the metrics report
    pub fn report(&self) -> MetricsReport {
        let inner = self.inner.lock().unwrap();

        let total_duration = inner
            .start_time
            .map(|s| s.elapsed().as_millis() as u64)
            .unwrap_or(0);

        let operations: Vec<OperationMetrics> = inner.operations.values().cloned().collect();
        let success = inner.errors.is_empty() && operations.iter().all(|o| o.success);

        MetricsReport {
            total_duration_ms: total_duration,
            packages_installed: inner.packages_installed,
            packages_removed: inner.packages_removed,
            operations,
            errors: inner.errors.clone(),
            success,
        }
    }

    /// Convert metrics to JSON
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self.report()).unwrap_or(serde_json::Value::Null)
    }

    /// Print a summary to stdout
    pub fn print_summary(&self) {
        let report = self.report();

        println!("\n=== Operation Summary ===");
        println!("Total duration: {} ms", report.total_duration_ms);
        println!("Packages installed: {}", report.packages_installed);
        println!("Packages removed: {}", report.packages_removed);

        if !report.operations.is_empty() {
            println!("\nOperations:");
            for op in &report.operations {
                let status = if op.success { "✓" } else { "✗" };
                let duration = op
                    .duration_ms
                    .map(|d| format!("{} ms", d))
                    .unwrap_or_else(|| "N/A".to_string());
                println!("  {} {} ({})", status, op.name, duration);
            }
        }

        if !report.errors.is_empty() {
            println!("\nErrors:");
            for err in &report.errors {
                println!("  - [{}] {}", err.operation, err.message);
            }
        }

        println!(
            "\nResult: {}",
            if report.success { "SUCCESS" } else { "FAILED" }
        );
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_basic() {
        let metrics = MetricsCollector::new();

        metrics.start_operation("test");
        metrics.record_install(5);
        metrics.end_operation("test");

        let report = metrics.report();
        assert_eq!(report.packages_installed, 5);
        assert!(report.success);
    }

    #[test]
    fn test_metrics_failure() {
        let metrics = MetricsCollector::new();

        metrics.start_operation("test");
        metrics.fail_operation("test", "Something went wrong");

        let report = metrics.report();
        assert!(!report.success);
        assert_eq!(report.errors.len(), 1);
    }
}
