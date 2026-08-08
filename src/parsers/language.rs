use crate::core::Package;
use crate::parsers::{or_unrecognised, ParseResult, Unrecognised};
use crate::utils::text::sanitize;
use serde_json::Value;

/// The nine language managers' installed listings, dispatched by name.
///
/// **The judgement is made once, here, rather than nine times below.** Each inner reader is a
/// `Vec`-returning parse of one manager's shape, and every one of them has the same way of
/// failing — a `serde_json` call ending in `unwrap_or_default()`, or a tree walk that matches
/// nothing — which spells *"I did not understand this"* as *"nothing is installed"*. Wrapping
/// the dispatch catches all nine at once and needs no change inside any of them.
///
/// It also closes the tenth, which was the widest: `_ => vec![]` answered *the machine is
/// empty* for a backend nobody had written a reader for.
pub fn parse_installed(backend: &str, output: &str) -> ParseResult {
    let clean = sanitize(output);
    if clean.is_empty() {
        return Ok(vec![]);
    }

    let found = match backend {
        "npm" | "pnpm" => parse_npm_style_json(&clean, backend),
        // `bun pm ls -g` prints an ASCII tree (a header path line + "├── name@ver"
        // rows), NOT npm's `--json` object — routing it through the JSON parser
        // silently returned nothing, which broke list AND made `info`/`remove`
        // no-ops (remove is gated on `info`), leaving stale manifest entries.
        "bun" => parse_bun_list(&clean),
        "pip" => parse_pip_json(&clean),
        "pipx" => parse_pipx_json(&clean),
        "cargo" => parse_cargo_list(&clean),
        // yarn brackets every command with its own chrome — `yarn global v1.22.22` on the way in
        // and `Done in 0.67s.` on the way out — so an EMPTY global install still prints two
        // lines. Neither is prose by the general rule (no trailing colon, no leading digit, and
        // "Done in 0.67s." is a three-word sentence), so the unread check counted them as rows it
        // had failed to read and `linix list` warned about yarn on every run of a machine with no
        // global packages. Measured on a real Windows box, 2026-08-07.
        //
        // Dropped here rather than in `is_prose_line`, because "what yarn prints around its
        // answer" is knowledge about yarn, and the general rule is for what every manager does.
        "yarn" => {
            let body: String = clean
                .lines()
                .filter(|l| !is_yarn_chrome(l))
                .collect::<Vec<_>>()
                .join("\n");
            return or_unrecognised(
                backend,
                parse_yarn_list(&body),
                &crate::parsers::data_lines(&body),
            );
        }
        "gem" => parse_gem_list(&clean),
        "composer" => parse_composer_json(&clean),
        "go" => parse_go_list(&clean),
        other => {
            return Err(Unrecognised {
                backend: other.to_string(),
                data_lines: 0,
                sample: "no installed-listing reader is wired for this backend".into(),
            })
        }
    };

    or_unrecognised(backend, found, &crate::parsers::data_lines(&clean))
}

pub fn parse_search(backend: &str, output: &str) -> Vec<Package> {
    let clean = sanitize(output);
    match backend {
        "cargo" => parse_cargo_search(&clean),
        "gem" => parse_gem_search(&clean),
        "composer" => parse_composer_search(&clean),
        _ => vec![],
    }
}

/// Handles JSON dependencies for NPM and PNPM global lists. npm emits a single object
/// `{"dependencies": {...}}`, but `pnpm ls -g --json` emits an ARRAY of such objects
/// (`[{"dependencies": {...}}]`) — so normalize to a list of entries and pull each one's
/// dependency map, or pnpm's global packages parse as empty.
fn parse_npm_style_json(output: &str, backend: &str) -> Vec<Package> {
    let json: Value = serde_json::from_str(output).unwrap_or_default();
    let mut res = vec![];
    let entries: Vec<&Value> = match &json {
        Value::Array(items) => items.iter().collect(),
        other => vec![other],
    };
    for entry in entries {
        if let Some(deps) = entry.get("dependencies").and_then(|d| d.as_object()) {
            for (name, val) in deps {
                let version = val
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                res.push(Package::with_version(name, version, backend));
            }
        }
    }
    res
}

