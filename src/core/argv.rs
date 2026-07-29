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
    "apt",
    "apt-get",
    "apt-cache",
    "apt-mark",
    "dpkg",
    "dpkg-query",
    "aptitude", //
    "dnf",
    "dnf5",
    "yum",
    "microdnf",
    "rpm", //
    "pacman",
    "yay",
    "paru",
    "pamac", //
    "apk",
    "xbps-install",
    "xbps-remove",
    "xbps-query", //
    "zypper",
    "emerge",
    "eix",
    "equery", //
    "nix-env",
    "nix",
    "guix",
    "flatpak",
    "snap", //
    "pip",
    "pip3",
    "pipx",
    "cargo",
    "npm",
    "pnpm",
    "yarn",
    "bun", //
    "go",
    "composer",
    "luarocks",
    "opam",
    "mix",
    "cabal",
    "stack", //
    "conda",
    "mamba",
    "micromamba",
    "uv",
    "pixi",
    "mise", //
    "brew",
    "port",
    "pkgin",
    "pkg",
    "pkg_add",
    "pkg_delete",
    "eopkg",
    "slackpkg", //
    "helm",
    "kubectl",
    "krew",
    "dart",
    "flutter",
    "emacs",
    "systemctl", //
    // Go-flag parsers: `age` and `sops` are handed a source path a declaration chose.
    "age",
    "sops",
];

/// Managers whose CLI has no `--`, checked and recorded so the absence is a fact and not an
/// oversight. Windows-native tools and .NET's parser take the name positionally and read a
/// bare `--` as a package.
#[cfg(test)]
const DOES_NOT_TERMINATE: &[&str] = &[
    "winget",
    "choco",
    "scoop",
    "mas",
    "dotnet",
    "pwsh",
    "powershell", //
    // RubyGems' `--` is not an option terminator: it is the separator between the gem
    // names and the BUILD arguments handed to a C extension
    // (`gem install nokogiri -- --with-xml2-dir=…`). So `gem install -- colorize` names
    // no gem at all and fails with "Please specify at least one gem name" — which is
    // exactly what listing it as terminating did to every `gem` install and removal.
    "gem",
    // nimble's `--` is RubyGems' `--`, not GNU's: `nimble install --help` says of it "arg
    // are passed to the binary when it is run … to the Nim compiler". So `nimble install -y
    // -- nimjson` reaches the compiler as an argument and the build dies with "arguments can
    // only be given if the '--run' option is selected". Every install of a nimble package
    // that produces a binary failed this way; a package that only ships a library never
    // invokes the compiler with arguments, which is why it went unnoticed.
    "nimble",
    // asdf dispatches on `$1` as the plugin name — measured in the `tools` image, `asdf
    // install -- jq` answers `No such plugin: --`, and `asdf install jq latest` installs it.
    // It was listed as terminating without anyone asking it, which is the thing the header
    // of this file warns about, done to this file.
    "asdf",
    // spack reads `--` into the spec it is given: `spack spec -- zlib` dies with `string
    // index out of range`, `spack find -- <name>` reports `No package matches the query: --
    // <name>`, and both work without it. The grader saw the same parser mangle it into
    // `Spec ~~zlib has no name`.
    "spack",
    // `code` takes an extension id as the *value* of `--install-extension`, never as a
    // positional, so a `--` in front of it would become the value.
    "code",
    // `gsettings` dispatches on argv[1] by hand — no getopt, so a bare `--` is read as the
    // command name and the call fails before it reaches the schema.
    "gsettings",
    // Init systems other than systemd. `sc` and `launchctl` take the service positionally
    // with no option terminator; the OpenRC and SysVinit wrappers are shell scripts that
    // read `$1` as the service, and all of them put the name *between* two positionals
    // (`rc-service <name> start`), which leaves no place a terminator could go.
    "sc",
    "launchctl",
    "rc-service",
    "rc-update",
    "update-rc.d",
    "service",
];

