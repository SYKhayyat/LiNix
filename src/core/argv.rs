//! Where a package name stops being an option (II.12b).
//!
//! The grammar refuses a name that starts with `-`. That is the layer that can name the file
//! and the line, and it holds for every manager. This is the other layer: the set of flags
//! belongs to the manager and changes without us, so an invocation ends its own options before
//! the names begin — `apt-get install -y -- ripgrep`.
//!
//! Not every CLI has a `--`. A manager that does not honour it is listed here by name, and
//! **it is listed rather than assumed**: a `--` a manager parses as a package name turns every
//! install into a failure, which is worse than the leading-dash refusal the grammar already
//! made. The default is therefore "does not terminate", and a manager joins the terminating
//! set when someone has checked its argument parser.

/// Managers whose CLI ends option parsing at `--`.
///
/// Keyed on the binary actually invoked, not the LiNix backend name, because that is what
/// parses the arguments (apt's installs run `apt-get`, its queries run `dpkg-query`).
const TERMINATES: &[&str] = &[
    // Coreutils/GNU-style parsers.
    "apt", "apt-get", "apt-cache", "apt-mark", "dpkg", "dpkg-query", "aptitude", //
    "dnf", "dnf5", "yum", "microdnf", "rpm", //
    "pacman", "yay", "paru", "pamac", //
    "apk", "xbps-install", "xbps-remove", "xbps-query", //
    "zypper", "emerge", "eix", "equery", //
    "nix-env", "nix", "guix", "flatpak", "snap", //
    "pip", "pip3", "pipx", "gem", "cargo", "npm", "pnpm", "yarn", "bun", //
    "go", "composer", "luarocks", "opam", "nimble", "mix", "cabal", "stack", //
    "conda", "mamba", "micromamba", "uv", "pixi", "spack", "asdf", "mise", //
    "brew", "port", "pkgin", "pkg", "pkg_add", "pkg_delete", "eopkg", "slackpkg", //
    "helm", "kubectl", "krew", "dart", "flutter", "emacs", "systemctl",
];

/// Managers whose CLI has no `--`, checked and recorded so the absence is a fact and not an
/// oversight. Windows-native tools and .NET's parser take the name positionally and read a
/// bare `--` as a package.
#[cfg(test)]
const DOES_NOT_TERMINATE: &[&str] = &[
    "winget", "choco", "scoop", "mas", "dotnet", "pwsh", "powershell", //
    // `code` takes an extension id as the *value* of `--install-extension`, never as a
    // positional, so a `--` in front of it would become the value.
    "code",
];

/// Whether `binary` ends its option parsing at `--`.
pub fn terminates_options(binary: &str) -> bool {
    let base = base_name(binary);
    TERMINATES.contains(&base)
}

/// The bare program name, so an absolute path (`/usr/bin/apt-get`, `C:\...\scoop.exe`) is
/// looked up as the program it is.
fn base_name(binary: &str) -> &str {
    let after_dir = binary
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(binary);
    after_dir.strip_suffix(".exe").unwrap_or(after_dir)
}

/// Append package names to an argument list, ending the manager's options first where the
/// manager honours `--`.
pub fn push_names<S: AsRef<str>>(args: &mut Vec<String>, binary: &str, names: impl IntoIterator<Item = S>) {
    let mut names = names.into_iter().peekable();
    if names.peek().is_none() {
        return;
    }
    if terminates_options(binary) {
        args.push("--".to_string());
    }
    for n in names {
        args.push(n.as_ref().to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gnu_style_manager_terminates_and_a_windows_one_does_not() {
        assert!(terminates_options("apt-get"));
        assert!(terminates_options("dnf"));
        assert!(terminates_options("pacman"));
        assert!(terminates_options("brew"));
        assert!(!terminates_options("winget"));
        assert!(!terminates_options("scoop"));
        assert!(!terminates_options("choco"));
    }

    #[test]
    fn the_two_tables_never_disagree() {
        for b in DOES_NOT_TERMINATE {
            assert!(
                !TERMINATES.contains(b),
                "`{}` is in both tables — one invocation cannot both end its options and not",
                b
            );
        }
    }

    #[test]
    fn a_path_or_an_exe_is_looked_up_as_the_program_it_is() {
        assert!(terminates_options("/usr/bin/apt-get"));
        assert!(terminates_options(r"C:\tools\cargo.exe"));
        assert!(!terminates_options(r"C:\Users\me\scoop\shims\scoop.exe"));
    }

    #[test]
    fn names_go_behind_the_terminator_where_there_is_one() {
        let mut args = vec!["install".to_string(), "-y".to_string()];
        push_names(&mut args, "apt-get", ["ripgrep"]);
        assert_eq!(args, ["install", "-y", "--", "ripgrep"]);

        let mut args = vec!["install".to_string()];
        push_names(&mut args, "winget", ["ripgrep"]);
        assert_eq!(args, ["install", "ripgrep"]);
    }

    #[test]
    fn no_names_means_no_terminator() {
        let mut args = vec!["autoremove".to_string()];
        push_names(&mut args, "apt-get", Vec::<String>::new());
        assert_eq!(args, ["autoremove"]);
    }
}
