//! How this platform runs a script file.
//!
//! One answer, because there are two callers — an `exec:` declaration (XIII.3) and a hook on
//! one of LiNix's own events (XIII.13) — and a second copy would be a second chance for one of
//! them to be wrong on Windows, where the answer is not "make it executable and run it".
//!
//! Pure: it decides argv and nothing else. Running it belongs to the caller, which has the
//! executor.

/// The extension a script file must carry for [`interpreter_for`] to run it.
///
/// **Paired with the interpreter deliberately.** `powershell -File` refuses a file whose name
/// does not end in `.ps1` — so a caller that writes a script to a temporary file and picks the
/// interpreter from here must take the suffix from here too, or it gets a refusal about the
/// file extension on Windows and nothing at all on Linux.
pub const SCRIPT_SUFFIX: &str = if cfg!(windows) { ".ps1" } else { ".sh" };

/// The command that runs `path` on this platform, as `(program, args)`.
///
/// **Not the file itself.** A Unix kernel reads the shebang; Windows has no such mechanism, so
/// a `.ps1`-shaped script handed straight to `CreateProcess` fails with a message about the
/// file not being a valid application — which reads as "your script is broken" rather than
/// "this platform needs an interpreter named".
pub fn interpreter_for(path: &std::path::Path) -> (&'static str, Vec<String>) {
    let script = path.to_string_lossy().to_string();
    if cfg!(windows) {
        (
            "powershell",
            vec![
                // No profile: a user's `$PROFILE` is not part of the script they wrote, and a
                // hook that behaves differently on one machine because of it is undebuggable.
                "-NoProfile".to_string(),
                // The script is already gated by II.12's approval ledger, which is a stronger
                // check than the execution policy and the reason this is not a hole.
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-File".to_string(),
                script,
            ],
        )
    } else {
        ("sh", vec![script])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// The script is always the last argument, whatever the platform — the property both
    /// callers depend on, and the one a new platform arm could quietly break.
    #[test]
    fn the_script_is_the_last_argument() {
        let (_, args) = interpreter_for(Path::new("/tmp/hook"));
        assert_eq!(args.last().map(String::as_str), Some("/tmp/hook"));
    }

    #[test]
    fn the_platform_gets_its_own_interpreter() {
        let (program, args) = interpreter_for(Path::new("/tmp/hook"));
        if cfg!(windows) {
            assert_eq!(program, "powershell");
            assert!(args.iter().any(|a| a == "-NoProfile"), "{:?}", args);
            assert!(args.iter().any(|a| a == "-File"), "{:?}", args);
        } else {
            assert_eq!(program, "sh");
            assert_eq!(args.len(), 1);
        }
    }

    /// A path with a space is one argument, not two. It reaches the process as an argv
    /// element, so no quoting is added — adding some would put literal quotes in the path.
    #[test]
    fn a_path_with_a_space_stays_one_argument() {
        let (_, args) = interpreter_for(Path::new("/tmp/my hooks/on_drift"));
        assert_eq!(
            args.last().map(String::as_str),
            Some("/tmp/my hooks/on_drift")
        );
    }

    /// The suffix and the interpreter are one fact: `powershell -File` refuses a name that is
    /// not `.ps1`, so a caller taking one from here must take the other.
    #[test]
    fn the_suffix_is_what_this_platforms_interpreter_accepts() {
        assert!(SCRIPT_SUFFIX.starts_with('.'), "{}", SCRIPT_SUFFIX);
        if cfg!(windows) {
            assert_eq!(SCRIPT_SUFFIX, ".ps1");
        }
    }
}
