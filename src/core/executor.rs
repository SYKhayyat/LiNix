use crate::core::{Error, ExitPolicy, Result, Retryability};
use async_trait::async_trait;
use dashmap::DashMap;
use fs2::FileExt;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Command as StdCommand;
use std::process::Output as StdOutput;
use std::process::Stdio;
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{debug, info};

/// Set on every process LiNix spawns, carrying the pid of the LiNix that spawned it. A
/// `linix` that finds it in its environment was started by a package manager LiNix is
/// already driving.
pub const INSIDE_LINIX: &str = "LINIX_INSIDE";

#[derive(Debug, Clone, Default)]
pub struct DryRunOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// An `ExitStatus` for a process that never ran.
///
/// `ExitStatus` has no public constructor, so this used to spawn `/bin/false` or `cmd /C exit 1`
/// and `.expect()` the result — two panics on a path whose whole point is that nothing runs, on
/// a host stripped enough not to have them. Both platforms expose the raw form; on Unix the raw
/// value is a wait status, where the exit code sits in the high byte.
fn fabricate_status(code: i32) -> std::process::ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code << 8)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code as u32)
    }
}

impl DryRunOutput {
    pub fn new() -> Self {
        Self::default()
    }

    /// A run that exited non-zero and complained — what a manager with no package index does.
    pub fn faulted(stderr: &str) -> StdOutput {
        StdOutput {
            status: fabricate_status(1),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }
}

/// A run that exited non-zero and said nothing at all — what a manager that fell over before it
/// could speak leaves behind. `winget` under concurrent cold start is the measured case.
#[cfg(test)]
pub(crate) fn silent_failure(code: i32) -> StdOutput {
    StdOutput {
        status: fabricate_status(code),
        stdout: Vec::new(),
        stderr: Vec::new(),
    }
}

/// A run that exited non-zero and still produced its answer.
#[cfg(test)]
fn spoken_failure(code: i32, stdout: &str, stderr: &str) -> StdOutput {
    StdOutput {
        status: fabricate_status(code),
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.as_bytes().to_vec(),
    }
}

impl From<DryRunOutput> for StdOutput {
    fn from(dry: DryRunOutput) -> Self {
        StdOutput {
            status: fabricate_status(0),
            stdout: dry.stdout,
            stderr: dry.stderr,
        }
    }
}

#[async_trait]
pub trait ExecutionLayer: Send + Sync {
    async fn execute(
        &self,
        cmd: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<StdOutput>;
    fn check_command(&self, cmd: &str) -> bool;
    async fn symlink(&self, src: &Path, dst: &Path) -> Result<()>;

    /// Whether a child spawned by this layer may read LiNix's own stdin. Only the raw layer
    /// behind mutations may; a layer that spawns nothing answers false.
    fn shares_stdin(&self) -> bool {
        false
    }
}

/// Whether a spawned child may read from LiNix's own stdin.
///
/// It is the only stream a child ever shares. stdout and stderr are captured on every path,
/// because every read parses `output.stdout` — a child writing straight to the terminal hands
/// the parser an empty string while the user sees raw manager output and believes it worked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildStdin {
    /// Reads and existence probes. There is nothing to type at them, and a read that consumed
    /// the terminal could not be answered anyway.
    Closed,
    /// Mutations. `sudo` asks for a password on the terminal it was started from, and a
    /// mutation that cannot ask for one cannot run.
    Interactive,
}

/// How long a child may produce **nothing at all** before LiNix stops waiting for it.
///
/// Silence, not duration. A `cargo install` compiling from source and an `apt dist-upgrade`
/// both run for tens of minutes and are working the whole time; no wall-clock cap can tell
/// them from a wedged one, because there is no number that is both above the first and below
/// the second. What separates them is that working commands *say* something — and the one
/// measured hang said nothing for 76 minutes while still holding its pipes open.
///
/// Seeded from `Config::command_idle_timeout_secs` at startup, where `0` means no bound.
static COMMAND_IDLE_TIMEOUT_SECS: once_cell::sync::OnceCell<u64> = once_cell::sync::OnceCell::new();

/// How long a **read** may produce nothing before LiNix stops waiting for it.
///
/// The bound above was chosen for `Checkpoint-Computer`, a mutation that legitimately runs
/// silent for minutes, and reads inherited it because there was only one number. They are not
/// the same job: `winget list` takes 1.5s, `apt list --installed` under a second, and a
/// question that has gone quiet for a quarter of an hour is not about to answer. Fifteen
/// minutes of rope for a one-second question means a stuck read costs fifteen minutes to
/// learn what twenty seconds could have told you.
///
/// Seeded from `Config::query_idle_timeout_secs`, where `0` means no bound — and a bound
/// above `command_idle_timeout_secs` is pointless rather than wrong, since the outer one
/// fires first.
static QUERY_IDLE_TIMEOUT_SECS: once_cell::sync::OnceCell<u64> = once_cell::sync::OnceCell::new();

/// How many times a read that failed *transiently* is asked again before LiNix gives up.
///
/// Reads are idempotent — that is the whole justification. A mutation retried on a guess can
/// install something twice; asking a manager what it has, twice, costs a second. Seeded from
/// `Config::read_retry_attempts`; `1` means ask once and do not retry.
static READ_RETRY_ATTEMPTS: once_cell::sync::OnceCell<u32> = once_cell::sync::OnceCell::new();

/// Above the longest legitimate silence anyone has measured, and far below the hang.
/// `Checkpoint-Computer` is the adversarial case: a real one is silent for its whole run.
pub const DEFAULT_COMMAND_IDLE_TIMEOUT_SECS: u64 = 900;

/// Set the process-wide command idle bound (called once during startup). Later calls no-op.
pub fn set_command_idle_timeout(secs: u64) {
    let _ = COMMAND_IDLE_TIMEOUT_SECS.set(secs);
}

fn command_idle_timeout() -> Option<std::time::Duration> {
    match *COMMAND_IDLE_TIMEOUT_SECS
        .get()
        .unwrap_or(&DEFAULT_COMMAND_IDLE_TIMEOUT_SECS)
    {
        0 => None,
        secs => Some(std::time::Duration::from_secs(secs)),
    }
}

/// Two minutes. The slowest read measured on any host here is a cold `winget list` at 2.6s
/// under sixteen-way contention, so this is ~46x the worst observed — wide enough that a fat
/// machine on a slow disk is never cut off, and narrow enough that a wedged question is a
/// two-minute wait instead of a fifteen-minute one.
pub const DEFAULT_QUERY_IDLE_TIMEOUT_SECS: u64 = 120;

/// Three: the measured failure is a cold-start collision that a warm winget does not
/// reproduce, so one more attempt is usually the whole fix and two more is the margin.
pub const DEFAULT_READ_RETRY_ATTEMPTS: u32 = 3;

/// Set the process-wide read bounds (called once during startup). Later calls no-op.
pub fn set_query_bounds(idle_secs: u64, retry_attempts: u32) {
    let _ = QUERY_IDLE_TIMEOUT_SECS.set(idle_secs);
    let _ = READ_RETRY_ATTEMPTS.set(retry_attempts);
}

/// The bound a read waits under: its own, but never longer than the outer one, which fires
/// first anyway. A `0` on either means that one imposes nothing.
fn query_idle_timeout() -> Option<std::time::Duration> {
    let own = match *QUERY_IDLE_TIMEOUT_SECS
        .get()
        .unwrap_or(&DEFAULT_QUERY_IDLE_TIMEOUT_SECS)
    {
        0 => None,
        secs => Some(std::time::Duration::from_secs(secs)),
    };
    match (own, command_idle_timeout()) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, outer) => outer,
    }
}

fn read_retry_attempts() -> u32 {
    (*READ_RETRY_ATTEMPTS
        .get()
        .unwrap_or(&DEFAULT_READ_RETRY_ATTEMPTS))
    .max(1)
}

/// Where a program lives, answered once per name per process.
///
/// `is_available()` on nearly every backend is a PATH lookup, `registry.available()` calls it
/// for all ~45 of them, and `available()` itself is called at 20+ sites — six times in
/// `AppContext` alone. On Windows a *miss* walks every PATH entry × every `PATHEXT` extension,
/// and a miss is the common case, because most registered backends are not installed on any
/// given host. The same lookup runs again on every spawn to decide how to launch the program.
///
/// One backend had already cached its own probe and the other forty-four had not; a memo here
/// closes all of them at once and dedupes across backends that probe the same program (`krew`
/// probes `kubectl`; `yay`/`paru`/`pacman` overlap).
static PATH_LOOKUP: once_cell::sync::Lazy<DashMap<String, Option<PathBuf>>> =
    once_cell::sync::Lazy::new(DashMap::new);

/// Resolve a program on PATH, from the memo.
///
/// Uses the `which` *crate* — an in-process PATH/PATHEXT search — rather than spawning the
/// external `which`/`where` program: minimal fedora/arch/alpine images do not ship `which`,
/// which made every backend read as OFFLINE there.
pub fn resolve_program(cmd: &str) -> Option<PathBuf> {
    if let Some(hit) = PATH_LOOKUP.get(cmd) {
        return hit.clone();
    }
    let found = which::which(cmd).ok();
    PATH_LOOKUP.insert(cmd.to_string(), found.clone());
    found
}

pub fn program_exists(cmd: &str) -> bool {
    resolve_program(cmd).is_some()
}

/// Drop the memo, for the one case where PATH really does change mid-run: LiNix has just
/// installed the program it is about to ask about.
///
/// Without this, `linix setup` would install a manager and then keep answering from the
/// lookup it took before the installer ran.
pub fn forget_path_lookups() {
    PATH_LOOKUP.clear();
    #[cfg(windows)]
    LAUNCH_PATH.clear();
}

/// Windows: the path a program is actually launched through, memoised.
///
/// Resolving it means a PATH scan *and* a `.ps1` stat beside the resolved shim, and it ran on
/// every single spawn — synchronously, inside an `async fn`, on the same task the planner's
/// `buffer_unordered` relies on to interleave.
#[cfg(windows)]
static LAUNCH_PATH: once_cell::sync::Lazy<DashMap<String, Option<PathBuf>>> =
    once_cell::sync::Lazy::new(DashMap::new);

#[cfg(windows)]
fn launch_path(cmd: &str) -> Option<PathBuf> {
    if let Some(hit) = LAUNCH_PATH.get(cmd) {
        return hit.clone();
    }
    let plan = resolve_program(cmd).map(|resolved| preferred_shim(&resolved));
    LAUNCH_PATH.insert(cmd.to_string(), plan.clone());
    plan
}

/// Lock directories already created, so `create_dir_all` runs once per directory per process
/// instead of once per exclusive command.
static LOCK_DIRS: once_cell::sync::Lazy<dashmap::DashSet<PathBuf>> =
    once_cell::sync::Lazy::new(dashmap::DashSet::new);

pub struct RawExecutor {
    stdin: ChildStdin,
    idle: Option<std::time::Duration>,
}

impl RawExecutor {
    /// The layer behind `run_output`/`search_output`/`command_exists`.
    ///
    /// Bounded by `query_idle_timeout`, not the mutation bound. A question that has said
    /// nothing for two minutes is not about to answer, and making it wait out a bound sized
    /// for `Checkpoint-Computer` buys nothing but the wait.
    pub fn reader() -> Self {
        Self {
            stdin: ChildStdin::Closed,
            idle: query_idle_timeout(),
        }
    }

    /// The layer behind `run`/`run_exclusive`.
    ///
    /// U40 lets a mutation — and only a mutation — share stdin, because `sudo` asks for a
    /// password on the terminal it was started from. `run_on` never inserts `sudo` on Windows
    /// (`if sudo && !cfg!(windows)`), so no Windows mutation has that question to ask, while
    /// the shared terminal stayed and cost the full `command_idle_timeout_secs` any time a
    /// manager asked something else. Measured on one install: 48ms with stdin closed, 21.9s
    /// with a real console — at the shipped bound, a fifteen-minute silence ending in failure
    /// anyway. Closing it means the manager's own prompt is captured and reported instead.
    pub fn mutator() -> Self {
        Self {
            stdin: if cfg!(windows) {
                ChildStdin::Closed
            } else {
                ChildStdin::Interactive
            },
            idle: command_idle_timeout(),
        }
    }

    #[cfg(test)]
    fn with_idle(stdin: ChildStdin, idle: Option<std::time::Duration>) -> Self {
        Self { stdin, idle }
    }

