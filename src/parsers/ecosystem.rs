// src/parsers/ecosystem.rs
//
// Output parsers for the "ecosystem" package managers added in the backend-expansion
// work (guix, eopkg, slackpkg, opam, luarocks, nimble, pixi, spack, mix, helm, asdf,
// emerge, cabal, krew, pub). Each parser takes the raw stdout plus the backend id (so a
// single implementation can be reused by several managers whose output shares a shape)
// and returns structured `Package`s. They are wired to backends via non-capturing
// closures in `backends/registry.rs`, e.g. `|o| ecosystem::ws_name_version(o, "guix")`.
//
// Kept deliberately lenient: package-manager output drifts across versions, so parsers
// skip blank lines, obvious table headers, and decorative rows rather than erroring.

use crate::core::Package;
use crate::parsers::utils::sanitize;

/// Header tokens that commonly lead a table's first column and must not be mistaken for a
/// package name.
fn is_header_token(tok: &str) -> bool {
    matches!(
        tok,
        "NAME" | "Name" | "PLUGIN" | "Plugin" | "Package" | "PACKAGE" | "Repository"
            | "Bucket" | "Source" | "Version" | "VERSION" | "Global"
    ) ||
    // All-caps alphabetic word of length >= 2 (e.g. "STATUS") — table headers are usually caps.
    (tok.len() >= 2 && tok.chars().all(|c| c.is_ascii_uppercase()))
}

/// True for a decorative / non-data line: empty, a tree connector, a dashed separator, or
/// a bracketed status banner.
fn is_noise_line(line: &str) -> bool {
    let t = line.trim();
    t.is_empty()
        || t.starts_with('#')
        || t.chars()
            .all(|c| matches!(c, '-' | '=' | '─' | '│' | '├' | '└' | ' '))
}

/// One package name per line, taking the first whitespace token. For managers whose list/
/// search prints bare identifiers (opam `--short`, spack `list`, pixi `search`, emerge
/// `qlist -I` atoms). Skips blank lines and table headers.
pub fn names_only(output: &str, backend: &str) -> Vec<Package> {
    let clean = sanitize(output);
    clean
        .lines()
        .filter(|l| !is_noise_line(l))
        .filter_map(|l| {
            let tok = l.split_whitespace().next()?;
            if is_header_token(tok) {
                return None;
            }
            Some(Package::new(tok, backend))
        })
        .collect()
}

/// `name version [extra…]` per line, whitespace- or tab-separated. Covers cabal
/// (`--simple-output`), spack (`find --format "{name} {version}"`), pub (`global list`),
/// krew (`list`), helm (`plugin list`), guix (`package -I`), luarocks (`--porcelain`).
/// The second column is treated as the version; any trailing columns are ignored.
pub fn ws_name_version(output: &str, backend: &str) -> Vec<Package> {
    let clean = sanitize(output);
    clean
        .lines()
        .filter(|l| !is_noise_line(l))
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let name = parts.next()?;
            if is_header_token(name) {
                return None;
            }
            match parts.next() {
                Some(ver) => Some(Package::with_version(name, ver, backend)),
                None => Some(Package::new(name, backend)),
            }
        })
        .collect()
}

/// eopkg `list-installed` / `search`: `name - Short description`. Take the field before
/// the first ` - ` (falling back to the first token).
pub fn eopkg_list(output: &str, backend: &str) -> Vec<Package> {
    let clean = sanitize(output);
    clean
        .lines()
        .filter(|l| !is_noise_line(l))
        .filter_map(|l| {
            let name = l.split(" - ").next().unwrap_or(l).trim();
            let name = name.split_whitespace().next().unwrap_or(name);
            if name.is_empty() || is_header_token(name) {
                return None;
            }
            Some(Package::new(name, backend))
        })
        .collect()
}

