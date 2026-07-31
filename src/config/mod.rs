#[allow(clippy::module_inception)]
pub mod config;
pub mod grammar;
pub mod parser;
pub mod settings;

pub use config::{Config, GuardSettings, VarsSettings, PREFERENCES_FILE_NAME};
pub use settings::{resolve_root, ResolvedRoot, RootSource, Settings};

/// A config file's text without the byte-order mark a Windows editor puts in front of it (Q22).
///
/// Notepad writes UTF-8 **with** a BOM by default, and so does PowerShell 5.1's `Set-Content
/// -Encoding utf8`. The three bytes are an encoding artefact, not content, and no editor shows
/// them — so before this they became part of the first name on the first line, and the refusal
/// named two strings that render identically (`` `cargo` is not a backend LiNix uses — add
/// `cargo` to your `priority` file ``).
///
/// **Only the mark, and only at the start.** A U+FEFF anywhere else is a zero-width character
/// nothing but a paste puts there, and the validator still refuses it by name. Stripping every
/// occurrence would be the silent-repair habit this codebase is a reaction to.
///
/// Applied where text enters a *parser* rather than where a file is read: `model/edit.rs` reads
/// the same files in order to rewrite them, and II.16 says LiNix must not rewrite your files —
/// including their encoding.
pub fn without_bom(body: &str) -> &str {
    body.strip_prefix('\u{feff}').unwrap_or(body)
}

/// Module-level constants for configuration defaults
pub const DEFAULT_BACKEND: &str = "apt";
pub const CONFIG_DIR: &str = ".config/linix";
