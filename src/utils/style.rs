// Color is only emitted to a real terminal and is
// suppressed when the standard `NO_COLOR` env var is set, so piped/redirected output stays
// clean. The `paint` core is pure so it can be unit-tested without a TTY.

use std::io::IsTerminal;

pub const GREEN: &str = "32";
pub const YELLOW: &str = "33";
pub const RED: &str = "31";
pub const BOLD: &str = "1";
/// For a fact that is not a verdict — a manager the user simply has not installed. Green,
/// yellow and red are all things to act on, and there are twenty-three of these on a healthy
/// Windows box.
pub const DIM: &str = "2";

/// Whether this environment permits colour at all, before asking about any one stream.
///
/// Two conventions, and only one of them was honoured. `NO_COLOR` was checked; `TERM=dumb` was
/// not, so a terminal that has told every other tool on the machine it cannot render escape
/// sequences got them from Shall anyway. On Windows there is no `TERM` and its absence means
/// nothing, so an unset variable is not read as "dumb".
fn color_allowed() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    !matches!(std::env::var("TERM").as_deref(), Ok("dumb") | Ok(""))
}

/// Whether colored output should be produced right now: stdout is a TTY and the environment
/// permits colour.
pub fn color_enabled() -> bool {
    color_allowed() && std::io::stdout().is_terminal()
}

/// The same question for the diagnostic stream.
///
/// **Two streams, two answers, and one of them was never asked.** The tracing subscriber writes
/// to stderr and was built with no `.with_ansi(…)` at all, so `tracing-subscriber`'s own default
/// — colour on, always — decided it: `shall install nosuchpkg 2>&1 | grep` came back carrying
/// escape codes, and a run redirected into a log file wrote them to disk. `color_enabled` was
/// the right answer to the wrong stream, and stdout being a pipe while stderr is a terminal is
/// the *usual* arrangement rather than an odd one.
pub fn color_enabled_on_stderr() -> bool {
    color_allowed() && std::io::stderr().is_terminal()
}

pub fn paint(enabled: bool, code: &str, text: &str) -> String {
    if enabled {
        format!("\u{1b}[{}m{}\u{1b}[0m", code, text)
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_wraps_only_when_enabled() {
        assert_eq!(paint(false, GREEN, "OK"), "OK");
        assert_eq!(paint(true, GREEN, "OK"), "\u{1b}[32mOK\u{1b}[0m");
    }

    #[test]
    fn paint_is_idempotent_when_disabled() {
        // Disabled painting never alters the string, so it's safe to always route through it.
        let s = "hello world";
        assert_eq!(paint(false, RED, s), s);
    }
}