/// guix `search`: recutils output with `name: <pkg>` and `version: <ver>` fields, one
/// blank-line-separated record per package. Pair each `name:` with the following
/// `version:` in the same record.
pub fn guix_search(output: &str, backend: &str) -> Vec<Package> {
    let clean = sanitize(output);
    let mut out = Vec::new();
    let mut pending_name: Option<String> = None;
    for line in clean.lines() {
        if let Some(rest) = line.strip_prefix("name:") {
            pending_name = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("version:") {
            if let Some(name) = pending_name.take() {
                out.push(Package::with_version(&name, rest.trim(), backend));
            }
        } else if line.trim().is_empty() {
            // record boundary: a name with no version still counts.
            if let Some(name) = pending_name.take() {
                out.push(Package::new(name, backend));
            }
        }
    }
    if let Some(name) = pending_name.take() {
        out.push(Package::new(name, backend));
    }
    out
}

/// Strip a Slackware package filename (`name-version-arch-build`) down to its name. The
/// last three `-`-separated fields are version, arch and build; everything before them is
/// the (possibly hyphenated) name.
fn slack_pkgname(field: &str) -> &str {
    let parts: Vec<&str> = field.split('-').collect();
    if parts.len() >= 4 {
        // Rejoin all but the last three fields; find the byte index of the 3rd-from-last '-'.
        let keep = parts.len() - 3;
        let mut idx = 0;
        let mut seen = 0;
        for (i, c) in field.char_indices() {
            if c == '-' {
                seen += 1;
                if seen == keep {
                    idx = i;
                    break;
                }
            }
        }
        if idx > 0 {
            return &field[..idx];
        }
    }
    field
}

/// slackpkg installed list: output of `ls /var/log/packages`, one `name-ver-arch-build`
/// filename per line.
pub fn slackpkg_installed(output: &str, backend: &str) -> Vec<Package> {
    let clean = sanitize(output);
    clean
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(|l| Package::new(slack_pkgname(l), backend))
        .collect()
}

/// slackpkg `search`: rows like `[ installed ] - name-ver-arch-build`. Pull the package
/// field after `] - ` (or `- `) and strip it to a name.
pub fn slackpkg_search(output: &str, backend: &str) -> Vec<Package> {
    let clean = sanitize(output);
    clean
        .lines()
        .filter_map(|l| {
            let field = l.rsplit("- ").next()?.trim();
            if field.is_empty() || !field.contains('-') {
                return None;
            }
            Some(Package::new(slack_pkgname(field), backend))
        })
        .collect()
}

/// nimble `list --installed`: `  pkgname  [1.0.0, 0.9.0]`. Name is the first token; the
/// version is the first entry inside the brackets.
pub fn nimble_list(output: &str, backend: &str) -> Vec<Package> {
    let clean = sanitize(output);
    clean
        .lines()
        .filter(|l| !is_noise_line(l))
        .filter_map(|l| {
            let t = l.trim();
            let name = t.split_whitespace().next()?;
            if is_header_token(name) {
                return None;
            }
            if let (Some(open), Some(close)) = (t.find('['), t.find(']')) {
                if close > open + 1 {
                    let ver = t[open + 1..close].split(',').next().unwrap_or("").trim();
                    if !ver.is_empty() {
                        return Some(Package::with_version(name, ver, backend));
                    }
                }
            }
            Some(Package::new(name, backend))
        })
        .collect()
}

/// mix `archive`: lines like `* hex-2.0.6`. Strip the leading bullet, then split the
/// trailing `-version` off the archive name.
pub fn mix_archive(output: &str, backend: &str) -> Vec<Package> {
    let clean = sanitize(output);
    clean
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            let entry = t.strip_prefix("* ").or_else(|| t.strip_prefix('*'))?.trim();
            if entry.is_empty() {
                return None;
            }
            match entry.rsplit_once('-') {
                Some((name, ver))
                    if !name.is_empty()
                        && ver.chars().next().is_some_and(|c| c.is_ascii_digit()) =>
                {
                    Some(Package::with_version(name, ver, backend))
                }
                _ => Some(Package::new(entry, backend)),
            }
        })
        .collect()
}

/// asdf `list`: non-indented lines are plugin (tool) names; indented lines are installed
/// versions of the preceding plugin and are skipped.
pub fn asdf_list(output: &str, backend: &str) -> Vec<Package> {
    let clean = sanitize(output);
    clean
        .lines()
        .filter(|l| {
            !l.trim().is_empty() && !l.starts_with([' ', '\t']) && !l.trim_start().starts_with('*')
        })
        .filter_map(|l| {
            let name = l.trim();
            if is_header_token(name) {
                return None;
            }
            Some(Package::new(name, backend))
        })
        .collect()
}

/// emerge `--search`: package hits are `*  category/pkg` lines.
pub fn emerge_search(output: &str, backend: &str) -> Vec<Package> {
    let clean = sanitize(output);
    clean
        .lines()
        .filter_map(|l| {
            let t = l.trim_start();
            let atom = t.strip_prefix("* ")?.trim();
            if atom.is_empty() || !atom.contains('/') {
                return None;
            }
            Some(Package::new(atom, backend))
        })
        .collect()
}

