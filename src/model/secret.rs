//! Hardware-token secrets: what a missing token looks like, and what `watch` does with one
//! (T3, T4).
//!
//! An `@decrypt` line whose identity is a hardware token — an age plugin like `age-plugin-yubikey`
//! — needs a physical touch to decrypt. That is fine at a terminal and a trap everywhere else: a
//! `sync` in a script, or a `watch` tick at 3am, would sit forever on a prompt nobody will
//! answer.
//!
//! Two rules:
//! - **T3:** a decrypt that does not complete times out, and LiNix names the token and the
//!   identity file rather than passing the plugin's own prompt text through.
//! - **T4:** an unattended `watch` tick does not even try a touch-required line — it skips it and
//!   says so once, rather than blocking the whole reconcile.
//!
//! Pure: recognising a plugin identity, and the two messages. Reading the file and enforcing the
//! timeout are the caller's.

use std::path::Path;
use std::time::Duration;

/// How long a decrypt may run before LiNix concludes it is waiting on a prompt nobody will
/// answer (T3). Generous — a real hardware touch is a few seconds, and a slow disk read of a
/// software key is well under this — so the timeout only fires on a genuine hang.
pub const DECRYPT_TIMEOUT: Duration = Duration::from_secs(30);

/// The age plugin an identity file names, if it is a hardware/interactive one.
///
/// An age plugin identity is `AGE-PLUGIN-<NAME>-1…`; the plugin's binary is `age-plugin-<name>`
/// and the touch is its doing. A software identity (`AGE-SECRET-KEY-1…`) has no plugin and no
/// touch, so it returns `None` and is decrypted the ordinary way. Case-insensitive, because an
/// identity file is user-written.
pub fn plugin_of(identity_contents: &str) -> Option<String> {
    for line in identity_contents.lines() {
        let line = line.trim();
        // The marker can be the identity itself or a `#` comment age-keygen writes above it.
        let body = line.trim_start_matches('#').trim();
        let upper = body.to_ascii_uppercase();
        if let Some(rest) = upper.strip_prefix("AGE-PLUGIN-") {
            // `AGE-PLUGIN-YUBIKEY-1ABC…` → `yubikey`.
            let name: String = rest.chars().take_while(|c| *c != '-').collect();
            if !name.is_empty() {
                return Some(name.to_ascii_lowercase());
            }
        }
    }
    None
}

/// T3: what a timed-out decrypt says. Names the token and the identity file, not the plugin's
/// own text — a message LiNix owns and can be acted on.
pub fn token_timeout_message(source: &Path, identity: &Path, plugin: Option<&str>) -> String {
    let token = match plugin {
        Some(p) => format!("the `{}` hardware token", p),
        None => "the hardware token".to_string(),
    };
    format!(
        "decrypting {} timed out after {}s — {} did not respond.\n  \
         Its identity is {}. If this is a key that needs a physical touch, do it at a terminal, \
         or run this where the token is present.",
        source.display(),
        DECRYPT_TIMEOUT.as_secs(),
        token,
        identity.display()
    )
}

/// T4: what an unattended `watch` says when it skips a touch-required line. Said once.
pub fn watch_skip_message(source: &Path, plugin: &str) -> String {
    format!(
        "skipping the encrypted {} this tick — its `{}` identity needs a physical touch, and \
         this is an unattended `watch` run. It will apply on the next `sync` you run yourself.",
        source.display(),
        plugin
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plugin_identity_is_recognised_by_name() {
        assert_eq!(
            plugin_of("AGE-PLUGIN-YUBIKEY-1QVX…").as_deref(),
            Some("yubikey")
        );
        // age-keygen writes the recipient in a comment above the identity.
        assert_eq!(
            plugin_of("# public key: age1yubikey1…\nAGE-PLUGIN-YUBIKEY-1ABC\n").as_deref(),
            Some("yubikey")
        );
        // A different plugin.
        assert_eq!(
            plugin_of("age-plugin-tpm-1xyz").as_deref(),
            Some("tpm")
        );
    }

    /// A software key has no plugin and no touch, so it decrypts the ordinary way — the timeout
    /// and the watch-skip are only for hardware.
    #[test]
    fn a_software_identity_has_no_plugin() {
        assert_eq!(
            plugin_of("AGE-SECRET-KEY-1QVX9…"),
            None
        );
        assert_eq!(plugin_of(""), None);
        assert_eq!(plugin_of("# just a comment\n"), None);
    }

    #[test]
    fn the_timeout_message_names_the_token_and_the_identity() {
        let msg = token_timeout_message(
            Path::new("/cfg/api.age"),
            Path::new("/home/a/.age/yubikey.txt"),
            Some("yubikey"),
        );
        assert!(msg.contains("yubikey"), "{}", msg);
        assert!(msg.contains("/cfg/api.age"), "{}", msg);
        assert!(msg.contains("yubikey.txt"), "{}", msg);
        assert!(msg.contains("30s"), "{}", msg);
    }

    #[test]
    fn the_watch_skip_message_says_why_and_when_it_will_apply() {
        let msg = watch_skip_message(Path::new("/cfg/api.age"), "yubikey");
        assert!(msg.contains("skipping"), "{}", msg);
        assert!(msg.contains("physical touch"), "{}", msg);
        assert!(msg.contains("sync"), "{}", msg);
    }
}
