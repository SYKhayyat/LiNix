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

/// The ceiling on one downloaded body, in bytes. 2 GiB.
///
/// **Sized to the largest thing anyone legitimately ships this way**, which is an AppImage: the
/// fattest in common use are a few hundred megabytes, so this is roughly eight times the real
/// worst case. It is not a security boundary — a hostile server that stays under it still gets a
/// file written — it is the bound that stops a redirect to something enormous, or a server that
/// never stops sending, from filling the disk while Shall reports progress.
pub const DEFAULT_MAX_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Seeded from `Config::max_download_bytes`; `0` removes the bound.
static MAX_DOWNLOAD_BYTES: once_cell::sync::OnceCell<u64> = once_cell::sync::OnceCell::new();

/// Set the process-wide download ceiling (called once during startup). Later calls no-op.
pub fn set_max_download_bytes(bytes: u64) {
    let _ = MAX_DOWNLOAD_BYTES.set(bytes);
}

fn max_download_bytes() -> Option<u64> {
    match *MAX_DOWNLOAD_BYTES
        .get()
        .unwrap_or(&DEFAULT_MAX_DOWNLOAD_BYTES)
    {
        0 => None,
        bytes => Some(bytes),
    }
}

/// Write a response body to `dest`, refusing one that is larger than the ceiling.
///
/// **Streamed, not buffered, and that is the larger half of the fix.** All three download
/// backends read the whole body into memory with `.bytes()` before writing it, so a URL that
/// answered with something enormous exhausted RAM before it ever touched the disk — and neither
/// the size nor the ceiling mattered, because there was no ceiling. Writing chunk by chunk
/// bounds the memory to one chunk whatever the server sends, and the counter bounds the disk.
///
/// **`Content-Length` is checked first and trusted for nothing.** When a server declares a size
/// over the ceiling the transfer is refused before a byte moves, which turns a two-gigabyte wait
/// into an immediate message; when it declares nothing, or lies, the running count catches it
/// anyway. One of those is a courtesy and the other is the actual bound.
pub async fn write_capped(
    response: reqwest::Response,
    dest: &std::path::Path,
    what: &str,
) -> Result<u64> {
    // **The check is where the write is** (`core::dry_run`). Every verb that can reach a
    // download backend's `install()` returns before it under `--dry-run` today, so this closes
    // nothing that is open — it moves the rule from five verbs remembering it to the one
    // function that creates the file, which is the argument that module makes for itself.
    if crate::core::dry_run::active() {
        crate::would!("download {}", what);
        return Ok(0);
    }
    write_capped_to(response, dest, what, max_download_bytes()).await
}

/// The body of [`write_capped`] with the ceiling passed in.
///
/// Split out so a test can name its own bound: the process-wide one is a `OnceCell` seeded at
/// startup, and a test that set it would decide the value for every other test in the binary.
async fn write_capped_to(
    response: reqwest::Response,
    dest: &std::path::Path,
    what: &str,
    cap: Option<u64>,
) -> Result<u64> {
    // Read from the header rather than `content_length()`: that method answers from the body's
    // size hint, which a streamed response does not have, so it reports `None` for exactly the
    // transfers the check below exists for. Shall's client enables no response decompression, so
    // the declared length is the length of what arrives.
    let declared = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .or_else(|| response.content_length());
    if let (Some(cap), Some(declared)) = (cap, declared) {
        if declared > cap {
            return Err(refused_size(what, declared, cap));
        }
    }

    // **One cleanup for every way out, not one for the way that was thought of.** A partial
    // artifact left on disk is one a later run can find and treat as complete, and the checksum
    // that would have caught that is the one `@unverified` is allowed to turn off. The cap
    // refusal removed it; the dropped connection, the full disk and the failed flush did not,
    // and `appimage:` streams straight onto the live artifact path — so a transfer that died at
    // 60% replaced a working binary on PATH with a truncated one. Wrapping the whole stream is
    // what stops a fifth exit inheriting the bug.
    match stream_capped(response, dest, what, cap, declared).await {
        Ok(written) => Ok(written),
        Err(e) => {
            let _ = tokio::fs::remove_file(dest).await;
            Err(e)
        }
    }
}