/// pixi `global list`: tree rows like `├── python: 3.11.0` (older `- python 3.11.0`).
pub fn pixi_list(output: &str, backend: &str) -> Vec<Package> {
    let clean = sanitize(output);
    clean
        .lines()
        .filter_map(|l| {
            let t = l
                .trim()
                .trim_start_matches(|c: char| {
                    c.is_whitespace() || matches!(c, '├' | '└' | '│' | '─' | '-' | '*')
                })
                .trim();
            if t.is_empty() || is_noise_line(t) {
                return None;
            }
            if let Some((name, ver)) = t.split_once(':') {
                let name = name.trim();
                let ver = ver.trim();
                // A real package name is a single token; a multi-word left side is a banner
                // like "Global environments at /path:" and must be skipped.
                if name.is_empty() || name.contains(char::is_whitespace) || is_header_token(name) {
                    return None;
                }
                return if ver.is_empty() {
                    Some(Package::new(name, backend))
                } else {
                    Some(Package::with_version(name, ver, backend))
                };
            }
            let mut parts = t.split_whitespace();
            let name = parts.next()?;
            if is_header_token(name) {
                return None;
            }
            match parts.next() {
                Some(ver) => Some(Package::with_version(name, ver, backend)),
                None => Some(Package::new(name, backend)),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_name_version_parses_and_skips_header() {
        let out = "NAME       VERSION\nfoo        1.2.3\nbar        0.1.0   some-desc\n";
        let pkgs = ws_name_version(out, "helm");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "foo");
        assert_eq!(pkgs[0].version.as_deref(), Some("1.2.3"));
        assert_eq!(pkgs[1].name, "bar");
        assert_eq!(pkgs[0].backend, "helm");
    }

    #[test]
    fn names_only_skips_headers_and_noise() {
        let out = "Package\n----------\nripgrep\nfd\n\n";
        let pkgs = names_only(out, "spack");
        assert_eq!(
            pkgs.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["ripgrep", "fd"]
        );
    }

    #[test]
    fn eopkg_list_takes_name_before_dash() {
        let out = "nano - Small, friendly text editor\ngit - Distributed VCS\n";
        let pkgs = eopkg_list(out, "eopkg");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "nano");
        assert_eq!(pkgs[1].name, "git");
    }

    #[test]
    fn guix_search_pairs_name_and_version() {
        let out = "name: hello\nversion: 2.12\nsynopsis: Hello, GNU world\n\nname: emacs\nversion: 29.1\n";
        let pkgs = guix_search(out, "guix");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "hello");
        assert_eq!(pkgs[0].version.as_deref(), Some("2.12"));
        assert_eq!(pkgs[1].name, "emacs");
        assert_eq!(pkgs[1].version.as_deref(), Some("29.1"));
    }

    #[test]
    fn slackpkg_installed_strips_version_arch_build() {
        let out = "bash-5.1.016-x86_64-4\naaa_base-15.0-x86_64-3\nvim-9.0.2000-x86_64-1\n";
        let pkgs = slackpkg_installed(out, "slackpkg");
        assert_eq!(pkgs[0].name, "bash");
        assert_eq!(pkgs[1].name, "aaa_base");
        assert_eq!(pkgs[2].name, "vim");
    }

    #[test]
    fn slackpkg_search_extracts_pkg_field() {
        let out = "[ installed ] - mc-4.8.29-x86_64-1\n[uninstalled] - htop-3.2.1-x86_64-1\n";
        let pkgs = slackpkg_search(out, "slackpkg");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "mc");
        assert_eq!(pkgs[1].name, "htop");
    }

    #[test]
    fn nimble_list_reads_name_and_bracket_version() {
        let out = "  jester  [0.5.0]\n  nimx  [0.1.0, 0.2.0]\n";
        let pkgs = nimble_list(out, "nimble");
        assert_eq!(pkgs[0].name, "jester");
        assert_eq!(pkgs[0].version.as_deref(), Some("0.5.0"));
        assert_eq!(pkgs[1].name, "nimx");
        assert_eq!(pkgs[1].version.as_deref(), Some("0.1.0"));
    }

    #[test]
    fn mix_archive_splits_name_and_version() {
        let out = "* hex-2.0.6\n* phx_new-1.7.0\n";
        let pkgs = mix_archive(out, "mix");
        assert_eq!(pkgs[0].name, "hex");
        assert_eq!(pkgs[0].version.as_deref(), Some("2.0.6"));
        assert_eq!(pkgs[1].name, "phx_new");
        assert_eq!(pkgs[1].version.as_deref(), Some("1.7.0"));
    }

    #[test]
    fn asdf_list_keeps_plugins_skips_versions() {
        let out = "nodejs\n  18.0.0\n  20.0.0\npython\n  3.11.0\n";
        let pkgs = asdf_list(out, "asdf");
        assert_eq!(
            pkgs.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["nodejs", "python"]
        );
    }

    #[test]
    fn emerge_search_extracts_atoms() {
        let out = "Searching...\n[ Results for search key : vim ]\n\n*  app-editors/vim\n      Latest version available: 9.0\n*  app-editors/gvim\n";
        let pkgs = emerge_search(out, "emerge");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "app-editors/vim");
        assert_eq!(pkgs[1].name, "app-editors/gvim");
    }

    #[test]
    fn pixi_list_handles_tree_and_flat() {
        let tree =
            "Global environments at /root/.pixi/envs:\n├── python: 3.11.0\n└── ripgrep: 14.0.0\n";
        let pkgs = pixi_list(tree, "pixi");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "python");
        assert_eq!(pkgs[0].version.as_deref(), Some("3.11.0"));
        assert_eq!(pkgs[1].name, "ripgrep");
    }

    #[test]
    fn guix_list_via_ws_name_version() {
        // `guix package -I` is tab-separated name<TAB>version<TAB>outputs<TAB>path.
        let out = "hello\t2.12\tout\t/gnu/store/xxx\nemacs\t29.1\tout\t/gnu/store/yyy\n";
        let pkgs = ws_name_version(out, "guix");
        assert_eq!(pkgs[0].name, "hello");
        assert_eq!(pkgs[0].version.as_deref(), Some("2.12"));
        assert_eq!(pkgs[1].name, "emacs");
    }
}