/// Parses the flat JSON array output of `pip list --format=json`.
fn parse_pip_json(output: &str) -> Vec<Package> {
    let json: Value = serde_json::from_str(output).unwrap_or_default();
    json.as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|p| {
            let name = p.get("name")?.as_str()?;
            let ver = p.get("version")?.as_str()?;
            Some(Package::with_version(name, ver, "pip"))
        })
        .collect()
}

/// Parses the complex JSON object of `pipx list --json`.
fn parse_pipx_json(output: &str) -> Vec<Package> {
    let json: Value = serde_json::from_str(output).unwrap_or_default();
    let mut res = vec![];
    if let Some(venvs) = json.get("venvs").and_then(|v| v.as_object()) {
        for (name, data) in venvs {
            // `package_version`, which is what pipx calls it. Reading `version` — a key that
            // schema does not have — gave every pipx package the version `unknown`, on every
            // machine, silently: `list` printed it, and `lock` skips a version reading
            // `unknown`, so `linix lock` pinned no pipx package and said nothing. Caught by the
            // first fixture captured from the tool itself.
            let ver = data
                .get("metadata")
                .and_then(|m| m.get("main_package"))
                .and_then(|p| p.get("package_version"))
                .and_then(|v| v.as_str());
            res.push(Package::with_version(
                name,
                ver.unwrap_or("unknown"),
                "pipx",
            ));
        }
    }
    res
}

/// Parses the formatted text list of `cargo install --list`.
/// Format: "name v1.2.3:"
fn parse_cargo_list(output: &str) -> Vec<Package> {
    output
        .lines()
        .filter(|l| l.contains(" v") && l.ends_with(':'))
        .filter_map(|l| {
            let parts: Vec<&str> = l.split_whitespace().collect();
            if parts.len() >= 2 {
                Some(Package::with_version(
                    parts[0],
                    parts[1].trim_matches(|c| c == 'v' || c == ':'),
                    "cargo",
                ))
            } else {
                None
            }
        })
        .collect()
}

/// Parses the ASCII tree output of `yarn global list`.
///
/// Top-level rows only, for the reason [`crate::parsers::ecosystem::pixi_list`] gives: in every
/// one of these trees an indented row is a child of the row above — a dependency, or a property
/// of the entry — and a parser that trims the indentation away before reading reports it as
/// something the user installed.
/// `yarn global list` — yarn 1, the only yarn that has `global` at all.
///
/// The package and its version appear in exactly one place, on the line yarn labels `info`:
///
/// ```text
/// yarn global v1.22.22
/// info "catj@1.0.4" has binaries:
///    - catj
/// Done in 0.07s.
/// ```
///
/// This parser used to be given `--json` and written for an ASCII tree, and yarn emits
/// neither. Measured on a host with `catj` installed: the JSON stream's only `list` record is
/// `{"type":"list","data":{"type":"bins-catj","items":["catj"]}}` — the *binaries*, not the
/// package — and the plain output has no tree either. So the filter that dropped every line
/// containing `info` dropped the one line that carries the answer, and `linix list -b yarn`
/// returned nothing on a machine with yarn packages on it.
///
/// That is not only `list`: `remove` is gated on `info`, which reads this same listing, so a
/// declared yarn package could not be removed and its manifest line went stale. **bun had the
/// identical bug for the identical reason** (see `parse_bun_list`) — a parser written for a
/// format the tool does not print, and nothing between them noticing, because an empty listing
/// looks exactly like an empty machine.
/// yarn's own banner and footer, which every `yarn` command prints whatever the answer is.
fn is_yarn_chrome(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("yarn global v") || t.starts_with("Done in ") || t.starts_with("warning ")
}

fn parse_yarn_list(output: &str) -> Vec<Package> {
    output
        .lines()
        .filter_map(|l| {
            // `info "<name>@<version>" has binaries:` / `... has no binaries`. The quotes are
            // what separate a package line from yarn's other `info` chatter, which carries none.
            let (spec, _) = l
                .trim()
                .strip_prefix("info ")?
                .strip_prefix('"')?
                .split_once('"')?;
            let (name, ver) = spec.rsplit_once('@')?;
            (!name.is_empty() && !ver.is_empty()).then(|| Package::with_version(name, ver, "yarn"))
        })
        .collect()
}

