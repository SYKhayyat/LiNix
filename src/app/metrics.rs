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
    /// How many packages the one manager command that covered this one carried. `1` unless it
    /// was batched.
    #[serde(default = "one")]
    pub batch_size: usize,
}

fn one() -> usize {
    1
}

/// What the run is doing, in the summary's words.
///
/// A rebuild removes a package and puts the same package back. Counting that as a plain
/// removal is true and unreadable: "Removals: 214" on a run where all 214 return is the
/// sentence that makes someone reach for the power button (II.11b, V.49).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Narration {
    Change,
    Rebuild,
}

/// The two counter labels, so `remove` is reserved for removals that stay removed.
fn summary_labels(narration: Narration) -> (&'static str, &'static str) {
    match narration {
        Narration::Change => ("Installs", "Removals"),
        Narration::Rebuild => ("Reinstalled", "Removed to reinstall"),
    }
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
        batch_size: usize,
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
            batch_size,
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

    /// Everything recorded so far.
    ///
    /// The summary is a *rendering* of this. Without a reader, the only way to ask what the
    /// collector holds is to print it and read the paragraph back, which is why the test that
    /// claims to check recording accuracy asserted nothing at all: it called `print_summary`
    /// twice and stopped, and would have passed against a collector that discarded every
    /// operation it was handed.
    pub fn operations(&self) -> Vec<OperationMetrics> {
        self.inner
            .lock()
            .expect("Metrics lock poisoned")
            .operations
            .clone()
    }

    /// The run's totals: `(installed, removed, bytes_downloaded)`.
    pub fn totals(&self) -> (u64, u64, u64) {
        let inner = self.inner.lock().expect("Metrics lock poisoned");
        (
            inner.packages_installed,
            inner.packages_removed,
            inner.total_bytes_downloaded,
        )
    }

    pub fn print_summary(&self, narration: Narration) {
        self.print_summary_opts(true, narration)
    }

    /// Like [`print_summary`], but `quiet` suppresses everything except errors — for a
    /// `--quiet` run that still needs to surface failures.
    pub fn print_summary_quiet(&self) {
        self.print_summary_opts(false, Narration::Change)
    }

    fn print_summary_opts(&self, verbose: bool, narration: Narration) {
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
        self.print_summary_full(narration)
    }

    fn print_summary_full(&self, narration: Narration) {
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
        let (installs, removals) = summary_labels(narration);
        println!(
            "{:<14}{}",
            format!("{}:", installs),
            inner.packages_installed
        );
        println!("{:<14}{}", format!("{}:", removals), inner.packages_removed);

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

                // Says *why* several packages share a duration to the millisecond. Six
                // identical numbers under a heading reading "Parallel Task Breakdown" is how
                // a fully serialised run passed for a parallel one; now they are identical
                // because they were one command, and the line says so.
                let batch_text = if op.batch_size > 1 {
                    format!(" (1 of {} in one `{}` command)", op.batch_size, op.backend)
                } else {
                    String::new()
                };

                println!(
                    "  {} [{:<8}] {:<20} ({}ms){}{}",
                    status_icon, op.backend, op.name, op.duration_ms, retry_text, batch_text
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
            batch_size: 1,
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
    fn a_rebuild_never_calls_its_removals_removals() {
        // K15. All of them come straight back, and a bare "Removals: 214" is the sentence
        // that reads as a machine being dismantled.
        let (_, removals) = summary_labels(Narration::Rebuild);
        assert!(!removals.eq_ignore_ascii_case("removals"));
        assert!(removals.contains("reinstall"));
        assert_eq!(summary_labels(Narration::Change).1, "Removals");
    }

    #[test]
    fn rollup_of_nothing_is_empty() {
        assert!(backend_rollup(&[]).is_empty());
    }
}
