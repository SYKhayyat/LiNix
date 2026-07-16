use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationMetrics {
    pub name: String,
    pub backend: String,
    pub started_at_unix: i64,
    pub duration_ms: u64,
    pub success: bool,
    pub error: Option<String>,
    pub retry_count: u32,
    pub bytes_downloaded: u64,
}

#[derive(Default)]
struct MetricsInner {
    operations: Vec<OperationMetrics>,
    packages_installed: u64,
    packages_removed: u64,
    total_bytes_downloaded: u64,
    errors: Vec<(String, String)>,
    start_time: Option<Instant>,
}

#[derive(Clone)]
pub struct MetricsCollector {
    inner: Arc<Mutex<MetricsInner>>,
}

impl MetricsCollector {
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

    pub fn print_summary(&self) {
        self.print_summary_opts(true)
    }

    /// Like [`print_summary`], but `quiet` suppresses everything except errors — for a
    /// `--quiet` run that still needs to surface failures.
    pub fn print_summary_quiet(&self) {
        self.print_summary_opts(false)
    }

    fn print_summary_opts(&self, verbose: bool) {
        if !verbose {
            // Quiet mode: say nothing on success, but never swallow errors.
            let inner = self.inner.lock().expect("Metrics lock poisoned");
            if !inner.errors.is_empty() {
                eprintln!("Transaction DEGRADED — {} error(s):", inner.errors.len());
                for (ctx, err) in &inner.errors {
                    eprintln!("  - [{}]: {}", ctx, err);
                }
            }
            return;
        }
        self.print_summary_full()
    }

    fn print_summary_full(&self) {
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

            // Per-backend rollup. This is SUMMED work-time, which can exceed the elapsed
            // wall-clock above because backends run in parallel — so we label it "work", not
            // "time", to keep the concurrency honest.
            let rollup = backend_rollup(&inner.operations);
            if rollup.len() > 1 {
                println!("\nPer-backend (work-time; parallel, so the sum exceeds elapsed):");
                for (backend, count, summed_ms, slowest_ms) in rollup {
                    println!(
                        "  {:<8} {:>2} op(s)  {:>6.1}s work  (slowest {:.1}s)",
                        backend,
                        count,
                        summed_ms as f64 / 1000.0,
                        slowest_ms as f64 / 1000.0,
                    );
                }
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

/// Aggregate operations per backend: `(backend, op_count, summed_ms, slowest_ms)`, sorted by
/// backend. `summed_ms` is total work and can exceed wall-clock because backends run in
/// parallel — callers must label it as work-time, not elapsed. Pure, unit-tested.
pub fn backend_rollup(ops: &[OperationMetrics]) -> Vec<(String, usize, u64, u64)> {
    use std::collections::BTreeMap;
    let mut map: BTreeMap<String, (usize, u64, u64)> = BTreeMap::new();
    for op in ops {
        let e = map.entry(op.backend.clone()).or_insert((0, 0, 0));
        e.0 += 1;
        e.1 += op.duration_ms;
        e.2 = e.2.max(op.duration_ms);
    }
    map.into_iter().map(|(b, (c, s, m))| (b, c, s, m)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(backend: &str, ms: u64) -> OperationMetrics {
        OperationMetrics {
            name: "x".into(),
            backend: backend.into(),
            started_at_unix: 0,
            duration_ms: ms,
            success: true,
            error: None,
            retry_count: 0,
            bytes_downloaded: 0,
        }
    }

    #[test]
    fn rollup_sums_and_tracks_slowest_per_backend() {
        let ops = vec![op("apt", 100), op("apt", 300), op("cargo", 50)];
        let r = backend_rollup(&ops);
        assert_eq!(r.len(), 2);
        // BTreeMap → sorted: apt then cargo.
        assert_eq!(r[0], ("apt".to_string(), 2, 400, 300));
        assert_eq!(r[1], ("cargo".to_string(), 1, 50, 50));
    }

    #[test]
    fn rollup_of_nothing_is_empty() {
        assert!(backend_rollup(&[]).is_empty());
    }
}