    /// Collect the child's output, optionally echoing it to the terminal as it arrives, and
    /// give up on a child that has gone silent.
    ///
    /// Both streams must be drained concurrently with the wait: a pipe buffer that fills while
    /// nothing reads it blocks the child forever, and a package manager writing more than the
    /// buffer holds is not an edge case.
    ///
    /// The wait is sliced rather than awaited whole so the child stays reachable between
    /// slices — killing it needs the same `&mut` the wait future holds.
    async fn wait_watched(
        mut child: tokio::process::Child,
        cmd: &str,
        mirror: bool,
        idle: Option<std::time::Duration>,
    ) -> Result<StdOutput> {
        use std::sync::Mutex as SyncMutex;
        use std::time::Instant;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        async fn pump<R: tokio::io::AsyncRead + Unpin>(
            mut src: R,
            mirror: bool,
            last: Arc<SyncMutex<Instant>>,
        ) -> std::io::Result<Vec<u8>> {
            let mut collected = Vec::new();
            let mut buf = [0u8; 8192];
            let mut sink = tokio::io::stderr();
            loop {
                let n = src.read(&mut buf).await?;
                *last.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
                if n == 0 {
                    return Ok(collected);
                }
                collected.extend_from_slice(&buf[..n]);
                if mirror {
                    sink.write_all(&buf[..n]).await?;
                    sink.flush().await?;
                }
            }
        }

        let last: Arc<SyncMutex<Instant>> = Arc::new(SyncMutex::new(Instant::now()));
        let out_pipe = child.stdout.take();
        let err_pipe = child.stderr.take();
        let mut out_task = tokio::spawn({
            let last = last.clone();
            async move {
                match out_pipe {
                    Some(p) => pump(p, mirror, last).await,
                    None => Ok(Vec::new()),
                }
            }
        });
        let mut err_task = tokio::spawn({
            let last = last.clone();
            async move {
                match err_pipe {
                    Some(p) => pump(p, mirror, last).await,
                    None => Ok(Vec::new()),
                }
            }
        });

        let status = match idle {
            None => child.wait().await?,
            Some(idle) => loop {
                let quiet = last.lock().unwrap_or_else(|e| e.into_inner()).elapsed();
                let remaining = idle.saturating_sub(quiet);
                if remaining.is_zero() {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    out_task.abort();
                    err_task.abort();
                    // Permanent: the retry loop would spend another `idle` per attempt on a
                    // command that has already proved it does not finish, and a user watching
                    // three silences instead of one learns nothing new from the second two.
                    return Err(Error::command_failed_permanently(format!(
                        "`{}` produced no output for {}s and had not exited; LiNix stopped \
                         waiting and killed it. If this command is legitimately silent for \
                         longer, raise `command_idle_timeout_secs` (0 disables the bound).",
                        cmd,
                        idle.as_secs(),
                    )));
                }
                if let Ok(status) = tokio::time::timeout(remaining, child.wait()).await {
                    break status?;
                }
            },
        };
        // The child has exited; its *output* may not have. A manager that hands stdout to a
        // background process and returns leaves a pipe held by something outside this process
        // tree — `child.wait()` is done, there is nothing left to kill, and an unclocked
        // `await` here waits on an EOF that never comes. Measured: a 20s bound, a 64s wall,
        // and the install reported as a success. So the same clock keeps running over the
        // readers, on silence rather than duration, so a command still printing is never cut.
        let mut stalled = false;
        let out = Self::drain_watched(&mut out_task, &last, idle, &mut stalled).await;
        let err = Self::drain_watched(&mut err_task, &last, idle, &mut stalled).await;
        if stalled {
            out_task.abort();
            err_task.abort();
            return Err(Error::command_failed_permanently(format!(
                "`{}` exited, but something still holds its output open and has printed \
                 nothing for {}s; LiNix stopped waiting. This is what a command that hands \
                 its work to a background process looks like from here. If it is legitimately \
                 silent for longer, raise `command_idle_timeout_secs` (0 disables the bound).",
                cmd,
                idle.map_or(0, |d| d.as_secs()),
            )));
        }
        let joined = |r: std::result::Result<std::io::Result<Vec<u8>>, tokio::task::JoinError>| {
            r.map_err(|e| Error::Other(format!("output reader failed: {}", e)))?
                .map_err(Error::from)
        };
        Ok(StdOutput {
            status,
            stdout: joined(out.expect("not stalled, so the reader finished"))?,
            stderr: joined(err.expect("not stalled, so the reader finished"))?,
        })
    }

    /// Await one output reader under the same silence bound the wait used.
    ///
    /// `stalled` latches: once either reader has gone quiet past the bound the other is not
    /// waited on at all, because the failure is already decided and waiting again would spend
    /// a second `idle` proving it.
    async fn drain_watched(
        task: &mut tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
        last: &Arc<std::sync::Mutex<std::time::Instant>>,
        idle: Option<std::time::Duration>,
        stalled: &mut bool,
    ) -> Option<std::result::Result<std::io::Result<Vec<u8>>, tokio::task::JoinError>> {
        if *stalled {
            return None;
        }
        let Some(idle) = idle else {
            return Some(task.await);
        };
        loop {
            let quiet = last.lock().unwrap_or_else(|e| e.into_inner()).elapsed();
            let remaining = idle.saturating_sub(quiet);
            if remaining.is_zero() {
                *stalled = true;
                return None;
            }
            if let Ok(done) = tokio::time::timeout(remaining, &mut *task).await {
                return Some(done);
            }
        }
    }
}

/// Windows only: some tools on PATH are not `.exe` files but shim scripts —
/// e.g. scoop ships as `scoop.ps1`. `where`/`which` find them (so availability checks
/// pass), but `CreateProcess` can't launch a `.ps1`/`.cmd`/`.bat` directly, so a plain
/// spawn fails with "program not found". Given the resolved path, return the interpreter
/// and argv to run it through. Args are forwarded as *separate* process arguments (via
/// PowerShell `-File` / `cmd /C`), never interpolated into a string, so there is no
/// command-injection surface.
#[cfg(windows)]
fn windows_shim_wrap(cmd: &str, resolved: &Path, args: &[String]) -> Option<(String, Vec<String>)> {
    let ext = resolved.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "ps1" => {
            // PowerShell tools like scoop emit *objects*, which only render when PowerShell
            // formats them. `-File`, `& 'path'`, and a trailing `; exit` all cause the
            // buffered table to be dropped when stdout is captured. The form that reliably
            // yields text AND propagates the exit code: invoke by bare name (so the tool's
            // own output formatting kicks in), pipe through Out-String into a variable,
            // emit it, then exit with the tool's last exit code. Each argument is wrapped
            // in a single-quoted literal (with `'` doubled), so a crafted package name
            // cannot break out of the string — no command-injection surface.
            let esc = |s: &str| format!("'{}'", s.replace('\'', "''"));
            let mut invocation = cmd.to_string();
            for a in args {
                invocation.push(' ');
                invocation.push_str(&esc(a));
            }
            let command = format!(
                "$o = ({} | Out-String -Width 4096); Write-Output $o; exit $LASTEXITCODE",
                invocation
            );
            Some((
                "powershell".to_string(),
                vec![
                    "-NoProfile".to_string(),
                    "-ExecutionPolicy".to_string(),
                    "Bypass".to_string(),
                    "-Command".to_string(),
                    command,
                ],
            ))
        }
        "cmd" | "bat" => {
            // Batch scripts are plain-text; `cmd /C` runs them and forwards args cleanly.
            let mut a = vec!["/C".to_string(), resolved.to_string_lossy().to_string()];
            a.extend(args.iter().cloned());
            Some(("cmd".to_string(), a))
        }
        _ => None,
    }
}

/// Resolve the actual (program, args) to spawn on Windows, wrapping shim scripts. Bare
/// `.exe`/native commands pass through unchanged.
#[cfg(windows)]
fn windows_effective_command(cmd: &str, args: &[String]) -> (String, Vec<String>) {
    if let Some(resolved) = launch_path(cmd) {
        if let Some(wrapped) = windows_shim_wrap(cmd, &resolved, args) {
            return wrapped;
        }
    }
    (cmd.to_string(), args.to_vec())
}

/// The shim to actually launch, when a manager ships more than one.
///
/// `which::which` honours `PATHEXT`, and the Windows default `PATHEXT` does not list `.PS1`.
/// scoop ships `scoop.cmd` and `scoop.ps1` side by side, so on a default box `which` returns
/// the `.cmd`, the `cmd /C` arm below runs it — and **`cmd /C` does not propagate the child's
/// exit code**. Measured on this host, the same failing install:
///
/// ```text
/// cmd /C ...\scoop.cmd install <bad>   -> exit 0
/// the `.ps1` branch                    -> exit 1
/// ```
///
/// So the careful PowerShell arm that already knew how to return `$LASTEXITCODE` was dead code
/// on every default installation, and every scoop verdict fell back to string-matching stdout —
/// one upstream wording change away from reporting a failed install as a success.
///
/// The choice is made on what is on disk, never on `PATHEXT`: a machine's `PATHEXT` is a user
/// setting, and a correctness property that depends on one is a property that holds on the
/// developer's box and not the user's.
#[cfg(windows)]
fn preferred_shim(resolved: &Path) -> std::path::PathBuf {
    let ext = resolved
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if ext == "cmd" || ext == "bat" {
        let ps1 = resolved.with_extension("ps1");
        if ps1.is_file() {
            return ps1;
        }
    }
    resolved.to_path_buf()
}

/// How LiNix actually launches `cmd` on this platform.
///
/// On Windows a manager is usually a `.cmd`/`.ps1` shim that `Command::new` cannot execute at
/// all, so the real launch goes through an interpreter. Anything that runs a manager the way
/// LiNix runs it must come through here — including the argv-drift gate, which asks each
/// manager about its own subcommands and was skipping every shimmed one on this platform as
/// "its help could not be read". A gate that launches programs differently from the product is
/// testing a different program from the one that ships.
pub fn effective_command(cmd: &str, args: &[String]) -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        windows_effective_command(cmd, args)
    }
    #[cfg(not(windows))]
    {
        (cmd.to_string(), args.to_vec())
    }
}

/// Name a spawned command well enough to find it again, in one line.
///
/// `powershell` alone does not identify a hang on a host running six of them; the argv is what
/// says which. Truncated from the right because the discriminating part is the front — the
/// cmdlet, the subcommand, the package name.
fn describe(cmd: &str, args: &[String]) -> String {
    const CAP: usize = 160;
    let mut s = String::from(cmd);
    for a in args {
        s.push(' ');
        s.push_str(a);
        if s.len() >= CAP {
            break;
        }
    }
    if s.chars().count() > CAP {
        s = s.chars().take(CAP).collect::<String>() + "…";
    }
    s
}

#[async_trait]
impl ExecutionLayer for RawExecutor {
    async fn execute(
        &self,
        cmd: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<StdOutput> {
        // On Windows, route shim scripts (scoop's `.ps1`, `.cmd`/`.bat` wrappers) through
        // their interpreter so they can actually launch.
        #[cfg(windows)]
        let (eff_cmd, eff_args) = windows_effective_command(cmd, args);
        #[cfg(windows)]
        let (cmd, args) = (eff_cmd.as_str(), eff_args.as_slice());

        let mut command = Command::new(cmd);
        command.args(args).envs(env);

        // A worker whose task is aborted — a failed node, the global timeout — drops this
        // future, and dropping a future does not kill the process it spawned. Without this an
        // `apt install` keeps running against the same dpkg lock the rollback is about to take,
        // and whatever it completes is in no history that could compensate it.
        command.kill_on_drop(true);

        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let interactive = self.stdin == ChildStdin::Interactive;
        command.stdin(if interactive && std::io::stdin().is_terminal() {
            Stdio::inherit()
        } else {
            // Not `inherit` when LiNix's own stdin is a pipe: a child that reads it would eat
            // input meant for LiNix, and one that blocks on it would never return.
            Stdio::null()
        });

        let child = command.spawn().map_err(|e| {
            let message = format!("Failed to spawn {}: {}", cmd, e);
            // A program that is not on PATH does not arrive during a backoff.
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::command_failed_permanently(message)
            } else {
                Error::command_failed(message)
            }
        })?;

        // A mutation can run for minutes. Its progress used to reach the terminal because the
        // handles were inherited — which is exactly what emptied `output.stdout` and broke
        // every parser. Capture it and mirror it instead, so the bytes go both places.
        // The mirror is stderr, never stdout: stdout carries LiNix's own answer, and a child's
        // chatter interleaved with it is not parseable by whoever piped us.
        let mirror = interactive && std::io::stderr().is_terminal();
        Self::wait_watched(child, &describe(cmd, args), mirror, self.idle).await
    }

