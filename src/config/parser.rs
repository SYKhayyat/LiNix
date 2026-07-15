use crate::core::{Error, Result};
use crate::utils::file::atomic_write;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::debug;

/// Represents a line in a manifest file, identifying if it's a package or a module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestLine {
    /// A standard package specification (e.g., "apt:curl@version=1.0")
    Package(String),
    /// A reference to another reusable module (e.g., "@module:development")
    Module(String),
    /// A reference to a group of packages (e.g., "group:editors")
    Group(String),
    /// Inline another manifest file or a remote URL (e.g., "include: ./base.txt" or
    /// "include: https://…/team-base.txt"), splicing its lines in place.
    Include(String),
}

/// Facts about the host used to evaluate `when` conditionals in a manifest, so a single
/// shared manifest can serve a heterogeneous fleet (Linux + macOS + Windows).
#[derive(Debug, Clone)]
pub struct HostFacts {
    pub os: String,
    pub arch: String,
    pub host: String,
    /// "unix" or "windows".
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

/// Filter manifest lines by their enclosing `when` conditionals. Supports block form:
///
/// ```text
/// when os == macos
///   brew:coreutils
/// end
/// ```
///
/// and inline form: `when os == linux then apt:htop`. Blocks nest. Pure — unit tested.
pub fn filter_conditional_lines(lines: Vec<String>, facts: &HostFacts) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(lines.len());
    // Each open block records whether its own predicate held; a line is emitted only when
    // every ancestor block is active.
    let mut stack: Vec<bool> = Vec::new();
    let active = |stack: &[bool]| stack.iter().all(|&b| b);

    for line in lines {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("when ") {
            if let Some((pred, payload)) = rest.split_once(" then ") {
                // Inline guard — never opens a block.
                if active(&stack) && eval_when(pred, facts)? {
                    out.push(payload.trim().to_string());
                }
                continue;
            }
            let cond = eval_when(rest, facts)?;
            stack.push(cond);
            continue;
        }
        if l == "end" || l == "endwhen" {
            if stack.pop().is_none() {
                return Err(Error::Config(
                    "manifest has an `end` with no matching `when`".into(),
                ));
            }
            continue;
        }
        if active(&stack) {
            out.push(line);
        }
    }
    if !stack.is_empty() {
        return Err(Error::Config(
            "manifest has an unclosed `when` block (missing `end`)".into(),
        ));
    }
    Ok(out)
}

/// Parses a package group file (.txt) or module file (.module.txt) asynchronously.
/// Hardened for Version 3.6.0: Recognizes @module prefixes and recursive structures.
/// Evaluates `when os/arch/host` conditional blocks against the current host.
pub async fn parse_group_file(path: &Path) -> Result<Vec<String>> {
    if !tokio::fs::try_exists(path).await.unwrap_or(false) {
        debug!("Manifest parser: File not found at {:?}", path);
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(path)
        .await
        .map_err(|e| Error::Io(format!("Failed to read manifest {:?}: {}", path, e)))?;

    parse_group_str(&content)
}

/// Parse manifest text (already in memory) the same way [`parse_group_file`] parses a file:
/// strip a UTF-8 BOM, drop blank/comment lines, then apply host `when` conditionals. Used for
/// remote `include:` targets fetched over HTTP.
pub fn parse_group_str(content: &str) -> Result<Vec<String>> {
    // Strip a leading UTF-8 BOM so a manifest saved by a Windows editor doesn't turn the
    // first entry's backend into "\u{feff}cargo".
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);

    let lines: Vec<String> = content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.to_string())
        .collect();

    // Resolve `when`/`end` conditionals for this host before returning package lines.
    filter_conditional_lines(lines, &HostFacts::current())
}

/// Helper to categorize a raw manifest line.
pub fn identify_line(line: &str) -> ManifestLine {
    if let Some(module_name) = line.strip_prefix("@module:") {
        ManifestLine::Module(module_name.trim().to_string())
    } else if let Some(group_name) = line.strip_prefix("group:") {
        ManifestLine::Group(group_name.trim().to_string())
    } else if let Some(target) = line
        .strip_prefix("include:")
        .or_else(|| line.strip_prefix("include "))
    {
        ManifestLine::Include(target.trim().to_string())
    } else {
        ManifestLine::Package(line.trim().to_string())
    }
}

