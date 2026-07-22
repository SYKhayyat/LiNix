use crate::core::{Error, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use fs2::FileExt;
use std::collections::HashMap;
use std::fs::File;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Output as StdOutput};
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::info;

#[derive(Debug, Clone, Default)]
pub struct DryRunOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl DryRunOutput {
    pub fn new() -> Self {
        Self::default()
    }

    /// A run that exited non-zero and complained — what a manager with no package index
    /// does. `ExitStatus` cannot be constructed, so a real failing process supplies one.
    pub fn faulted(stderr: &str) -> StdOutput {
        let status = if cfg!(windows) {
            StdCommand::new("cmd").args(["/C", "exit", "1"]).status()
        } else {
            StdCommand::new("false").status()
        }
        .expect("failed to create dummy status");
        StdOutput {
            status,
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }
}

impl From<DryRunOutput> for StdOutput {
    fn from(dry: DryRunOutput) -> Self {
        let status = if cfg!(windows) {
            StdCommand::new("cmd")
                .args(["/C", "exit", "0"])
                .status()
                .expect("failed to create dummy status")
        } else {
            StdCommand::new("true")
                .status()
                .expect("failed to create dummy status")
        };
        StdOutput {
            status,
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
}

pub struct RawExecutor;

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
    if let Ok(resolved) = which::which(cmd) {
        if let Some(wrapped) = windows_shim_wrap(cmd, &resolved, args) {
            return wrapped;
        }
    }
    (cmd.to_string(), args.to_vec())
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

        if std::io::stdin().is_terminal() {
            command
                .stdin(std::process::Stdio::inherit())
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit());
        } else {
            command
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
        }

        let output = command
            .spawn()
            .map_err(|e| Error::CommandFailed(format!("Failed to spawn {}: {}", cmd, e)))?
            .wait_with_output()
            .await?;
        Ok(output)
    }

    fn check_command(&self, cmd: &str) -> bool {
        // Resolve via the `which` CRATE (in-process PATH/PATHEXT search) rather than
        // spawning the external `which`/`where` program — minimal fedora/arch/alpine
        // images don't ship `which`, which made every backend read as OFFLINE there
        // (breaking query/remove even though the manager was installed).
        which::which(cmd).is_ok()
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
        RawExecutor.check_command(cmd)
    }

    async fn symlink(&self, src: &Path, dst: &Path) -> Result<()> {
        let val = format!("LINK:{}", src.display());
        self.vfs.insert(dst.to_path_buf(), val);
        Ok(())
    }
}

pub struct MockExecutor {
    pub responses: DashMap<String, Result<StdOutput>>,
    pub command_existence: DashMap<String, bool>,
    pub call_log: Arc<Mutex<Vec<String>>>,
    vfs: Arc<DashMap<PathBuf, String>>,
}

