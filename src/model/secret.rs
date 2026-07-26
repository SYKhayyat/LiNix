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

use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

/// A declared secret-decryption provider (U38): sops, Vault, 1Password, a KMS, plain GPG — any
/// command that turns a reference into plaintext. `age` and `sops` stay built in (age carries the
/// hardware-token handling above); this opens the door to the rest as data, the same "rows, not
/// Rust" move the other provider surfaces made.
///
/// **This is the one surface where openness is not cheap, and its rule is the strictest.** A
/// decrypt provider's output *is* a secret, so a provider is bound by the T-series plaintext
/// rules: the plaintext must come out on **stdout** and nowhere else — never a file it writes,
/// never a line it logs — so LiNix captures it in memory and restricts the destination before it
/// is written (T5), never backs it up (T1), and never lets it reach the repo (T2). A provider
/// that cannot promise stdout-only is **refused, not trusted** — and the promise is explicit:
/// `stdout_only = true` must be in the block, or the provider does not load. The unsafe reading
/// is never the default (V.80).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SecretProvider {
    pub name: String,
    /// The argv that decrypts. `{ref}` is the reference from the line (the source path or a
    /// secret id); `{identity}` is the `@identity=` value if one was given. Plaintext on stdout.
    pub decrypt: Vec<String>,
    /// The T-series promise, and it is required to be `true`: the provider writes the plaintext
    /// to stdout only. A block that omits it (default `false`) is refused — LiNix will not hand a
    /// secret to a command that has not promised to keep it off disk and out of the logs.
    #[serde(default)]
    pub stdout_only: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SecretProviderFile {
    #[serde(default)]
    pub secret: Vec<SecretProvider>,
}

impl SecretProvider {
    /// A provider LiNix will trust with a secret, or why it will not.
    pub fn is_usable(&self) -> Option<&'static str> {
        if self.name.trim().is_empty() {
            return Some("it has no `name`");
        }
        if self.decrypt.iter().all(|a| a.trim().is_empty()) {
            return Some("it has no `decrypt` command");
        }
        if !self.stdout_only {
            return Some(
                "it does not declare `stdout_only = true` — a secret provider must promise the \
                 plaintext reaches stdout only (never a file, never a log), or LiNix will not \
                 hand it a secret (T-series)",
            );
        }
        None
    }

    /// The `(program, args)` to run, with `{ref}` and `{identity}` filled in. Returns `None` when
    /// the block has no program (already rejected by `is_usable`, but never unwrapped).
    pub fn command(&self, reference: &str, identity: Option<&str>) -> Option<(String, Vec<String>)> {
        let fill = |a: &String| {
            a.replace("{ref}", reference)
                .replace("{identity}", identity.unwrap_or(""))
        };
        let (program, rest) = self.decrypt.split_first()?;
        Some((fill(program), rest.iter().map(fill).collect()))
    }
}

/// Every secret provider this machine knows, from `adapters/secret.toml` — the user's rows,
/// filtered to the usable ones (an unusable row is dropped with a reason, never half-trusted).
pub fn providers(rows: Vec<SecretProvider>) -> Vec<SecretProvider> {
    rows.into_iter()
        .filter(|p| match p.is_usable() {
            Some(why) => {
                tracing::warn!("ignoring the `{}` secret provider: {}.", p.name, why);
                false
            }
            None => true,
        })
        .collect()
}

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

    // --- U38: declared secret providers ---

    fn provider(name: &str, decrypt: &[&str], stdout_only: bool) -> SecretProvider {
        SecretProvider {
            name: name.into(),
            decrypt: decrypt.iter().map(|s| s.to_string()).collect(),
            stdout_only,
        }
    }

    #[test]
    fn a_secret_provider_must_promise_stdout_only() {
        // The T-series rule as a load-time gate: a provider that has not declared stdout_only is
        // refused, not trusted — the unsafe reading is never the default (V.80).
        let no_promise = provider("vault", &["vault", "kv", "get", "{ref}"], false);
        assert!(no_promise.is_usable().unwrap().contains("stdout_only"));
        // With the promise, and a command, it loads.
        assert!(provider("vault", &["vault", "kv", "get", "{ref}"], true)
            .is_usable()
            .is_none());
    }

    #[test]
    fn a_provider_with_no_name_or_command_is_refused() {
        assert!(provider("", &["x"], true).is_usable().is_some());
        assert!(provider("v", &[], true).is_usable().is_some());
    }

    #[test]
    fn providers_drops_the_unusable_and_keeps_the_rest() {
        let rows = vec![
            provider("good", &["sops", "-d", "{ref}"], true),
            provider("leaky", &["writes-a-file", "{ref}"], false), // no stdout_only → dropped
        ];
        let kept = providers(rows);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].name, "good");
    }

    #[test]
    fn a_provider_command_fills_the_reference_and_identity() {
        let p = provider("vault", &["vault", "read", "-field=v", "{ref}"], true);
        let (prog, args) = p.command("secret/api", None).unwrap();
        assert_eq!(prog, "vault");
        assert_eq!(args, vec!["read", "-field=v", "secret/api"]);

        let gpg = provider("gpg", &["gpg", "--decrypt", "--recipient", "{identity}", "{ref}"], true);
        let (_, args) = gpg.command("api.gpg", Some("me@example.com")).unwrap();
        assert!(args.contains(&"me@example.com".to_string()), "{:?}", args);
        assert!(args.contains(&"api.gpg".to_string()), "{:?}", args);
    }

    #[test]
    fn the_provider_schema_parses() {
        let toml = r#"
[[secret]]
name = "onepassword"
decrypt = ["op", "read", "{ref}"]
stdout_only = true
"#;
        let file: SecretProviderFile = toml::from_str(toml).unwrap();
        assert_eq!(file.secret.len(), 1);
        assert!(file.secret[0].is_usable().is_none());
    }
}