/// The streaming half of [`write_capped_to`], with no cleanup of its own.
///
/// Every exit here is an error exit its caller cleans up after, which is the only reason it can
/// be written straight through.
async fn stream_capped(
    response: reqwest::Response,
    dest: &std::path::Path,
    what: &str,
    cap: Option<u64>,
    declared: Option<u64>,
) -> Result<u64> {
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    let mut file = tokio::fs::File::create(dest).await.map_err(Error::from)?;
    let mut written: u64 = 0;
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(Error::from)?;
        written += chunk.len() as u64;
        if let Some(cap) = cap {
            if written > cap {
                return Err(refused_size(what, written, cap));
            }
        }
        file.write_all(&chunk).await.map_err(Error::from)?;
    }
    file.flush().await.map_err(Error::from)?;
    // **A body that stops early is not a body that ended.** Nothing else notices: the stream
    // reports no error, and the hash that would have caught it is the one `@unverified` turns
    // off and the one VIII.2 exempts every `github:` line from. When the server said how many
    // bytes were coming, the count is the check.
    if let Some(declared) = declared {
        if written != declared {
            return Err(Error::command_failed(format!(
                "{} ended after {} of the {} the server said it would send — the transfer was \
                 cut short, and what arrived is not the file.",
                what,
                human_bytes(written),
                human_bytes(declared)
            )));
        }
    }
    Ok(written)
}

/// Permanent: the same URL answers with the same size next time, so a retry spends the transfer
/// again to reach the same refusal.
fn refused_size(what: &str, size: u64, cap: u64) -> Error {
    Error::command_failed_permanently(format!(
        "{} is {} and the ceiling is {} — refused before it filled the disk. Raise \
         `max_download_bytes`, or set it to 0 to remove the bound.",
        what,
        human_bytes(size),
        human_bytes(cap)
    ))
}

/// Bytes as a person reads them. A refusal that says `2147483648` makes the reader do the
/// arithmetic before they can decide whether the number is wrong.
fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", n, UNITS[0])
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

/// Whether a bare flag is set on a spec. The grammar stores a bare `@flag` as `"true"`.
fn flag(spec: &PackageSpec, name: &str) -> bool {
    spec.options.one(name).is_some_and(|v| v == "true")
}

pub fn allows_http(spec: &PackageSpec) -> bool {
    flag(spec, "allow_http")
}

pub fn is_unverified(spec: &PackageSpec) -> bool {
    flag(spec, "unverified")
}

