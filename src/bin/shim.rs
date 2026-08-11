use std::env;
use std::process::{exit, Command};

/// A PATH stand-in that forwards to a package Shall manages.
///
/// Deployed into ~/.local/bin for a line carrying `@shim=true`.
///
/// It performs:
/// 1. Zero-cost argument forwarding.
/// 2. Automatic profile/environment swapping.
/// 3. Transparent delegation to the 'shall run' orchestrator.
fn main() {
    // 1. Collect arguments passed to the shim
    let args: Vec<String> = env::args().collect();

    // 2. Identify the intended binary name (the name of this shim)
    let binary_name = env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "unknown".to_string());

    // 3. Construct the delegation command: shall run -p <binary_name> -- <binary_name> <args...>
    let mut cmd = Command::new("shall");

    cmd.arg("run")
        .arg("--packages")
        .arg(&binary_name)
        .arg("--")
        .arg(&binary_name);

    // Forward all arguments except the first one (which is the shim path itself)
    if args.len() > 1 {
        cmd.args(&args[1..]);
    }

    // 4. Execute the orchestrator

    #[cfg(unix)]
    {
        // On Unix, use exec() to replace the current process image with Shall,
        // ensuring zero overhead for signal handling or process management.
        use std::os::unix::process::CommandExt;
        let err = cmd.exec();
        eprintln!("Shall Shim Error: Failed to execute 'shall run': {}", err);
        exit(1);
    }

    #[cfg(windows)]
    {
        // Fallback for Windows (where exec() is not available)
        match cmd.status() {
            Ok(status) => exit(status.code().unwrap_or(0)),
            Err(e) => {
                eprintln!("Shall Shim Error: Failed to spawn child process: {}", e);
                exit(1);
            }
        }
    }
}
