use crate::core::{Error, Result};

/// Facts about the host used to evaluate `when` conditionals, so a single shared repo can
/// serve a heterogeneous fleet (Linux + macOS + Windows). The model reads these when it
/// resolves `when` gates in `active`, profiles and modules (II.2).
#[derive(Debug, Clone)]
pub struct HostFacts {
    pub os: String,
    pub arch: String,
    pub host: String,
    /// "unix" or "windows" — NOT the distribution family. `when family == debian` therefore
    /// never matches, though the spec's examples use exactly that. See `distro_family()`.
    pub family: String,
}

impl HostFacts {
    /// Gather this machine's facts.
    pub fn current() -> Self {
        Self {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            host: crate::config::Config::get_hostname(),
            family: std::env::consts::FAMILY.to_string(),
        }
    }

    fn value_for(&self, key: &str) -> Option<&str> {
        match key {
            "os" => Some(&self.os),
            "arch" => Some(&self.arch),
            "host" | "hostname" => Some(&self.host),
            "family" => Some(&self.family),
            _ => None,
        }
    }
}

/// The distribution family — `debian`, `fedora`, `arch`, `suse`, … — read from
/// `/etc/os-release`, which is the only place that answers it.
///
/// This is NOT `HostFacts::family`, which is `std::env::consts::FAMILY` and answers
/// "unix or windows". The two are different questions and the names collide; see the note in
/// `HostFacts`.
pub fn distro_family() -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let text = std::fs::read_to_string("/etc/os-release").ok()?;
    parse_os_release_family(&text)
}

/// `ID_LIKE` names the family and `ID` names the distribution, so a derivative
/// (`linuxmint`, whose `ID_LIKE` is `ubuntu debian`) resolves to the family that decides
/// which artifact installs. `ID_LIKE` is checked first for exactly that reason.
fn parse_os_release_family(text: &str) -> Option<String> {
    let field = |key: &str| -> Option<String> {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(key)?.strip_prefix('='))
            .map(|v| v.trim().trim_matches('"').to_lowercase())
    };

    const FAMILIES: [&str; 6] = ["debian", "fedora", "rhel", "suse", "arch", "alpine"];
    let id_like = field("ID_LIKE").unwrap_or_default();
    for family in FAMILIES {
        if id_like.split_whitespace().any(|w| w == family) {
            return Some(family.to_string());
        }
    }

    let id = field("ID")?;
    // `ubuntu` has no `ID_LIKE` on some releases and is not itself in the family list.
    match id.as_str() {
        "ubuntu" | "debian" | "raspbian" => Some("debian".into()),
        "fedora" => Some("fedora".into()),
        "centos" | "rhel" | "rocky" | "almalinux" => Some("rhel".into()),
        "opensuse" | "sles" => Some("suse".into()),
        "arch" | "manjaro" | "endeavouros" => Some("arch".into()),
        "alpine" => Some("alpine".into()),
        other => Some(other.to_string()),
    }
}

/// Evaluate a `when` predicate against host facts. Supported forms (case-insensitive on the
/// value): `os == linux`, `arch != x86_64`, `host == laptop`, `os in [linux, macos]`.
/// Pure — unit tested.
pub fn eval_when(pred: &str, facts: &HostFacts) -> Result<bool> {
    let pred = pred.trim();

    // Membership form: `key in [a, b, c]`  (brackets optional)
    if let Some((key, rest)) = pred.split_once(" in ") {
        let key = key.trim();
        let actual = facts
            .value_for(key)
            .ok_or_else(|| Error::Config(format!("unknown `when` key '{}'", key)))?;
        let list = rest.trim().trim_start_matches('[').trim_end_matches(']');
        let hit = list
            .split(',')
            .map(|s| s.trim())
            .any(|v| v.eq_ignore_ascii_case(actual));
        return Ok(hit);
    }

    // Comparison form: `key == value` or `key != value`
    let (negate, sep) = if pred.contains("!=") {
        (true, "!=")
    } else if pred.contains("==") {
        (false, "==")
    } else {
        return Err(Error::Config(format!(
            "invalid `when` predicate '{}' (use `key == value`, `key != value`, or `key in [..]`)",
            pred
        )));
    };
    let (key, value) = pred
        .split_once(sep)
        .ok_or_else(|| Error::Config(format!("invalid `when` predicate '{}'", pred)))?;
    let key = key.trim();
    let value = value.trim();
    let actual = facts
        .value_for(key)
        .ok_or_else(|| Error::Config(format!("unknown `when` key '{}'", key)))?;
    let eq = actual.eq_ignore_ascii_case(value);
    Ok(eq != negate)
}

