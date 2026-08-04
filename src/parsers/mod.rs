pub mod apt;
pub mod bsd;
pub mod common;
pub mod conda;
pub mod dnf;
pub mod dotnet;
pub mod ecosystem;
pub mod language;
pub mod macos;
pub mod pacman;
pub mod pkgsrc;
pub mod utils;
pub mod windows;

use crate::core::Package;

pub trait OutputParser: Send + Sync {
    fn parse_installed(&self, output: &str) -> Vec<Package>;

    fn parse_search(&self, output: &str) -> Vec<Package>;

    /// Parses a manager's listing of packages the OS itself treats as essential — the
    /// ones removal must never touch, whatever a manifest says. Default: the manager
    /// exposes no such concept, so it reports none.
    fn parse_essential(&self, _output: &str) -> Vec<String> {
        Vec::new()
    }
}

/// Parses a listing of bare package names, one per line — the shape every manager that
/// can report its *explicit* set emits (`apt-mark showmanual`, `dnf repoquery
/// --userinstalled`, `xbps-query --list-manual-pkgs`, apk's `/etc/apk/world`). Versions
/// are absent by design; callers needing one reconcile against `list_installed`.
///
/// A trailing version constraint (`busybox>=1.36`) and a repository tag (`nodejs@edge`)
/// are stripped — apk's world file carries both. `!name` entries are conflict markers,
/// not installs, and are dropped.
///
/// An architecture qualifier is also stripped: `apt-mark showmanual` prints `libc6:i386`
/// on a multi-arch host, while `dpkg-query -W -f='${Package}'` prints the bare `libc6`.
/// Keeping the suffix would record a managed package whose name matches nothing the
/// installed-listing ever reports — permanent phantom drift, and a removal candidate that
/// can never be satisfied.
pub fn parse_bare_names(output: &str, backend: &str) -> Vec<Package> {
    crate::utils::text::sanitize(output)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('!'))
        .filter_map(|l| {
            let name = l.split(['>', '<', '=', '~', '@', ':', ' ']).next()?.trim();
            (!name.is_empty()).then(|| Package::new(name, backend))
        })
        .collect()
}

/// A Functional Strategy Parser that allows injecting functions as data.
/// Used in backends/registry.rs to configure GenericManagers without
/// creating dozens of boilerplate structs.
pub struct LambdaParser {
    pub installed_fn: fn(&str) -> Vec<Package>,
    pub search_fn: fn(&str) -> Vec<Package>,
}

impl OutputParser for LambdaParser {
    fn parse_installed(&self, output: &str) -> Vec<Package> {
        (self.installed_fn)(output)
    }
    fn parse_search(&self, output: &str) -> Vec<Package> {
        (self.search_fn)(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_names_parses_apt_mark_showmanual() {
        // `apt-mark showmanual` prints bare names, no versions — which is why the normal
        // apt list parser (which splits "name version") silently returned nothing.
        let pkgs = parse_bare_names("apt\nbase-files\njq\n", "apt");
        let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["apt", "base-files", "jq"]);
        assert_eq!(pkgs[0].backend, "apt");
    }

    #[test]
    fn bare_names_strips_the_architecture_qualifier() {
        // showmanual prints `libc6:i386` on a multi-arch host while dpkg-query prints the
        // bare `libc6`. Keeping the suffix records a package nothing can ever match.
        let pkgs = parse_bare_names("libc6:i386\n", "apt");
        assert_eq!(pkgs[0].name, "libc6");
    }

    #[test]
    fn bare_names_handles_apk_world_entries() {
        // apk's world file carries version constraints, repo tags, comments, and `!`
        // conflict markers, which are not installs.
        let pkgs = parse_bare_names(
            "# comment\nbusybox>=1.36\nnodejs@edge\nbash=5.2\n!conflicting\n\ncurl\n",
            "apk",
        );
        let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["busybox", "nodejs", "bash", "curl"]);
    }
}