/// Parses the ASCII-tree output of `bun pm ls -g`.
/// Format: a header path line (e.g. "/root/.bun/install/global node_modules (2)")
/// followed by "├── name@version" / "└── name@version" rows. Scoped packages keep
/// their leading '@' in the name (e.g. "@scope/pkg@1.2.3" -> name "@scope/pkg").
/// The header has no '@', so it is filtered out.
///
/// A row indented past the first column is a dependency of the one above it — `bun pm ls -g
/// --all` prints four levels — and it is not globally installed. One flag away from forty
/// invented packages, which is `pixi global list`'s `exposes:` row in another tool.
fn parse_bun_list(output: &str) -> Vec<Package> {
    output
        .lines()
        .filter(|l| l.contains('@'))
        .filter(|l| !l.starts_with(char::is_whitespace) && !l.starts_with('│'))
        .filter_map(|l| {
            let cleaned = l
                .trim()
                .trim_start_matches(|c: char| {
                    c.is_whitespace() || matches!(c, '├' | '└' | '│' | '─')
                })
                .trim();
            let (name, ver) = cleaned.rsplit_once('@')?;
            if name.is_empty() || ver.is_empty() || ver.contains(char::is_whitespace) {
                return None;
            }
            Some(Package::with_version(name, ver, "bun"))
        })
        .collect()
}

/// Parses the text output of `gem list --local`.
/// Format: "name (1.2.3, 1.1.0)"
fn parse_gem_list(output: &str) -> Vec<Package> {
    output
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with("***"))
        .filter_map(|line| {
            let (name, rest) = line.split_once(' ')?;
            let ver = rest
                .trim()
                .trim_matches(|c| c == '(' || c == ')')
                .split(',')
                .next()?;
            // `bundler (default: 4.0.10)` — RubyGems marks the gems that ship with Ruby,
            // and the marker is not part of the version. Kept as one, `linix list` printed
            // `default: 4.0.10` in its version column: an `@version=` can never match it,
            // and `list --outdated` shows it beside a real version, which reads as LiNix
            // not knowing what is installed.
            let ver = ver.trim().strip_prefix("default:").unwrap_or(ver).trim();
            Some(Package::with_version(name.trim(), ver, "gem"))
        })
        .collect()
}

/// Parses the JSON output of `composer global show --format=json`.
fn parse_composer_json(output: &str) -> Vec<Package> {
    let json: Value = serde_json::from_str(output).unwrap_or_default();
    let mut res = vec![];
    if let Some(installed) = json.get("installed").and_then(|i| i.as_array()) {
        for pkg in installed {
            let name = pkg.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let version = pkg.get("version").and_then(|v| v.as_str()).unwrap_or("");
            if !name.is_empty() {
                res.push(Package::with_version(name, version, "composer"));
            }
        }
    }
    res
}

/// Parses simple binary names from Go-related outputs.
fn parse_go_list(output: &str) -> Vec<Package> {
    output
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| Package::new(l.trim(), "go"))
        .collect()
}

/// Specialized parser for `cargo search`.
fn parse_cargo_search(output: &str) -> Vec<Package> {
    output
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|l| {
            let (name, _) = l.split_once('=')?;
            Some(Package::new(name.trim(), "cargo"))
        })
        .collect()
}

/// Specialized parser for `gem search`.
fn parse_gem_search(output: &str) -> Vec<Package> {
    output
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with("***"))
        .filter_map(|l| {
            let (name, _) = l.split_once(' ')?;
            Some(Package::new(name.trim(), "gem"))
        })
        .collect()
}