/// Split a removal target like `backend:name[@opts]` into `(Some(backend), bare_name)`
/// when the prefix names a real backend, or `(None, name)` otherwise. `@options` are
/// stripped from the name. `is_known_backend` decides whether a `prefix:` is a backend
/// (so package names that legitimately contain a colon aren't misread as `backend:name`).
///
/// This is the parsing `remove` must use to match how `install` reads its arguments —
/// passing the whole `backend:name` string to a backend's `info()`/`remove()` (which
/// expect the *bare* name) silently makes `remove backend:pkg` a no-op. It consults the
/// registry (unlike a blind `split_once(':')`), which is why it is not one of the parsers
/// C13 retired.
pub fn split_removal_target(
    input: &str,
    is_known_backend: impl Fn(&str) -> bool,
) -> (Option<String>, String) {
    let (backend, name_part) = match input.split_once(':') {
        Some((b, n)) if is_known_backend(b) => (Some(b.to_string()), n),
        _ => (None, input),
    };
    let bare = name_part.split('@').next().unwrap_or(name_part).to_string();
    (backend, bare)
}

#[cfg(test)]
mod conditional_tests {
    use super::*;

    fn facts() -> HostFacts {
        HostFacts {
            os: "linux".into(),
            arch: "x86_64".into(),
            host: "laptop".into(),
            family: "unix".into(),
        }
    }

    #[test]
    fn eval_equality_and_inequality() {
        let f = facts();
        assert!(eval_when("os == linux", &f).unwrap());
        assert!(!eval_when("os == macos", &f).unwrap());
        assert!(eval_when("os != windows", &f).unwrap());
        assert!(!eval_when("arch != x86_64", &f).unwrap());
        // case-insensitive value match
        assert!(eval_when("os == LINUX", &f).unwrap());
    }

    #[test]
    fn eval_membership() {
        let f = facts();
        assert!(eval_when("os in [linux, macos]", &f).unwrap());
        assert!(!eval_when("os in [windows, macos]", &f).unwrap());
        assert!(eval_when("host in [laptop, desktop]", &f).unwrap());
    }

    #[test]
    fn eval_rejects_unknown_key_and_bad_syntax() {
        let f = facts();
        assert!(eval_when("kernel == 6.1", &f).is_err());
        assert!(eval_when("os linux", &f).is_err());
    }
}

#[cfg(test)]
mod removal_target_tests {
    use super::split_removal_target;

    // A tiny fixed backend set for the tests.
    fn known(b: &str) -> bool {
        matches!(b, "apt" | "uv" | "npm" | "web" | "cargo")
    }

    #[test]
    fn backend_prefix_scopes_and_strips_to_bare_name() {
        assert_eq!(
            split_removal_target("uv:ruff", known),
            (Some("uv".to_string()), "ruff".to_string())
        );
        assert_eq!(
            split_removal_target("apt:tree", known),
            (Some("apt".to_string()), "tree".to_string())
        );
    }

    #[test]
    fn bare_name_has_no_backend() {
        assert_eq!(
            split_removal_target("ripgrep", known),
            (None, "ripgrep".to_string())
        );
    }

    #[test]
    fn options_are_stripped_from_the_name() {
        assert_eq!(
            split_removal_target("npm:typescript@version=5", known),
            (Some("npm".to_string()), "typescript".to_string())
        );
    }

    #[test]
    fn unknown_prefix_is_not_treated_as_a_backend() {
        // A colon in a name whose prefix isn't a real backend stays part of the name.
        assert_eq!(
            split_removal_target("some:weird-name", known),
            (None, "some:weird-name".to_string())
        );
    }

    #[test]
    fn web_url_name_keeps_its_scheme_colon() {
        // web:https://x -> backend web, name "https://x" (only the first colon is the split).
        assert_eq!(
            split_removal_target("web:https://example.com/a.tar.gz", known),
            (
                Some("web".to_string()),
                "https://example.com/a.tar.gz".to_string()
            )
        );
    }
}
