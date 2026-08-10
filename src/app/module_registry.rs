// src/app/module_registry.rs
//
// Remote module sharing: resolve a source reference (`github:user/repo`, a raw URL) into a
// concrete fetch URL + a suggested local name, so `linix module add <source>` can pull a
// community-maintained module into the local modules directory. The URL resolution is pure
// and unit-tested; the network fetch lives in the command handler.

use crate::core::{Error, Result};

/// Resolve a module source reference into `(raw_url, suggested_name)`.
///
/// Supported forms:
///   * `github:user/repo`                       → HEAD:`<repo>.module.txt`
///   * `github:user/repo/path/to/file.module.txt`
///   * `github:user/repo@v1.2/path/file.txt`    → pinned to a ref/branch/tag
///   * `https://.../something.module.txt`        → fetched verbatim
///
/// Pure — no I/O.
pub fn resolve_module_source(source: &str) -> Result<(String, String)> {
    let source = source.trim();

    if let Some(rest) = source.strip_prefix("github:") {
        let segs: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
        if segs.len() < 2 {
            return Err(Error::Config(format!(
                "github module source needs at least user/repo (got '{}')",
                source
            )));
        }
        let user = segs[0];
        // An optional `@ref` on the repo segment pins the branch/tag/commit; default HEAD.
        let (repo, reference) = match segs[1].split_once('@') {
            Some((r, rf)) if !rf.trim().is_empty() => (r, rf.trim()),
            _ => (segs[1], "HEAD"),
        };
        let (file_path, name) = if segs.len() > 2 {
            let fp = segs[2..].join("/");
            let base = *segs.last().unwrap();
            (fp, strip_module_ext(base))
        } else {
            (format!("{}.module.txt", repo), repo.to_string())
        };
        let url = format!(
            "https://raw.githubusercontent.com/{}/{}/{}/{}",
            user, repo, reference, file_path
        );
        return Ok((url, name));
    }

    if source.starts_with("http://") || source.starts_with("https://") {
        let base = source
            .rsplit('/')
            .find(|s| !s.is_empty())
            .unwrap_or("module");
        let mut name = strip_module_ext(base);
        if name.is_empty() {
            name = "module".to_string();
        }
        return Ok((source.to_string(), name));
    }

    Err(Error::Config(format!(
        "unsupported module source '{}' — use `github:user/repo` or an https URL",
        source
    )))
}

/// Strip a trailing `.module.txt` / `.txt` extension to derive a bare module name.
fn strip_module_ext(s: &str) -> String {
    s.strip_suffix(".module.txt")
        .or_else(|| s.strip_suffix(".txt"))
        .unwrap_or(s)
        .to_string()
}

/// Count the meaningful (non-blank, non-comment) entries in a fetched module body, for a
/// friendly summary after `module add`. Pure.
pub fn count_entries(body: &str) -> usize {
    crate::utils::file::filtered_lines(body).len()
}

/// Heuristic guard: a fetched module that starts with `<` is almost certainly an HTML error
/// page (404/login wall), not a manifest. Pure.
pub fn looks_like_html(body: &str) -> bool {
    body.trim_start().starts_with('<')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_user_repo_defaults_to_repo_module() {
        let (url, name) = resolve_module_source("github:acme/rust-dev").unwrap();
        assert_eq!(
            url,
            "https://raw.githubusercontent.com/acme/rust-dev/HEAD/rust-dev.module.txt"
        );
        assert_eq!(name, "rust-dev");
    }

    #[test]
    fn github_with_explicit_path_and_ref() {
        let (url, name) =
            resolve_module_source("github:acme/dotfiles@v2/modules/editors.module.txt").unwrap();
        assert_eq!(
            url,
            "https://raw.githubusercontent.com/acme/dotfiles/v2/modules/editors.module.txt"
        );
        assert_eq!(name, "editors");
    }

    #[test]
    fn raw_https_url_is_used_verbatim() {
        let (url, name) =
            resolve_module_source("https://example.com/share/gaming.module.txt").unwrap();
        assert_eq!(url, "https://example.com/share/gaming.module.txt");
        assert_eq!(name, "gaming");
    }

    #[test]
    fn rejects_bad_sources() {
        assert!(resolve_module_source("github:acme").is_err());
        assert!(resolve_module_source("ftp://nope").is_err());
        assert!(resolve_module_source("just-a-name").is_err());
    }

    #[test]
    fn entry_count_and_html_guard() {
        let body = "# header\n\napt:htop\ncargo:ripgrep\n# note\n";
        assert_eq!(count_entries(body), 2);
        assert!(looks_like_html("<!DOCTYPE html>"));
        assert!(!looks_like_html("apt:htop"));
    }
}