/// Specialized parser for `composer search`.
fn parse_composer_search(output: &str) -> Vec<Package> {
    let json: Value = serde_json::from_str(output).unwrap_or_default();
    json.as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|p| {
            let name = p.get("name")?.as_str()?;
            Some(Package::new(name, "composer"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim from `yarn global list` on a host with `catj` installed, node's deprecation
    /// warning and all — the output a parser has to survive, not the one it would like.
    ///
    /// Before this, `parse_yarn_list` dropped every line containing `info` and looked for an
    /// ASCII tree. yarn prints no tree, and the `info` line is the only one carrying the
    /// package. So `linix list -b yarn` was empty on a machine with yarn packages installed,
    /// and `remove` — which is gated on `info`, reading the same listing — could not remove
    /// them either.
    #[test]
    fn yarn_reads_the_only_line_that_names_the_package() {
        let real = "yarn global v1.22.22
                    (node:12864) [DEP0169] DeprecationWarning: `url.parse()` behavior is not standardized
                    info \"catj@1.0.4\" has binaries:
   - catj
Done in 0.07s.
";
        let r = parse_yarn_list(real);
        assert_eq!(r.len(), 1, "{:?}", r);
        assert_eq!(r[0].name, "catj");
        assert_eq!(r[0].version.as_deref(), Some("1.0.4"));

        // A package with no binaries is still a package, and a scoped name keeps its own `@`.
        let more = "info \"@babel/cli@7.24.1\" has no binaries
info \"left-pad@1.3.0\" has no binaries
";
        let r = parse_yarn_list(more);
        assert_eq!(
            r.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["@babel/cli", "left-pad"]
        );
        assert_eq!(r[0].version.as_deref(), Some("7.24.1"));

        // yarn's other chatter carries no quoted spec and must not become a package. An
        // invented name is worse than a missing one: it reads as drift and schedules a removal.
        let chatter = "yarn global v1.22.22
info Visit https://yarnpkg.com/en/docs/cli/global for documentation
Done in 0.05s.
";
        assert!(parse_yarn_list(chatter).is_empty());
    }

    #[test]
    fn test_cargo_list_parsing() {
        let input = "ripgrep v13.0.0:\nexa v0.10.1:\n";
        let res = parse_cargo_list(input);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].name, "ripgrep");
        assert_eq!(res[1].version, Some("0.10.1".into()));
    }

    #[test]
    fn test_npm_object_and_pnpm_array_both_parse() {
        // npm: a single top-level object.
        let npm =
            r#"{"dependencies":{"cowsay":{"version":"1.6.0"},"typescript":{"version":"5.3.3"}}}"#;
        let r = parse_npm_style_json(npm, "npm");
        assert_eq!(r.len(), 2);
        assert!(r
            .iter()
            .any(|p| p.name == "cowsay" && p.version.as_deref() == Some("1.6.0")));
        // pnpm: `pnpm ls -g --json` wraps the same shape in an ARRAY — must parse too.
        let pnpm = r#"[{"path":"/x","private":true,"dependencies":{"cowsay":{"from":"cowsay","version":"1.6.0"}}}]"#;
        let r2 = parse_npm_style_json(pnpm, "pnpm");
        assert_eq!(
            r2.len(),
            1,
            "pnpm array form must parse (was empty before the fix)"
        );
        assert_eq!(r2[0].name, "cowsay");
        assert_eq!(r2[0].version.as_deref(), Some("1.6.0"));
        assert_eq!(r2[0].backend, "pnpm");
    }

    /// The tool's own output, captured from `bun pm ls -g` and `bun pm ls -g --all` on the
    /// machine this was written on. The second is the one with teeth: bun nests four levels
    /// deep, and a parser that trims the tree glyphs off before reading reports every
    /// dependency as a package someone installed.
    #[test]
    fn bun_reads_its_own_output_and_no_nested_row() {
        const FLAT: &str = include_str!("../../tests/fixtures/bun/ls-global.txt");
        const NESTED: &str = include_str!("../../tests/fixtures/bun/ls-global-all.txt");

        let flat = parse_bun_list(FLAT);
        assert_eq!(
            flat.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["cowsay"],
            "the header path line is not a package"
        );

        let nested = parse_bun_list(NESTED);
        let found = |n: &str, v: &str| {
            nested
                .iter()
                .any(|p| p.name == n && p.version.as_deref() == Some(v))
        };
        // `is-fullwidth-code-point` appears twice in that fixture: 2.0.0 hoisted to the top
        // level, 3.0.0 only as a child of `string-width`. Exactly one of them is installed.
        assert!(found("is-fullwidth-code-point", "2.0.0"), "{nested:?}");
        assert!(
            !found("is-fullwidth-code-point", "3.0.0"),
            "a row nested under `string-width@4.2.3` was reported as a global install"
        );
        assert!(!found("ansi-regex", "5.0.1"), "nested under `strip-ansi`");
    }

    #[test]
    fn test_bun_list_parsing() {
        // Real `bun pm ls -g` shape: header path line + tree rows, incl. a scoped pkg.
        let input = "/root/.bun/install/global node_modules (2)\n\
                     ├── cowsay@1.6.0\n\
                     └── @scope/tool@2.3.4\n";
        let res = parse_bun_list(input);
        assert_eq!(
            res.len(),
            2,
            "header line must be skipped, both pkgs parsed"
        );
        assert_eq!(res[0].name, "cowsay");
        assert_eq!(res[0].version, Some("1.6.0".into()));
        // Scoped names keep their leading '@'; only the trailing @version splits off.
        assert_eq!(res[1].name, "@scope/tool");
        assert_eq!(res[1].version, Some("2.3.4".into()));
    }

    #[test]
    fn test_composer_json_parsing() {
        let input = r#"{"installed": [{"name": "laravel/installer", "version": "v4.0.0"}]}"#;
        let res = parse_composer_json(input);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].name, "laravel/installer");
        assert_eq!(res[0].version, Some("v4.0.0".into()));
    }
}