/// Split a removal target like `backend:name[@opts]` into `(Some(backend), bare_name)`
/// when the prefix names a real backend, or `(None, name)` otherwise. `@options` are
/// stripped from the name. `is_known_backend` decides whether a `prefix:` is a backend
/// (so package names that legitimately contain a colon aren't misread as `backend:name`).
///
/// This is the parsing `remove` must use to match how `install` reads its arguments —
/// passing the whole `backend:name` string to a backend's `info()`/`remove()` (which
/// expect the *bare* name) silently makes `remove backend:pkg` a no-op.
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

/// Adds a package string to the local declarative configuration.
/// Logic is async-safe and prevents duplicates.
pub async fn add_package_to_local(groups_dir: &Path, package_str: &str) -> Result<()> {
    let local_file = groups_dir.join("local.txt");

    if !tokio::fs::try_exists(groups_dir).await.unwrap_or(false) {
        fs::create_dir_all(groups_dir).await.map_err(Error::from)?;
    }

    let mut lines: Vec<String> = if tokio::fs::try_exists(&local_file).await.unwrap_or(false) {
        fs::read_to_string(&local_file)
            .await
            .map_err(Error::from)?
            .lines()
            .map(|s| s.to_string())
            .collect()
    } else {
        vec![
            "# LiNix Local Manifest".to_string(),
            "# This file is automatically updated by imperative commands.".to_string(),
            "".to_string(),
        ]
    };

    let is_duplicate = lines.iter().any(|l| {
        let clean = l.trim();
        clean == package_str
    });

    if !is_duplicate {
        lines.push(package_str.to_string());
        let new_content = lines.join("\n") + "\n";

        let path_owned = local_file.clone();
        tokio::task::spawn_blocking(move || atomic_write(&path_owned, &new_content))
            .await
            .map_err(|e| Error::Other(e.to_string()))??;

        debug!("Manifest parser: Added '{}' to local.txt", package_str);
    }

    Ok(())
}

/// Removes a package string from the local declarative configuration.
/// Also handles identifying matches with or without backend/version tags.
pub async fn remove_package_from_local(groups_dir: &Path, package_name: &str) -> Result<()> {
    let local_file = groups_dir.join("local.txt");
    if !tokio::fs::try_exists(&local_file).await.unwrap_or(false) {
        return Ok(());
    }

    let content = fs::read_to_string(&local_file).await.map_err(Error::from)?;
    let mut lines: Vec<String> = Vec::new();
    let mut found = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            lines.push(line.to_string());
            continue;
        }

        let is_match = if let Some((b, n)) = trimmed.split_once(':') {
            let name_only = n.split('@').next().unwrap_or(n).trim();
            trimmed == package_name || name_only == package_name || b == package_name
        } else {
            let name_only = trimmed.split('@').next().unwrap_or(trimmed).trim();
            trimmed == package_name || name_only == package_name
        };

        if is_match {
            found = true;
            debug!("Manifest parser: Removing '{}' from local.txt", trimmed);
            continue;
        }
        lines.push(line.to_string());
    }

    if found {
        let new_content = lines.join("\n") + "\n";
        let path_owned = local_file.clone();
        tokio::task::spawn_blocking(move || atomic_write(&path_owned, &new_content))
            .await
            .map_err(|e| Error::Other(e.to_string()))??;
    }

    Ok(())
}

/// Returns the path to the user's primary local manifest.
pub async fn get_user_group_file(groups_dir: &Path) -> PathBuf {
    if !tokio::fs::try_exists(groups_dir).await.unwrap_or(false) {
        let _ = fs::create_dir_all(groups_dir).await;
    }
    groups_dir.join("local.txt")
}

/// Writes a list of packages to a manifest file atomically.
pub async fn write_group_file(path: &Path, packages: &[String]) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !tokio::fs::try_exists(parent).await.unwrap_or(false) {
            fs::create_dir_all(parent).await.map_err(Error::from)?;
        }
    }
    let content = packages.join("\n") + "\n";
    let path_owned = path.to_path_buf();
    tokio::task::spawn_blocking(move || atomic_write(&path_owned, &content))
        .await
        .map_err(|e| Error::Other(e.to_string()))??;
    Ok(())
}

