//! A backend that answers the two capability questions both ways and writes down what it was
//! sent.
//!
//! **`TestKernel`'s mock stops one layer too low for the executor's own decisions.**
//! `MockExecutor` records the argv a *backend* built, which is the right subject for "does
//! `brew` spell a version `@1.6`". It cannot be the subject for "does the engine send a version
//! at all", because the four branches that decide that read
//! [`Installable::pins_version`](shall::core::Installable::pins_version) and
//! [`Installable::supports_purge`](shall::core::Installable::supports_purge) — and every backend
//! the kernel registers answers each of those exactly one way. A question with one answer
//! available is a question no test can get wrong.
//!
//! So this is a backend rather than an executor: it is registered under whatever name the test
//! wants, told what it can and cannot do, and it appends one line per call to a log the test
//! reads. Several of them can share the log, which is what makes cross-manager ordering
//! assertable — two mock managers with two private logs cannot say which command ran first.

use async_trait::async_trait;
use shall::app::sync::guard::Reaped;
use shall::core::installed::InstalledListings;
use shall::core::{
    BackendCapabilities, BackendCore, Error, Installable, Package, PackageSpec, Queryable, Result,
    Retryability,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

/// The ordered call log several backends can share.
pub type CallLog = Arc<Mutex<Vec<String>>>;

pub fn shared_log() -> CallLog {
    Arc::new(Mutex::new(Vec::new()))
}

/// What the mock machine holds and what it has been asked to do.
#[derive(Default)]
struct Machine {
    /// Package name to the version `info` reports, which is what `Prior::Present` carries.
    installed: BTreeMap<String, Option<String>>,
    /// Names whose install fails, permanently, until [`RecordingBackend::let_it_succeed`].
    failing: BTreeSet<String>,
    /// Names whose install fails transiently EVERY time - a manager whose index is down for
    /// this name and no other. What a bisecting narrower has to be able to find.
    always_flaky: BTreeSet<String>,
    /// Names whose install fails **once**, transiently, and succeeds on the retry. The manager
    /// that is briefly unreachable, which is the only state in which the retry loop's backoff
    /// runs at all.
    flaky: BTreeSet<String>,
}

pub struct RecordingBackend {
    name: String,
    pins_version: bool,
    supports_purge: bool,
    log: CallLog,
    machine: Mutex<Machine>,
    listings: InstalledListings,
}

/// Everything a test sets before the backend is sealed into an `Arc`.
pub struct RecordingBackendBuilder {
    name: String,
    pins_version: bool,
    supports_purge: bool,
    log: CallLog,
    machine: Machine,
}

impl RecordingBackend {
    /// A backend that can install and be queried, pins nothing and purges nothing.
    pub fn named(name: &str, log: &CallLog) -> RecordingBackendBuilder {
        RecordingBackendBuilder {
            name: name.to_string(),
            pins_version: false,
            supports_purge: false,
            log: log.clone(),
            machine: Machine::default(),
        }
    }

    /// Everything sent to every backend sharing this log, oldest first.
    pub fn calls(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    /// Stop failing this package's install. The other half of
    /// [`RecordingBackendBuilder::failing`], for the run that has to fail once and then succeed.
    pub fn let_it_succeed(&self, name: &str) {
        self.machine.lock().unwrap().failing.remove(name);
    }

    fn record(&self, line: String) {
        self.log.lock().unwrap().push(line);
    }

    /// What one package looks like on a command line, as the *engine* handed it over: the name,
    /// and the version only when the engine let one through.
    fn operand(spec: &PackageSpec) -> String {
        match spec.options.one("version") {
            Some(v) => format!("{}@{}", spec.name, v),
            None => spec.name.clone(),
        }
    }
}

impl RecordingBackendBuilder {
    /// This manager can be asked for an exact version at install time.
    pub fn pinning(mut self) -> Self {
        self.pins_version = true;
        self
    }

    /// This manager's `purge` does something its `remove` does not.
    pub fn purging(mut self) -> Self {
        self.supports_purge = true;
        self
    }

    /// The machine already holds this, at this version.
    pub fn holding(mut self, name: &str, version: Option<&str>) -> Self {
        self.machine
            .installed
            .insert(name.to_string(), version.map(str::to_string));
        self
    }

    /// Installing this name fails, permanently, so the retry loop does not spend the test's
    /// wall-clock on backoffs it is not measuring.
    pub fn failing(mut self, name: &str) -> Self {
        self.machine.failing.insert(name.to_string());
        self
    }

    /// Installing this name fails transiently EVERY time, and every other name on the same
    /// command line fails with it - which is what makes a batch worth narrowing.
    pub fn always_flaky(mut self, name: &str) -> Self {
        self.machine.always_flaky.insert(name.to_string());
        self
    }

    /// Installing this name fails **once**, transiently, and works on the next attempt — the
    /// only state in which the engine's backoff runs.
    pub fn flaky_once(mut self, name: &str) -> Self {
        self.machine.flaky.insert(name.to_string());
        self
    }

    pub fn build(self) -> Arc<RecordingBackend> {
        Arc::new(RecordingBackend {
            name: self.name,
            pins_version: self.pins_version,
            supports_purge: self.supports_purge,
            log: self.log,
            machine: Mutex::new(self.machine),
            listings: InstalledListings::new(),
        })
    }
}

/// The same object behind all three capability slots, so a test holding the `Arc` reads the
/// calls the registry's copy recorded.
pub fn capabilities(backend: &Arc<RecordingBackend>) -> Arc<BackendCapabilities> {
    Arc::new(
        BackendCapabilities::builder(backend.clone() as Arc<dyn BackendCore>)
            .with_installable(backend.clone() as Arc<dyn Installable>)
            .with_queryable(backend.clone() as Arc<dyn Queryable>)
            .build(),
    )
}

#[async_trait]
impl BackendCore for RecordingBackend {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        true
    }
    fn probes(&self) -> Vec<String> {
        Vec::new()
    }
    fn needs_root(&self) -> bool {
        false
    }
}

#[async_trait]
impl Installable for RecordingBackend {
    async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        let operands: Vec<String> = specs.iter().map(RecordingBackend::operand).collect();
        self.record(format!("{} install {}", self.name, operands.join(" ")));

        let mut machine = self.machine.lock().unwrap();
        if let Some(bad) = specs.iter().find(|s| machine.failing.contains(&s.name)) {
            return Err(Error::CommandFailed {
                message: format!("`{}` cannot install {}", self.name, bad.name),
                // Permanent, so one attempt is the whole story and no test waits out a backoff
                // it is not asserting on.
                retry: Retryability::Permanent,
                absent_name: false,
            });
        }
        // A name whose ecosystem is down: transient, and it stays that way. Every package on the
        // command line fails with it, exactly as a real manager fails a whole `apt install`.
        if let Some(bad) = specs
            .iter()
            .find(|s| machine.always_flaky.contains(&s.name))
        {
            return Err(Error::CommandFailed {
                message: format!("`{}` could not reach its index for {}", self.name, bad.name),
                retry: Retryability::Transient,
                absent_name: false,
            });
        }
        // Transient, and only the first time: the engine retries, and the attempt after this one
        // falls through to the success below.
        if let Some(bad) = specs
            .iter()
            .find(|s| machine.flaky.contains(&s.name))
            .map(|s| s.name.clone())
        {
            machine.flaky.remove(&bad);
            return Err(Error::CommandFailed {
                message: format!("`{}` could not reach its index for {}", self.name, bad),
                retry: Retryability::Transient,
                absent_name: false,
            });
        }
        for spec in specs {
            machine.installed.insert(
                spec.name.clone(),
                spec.options.one("version").map(str::to_string),
            );
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool, _reaped: Reaped) -> Result<()> {
        self.record(format!("{} remove {}", self.name, names.join(" ")));
        let mut machine = self.machine.lock().unwrap();
        for name in names {
            machine.installed.remove(name);
        }
        Ok(())
    }

    async fn purge(&self, names: &[String], _sudo: bool, _reaped: Reaped) -> Result<()> {
        self.record(format!("{} purge {}", self.name, names.join(" ")));
        let mut machine = self.machine.lock().unwrap();
        for name in names {
            machine.installed.remove(name);
        }
        Ok(())
    }

    fn supports_purge(&self) -> bool {
        self.supports_purge
    }

    fn pins_version(&self) -> bool {
        self.pins_version
    }
}

#[async_trait]
impl Queryable for RecordingBackend {
    fn installed_cache(&self) -> (&InstalledListings, &str) {
        (&self.listings, &self.name)
    }

    async fn fetch_installed(&self) -> Result<Vec<Package>> {
        let machine = self.machine.lock().unwrap();
        Ok(machine
            .installed
            .iter()
            .map(|(name, version)| Package {
                name: name.clone(),
                backend: self.name.clone(),
                version: version.clone(),
                properties: Default::default(),
            })
            .collect())
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.fetch_installed().await
    }

    /// Read straight off the machine rather than through the memo: the engine asks this before
    /// **and** after it changes something, and a run whose second answer is its first cannot
    /// see a rollback put anything back.
    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let machine = self.machine.lock().unwrap();
        Ok(machine.installed.get(name).map(|version| Package {
            name: name.to_string(),
            backend: self.name.clone(),
            version: version.clone(),
            properties: Default::default(),
        }))
    }
}