/// `pip list --outdated --format=json` (`Q44`).
///
/// ```json
/// [{"name": "pip", "version": "26.1.2", "latest_version": "26.2.1", "latest_filetype": "wheel"}]
/// ```
///
/// `latest_version`, never `version` — the second is what is installed, and reporting it as the
/// available one makes every package look current.
pub fn parse_pip_outdated(output: &str) -> Vec<Package> {
    let Ok(items) = serde_json::from_str::<Value>(output) else {
        return Vec::new();
    };
    let Some(items) = items.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|p| {
            let name = p.get("name")?.as_str()?.trim();
            let latest = p.get("latest_version")?.as_str()?.trim();
            if name.is_empty() || latest.is_empty() {
                return None;
            }
            Some(Package::with_version(name, latest, "pip"))
        })
        .collect()
}

/// `npm outdated -g --json` / `pnpm outdated -g --json` (`Q44`).
///
/// ```json
/// {"typescript": {"current": "5.4.0", "wanted": "5.6.0", "latest": "5.7.2"}}
/// ```
///
/// **`latest`, not `wanted`.** `wanted` is the newest release satisfying the range in a
/// package.json, and a global install has no package.json to constrain it — reporting `wanted`
/// would hide a major version behind a caret nobody wrote.
///
/// `npm outdated` exits non-zero when it *finds* something, which is why this is read through
/// `run_output` rather than a status-checked reader.
pub fn parse_npm_outdated(output: &str, backend: &str) -> Vec<Package> {
    let Ok(doc) = serde_json::from_str::<Value>(output) else {
        return Vec::new();
    };
    let Some(map) = doc.as_object() else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(name, v)| {
            let latest = v.get("latest")?.as_str()?.trim();
            if name.trim().is_empty() || latest.is_empty() {
                return None;
            }
            Some(Package::with_version(name.trim(), latest, backend))
        })
        .collect()
}

/// `gem outdated`: `name (installed < latest)`, one per line (`Q44`).
pub fn parse_gem_outdated(output: &str) -> Vec<Package> {
    sanitize(output)
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (name, rest) = line.split_once(" (")?;
            let latest = rest.trim_end_matches(')').split('<').nth(1)?.trim();
            if name.is_empty() || latest.is_empty() {
                return None;
            }
            Some(Package::with_version(name.trim(), latest, "gem"))
        })
        .collect()
}

#[cfg(test)]
mod outdated_tests {
    use super::*;