/// Scans the groups directory and identifies all packages requested by the user.
/// Correctly handles hostname-specific manifests (host-NAME.txt).
pub async fn load_all_packages(groups_dir: &Path) -> Result<HashSet<String>> {
    let mut all_packages = HashSet::new();
    if !tokio::fs::try_exists(groups_dir).await.unwrap_or(false) {
        return Ok(all_packages);
    }

    let current_hostname = crate::config::Config::get_hostname();
    let host_file_name = format!("host-{}.txt", current_hostname);

    let groups_dir_owned = groups_dir.to_path_buf();
    let entries: Vec<PathBuf> = tokio::task::spawn_blocking(move || {
        walkdir::WalkDir::new(groups_dir_owned)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .map(|e| e.path().to_path_buf())
            .collect()
    })
    .await
    .map_err(|e| Error::Other(e.to_string()))?;

    for path in entries {
        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");

        // Skip files that are host-specific but not for THIS host
        if file_name.starts_with("host-") && file_name != host_file_name {
            continue;
        }

        if file_name.ends_with(".txt") {
            let packages = parse_group_file(&path).await?;
            all_packages.extend(packages);
        }
    }
    Ok(all_packages)
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
    fn identify_line_recognizes_include_and_others() {
        assert_eq!(
            identify_line("include: ./base.txt"),
            ManifestLine::Include("./base.txt".into())
        );
        assert_eq!(
            identify_line("include https://x/y.txt"),
            ManifestLine::Include("https://x/y.txt".into())
        );
        assert_eq!(
            identify_line("@module:dev"),
            ManifestLine::Module("dev".into())
        );
        assert_eq!(
            identify_line("group:editors"),
            ManifestLine::Group("editors".into())
        );
        assert_eq!(
            identify_line("apt:curl"),
            ManifestLine::Package("apt:curl".into())
        );
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

    #[test]
    fn block_filtering_keeps_matching_and_drops_others() {
        let f = facts();
        let lines = vec![
            "apt:always".to_string(),
            "when os == linux".to_string(),
            "apt:htop".to_string(),
            "end".to_string(),
            "when os == macos".to_string(),
            "brew:coreutils".to_string(),
            "end".to_string(),
        ];
        let out = filter_conditional_lines(lines, &f).unwrap();
        assert_eq!(out, vec!["apt:always", "apt:htop"]);
    }

    #[test]
    fn nested_blocks_require_all_ancestors() {
        let f = facts();
        let lines = vec![
            "when os == linux".to_string(),
            "when arch == arm64".to_string(),
            "apt:only-arm".to_string(),
            "end".to_string(),
            "apt:any-linux".to_string(),
            "end".to_string(),
        ];
        let out = filter_conditional_lines(lines, &f).unwrap();
        assert_eq!(out, vec!["apt:any-linux"]);
    }

    #[test]
    fn inline_then_guard() {
        let f = facts();
        let lines = vec![
            "when os == linux then apt:htop".to_string(),
            "when os == windows then winget:foo".to_string(),
        ];
        let out = filter_conditional_lines(lines, &f).unwrap();
        assert_eq!(out, vec!["apt:htop"]);
    }

    #[test]
    fn unbalanced_blocks_error() {
        let f = facts();
        assert!(filter_conditional_lines(vec!["when os == linux".into()], &f).is_err());
        assert!(filter_conditional_lines(vec!["end".into()], &f).is_err());
    }

    #[tokio::test]
    async fn parse_group_file_strips_utf8_bom() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("local.txt");
        // A manifest saved by a Windows editor with a UTF-8 BOM on the first line.
        tokio::fs::write(&path, "\u{feff}cargo:ripgrep\napt:htop\n")
            .await
            .unwrap();
        let lines = parse_group_file(&path).await.unwrap();
        assert_eq!(lines, vec!["cargo:ripgrep", "apt:htop"]);
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
