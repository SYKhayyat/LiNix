//! Which backends the artifact options are legal on.
//!
//! Deliberately a static table rather than a question put to the registry. Backend
//! *existence* is host-dependent — there is no `snap` on Windows — but "snap publishes
//! channels" is true everywhere, and a module shared across a fleet has to parse the same way
//! on every machine in it. Deriving this from what is installed here would make a file legal
//! on one box and a syntax error on the next.

/// Backends where one declared name resolves to several downloadable artifacts, so `formats`,
/// `asset` and `bin` are meaningful.
///
/// `web:` is absent on purpose: a `web:URL` spec names exactly one file, so there is nothing
/// to select. It joins this list if and when a `web:` spec can resolve to several candidates.
/// `appimage:` is absent because the backend name is already the format.
const SELECTS_ARTIFACTS: &[&str] = &["github"];

/// Backends that fetch a URL, make the result executable and put it on `PATH`, so SEC2's
/// download rules — HTTPS, a checksum, and the two flags that relax them — mean something.
///
/// Every other backend asks a package manager, which has its own signed index; `@allow_http`
/// or `@unverified` there would be a line that does nothing.
const DOWNLOADS: &[&str] = &["web", "appimage", "github"];

/// Backends that publish one artifact in several version streams.
const HAS_CHANNELS: &[&str] = &["snap", "flatpak"];

pub fn selects_artifacts(backend: &str) -> bool {
    SELECTS_ARTIFACTS.contains(&backend)
}

pub fn downloads(backend: &str) -> bool {
    DOWNLOADS.contains(&backend)
}

pub fn download_backends() -> String {
    DOWNLOADS.join(", ")
}

pub fn has_channels(backend: &str) -> bool {
    HAS_CHANNELS.contains(&backend)
}

pub fn artifact_backends() -> String {
    SELECTS_ARTIFACTS.join(", ")
}

pub fn channel_backends() -> String {
    HAS_CHANNELS.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_axes_do_not_overlap() {
        for b in SELECTS_ARTIFACTS {
            assert!(
                !has_channels(b),
                "{} would accept both formats and channel",
                b
            );
        }
    }

    #[test]
    fn a_backend_whose_ecosystem_chose_the_file_has_neither() {
        for b in ["apt", "dnf", "cargo", "npm", "pacman"] {
            assert!(!selects_artifacts(b));
            assert!(!has_channels(b));
        }
    }

    #[test]
    fn appimage_does_not_select_a_format_because_it_is_one() {
        assert!(!selects_artifacts("appimage"));
    }
}