    /// Verbatim from `pip list --outdated --format=json` on this host.
    const PIP: &str = r#"[{"name": "pip", "version": "26.1.2", "latest_version": "26.2.1", "latest_filetype": "wheel"}]"#;

    #[test]
    fn pip_reports_the_latest_version_not_the_installed_one() {
        let p = parse_pip_outdated(PIP);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "pip");
        assert_eq!(
            p[0].version.as_deref(),
            Some("26.2.1"),
            "`version` is what is installed; reading it as the available one makes every \
             package look current"
        );
    }

    /// `wanted` is the newest release inside a package.json range. A global install has no
    /// package.json, so honouring a caret nobody wrote would hide a major version.
    #[test]
    fn npm_reports_latest_rather_than_the_range_satisfying_wanted() {
        let out = r#"{"typescript":{"current":"5.4.0","wanted":"5.6.0","latest":"5.7.2"}}"#;
        let p = parse_npm_outdated(out, "npm");
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "typescript");
        assert_eq!(p[0].version.as_deref(), Some("5.7.2"));
        assert_eq!(p[0].backend, "npm");
    }

    /// Verbatim from `gem outdated` on this host.
    #[test]
    fn gem_reads_the_right_side_of_the_comparison() {
        let out = "bigdecimal (4.0.1 < 4.1.2)\nbundler (4.0.10 < 4.0.18)\ncsv (3.3.5 < 3.3.6)\n";
        let p = parse_gem_outdated(out);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].name, "bigdecimal");
        assert_eq!(p[0].version.as_deref(), Some("4.1.2"));
        assert_eq!(p[2].version.as_deref(), Some("3.3.6"));
    }

    /// Nothing outdated is a real answer and every one of these has its own way of saying it.
    #[test]
    fn nothing_outdated_is_nothing_not_a_row() {
        assert!(parse_pip_outdated("[]").is_empty());
        assert!(parse_pip_outdated("").is_empty());
        assert!(parse_npm_outdated("{}", "npm").is_empty());
        assert!(parse_npm_outdated("", "npm").is_empty());
        assert!(parse_gem_outdated("").is_empty());
        // gem prints nothing at all when everything is current.
        assert!(parse_gem_outdated("\n\n").is_empty());
    }
}

#[cfg(test)]
mod gem_default_tests {
    use super::*;

    /// `gem list --local` marks the gems that ship with Ruby, and the marker is not part of
    /// the version. Verbatim from this host.
    #[test]
    fn a_default_gem_reports_its_version_and_not_the_marker() {
        let out = "bigdecimal (4.0.1)\nbundler (default: 4.0.10)\ncsv (3.3.5, 3.3.4)\n";
        let p = parse_installed("gem", out).expect("this fixture parses");
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].version.as_deref(), Some("4.0.1"));
        assert_eq!(
            p[1].version.as_deref(),
            Some("4.0.10"),
            "`default:` is RubyGems saying the gem ships with Ruby, not part of the version"
        );
        // The existing behaviour for a multi-version gem is unchanged: the newest wins.
        assert_eq!(p[2].version.as_deref(), Some("3.3.5"));
    }
}

/// `composer global outdated --format=json` (`Q44`).
///
/// ```json
/// {"installed":[{"name":"psr/log","version":"1.1.4","latest":"3.0.2",
///                "latest-status":"update-possible"}]}
/// ```
///
/// **`latest`, not `version`** — the second is what is installed. And the JSON is *found*, not
/// assumed: composer prints `Changed current directory to /root/.composer` ahead of it, so a
/// strict parse fails and reports nothing outdated on every machine with a global config dir,
/// which is all of them.
pub fn parse_composer_outdated(output: &str) -> Vec<Package> {
    let text = sanitize(output);
    let Some(start) = text.find('{') else {
        return Vec::new();
    };
    let Ok(doc) = serde_json::from_str::<Value>(&text[start..]) else {
        return Vec::new();
    };
    let Some(items) = doc.get("installed").and_then(|i| i.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|p| {
            let name = p.get("name")?.as_str()?.trim();
            let latest = p.get("latest")?.as_str()?.trim();
            if name.is_empty() || latest.is_empty() {
                return None;
            }
            Some(Package::with_version(name, latest, "composer"))
        })
        .collect()
}

