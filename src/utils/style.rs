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

/// Whether colored output should be produced right now: stdout is a TTY and `NO_COLOR` is unset.
pub fn color_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
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