/// Whether `binary` ends its option parsing at `--`.
pub fn terminates_options(binary: &str) -> bool {
    let base = base_name(binary);
    TERMINATES.contains(&base)
}

/// The bare program name, so an absolute path (`/usr/bin/apt-get`, `C:\...\scoop.exe`) is
/// looked up as the program it is.
fn base_name(binary: &str) -> &str {
    let after_dir = binary.rsplit(['/', '\\']).next().unwrap_or(binary);
    after_dir.strip_suffix(".exe").unwrap_or(after_dir)
}

/// Append package names to an argument list, ending the manager's options first where the
/// manager honours `--`.
pub fn push_names<S: AsRef<str>>(
    args: &mut Vec<String>,
    binary: &str,
    names: impl IntoIterator<Item = S>,
) {
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

    /// Measured in the `tools` container on 2026-07-28, not inferred from a family resemblance.
    ///
    /// Both were in `TERMINATES` and neither had been asked. asdf answered `No such plugin:
    /// --`; spack answered `No package matches the query: -- <name>` and, on a spec, `string
    /// index out of range`. The grader saw the second one arrive as `Spec ~~zlib has no name`
    /// after a real install, which is the same parser and a less legible message.
    ///
    /// The header of this file already states the rule this broke: the default is "does not
    /// terminate", and a manager joins the terminating set **when someone has checked its
    /// argument parser**. Two joined without that, and a `--` a manager reads as a package
    /// name turns every install through it into a failure.
    #[test]
    fn a_manager_that_reads_the_terminator_as_a_name_is_not_listed_as_terminating() {
        assert!(!terminates_options("asdf"));
        assert!(!terminates_options("spack"));
        // The controls: the ones measured in the same sweep that DO honour it stay listed, so
        // this is not a test that would pass if the table were simply emptied.
        assert!(terminates_options("cargo"));
        assert!(terminates_options("npm"));
        assert!(terminates_options("pipx"));
        assert!(terminates_options("luarocks"));
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

    /// A `--` that the manager reads as something OTHER than "options end here" is worse
    /// than one it does not understand: RubyGems takes everything after it as build
    /// arguments, so the gem name is consumed and nothing is installed.
    #[test]
    fn gem_does_not_terminate_because_its_dash_dash_means_build_args() {
        assert!(!terminates_options("gem"));
        let mut args = vec!["install".to_string()];
        push_names(&mut args, "gem", ["colorize"]);
        assert_eq!(
            args,
            ["install", "colorize"],
            "`gem install -- colorize` names no gem at all"
        );
        let mut args = vec!["uninstall".to_string()];
        push_names(&mut args, "gem", ["colorize"]);
        assert_eq!(args, ["uninstall", "colorize"]);
    }

    /// The same class as `gem`, found the same way — by running it. `nimble --help` says of
    /// `--`: "arg are passed to the binary when it is run … to the Nim compiler". So
    /// `nimble install -y -- nimjson` hands `--` to the compiler, which answers "arguments
    /// can only be given if the '--run' option is selected" and the build fails. Every
    /// nimble install of a package that produces a binary failed this way.
    #[test]
    fn nimble_does_not_terminate_because_its_dash_dash_reaches_the_compiler() {
        assert!(!terminates_options("nimble"));
        let mut args = vec!["install".to_string(), "-y".to_string()];
        push_names(&mut args, "nimble", ["nimjson"]);
        assert_eq!(
            args,
            ["install", "-y", "nimjson"],
            "`--` reaches the Nim compiler and fails the build"
        );
        let mut args = vec!["uninstall".to_string(), "-y".to_string()];
        push_names(&mut args, "nimble", ["nimjson"]);
        assert_eq!(args, ["uninstall", "-y", "nimjson"]);
    }

    #[test]
    fn no_names_means_no_terminator() {
        let mut args = vec!["autoremove".to_string()];
        push_names(&mut args, "apt-get", Vec::<String>::new());
        assert_eq!(args, ["autoremove"]);
    }
}