#[cfg(test)]
mod composer_outdated_tests {
    use super::*;

    /// Verbatim from `composer global outdated --format=json` in an `ubuntu:24.04` container,
    /// banner included — composer really does print that line first.
    const REAL: &str = r#"Changed current directory to /root/.composer
{
    "installed": [
        {
            "name": "psr/log",
            "direct-dependency": true,
            "version": "1.1.4",
            "latest": "3.0.2",
            "latest-status": "update-possible"
        }
    ]
}"#;

    #[test]
    fn composer_reads_latest_past_its_own_banner() {
        let p = parse_composer_outdated(REAL);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "psr/log");
        assert_eq!(
            p[0].version.as_deref(),
            Some("3.0.2"),
            "`version` is what is installed; `latest` is the answer"
        );
        assert_eq!(p[0].backend, "composer");
    }

    #[test]
    fn nothing_outdated_is_nothing() {
        assert!(parse_composer_outdated("").is_empty());
        assert!(parse_composer_outdated("Changed current directory to /x\n").is_empty());
        assert!(parse_composer_outdated(r#"{"installed":[]}"#).is_empty());
    }
}

/// The two listings a real machine had been failing to read, silently, until `LX-1` gave a
/// parser the words for it.
#[cfg(test)]
mod an_empty_listing_is_not_an_unreadable_one_tests {
    use super::*;

    /// **Both of these were found by `linix list` on a real Windows box, 2026-08-07** — the run
    /// that `LX-1` made capable of complaining. Before it, each was a silent empty listing, which
    /// the planner reads as *"this machine has none of these"* and answers by installing every
    /// declared package and dropping every removal.
    ///
    /// They are opposite mistakes with the same symptom, which is why both are pinned here.
    #[test]
    fn an_empty_pipx_is_empty_and_not_unreadable() {
        // `pipx list --json` on a machine with nothing: a sentence, then a JSON document whose
        // `venvs` object is empty. Four lines that are neither prose nor package rows, so the
        // line count said "unread" about an answer the reader understood perfectly.
        let out = "nothing has been installed with pipx 
{
    \"pipx_spec_version\": \"0.1\",
    \"venvs\": {}
}
";
        assert_eq!(
            parse_installed("pipx", out).expect("an empty pipx is empty, not unreadable"),
            Vec::new()
        );
    }

    #[test]
    fn a_populated_pipx_still_parses() {
        // The other half: the JSON short-circuit must not swallow a real answer.
        let out = r#"{"pipx_spec_version":"0.1","venvs":{"black":{"metadata":{"main_package":{"package":"black","package_version":"24.1.0"}}}}}"#;
        let pkgs = parse_installed("pipx", out).expect("parses");
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "black");
        assert_eq!(pkgs[0].version.as_deref(), Some("24.1.0"));
    }

    #[test]
    fn an_empty_yarn_global_is_empty_and_not_unreadable() {
        // yarn brackets every command with a banner and a footer, so an empty global install
        // still prints two lines — and neither is prose by the general rule.
        let out = "yarn global v1.22.22
Done in 0.67s.
";
        assert_eq!(
            parse_installed("yarn", out).expect("an empty yarn global is empty, not unreadable"),
            Vec::new()
        );
    }

    #[test]
    fn a_populated_yarn_global_still_parses_through_its_own_chrome() {
        let out = "yarn global v1.22.22
info \"typescript@5.4.5\" has binaries:
   - tsc
Done in 0.67s.
";
        let pkgs = parse_installed("yarn", out).expect("parses");
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "typescript");
        assert_eq!(pkgs[0].version.as_deref(), Some("5.4.5"));
    }

    /// The check that keeps the JSON short-circuit honest: a document that does NOT parse is
    /// still unread, however many braces it opens with.
    #[test]
    fn broken_json_is_still_unreadable() {
        assert!(parse_installed("pipx", "{
  \"venvs\": {
").is_err());
    }
}