/// `@system=true` — this line may write into an environment the OS owns (`Q49`).
///
/// Here beside `is_unverified` because it is the same shape of thing: a per-line opt-in to a
/// refusal that exists for a good reason, read once so no caller re-derives it from the option
/// map. It is not about downloads; it is about the flag readers this module already owns.
pub fn is_system(spec: &PackageSpec) -> bool {
    flag(spec, "system")
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
    if spec.options.contains("sha256") || is_unverified(spec) {
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
/// A download carries no whole-request timeout: a release asset can legitimately take an hour,
/// and a bound sized for an API call turns a slow link into a corrupt install.
pub fn client(allow_http: bool, user_agent: &str) -> Result<reqwest::Client> {
    crate::core::http::client(user_agent, allow_http, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(opts: &[(&str, &str)]) -> PackageSpec {
        PackageSpec {
            name: "http://example.invalid/x".into(),
            backend: "web".into(),
            options: opts.iter().map(|(k, v)| (*k, *v)).collect(),
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

    /// A body with no `Content-Length`, which is the case the running counter exists for — a
    /// chunked response declares nothing, so a ceiling that only read the header would bound
    /// exactly the servers that are honest about their size.
    fn undeclared(body: &'static str) -> reqwest::Response {
        let stream = futures::stream::once(async move { Ok::<_, std::io::Error>(body.as_bytes()) });
        reqwest::Response::from(
            http::Response::builder()
                .body(reqwest::Body::wrap_stream(stream))
                .expect("a response with a streamed body"),
        )
    }

    #[tokio::test]
    async fn a_body_over_the_ceiling_is_refused_and_leaves_nothing_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("artifact");
        let err = write_capped_to(undeclared("0123456789"), &dest, "an artifact", Some(4))
            .await
            .expect_err("ten bytes under a four-byte ceiling");
        let message = err.to_string();
        assert!(message.contains("ceiling"), "{message}");
        assert!(
            !dest.exists(),
            "the partial file survived, and a later run would read it as a complete download"
        );
    }

    #[tokio::test]
    async fn a_body_under_the_ceiling_is_written_whole() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("artifact");
        let written = write_capped_to(undeclared("0123456789"), &dest, "an artifact", Some(1024))
            .await
            .expect("ten bytes under a kilobyte ceiling");
        assert_eq!(written, 10);
        assert_eq!(
            std::fs::read(&dest).expect("the file was written"),
            b"0123456789"
        );
    }

    /// `0` means no bound, and it has to mean that all the way down — a `Some(0)` would refuse
    /// every download instead of allowing every one.
    #[tokio::test]
    async fn no_ceiling_writes_anything() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("artifact");
        assert_eq!(
            write_capped_to(undeclared("0123456789"), &dest, "an artifact", None)
                .await
                .expect("no ceiling refuses nothing"),
            10
        );
    }

    /// A body that fails partway, with no `Content-Length` — the dropped-connection exit.
    fn breaks_after(prefix: &'static str) -> reqwest::Response {
        use futures::StreamExt;
        let stream = futures::stream::once(
            async move { Ok::<_, std::io::Error>(prefix.as_bytes()) },
        )
        .chain(futures::stream::once(async {
            Err::<&[u8], _>(std::io::Error::other("the connection went away"))
        }));
        reqwest::Response::from(
            http::Response::builder()
                .body(reqwest::Body::wrap_stream(stream))
                .expect("a response with a streamed body"),
        )
    }

    /// A body that declares more than it sends — the truncated-but-clean exit.
    fn declares(len: u64, body: &'static str) -> reqwest::Response {
        let stream = futures::stream::once(async move { Ok::<_, std::io::Error>(body.as_bytes()) });
        reqwest::Response::from(
            http::Response::builder()
                .header(http::header::CONTENT_LENGTH, len)
                .body(reqwest::Body::wrap_stream(stream))
                .expect("a response with a streamed body"),
        )
    }

    /// **Every error exit takes the partial file with it, not just the one that was thought of.**
    ///
    /// The cap refusal cleaned up and said why; the dropped connection, the failed write and the
    /// failed flush did not. `appimage:` streamed straight onto the live artifact path, so the
    /// leftover was a truncated binary under the PATH symlink of the last good install.
    #[tokio::test]
    async fn a_failed_transfer_leaves_nothing_behind() {
        let dir = tempfile::tempdir().expect("tempdir");

        let dropped = dir.path().join("dropped");
        write_capped_to(breaks_after("012345"), &dropped, "an artifact", None)
            .await
            .expect_err("a connection that goes away is not a download");
        assert!(
            !dropped.exists(),
            "a partial file survived a dropped connection"
        );

        let short = dir.path().join("short");
        let err = write_capped_to(declares(10, "012345"), &short, "an artifact", None)
            .await
            .expect_err("six bytes of a declared ten is not the file");
        assert!(err.to_string().contains("cut short"), "{err}");
        assert!(!short.exists(), "a truncated file survived a short body");
    }

    /// A declared length that is met is not a failure — the check must not refuse the ordinary
    /// case it was added to bound.
    #[tokio::test]
    async fn a_body_that_matches_its_declared_length_is_accepted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("artifact");
        assert_eq!(
            write_capped_to(declares(10, "0123456789"), &dest, "an artifact", Some(1024))
                .await
                .expect("a complete body"),
            10
        );
    }

    #[test]
    fn a_size_is_reported_the_way_a_person_reads_one() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2 * 1024 * 1024 * 1024), "2.0 GiB");
        assert_eq!(human_bytes(DEFAULT_MAX_DOWNLOAD_BYTES), "2.0 GiB");
    }
}