impl MockExecutor {
    pub fn new(vfs: Arc<DashMap<PathBuf, String>>) -> Self {
        Self {
            responses: DashMap::new(),
            command_existence: DashMap::new(),
            call_log: Arc::new(Mutex::new(Vec::new())),
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
        _env: &HashMap<String, String>,
    ) -> Result<StdOutput> {
        let full_cmd = format!("{} {}", cmd, args.join(" "));
        {
            let mut log = self.call_log.lock().await;
            log.push(full_cmd.clone());
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
}

impl CommandExecutor {
    pub fn new(dry_run: bool, verbose: bool) -> Self {
        let vfs = Arc::new(DashMap::new());
        let lock_map = Arc::new(DashMap::new());
        let inner: Arc<dyn ExecutionLayer> = if dry_run {
            Arc::new(DryRunExecutor::new(vfs.clone()))
        } else {
            Arc::new(RawExecutor)
        };
        Self {
            dry_run,
            verbose,
            inner,
            reader: Arc::new(RawExecutor),
            vfs,
            lock_map,
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
        }
    }

    pub fn duplicate(&self) -> Self {
        self.clone()
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

        layer
            .execute(&final_cmd, &final_args, &HashMap::new())
            .await
    }

    /// Run a *mutating* command and enforce success. `RawExecutor::execute` hands back the
    /// process output regardless of exit status, so without this a failed `apt remove` /
    /// `npm install` / `btrfs subvolume delete` would be silently reported as OK and the
    /// caller would trust a mutation that never actually happened. Callers that legitimately
    /// tolerate a non-zero exit (searches, existence probes) must use `run_output`/`run_raw`.
    pub async fn run(&self, cmd: &str, args: &[&str], sudo: bool) -> Result<StdOutput> {
        let output = self.run_raw(cmd, args, sudo).await?;
        Self::ensure_status(cmd, output)
    }

    pub async fn run_output(&self, cmd: &str, args: &[&str], sudo: bool) -> Result<String> {
        // Reads tolerate a non-zero exit on purpose (empty results, missing packages),
        // so this goes through the unchecked primitive, never `run`.
        let output = self.read_raw(cmd, args, sudo).await?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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

        let lock_path = std::env::temp_dir().join(format!("linix_{}.lock", lock_key));
        let lock_file = File::create(lock_path).map_err(Error::from)?;
        lock_file.lock_exclusive().map_err(Error::from)?;
        let result = self.run_raw(cmd, args, sudo).await;
        let _ = lock_file.unlock();
        // Enforce status only after releasing the lock, so a failed mutation still frees it.
        Self::ensure_status(cmd, result?)
    }

    /// Classify a finished mutating command as success or failure. A non-zero exit is a
    /// failure EXCEPT for a few Windows managers that signal benign outcomes with non-zero
    /// codes (see `is_benign_exit`). Surfaces captured stderr (present when output is piped,
    /// i.e. non-interactive) so logs can explain what went wrong.
    fn ensure_status(cmd: &str, output: StdOutput) -> Result<StdOutput> {
        let status_ok = output.status.success() || Self::is_benign_exit(cmd, output.status.code());
        if status_ok && !Self::output_signals_failure(cmd, &output.stdout, &output.stderr) {
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
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = {
            let e = stderr.trim();
            if e.is_empty() {
                stdout.trim()
            } else {
                e
            }
        };
        let msg = if detail.is_empty() {
            format!("`{}` failed (exit {})", cmd, code)
        } else {
            format!("`{}` failed (exit {}): {}", cmd, code, detail)
        };
        Err(Error::CommandFailed(msg))
    }

    /// A few managers exit 0 even when they did NOTHING because the target could not be
    /// found — notably scoop: `scoop install <missing>` prints "Couldn't find manifest for
    /// 'x'." and still returns 0, so a bogus install would be silently trusted. Scan the
    /// captured output for such hard-failure markers and treat them as a real failure.
    /// Only consulted when output is piped (non-interactive); interactive runs surface the
    /// message to the user directly and are rare in automation.
    fn output_signals_failure(cmd: &str, stdout: &[u8], stderr: &[u8]) -> bool {
        // Normalize `\` to `/` before taking the stem: a Windows shim path like
        // `C:\…\scoop.ps1` only splits on `\` when running ON Windows, so on Linux/CI
        // `Path::file_stem` would keep the whole string and miss the `scoop` rule.
        let normalized = cmd.replace('\\', "/");
        let base = std::path::Path::new(&normalized)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(cmd)
            .to_ascii_lowercase();
        match base.as_str() {
            "scoop" => {
                let mut hay = String::from_utf8_lossy(stdout).into_owned();
                hay.push_str(&String::from_utf8_lossy(stderr));
                hay.make_ascii_lowercase();
                // "Couldn't find manifest for 'x'." — the tail is stable across scoop versions.
                hay.contains("find manifest for")
            }
            _ => false,
        }
    }

    /// Some Windows package managers report success — or benign no-ops — with non-zero exit
    /// codes; treat those as success so mutating ops don't spuriously fail. Every other
    /// non-zero exit is a real failure now that the write paths enforce status.
    fn is_benign_exit(cmd: &str, code: Option<i32>) -> bool {
        let code = match code {
            Some(c) => c,
            None => return false, // killed by a signal — never benign
        };
        // Match on the basename so path-qualified or sudo-wrapped invocations still resolve.
        // Normalize `\` to `/` first so a Windows shim path resolves on Linux/CI too.
        let normalized = cmd.replace('\\', "/");
        let base = std::path::Path::new(&normalized)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(cmd)
            .to_ascii_lowercase();
        match base.as_str() {
            // choco surfaces MSI conventions: 1641 reboot initiated, 3010 reboot required,
            // 1605/1614/1618 already-removed / uninstall-in-progress no-ops.
            "choco" | "chocolatey" => matches!(code, 1605 | 1614 | 1618 | 1641 | 3010),
            // winget HRESULT-style "success but noteworthy": no applicable upgrade,
            // already installed, no installed package found (benign on sweeps).
            "winget" => matches!(code, -1978335189 | -1978335212 | -1978335215),
            _ => false,
        }
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

    pub async fn start_sudo_keepalive(&self) -> Option<tokio::task::JoinHandle<()>> {
        if cfg!(windows) || Self::is_root() || self.dry_run {
            return None;
        }
        Some(tokio::spawn(async move {
            loop {
                let _ = StdCommand::new("sudo").arg("-v").status();
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        }))
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
mod search_read_tests {
    use super::{CommandExecutor, DryRunOutput, MockExecutor, StdOutput};
    use dashmap::DashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn exec(cmdline: &str, response: StdOutput) -> (CommandExecutor, Arc<MockExecutor>) {
        let vfs: Arc<DashMap<PathBuf, String>> = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        mock.set_response(cmdline, Ok(response));
        let e = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(DashMap::new()),
        );
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
    use super::CommandExecutor;

    #[test]
    fn scoop_missing_manifest_is_a_failure_despite_exit_zero() {
        // scoop prints this to stdout and STILL returns 0 — must be caught as a failure.
        let out = b"Couldn't find manifest for 'linix-nonexistent-pkg'.\n";
        assert!(CommandExecutor::output_signals_failure("scoop", out, b""));
        // path-qualified shim name still resolves to the scoop rule
        assert!(CommandExecutor::output_signals_failure(
            r"C:\Users\me\scoop\shims\scoop.ps1",
            out,
            b""
        ));
        // a normal scoop success must NOT be flagged
        assert!(!CommandExecutor::output_signals_failure(
            "scoop",
            b"'jq' (1.8.2) was installed successfully!\n",
            b""
        ));
        // other managers are unaffected by scoop's marker
        assert!(!CommandExecutor::output_signals_failure(
            "apt-get", out, b""
        ));
    }

    #[test]
    fn ordinary_nonzero_is_never_benign() {
        // apt/apk/dnf/… have no special codes: any non-zero exit is a real failure.
        assert!(!CommandExecutor::is_benign_exit("apk", Some(1)));
        assert!(!CommandExecutor::is_benign_exit("apt-get", Some(100)));
        assert!(!CommandExecutor::is_benign_exit("dnf", Some(1)));
    }

    #[test]
    fn choco_msi_reboot_codes_are_benign() {
        for code in [1605, 1614, 1618, 1641, 3010] {
            assert!(CommandExecutor::is_benign_exit("choco", Some(code)));
        }
        // …but a genuine choco failure is not.
        assert!(!CommandExecutor::is_benign_exit("choco", Some(1)));
    }

    #[test]
    fn winget_noteworthy_codes_are_benign() {
        assert!(CommandExecutor::is_benign_exit("winget", Some(-1978335189)));
        assert!(CommandExecutor::is_benign_exit("winget", Some(-1978335212)));
        assert!(!CommandExecutor::is_benign_exit("winget", Some(1)));
    }

    #[test]
    fn benign_codes_are_scoped_to_their_own_manager() {
        // 3010 is benign for choco, but must NOT leak to an unrelated manager.
        assert!(!CommandExecutor::is_benign_exit("apk", Some(3010)));
    }

    #[test]
    fn allowlist_resolves_path_qualified_and_exe_invocations() {
        // Forward slashes are recognized as separators on both Windows and Unix, so this
        // exercises basename + extension stripping portably (the suite may run on Linux CI).
        assert!(CommandExecutor::is_benign_exit(
            "/opt/chocolatey/bin/choco.exe",
            Some(3010)
        ));
    }

    #[test]
    fn signal_termination_is_never_benign() {
        // No exit code (killed by signal) is always a failure, even for winget/choco.
        assert!(!CommandExecutor::is_benign_exit("choco", None));
        assert!(!CommandExecutor::is_benign_exit("winget", None));
    }
}