    fn shares_stdin(&self) -> bool {
        self.stdin == ChildStdin::Interactive
    }

    fn check_command(&self, cmd: &str) -> bool {
        program_exists(cmd)
    }

    async fn symlink(&self, src: &Path, dst: &Path) -> Result<()> {
        #[cfg(unix)]
        {
            tokio::fs::symlink(src, dst)
                .await
                .map_err(|e| Error::Io(e.to_string()))
        }
        #[cfg(windows)]
        {
            if src.is_dir() {
                tokio::fs::symlink_dir(src, dst)
                    .await
                    .map_err(|e| Error::Io(e.to_string()))
            } else {
                tokio::fs::symlink_file(src, dst)
                    .await
                    .map_err(|e| Error::Io(e.to_string()))
            }
        }
    }
}

pub struct DryRunExecutor {
    vfs: Arc<DashMap<PathBuf, String>>,
}

impl DryRunExecutor {
    pub fn new(vfs: Arc<DashMap<PathBuf, String>>) -> Self {
        Self { vfs }
    }
}

#[async_trait]
impl ExecutionLayer for DryRunExecutor {
    async fn execute(
        &self,
        cmd: &str,
        args: &[String],
        _env: &HashMap<String, String>,
    ) -> Result<StdOutput> {
        info!("[DRY-RUN] Would execute: {} {}", cmd, args.join(" "));
        Ok(DryRunOutput::new().into())
    }

    /// Whether a command exists is a fact about this machine, not something a preview gets
    /// to invent. Answering `true` for everything made every backend look installed.
    fn check_command(&self, cmd: &str) -> bool {
        RawExecutor::reader().check_command(cmd)
    }

    async fn symlink(&self, src: &Path, dst: &Path) -> Result<()> {
        let val = format!("LINK:{}", src.display());
        self.vfs.insert(dst.to_path_buf(), val);
        Ok(())
    }
}

