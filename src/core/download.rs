//! The rules a remote download obeys before anything it produced reaches your PATH (SEC2).
//!
//! `web:`, `appimage:` and `github:` all do the same three things: fetch a URL, mark the
//! result executable, and put it on `PATH`. That is a code-execution path with the network on
//! the other end of it, so **HTTPS and a checksum are the default and each relaxation is an
//! explicit, separate flag on the line that needs it**:
//!
//! - `@allow_http` — the URL may be `http://`.
//! - `@unverified` — no `@sha256` is required.
//!
//! **They never imply each other.** Allowing plain HTTP for a host that only serves HTTP must
//! not silently also drop the checksum: that combination is precisely the one where the
//! checksum is doing the most work, because anyone on the path can rewrite the response.
//!
//! *Why per-line and not a config key:* a global "require checksums" switch gets turned off
//! once, by the first person who meets a publisher that does not publish hashes, and never
//! gets turned back on — leaving a system that looks protected and is not. A flag on the line
//! has to be written for each spec that needs it, and it stays in the file where the next
//! reader sees it.

use crate::core::{Error, PackageSpec, Result};

/// Whether a bare flag is set on a spec. The grammar stores a bare `@flag` as `"true"`.
fn flag(spec: &PackageSpec, name: &str) -> bool {
    spec.options.get(name).is_some_and(|v| v == "true")
}

pub fn allows_http(spec: &PackageSpec) -> bool {
    flag(spec, "allow_http")
}

pub fn is_unverified(spec: &PackageSpec) -> bool {
    flag(spec, "unverified")
}

/// Refuse a URL that is not `https://`, unless this spec opted out.
///
/// Applied to **every URL actually fetched, not only the one that was typed**: reqwest follows
/// up to ten redirects, so an `https://` seed can be bounced to `http://` and the check on the
/// typed string would pass while the bytes arrive in clear.
pub fn check_scheme(url: &str, allow_http: bool, what: &str) -> Result<()> {
    if url.starts_with("https://") || allow_http {
        return Ok(());
    }
    Err(Error::Refused(format!(
        "refusing to download {} over plain HTTP: {}\n  \
         The file is made executable and put on your PATH, so anyone between you and that \
         host chooses what runs. Use `https://`, or add `@allow_http` to the line if the \
         publisher genuinely offers nothing else.",
        what, url
    )))
}

/// Refuse a download that carries no `@sha256`, unless this spec opted out.
///
/// **`github:` is exempt, and that is a ruling, not an omission** (owner, 2026-07-21). One
/// GitHub release ships a `.deb`, an `.rpm` and a tarball, so VIII.2 makes a hand-written
/// `@sha256` legal there only when the line pins exactly one format — requiring one would
/// force `@formats=` onto every github line, or push everyone to write `@unverified`, which
/// turns the flag into noise instead of a decision. github's integrity is `locks/github.toml`
/// instead: the hash of what was downloaded is recorded, and the same release arriving with
/// different bytes later is refused. The HTTPS half still applies to it, on every redirect
/// hop.
pub fn check_checksum_declared(spec: &PackageSpec) -> Result<()> {
    if spec.options.contains_key("sha256") || is_unverified(spec) {
        return Ok(());
    }
    Err(Error::Refused(format!(
        "refusing to install `{}` unverified: no `@sha256` on the line.\n  \
         The downloaded file is made executable and put on your PATH. Add `@sha256=<hash>`, \
         or `@unverified` to say you accept whatever the host serves. `@allow_http` does not \
         cover this — HTTP and no-checksum are separate decisions.",
        spec.name
    )))
}

/// A client whose redirect policy enforces the scheme on every hop.
///
/// The binding requirement is that the *final* download is HTTPS; checking each hop is the
/// cheapest correct form and also catches a downgrade in the middle of a chain that ends back
/// on HTTPS.
pub fn client(allow_http: bool, user_agent: &str) -> Result<reqwest::Client> {
    let policy = if allow_http {
        reqwest::redirect::Policy::default()
    } else {
        reqwest::redirect::Policy::custom(|attempt| {
            if attempt.url().scheme() != "https" {
                return attempt.error("redirected to a non-HTTPS URL");
            }
            if attempt.previous().len() >= 10 {
                return attempt.stop();
            }
            attempt.follow()
        })
    };
    reqwest::Client::builder()
        .user_agent(user_agent.to_string())
        .redirect(policy)
        .build()
        .map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn spec(opts: &[(&str, &str)]) -> PackageSpec {
        PackageSpec {
            name: "http://example.invalid/x".into(),
            backend: "web".into(),
            options: opts
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<HashMap<_, _>>(),
            requires: vec![],
            present: true,
        }
    }

    #[test]
    fn plain_http_is_refused_and_the_flag_is_what_allows_it() {
        assert!(check_scheme("http://x/y", false, "x").is_err());
        assert!(check_scheme("http://x/y", true, "x").is_ok());
        assert!(check_scheme("https://x/y", false, "x").is_ok());
    }

    #[test]
    fn a_download_with_no_checksum_is_refused() {
        assert!(check_checksum_declared(&spec(&[])).is_err());
        assert!(check_checksum_declared(&spec(&[("sha256", "abc")])).is_ok());
        assert!(check_checksum_declared(&spec(&[("unverified", "true")])).is_ok());
    }

    #[test]
    fn allowing_http_does_not_also_drop_the_checksum() {
        // The whole point of keeping them separate: over HTTP the checksum is the only thing
        // left, so the flag that permits HTTP must not be the flag that removes it.
        let s = spec(&[("allow_http", "true")]);
        assert!(allows_http(&s));
        assert!(check_checksum_declared(&s).is_err());
    }

    #[test]
    fn an_unset_flag_is_not_a_set_one() {
        assert!(!allows_http(&spec(&[])));
        assert!(!is_unverified(&spec(&[("unverified", "false")])));
    }
}
