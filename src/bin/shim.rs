use std::env;
use std::process::{Command, exit};
use std::os::unix::process::CommandExt;

/// LiNix High-Performance Binary Shim (Phase 4.2)
/// 
/// This is a tiny, sub-millisecond Rust binary intended to be compiled 
/// and placed in ~/.local/bin. It replaces slower shell-script shims.
/// 
/// It performs:
/// 1. Zero-cost argument forwarding.
/// 2. Automatic profile/environment swapping.
/// 3. Transparent delegation to the 'linix run' orchestrator.
fn main() {
    // 1. Collect arguments passed to the shim
    let args: Vec<String> = env::args().collect();
    
    // 2. Identify the intended binary name (the name of this shim)
    let binary_name = env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "unknown".to_string());

    // 3. Construct the delegation command: linix run -p <binary_name> -- <binary_name> <args...>
    // We use 'exec' (on Unix) to replace the current process image with LiNix,
    // ensuring zero overhead for signal handling or process management.
    let mut cmd = Command::new("linix");
    
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
    // On Unix, this call does not return if successful.
    #[cfg(unix)]
    {
        let err = cmd.exec();
        eprintln!("LiNix Shim Error: Failed to execute 'linix run': {}", err);
        exit(1);
    }

    // Fallback for Windows (where exec() is not available)
    #[cfg(windows)]
    {
        match cmd.status() {
            Ok(status) => exit(status.code().unwrap_or(0)),
            Err(e) => {
                eprintln!("LiNix Shim Error: Failed to spawn child process: {}", e);
                exit(1);
            }
        }
    }
}