/// Strip inherited access and grant only the running user, via the tool Windows ships with.
///
/// Windows has no `mode` to create a file with, so "created restricted" is achieved by
/// restricting the temporary file and then renaming it into place — the destination never
/// exists in a readable state. A failure here is an error rather than a warning: the caller
/// is about to place a decrypted secret, and a secret that is not protected must not be
/// written at all (T5).
#[cfg(windows)]
fn restrict_to_owner(path: &Path) -> Result<()> {
    let user = std::env::var("USERNAME").map_err(|_| {
        Error::Refused(
            "cannot restrict the file: %USERNAME% is unset, so there is no account to grant \
             access to. Refusing to write a secret nothing protects."
                .into(),
        )
    })?;
    let output = StdCommand::new("icacls")
        .arg(path)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(format!("{}:F", user))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| {
            Error::Other(format!(
                "could not run icacls to restrict {:?}: {}",
                path, e
            ))
        })?;
    if !output.status.success() {
        return Err(Error::Other(format!(
            "icacls could not restrict {:?}: {}",
            path,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

pub struct MockExecutor {
    pub responses: DashMap<String, Result<StdOutput>>,
    pub command_existence: DashMap<String, bool>,
    pub call_log: Arc<Mutex<Vec<String>>>,
    /// The environment the last call carried. The env map is where the pager suppression and
    /// the recursion guard live, and neither is visible in the argv the call log records.
    pub last_env: Arc<Mutex<HashMap<String, String>>>,
    vfs: Arc<DashMap<PathBuf, String>>,
}

impl MockExecutor {
    pub fn new(vfs: Arc<DashMap<PathBuf, String>>) -> Self {
        Self {
            responses: DashMap::new(),
            command_existence: DashMap::new(),
            call_log: Arc::new(Mutex::new(Vec::new())),
            last_env: Arc::new(Mutex::new(HashMap::new())),
            vfs,
        }
    }

    pub fn set_response(&self, cmd_pattern: &str, response: Result<StdOutput>) {
        self.responses.insert(cmd_pattern.to_string(), response);
    }

    pub fn set_command_exists(&self, cmd: &str, exists: bool) {
        self.command_existence.insert(cmd.to_string(), exists);
    }

    pub async fn get_calls(&self) -> Vec<String> {
        self.call_log.lock().await.clone()
    }
}

#[async_trait]
impl ExecutionLayer for MockExecutor {
    async fn execute(
        &self,
        cmd: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<StdOutput> {
        let full_cmd = format!("{} {}", cmd, args.join(" "));
        {
            let mut log = self.call_log.lock().await;
            log.push(full_cmd.clone());
        }
        {
            let mut seen = self.last_env.lock().await;
            *seen = env.clone();
        }
        if let Some(res) = self.responses.get(&full_cmd) {
            return res.clone();
        }
        Ok(DryRunOutput::new().into())
    }

    fn check_command(&self, cmd: &str) -> bool {
        self.command_existence
            .get(cmd)
            .map(|r| *r.value())
            .unwrap_or(true)
    }

    async fn symlink(&self, src: &Path, dst: &Path) -> Result<()> {
        let val = format!("LINK:{}", src.display());
        self.vfs.insert(dst.to_path_buf(), val);
        Ok(())
    }
}

#[derive(Clone)]
pub struct CommandExecutor {
    pub dry_run: bool,
    pub verbose: bool,
    pub inner: Arc<dyn ExecutionLayer>,
    /// Where questions go. A search or an existence probe changes nothing, so it runs for
    /// real even under `--dry-run`: stubbing it does not make the preview safer, it makes
    /// the preview wrong. `apt-cache search jq` answered from a stub is an empty answer,
    /// which reads as "apt does not have jq" and hands the name to whichever manager
    /// answers over the network instead.
    reader: Arc<dyn ExecutionLayer>,
    vfs: Arc<DashMap<PathBuf, String>>,
    lock_map: Arc<DashMap<String, Arc<Mutex<()>>>>,
    /// What this backend's manager means by its exit codes and its complaints. Empty until a
    /// backend claims one with [`with_exit_policy`](Self::with_exit_policy), which is what
    /// keeps manager names out of this file.
    exit_policy: Arc<ExitPolicy>,
    /// This run's installed listings, one per manager.
    ///
    /// Lives here rather than in a global because every backend of one `App` is built on a
    /// duplicate of that `App`'s executor and every duplicate shares this `Arc` — so the memo
    /// is scoped to the run, which is also what keeps one test's mock listing out of the next
    /// test's in a suite where a hundred `App`s live in one process.
    installed: Arc<crate::core::installed::InstalledListings>,
}

impl CommandExecutor {
    pub fn new(dry_run: bool, verbose: bool) -> Self {
        let vfs = Arc::new(DashMap::new());
        let lock_map = Arc::new(DashMap::new());
        let inner: Arc<dyn ExecutionLayer> = if dry_run {
            Arc::new(DryRunExecutor::new(vfs.clone()))
        } else {
            Arc::new(RawExecutor::mutator())
        };
        Self {
            dry_run,
            verbose,
            inner,
            reader: Arc::new(RawExecutor::reader()),
            vfs,
            lock_map,
            exit_policy: Arc::new(ExitPolicy::default()),

            installed: Arc::new(crate::core::installed::InstalledListings::new()),
        }
    }

    pub fn with_layer(
        dry_run: bool,
        verbose: bool,
        layer: Arc<dyn ExecutionLayer>,
        vfs: Arc<DashMap<PathBuf, String>>,
        lock_map: Arc<DashMap<String, Arc<Mutex<()>>>>,
    ) -> Self {
        // A test injects one layer and expects to see every call on it, reads included.
        Self {
            dry_run,
            verbose,
            reader: layer.clone(),
            inner: layer,
            vfs,
            lock_map,
            exit_policy: Arc::new(ExitPolicy::default()),

            installed: Arc::new(crate::core::installed::InstalledListings::new()),
        }
    }

    pub fn duplicate(&self) -> Self {
        self.clone()
    }

    /// Let this run reuse installed listings written by earlier runs, for `secs` (0 = never).
    ///
    /// Set from config after the executor exists, rather than taken as a constructor argument:
    /// `CommandExecutor::new` is called from twenty-odd tests that have no config and want the
    /// off default, and a third constructor parameter would have to be spelled out in all of
    /// them to say "the same as before".
    pub fn set_installed_cache(&mut self, secs: u64) {
        if secs > 0 {
            self.installed = Arc::new(crate::core::installed::InstalledListings::with_ttl(secs));
        }
    }

    /// Bind this manager's exit conventions to the executor the backend will run on.
    ///
    /// Called at registration, beside the rest of that backend's definition, so a manager
    /// with a new convention is added by declaring one — never by editing this file.
    /// Can this executor's manager tell LiNix that a name does not exist?
    ///
    /// Exposed so the property is *observable*: a backend that quietly lost its policy still
    /// runs the same argv, so nothing else can see the loss. `cargo` and `pipx` lost theirs in
    /// the 2026-08-04 conversion with every argv assertion green.
    pub fn classifies_absent_names(&self) -> bool {
        !self.exit_policy.absent_markers.is_empty()
    }

    /// Whether this executor's manager means `code` as an outcome that is not a failure.
    ///
    /// Exposed for the reason [`classifies_absent_names`](Self::classifies_absent_names) is: a
    /// backend that never attached its policy runs identical argv, so nothing else can observe
    /// the loss.
    pub fn forgives_exit(&self, code: i32) -> bool {
        self.exit_policy.is_benign(Some(code))
    }

    pub fn with_exit_policy(mut self, policy: ExitPolicy) -> Self {
        self.exit_policy = Arc::new(policy);
        self
    }

    pub fn is_root() -> bool {
        #[cfg(unix)]
        {
            unsafe { libc::geteuid() == 0 }
        }
        #[cfg(windows)]
        {
            false
        }
    }

    /// Run a command and return its raw output WITHOUT enforcing exit status. Reads and
    /// existence probes use this (directly or via `run_output`), because a non-zero exit
    /// is frequently a normal answer there — an empty search, a "not installed" query, an
    /// inactive service unit. Mutating callers must use `run`/`run_exclusive` instead.
    async fn run_raw(&self, cmd: &str, args: &[&str], sudo: bool) -> Result<StdOutput> {
        self.run_on(&self.inner, cmd, args, sudo).await
    }

    /// The same primitive, aimed at the layer that never stubs. Reads only.
    async fn read_raw(&self, cmd: &str, args: &[&str], sudo: bool) -> Result<StdOutput> {
        self.run_on(&self.reader, cmd, args, sudo).await
    }

    async fn run_on(
        &self,
        layer: &Arc<dyn ExecutionLayer>,
        cmd: &str,
        args: &[&str],
        sudo: bool,
    ) -> Result<StdOutput> {
        let mut final_cmd = cmd.to_string();
        let mut final_args: Vec<String> = args.iter().map(|s| s.to_string()).collect();

        if sudo && !cfg!(windows) && !Self::is_root() {
            final_args.insert(0, final_cmd);
            final_cmd = "sudo".to_string();
        }

        // `apt install`, run by a sync that already holds the data-directory lock, fires the
        // `DPkg::Post-Invoke` hook LiNix installed — which is another `linix`, and it would
        // wait on a lock this process does not release until it exits. The env var travels
        // to every descendant, and `hook-reconcile` stands down when it sees it.
        let mut env = HashMap::new();
        env.insert(
            crate::core::executor::INSIDE_LINIX.to_string(),
            std::process::id().to_string(),
        );
        Self::suppress_pagers(&mut env);

        // Every manager invocation funnels through this one call, which is what makes
        // `--timings` a breakdown of the whole run rather than of whichever verbs remembered
        // to instrument themselves.
        let timing = crate::core::timing::begin();
        let result = layer.execute(&final_cmd, &final_args, &env).await;
        crate::core::timing::end(timing, &final_cmd, &final_args);
        result
    }

    /// Stop a child from piping itself into a pager.
    ///
    /// `systemctl status`, `git log` and friends page when they believe a human is watching.
    /// A pager waits for a keypress that a captured child will never get, so the run hangs;
    /// and even when it does not, the escape sequences and the `lines 1-16/16 (END)` banner
    /// land in the text a parser is about to read. Capturing stdout removes the usual trigger,
    /// but `$PAGER`/`$SYSTEMD_PAGER` in the user's environment forces one anyway — so the
    /// suppression is set here, on the one env map every spawn inherits, rather than trusted
    /// to the absence of a terminal.
    fn suppress_pagers(env: &mut HashMap<String, String>) {
        // systemd reads an empty value as "no pager"; git and the rest need a command that
        // exists and exits, so `cat`.
        env.insert("SYSTEMD_PAGER".to_string(), String::new());
        env.insert("SYSTEMD_LESS".to_string(), String::new());
        env.insert("PAGER".to_string(), "cat".to_string());
        env.insert("GIT_PAGER".to_string(), "cat".to_string());
    }

    /// Run a *mutating* command and enforce success. `RawExecutor::execute` hands back the
    /// process output regardless of exit status, so without this a failed `apt remove` /
    /// `npm install` / `btrfs subvolume delete` would be silently reported as OK and the
    /// caller would trust a mutation that never actually happened. Callers that legitimately
    /// tolerate a non-zero exit (searches, existence probes) must use `run_output`/`run_raw`.
    pub async fn run(&self, cmd: &str, args: &[&str], sudo: bool) -> Result<StdOutput> {
        let output = self.run_raw(cmd, args, sudo).await?;
        let checked = self.ensure_status(cmd, output);
        // A mutation is the only thing that can change what is on PATH — or what is installed —
        // during a one-shot run. Installing `npm` and then asking whether `npm` is available is
        // a real sequence inside one `sync`. Every read stays memoised; a mutation is the edge
        // that invalidates them.
        self.forget_run_scoped_answers();
        checked
    }

    /// This run's installed listings, shared by every backend built on this executor.
    pub fn installed_listings(&self) -> &Arc<crate::core::installed::InstalledListings> {
        &self.installed
    }

    /// A read whose answer is its stdout.
    ///
    /// **A non-zero exit is tolerated; a non-zero exit that said nothing at all is not.** The
    /// distinction is the whole of it. "No such package" and "no results" are legitimate
    /// non-zero replies and they arrive with their reason on the page, so the exit code alone
    /// must never be the verdict — that is why this goes through the unchecked primitive rather
    /// than `run`. But a failure with nothing on *either* stream is not a reply at all, and
    /// returning `Ok("")` for it hands every caller an answer the manager never gave.
    ///
    /// Measured: 3 of 16 concurrent cold-start `winget list` exit `0x8A150001` having written
    /// zero bytes anywhere. Read as an empty listing, that made `linix list --backend winget`
    /// print nothing and exit 0 on a machine with 280 packages on it — and `info` report an
    /// installed package as absent, which is the shape it was first noticed in.
    ///
    /// **Retried when — and only when — the failure is classified transient.** Reads are
    /// idempotent, which is the whole justification: asking a manager what it has, twice,
    /// costs a second, where a mutation retried on a guess installs something twice. The
    /// measured case is a cold-start collision that a warm winget does not reproduce, so the
    /// second attempt is usually the entire fix.
    pub async fn run_output(&self, cmd: &str, args: &[&str], sudo: bool) -> Result<String> {
        let attempts = read_retry_attempts();
        let mut backoff = std::time::Duration::from_millis(200);
        for attempt in 1..=attempts {
            let output = self.read_raw(cmd, args, sudo).await?;
            let stdout = crate::utils::text::sanitize(&String::from_utf8_lossy(&output.stdout));
            let benign = output.status.success() || self.exit_policy.is_benign(output.status.code());
            // **Both streams, not just stdout.** A read that failed and *said* something has
            // described its own situation, and the caller may legitimately read that as an
            // empty result: `Get-ComputerRestorePoint` on an unelevated shell exits 1 with
            // `Access denied` on stderr and nothing on stdout, and treating that as fatal
            // failed a whole `sync` that had no business caring. Whether *that* is the right
            // answer is a separate question about snapshots, not about this primitive.
            //
            // Silence on both is the case with no second reading. Nothing expresses "you have
            // none of these" by saying nothing at all and failing.
            let said_nothing =
                stdout.trim().is_empty() && String::from_utf8_lossy(&output.stderr).trim().is_empty();
            if benign || !said_nothing {
                return Ok(stdout);
            }
            let err = self.answerless_read(cmd, args, &output);
            if attempt == attempts || err.retryability() != Retryability::Transient {
                return Err(err);
            }
            debug!(
                "`{cmd}` produced no answer and the failure is transient; \
                 asking again ({attempt}/{attempts})"
            );
            tokio::time::sleep(backoff).await;
            backoff *= 3;
        }
        unreachable!("the loop returns on its last attempt")
    }

    /// The failure for a read that exited non-zero without producing an answer.
    ///
    /// Carries the manager's own words when it left any, and its retryability, so the retry
    /// loop does not have to read the sentence back to decide what to do.
    fn answerless_read(&self, cmd: &str, args: &[&str], output: &StdOutput) -> Error {
        let code = output
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "terminated by signal".to_string());
        let stderr = String::from_utf8_lossy(&output.stderr);
        let said = stderr.trim().lines().next().unwrap_or("").trim().to_string();
        let detail = if said.is_empty() {
            String::new()
        } else {
            format!(" It said: {said}")
        };
        let hay = ExitPolicy::haystack(&output.stdout, &output.stderr);
        Error::CommandFailed {
            message: format!(
                "`{} {}` exited {code} with no output, so LiNix has no answer from it — not an \
                 empty one.{detail}",
                cmd,
                args.join(" ")
            ),
            // By code as well as by text. This is the one failure whose haystack is reliably
            // empty, so the text lists cannot classify it and the code is the only signal
            // there is.
            retry: self
                .exit_policy
                .retryability_of(output.status.code(), &hay),
            absent_name: false,
        }
    }

    /// A read whose emptiness is an *answer*, so a command that could not produce one must
    /// say so instead of returning nothing.
    ///
    /// "This manager has no such package" and "this manager has no package index" both print
    /// nothing. Reading the second as the first is how a bare name walks past the manager
    /// that has it and freezes to a lower one (V.7c). A non-zero exit alone is not the
    /// signal — `pacman -Ss`, `dnf search` and `brew search` all exit non-zero for an
    /// ordinary empty result — so the fault is a non-zero exit *with* a complaint on stderr.
    pub async fn search_output(&self, cmd: &str, args: &[&str], sudo: bool) -> Result<String> {
        let output = self.read_raw(cmd, args, sudo).await?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        let complaint = stderr.trim();
        if !output.status.success() && !complaint.is_empty() {
            let first = complaint.lines().next().unwrap_or(complaint);
            // Not `CommandFailed`: this sentence is read by a user, in a line that
            // already says which manager and which package, and "Command execution
            // failed:" in front of it is noise.
            return Err(Error::Other(format!(
                "`{}` could not answer: {}",
                cmd, first
            )));
        }
        Ok(crate::utils::text::sanitize(&String::from_utf8_lossy(
            &output.stdout,
        )))
    }

    /// The file behind `run_exclusive`'s cross-process lock, in LiNix's own data directory.
    ///
    /// It lived at a fixed, guessable name in the shared temp directory and was opened with
    /// `File::create`, which truncates and follows symlinks — so anyone with write access to
    /// that directory could pre-plant `linix_apt.lock` as a symlink and have the next
    /// exclusive run, frequently privileged, truncate the target. `datalock.rs` had already
    /// solved this; this is the same treatment, so there is one locking style in the tree and
    /// not two.
    fn open_exec_lock(lock_key: &str) -> Result<File> {
        Self::open_lock_at(&crate::utils::safe_data_dir().join("exec-locks"), lock_key)
    }

    pub(crate) fn open_lock_at(dir: &Path, lock_key: &str) -> Result<File> {
        // Created once per process, not once per lock: this runs on every exclusive command,
        // for a directory that exists after the first one.
        if LOCK_DIRS.insert(dir.to_path_buf()) {
            std::fs::create_dir_all(dir).map_err(Error::from)?;
        }
        // A lock key is a backend name, and a backend name comes from a config file. Anything
        // that is not a plain name would otherwise pick the directory the lock lands in.
        let stem: String = lock_key
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join(format!("{}.lock", stem)))
            .map_err(Error::from)
    }

    pub async fn run_exclusive(
        &self,
        lock_key: &str,
        cmd: &str,
        args: &[&str],
        sudo: bool,
    ) -> Result<StdOutput> {
        let mutex = self
            .lock_map
            .entry(lock_key.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let _thread_guard = mutex.lock().await;

        if self.dry_run {
            return self.run(cmd, args, sudo).await;
        }

        let lock_file = Self::open_exec_lock(lock_key)?;
        // `fs2`'s flock is a blocking syscall. When a second `linix` holds this lock, taking it
        // inline parks a whole runtime worker for however long that other process runs —
        // minutes, for an `apt dist-upgrade`. The wait belongs on the blocking pool; the file
        // comes back out so the unlock below is the same handle.
        let lock_file =
            tokio::task::spawn_blocking(move || lock_file.lock_exclusive().map(|_| lock_file))
                .await
                .map_err(|e| Error::Io(format!("waiting for the `{}` lock: {}", lock_key, e)))?
                .map_err(Error::from)?;
        let result = self.run_raw(cmd, args, sudo).await;
        let _ = lock_file.unlock();
        // Same invalidation as `run`, and it has to be here too: this is the path most installs
        // and removals actually take, and it reaches `run_raw` directly rather than through
        // `run`. Without it the run-scoped listings would still be answering from before the
        // install — which `Prior` reads to decide what a rollback puts back.
        self.forget_run_scoped_answers();
        // Enforce status only after releasing the lock, so a failed mutation still frees it.
        self.ensure_status(cmd, result?)
    }

    /// Forget everything memoised for the length of a run that a mutation could have changed:
    /// what is on `PATH`, and what each manager has installed.
    fn forget_run_scoped_answers(&self) {
        forget_path_lookups();
        self.installed.forget_all();
    }

    /// What a failed command's own output is allowed to put on a terminal.
    ///
    /// A manager's stream is untrusted text of unbounded length: one `scoop` typo produced ~110
    /// lines of unrelated bucket commits with raw SGR sequences, and the sentence that mattered
    /// — `Couldn't find manifest for 'x'` — was the fourth of them. The escapes were already
    /// handled for the *machine* (`ExitPolicy::opening` strips them so detection works) and not
    /// for the person, which is the wrong way round.
    ///
    /// So: the manager's own vocabulary picks the lines that explain the failure; failing that,
    /// the tail, because a tool that says nothing it declared usually ends with its complaint.
    /// Whatever is dropped is named and reachable — the whole stream is logged at `-v` by the
    /// caller, and a count with nowhere to look is not one place to look.
    fn detail_for_user(policy: &ExitPolicy, stream: &str) -> String {
        /// Enough for a stack trace's opening, far short of a bucket update.
        const CAP: usize = 8;

        let lines: Vec<&str> = stream
            .lines()
            .map(str::trim_end)
            .filter(|l| !l.trim().is_empty())
            .collect();
        if lines.is_empty() {
            return String::new();
        }
        let explaining = policy.explaining_lines(stream);
        let chosen: Vec<&str> = if !explaining.is_empty() {
            explaining.into_iter().take(CAP).collect()
        } else if lines.len() > CAP {
            lines[lines.len() - CAP..].to_vec()
        } else {
            lines.clone()
        };
        /// A line long enough for any sentence a manager writes, and short enough that a stream
        /// with no newlines in it — a progress bar drawn with bare carriage returns is one long
        /// line to `lines()` — cannot fill a terminal past the cap above.
        const WIDTH: usize = 240;

        let shown: Vec<String> = chosen
            .iter()
            // A tab is a column, not an escape: kept as spaces so a manager's table still reads
            // as one. Everything else that moves a cursor or reverses a line is named by
            // codepoint, by the same function the grammar's refusals use.
            .map(|l| crate::core::validator::printable(&l.replace('\t', "    ")))
            .map(|l| {
                if l.chars().count() > WIDTH {
                    let head: String = l.chars().take(WIDTH).collect();
                    format!("{head}…")
                } else {
                    l
                }
            })
            .collect();
        let dropped = lines.len().saturating_sub(shown.len());
        let mut out = shown.join("\n");
        if dropped > 0 {
            out.push_str(&format!(
                "\n({} more line(s) of output; re-run with -v to see all of it)",
                dropped
            ));
        }
        out
    }

    /// Classify a finished mutating command as success or failure, and — when it failed —
    /// whether another attempt could go differently.
    ///
    /// What a non-zero exit or a zero-exit complaint means belongs to the manager, so it is
    /// read off this executor's [`ExitPolicy`] rather than matched on the program's name
    /// here. An executor with no policy is the honest default: every non-zero exit fails and
    /// nothing is classified, which is what the retry loop already assumed.
    fn ensure_status(&self, cmd: &str, output: StdOutput) -> Result<StdOutput> {
        let status_ok = output.status.success() || self.exit_policy.is_benign(output.status.code());
        // One lowercased join of both streams for all three marker questions below. Each used
        // to build its own, so a command's whole transcript was copied and lowercased three
        // times — per package, and an `apt install` or `cargo build` transcript is not small.
        // A policy with no markers answers every question from its empty lists and never reads
        // this, so it is not built for one.
        let hay = if self.exit_policy.reads_output() {
            ExitPolicy::haystack(&output.stdout, &output.stderr)
        } else {
            String::new()
        };
        if status_ok && !self.exit_policy.signals_failure(&hay) {
            return Ok(output);
        }
        let code = output
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "terminated by signal".to_string());
        // scoop's failure marker lands on stdout, not stderr, so fall back to stdout for
        // the diagnostic when stderr is empty (e.g. a `status_ok` malignant-success case).
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = crate::utils::text::sanitize(&String::from_utf8_lossy(&output.stdout));
        let stream = {
            let e = stderr.trim();
            if e.is_empty() {
                stdout.trim()
            } else {
                e
            }
        };
        // Everything the command said, for whoever asked for the internals. The message below
        // keeps only what explains the failure, so this is the one place the rest survives.
        if !stream.is_empty() {
            debug!("`{}` said:\n{}", cmd, stream);
        }
        let detail = Self::detail_for_user(&self.exit_policy, stream);
        // A manager that exited 0 and said it failed is not described by its exit code.
        // "`scoop` failed (exit 0)" is a sentence that argues with itself, and it was the
        // first thing a user saw after a typo; when the verdict comes from what the command
        // printed, the message says that instead of quoting a status that means the opposite.
        let verdict = if output.status.success() {
            format!("`{}` reported a failure", cmd)
        } else {
            format!("`{}` failed (exit {})", cmd, code)
        };
        let msg = if detail.is_empty() {
            verdict
        } else {
            format!("{}: {}", verdict, detail)
        };
        Err(Error::CommandFailed {
            message: msg,
            retry: self.exit_policy.retryability(&hay),
            // Asked of the whole output, here, because this is the last place it exists: the
            // message keeps whichever stream was non-empty, so a marker on the other one is
            // gone by the time any caller sees it.
            absent_name: self.exit_policy.names_an_absent_package(&hay),
        })
    }

    pub async fn read_file(&self, path: &Path) -> Result<String> {
        if self.dry_run {
            if let Some(content) = self.vfs.get(path) {
                return Ok(content.clone());
            }
        }
        tokio::fs::read_to_string(path).await.map_err(Error::from)
    }

    pub fn read_file_sync(&self, path: &Path) -> Result<String> {
        if self.dry_run {
            if let Some(content) = self.vfs.get(path) {
                return Ok(content.clone());
            }
        }
        std::fs::read_to_string(path).map_err(Error::from)
    }

    pub async fn write_atomic(&self, path: &Path, content: &str) -> Result<()> {
        if self.dry_run {
            self.vfs.insert(path.to_path_buf(), content.to_string());
            return Ok(());
        }
        let dir = path
            .parent()
            .ok_or_else(|| Error::Other("Invalid path: no parent directory".into()))?;
        tokio::fs::create_dir_all(dir).await.map_err(Error::from)?;

        let mut temp_file = tokio::task::spawn_blocking({
            let dir = dir.to_path_buf();
            move || NamedTempFile::new_in(dir)
        })
        .await
        .map_err(|e| Error::Other(format!("IO thread failure: {}", e)))?
        .map_err(Error::from)?;

        temp_file
            .write_all(content.as_bytes())
            .map_err(Error::from)?;
        temp_file.persist(path).map_err(Error::from)?;
        Ok(())
    }

    /// Write content that must never be world-readable, restricted **before** it reaches its
    /// destination (T5).
    ///
    /// The restriction is applied to the temporary file and the file is then renamed into
    /// place, so there is no instant at which the target path holds readable plaintext. A
    /// chmod after the write would be that instant, however short — and a secret is exactly
    /// the file where "however short" is not an argument.
    ///
    /// On Unix the temp file is already created `0600` by `tempfile`; this asserts it rather
    /// than assuming it. On Windows the inherited ACEs are stripped and only the running user
    /// is granted access, via `icacls` — LiNix drives the tool the OS already has.
    pub async fn write_secret(&self, path: &Path, content: &str) -> Result<()> {
        if self.dry_run {
            self.vfs.insert(path.to_path_buf(), content.to_string());
            return Ok(());
        }
        let dir = path
            .parent()
            .ok_or_else(|| Error::Other("Invalid path: no parent directory".into()))?;
        tokio::fs::create_dir_all(dir).await.map_err(Error::from)?;

        let mut temp_file = tokio::task::spawn_blocking({
            let dir = dir.to_path_buf();
            move || NamedTempFile::new_in(dir)
        })
        .await
        .map_err(|e| Error::Other(format!("IO thread failure: {}", e)))?
        .map_err(Error::from)?;

        temp_file
            .write_all(content.as_bytes())
            .map_err(Error::from)?;
        temp_file.flush().map_err(Error::from)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(temp_file.path(), std::fs::Permissions::from_mode(0o600))
                .map_err(Error::from)?;
        }
        #[cfg(windows)]
        {
            restrict_to_owner(temp_file.path())?;
        }

        temp_file.persist(path).map_err(Error::from)?;
        Ok(())
    }

    pub async fn symlink(&self, src: &Path, dst: &Path) -> Result<()> {
        self.inner.symlink(src, dst).await
    }

    pub fn get_vfs_diff(&self) -> Vec<(PathBuf, String)> {
        self.vfs
            .iter()
            .map(|item| (item.key().clone(), item.value().clone()))
            .collect()
    }

    pub async fn command_exists(&self, cmd: &str) -> bool {
        self.reader.check_command(cmd)
    }

    pub fn command_exists_sync(&self, cmd: &str) -> bool {
        self.reader.check_command(cmd)
    }

    /// Refresh the `sudo` timestamp for as long as the returned guard is held, so a long sync
    /// is not interrupted halfway by a password prompt.
    ///
    /// The guard is the whole point: the previous version handed back a bare `JoinHandle`,
    /// which detaches when dropped, so the loop outlived every caller and could not be
    /// stopped by any of them.
    pub async fn start_sudo_keepalive(&self) -> SudoKeepalive {
        if cfg!(windows) || Self::is_root() || self.dry_run {
            return SudoKeepalive(None);
        }
        SudoKeepalive(Some(tokio::spawn(async move {
            loop {
                // `-n` so this never prompts. The foreground command owns the terminal; a
                // background task racing it for the same password prompt is two processes
                // reading one keyboard, and the visible one loses. An expired timestamp is
                // the foreground command's to raise, where the user can see which command
                // is asking.
                let _ = Command::new("sudo")
                    .args(["-n", "-v"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .kill_on_drop(true)
                    .status()
                    .await;
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        })))
    }
}

/// Stops the `sudo -v` loop when it goes out of scope.
pub struct SudoKeepalive(Option<tokio::task::JoinHandle<()>>);

impl SudoKeepalive {
    /// Whether a refresher is actually running — false on Windows, as root, and under
    /// `--dry-run`, where there is no timestamp to keep warm.
    pub fn is_running(&self) -> bool {
        self.0.as_ref().is_some_and(|h| !h.is_finished())
    }
}

impl Drop for SudoKeepalive {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
}

#[cfg(all(test, windows))]
mod windows_shim_tests {
    use super::windows_shim_wrap;
    use std::path::Path;

    #[test]
    fn wraps_ps1_via_command_with_out_string() {
        let (prog, args) = windows_shim_wrap(
            "scoop",
            Path::new(r"C:\tools\scoop\shims\scoop.ps1"),
            &["search".to_string(), "ripgrep".to_string()],
        )
        .expect("ps1 should be wrapped");
        assert_eq!(prog, "powershell");
        assert!(args.contains(&"-Command".to_string()));
        let command = args.last().unwrap();
        assert!(command.starts_with("$o = (scoop 'search' 'ripgrep' | Out-String"));
        assert!(command.contains("exit $LASTEXITCODE"));
    }

    #[test]
    fn ps1_args_are_single_quote_escaped_no_injection() {
        let (_prog, args) = windows_shim_wrap(
            "scoop",
            Path::new(r"C:\s.ps1"),
            &["install".to_string(), "evil'; rm x; '".to_string()],
        )
        .unwrap();
        // The embedded quote is doubled so the whole thing stays one literal string.
        assert!(args.last().unwrap().contains("'evil''; rm x; '''"));
    }

    #[test]
    fn wraps_cmd_via_cmd_c() {
        let (prog, args) =
            windows_shim_wrap("foo", Path::new(r"C:\x\foo.cmd"), &["list".to_string()]).unwrap();
        assert_eq!(prog, "cmd");
        assert_eq!(args[0], "/C");
        assert_eq!(args.last().unwrap(), "list");
    }

    #[test]
    fn leaves_exe_alone() {
        assert!(windows_shim_wrap("winget", Path::new(r"C:\x\winget.exe"), &[]).is_none());
    }
}

#[cfg(test)]
mod child_process_tests {
    use super::{ChildStdin, CommandExecutor, ExecutionLayer, MockExecutor, RawExecutor};
    use crate::core::Retryability;
    use dashmap::DashMap;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn wired() -> (CommandExecutor, Arc<MockExecutor>) {
        let vfs: Arc<DashMap<PathBuf, String>> = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let e =
            CommandExecutor::with_layer(false, false, mock.clone(), vfs, Arc::new(DashMap::new()));
        (e, mock)
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("linix-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A pager waits for a keypress nothing will send, and its escape sequences land in the
    /// text a parser is about to read. `systemctl status` and `git log` both start one.
    #[tokio::test]
    async fn every_spawn_carries_the_pager_suppression() {
        let (e, mock) = wired();
        let _ = e.run("systemctl", &["status", "--", "nginx"], false).await;
        let env = mock.last_env.lock().await.clone();
        assert_eq!(env.get("SYSTEMD_PAGER").map(String::as_str), Some(""));
        assert_eq!(env.get("PAGER").map(String::as_str), Some("cat"));
        assert_eq!(env.get("GIT_PAGER").map(String::as_str), Some("cat"));
    }

    /// A read that failed **without a word** is not a machine with nothing on it.
    ///
    /// Measured, on a real host: 3 of 16 cold-start concurrent `winget list` exit `0x8A150001`
    /// having written zero bytes to either stream. `run_output` did not look at the status, so
    /// the caller got `Ok("")`, the parser found no packages in it, and `list_installed`
    /// answered `Ok(vec![])`. Nothing anywhere thought winget had failed — LiNix believed the
    /// machine was empty and said so at exit 0:
    ///
    /// ```text
    /// round 1 : rows min=0 max=280   EMPTY_LISTINGS=1/16
    ///         rc=0  ms=2285  rows=0   <-- `linix list --backend winget` reported no packages
    /// ```
    ///
    /// A silent non-zero exit is the one case that cannot be an answer: every manager with
    /// nothing to report says so by exiting 0, or by printing a header. Saying nothing *and*
    /// failing is the absence of an answer, and the caller has to be told which it got.
    #[tokio::test]
    async fn a_read_that_failed_without_a_word_is_not_an_empty_machine() {
        let (e, mock) = wired();
        mock.set_response("winget list", Ok(super::silent_failure(1)));
        let err = e
            .run_output("winget", &["list"], false)
            .await
            .expect_err("a silent non-zero read must not read as an empty listing");
        let msg = err.to_string();
        assert!(
            msg.contains("winget"),
            "the failure must name the command that produced nothing: {msg}"
        );
        assert!(
            msg.contains("no output"),
            "the failure must say the command produced nothing, so it cannot be              mistaken for an empty result: {msg}"
        );
    }

    /// The other half, and the reason this is not simply "non-zero reads are errors". A read
    /// that exited non-zero and *printed a listing* has answered; the exit code is the manager
    /// grumbling about something else. `apt list --installed` warning about an unstable CLI is
    /// the shape.
    #[tokio::test]
    async fn a_read_that_failed_but_still_answered_keeps_its_answer() {
        let (e, mock) = wired();
        mock.set_response(
            "winget list",
            Ok(super::spoken_failure(1, "7zip.7zip  25.01
", "a warning")),
        );
        let out = e
            .run_output("winget", &["list"], false)
            .await
            .expect("a read with a listing in it is an answer, whatever the exit code");
        assert!(out.contains("7zip.7zip"), "{out}");
    }

    /// The code is the only signal this failure leaves, so the classifier has to read it —
    /// and having classified it transient, a read must actually ask again.
    ///
    /// Measured: winget loses ~3 of a cold burst of 16 concurrent listings and none of the
    /// next 32. The second attempt is usually the whole fix, which is why this is worth a
    /// retry at all — and why it is worth it only for reads, which are idempotent.
    #[tokio::test]
    async fn a_read_whose_only_signal_is_its_exit_code_is_still_classified_and_retried() {
        let vfs: Arc<DashMap<PathBuf, String>> = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let e = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(DashMap::new()),
        )
        .with_exit_policy(crate::core::exit_policy::winget());
        // 0x8A150001, silent — the exact shape measured on the host.
        mock.set_response("winget list", Ok(super::silent_failure(0x8A15_0001_u32 as i32)));

        let err = e
            .run_output("winget", &["list"], false)
            .await
            .expect_err("a silent failure is still a failure once the retries are spent");
        assert_eq!(
            err.retryability(),
            Retryability::Transient,
            "the exit code is the only thing that can classify this, and it must: {err}"
        );
        let tries = mock
            .get_calls()
            .await
            .iter()
            .filter(|c| c.as_str() == "winget list")
            .count();
        assert!(
            tries > 1,
            "a transient read was asked exactly once — the classification bought nothing              (tried {tries}x)"
        );
    }

    /// The other side: a failure the policy does **not** classify is asked once and reported.
    /// Retrying everything would turn a manager that is simply broken into a slow one.
    #[tokio::test]
    async fn an_unclassified_silent_failure_is_not_retried() {
        let vfs: Arc<DashMap<PathBuf, String>> = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let e = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(DashMap::new()),
        )
        .with_exit_policy(crate::core::exit_policy::winget());
        mock.set_response("winget list", Ok(super::silent_failure(1)));

        let _ = e.run_output("winget", &["list"], false).await.unwrap_err();
        let tries = mock
            .get_calls()
            .await
            .iter()
            .filter(|c| c.as_str() == "winget list")
            .count();
        assert_eq!(tries, 1, "an unclassified failure must be asked once, not {tries} times");
    }

    /// A read and a mutation are bounded differently, and the reason is in the numbers: the
    /// mutation bound exists for `Checkpoint-Computer`, silent for its whole run, and a read
    /// takes seconds. Collapsing them back to one number is the change this pins against —
    /// it reads as a simplification and costs fifteen minutes per wedged listing.
    #[test]
    fn a_read_is_bounded_tighter_than_a_mutation() {
        let read = super::RawExecutor::reader().idle;
        let write = super::RawExecutor::mutator().idle;
        let (Some(read), Some(write)) = (read, write) else {
            panic!("both layers must carry a bound by default: {read:?} / {write:?}");
        };
        assert!(
            read < write,
            "a read waits as long as a mutation ({read:?} vs {write:?})"
        );
        assert_eq!(read.as_secs(), super::DEFAULT_QUERY_IDLE_TIMEOUT_SECS);
        assert_eq!(write.as_secs(), super::DEFAULT_COMMAND_IDLE_TIMEOUT_SECS);
    }

    /// The boundary, and it was drawn in the wrong place first.
    ///
    /// A read that failed and *complained* is left alone here. It looks like it ought to be a
    /// failure — `search_output` treats exactly that shape as one — but a listing is not a
    /// search, and the caller may have a reading for it that this primitive cannot know.
    /// Making it fatal failed a whole `sync`: `Get-ComputerRestorePoint` on an unelevated
    /// shell exits 1 with `Access denied` on stderr and nothing on stdout, and the snapshot
    /// check that asked has every right to carry on without a restore-point list.
    ///
    /// Whether *that* caller should care is a real question and a separate one. The rule
    /// here is only about what a command said: silence on both streams has one reading,
    /// and a complaint has two.
    #[tokio::test]
    async fn a_read_that_failed_but_explained_itself_is_left_to_its_caller() {
        let (e, mock) = wired();
        mock.set_response(
            "winget list",
            Ok(super::spoken_failure(1, "", "Failed when opening source")),
        );
        let out = e
            .run_output("winget", &["list"], false)
            .await
            .expect("a read that complained has said something; this primitive does not judge it");
        assert!(out.is_empty(), "the answer is still the empty stdout: {out:?}");
    }

    /// The suppression must not be a property of the mutating path only — a read is exactly
    /// where a pager's banner corrupts the parse.
    #[tokio::test]
    async fn a_read_carries_it_too() {
        let (e, mock) = wired();
        let _ = e.run_output("git", &["log", "--oneline"], false).await;
        let env = mock.last_env.lock().await.clone();
        assert_eq!(env.get("GIT_PAGER").map(String::as_str), Some("cat"));
        assert!(env.contains_key(super::INSIDE_LINIX));
    }

    /// A child that prints nothing and never exits. `Checkpoint-Computer` past its restore
    /// point is the measured one; `sleep` is the same shape with a shorter fuse.
    fn silent_forever() -> (&'static str, Vec<String>) {
        #[cfg(unix)]
        {
            ("sleep", vec!["600".to_string()])
        }
        #[cfg(windows)]
        {
            (
                "powershell",
                vec![
                    "-NoProfile".to_string(),
                    "-Command".to_string(),
                    "Start-Sleep -Seconds 600".to_string(),
                ],
            )
        }
    }

    /// A child that hands its stdout to a background process and exits immediately.
    ///
    /// This is what a real manager does, not a contrived shape: measured on this host,
    /// `nimble` and `pnpm` both left descendants running with `PPID 0` after their direct
    /// child was gone. The pipe stays open because the orphan still holds the write end, so a
    /// read to EOF never returns — and the child whose exit the bound watches has already
    /// exited.
    /// **Windows needs a script file.** Spelling the same thing as a single `cmd /C "start /b
    /// cmd /c \"...\""` argument does not survive cmd's quote parsing — it dies instantly with
    /// `is not recognized`, and the test around it passed in 0.06s having proved nothing.
    fn detaches_holding_stdout(seconds: usize) -> (String, Vec<String>) {
        #[cfg(unix)]
        {
            (
                "sh".to_string(),
                vec!["-c".to_string(), format!("sleep {seconds} & exit 0")],
            )
        }
        #[cfg(windows)]
        {
            let path =
                std::env::temp_dir().join(format!("linix-detach-{}.cmd", std::process::id()));
            std::fs::write(
                &path,
                format!(
                    "@echo off\r\nstart /b cmd /c \"ping -n {} 127.0.0.1 > nul\"\r\nexit /b 0\r\n",
                    seconds + 1
                ),
            )
            .expect("write the detaching fixture");
            (
                "cmd".to_string(),
                vec!["/C".to_string(), path.display().to_string()],
            )
        }
    }

    /// Runs far longer than the bound, but is never quiet for it. A build, a big download.
    fn chatty_for_a_while(ticks: usize) -> (&'static str, Vec<String>) {
        #[cfg(unix)]
        {
            (
                "sh",
                vec![
                    "-c".to_string(),
                    format!(
                        "i=0; while [ $i -lt {ticks} ]; do echo tick; i=$((i+1)); sleep 0.5; done"
                    ),
                ],
            )
        }
        #[cfg(windows)]
        {
            (
                "powershell",
                vec![
                    "-NoProfile".to_string(),
                    "-Command".to_string(),
                    // `[Console]::Out`, not `Write-Output`: PowerShell buffers the pipeline
                    // when stdout is redirected, so a fixture that "talks" through the
                    // pipeline can be silent for its whole run — which is the one thing this
                    // fixture must not be.
                    format!(
                        "1..{ticks} | ForEach-Object {{ [Console]::Out.WriteLine('tick'); \
                         [Console]::Out.Flush(); Start-Sleep -Milliseconds 500 }}"
                    ),
                ],
            )
        }
    }

    /// What this host charges to start the interpreter and get one line out of it.
    ///
    /// **The idle clock starts at spawn, so start-up is silence.** That is right for the product
    /// — an interpreter that never starts *is* a hang — and it makes any fixed bound in a test
    /// below an assertion about the host's scheduler rather than about the code. The bound was
    /// 3s and failed on a machine running three `cargo test` jobs; it was raised to 5s and
    /// failed again on 2026-08-04, under a `cargo test` sharing the box with a container build.
    /// Raising a constant that has already been raised once is not a fix. So the cost is
    /// measured on the machine the test is running on, and the bound is set from it.
    async fn interpreter_start_up_cost() -> std::time::Duration {
        let layer = RawExecutor::with_idle(ChildStdin::Closed, None);
        let (cmd, args) = chatty_for_a_while(1);
        let started = std::time::Instant::now();
        let _ = layer.execute(cmd, &args, &HashMap::new()).await;
        started.elapsed()
    }

    /// The bug: `linix uninstall choco:bat` sat 76 minutes on a `Checkpoint-Computer` that had
    /// already written its restore point, because nothing outside the DAG bounded a child at
    /// all. Two earlier hangs were killed by hand and recorded as undiagnosed.
    #[tokio::test]
    async fn a_child_that_goes_silent_is_killed_and_named() {
        let layer =
            RawExecutor::with_idle(ChildStdin::Closed, Some(std::time::Duration::from_secs(2)));
        let (cmd, args) = silent_forever();
        let started = std::time::Instant::now();
        let err = layer
            .execute(cmd, &args, &HashMap::new())
            .await
            .expect_err("a child silent past the bound must not be waited on forever");
        let waited = started.elapsed();

        assert!(
            waited < std::time::Duration::from_secs(45),
            "waited {:?}, so the bound did not fire",
            waited
        );
        let message = err.to_string();
        assert!(
            message.contains("produced no output"),
            "the error must say what happened: {}",
            message
        );
        // Naming the program alone does not identify a hang on a host running six of them.
        assert!(
            message.contains(cmd) && args.iter().all(|a| message.contains(a.as_str())),
            "the error must name the command that hung: {}",
            message
        );
        assert!(
            message.contains("command_idle_timeout_secs"),
            "the error must name the dial that changes it: {}",
            message
        );
    }

    /// The bound is on silence, not on duration — the distinction the whole fix rests on. A
    /// wall-clock cap set low enough to catch the hang would kill every real build.
    ///
    /// **The bound is measured, not written down.** It is this host's interpreter start-up cost
    /// plus three seconds, because the idle clock starts at spawn and start-up therefore counts
    /// as silence — see [`interpreter_start_up_cost`]. A constant here is an assertion about the
    /// machine: 3s failed under three concurrent `cargo test` jobs, 5s failed under one sharing
    /// the box with a container build, and the next constant fails on the next busier day. The
    /// property under test never changed: a child that runs *longer* than the idle bound while
    /// still printing survives it.
    ///
    /// **How long it talks is derived from the bound, not written down either** — and that is the
    /// second calibration this test needed. A fixed twelve ticks was six seconds of talking
    /// against a bound of *measured start-up + 3s*, which holds only while the fixture's own
    /// start-up matches the one the calibration measured. Under a full `cargo test
    /// --no-fail-fast` they diverged — the calibration caught a 4.6s start-up and the fixture a
    /// 1.5s one — so the run finished **29ms under** its own bound and the self-test below
    /// correctly said the test had proved nothing. Talking for the whole bound plus the margin is
    /// true whatever either start-up turns out to be.
    #[tokio::test]
    async fn a_child_that_keeps_talking_outlives_the_bound() {
        let margin = std::time::Duration::from_secs(3);
        let bound = interpreter_start_up_cost().await + margin;
        let ticks = ((bound + margin).as_millis() / 500) as usize;

        let layer = RawExecutor::with_idle(ChildStdin::Closed, Some(bound));
        let (cmd, args) = chatty_for_a_while(ticks);
        let started = std::time::Instant::now();
        let out = layer
            .execute(cmd, &args, &HashMap::new())
            .await
            .unwrap_or_else(|e| {
                panic!("a command that is still printing has not hung (bound {bound:?}): {e}")
            });
        assert!(
            started.elapsed() > bound,
            "the fixture ran {:?} against a {bound:?} bound — it must outlive it or it proves \
             nothing",
            started.elapsed()
        );
        assert!(out.status.success());
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).matches("tick").count(),
            ticks
        );
    }

    /// `0` in the config means the old behaviour, for whoever has a legitimately silent
    /// half-hour command and would rather wait than tune a number.
    #[tokio::test]
    async fn no_bound_means_no_bound() {
        let layer = RawExecutor::with_idle(ChildStdin::Closed, None);
        let (cmd, args) = silent_forever();
        let ran = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            layer.execute(cmd, &args, &HashMap::new()),
        )
        .await;
        assert!(ran.is_err(), "an unbounded layer must still be waiting");
    }

    /// The bound watches the child's **exit**, and the read of its output sits outside it.
    ///
    /// A manager that backgrounds its work and returns leaves LiNix reading a pipe the orphan
    /// still holds open: `child.wait()` has already returned, so the loop that could abort the
    /// readers is over, and `out_task.await` has no clock on it at all. There is no child left
    /// to kill and nothing in the tree that can end the wait.
    ///
    /// Found by capturing a wedged sweep instead of killing it: `linix -y install
    /// nimble:nimjson` at zero CPU with **no children**, while three orphaned `nim.exe` ran
    /// outside its process tree. Then reproduced to a number — a 20s bound and a child that
    /// detached for 60s took 64s and reported SUCCESS.
    #[tokio::test]
    async fn a_detached_grandchild_cannot_hold_the_read_open_past_the_bound() {
        const HOLD: usize = 10;
        let (cmd, args) = detaches_holding_stdout(HOLD);

        // **The fixture is checked before it is trusted.** The first version of this test used a
        // command Windows rejected for bad quoting, so it returned in 0.06s and passed while
        // proving nothing — the defect class this whole file exists to catch. An unbounded layer
        // must sit here for the orphan's whole life; if it does not, the fixture is not holding
        // the pipe and the assertion below would be meaningless.
        let control = RawExecutor::with_idle(ChildStdin::Closed, None);
        let t0 = std::time::Instant::now();
        let _ = control.execute(&cmd, &args, &HashMap::new()).await;
        let held = t0.elapsed();
        assert!(
            held >= std::time::Duration::from_secs(7),
            "the fixture returned after {:?}, so no orphan is holding the pipe and this test \
             cannot fail — fix the fixture before reading the result",
            held
        );

        let layer =
            RawExecutor::with_idle(ChildStdin::Closed, Some(std::time::Duration::from_secs(2)));
        let started = std::time::Instant::now();
        let outcome = layer.execute(&cmd, &args, &HashMap::new()).await;
        let waited = started.elapsed();

        // Above the bound and below the orphan's lifetime: this asserts that *a* bound applied,
        // not how long it took to apply.
        assert!(
            waited < std::time::Duration::from_secs(7),
            "waited {:?} for a child that had already exited, on a 2s bound — the bound covers \
             the wait but not the read, so an orphan holding the pipe sets the duration",
            waited
        );
        // The separable half, and the worse one: the child exits 0, so before the bound reached
        // the readers this returned SUCCESS after waiting out the orphan. A command LiNix
        // stopped waiting on did not do what it was asked (Q28).
        let err = outcome.expect_err("a command LiNix gave up on must not report success");
        let said = err.to_string();
        assert!(
            said.contains(&cmd) && said.contains("holds its output open"),
            "the failure must name the command and why LiNix stopped: {}",
            said
        );
        assert_eq!(
            err.retryability(),
            Retryability::Permanent,
            "a second attempt spends another bound reproducing this"
        );
    }

    /// A bound that reports the hang but leaves the process running has moved the leak, not
    /// closed it — the wedged `Checkpoint-Computer` held its restore point for 76 minutes.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_killed_child_is_actually_dead() {
        let layer =
            RawExecutor::with_idle(ChildStdin::Closed, Some(std::time::Duration::from_secs(2)));
        // `pgrep -f` on a marker only this test uses; a survivor is visible to the whole host.
        let marker = format!("linix-idle-probe-{}", std::process::id());
        let args = vec!["-c".to_string(), format!("# {}\nsleep 600", marker)];
        let _ = layer.execute("sh", &args, &HashMap::new()).await;
        let found = std::process::Command::new("pgrep")
            .args(["-f", &marker])
            .output();
        if let Ok(found) = found {
            assert!(
                String::from_utf8_lossy(&found.stdout).trim().is_empty(),
                "the child outlived the bound that killed it"
            );
        }
    }

    /// Reads must never take the terminal: a read is answered by parsing its output, and a
    /// read that could block on input has nobody to answer it.
    #[test]
    fn the_reader_layer_never_shares_stdin() {
        assert_eq!(RawExecutor::reader().stdin, ChildStdin::Closed);
        // U40's *reason* is `sudo`'s password prompt, and `run_on` never inserts `sudo` on
        // Windows — so the rule reaches only the platform the reason reaches.
        assert_eq!(
            RawExecutor::mutator().stdin,
            if cfg!(windows) {
                ChildStdin::Closed
            } else {
                ChildStdin::Interactive
            }
        );
    }

    /// The two layers a `CommandExecutor` builds must be the two policies, not one policy
    /// twice — routing reads through the mutating layer is how the parsers were starved.
    ///
    /// The mutating layer shares stdin wherever `sudo` can be inserted, and only there: on
    /// Windows `run_on` never inserts it, so the terminal buys nothing and costs the whole
    /// idle bound the first time a manager asks a question (Q35).
    #[test]
    fn a_real_executor_wires_a_reader_and_a_mutator() {
        let e = CommandExecutor::new(false, false);
        assert!(!e.reader.shares_stdin(), "a read took the terminal");
        assert_eq!(
            e.inner.shares_stdin(),
            !cfg!(windows),
            "sudo must be able to ask for a password on the platforms it is inserted on, and \
             nothing may hold the terminal on the one where it is not"
        );
    }

    /// The lock is a shared, guessable name by design — that is what makes it a lock. It must
    /// therefore never truncate what it opens, or a symlink planted at that path destroys the
    /// file it points at, often as root.
    #[cfg(unix)]
    #[test]
    fn taking_the_exec_lock_does_not_truncate_a_planted_symlink() {
        let root = tmpdir("execlock");
        let canary = root.join("canary");
        std::fs::write(&canary, "must survive").unwrap();
        let lock_dir = root.join("exec-locks");
        std::fs::create_dir_all(&lock_dir).unwrap();
        std::os::unix::fs::symlink(&canary, lock_dir.join("apt.lock")).unwrap();

        drop(CommandExecutor::open_lock_at(&lock_dir, "apt").unwrap());
        assert_eq!(std::fs::read_to_string(&canary).unwrap(), "must survive");
    }

    /// A key is a backend name from a config file; one carrying a separator would otherwise
    /// pick the directory the lock file lands in.
    #[test]
    fn a_lock_key_cannot_escape_the_lock_directory() {
        let root = tmpdir("execkey");
        let lock_dir = root.join("exec-locks");
        assert!(CommandExecutor::open_lock_at(&lock_dir, "../../evil").is_ok());
        let landed: Vec<String> = std::fs::read_dir(&lock_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(landed.len(), 1, "{:?}", landed);
        assert!(!landed[0].contains('.') || landed[0].ends_with(".lock"));
        assert!(landed[0].ends_with("evil.lock"), "{:?}", landed);
        assert!(!root.parent().unwrap().join("evil.lock").exists());
    }

    /// An existing lock file must survive being opened — the contended case is a second
    /// process arriving while the first holds it, and truncating it there is how the owner
    /// stamp beside it stopped meaning anything.
    #[test]
    fn opening_an_existing_lock_keeps_its_contents() {
        let root = tmpdir("execkeep");
        let lock_dir = root.join("exec-locks");
        std::fs::create_dir_all(&lock_dir).unwrap();
        std::fs::write(lock_dir.join("dnf.lock"), "held").unwrap();
        drop(CommandExecutor::open_lock_at(&lock_dir, "dnf").unwrap());
        assert_eq!(
            std::fs::read_to_string(lock_dir.join("dnf.lock")).unwrap(),
            "held"
        );
    }

    /// The old keepalive returned a bare `JoinHandle`, which detaches on drop — so nothing a
    /// caller did could stop the loop.
    #[tokio::test]
    async fn dropping_the_keepalive_guard_stops_the_loop() {
        let e = CommandExecutor::new(false, false);
        let keep = e.start_sudo_keepalive().await;
        if !keep.is_running() {
            return; // root, Windows: there is no timestamp to refresh
        }
        let handle = keep.0.as_ref().map(|h| h.abort_handle()).unwrap();
        drop(keep);
        for _ in 0..200 {
            if handle.is_finished() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("the keepalive outlived its guard");
    }

    /// `envs()` adds to what the child inherits, so a value LiNix sets must win over the same
    /// name in the environment it was started with.
    /// The fabricated statuses used to be `/bin/false` and `cmd /C exit 1` with `.expect()` on
    /// the spawn — a panic on the one path whose premise is that nothing runs.
    #[test]
    fn a_fabricated_status_carries_the_code_without_spawning_anything() {
        let ok: std::process::Output = super::DryRunOutput::new().into();
        assert!(ok.status.success());
        assert_eq!(ok.status.code(), Some(0));

        let bad = super::DryRunOutput::faulted("E: no package index");
        assert!(!bad.status.success());
        assert_eq!(bad.status.code(), Some(1));
        assert_eq!(bad.stderr, b"E: no package index");
    }

    #[test]
    fn the_suppression_overrides_a_user_pager() {
        let mut env = HashMap::new();
        env.insert("PAGER".to_string(), "less -R".to_string());
        CommandExecutor::suppress_pagers(&mut env);
        assert_eq!(env.get("PAGER").map(String::as_str), Some("cat"));
    }
}

#[cfg(test)]
mod search_read_tests {
    use super::{CommandExecutor, DryRunOutput, MockExecutor, StdOutput};
    use dashmap::DashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn exec(cmdline: &str, response: StdOutput) -> (CommandExecutor, Arc<MockExecutor>) {
        let vfs: Arc<DashMap<PathBuf, String>> = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        mock.set_response(cmdline, Ok(response));
        let e =
            CommandExecutor::with_layer(false, false, mock.clone(), vfs, Arc::new(DashMap::new()));
        (e, mock)
    }

    /// Every manager that can be first in `priority` reaches this through its own
    /// `search`; the rule has to hold for all of them, so it is tested here once.
    #[tokio::test]
    async fn a_search_that_could_not_run_is_an_error_not_an_empty_answer() {
        let (e, _m) = exec(
            "apt-cache search jq",
            DryRunOutput::faulted("E: The package lists are empty."),
        );
        let err = e
            .search_output("apt-cache", &["search", "jq"], false)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("could not answer"), "{}", err);
        assert!(err.contains("package lists are empty"), "{}", err);
    }

    /// `pacman -Ss`, `dnf search` and `brew search` all exit non-zero when the query
    /// simply matched nothing. That is an answer, and must survive as one.
    #[tokio::test]
    async fn a_quiet_nonzero_exit_is_an_ordinary_empty_result() {
        let (e, _m) = exec("pacman -Ss nosuchpkg", DryRunOutput::faulted(""));
        let out = e
            .search_output("pacman", &["-Ss", "nosuchpkg"], false)
            .await
            .expect("an empty search is not a fault");
        assert!(out.is_empty());
    }

    /// A manager that warns on the way to a real answer has still answered.
    #[tokio::test]
    async fn a_warning_alongside_a_successful_run_is_not_a_fault() {
        let mut ok: StdOutput = DryRunOutput::new().into();
        ok.stdout = b"jq - lightweight JSON processor\n".to_vec();
        ok.stderr = b"WARNING: repository is out of date\n".to_vec();
        let (e, _m) = exec("apt-cache search jq", ok);
        let out = e
            .search_output("apt-cache", &["search", "jq"], false)
            .await
            .unwrap();
        assert!(out.contains("lightweight JSON processor"), "{}", out);
    }
}

#[cfg(test)]
mod exit_status_tests {
    use super::{fabricate_status, CommandExecutor};
    use crate::core::{exit_policy, Error, ExitPolicy, Retryability};
    use std::process::Output as StdOutput;

    fn finished(code: i32, stdout: &str, stderr: &str) -> StdOutput {
        StdOutput {
            status: fabricate_status(code),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    fn executor_for(policy: ExitPolicy) -> CommandExecutor {
        CommandExecutor::new(true, false).with_exit_policy(policy)
    }

    #[test]
    fn scoop_missing_manifest_is_a_failure_despite_exit_zero() {
        let scoop = executor_for(exit_policy::scoop());
        let out = "Couldn't find manifest for 'linix-nonexistent-pkg'.\n";
        assert!(scoop.ensure_status("scoop", finished(0, out, "")).is_err());
        assert!(scoop
            .ensure_status(
                "scoop",
                finished(0, "'jq' (1.8.2) was installed successfully!\n", "")
            )
            .is_ok());
    }

    /// The marker travels with the backend, not with the program name — so a scoop that
    /// resolves to `scoop.ps1`, or runs through a shim, is classified the same way, and no
    /// other manager inherits scoop's marker.
    #[test]
    fn a_policy_belongs_to_its_backend_and_not_to_a_program_name() {
        let out = "Couldn't find manifest for 'nope'.\n";
        let scoop = executor_for(exit_policy::scoop());
        assert!(scoop
            .ensure_status(r"C:\Users\me\scoop\shims\scoop.ps1", finished(0, out, ""))
            .is_err());
        let apt = executor_for(exit_policy::apt());
        assert!(apt.ensure_status("apt-get", finished(0, out, "")).is_ok());
    }

    /// G-5. One `scoop` typo printed ~110 lines of unrelated bucket commits at the user, with
    /// raw SGR sequences, and the sentence that mattered was the fourth of them. The stream is
    /// reproduced here in its real shape: a coloured banner, a long body, the complaint early.
    #[test]
    fn a_failed_command_shows_the_line_that_explains_it_and_not_the_bucket_update() {
        let mut stream =
            String::from("\u{1b}[32mUpdating Scoop...\u{1b}[0m\nUpdating 'main' bucket...\n");
        stream.push_str("Couldn't find manifest for 'definitely-not-real-xyz123'.\n");
        for i in 0..110 {
            stream.push_str(&format!("   \u{1b}[33m* commit {i} in main\u{1b}[0m\n"));
        }
        let err = executor_for(exit_policy::scoop())
            .ensure_status("scoop", finished(0, &stream, ""))
            .unwrap_err();
        let msg = err.to_string();

        assert!(
            msg.contains("Couldn't find manifest for 'definitely-not-real-xyz123'"),
            "the message dropped the one line that explains the failure:\n{msg}"
        );
        assert!(
            !msg.contains('\u{1b}'),
            "an escape sequence reached the user's terminal; the machine was protected from \
             these and the human was not:\n{msg:?}"
        );
        assert!(
            !msg.contains("commit 42 in main"),
            "the whole bucket update was pasted at the user:\n{msg}"
        );
        assert!(
            msg.lines().count() <= 10,
            "{} lines of a manager's output reached the user; a failure names one place to \
             look:\n{msg}",
            msg.lines().count()
        );
        assert!(
            msg.contains("more line(s) of output"),
            "output was dropped and not accounted for; a count with nowhere to look is not one \
             place to look:\n{msg}"
        );
    }

    /// A manager with nothing declared still has to be legible: no markers, so no line can be
    /// singled out, and the tail is what a tool that failed usually ends with.
    #[test]
    fn a_manager_with_a_bare_policy_gets_the_tail_and_a_count() {
        let stream: String = (0..40).map(|i| format!("line {i}\n")).collect();
        let err = executor_for(ExitPolicy::default())
            .ensure_status("somepm", finished(1, "", &stream))
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("line 39"), "the tail is missing:\n{msg}");
        assert!(!msg.contains("line 0\n"), "the head was kept:\n{msg}");
        assert!(msg.contains("32 more line(s)"), "no count:\n{msg}");
    }

    /// A stream with no newlines is one line, and one line has no cap without this. winget draws
    /// its progress spinner with bare carriage returns, which `lines()` does not split on.
    #[test]
    fn a_single_enormous_line_is_cut_rather_than_printed_whole() {
        let stream = "x".repeat(9_000);
        let err = executor_for(ExitPolicy::default())
            .ensure_status("somepm", finished(1, "", &stream))
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.chars().count() < 400,
            "{} characters reached the user from a single line",
            msg.chars().count()
        );
        assert!(msg.contains('…'), "nothing said it had been cut:\n{msg}");
    }

    /// Trojan source, in a package manager's output rather than in a module: U+202E reverses
    /// everything after it as it renders, so a failure can be made to read as its opposite.
    /// The grammar's refusals have named it by codepoint since W38; a command's output did not.
    #[test]
    fn an_invisible_character_in_a_managers_output_is_named_not_drawn() {
        let err = executor_for(ExitPolicy::default())
            .ensure_status("somepm", finished(1, "", "failed: \u{202E}drowssap\n"))
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("<U+202E>"),
            "the override was not named:\n{msg}"
        );
        assert!(
            !msg.contains('\u{202E}'),
            "the override was reprinted at the terminal"
        );
    }

    #[test]
    fn ordinary_nonzero_is_never_benign() {
        for (policy, cmd, code) in [
            (exit_policy::apk(), "apk", 1),
            (exit_policy::apt(), "apt-get", 100),
            (exit_policy::dnf(), "dnf", 1),
        ] {
            assert!(executor_for(policy)
                .ensure_status(cmd, finished(code, "", "boom"))
                .is_err());
        }
    }

    /// A Windows exit code does not survive a Unix `ExitStatus`: `from_raw(code << 8)` reads
    /// back as `code & 0xff`, so 1605 becomes 69 and -1978335189 becomes something else again.
    /// These two tests ran green on Windows and red everywhere else for exactly that reason,
    /// and nothing said so until the first push in 25 commits.
    ///
    /// So the policy — which is plain data and true on every platform — is asserted directly,
    /// and the `ensure_status` wiring around it is proved with a code the OS can carry.
    #[test]
    fn choco_msi_reboot_codes_are_benign_and_do_not_leak() {
        let choco = exit_policy::choco();
        let apk = exit_policy::apk();
        for code in [1605, 1614, 1618, 1641, 3010] {
            assert!(choco.is_benign(Some(code)), "choco {code} is a reboot code");
            assert!(
                !apk.is_benign(Some(code)),
                "apk borrowed choco's code {code}"
            );
        }
        assert!(!choco.is_benign(Some(1)), "1 is a plain choco failure");
        assert!(
            !choco.is_benign(None),
            "a killed command chose no code at all"
        );
        assert!(executor_for(exit_policy::choco())
            .ensure_status("choco", finished(1, "", ""))
            .is_err());
    }

    const CHOCO_UNINSTALL_ABSENT: &str =
        include_str!("../../tests/fixtures/choco/uninstall-not-installed.txt");
    const CHOCO_INSTALL_OK: &str = include_str!("../../tests/fixtures/choco/install-success.txt");

    /// Chocolatey forces its exit code to 1 when a package failed *only if nothing else already
    /// set one* — so a dependency that asks for a reboot leaves 3010 standing over a package
    /// that never installed. 3010 is benign here, which made a 10-of-11 install a success and
    /// the harness say `choco installed bat for real` about a `bat` that was not there.
    ///
    /// The count sentence is what knows, and choco prints it only when something failed.
    /// Asserted on the policy rather than through a fabricated `ExitStatus`, for the reason the
    /// test above gives: 3010 does not survive a Unix status, and 0 does.
    #[test]
    fn a_choco_package_that_failed_outranks_a_benign_reboot_code() {
        let choco = exit_policy::choco();
        assert!(
            choco.signals_failure(&ExitPolicy::haystack(
                CHOCO_UNINSTALL_ABSENT.as_bytes(),
                b""
            )),
            "choco said 0/1 and 1 failed"
        );
        assert!(
            choco.signals_failure(&ExitPolicy::haystack(
                b"Chocolatey installed 10/11 packages. 1 packages failed.",
                b""
            )),
            "a dependency took the package down with it"
        );
        assert!(
            !choco.signals_failure(&ExitPolicy::haystack(CHOCO_INSTALL_OK.as_bytes(), b"")),
            "a clean 11/11 install says nothing about failing"
        );
        assert!(choco.is_benign(Some(3010)), "still a reboot, still benign");
        assert!(executor_for(exit_policy::choco())
            .ensure_status("choco", finished(0, CHOCO_UNINSTALL_ABSENT, ""))
            .is_err());
        assert!(executor_for(exit_policy::choco())
            .ensure_status("choco", finished(0, CHOCO_INSTALL_OK, ""))
            .is_ok());
    }

    /// Chocolatey prints six lines of generic troubleshooting advice after *any* failure, and
    /// the reason above it. With no vocabulary to pick lines by, `detail_for_user` fell back to
    /// the tail — so every choco failure ever reported was those six lines, and the sentence
    /// that said what went wrong was inside the `(13 more line(s))` nobody could read.
    #[test]
    fn a_choco_failure_names_the_reason_and_not_its_troubleshooting_footer() {
        let shown = CommandExecutor::detail_for_user(&exit_policy::choco(), CHOCO_UNINSTALL_ABSENT);
        assert!(
            shown.contains("Cannot uninstall a non-existent package"),
            "the reason is missing:\n{shown}"
        );
        assert!(
            !shown.contains("dependencies for a reason"),
            "the footer crowded out the reason:\n{shown}"
        );
    }

    /// winget is choco's twin — the only other manager that forgives a non-zero exit — and it
    /// forgives the same code for opposite events. `install` of a name that does not exist and
    /// `uninstall` of something already gone both exit -1978335212; only the second is what was
    /// asked for. Measured on Windows 11, winget's own wording tells them apart.
    #[test]
    fn winget_install_of_an_absent_name_is_not_the_success_its_exit_code_claims() {
        let winget = exit_policy::winget();
        assert!(
            winget.signals_failure(&ExitPolicy::haystack(
                b"No package found matching input criteria.",
                b""
            )),
            "nothing was installed"
        );
        assert!(
            !winget.signals_failure(&ExitPolicy::haystack(
                b"No installed package found matching input criteria.",
                b""
            )),
            "removing something already gone is the outcome that was wanted"
        );
    }

    #[test]
    fn winget_noteworthy_codes_are_benign() {
        let winget = exit_policy::winget();
        for code in [-1978335189, -1978335212, -1978335215] {
            assert!(winget.is_benign(Some(code)), "winget {code}");
        }
        assert!(!winget.is_benign(Some(1)));
        assert!(!winget.is_benign(None));
        assert!(executor_for(exit_policy::winget())
            .ensure_status("winget", finished(1, "", ""))
            .is_err());
    }

    /// The trap the two tests above fell into, stated once so it cannot be re-entered
    /// silently: a fabricated status only round-trips codes the running OS can represent.
    #[test]
    fn a_fabricated_status_round_trips_only_what_this_os_can_hold() {
        assert_eq!(finished(1, "", "").status.code(), Some(1));
        assert_eq!(finished(100, "", "").status.code(), Some(100));
        #[cfg(unix)]
        assert_ne!(
            finished(1605, "", "").status.code(),
            Some(1605),
            "a Unix status held a Windows code — the platform note above is stale"
        );
        #[cfg(windows)]
        assert_eq!(finished(1605, "", "").status.code(), Some(1605));
    }

    /// An executor nobody gave a policy fails every non-zero exit and classifies nothing —
    /// the behaviour every backend had before policies existed.
    #[test]
    fn an_executor_with_no_policy_classifies_nothing() {
        let plain = CommandExecutor::new(true, false);
        assert!(plain
            .ensure_status("choco", finished(3010, "", ""))
            .is_err());
        assert!(plain.ensure_status("apt", finished(0, "", "")).is_ok());
        let err = plain
            .ensure_status("apt", finished(100, "", "E: Unable to locate package nope"))
            .expect_err("non-zero must fail");
        assert_eq!(err.retryability(), Retryability::Unknown);
    }

    /// The point of the whole change: the failure the retry loop reads carries its own
    /// verdict, so it never has to read the message back.
    #[test]
    fn a_failure_carries_its_retryability() {
        let apt = executor_for(exit_policy::apt());
        let permanent = apt
            .ensure_status("apt", finished(100, "", "E: Unable to locate package nope"))
            .expect_err("exit 100 is a failure");
        assert_eq!(permanent.retryability(), Retryability::Permanent);

        let transient = apt
            .ensure_status(
                "apt",
                finished(100, "", "E: Could not get lock /var/lib/dpkg/lock-frontend"),
            )
            .expect_err("exit 100 is a failure");
        assert_eq!(transient.retryability(), Retryability::Transient);
        assert!(matches!(transient, Error::CommandFailed { .. }));
    }

    #[test]
    fn signal_termination_is_never_benign() {
        let choco = executor_for(exit_policy::choco());
        let killed = StdOutput {
            status: fabricate_status(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        };
        // `fabricate_status` always yields a code, so exercise the None arm directly.
        assert!(!exit_policy::choco().is_benign(None));
        assert!(!exit_policy::winget().is_benign(None));
        assert!(choco.ensure_status("choco", killed).is_ok());
    }
}
